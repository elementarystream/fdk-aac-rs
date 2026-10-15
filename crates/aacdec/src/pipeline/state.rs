// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the ownership of the decoder state between the two stages of the frame pipeline.

//! Which stage owns which member of [`AacDecoderInstance`], and how the state moves between
//! them.
//!
//! Stage A runs on the decoder instance (the *main* instance); stage B runs on a second
//! instance (the *shadow*) on the worker thread, or on the main instance when it runs in the
//! calling thread. The members fall into four groups:
//!
//! | Group | Members | Handling |
//! |---|---|---|
//! | Owned by stage B | `pAacDecoderStaticChannelInfo[]`, `pCpeStaticData[]`, `pComStaticData`, `pWorkBufferCore1`, `workBufferCore1`, `pTimeData2`, `hSbrDecoder`, `qmfDomain`, `hDrcInfo`, `pMpegSurroundDecoder`, `hPcmUtils`, `hLimiter`, `hUniDrcDecoder`, `pTimeDataFlush[]`, `usacResidualDelay`, and the per channel thread pool with its scratch memory (`chan_par`, not in C) | moved into the shadow at the hand-over ([`hand_over`]) and back at the join ([`return_from_stage_b`]); stage A reaches them only after a join ([`super::stage_a`]) |
//! | Per frame channel memory | `pAacDecoderChannelInfo[]` (with `pDynData`), `pComData`, `workBufferCore2` (the spectra) | two sets: stage B of frame N works on the set stage A of frame N filled, while stage A of frame N + 1 fills the other one ([`swap_channel_memory`]). A frame that is handed over reads nothing an earlier frame left in it, except members that are equal in both sets (set at initialization, or written with the same value every frame of a configuration); checked by poisoning ([`poison_stage_a`]). A frame that is not handed over after its stage A wrote the other set (errors, concealment, state of stage B needed) is decoded again on the set of the previous frame ([`take_snapshot`], [`restart`]); every join outside such a stage A gives the main instance the set of the joined frame back ([`super::join`]) |
//! | Written by stage B | the stream information of the output (`sampleRate`, `frameSize`, `numChannels`, `outputDelay`, `flags`, the statistics, the DRC and loudness fields, `pChannelType[]` / `pChannelIndices[]`), `flags` bits `AC_SBR_PRESENT`, `AC_PS_PRESENT`, `AC_DRC_PRESENT`, `psPossible`, `qmfModeCurr`, `mpsEnableCurr`, `mpsOutChannelsLast`, `mpsFrameSizeLast`, `extGain`, `extGainDelay`, `applyCrossfade` | copied back into the main instance at the join ([`merge_b_scalars`]). Stage A does not read or write them before (its writes of `qmfModeCurr`, `psPossible` and `mpsEnableCurr` in `CAacDecoder_SyncQmfMode()` are recorded for stage B; the bits of `flags` it reads are not those, or not while stage B could change them, see below); the channel tables are merged entry by entry (see below) |
//! | Everything else | configuration, the transport decoder, the element tables, `frameOK`, ... | the main instance's; copied into the shadow at the hand-over (stage B reads them as stage A left them) |
//!
//! `AC_SBR_PRESENT`: stage A reads it only together with `sbrEnabled == 1`
//! (`AC_SBR_PRESENT || sbrEnabled == 1`); stage B sets it only after an SBR decoding, which
//! needs `sbrEnabled` set by stage A of the same frame, so stage A of the next frame starts
//! with `sbrEnabled == 1` and does not depend on the bit. `AC_PS_PRESENT` is read by stage A
//! only for MPEG Surround payloads, after a join.
//!
//! The channel tables (`pChannelType[]`, `pChannelIndices[]`): stage A writes the entries of
//! the channels it finds (`CProgramConfig_LookupElement()`), stores all eight in
//! `channelTypePrev` / `channelIndicesPrev`, restores them from there for a defective frame,
//! and reorders the entries of its channels; stage B (SBR with PS, MPEG Surround, the downmix)
//! writes the entries of the output. While stage B of the previous frame runs, stage A records
//! which entries it writes (`t_mask`); at the join, the other entries take the values of stage
//! B, which come before stage A's writes in the sequential order, and the entries of
//! `channelTypePrev` / `channelIndicesPrev` that stage A had not written when it stored them
//! take them as well. Stage A reads entries it did not write only when it reorders the tables
//! of a frame whose channels it did not all write; it waits for stage B first then.

use super::stage_a::{join_mid_a, payload_bitstream, SbrCache};
use super::worker::{Job, Worker};
use super::{AncSnapshot, BOp, BWrite, HandOver, Pending, Pipeline};
use crate::aacdecoder::{caac_decoder_sync_qmf_mode, AacDecoderInstance};
use crate::aacdecoder_lib::CStreamInfo;
use crate::channelinfo::CAacDecoderChannelInfo;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_sbrdec::sbrdecoder::sbr_decoder_parse;

/// Whether stage A may run for a call with these flags: only calls without flags, outside the
/// flushing and build-up of a configuration change and without pre-roll (every other call runs
/// the sequential decoder after a join).
pub(crate) fn eligible_before_a(self_: &AacDecoderInstance, flags: u32) -> bool {
    flags == 0
        && self_.flush_status == 0
        && self_.build_up_status == 0
        && self_.has_audio_pre_roll == 0
        && self_.h_input.is_some()
}

/// Whether the configuration after the transport decoder's part of the call can be decoded by
/// stage A: no USAC (`AC_USAC`, `AC_RSVD50`, `AC_RSV603DA`), no DRM, no RVLC (their parsers use
/// the static channel infos or the SBR decoder's state), and no pre-roll or configuration
/// change in progress.
pub(crate) fn eligible_after_read(self_: &AacDecoderInstance) -> bool {
    self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_DRM | AC_ER_RVLC) == 0
        && self_.flush_status == 0
        && self_.build_up_status == 0
        && self_.has_audio_pre_roll == 0
        && self_.p_aac_decoder_channel_info.iter().filter(|c| c.is_some()).count()
            == self_.aac_channels.clamp(0, 8) as usize
}

fn pipe(self_: &mut AacDecoderInstance) -> &mut Pipeline {
    self_.pipe.as_deref_mut().expect("pipeline")
}

/// Prepares stage A of a frame.
pub(crate) fn begin_stage_a(self_: &mut AacDecoderInstance) {
    let p = pipe(self_);
    p.cb_join = false;
    p.pce_in_frame = false;
    p.t_mask = 0;
    p.t_prev_stored = false;
    p.t_prev_mask = 0;
    debug_assert!(p.b_ops.is_empty());
}

/// Stage A stored `channelTypePrev` / `channelIndicesPrev`.
pub(crate) fn note_prev_store(self_: &mut AacDecoderInstance) {
    if let Some(p) = self_.pipe.as_deref_mut() {
        if p.away() {
            p.t_prev_stored = true;
            p.t_prev_mask = p.t_mask;
        }
    }
}

/// Stage A reorders the channel tables of `aac_channels` channels: it reads their entries,
/// which must not be entries stage B of the previous frame may still write.
pub(crate) fn before_reorder(self_: &mut AacDecoderInstance, aac_channels: i32) {
    let Some(p) = self_.pipe.as_deref() else { return };
    if !p.away() {
        return;
    }
    let n = aac_channels.clamp(0, 8) as u32;
    let need: u8 = if n >= 8 { 0xff } else { ((1u32 << n) - 1) as u8 };
    if p.t_mask & need != need {
        // (a restart: the reordered tables are discarded)
        let _ = join_mid_a(self_);
    }
}

/// Stage A read a program config element (the downmix settings of the PCE go to stage B's PCM
/// downmix: the frame is decoded in the calling thread).
pub(crate) fn note_pce(self_: &mut AacDecoderInstance) {
    if let Some(p) = self_.pipe.as_deref_mut() {
        p.pce_in_frame = true;
    }
}

/// A configuration callback of the transport decoder in stage A: waits for stage B (the
/// callbacks use state of both stages); the frame is decoded in the calling thread.
pub(crate) fn callback_join(self_: &mut AacDecoderInstance) {
    if self_.pipe.is_some() {
        // (the transport decoder calls them only in transportDec_ReadAccessUnit(), before
        // stage A writes the per frame channel memory: the join does not restart the frame)
        let restart = join_mid_a(self_);
        debug_assert!(!restart);
        pipe(self_).cb_join = true;
    }
}

/// `aacDecoder_ConfigCallback()` of a decoder with a pipeline.
///
/// The transport decoder calls it for every configuration it reads: twice for every ADTS
/// frame (`AC_CM_DET_CFG_CHANGE`, then `AC_CM_ALLOC_MEM`), and for every LATM StreamMuxConfig.
/// While stage B of the previous frame runs, a call that cannot change the configuration runs
/// without waiting for it: the detection pass, and the allocation pass of an unchanged
/// configuration, of an MPEG-4 GA or ER configuration without a PCE (a PCE sets the downmix
/// of stage B). Such a call writes only state of stage A, except `streamInfo.bitRate` (0),
/// `flags` (`CAacDecoder_AcceptFlags()`), `streamInfo.flags` and, for ELD, `mpsEnableCurr`,
/// which are recorded ([`BWrite`]), and the expiry settings of the DRC and downmix modules
/// (`aacDecoder_setMetadataExpiry()`), which are recorded for stage B ([`BOp`]). It reads no
/// state of stage B (`mpsEnableCurr` is read, and written back unchanged, except for ELD and
/// USAC, where it is computed). A change of the concealment method waits for stage B.
pub(crate) fn config_callback(
    dec: &mut AacDecoderInstance,
    asc: &fdk_aac_rs_tpdec::tpdec_lib::CSAudioSpecificConfig,
    config_mode: u8,
    config_changed: &mut u8,
) -> i32 {
    use crate::aacdecoder_lib::{
        aac_decoder_config, aac_decoder_config_callback, is_init_error, metadata_expiry_frames,
        set_conceal_method, set_metadata_expiry_frames, AAC_DEC_NEED_TO_RESTART, AAC_DEC_OK,
    };
    use crate::conceal::cconcealment_get_delay;
    use crate::conceal_types::ConcealMethodNone;
    use fdk_aac_rs_tpdec::tpdec_lib::{
        cprogram_config_is_valid, TRANSPORTDEC_NEED_TO_RESTART, TRANSPORTDEC_OK,
        TRANSPORTDEC_UNKOWN_ERROR, TRANSPORTDEC_UNSUPPORTED_FORMAT,
    };

    let away = dec.pipe.as_deref().is_some_and(|p| p.away());
    let alloc = config_mode & AC_CM_ALLOC_MEM != 0;
    let unchanging = config_mode & AC_CM_DET_CFG_CHANGE != 0 || (alloc && *config_changed == 0);
    let plain_config = matches!(
        asc.m_aot,
        AOT_AAC_LC | AOT_SBR | AOT_PS | AOT_ER_AAC_LC | AOT_ER_AAC_LD | AOT_ER_AAC_ELD
    ) && !(asc.m_channel_configuration == 0
        && cprogram_config_is_valid(&asc.m_progr_config_element) != 0);
    if !(away && unchanging && plain_config) {
        if away {
            let restart = join_mid_a(dec);
            debug_assert!(!restart);
        }
        let r = aac_decoder_config_callback(dec, asc, config_mode, config_changed);
        if !unchanging || !plain_config || *config_changed != 0 {
            // (a configuration change: stage B of this frame runs in the calling thread)
            pipe(dec).cb_join = true;
        }
        return r;
    }

    // The bit rate is written before most of the error returns of CAacDecoder_Init().
    const MARK: i32 = i32::MIN;
    let bit_rate = dec.stream_info.bit_rate;
    dec.stream_info.bit_rate = MARK;
    let mps_enable_curr = dec.mps_enable_curr;
    let err = aac_decoder_config(dec, asc, config_mode, config_changed);
    debug_assert!(!alloc || *config_changed == 0);
    let mut writes = Vec::new();
    if dec.stream_info.bit_rate == MARK {
        dec.stream_info.bit_rate = bit_rate;
    } else {
        writes.push(BWrite::BitRate(dec.stream_info.bit_rate));
    }
    if err == AAC_DEC_OK {
        // CAacDecoder_AcceptFlags() (both passes), and at the end of the allocation pass
        // streamInfo.flags and mpsEnableCurr.
        writes.push(BWrite::FlagsB(dec.flags[0]));
        if alloc {
            writes.push(BWrite::SiFlags(dec.stream_info.flags));
            if asc.m_aot == AOT_ER_AAC_ELD {
                writes.push(BWrite::MpsEnableCurr(dec.mps_enable_curr));
            } else {
                debug_assert!(dec.mps_enable_curr == mps_enable_curr);
            }
        }
    }
    pipe(dec).b_writes.extend(writes);

    if err == AAC_DEC_OK {
        let delay = cconcealment_get_delay(Some(&dec.conceal_common_data));
        if ((dec.flags[0] & (AC_LD | AC_ELD)) != 0
            && (dec.conceal_method_user == ConcealMethodNone)
            && delay > 0)
            || ((dec.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA)) != 0 && delay > 0)
        {
            // (sets parameters of stage B)
            callback_join(dec);
            set_conceal_method(dec, 1);
        }
        let frames = metadata_expiry_frames(dec, dec.metadata_expiry);
        if dec.pipe.as_deref().is_some_and(|p| p.away()) {
            pipe(dec).b_ops.push(BOp::MetadataExpiry(frames));
        } else {
            set_metadata_expiry_frames(dec, frames);
        }
        TRANSPORTDEC_OK
    } else if err == AAC_DEC_NEED_TO_RESTART {
        TRANSPORTDEC_NEED_TO_RESTART
    } else if is_init_error(err) != 0 {
        TRANSPORTDEC_UNSUPPORTED_FORMAT
    } else {
        TRANSPORTDEC_UNKOWN_ERROR
    }
}

/// Whether stage B of the frame can run on the worker thread (otherwise it runs in the calling
/// thread): a frame without errors (`frameOK`, valid output), without configuration callbacks
/// and PCE, and without DRC payloads (the DRC prolog and epilog of stage B read them from the
/// transport decoder's bitstream). After the join ([`can_defer_a`] is the part stage A knows
/// before it).
pub(crate) fn can_defer(self_: &AacDecoderInstance, ho: &HandOver) -> bool {
    let drc_payloads = self_
        .h_drc_info
        .as_deref()
        .is_some_and(|d| d.num_payloads != 0 || d.dvb_anc_data_available != 0);
    can_defer_a(self_, ho) && !drc_payloads && self_.p_time_data2.is_some()
}

/// The conditions of [`can_defer`] on the state of stage A.
///
/// The others follow from them for a frame whose stage A ran while stage B of the previous
/// frame was running and did not wait for it (the frames [`restart`] does not redo): the DRC
/// payloads are marked by stage A only after a join (DRC extension payloads, data stream
/// elements), stage B only clears them, and they were cleared when the previous frame was
/// handed over; `pTimeData2` is freed only by a configuration change (after a join).
pub(crate) fn can_defer_a(self_: &AacDecoderInstance, ho: &HandOver) -> bool {
    let Some(p) = self_.pipe.as_deref() else { return false };
    p.threads > 1
        && !p.cb_join
        && !p.pce_in_frame
        && self_.frame_ok != 0
        && ho.core_err == 0
        && ho.flags == 0
        && ho.call_flags == 0
        && self_.work_buffer_core2.is_some()
}

/// The transport decoder state that stage A changes after `transportDec_ReadAccessUnit()`
/// (the bitstream position, the CRC regions, the access unit counters of
/// `transportDec_EndAccessUnit()`); its configuration (`asc`) and its bit buffer content do
/// not change.
#[derive(Clone)]
struct TpSnapshot {
    bit_stream: [fdk_aac_rs_fdk::bitstream::FdkBitStream; 1],
    parser: fdk_aac_rs_tpdec::tpdec_lib::TransportDecParser,
    ctrl_cfg_change: [fdk_aac_rs_tpdec::tp_data::CCtrlCFGChange; 1],
    global_frame_pos: u32,
    access_unit_anchor: [u32; 1],
    au_length: [i32; 1],
    number_of_raw_data_blocks: i32,
    avg_bit_rate: u32,
    last_valid_buffer_fullness: u32,
    remainder: i32,
    missing_access_units: i32,
    burst_period: u32,
    hold_off_frames: u32,
    flags: u32,
    target_layout: i32,
}

impl TpSnapshot {
    fn take(t: &fdk_aac_rs_tpdec::tpdec_lib::TransportDec) -> TpSnapshot {
        TpSnapshot {
            bit_stream: t.bit_stream.clone(),
            parser: t.parser,
            ctrl_cfg_change: t.ctrl_cfg_change,
            global_frame_pos: t.global_frame_pos,
            access_unit_anchor: t.access_unit_anchor,
            au_length: t.au_length,
            number_of_raw_data_blocks: t.number_of_raw_data_blocks,
            avg_bit_rate: t.avg_bit_rate,
            last_valid_buffer_fullness: t.last_valid_buffer_fullness,
            remainder: t.remainder,
            missing_access_units: t.missing_access_units,
            burst_period: t.burst_period,
            hold_off_frames: t.hold_off_frames,
            flags: t.flags,
            target_layout: t.target_layout,
        }
    }

    fn restore(&self, t: &mut fdk_aac_rs_tpdec::tpdec_lib::TransportDec) {
        t.bit_stream = self.bit_stream.clone();
        t.parser = self.parser;
        t.ctrl_cfg_change = self.ctrl_cfg_change;
        t.global_frame_pos = self.global_frame_pos;
        t.access_unit_anchor = self.access_unit_anchor;
        t.au_length = self.au_length;
        t.number_of_raw_data_blocks = self.number_of_raw_data_blocks;
        t.avg_bit_rate = self.avg_bit_rate;
        t.last_valid_buffer_fullness = self.last_valid_buffer_fullness;
        t.remainder = self.remainder;
        t.missing_access_units = self.missing_access_units;
        t.burst_period = self.burst_period;
        t.hold_off_frames = self.hold_off_frames;
        t.flags = self.flags;
        t.target_layout = self.target_layout;
    }
}

/// The state of a call after the transport decoder's part, while stage B of the previous frame
/// runs ([`take_snapshot`], [`restart`]).
pub(crate) struct Snapshot {
    /// The members of the decoder instance that [`copy_scalars`] copies.
    inst: Box<AacDecoderInstance>,
    tp: Option<TpSnapshot>,
    anc_offset: [i32; 8],
    anc_nr_elements: i32,
    /// The lengths of `b_ops` and `b_writes` of the pipeline.
    b_ops: usize,
    b_writes: usize,
    t_mask: u8,
    t_prev_stored: bool,
    t_prev_mask: u8,
    cb_join: bool,
    pce_in_frame: bool,
}

/// Takes the snapshot of the state after the transport decoder's part of a call whose stage A
/// runs while stage B of the previous frame runs (no snapshot otherwise).
///
/// Such a stage A writes the per frame channel memory the main instance got at the last
/// hand-over, which holds what the frame before the previous one left (the sequential decoder
/// has what the previous frame left). A frame that is handed over reads only what its stage A
/// wrote (checked by poisoning, [`poison_stage_a`]); a frame with an error, a concealed frame,
/// or a frame that needs state of stage B in its stage A may read what an earlier frame left
/// (the window sequence of a channel that a defective frame did not read, for example). Such a
/// frame is decoded again from this snapshot by the sequential decoder on the set of the
/// previous frame ([`restart`]).
pub(crate) fn take_snapshot(self_: &mut AacDecoderInstance) {
    let Some(p) = self_.pipe.as_deref_mut() else { return };
    if !p.away() {
        return;
    }
    let mut snap = p.snap.take().unwrap_or_else(|| {
        Box::new(Snapshot {
            inst: AacDecoderInstance::zeroed(),
            tp: None,
            anc_offset: [0; 8],
            anc_nr_elements: 0,
            b_ops: 0,
            b_writes: 0,
            t_mask: 0,
            t_prev_stored: false,
            t_prev_mask: 0,
            cb_join: false,
            pce_in_frame: false,
        })
    });
    snap.b_ops = p.b_ops.len();
    snap.b_writes = p.b_writes.len();
    snap.t_mask = p.t_mask;
    snap.t_prev_stored = p.t_prev_stored;
    snap.t_prev_mask = p.t_prev_mask;
    snap.cb_join = p.cb_join;
    snap.pce_in_frame = p.pce_in_frame;
    copy_scalars(&mut snap.inst, self_);
    snap.tp = self_.h_input.as_deref().map(TpSnapshot::take);
    snap.anc_offset = self_.anc_data.offset;
    snap.anc_nr_elements = self_.anc_data.nr_elements;
    pipe(self_).snap = Some(snap);
}

/// Stage A from here on writes the per frame channel memory (if stage B of the previous frame
/// runs: see [`take_snapshot`]).
pub(crate) fn begin_core_a(self_: &mut AacDecoderInstance) {
    let Some(p) = self_.pipe.as_deref_mut() else { return };
    if p.away() {
        debug_assert!(p.snap.is_some());
        p.touched = true;
    }
}

/// Returns the decoder to the state of [`take_snapshot`] (with stage B of the previous frame
/// joined and the per frame channel memory of the previous frame): what stage A did after the
/// snapshot is undone. The operations and writes stage A recorded for stage B before the
/// snapshot (in configuration callbacks) are kept and executed at the join.
pub(crate) fn restart(self_: &mut AacDecoderInstance) {
    let p = pipe(self_);
    debug_assert!(p.touched && p.away());
    let snap = p.snap.take().expect("snapshot of a restarted frame");
    let keep = snap.b_ops.min(p.b_ops.len());
    let dropped: Vec<BOp> = p.b_ops.drain(keep..).collect();
    for op in dropped {
        if let BOp::Parse(mut sp) = op {
            p.sbr_pool.push(core::mem::take(&mut sp.data));
        }
    }
    p.b_writes.truncate(snap.b_writes);
    p.t_mask = snap.t_mask;
    p.t_prev_stored = snap.t_prev_stored;
    p.t_prev_mask = snap.t_prev_mask;
    p.cb_join = snap.cb_join;
    p.pce_in_frame = snap.pce_in_frame;
    p.touched = false;
    p.restart = false;
    p.restarts += 1;
    p.backoff = (super::BACKOFF_MIN << p.fail_streak.min(16)).min(super::BACKOFF_MAX);
    p.fail_streak = p.fail_streak.saturating_add(1);
    copy_scalars(self_, &snap.inst);
    if let (Some(t), Some(s)) = (self_.h_input.as_deref_mut(), snap.tp.as_ref()) {
        s.restore(t);
    }
    self_.anc_data.offset = snap.anc_offset;
    self_.anc_data.nr_elements = snap.anc_nr_elements;
    pipe(self_).snap = Some(snap);
    // (gives the main instance the per frame channel memory of the previous frame back)
    super::join(self_);
    flush_b_ops(self_);
}

/// Exchanges the per frame channel memory of two instances.
pub(crate) fn swap_channel_memory(a: &mut AacDecoderInstance, b: &mut AacDecoderInstance) {
    core::mem::swap(&mut a.p_aac_decoder_channel_info, &mut b.p_aac_decoder_channel_info);
    core::mem::swap(&mut a.com_data, &mut b.com_data);
    core::mem::swap(&mut a.work_buffer_core2, &mut b.work_buffer_core2);
}

/// Moves the members owned by stage B from `src` to `dst` (`src` keeps empty values).
fn move_b_owned(dst: &mut AacDecoderInstance, src: &mut AacDecoderInstance) {
    core::mem::swap(
        &mut dst.p_aac_decoder_static_channel_info,
        &mut src.p_aac_decoder_static_channel_info,
    );
    core::mem::swap(&mut dst.cpe_static_data, &mut src.cpe_static_data);
    core::mem::swap(&mut dst.com_static_data, &mut src.com_static_data);
    core::mem::swap(&mut dst.p_work_buffer_core1, &mut src.p_work_buffer_core1);
    core::mem::swap(&mut dst.work_buffer_core1, &mut src.work_buffer_core1);
    core::mem::swap(&mut dst.p_time_data2, &mut src.p_time_data2);
    core::mem::swap(&mut dst.h_sbr_decoder, &mut src.h_sbr_decoder);
    core::mem::swap(&mut dst.qmf_domain, &mut src.qmf_domain);
    core::mem::swap(&mut dst.h_drc_info, &mut src.h_drc_info);
    core::mem::swap(&mut dst.p_mpeg_surround_decoder, &mut src.p_mpeg_surround_decoder);
    core::mem::swap(&mut dst.h_pcm_utils, &mut src.h_pcm_utils);
    core::mem::swap(&mut dst.h_limiter, &mut src.h_limiter);
    core::mem::swap(&mut dst.h_uni_drc_decoder, &mut src.h_uni_drc_decoder);
    core::mem::swap(&mut dst.p_time_data_flush, &mut src.p_time_data_flush);
    core::mem::swap(&mut dst.usac_residual_delay, &mut src.usac_residual_delay);
    core::mem::swap(&mut dst.chan_par, &mut src.chan_par);
}

/// Copies every member of `src` that is not owned by a stage or per frame channel memory into
/// `dst` (the state stage B of the frame reads, as stage A left it). The destructuring lists
/// every member, so that a new member must be classified here.
fn copy_scalars(dst: &mut AacDecoderInstance, src: &AacDecoderInstance) {
    let AacDecoderInstance {
        aac_channels,
        asc_channels,
        block_number,
        nr_of_layers,
        output_interleaved,
        aac_out_data_headroom,
        h_input: _,
        sampling_rate_info,
        frame_ok,
        flags,
        el_flags,
        elements,
        el_tags,
        ch_mapping,
        map_descr,
        ch_map_index,
        sbr_data_len,
        pce,
        stream_info,
        p_aac_decoder_channel_info: _,
        p_aac_decoder_static_channel_info: _,
        p_com_data,
        p_com_static_data,
        com_data: _,
        com_static_data: _,
        p_work_buffer_core1: _,
        work_buffer_core1: _,
        work_buffer_core2: _,
        p_time_data2: _,
        time_data2_size,
        cpe_static_data: _,
        conceal_common_data,
        conceal_method_user,
        usac_core_ext,
        num_usac_elements,
        usac_stereo_config_index,
        p_usac_config,
        nb_div,
        use_ld_qmf_time_align,
        aac_channels_prev,
        channel_type_prev,
        channel_indices_prev,
        downscale_factor,
        downscale_factor_in_bs,
        h_sbr_decoder: _,
        sbr_enabled,
        sbr_enabled_prev,
        ps_possible,
        sbr_params,
        p_drm_bs_buffer,
        drm_bs_buffer_size,
        qmf_domain: _,
        qmf_mode_curr,
        qmf_mode_user,
        h_drc_info: _,
        metadata_expiry,
        p_mpeg_surround_decoder: _,
        mps_enable_user,
        mps_enable_curr,
        mps_applicable,
        mps_output_mode,
        mps_out_channels_last,
        mps_frame_size_last,
        anc_data: _,
        h_pcm_utils: _,
        h_limiter: _,
        limiter_enable_user,
        limiter_enable_curr,
        ext_gain,
        ext_gain_delay,
        h_uni_drc_decoder: _,
        multiband_drc_present,
        num_time_slots,
        loudness_info_set_position,
        default_target_loudness,
        p_time_data_flush: _,
        flush_status,
        flush_cnt,
        build_up_status,
        build_up_cnt,
        has_audio_pre_roll,
        preroll_au_length,
        access_unit,
        apply_crossfade,
        usac_residual_delay: _,
        pipe: _,
        chan_par: _,
        d_gen,
    } = src;
    dst.aac_channels = *aac_channels;
    dst.asc_channels = *asc_channels;
    dst.block_number = *block_number;
    dst.nr_of_layers = *nr_of_layers;
    dst.output_interleaved = *output_interleaved;
    dst.aac_out_data_headroom = *aac_out_data_headroom;
    dst.sampling_rate_info = *sampling_rate_info;
    dst.frame_ok = *frame_ok;
    dst.flags = *flags;
    dst.el_flags = *el_flags;
    dst.elements = *elements;
    dst.el_tags = *el_tags;
    dst.ch_mapping = *ch_mapping;
    dst.map_descr = *map_descr;
    dst.ch_map_index = *ch_map_index;
    dst.sbr_data_len = *sbr_data_len;
    dst.pce = *pce;
    dst.stream_info = *stream_info;
    dst.p_com_data = *p_com_data;
    dst.p_com_static_data = *p_com_static_data;
    dst.time_data2_size = *time_data2_size;
    dst.conceal_common_data.clone_from(conceal_common_data);
    dst.conceal_method_user = *conceal_method_user;
    dst.usac_core_ext = *usac_core_ext;
    dst.num_usac_elements = *num_usac_elements;
    dst.usac_stereo_config_index = *usac_stereo_config_index;
    dst.p_usac_config = *p_usac_config;
    dst.nb_div = *nb_div;
    dst.use_ld_qmf_time_align = *use_ld_qmf_time_align;
    dst.aac_channels_prev = *aac_channels_prev;
    dst.channel_type_prev = *channel_type_prev;
    dst.channel_indices_prev = *channel_indices_prev;
    dst.downscale_factor = *downscale_factor;
    dst.downscale_factor_in_bs = *downscale_factor_in_bs;
    dst.sbr_enabled = *sbr_enabled;
    dst.sbr_enabled_prev = *sbr_enabled_prev;
    dst.ps_possible = *ps_possible;
    dst.sbr_params = *sbr_params;
    dst.p_drm_bs_buffer = *p_drm_bs_buffer;
    dst.drm_bs_buffer_size = *drm_bs_buffer_size;
    dst.qmf_mode_curr = *qmf_mode_curr;
    dst.qmf_mode_user = *qmf_mode_user;
    dst.metadata_expiry = *metadata_expiry;
    dst.mps_enable_user = *mps_enable_user;
    dst.mps_enable_curr = *mps_enable_curr;
    dst.mps_applicable = *mps_applicable;
    dst.mps_output_mode = *mps_output_mode;
    dst.mps_out_channels_last = *mps_out_channels_last;
    dst.mps_frame_size_last = *mps_frame_size_last;
    dst.limiter_enable_user = *limiter_enable_user;
    dst.limiter_enable_curr = *limiter_enable_curr;
    dst.ext_gain = *ext_gain;
    dst.ext_gain_delay = *ext_gain_delay;
    dst.multiband_drc_present = *multiband_drc_present;
    dst.num_time_slots = *num_time_slots;
    dst.loudness_info_set_position = *loudness_info_set_position;
    dst.default_target_loudness = *default_target_loudness;
    dst.flush_status = *flush_status;
    dst.flush_cnt = *flush_cnt;
    dst.build_up_status = *build_up_status;
    dst.build_up_cnt = *build_up_cnt;
    dst.has_audio_pre_roll = *has_audio_pre_roll;
    dst.preroll_au_length = *preroll_au_length;
    dst.access_unit = *access_unit;
    dst.apply_crossfade = *apply_crossfade;
    dst.d_gen = *d_gen;
}

/// The `flags` bits stage B writes.
pub(crate) const B_FLAGS: u32 = AC_SBR_PRESENT | AC_PS_PRESENT | AC_DRC_PRESENT;

/// Copies the state stage B wrote from the shadow back into the main instance (see the module
/// documentation), with the merge of the channel tables.
fn merge_b_scalars(main: &mut AacDecoderInstance, shadow: &AacDecoderInstance, t_mask: u8) {
    let s = &shadow.stream_info;
    let m = &mut main.stream_info;
    m.sample_rate = s.sample_rate;
    m.frame_size = s.frame_size;
    m.num_channels = s.num_channels;
    m.output_delay = s.output_delay;
    m.flags = s.flags;
    m.bit_rate = s.bit_rate;
    m.num_total_bytes = s.num_total_bytes;
    m.num_bad_bytes = s.num_bad_bytes;
    m.num_total_access_units = s.num_total_access_units;
    m.num_bad_access_units = s.num_bad_access_units;
    m.drc_prog_ref_lev = s.drc_prog_ref_lev;
    m.drc_pres_mode = s.drc_pres_mode;
    m.output_loudness = s.output_loudness;
    for i in 0..8 {
        if t_mask & (1 << i) == 0 {
            m.p_channel_type[i] = s.p_channel_type[i];
            m.p_channel_indices[i] = s.p_channel_indices[i];
        }
    }
    main.flags[0] = (main.flags[0] & !B_FLAGS) | (shadow.flags[0] & B_FLAGS);
    main.ps_possible = shadow.ps_possible;
    main.qmf_mode_curr = shadow.qmf_mode_curr;
    main.mps_enable_curr = shadow.mps_enable_curr;
    main.mps_out_channels_last = shadow.mps_out_channels_last;
    main.mps_frame_size_last = shadow.mps_frame_size_last;
    main.ext_gain = shadow.ext_gain;
    main.ext_gain_delay = shadow.ext_gain_delay;
    main.apply_crossfade = shadow.apply_crossfade;
}

/// Hands stage B of the frame over to the worker thread: the shadow instance gets a copy of
/// the state, the members owned by stage B and the per frame channel memory stage A filled;
/// the main instance gets the other set of per frame channel memory.
pub(crate) fn hand_over(
    self_: &mut AacDecoderInstance,
    mut ho: Box<HandOver>,
    out: Vec<IntPcm>,
    anc: Option<AncSnapshot>,
) {
    let cache = SbrCache::capture(self_.h_sbr_decoder.as_deref());
    let d_gen = self_.d_gen;
    let p = pipe(self_);
    p.sbr_cache = cache;
    ho.b_ops.append(&mut p.b_ops);
    let mut shadow = p.shadow.take().unwrap_or_else(AacDecoderInstance::zeroed);
    let spare_valid = p.spare_gen == Some(d_gen);

    copy_scalars(&mut shadow, self_);
    move_b_owned(&mut shadow, self_);
    if !spare_valid {
        // A second set of per frame channel memory like the first one.
        shadow.p_aac_decoder_channel_info = self_.p_aac_decoder_channel_info.clone();
        shadow.com_data = self_.com_data.clone();
        shadow.work_buffer_core2 = self_.work_buffer_core2.clone();
    }
    swap_channel_memory(&mut shadow, self_);

    let p = pipe(self_);
    p.spare_gen = Some(d_gen);
    p.anc_in_flight = anc;
    p.frames_deferred += 1;
    if p.poison & 2 != 0 {
        poison_b_scalars(self_);
    }
    let p = pipe(self_);
    // (stage A of the next frame runs in the calling thread meanwhile)
    let cap = p.worker_cap as usize;
    let w = p.worker.get_or_insert_with(Worker::new);
    w.post(Job { shadow, ho, out, cap });
    p.queue.push_back(Pending::InFlight);
}

/// Takes the shadow instance back after its stage B: the members owned by stage B return to
/// the main instance, and the state stage B wrote is merged into it.
pub(crate) fn return_from_stage_b(self_: &mut AacDecoderInstance, shadow: &mut AacDecoderInstance) {
    let (t_mask, t_prev_stored, t_prev_mask) = {
        let p = pipe(self_);
        (p.t_mask, p.t_prev_stored, p.t_prev_mask)
    };
    move_b_owned(self_, shadow);
    merge_b_scalars(self_, shadow, t_mask);
    if t_prev_stored {
        for i in 0..8 {
            if t_prev_mask & (1 << i) == 0 {
                self_.channel_type_prev[i] = shadow.stream_info.p_channel_type[i];
                self_.channel_indices_prev[i] = shadow.stream_info.p_channel_indices[i];
            }
        }
    }
    // The writes of stage A to that state come after it.
    let writes = core::mem::take(&mut pipe(self_).b_writes);
    for w in writes {
        match w {
            BWrite::BitRate(v) => self_.stream_info.bit_rate = v,
            BWrite::SiFlags(v) => self_.stream_info.flags = v,
            BWrite::FlagsB(v) => self_.flags[0] = (self_.flags[0] & !B_FLAGS) | (v & B_FLAGS),
            BWrite::MpsEnableCurr(v) => self_.mps_enable_curr = v,
            BWrite::Counters(n, err) => {
                super::stage_b::update_counters(&mut self_.stream_info, n, err);
                // (the consistency check of IS_OUTPUT_VALID() does not apply: no valid output)
                debug_assert!(crate::aacdecoder_lib::is_output_valid(err) == 0);
            }
        }
    }
    let p = pipe(self_);
    // From here on stage A writes the complete state.
    p.t_mask = 0;
    p.t_prev_stored = false;
    p.t_prev_mask = 0;
}

/// Executes the SBR operations stage A recorded, on the main instance (after a join).
pub(crate) fn flush_b_ops(self_: &mut AacDecoderInstance) {
    let ops = core::mem::take(&mut pipe(self_).b_ops);
    for op in ops {
        exec_b_op(self_, op);
    }
}

/// Executes a recorded SBR operation of stage A.
pub(crate) fn exec_b_op(inst: &mut AacDecoderInstance, op: BOp) {
    match op {
        BOp::SyncQmfMode { flags, aac_num_channels } => {
            // With the decoder flags and channel count of stage A at the call.
            let (f, n) = (inst.flags[0], inst.stream_info.aac_num_channels);
            inst.flags[0] = flags;
            inst.stream_info.aac_num_channels = aac_num_channels;
            caac_decoder_sync_qmf_mode(inst);
            // (CAacDecoder_SyncQmfMode() does not write these two)
            inst.flags[0] = f;
            inst.stream_info.aac_num_channels = n;
        }
        BOp::MetadataExpiry(frames) => {
            crate::aacdecoder_lib::set_metadata_expiry_frames(inst, frames);
        }
        BOp::Parse(mut p) => {
            let Some(sbr) = inst.h_sbr_decoder.as_deref_mut() else {
                return;
            };
            let mut bs = payload_bitstream(&p);
            let start = bs_pos(&bs);
            let mut count = p.count;
            let _ = sbr_decoder_parse(
                sbr,
                &mut bs,
                &mut [],
                &mut count,
                p.bs_pay_len,
                p.crc_flag,
                p.prev_element,
                p.element_index,
                p.ac_flags,
                &inst.el_flags,
            );
            // The parser must not have read beyond the copied bits (see SBR_COPY_MARGIN).
            let read = bs_pos(&bs).wrapping_sub(start) as usize;
            debug_assert!(read <= p.data_len * 8, "SBR payload copy too short");
            if read > p.data_len * 8 {
                super::note_mismatch();
            }
            if let Some(pp) = inst.pipe.as_deref_mut() {
                pp.sbr_pool.push(core::mem::take(&mut p.data));
            } else {
                RETURNED_COPY.with(|c| *c.borrow_mut() = Some(core::mem::take(&mut p.data)));
            }
        }
    }
}

std::thread_local! {
    /// (The copies of payloads parsed on the worker thread are dropped.)
    static RETURNED_COPY: core::cell::RefCell<Option<Vec<u8>>> = const { core::cell::RefCell::new(None) };
}

/// The bit position of a bitstream (bits read since its start, modulo the buffer).
fn bs_pos(bs: &fdk_aac_rs_fdk::bitstream::FdkBitStream) -> u32 {
    bs.h_bit_buf.bit_ndx.wrapping_sub(bs.bits_in_cache)
}

/// Buffer poisoning of a test build (`FDK_AAC_RS_POISON`, bit 1): when stage A of a frame
/// starts to write the per frame channel memory the main instance got at the last hand-over
/// (the set stage B of the frame before the previous one used; [`begin_core_a`]), that memory
/// is filled with a pattern, except the members set at initialization (`granuleLength`, the
/// PNS seed link) and `renderMode` (written with the value of the configuration in every
/// frame). If no frame that is handed over reads what an earlier frame left there, the output
/// does not change (the other frames are decoded again on the set of the previous frame,
/// [`restart`]).
pub(crate) fn poison_stage_a(self_: &mut AacDecoderInstance) {
    let Some(p) = self_.pipe.as_deref() else { return };
    if p.poison & 1 == 0 || !p.touched {
        return;
    }
    let seed = POISON_SEED.with(|s| {
        let v = s.get().wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        s.set(v);
        v
    });
    poison_channel_memory(self_, seed);
}

std::thread_local! {
    static POISON_SEED: core::cell::Cell<u64> = const { core::cell::Cell::new(0x9e37_79b9_7f4a_7c15) };
}

/// Fills the per frame channel memory of `inst` with a pattern derived from `seed`.
pub(crate) fn poison_channel_memory(inst: &mut AacDecoderInstance, seed: u64) {
    let mut x = seed | 1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    if let Some(wb2) = inst.work_buffer_core2.as_deref_mut() {
        for v in wb2.iter_mut() {
            *v = next() as i32;
        }
    }
    for c in inst.p_aac_decoder_channel_info.iter_mut().flatten() {
        poison_channel_info(c, &mut next);
    }
    for cd in inst.com_data.iter_mut() {
        poison_ints(&mut cd.pns_inter_channel_data.correlated, &mut next);
        poison_ints(&mut cd.pns_random_seed, &mut next);
        let js = &mut cd.joint_stereo_data;
        js.ms_mask_present = next() as u8;
        poison_ints(&mut js.ms_used, &mut next);
        js.igf_ms_mask_present = next() as u8;
        js.cplx_pred_flag = next() as u8;
        js.igf_cplx_pred_flag = next() as u8;
        poison_ints(&mut cd.overlay.aac.a_rvlc_scf_esc, &mut next);
        poison_ints(&mut cd.overlay.aac.a_rvlc_scf_fwd, &mut next);
        poison_ints(&mut cd.overlay.aac.a_rvlc_scf_bwd, &mut next);
    }
}

/// Fills an array of integers with a pattern.
fn poison_ints<T: PoisonInt>(a: &mut [T], next: &mut impl FnMut() -> u64) {
    for v in a.iter_mut() {
        *v = T::from_bits(next());
    }
}

/// Integers that [`poison_ints`] fills.
trait PoisonInt: Copy {
    fn from_bits(x: u64) -> Self;
}
macro_rules! poison_int {
    ($($t:ty),*) => {$(impl PoisonInt for $t { fn from_bits(x: u64) -> Self { x as $t } })*};
}
poison_int!(u8, i8, i16, u16, i32, u32);

/// Fills the per frame members of a channel info with a pattern: all except `granuleLength`
/// (set at initialization), the link of the PNS seed (set at initialization) and `renderMode`
/// (written with the value of the configuration in every frame: `AACDEC_RENDER_IMDCT`, or
/// `AACDEC_RENDER_ELDFB` for ELD).
fn poison_channel_info(c: &mut CAacDecoderChannelInfo, next: &mut impl FnMut() -> u64) {
    poison_ints(&mut c.spec_scale, next);
    let ics = &mut c.ics_info;
    poison_ints(&mut ics.window_group_length, next);
    ics.window_groups = next() as u8;
    ics.valid = next() as u8;
    ics.window_shape = next() as u8;
    ics.window_sequence = next() as i32;
    ics.max_sf_bands = next() as u8;
    ics.max_sfb_ste = next() as u8;
    ics.scale_factor_grouping = next() as u8;
    ics.total_sf_bands = next() as u8;
    c.element_instance_tag = next() as u8;
    let pns = &mut c.data.aac.pns_data;
    poison_ints(&mut pns.pns_used, next);
    pns.current_energy = next() as i32;
    pns.pns_active = next() as u8;
    let d = &mut *c.p_dyn_data;
    poison_ints(&mut d.a_scale_factor, next);
    poison_ints(&mut d.a_sfb_scale, next);
    poison_ints(&mut d.a_code_book, next);
    poison_ints(&mut d.band_is_noise, next);
    for w in d.tns_data.filter.iter_mut() {
        for f in w.iter_mut() {
            poison_ints(&mut f.coeff, next);
            f.start_band = next() as u8;
            f.stop_band = next() as u8;
            f.direction = next() as i8;
            f.resolution = next() as i8;
            f.order = next() as u8;
        }
    }
    poison_ints(&mut d.tns_data.number_of_filters, next);
    d.tns_data.data_present = next() as u8;
    d.tns_data.active = next() as u8;
    d.tns_data.gain_ld = next() as u8;
    d.raw_data_info.common_window = next() as u8;
    d.raw_data_info.global_gain = next() as u8;
    let a = &mut d.specific_to.aac;
    a.pulse_data.pulse_data_present = next() as u8;
    a.pulse_data.number_pulse = next() as u8;
    a.pulse_data.pulse_start_band = next() as u8;
    poison_ints(&mut a.pulse_data.pulse_offset, next);
    poison_ints(&mut a.pulse_data.pulse_amp, next);
    poison_ints(&mut a.a_num_line_in_sec4_hcr, next);
    poison_ints(&mut a.a_code_books4_hcr, next);
    a.len_of_reordered_spectral_data = next() as i16;
    a.len_of_longest_codeword = next() as i8;
    a.number_section = next() as i8;
    a.rvlc_current_scale_factor_ok = next() as i8;
    a.rvlc_intensity_used = next() as i8;
}

/// Poisons the state of stage B that the main instance still holds while stage B runs (bit 2
/// of `FDK_AAC_RS_POISON`): stage A must not read it; the join replaces it.
fn poison_b_scalars(self_: &mut AacDecoderInstance) {
    let si: &mut CStreamInfo = &mut self_.stream_info;
    si.sample_rate = 0x7fff_0101;
    si.frame_size = 0x7fff_0202;
    si.num_channels = 0x7fff_0303;
    si.output_delay = 0x7fff_0404;
    si.flags = 0x7fff_0505;
    si.bit_rate = 0x7fff_0606;
    si.num_total_bytes = 0x7fff_0707;
    si.num_bad_bytes = 0x7fff_0808;
    si.num_total_access_units = 0x7fff_0909;
    si.num_bad_access_units = 0x7fff_0a0a;
    si.drc_prog_ref_lev = 0x55;
    si.drc_pres_mode = 0x56;
    si.output_loudness = 0x7fff_0b0b;
    si.p_channel_type = [0x7fff_0c0c; 8];
    si.p_channel_indices = [0xcc; 8];
    self_.ps_possible = 0xa5;
    self_.qmf_mode_curr = 0x7fff_0d0d;
    self_.mps_enable_curr = 0xa6;
    self_.mps_out_channels_last = 0x7fff_0e0e;
    self_.mps_frame_size_last = 0x7fff_0f0f;
    self_.ext_gain = [0x7fff_1010];
    self_.ext_gain_delay = 0x7fff_1111;
}

#[allow(unused_imports)]
use AncSnapshot as _;

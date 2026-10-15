// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Rearranged copy of parts of libAACdec/src/aacdecoder_lib.cpp (aacDecoder_DecodeFrame() up to
// transportDec_EndAccessUnit()) and libAACdec/src/aacdecoder.cpp (CAacDecoder_DecodeFrame()
// without the element decode loop and the inverse transform, CAacDecoder_ExtPayloadParse()).

//! Stage A of the frame pipeline: reading and parsing an access unit ([`super`]).
//!
//! The code is that of the sequential decoder for a call without flags, of one access unit
//! without pre-roll, in a configuration that [`super::state::eligible_before_a`] accepts (no
//! USAC, no DRM, no RVLC). While stage B of the previous frame runs, the state it owns is not
//! in the decoder instance ([`super::state`]); stage A reaches it as follows:
//!
//! - Configuration callbacks of the transport decoder, the signalling of an interruption,
//!   coupling channel elements, data stream elements (ancillary data, DVB DRC and downmix
//!   metadata), program config elements, MPEG-4 DRC payloads, MPEG Surround payloads and the
//!   explicit SBR / MPEG Surround payloads of ELD need stage B ([`join_mid_a`]). Before the
//!   element loop (the configuration callbacks, the interruption), stage A waits for it and
//!   the rest of the frame runs as in the sequential decoder; in the element loop, the frame
//!   is decoded again by the sequential decoder from the state after the transport decoder's
//!   part ([`state::take_snapshot`], [`state::restart`]), as is every frame whose stage A ran
//!   while stage B of the previous frame was running and that is not handed over to the
//!   worker thread (after a restart, the sequential decoder decodes the next calls,
//!   [`super::BACKOFF_MIN`]).
//! - `sbrDecoder_InitElement()` is not called when its result is known: it returns at once
//!   with `SBRDEC_OK` when the configuration of the element did not change ([`SbrCache`]).
//!   Otherwise stage A waits for stage B and calls it.
//! - `CAacDecoder_SyncQmfMode()` and `sbrDecoder_Parse()` of an SBR fill element are recorded
//!   and executed by stage B of the same frame before anything else ([`super::BOp`]). Stage A
//!   needs only whether `sbrDecoder_Parse()` returns `SBRDEC_NOT_INITIALIZED` (no SBR element
//!   at that index), which [`SbrCache`] knows; the bitstream position after a fill element
//!   does not depend on the parser. The parser reads a copy of the bits of the transport
//!   decoder's bit buffer.
//! - `streamInfo.pChannelType` / `pChannelIndices` (and `channelTypePrev`,
//!   `channelIndicesPrev`) are written by stage A as in C; the entries stage A writes are
//!   recorded, and the others are taken from stage B of the previous frame when it is joined
//!   ([`super::state`]). If a frame reads entries it did not write (the reordering of the
//!   channel tables of a frame that did not write all its channels), stage A waits for stage B
//!   first.

use super::{state, BOp, HandOver, Pipeline, SbrParse, SBR_COPY_MARGIN};
use crate::aac_rom::PcmDec;
use crate::aacdec_drc::{aac_decoder_drc_mark_payload, MPEG_DRC_EXT_DATA};
use crate::aacdec_pns::cpns_reset_data;
use crate::aacdecoder::{
    aac_decoder_parse_explicit_mps_and_sbr, caac_decoder_anc_data_parse,
    caac_decoder_anc_data_reset, caac_decoder_get_el_channels, caac_decoder_sync_qmf_mode,
    cdata_stream_element_read, channels_mut, cprogram_config_element_read, element_com_data,
    mps_is_full_instance_available, nz, read_coupling_channel_element, sbr_init_element, sbr_parse,
    tail_mut, validate_element_instance_tags, AacDecoderInstance, MAX_ELEMENTS, NCH,
};
use crate::aacdecoder_lib::{
    aac_decoder_signal_interruption, decode_frame_lib_after_read, is_output_valid,
    lib_read_access_unit, lib_tail, AACDEC_CONCEAL, AACDEC_FLUSH, AAC_DEC_DECODE_FRAME_ERROR,
    AAC_DEC_OK, AAC_DEC_OUT_OF_MEMORY, AAC_DEC_PARSE_ERROR, AAC_DEC_TRANSPORT_SYNC_ERROR,
    AAC_DEC_UNKNOWN, AAC_DEC_UNSUPPORTED_CHANNELCONFIG, AAC_DEC_UNSUPPORTED_SAMPLINGRATE,
};
use crate::channel::{c_channel_element_inverse_quantize, c_channel_element_read_deferred_iq};
use crate::channelinfo::*;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_get_valid_bits, fdk_init_bit_stream, fdk_push_bi_directional, fdk_push_for,
    fdk_read_bit, fdk_read_bits, FdkBitStream, BS_READER,
};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::syslib_channel_map_descr::fdk_ch_map_descr_get_map_value;
use fdk_aac_rs_pcmutils::pcmdmx_lib::pcm_dmx_set_matrix_mixdown_from_pce;
use fdk_aac_rs_sacdec::sac_dec_errorcodes::MPS_OK;
use fdk_aac_rs_sacdec::sac_dec_lib::{
    mpeg_surround_decoder_open, mpeg_surround_decoder_parse, SAC_INSTANCE_NOT_FULL_AVAILABLE,
};
use fdk_aac_rs_sbrdec::env_extr::SBRDEC_FORCE_RESET;
use fdk_aac_rs_sbrdec::sbrdecoder::{
    SbrDecoderInstance, SbrError, SBRDEC_NOT_INITIALIZED, SBRDEC_OK, SBRDEC_PARSE_ERROR,
    SBRDEC_UNSUPPORTED_CONFIG,
};
use fdk_aac_rs_tpdec::tpdec_lib::{
    cprogram_config_get_element_table, cprogram_config_get_pce_ch_map, cprogram_config_init,
    cprogram_config_is_valid, cprogram_config_lookup_element, cprogram_config_reset,
    transport_dec_crc_check, transport_dec_end_access_unit, transport_dec_get_au_bits_remaining,
    transport_dec_get_au_bits_total, transport_dec_get_format, TRANSPORTDEC_OK,
    TRANSPORTDEC_UNKOWN_ERROR,
};

/// `AAC_DECODER_ERROR`
type AacDecoderError = i32;

/// The outcome of stage A of a call.
pub(crate) enum AOutcome {
    /// The call ended with this return value without a stage B (no valid output, or decoded
    /// by the sequential decoder); the pipeline was joined, the decoder state is that of the
    /// sequential decoder after the call.
    Done(AacDecoderError),
    /// Stage B is to run.
    Frame(Box<HandOver>),
}

/// What stage A knows about the SBR decoder while stage B owns it: the configuration that
/// `sbrDecoder_InitElement()` compares and the elements that exist, captured whenever the SBR
/// decoder leaves the decoder instance for stage B.
///
/// Between that moment and stage A's use, stage B can only clear `SBRDEC_FORCE_RESET` (in
/// `sbrDecoder_Apply()`): none of the operations of stage B (and of the recorded operations of
/// stage A) changes the sample rates, the frame size, the core codec, the elements or
/// `harmonicSBR`. So a call that returns at once according to the cache returns at once in the
/// sequential order too; a cache that sees `SBRDEC_FORCE_RESET` makes stage A wait for stage B
/// and call `sbrDecoder_InitElement()` (which then decides as in C).
#[derive(Clone, Copy, Default)]
pub(crate) struct SbrCache {
    /// `hSbrDecoder != NULL`
    pub present: bool,
    pub sample_rate_in: i32,
    pub sample_rate_out: i32,
    pub codec_frame_size: u16,
    pub core_codec: AudioObjectType,
    pub harmonic_sbr: u8,
    pub force_reset: bool,
    /// `pSbrElement[i] != NULL ? pSbrElement[i]->elementID`
    pub elements: [Option<Mp4ElementId>; 8],
}

impl SbrCache {
    /// The cache of the SBR decoder `h`.
    pub(crate) fn capture(h: Option<&SbrDecoderInstance>) -> SbrCache {
        let Some(s) = h else {
            return SbrCache::default();
        };
        let mut c = SbrCache {
            present: true,
            sample_rate_in: s.sample_rate_in,
            sample_rate_out: s.sample_rate_out,
            codec_frame_size: s.codec_frame_size,
            core_codec: s.core_codec,
            harmonic_sbr: s.harmonic_sbr,
            force_reset: s.flags & SBRDEC_FORCE_RESET != 0,
            elements: [None; 8],
        };
        for (i, e) in s.p_sbr_element.iter().enumerate().take(8) {
            c.elements[i] = e.as_deref().map(|e| e.element_id);
        }
        c
    }

    /// The result of `sbrDecoder_InitElement()` if it returns at once (`Some(SBRDEC_OK)`, or
    /// `SBRDEC_NOT_INITIALIZED` without an SBR decoder); `None` if it would (re)configure or
    /// fail.
    #[allow(clippy::too_many_arguments)]
    fn init_noop(
        &self,
        sample_rate_in: i32,
        sample_rate_out: i32,
        samples_per_frame: i32,
        core_codec: AudioObjectType,
        element_id: Mp4ElementId,
        element_index: i32,
        harmonic_sbr: u8,
    ) -> Option<SbrError> {
        if !self.present {
            return Some(SBRDEC_NOT_INITIALIZED);
        }
        if !(0..8).contains(&element_index) {
            return None;
        }
        if element_id != ID_SCE && element_id != ID_CPE && element_id != ID_LFE {
            return None;
        }
        let codec_valid = matches!(
            core_codec,
            AOT_AAC_LC
                | AOT_SBR
                | AOT_PS
                | AOT_ER_AAC_SCAL
                | AOT_ER_AAC_ELD
                | AOT_DRM_AAC
                | AOT_DRM_SURROUND
                | AOT_USAC
        );
        if !codec_valid {
            return None;
        }
        let same = self.sample_rate_in == sample_rate_in
            && self.codec_frame_size as i32 == samples_per_frame
            && self.core_codec == core_codec
            && self.elements[element_index as usize] == Some(element_id)
            && !self.force_reset
            && (sample_rate_out == 0 || self.sample_rate_out == sample_rate_out)
            && (harmonic_sbr == 2 || self.harmonic_sbr == harmonic_sbr);
        if same {
            Some(SBRDEC_OK)
        } else {
            None
        }
    }
}

fn pipe(self_: &mut AacDecoderInstance) -> &mut Pipeline {
    self_.pipe.as_deref_mut().expect("pipeline")
}

fn away(self_: &AacDecoderInstance) -> bool {
    self_.pipe.as_deref().is_some_and(|p| p.away())
}

/// Waits for stage B of the previous frame in the middle of stage A, and executes the SBR
/// operations stage A recorded so far: from here on, stage A runs as the sequential decoder.
///
/// After stage A began to write the per frame channel memory ([`state::begin_core_a`]), this
/// does not wait: it returns `true`, and the frame is decoded again by the sequential decoder
/// ([`state::restart`]); the caller must not touch state of stage B and ends stage A as soon
/// as possible (what stage A does until then is undone).
pub(crate) fn join_mid_a(self_: &mut AacDecoderInstance) -> bool {
    if away(self_) {
        let p = pipe(self_);
        if p.touched {
            p.restart = true;
            return true;
        }
        super::join(self_);
        state::flush_b_ops(self_);
    }
    false
}

/// A restart is pending ([`join_mid_a`]).
fn restarting(self_: &AacDecoderInstance) -> bool {
    self_.pipe.as_deref().is_some_and(|p| p.restart)
}

/// `sbrDecoder_InitElement()` (through `sbr_init_element()`) in stage A.
#[allow(clippy::too_many_arguments)]
fn sbr_init_a(
    self_: &mut AacDecoderInstance,
    sample_rate_in: i32,
    sample_rate_out: i32,
    samples_per_frame: i32,
    core_codec: AudioObjectType,
    element_id: Mp4ElementId,
    element_index: i32,
    harmonic_sbr: u8,
    config_mode: u8,
    config_changed: &mut u8,
    downscale_factor: i32,
) -> SbrError {
    if away(self_) {
        if let Some(r) = pipe(self_).sbr_cache.init_noop(
            sample_rate_in,
            sample_rate_out,
            samples_per_frame,
            core_codec,
            element_id,
            element_index,
            harmonic_sbr,
        ) {
            return r;
        }
        if join_mid_a(self_) {
            return SBRDEC_NOT_INITIALIZED;
        }
    }
    sbr_init_element(
        self_.h_sbr_decoder.as_deref_mut(),
        &mut self_.qmf_domain,
        sample_rate_in,
        sample_rate_out,
        samples_per_frame,
        core_codec,
        element_id,
        element_index,
        harmonic_sbr,
        0,
        config_mode,
        config_changed,
        downscale_factor,
    )
}

/// `CAacDecoder_SyncQmfMode()` in stage A.
fn sync_qmf_mode_a(self_: &mut AacDecoderInstance) {
    if away(self_) {
        let op = BOp::SyncQmfMode {
            flags: self_.flags[0],
            aac_num_channels: self_.stream_info.aac_num_channels,
        };
        pipe(self_).b_ops.push(op);
    } else {
        caac_decoder_sync_qmf_mode(self_);
    }
}

/// `sbrDecoder_Parse()` of an SBR fill element (through `sbr_parse()`) in stage A. While
/// stage B owns the SBR decoder, the call is recorded with a copy of the bits from the payload
/// on; the result is `SBRDEC_NOT_INITIALIZED` exactly when there is no SBR element at the
/// index (or no SBR decoder), and `SBRDEC_OK` or `SBRDEC_PARSE_ERROR` otherwise, which the
/// caller does not distinguish. `count` is left as the caller's push to the end of the fill
/// element expects it (the payload length is known, so the position after it is the same).
fn sbr_parse_fill_a(
    self_: &mut AacDecoderInstance,
    h_bs: &mut FdkBitStream,
    count: &mut i32,
    crc_flag: i32,
    prev_element: Mp4ElementId,
    element_index: i32,
) -> SbrError {
    if !away(self_) {
        let count_in = *count;
        return sbr_parse(self_, h_bs, count, count_in, crc_flag, prev_element, element_index);
    }
    let cache = pipe(self_).sbr_cache;
    let result = if !cache.present {
        SBRDEC_NOT_INITIALIZED
    } else if *count <= 0 {
        SBRDEC_OK
    } else if !(0..8).contains(&element_index) || cache.elements[element_index as usize].is_none() {
        SBRDEC_NOT_INITIALIZED
    } else {
        SBRDEC_PARSE_ERROR
    };
    if cache.present {
        let p = copy_payload(self_, h_bs, *count);
        let op = SbrParse {
            count: *count,
            bs_pay_len: *count,
            crc_flag,
            prev_element,
            element_index,
            ac_flags: self_.flags[0],
            ..p
        };
        pipe(self_).b_ops.push(BOp::Parse(Box::new(op)));
    }
    result
}

/// Copies the bits of the bit buffer of `h_bs` from the current position on: `count` bits and
/// [`SBR_COPY_MARGIN`] bytes beyond them, as they are in the bit buffer now (including stale
/// content beyond the valid bits), with its wrap-around.
fn copy_payload(self_: &mut AacDecoderInstance, h_bs: &mut FdkBitStream, count: i32) -> SbrParse {
    let valid_bits = fdk_get_valid_bits(h_bs); // (synchronizes the cache, as sbrDecoder_Parse())
    let bb = &h_bs.h_bit_buf;
    let pos = bb.bit_ndx;
    let byte0 = (pos >> 3) as usize;
    let bit_offset = pos & 7;
    let len = ((count.max(0) as usize + bit_offset as usize + 7) >> 3) + SBR_COPY_MARGIN;
    let cap = len.next_power_of_two();
    let mut data = pipe(self_).sbr_pool.pop().unwrap_or_default();
    data.clear();
    data.resize(cap, 0);
    let n = bb.buf_size() as usize;
    if n > 0 && !bb.mem().is_empty() {
        let base = bb.base() as usize;
        for (i, d) in data[..len].iter_mut().enumerate() {
            *d = bb.mem().get(base + (byte0 + i) % n);
        }
    }
    SbrParse {
        data,
        data_len: len,
        bit_offset,
        valid_bits,
        count: 0,
        bs_pay_len: 0,
        crc_flag: 0,
        prev_element: ID_NONE,
        element_index: 0,
        ac_flags: 0,
    }
}

/// A bitstream reading the copy of a deferred SBR payload: positioned at the payload, with the
/// valid bits the transport decoder had there.
pub(crate) fn payload_bitstream(p: &SbrParse) -> FdkBitStream {
    let mut bs = FdkBitStream::default();
    let cap = p.data.len() as u32;
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&p.data), cap, 0, BS_READER);
    bs.h_bit_buf.valid_bits = p.valid_bits.wrapping_add(p.bit_offset);
    fdk_push_for(&mut bs, p.bit_offset);
    bs
}

/// `CProgramConfig_LookupElement()` on the channel tables of the stream info, recording the
/// entries it writes while stage B of the previous frame runs ([`Pipeline::t_mask`]).
///
/// The function writes the type and the index of an entry together; it is run on tables
/// marked with a type it never writes (`i32::MIN`), and the entries it did not write get their
/// values back. The entries it writes are functions of the configuration and the channel
/// index, except one case: the second channel of a CPE of a channel configuration > 0 gets a
/// copy of the first channel's entry (index + 1), and the first channel's entry is only
/// written for the known implicit channel types. If it was not written (an unknown channel
/// configuration), the copy is of an entry that stage B of the previous frame may still write:
/// stage A then waits for stage B and makes the copy from the complete tables.
fn lookup_element_a(
    self_: &mut AacDecoderInstance,
    tag: u32,
    channel_idx: u32,
    el_mapping: &mut u8,
    el_type: Mp4ElementId,
) -> i32 {
    let channel_config = self_.stream_info.channel_config as u32;
    if !away(self_) {
        return cprogram_config_lookup_element(
            &mut self_.pce,
            channel_config,
            tag,
            channel_idx,
            &mut self_.ch_mapping,
            &mut self_.stream_info.p_channel_type,
            &mut self_.stream_info.p_channel_indices,
            8,
            el_mapping,
            &mut self_.elements,
            el_type,
        );
    }
    const MARK: AudioChannelType = i32::MIN;
    let old_t = self_.stream_info.p_channel_type;
    let old_i = self_.stream_info.p_channel_indices;
    let mut t = [MARK; 8];
    let mut ix = old_i;
    let r = cprogram_config_lookup_element(
        &mut self_.pce,
        channel_config,
        tag,
        channel_idx,
        &mut self_.ch_mapping,
        &mut t,
        &mut ix,
        8,
        el_mapping,
        &mut self_.elements,
        el_type,
    );
    let ci = channel_idx as usize;
    let cpe_copy_of_unwritten = r != 0
        && channel_config > 0
        && is_channel_element(el_type)
        && (el_type == ID_CPE || el_type == ID_USAC_CPE)
        && ci + 1 < 8
        && t[ci] == MARK;
    let mut mask = 0u8;
    for i in 0..8 {
        if t[i] == MARK {
            t[i] = old_t[i];
            ix[i] = old_i[i];
        } else {
            mask |= 1 << i;
        }
    }
    self_.stream_info.p_channel_type = t;
    self_.stream_info.p_channel_indices = ix;
    pipe(self_).t_mask |= mask;
    if cpe_copy_of_unwritten {
        // (not reached for the channel configurations 1 to 7)
        if join_mid_a(self_) {
            return 0;
        }
        let si = &mut self_.stream_info;
        si.p_channel_type[ci + 1] = si.p_channel_type[ci];
        si.p_channel_indices[ci + 1] = si.p_channel_indices[ci].wrapping_add(1);
    }
    r
}

/// Stage A of a call of `aacDecoder_DecodeFrame()` without flags (see the module
/// documentation). `p_time_data` is the output buffer of the call, used if the call is decoded
/// by the sequential decoder after the transport decoder's part (a configuration that stage A
/// does not handle).
#[allow(clippy::cognitive_complexity)]
pub(crate) fn stage_a(
    self_: &mut AacDecoderInstance,
    time_data_size: i32,
    flags: u32,
    p_time_data: &mut [IntPcm],
) -> AOutcome {
    let error_status: AacDecoderError;
    let mut f_tp_interruption = 0; /* Transport originated interruption detection. */
    let mut f_tp_conceal = 0; /* Transport originated concealment. */

    debug_assert!(flags == 0);
    // (AACDEC_INTR, AACDEC_CLRHIST: no flags in stage A)

    /* Get current bits position for bitrate calculation. */
    let n_bits: i32 = match self_.h_input.as_deref_mut() {
        Some(tp) => fdk_get_valid_bits(&mut tp.bit_stream[0]) as i32,
        None => 0,
    };

    // The transport decoder: the code of the sequential decoder (its configuration callbacks
    // wait for stage B, see super::state::callback_join()).
    if let Err(e) = lib_read_access_unit(self_, flags, &mut f_tp_interruption, &mut f_tp_conceal) {
        if e == crate::aacdecoder_lib::AAC_DEC_NOT_ENOUGH_BITS
            && away(self_)
            && pipe(self_).b_ops.is_empty()
        {
            // Not enough bits while stage B of the previous frame runs: the end of the call
            // (aacDecoder_UpdateBitStreamCounters(); the other parts of the end of the call do
            // nothing without valid output) updates state of stage B; it is recorded and done
            // at the join (the stream information after this call is complete after a join,
            // see super::sync_stream_info()).
            let valid_after: i32 = match self_.h_input.as_deref_mut() {
                Some(tp) => fdk_get_valid_bits(&mut tp.bit_stream[0]) as i32,
                None => 0,
            };
            let n = n_bits.wrapping_sub(valid_after);
            pipe(self_).b_writes.push(super::BWrite::Counters(n, e));
            return AOutcome::Done(e);
        }
        // (stage A did not write the per frame channel memory: no restart)
        join_mid_a(self_);
        return AOutcome::Done(lib_tail(self_, n_bits, e, 0, 0, 0));
    }

    // The sequential decoder for the rest of the call (after a join).
    let sequential = |self_: &mut AacDecoderInstance, p_time_data: &mut [IntPcm]| {
        let mut td2 = self_.p_time_data2.take().unwrap_or_default();
        let err = decode_frame_lib_after_read(
            self_,
            p_time_data,
            time_data_size,
            flags,
            &mut td2,
            n_bits,
            f_tp_interruption,
            f_tp_conceal,
        );
        self_.p_time_data2 = Some(td2);
        AOutcome::Done(err)
    };

    if !state::eligible_after_read(self_) {
        // A configuration stage A does not handle (set by a callback of this call): the
        // sequential decoder decodes the rest of the call.
        join_mid_a(self_);
        return sequential(self_, p_time_data);
    }

    // (while stage B of the previous frame runs: the state to decode the frame again from)
    state::take_snapshot(self_);

    let mut ho = Box::new(HandOver::new());
    let mut core_reached = false;

    'bail: {
        self_.frame_ok = 1;

        for i in 0..self_.preroll_au_length.len() {
            self_.preroll_au_length[i] = 0;
        }

        // (no pre-roll: hasAudioPreRoll == 0, buildUpStatus == 0, flushStatus == 0)

        self_.access_unit = 0;

        /* Signal bit stream interruption to other modules if required. */
        if f_tp_interruption != 0 {
            // (before state::begin_core_a(): no restart)
            join_mid_a(self_);
            aac_decoder_signal_interruption(self_);
            error_status = AAC_DEC_TRANSPORT_SYNC_ERROR;
            break 'bail;
        }

        // (streamInfo.outputDelay = 0 and extGain[0] = AACDEC_DRC_GAIN_INIT_VALUE: stage B,
        // or below for a call that ends in stage A)
        core_reached = true;

        if self_.limiter_enable_user == -2i8 as u8 {
            /* Enable limiter only for RSVD60. */
            self_.limiter_enable_curr = ((self_.flags[0] & AC_RSV603DA) != 0) as u8;
        } else if self_.limiter_enable_user == -1i8 as u8 {
            /* Enable limiter for all non-lowdelay AOT's. */
            self_.limiter_enable_curr = ((self_.flags[0] & (AC_LD | AC_ELD)) == 0) as u8;
        } else {
            /* Use limiter configuration as requested. */
            self_.limiter_enable_curr = self_.limiter_enable_user;
        }

        let time_data2_size: i32 = self_.time_data2_size / core::mem::size_of::<PcmDec>() as i32;

        let core_flags = flags | (if f_tp_conceal != 0 { AACDEC_CONCEAL } else { 0 });
        let spf = self_.stream_info.aac_samples_per_frame;
        state::begin_core_a(self_);
        // (test builds: the per frame channel memory this stage A got at the last hand-over)
        state::poison_stage_a(self_);
        error_status = match self_.h_input.as_deref_mut() {
            Some(tp) => {
                let mut bs: FdkBitStream = tp.bit_stream[0].clone();
                let r = core_a(self_, core_flags, time_data2_size, spf, &mut bs, &mut ho);
                if let Some(tp) = self_.h_input.as_deref_mut() {
                    tp.bit_stream[0] = bs;
                }
                r
            }
            // No transport decoder: the reference has no defined result here.
            None => AAC_DEC_UNKNOWN,
        };
        ho.flags = core_flags;
        ho.call_flags = flags;
        ho.time_data_size = time_data_size;

        /* USAC DASH IPF flushing possible begin */
        if f_tp_conceal == 0 {
            // (flushStatus == 0; accessUnit >= numPrerollAU)
            let tp_err = match self_.h_input.as_deref_mut() {
                Some(tp) => transport_dec_end_access_unit(tp),
                None => TRANSPORTDEC_UNKOWN_ERROR,
            };
            if tp_err != TRANSPORTDEC_OK {
                self_.frame_ok = 0;
            }
        }

        if is_output_valid(error_status) == 0 {
            break 'bail;
        }

        if self_.pipe.as_deref().is_some_and(|p| p.touched)
            && (restarting(self_) || !state::can_defer_a(self_, &ho))
        {
            // Not handed over: decoded again on the per frame channel memory of the
            // previous frame.
            state::restart(self_);
            return sequential(self_, p_time_data);
        }

        let valid_after: i32 = match self_.h_input.as_deref_mut() {
            Some(tp) => fdk_get_valid_bits(&mut tp.bit_stream[0]) as i32,
            None => 0,
        };
        ho.n_bits_consumed = n_bits.wrapping_sub(valid_after);
        ho.core_err = error_status;
        return AOutcome::Frame(ho);
    }

    /* bail: the call ends in stage A (no valid output) */
    if self_.pipe.as_deref().is_some_and(|p| p.touched) {
        // (decoded again on the per frame channel memory of the previous frame)
        state::restart(self_);
        return sequential(self_, p_time_data);
    }
    join_mid_a(self_);
    if core_reached {
        self_.stream_info.output_delay = 0;
        self_.ext_gain[0] = crate::aacdec_drc::AACDEC_DRC_GAIN_INIT_VALUE;
    }
    AOutcome::Done(lib_tail(self_, n_bits, error_status, 0, 0, 0))
}

/// `CAacDecoder_DecodeFrame()` without the element decode loop and without the inverse
/// transform (see the module documentation). `time_data_size` is that of `pTimeData2`.
#[allow(clippy::cognitive_complexity)]
fn core_a(
    self_: &mut AacDecoderInstance,
    flags: u32,
    time_data_size: i32,
    time_data_channel_offset: i32,
    bs: &mut FdkBitStream,
    ho: &mut HandOver,
) -> AacDecoderError {
    let _ = (time_data_size, time_data_channel_offset);
    let mut error_status = AAC_DEC_OK;

    let mut type_: Mp4ElementId = ID_NONE; /* Current element type */
    let mut aac_channels: i32 = 0; /* Channel counter for channels found in the bitstream */
    let stream_index: usize = 0; /* index of the current substream */

    let au_start_anchor = fdk_get_valid_bits(bs) as i32; /* AU start bit buffer position for AU byte alignment */

    let mut check_sample_rate = self_.stream_info.aac_sample_rate;

    if self_.aac_channels <= 0 {
        return AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
    }

    /* Any supported base layer valid AU will require more than 16 bits. */
    if (self_.with_tp(bs, |tp| transport_dec_get_au_bits_remaining(tp, 0)) < 15)
        && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
    {
        self_.frame_ok = 0;
        error_status = AAC_DEC_DECODE_FRAME_ERROR;
    }

    /* Reset Program Config structure */
    cprogram_config_reset(&mut self_.pce);

    caac_decoder_anc_data_reset(&mut self_.anc_data);
    if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
        && (self_.flags[0] & (AC_USAC | AC_RSV603DA)) == 0
        && self_.stream_info.channel_config == 0
    {
        /* Init Channel/Element mapping table */
        for ch in 0..8 {
            self_.ch_mapping[ch] = 255;
        }
        if cprogram_config_is_valid(&self_.pce) == 0 {
            for el in 0..MAX_ELEMENTS {
                self_.elements[el] = ID_NONE;
            }
        }
    }

    if self_.downscale_factor > 1 && (self_.flags[0] & AC_ELD) != 0 {
        self_.flags[0] |= AC_ELD_DOWNSCALE;
    } else {
        self_.flags[0] &= !AC_ELD_DOWNSCALE;
    }
    /* unsupported dsf (aacSampleRate has not yet been divided by dsf) -> divide */
    if self_.downscale_factor_in_bs > 1 && (self_.flags[0] & AC_ELD_DOWNSCALE) == 0 {
        check_sample_rate = self_.stream_info.aac_sample_rate / self_.downscale_factor_in_bs as i32;
    }

    /* Check sampling frequency  */
    if self_.stream_info.aac_sample_rate <= 0 {
        /* Instance maybe uninitialized! */
        return AAC_DEC_UNSUPPORTED_SAMPLINGRATE;
    }
    match check_sample_rate {
        96000 | 88200 | 64000 | 16000 | 12000 | 11025 | 8000 | 7350 | 48000 | 44100 | 32000
        | 24000 | 22050 => {}
        _ => {
            if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA) == 0 {
                return AAC_DEC_UNSUPPORTED_SAMPLINGRATE;
            }
        }
    }

    // (AACDEC_CLRHIST: not in stage A)

    let mut pce_read = 0; /* Flag indicating a PCE in the current raw_data_block() */

    let hdaac_decoded: i32 = 0;
    let mut previous_element: Mp4ElementId = ID_END; /* Last element ID (required for extension payload mapping */
    let mut previous_element_index: u8 = 0; /* Canonical index of last element */
    let mut element_count: i32 = 0; /* Element counter for elements found in the bitstream */
    let mut channel_element_count: i32 = 0; /* Channel element counter */
    /* Channel elements in bit stream order. */
    let mut channel_elements = [ID_NONE; MAX_ELEMENTS];
    let mut el_cnt = [0i32; ID_LAST as usize]; /* element counter ( robustness ) */
    let element_count_prev_streams: i32 = 0; /* Element count of all previous sub streams. */
    let _ = element_count_prev_streams;

    while (type_ != ID_END) && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0 && self_.frame_ok != 0
    {
        let el_channels: i32;

        if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_ELD | AC_SCALABLE | AC_ER) == 0
        {
            type_ = fdk_read_bits(bs, 3) as Mp4ElementId;
        } else {
            if element_count as usize >= MAX_ELEMENTS {
                self_.frame_ok = 0;
                error_status = AAC_DEC_PARSE_ERROR;
                break;
            }
            type_ = self_.elements[element_count as usize];
        }

        // (USAC independency flag: not in stage A)

        if (fdk_get_valid_bits(bs) as i32) < 0 {
            self_.frame_ok = 0;
        }

        match type_ {
            ID_SCE | ID_CPE | ID_LFE | ID_USAC_SCE | ID_USAC_CPE | ID_USAC_LFE => 'el: {
                if element_count as usize >= MAX_ELEMENTS {
                    self_.frame_ok = 0;
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'el;
                }

                el_channels = caac_decoder_get_el_channels(
                    type_,
                    self_.usac_stereo_config_index[element_count as usize],
                );

                /*
                 Consistency check
                */
                {
                    let mut total_asc_channels = 0;

                    for i in 0..1 {
                        total_asc_channels += self_.asc_channels[i];
                    }
                    if (el_cnt[type_ as usize] >= (total_asc_channels >> (el_channels - 1)))
                        || (aac_channels > (total_asc_channels - el_channels))
                    {
                        error_status = AAC_DEC_DECODE_FRAME_ERROR;
                        self_.frame_ok = 0;
                        break 'el;
                    }
                }

                let a = aac_channels as usize;

                if self_.flags[stream_index] & (AC_USAC | AC_RSVD50 | AC_RSV603DA) == 0 {
                    for ch in 0..el_channels as usize {
                        let com = self_.p_com_data.get(a + ch).copied().flatten();
                        if let (Some(ci), Some(cd)) = (
                            self_
                                .p_aac_decoder_channel_info
                                .get_mut(a + ch)
                                .and_then(|c| c.as_deref_mut()),
                            com,
                        ) {
                            cpns_reset_data(
                                &mut ci.data.aac.pns_data,
                                &mut self_.com_data[cd].pns_inter_channel_data,
                            );
                        }
                    }
                }

                if self_.frame_ok != 0 {
                    let mut deferred = false;
                    error_status = read_channel_element_a(
                        self_,
                        bs,
                        a,
                        el_channels,
                        element_count as usize,
                        &mut deferred,
                    );
                    if deferred {
                        // the inverse quantization of the element's channels (stage B)
                        ho.iq_list[ho.iq_count] = (a as u8, el_channels as u8);
                        ho.iq_count += 1;
                    }
                    if error_status != AAC_DEC_OK {
                        self_.frame_ok = 0;
                    }
                }

                if self_.frame_ok != 0 {
                    /* Lookup the element and decode it only if it belongs to the current program */
                    let tag = self_.p_aac_decoder_channel_info[a.min(NCH - 1)]
                        .as_deref()
                        .map_or(0, |c| c.element_instance_tag);
                    if lookup_element_a(
                        self_,
                        tag as u32,
                        aac_channels as u32,
                        &mut previous_element_index,
                        type_,
                    ) != 0
                    {
                        channel_elements[channel_element_count as usize] = type_;
                        channel_element_count += 1;
                        aac_channels += el_channels;
                    } else {
                        self_.frame_ok = 0;
                    }
                    /* Create SBR element for SBR for upsampling for LFE elements, and if SBR was
                    implicitly signaled, because the first frame(s) may not contain SBR payload
                    (broken encoder, bit errors). */
                    if self_.frame_ok != 0
                        && ((self_.flags[stream_index] & AC_SBR_PRESENT) != 0
                            || (self_.sbr_enabled == 1))
                        && (self_.flags[stream_index] & AC_USAC) == 0
                    /* Is done during explicit config set up */
                    {
                        let mut config_mode: u8 = 0;
                        let mut config_changed: u8 = 0;
                        config_mode |= AC_CM_ALLOC_MEM;

                        let sbr_error = sbr_init_a(
                            self_,
                            self_.stream_info.aac_sample_rate,
                            self_.stream_info.ext_sampling_rate,
                            self_.stream_info.aac_samples_per_frame,
                            self_.stream_info.aot,
                            type_,
                            previous_element_index as i32,
                            2, /* Signalize that harmonicSBR shall be ignored in the config
                               change detection */
                            config_mode,
                            &mut config_changed,
                            self_.downscale_factor as i32,
                        );
                        if sbr_error != SBRDEC_OK {
                            /* Do not try to apply SBR because initializing the element failed. */
                            self_.sbr_enabled = 0;
                        }
                    }
                }

                el_cnt[type_ as usize] += 1;
                // (USAC explicit MPS and SBR: not in stage A)
            }

            ID_CCE => 'el: {
                /*
                  Consistency check
                */
                if el_cnt[type_ as usize] > self_.asc_channels[stream_index] {
                    error_status = AAC_DEC_DECODE_FRAME_ERROR;
                    self_.frame_ok = 0;
                    break 'el;
                }

                if self_.frame_ok != 0 {
                    // (the channel tables: see lookup_element_a(); rare, so wait here)
                    if join_mid_a(self_) {
                        self_.frame_ok = 0;
                        break 'el;
                    }
                    error_status =
                        read_coupling_channel_element(self_, bs, &mut previous_element_index);
                }
                el_cnt[type_ as usize] += 1;
            }

            ID_DSE => 'el: {
                let mut element_instance_tag: u8 = 0;

                // DVB DRC and downmix metadata: state of stage B.
                if join_mid_a(self_) {
                    self_.frame_ok = 0;
                    break 'el;
                }
                cdata_stream_element_read(
                    self_,
                    bs,
                    &mut element_instance_tag,
                    au_start_anchor as u32,
                );

                if cprogram_config_lookup_element(
                    &mut self_.pce,
                    self_.stream_info.channel_config as u32,
                    element_instance_tag as u32,
                    0,
                    &mut self_.ch_mapping,
                    &mut self_.stream_info.p_channel_type,
                    &mut self_.stream_info.p_channel_indices,
                    8,
                    &mut previous_element_index,
                    &mut self_.elements,
                    type_,
                ) == 0
                {
                    /* most likely an error in bitstream occured */
                    // self->frameOK = 0;
                }
            }

            ID_PCE => 'el: {
                if join_mid_a(self_) {
                    self_.frame_ok = 0;
                    break 'el;
                }
                let channel_config = self_.stream_info.channel_config as u32;
                let result = cprogram_config_element_read(
                    bs,
                    self_.h_input.as_deref_mut(),
                    &mut self_.pce,
                    channel_config,
                    au_start_anchor as u32,
                );
                if result < 0 {
                    /* Something went wrong */
                    error_status = AAC_DEC_PARSE_ERROR;
                    self_.frame_ok = 0;
                } else if result > 1 {
                    /* Built element table */
                    let mut el_idx = cprogram_config_get_element_table(
                        &self_.pce,
                        &mut self_.elements,
                        (8 + 8) as i32,
                        &mut self_.ch_map_index,
                    );
                    /* Reset the remaining tabs */
                    while (el_idx as usize) < MAX_ELEMENTS {
                        self_.elements[el_idx.max(0) as usize] = ID_NONE;
                        el_idx += 1;
                    }
                    /* Make new number of channel persistent */
                    self_.asc_channels[stream_index] = self_.pce.num_channels as i32;
                    /* If PCE is not first element conceal this frame to avoid inconsistencies */
                    if element_count != 0 {
                        self_.frame_ok = 0;
                    }
                }
                pce_read = if result >= 0 { 1 } else { 0 };
            }

            ID_FIL => {
                let mut bit_cnt = fdk_read_bits(bs, 4) as i32; /* bs_count */

                if bit_cnt == 15 {
                    let esc_count = fdk_read_bits(bs, 8) as i32; /* bs_esc_count */
                    bit_cnt = esc_count + 14;
                }

                /* Convert to bits */
                bit_cnt <<= 3;

                while bit_cnt > 0 {
                    error_status = ext_payload_parse_a(
                        self_,
                        bs,
                        &mut bit_cnt,
                        previous_element,
                        previous_element_index as i32,
                        1,
                    );
                    if error_status != AAC_DEC_OK {
                        self_.frame_ok = 0;
                        break;
                    }
                }
            }

            ID_EXT => 'el: {
                if element_count as usize >= MAX_ELEMENTS {
                    self_.frame_ok = 0;
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'el;
                }

                // The explicit SBR and MPEG Surround payloads of ELD: the bitstream position
                // after them depends on their parsers.
                if join_mid_a(self_) {
                    self_.frame_ok = 0;
                    break 'el;
                }
                error_status = aac_decoder_parse_explicit_mps_and_sbr(
                    self_,
                    bs,
                    previous_element,
                    previous_element_index as i32,
                    element_count,
                    &el_cnt,
                );
            }

            ID_END | ID_USAC_END => {}

            _ => {
                error_status = AAC_DEC_DECODE_FRAME_ERROR;
                self_.frame_ok = 0;
            }
        }

        previous_element = type_;
        element_count += 1;
    } /* while ( (type != ID_END) ... ) */

    if restarting(self_) {
        // (the frame is decoded again)
        return AAC_DEC_UNKNOWN;
    }

    if (self_.flags[stream_index]
        & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_BSAC | AC_LD | AC_ELD | AC_ER | AC_SCALABLE))
        == 0
        && (self_.stream_info.channel_config == 0)
        && self_.pce.is_valid != 0
        && (error_status == AAC_DEC_OK)
        && self_.frame_ok != 0
        && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
    {
        /* Check whether all PCE listed element instance tags are present in raw_data_block() */
        if validate_element_instance_tags(
            &self_.pce,
            &self_.p_aac_decoder_channel_info,
            aac_channels,
            &channel_elements,
            channel_element_count.min(MAX_ELEMENTS as i32),
        ) == 0
        {
            error_status = AAC_DEC_DECODE_FRAME_ERROR;
            self_.frame_ok = 0;
        }
    }

    if flags & (AACDEC_CONCEAL | AACDEC_FLUSH) == 0 {
        let preroll_len =
            self_.preroll_au_length.get(self_.access_unit as usize).copied().unwrap_or(0);
        /* float decoder checks if bitsLeft is in range 0-7; only prerollAUs are byteAligned with
         * respect to the first bit */
        /* Byte alignment with respect to the first bit of the raw_data_block(). */
        if (self_.flags[stream_index] & (AC_RSVD50 | AC_USAC)) == 0 || preroll_len != 0
        /* indicates preroll */
        {
            fdk_byte_align(bs, au_start_anchor as u32);
        }

        /* Check if all bits of the raw_data_block() have been read. */
        if self_.with_tp(bs, |tp| transport_dec_get_au_bits_total(tp, 0)) > 0 {
            let mut unread_bits =
                self_.with_tp(bs, |tp| transport_dec_get_au_bits_remaining(tp, 0));
            /* for pre-roll frames pre-roll length has to be used instead of total AU lenght */
            /* unreadBits regarding preroll bounds */
            if preroll_len != 0 {
                unread_bits = unread_bits
                    .wrapping_sub(self_.with_tp(bs, |tp| transport_dec_get_au_bits_total(tp, 0)))
                    .wrapping_add(preroll_len as i32);
            }
            let usac = (self_.flags[stream_index] & (AC_RSVD50 | AC_USAC)) != 0;
            if (usac && ((unread_bits < 0) || (unread_bits > 7)) && preroll_len == 0)
                || ((!usac || preroll_len != 0) && (unread_bits != 0))
            {
                let is_drm = self_
                    .h_input
                    .as_deref()
                    .is_some_and(|tp| transport_dec_get_format(tp) == TT_DRM);
                if (((unread_bits < 0) || (unread_bits > 7)) && self_.frame_ok != 0)
                    && (is_drm && (self_.flags[stream_index] & AC_USAC) != 0)
                {
                    /* Set frame OK because of fill bits. */
                    self_.frame_ok = 1;
                } else {
                    self_.frame_ok = 0;
                }

                /* Do not overwrite current error */
                if error_status == AAC_DEC_OK && self_.frame_ok == 0 {
                    error_status = AAC_DEC_PARSE_ERROR;
                }
                /* Always put the bitbuffer at the right position after the current Access
                 * Unit. */
                fdk_push_bi_directional(bs, unread_bits);
            }
        }

        /* Check the last element. The terminator (ID_END) has to be the last one (even if ER
         * syntax is used). */
        if self_.frame_ok != 0 && type_ != ID_END {
            /* Do not overwrite current error */
            if error_status == AAC_DEC_OK {
                error_status = AAC_DEC_PARSE_ERROR;
            }
            self_.frame_ok = 0;
        }
    }

    if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0 && self_.frame_ok != 0 {
        // channel_element_count is at most 8 here (the consistency check of the channel
        // elements); a write outside the array is dropped.
        if let Some(e) = channel_elements.get_mut(channel_element_count as usize) {
            *e = ID_END;
        }
        channel_element_count += 1;
    }
    let _ = channel_element_count;

    // The element decode loop (CChannelElement_Decode()) runs in stage B; here only which
    // elements it decodes, and the channel count, element count and channel_elements[] it
    // leaves (the loop of C without the decoding).
    ho.decode_frame_ok = self_.frame_ok != 0;
    element_count = 0;
    aac_channels = 0;
    type_ = ID_NONE;
    previous_element_index = 0;
    let _ = previous_element_index;
    ho.decode_count = 0;

    while type_ != ID_END && (element_count as usize) < MAX_ELEMENTS {
        let ec = element_count as usize;

        if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) != 0 || self_.frame_ok == 0 {
            channel_elements[ec] = self_.elements[ec];
            if channel_elements[ec] == ID_NONE {
                channel_elements[ec] = ID_END;
            }
        }

        if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA | AC_BSAC) != 0 {
            type_ = self_.elements[ec];
        } else {
            type_ = channel_elements[ec];
        }

        if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0 && self_.frame_ok != 0 {
            match type_ {
                ID_SCE | ID_CPE | ID_LFE | ID_USAC_SCE | ID_USAC_CPE | ID_USAC_LFE => {
                    let el_channels =
                        caac_decoder_get_el_channels(type_, self_.usac_stereo_config_index[ec]);

                    if hdaac_decoded == 0 {
                        ho.decode_list[ho.decode_count] =
                            (ec as u8, aac_channels as u8, el_channels as u8);
                        ho.decode_count += 1;
                    }
                    aac_channels += el_channels;
                }
                ID_NONE => {
                    type_ = ID_END;
                }
                _ => {}
            }
        }
        element_count += 1;
    }
    ho.element_count = element_count;

    /* More AAC channels than specified by the ASC not allowed. */
    if (aac_channels == 0 || aac_channels > self_.aac_channels)
        && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
    {
        /* Do not overwrite current error */
        if error_status == AAC_DEC_OK {
            error_status = AAC_DEC_DECODE_FRAME_ERROR;
        }
        self_.frame_ok = 0;
        aac_channels = 0;
    }

    if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
        && TRANSPORTDEC_OK != self_.with_tp(bs, transport_dec_crc_check)
    {
        error_status = crate::aacdecoder_lib::AAC_DEC_CRC_ERROR;
        self_.frame_ok = 0;
    }

    /* Ensure that in case of concealment a proper error status is set. */
    if (self_.frame_ok == 0) && (error_status == AAC_DEC_OK) {
        error_status = AAC_DEC_DECODE_FRAME_ERROR;
    }

    if self_.frame_ok != 0 && (flags & AACDEC_FLUSH) != 0 {
        unreachable!("AACDEC_FLUSH in stage A");
    } else {
        /* store or restore the number of channels and the corresponding info */
        if self_.frame_ok != 0 && (flags & AACDEC_CONCEAL) == 0 {
            // The stored tables include the entries this frame did not write.
            state::note_prev_store(self_);
            self_.aac_channels_prev = aac_channels; /* store */
            self_.channel_type_prev = self_.stream_info.p_channel_type; /* store */
            self_.channel_indices_prev = self_.stream_info.p_channel_indices; /* store */
            self_.sbr_enabled_prev = self_.sbr_enabled;
        } else if self_.aac_channels > 0 {
            if (self_.build_up_status == crate::aacdecoder::AACDEC_RSV60_BUILD_UP_ON)
                || (self_.build_up_status == crate::aacdecoder::AACDEC_RSV60_BUILD_UP_ON_IN_BAND)
                || (self_.build_up_status == crate::aacdecoder::AACDEC_USAC_BUILD_UP_ON)
            {
                aac_channels = self_.aac_channels;
                self_.aac_channels_prev = aac_channels; /* store */
            } else {
                aac_channels = self_.aac_channels_prev; /* restore */
            }
            // (the restored tables are complete: written by an earlier stage A and completed
            // at the join that followed it)
            self_.stream_info.p_channel_type = self_.channel_type_prev; /* restore */
            self_.stream_info.p_channel_indices = self_.channel_indices_prev; /* restore */
            // (t_mask records only writes made while stage B of the previous frame runs: a
            // mask left from a stage A without one would make a later join that follows no
            // stage A, such as that of a sequential call, keep these entries instead of stage
            // B's)
            if let Some(p) = self_.pipe.as_deref_mut().filter(|p| p.away()) {
                p.t_mask = 0xff;
            }
            self_.sbr_enabled = self_.sbr_enabled_prev;
        }
    }

    /* Update number of output channels */
    self_.stream_info.aac_num_channels = aac_channels;

    /* Ensure consistency of IS_OUTPUT_VALID() macro. */
    if aac_channels == 0 {
        error_status = AAC_DEC_UNKNOWN;
    }

    if pce_read == 1 && cprogram_config_is_valid(&self_.pce) != 0 {
        /* Set matrix mixdown infos if available from PCE. */
        if join_mid_a(self_) {
            return AAC_DEC_UNKNOWN;
        }
        crate::pipeline::state::note_pce(self_);
        pcm_dmx_set_matrix_mixdown_from_pce(
            self_.h_pcm_utils.as_deref_mut(),
            self_.pce.matrix_mixdown_index_present as i32,
            self_.pce.matrix_mixdown_index as i32,
            self_.pce.pseudo_surround_enable as i32,
        );
    }

    /* If there is no valid data to transfrom into time domain, return. */
    if is_output_valid(error_status) == 0 {
        return error_status;
    }

    /* Setup the output channel mapping. (see the table in CAacDecoder_DecodeFrame()) */
    if (self_.stream_info.channel_config == 0) && self_.pce.is_valid == 0 {
        self_.ch_map_index = if aac_channels < 7 { aac_channels as u8 } else { 0 };
    }

    /*
      Inverse transform: the DRC channel map and the reverse mapping here; the rest in stage B.
    */
    {
        let mut f_copy_ch_map = 1;
        let mut drc_ch_map = [0u8; 8];

        if (self_.stream_info.channel_config == 0) && cprogram_config_is_valid(&self_.pce) != 0 {
            /* ISO/IEC 14496-3 says: If a PCE is present, the exclude_mask bits correspond to the
            audio channels in the SCE, CPE, CCE and LFE syntax elements in the order of their
            appearance in the PCE. ... Thus we have to convert the internal channel mapping from
            "canonical" MPEG to PCE order: */
            let mut tmp_ch_map = [0u8; 8];
            if cprogram_config_get_pce_ch_map(&self_.pce, &mut tmp_ch_map, 8) == 0 {
                for c in 0..aac_channels.clamp(0, 8) as usize {
                    drc_ch_map[c] = if self_.ch_mapping[c] == 255 {
                        255
                    } else {
                        // chMapping values are channel indices below 8; other values read 0.
                        tmp_ch_map.get(self_.ch_mapping[c] as usize).copied().unwrap_or(0)
                    };
                }
                f_copy_ch_map = 0;
            }
        }
        if f_copy_ch_map != 0 {
            drc_ch_map.copy_from_slice(&self_.ch_mapping[..8]);
        }
        ho.drc_ch_map = drc_ch_map;

        /* Create a reverse mapping table */
        let mut reverse_ch_mapping = [0u8; 8 * 2];
        for c in 0..aac_channels.clamp(0, 16) {
            let mut d: i32 = 0;
            while d < aac_channels - 1 {
                if self_.ch_mapping[d as usize] as i32 == c {
                    break;
                }
                d += 1;
            }
            reverse_ch_mapping[c as usize] = d as u8;
        }
        ho.reverse_ch_mapping = reverse_ch_mapping;
    }
    ho.aac_channels = aac_channels;
    ho.channel_elements = channel_elements;

    /* Reorder channel type information tables (at the end of CAacDecoder_DecodeFrame(); stage
    B does not read or write them before). */
    if self_.flags[0] & AC_RSV603DA == 0 {
        state::before_reorder(self_, aac_channels);
        let types = self_.stream_info.p_channel_type;
        let idx = self_.stream_info.p_channel_indices;

        for c in 0..aac_channels.clamp(0, 8) as usize {
            let map_value = fdk_ch_map_descr_get_map_value(
                &self_.map_descr,
                c as u8,
                self_.ch_map_index as u32,
            ) as usize;
            // The map values of the channel maps are below 8 (validated descriptors).
            if map_value < 8 {
                self_.stream_info.p_channel_type[map_value] = types[c];
                self_.stream_info.p_channel_indices[map_value] = idx[c];
                // (only while stage B of the previous frame runs, see above)
                if let Some(p) = self_.pipe.as_deref_mut().filter(|p| p.away()) {
                    p.t_mask |= 1 << map_value;
                }
            }
        }
    }

    self_.block_number += 1;

    error_status
}

/// `CChannelElement_Read()` of the channel element of the channels `a..a + el_channels`,
/// without the static channel infos, `pCpeStaticData` and `pComStaticData`, which belong to
/// stage B: the reader only uses them for USAC, the arithmetic coder and RVLC
/// ([`super::state::eligible_before_a`] excludes these configurations).
fn read_channel_element_a(
    self_: &mut AacDecoderInstance,
    bs: &mut FdkBitStream,
    a: usize,
    el_channels: i32,
    element_count: usize,
    deferred: &mut bool,
) -> AacDecoderError {
    let n = el_channels.max(0) as usize;
    let iq_in_a = self_.pipe.as_deref().is_some_and(|p| p.iq_in_a);
    let mut p = channels_mut(&mut self_.p_aac_decoder_channel_info, a, n);
    if p.len() < n {
        // The channel infos below ascChannels exist (C dereferences them).
        return AAC_DEC_UNKNOWN;
    }
    if self_.p_aac_decoder_static_channel_info.get(a).is_some_and(|s| s.is_none())
        && !self_.pipe.as_deref().is_some_and(|p| p.away())
    {
        // The static channel infos below ascChannels exist when stage B does not own them.
        return AAC_DEC_UNKNOWN;
    }

    let mut fb_com: Option<Box<CAacDecoderCommonData>> = None;
    let (com, com_r) = element_com_data(&mut self_.com_data, &self_.p_com_data, a, n);
    let com: &mut CAacDecoderCommonData = match com {
        Some(c) => c,
        None => fb_com.insert(Box::<CAacDecoderCommonData>::default()),
    };
    let mut com_static = CAacDecoderCommonStaticData::default();
    let mut empty: [fdk_aac_rs_fdk::common_fix::FixpDbl; 0] = [];
    let spec = match self_.work_buffer_core2.as_deref_mut() {
        Some(wb2) => tail_mut(wb2, a * 1024),
        None => &mut empty,
    };

    let err = c_channel_element_read_deferred_iq(
        bs,
        &mut p,
        None,
        self_.stream_info.aot,
        &self_.sampling_rate_info[0],
        self_.flags[0],
        self_.el_flags[element_count],
        self_.stream_info.aac_samples_per_frame as u32,
        el_channels as u8,
        self_.stream_info.ep_config,
        self_.h_input.as_deref_mut(),
        spec,
        com,
        com_r,
        &mut com_static,
        None,
        deferred,
    );
    if *deferred && iq_in_a {
        // The thread mode does the inverse quantization in stage A (as CChannelElement_Read()
        // in C, a little later: what runs in between does not touch the channels' spectra and
        // channel infos, see c_channel_element_read_deferred_iq(), which defers it further, to
        // stage B), so that the work of the two stages is balanced.
        c_channel_element_inverse_quantize(&mut p, &self_.sampling_rate_info[0], spec);
        *deferred = false;
    }
    err
}

/// `CAacDecoder_ExtPayloadParse()` in stage A (see the module documentation).
pub(crate) fn ext_payload_parse_a(
    self_: &mut AacDecoderInstance,
    h_bs: &mut FdkBitStream,
    count: &mut i32,
    previous_element: Mp4ElementId,
    el_index: i32,
    f_is_fill_element: i32,
) -> AacDecoderError {
    let mut error = AAC_DEC_OK;
    let mut bytes: i32 = (*count) >> 3;
    let mut crc_flag = 0;

    if *count < 4 {
        return AAC_DEC_PARSE_ERROR;
    } else if (fdk_get_valid_bits(h_bs) as i32) < *count {
        return AAC_DEC_DECODE_FRAME_ERROR;
    }

    let mut extension_type: ExtPayloadType = fdk_read_bits(h_bs, 4) as ExtPayloadType; /* bs_extension_type */
    *count -= 4;

    /* For ELD, the SBR signaling is explicit and parsed in aacDecoder_ParseExplicitMpsAndSbr(),
    therefore skip SBR if implicit present. */
    if (self_.flags[0] & AC_ELD) != 0
        && ((extension_type == EXT_SBR_DATA_CRC) || (extension_type == EXT_SBR_DATA))
    {
        extension_type = EXT_FIL; /* skip sbr data */
    }

    'bail: {
        match extension_type {
            EXT_DYNAMIC_RANGE => {
                // The DRC payload is parsed by the DRC prolog / epilog of stage B from the
                // bitstream: the frame is decoded in the calling thread.
                if join_mid_a(self_) {
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
                let read_bits = aac_decoder_drc_mark_payload(
                    self_.h_drc_info.as_deref_mut(),
                    h_bs,
                    MPEG_DRC_EXT_DATA,
                );

                if read_bits > *count {
                    /* Read too much. Something went wrong! */
                    error = AAC_DEC_PARSE_ERROR;
                }
                *count -= read_bits;
            }
            EXT_LDSAC_DATA | EXT_SAC_DATA => {
                if join_mid_a(self_) {
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
                /* Read MPEG Surround Extension payload */
                if self_.flags[0] & AC_PS_PRESENT != 0 {
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }

                /* Handle SBR dual rate case */
                let (mps_sample_rate, mps_frame_size) = if self_.stream_info.ext_sampling_rate != 0
                {
                    (
                        self_.stream_info.ext_sampling_rate,
                        self_.stream_info.aac_samples_per_frame.wrapping_mul(
                            self_.stream_info.ext_sampling_rate
                                / nz(self_.stream_info.aac_sample_rate),
                        ),
                    )
                } else {
                    (self_.stream_info.aac_sample_rate, self_.stream_info.aac_samples_per_frame)
                };
                /* Setting of internal MPS state; may be reset in CAacDecoder_SyncQmfMode if
                decoder is unable to decode with user defined qmfMode */
                if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_ELD) == 0 {
                    self_.mps_enable_curr = self_.mps_enable_user;
                }
                if self_.mps_enable_curr != 0 {
                    if self_.qmf_domain.global_conf.qmf_domain_explicit_config == 0 {
                        /* if not done yet, allocate full MPEG Surround decoder instance */
                        if mps_is_full_instance_available(self_.p_mpeg_surround_decoder.as_deref())
                            == SAC_INSTANCE_NOT_FULL_AVAILABLE
                            && mpeg_surround_decoder_open(
                                &mut self_.p_mpeg_surround_decoder,
                                -1,
                                Some(&mut self_.qmf_domain),
                            ) != 0
                        {
                            return AAC_DEC_OUT_OF_MEMORY;
                        }
                    }
                    let err = mpeg_surround_decoder_parse(
                        self_.p_mpeg_surround_decoder.as_deref_mut(),
                        h_bs,
                        count,
                        self_.stream_info.aot,
                        mps_sample_rate,
                        mps_frame_size,
                        (self_.flags[0] & AC_INDEP) as i32,
                    );
                    if err == MPS_OK {
                        self_.flags[0] |= AC_MPS_PRESENT;
                    } else {
                        error = AAC_DEC_PARSE_ERROR;
                    }
                }
                /* Skip any trailing bytes */
                fdk_push_for(h_bs, *count as u32);
                *count = 0;
            }

            EXT_SBR_DATA_CRC | EXT_SBR_DATA => {
                if extension_type == EXT_SBR_DATA_CRC {
                    crc_flag = 1;
                }
                if is_channel_element(previous_element) {
                    let mut config_mode: u8 = 0;
                    let mut config_changed: u8 = 0;

                    sync_qmf_mode_a(self_);

                    config_mode |= AC_CM_ALLOC_MEM;

                    let mut sbr_error = sbr_init_a(
                        self_,
                        self_.stream_info.aac_sample_rate,
                        self_.stream_info.ext_sampling_rate,
                        self_.stream_info.aac_samples_per_frame,
                        self_.stream_info.aot,
                        previous_element,
                        el_index,
                        2, /* Signalize that harmonicSBR shall be ignored in the config change
                           detection */
                        config_mode,
                        &mut config_changed,
                        self_.downscale_factor as i32,
                    );

                    if sbr_error == SBRDEC_OK {
                        let count_in = *count;
                        sbr_error = sbr_parse_fill_a(
                            self_,
                            h_bs,
                            count,
                            crc_flag,
                            previous_element,
                            el_index,
                        );
                        if !away(self_) && *count != count_in {
                            // (parsed directly: count is the remaining payload)
                        }
                        /* Enable SBR for implicit SBR signalling but only if no severe error
                         * happend. */
                        if (sbr_error == SBRDEC_OK) || (sbr_error == SBRDEC_PARSE_ERROR) {
                            self_.sbr_enabled = 1;
                        }
                    } else {
                        /* Do not try to apply SBR because initializing the element failed. */
                        self_.sbr_enabled = 0;
                    }
                    let _ = SBRDEC_UNSUPPORTED_CONFIG;
                    /* Citation from ISO/IEC 14496-3 chapter 4.5.2.1.5.2
                    Fill elements containing an extension_payload() with an extension_type of
                    EXT_SBR_DATA or EXT_SBR_DATA_CRC shall not contain any other
                    extension_payload of any other extension_type.
                    */
                    if f_is_fill_element != 0 {
                        fdk_push_bi_directional(h_bs, *count);
                        *count = 0;
                    } else {
                        /* If this is not a fill element with a known length, we are screwed and
                         * further parsing makes no sense. */
                        if sbr_error != SBRDEC_OK {
                            self_.frame_ok = 0;
                        }
                    }
                } else {
                    error = AAC_DEC_PARSE_ERROR;
                }
            }

            EXT_FILL_DATA => {
                let mut temp = fdk_read_bits(h_bs, 4) as i32;
                bytes -= 1;
                if temp != 0 {
                    error = AAC_DEC_PARSE_ERROR;
                } else {
                    while bytes > 0 {
                        temp = fdk_read_bits(h_bs, 8) as i32;
                        bytes -= 1;
                        if temp != 0xa5 {
                            error = AAC_DEC_PARSE_ERROR;
                            break;
                        }
                    }
                    *count = bytes << 3;
                }
            }

            EXT_DATA_ELEMENT => {
                let data_element_version = fdk_read_bits(h_bs, 4) as i32;
                *count -= 4;
                if data_element_version == 0
                /* ANC_DATA */
                {
                    let mut data_element_length: i32 = 0;
                    loop {
                        let temp = fdk_read_bits(h_bs, 8) as i32;
                        *count -= 8;
                        data_element_length += temp;
                        if temp != 255 {
                            break;
                        }
                    }

                    caac_decoder_anc_data_parse(&mut self_.anc_data, h_bs, data_element_length);
                    *count -= data_element_length << 3;
                } else {
                    /* align = 0 */
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
            }

            _ => {
                if extension_type == EXT_DATA_LENGTH
                    && f_is_fill_element == 0 /* Makes no sens to have an additional length in a fill ... */
                    && (self_.flags[0] & AC_ER) != 0
                /* ... element because this extension payload type was ... */
                {
                    /* ... created to circumvent the missing length in ER-Syntax. */
                    let mut len = fdk_read_bits(h_bs, 4) as i32;
                    *count -= 4;

                    if len == 15 {
                        let add_len = fdk_read_bits(h_bs, 8) as i32;
                        *count -= 8;
                        len += add_len;

                        if add_len == 255 {
                            len += fdk_read_bits(h_bs, 16) as i32;
                            *count -= 16;
                        }
                    }
                    len <<= 3;
                    let mut bit_cnt = len;

                    if fdk_read_bits(h_bs, 4) as ExtPayloadType == EXT_DATA_LENGTH {
                        /* Check NOTE 2: The extension_payload() included here must not have
                         * extension_type == EXT_DATA_LENGTH. */
                        error = AAC_DEC_PARSE_ERROR;
                        break 'bail;
                    } else {
                        /* rewind and call myself again. */
                        fdk_core_push_back(h_bs, 4);

                        error = ext_payload_parse_a(
                            self_,
                            h_bs,
                            &mut bit_cnt,
                            previous_element,
                            el_index,
                            1,
                        ); /* Treat same as fill element */

                        *count -= len - bit_cnt;
                    }
                    /* Note: the fall through in case the if statement above is not taken is
                     * intentional. */
                } else {
                    /* EXT_FIL and default: align = 4 */
                    fdk_push_for(h_bs, *count as u32);
                    *count = 0;
                }
            }
        }
    }

    /* bail: */
    if (error != AAC_DEC_OK) && f_is_fill_element != 0 {
        /* Skip the remaining extension bytes */
        fdk_push_bi_directional(h_bs, *count);
        *count = 0;
        /* Patch error code because decoding can go on. */
        error = AAC_DEC_OK;
        /* Be sure that parsing errors have been stored. */
    }
    error
}

#[inline]
fn fdk_core_push_back(h_bs: &mut FdkBitStream, n: u32) {
    fdk_aac_rs_fdk::bitstream::fdk_push_back(h_bs, n);
}

#[allow(unused_imports)]
use {cprogram_config_init as _, fdk_read_bit as _};

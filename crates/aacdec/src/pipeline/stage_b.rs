// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Rearranged copy of parts of libAACdec/src/aacdecoder.cpp (CAacDecoder_DecodeFrame(), from the
// element decode loop on) and libAACdec/src/aacdecoder_lib.cpp (aacDecoder_DecodeFrame(), after
// CAacDecoder_DecodeFrame() and transportDec_EndAccessUnit()).

//! Stage B of the frame pipeline: the rest of the decoding of a frame after stage A
//! ([`super::stage_a`]).
//!
//! The code is that of the sequential decoder (`aacdecoder::decode_frame_core()` and
//! `aacdecoder_lib::decode_frame_lib()`) for a call of one access unit without pre-roll, with
//! these differences, which do not change the result:
//!
//! - The element decode loop (`CChannelElement_Decode()`) runs here, after the checks that
//!   follow it in C (missing channels, CRC, the channel counters, the PCE downmix settings). It
//!   reads and writes the channel infos, the spectra, the static channel infos, the joint
//!   stereo scratch buffer and `pComData`, which none of these checks touches; the checks do
//!   not read what it writes. Which elements it decodes, and the element count and
//!   `channel_elements[]` it leaves, are computed by stage A ([`super::HandOver`]).
//! - `streamInfo.outputDelay = 0` and `extGain[0] = AACDEC_DRC_GAIN_INIT_VALUE`, which
//!   `aacDecoder_DecodeFrame()` does before `CAacDecoder_DecodeFrame()`, are done here: stage
//!   A does not read them.
//! - The reordering of the channel type tables and the frame counter (at the end of
//!   `CAacDecoder_DecodeFrame()`) are done by stage A: stage B does not read or write them
//!   before.
//! - The statistics take the number of bits the transport decoder consumed in the call, which
//!   stage A measured after `transportDec_EndAccessUnit()` (stage B does not move the
//!   bitstream).
//! - The DRC prolog and epilog read DRC payloads from the transport decoder's bitstream. A
//!   deferred stage B has no bitstream: stage A only defers frames without DRC payloads, for
//!   which they do not read it (they move it by zero bits).

use super::{state, HandOver, MAX_FRAME_SAMPLES};
use crate::aac_rom::PcmDec;
use crate::aacdec_drc::{
    aac_decoder_drc_apply, aac_decoder_drc_disable, aac_decoder_drc_epilog,
    aac_decoder_drc_get_info, aac_decoder_drc_init_channel_data, aac_decoder_drc_prolog,
    aac_decoder_drc_reset, apply_drc_level_normalization, AACDEC_DRC_GAIN_INIT_VALUE,
    AACDEC_DRC_GAIN_SCALING,
};
use crate::aacdecoder::{
    caac_decoder_apply_cross_fade, caac_decoder_get_el_channels, caac_decoder_prepare_cross_fade,
    caac_decoder_sync_qmf_mode, channels_mut, decode_channel_element,
    mps_is_full_instance_available, tail_mut, AacDecoderInstance, AACDEC_CROSSFADE_BITMASK_OFF,
    AACDEC_USAC_DASH_IPF_FLUSH_ON, NCH, TIME_DATA_FLUSH_SIZE,
};
use crate::aacdecoder_lib::{
    aac_decoder_signal_interruption, as_int_pcm, is_output_valid, tail_mut_dbl, CStreamInfo,
    AACDEC_CLRHIST, AACDEC_CONCEAL, AACDEC_FLUSH, AACDEC_INTR, AAC_DEC_DECODE_FRAME_ERROR,
    AAC_DEC_OK, AAC_DEC_OUTPUT_BUFFER_TOO_SMALL, AAC_DEC_OUT_OF_MEMORY, AAC_DEC_SET_PARAM_FAIL,
    AAC_DEC_UNKNOWN, AAC_DEC_UNSUPPORTED_FORMAT, PCM_OUT_HEADROOM,
};
use crate::block::{cblock_frequency_to_time, cblock_frequency_to_time_low_delay};
use crate::channel::c_channel_element_inverse_quantize;
use crate::channelinfo::*;
use crate::conceal::{cconcealment_apply, cconcealment_get_delay, cconcealment_td_fading};
use crate::conceal_types::{ConcealState_FadeIn, ConcealState_Mute};
use crate::delay::fdk_delay_apply;
use crate::usacdec_lpd::clpd_render_time_signal;
use fdk_aac_rs_drc::drc_dec_lib::{
    fdk_drc_dec_apply_downmix, fdk_drc_dec_get_param, fdk_drc_dec_preprocess,
    fdk_drc_dec_process_time, fdk_drc_dec_set_channel_gains, fdk_drc_dec_set_param, DRC_DEC_DRC1,
    DRC_DEC_DRC2_DRC3, DRC_DEC_FRAME_SIZE, DRC_DEC_IS_ACTIVE, DRC_DEC_LOUDNESS_NOT_PRESENT,
    DRC_DEC_OUTPUT_LOUDNESS, DRC_DEC_SAMPLE_RATE, DRC_DEC_TARGET_LOUDNESS,
};
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, FdkBitStream, BS_READER};
use fdk_aac_rs_fdk::common_fix::DFRACT_BITS;
use fdk_aac_rs_fdk::common_fix::{f_mult, fl2fxconst_dbl, fl2fxconst_sgl, FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm, f_mult_norm_to};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::qmf_domain::{
    fdk_qmf_domain_configure, QMF_DOMAIN_INIT_ERROR, QMF_DOMAIN_OK, QMF_DOMAIN_OUT_OF_MEMORY,
};
use fdk_aac_rs_fdk::scale::scale_values_saturate_dbl_to_sgl;
use fdk_aac_rs_fdk::syslib_channel_map_descr::fdk_ch_map_descr_get_map_value;
use fdk_aac_rs_pcmutils::limiter::{
    pcm_limiter_apply, pcm_limiter_get_delay, pcm_limiter_reset, pcm_limiter_set_n_channels,
    pcm_limiter_set_sample_rate,
};
use fdk_aac_rs_pcmutils::pcm_utils::{fdk_deinterleave, fdk_interleave, fdk_interleave_sgl};
use fdk_aac_rs_pcmutils::pcmdmx_lib::{
    pcm_dmx_apply_frame, pcm_dmx_get_param, pcm_dmx_reset, MAX_NUMBER_OF_OUTPUT_CHANNELS,
    PCMDMX_INVALID_MODE, PCMDMX_OK, PCMDMX_OUTPUT_BUFFER_TOO_SMALL, PCMDMX_RESET_BS_DATA,
};
use fdk_aac_rs_sacdec::sac_dec_errorcodes::MPS_OUTPUT_BUFFER_TOO_SMALL;
use fdk_aac_rs_sacdec::sac_dec_lib::{
    mpeg_surround_decoder_apply, mpeg_surround_decoder_configure_qmf_domain,
    mpeg_surround_decoder_get_delay, mpeg_surround_decoder_is_pseudo_lr,
    mpeg_surround_decoder_open, mpeg_surround_decoder_set_param, SACDEC_INTERFACE,
    SAC_INSTANCE_NOT_FULL_AVAILABLE, SAC_INTERFACE_QMF, SAC_INTERFACE_TIME,
};
use fdk_aac_rs_sbrdec::sbrdecoder::{
    sbr_decoder_apply, sbr_decoder_apply_pool, sbr_decoder_get_delay, sbr_decoder_set_param,
    SBRDEC_INVALID_ARGUMENT, SBRDEC_OK, SBRDEC_OUTPUT_BUFFER_TOO_SMALL, SBR_FLUSH_DATA,
    SBR_LD_QMF_TIME_ALIGN, SBR_SKIP_QMF, SBR_SYSTEM_BITSTREAM_DELAY,
};

/// `AAC_DECODER_ERROR`
type AacDecoderError = i32;

/// A bitstream without data for the DRC prolog and epilog of a deferred stage B (they do not
/// read it without DRC payloads; see the module documentation).
struct NoBitstream {
    bs: FdkBitStream,
}

impl NoBitstream {
    fn new() -> Box<NoBitstream> {
        let mut n = Box::new(NoBitstream { bs: FdkBitStream::default() });
        fdk_init_bit_stream(&mut n.bs, &BitMem::new(4), 4, 0, BS_READER);
        n
    }
}

/// Stage B of a frame on `inst` (the decoder instance in the calling thread, or the instance
/// of the worker thread), with the hand-over of stage A, the time signal buffer (`pTimeData2`)
/// and the output buffer of the call. `bs` is the transport decoder's bitstream (`None` for a
/// deferred stage B), `cap` the thread count of the per channel parallelism
/// ([`super::chan`]). Returns the return value of `aacDecoder_DecodeFrame()` and the number
/// of valid output samples.
pub(crate) fn stage_b(
    inst: &mut AacDecoderInstance,
    ho: &mut HandOver,
    td2: &mut [PcmDec],
    p_time_data: &mut [IntPcm],
    bs: Option<&mut FdkBitStream>,
    cap: usize,
) -> (AacDecoderError, usize) {
    // The SBR decoder operations of stage A, in their order.
    for op in ho.b_ops.drain(..) {
        state::exec_b_op(inst, op);
    }

    // aacDecoder_DecodeFrame(), before CAacDecoder_DecodeFrame():
    /* Reset the output delay field. The modules will add their figures one after another. */
    inst.stream_info.output_delay = 0;
    /* reset DRC level normalization gain on a per frame basis */
    inst.ext_gain[0] = AACDEC_DRC_GAIN_INIT_VALUE;

    let mut no_bs;
    let bs: &mut FdkBitStream = match bs {
        Some(b) => b,
        None => {
            no_bs = NoBitstream::new();
            &mut no_bs.bs
        }
    };

    let time_data2_size: i32 = inst.time_data2_size / core::mem::size_of::<PcmDec>() as i32;
    let spf = inst.stream_info.aac_samples_per_frame;
    let core_err = core_b(inst, ho, td2, time_data2_size, spf, bs, cap);

    let err = lib_b(inst, ho, td2, p_time_data, core_err, cap);

    let si = &inst.stream_info;
    let n = if is_output_valid(err) != 0 {
        (si.frame_size as i64 * si.num_channels as i64).clamp(0, p_time_data.len() as i64) as usize
    } else {
        0
    };
    debug_assert!(n <= MAX_FRAME_SAMPLES);
    (err, n)
}

/// `CAacDecoder_DecodeFrame()` from the element decode loop on (see the module documentation).
#[allow(clippy::cognitive_complexity)]
fn core_b(
    self_: &mut AacDecoderInstance,
    ho: &mut HandOver,
    p_time_data: &mut [PcmDec],
    time_data_size: i32,
    time_data_channel_offset: i32,
    bs: &mut FdkBitStream,
    cap: usize,
) -> AacDecoderError {
    let flags = ho.flags;
    let stream_index = 0usize;
    let mut error_status = ho.core_err;
    let aac_channels = ho.aac_channels;
    let element_count = ho.element_count;
    let channel_elements = ho.channel_elements;

    // Not in C: the inverse quantization and the element decode loop below on several threads
    // (super::chan; none: the loops below).
    let par = super::chan::decode_elements_pool(
        self_,
        cap,
        &ho.iq_list[..ho.iq_count],
        &ho.decode_list[..ho.decode_count],
    );
    let iq_done = par || super::chan::inverse_quantize_pool(self_, cap, &ho.iq_list[..ho.iq_count]);

    /* The inverse quantization at the end of CChannelElement_Read() of the elements of stage A
    (it reads and writes only the element's channel infos and spectra). */
    for i in 0..if iq_done { 0 } else { ho.iq_count } {
        let (a, n) = ho.iq_list[i];
        let (a, n) = (a as usize, n as usize);
        let mut p = channels_mut(&mut self_.p_aac_decoder_channel_info, a, n);
        let mut empty: [FixpDbl; 0] = [];
        let spec: &mut [FixpDbl] = match self_.work_buffer_core2.as_deref_mut() {
            Some(wb2) => tail_mut(wb2, a * 1024),
            None => &mut empty,
        };
        c_channel_element_inverse_quantize(&mut p, &self_.sampling_rate_info[0], spec);
    }

    /* The element decode loop (CChannelElement_Decode()). */
    for i in 0..if par { 0 } else { ho.decode_count } {
        let (el, ch, n) = ho.decode_list[i];
        // pCpeStaticData->jointStereoPersistentData.scratchBuffer = pTimeData
        decode_channel_element(self_, ch as usize, n as i32, el as usize, p_time_data);
    }

    if is_output_valid(error_status) == 0 {
        // (stage A does not hand over frames without valid output)
        return error_status;
    }

    /*
      Inverse transform
    */
    {
        let drc_ch_map = ho.drc_ch_map;

        /* Extract DRC control data and map it to channels (without bitstream delay) */
        let mapped = aac_decoder_drc_prolog(
            self_.h_drc_info.as_deref_mut(),
            bs,
            &mut self_.p_aac_decoder_static_channel_info,
            self_.pce.element_instance_tag,
            &drc_ch_map,
            aac_channels,
        );
        if mapped > 0 {
            if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) == 0 {
                /* If at least one DRC thread has been mapped to a channel there was DRC data in
                 * the bitstream. */
                self_.flags[stream_index] |= AC_DRC_PRESENT;
            } else {
                error_status = AAC_DEC_UNSUPPORTED_FORMAT;
            }
        }
        if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) != 0 {
            if let Some(d) = self_.h_drc_info.as_deref_mut() {
                aac_decoder_drc_disable(d);
            }
        }

        let reverse_ch_mapping = ho.reverse_ch_mapping;
        let mut cconceal_td_fading_applied = [0i32; NCH];

        // The channel loop with the inverse transforms on several threads (none: the loop
        // below).
        let par = super::chan::render_channels_pool(
            self_,
            cap,
            flags,
            element_count,
            &channel_elements,
            aac_channels,
            &reverse_ch_mapping,
            p_time_data,
            time_data_size,
            time_data_channel_offset,
            &mut cconceal_td_fading_applied,
            error_status,
        );
        if let Some(e) = par {
            error_status = e;
        }
        let seq_elements = if par.is_some() { 0 } else { element_count.max(0) as usize };

        let mut c: i32 = 0;
        let mut c_idx: i32 = 0;
        let mut el_channels_acc: i32 = 0;
        'els: for el in 0..seq_elements {
            /* Force frame concealment during mute release active state. */
            let mut frame_ok_but_conceal = 0;

            let type_: Mp4ElementId =
                if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA | AC_BSAC) != 0 {
                    self_.elements[el]
                } else {
                    channel_elements[el]
                };

            {
                let n_element_channels =
                    caac_decoder_get_el_channels(type_, self_.usac_stereo_config_index[el]);

                el_channels_acc += n_element_channels;

                if n_element_channels == 0 {
                    continue;
                }
            }

            let mut el_ch = 0;
            /* "c" iterates in canonical MPEG channel order */
            while c_idx < el_channels_acc {
                /* Robustness check */
                if c >= aac_channels {
                    return AAC_DEC_UNKNOWN;
                }

                let cu = c as usize;
                let rev_c = reverse_ch_mapping[cu] as usize;

                /* Setup offset for time buffer traversal. */
                let offset: i32 = fdk_ch_map_descr_get_map_value(
                    &self_.map_descr,
                    reverse_ch_mapping[c_idx as usize],
                    self_.ch_map_index as u32,
                ) as i32
                    * time_data_channel_offset;

                let spec_off = cu * 1024;

                if flags & AACDEC_FLUSH != 0 {
                    /* Clear pAacDecoderChannelInfo->pSpectralCoefficient because with
                     * AACDEC_FLUSH set it contains undefined data. */
                    if let Some(wb2) = self_.work_buffer_core2.as_deref_mut() {
                        let s = tail_mut(wb2, spec_off);
                        let n =
                            (self_.stream_info.aac_samples_per_frame.max(0) as usize).min(s.len());
                        s[..n].fill(0);
                    }
                }

                let (Some(ci), Some(sci)) = (
                    self_.p_aac_decoder_channel_info[cu].as_deref_mut(),
                    self_
                        .p_aac_decoder_static_channel_info
                        .get_mut(rev_c)
                        .and_then(|s| s.as_deref_mut()),
                ) else {
                    // The channel infos below aacChannels exist (C dereferences them).
                    c += 1;
                    c_idx += 1;
                    el_ch += 1;
                    continue;
                };

                /* if The ics info is not valid and it will be stored and used in the following
                 * concealment method, mark the frame as erroneous */
                {
                    let h_concealment_info = &sci.concealment_info;
                    let mute_release_active = (self_.frame_ok != 0
                        && (flags & AACDEC_CONCEAL) == 0)
                        && ((h_concealment_info.conceal_state >= ConcealState_Mute)
                            && (h_concealment_info.cnt_valid_frames + 1
                                <= self_.conceal_common_data.num_mute_release_frames));
                    let ics_is_invalid = get_scale_factor_bands_transmitted(&ci.ics_info)
                        > get_scale_factor_bands_total(&ci.ics_info);
                    let ics_info_usedin_fade_out =
                        !(ci.render_mode == AACDEC_RENDER_LPD && sci.last_lpd_mode == 0);
                    if ics_info_usedin_fade_out && ics_is_invalid && !mute_release_active {
                        self_.frame_ok = 0;
                    }
                }

                let mut empty: [FixpDbl; 0] = [];
                let spec: &mut [FixpDbl] = match self_.work_buffer_core2.as_deref_mut() {
                    Some(wb2) => tail_mut(wb2, spec_off),
                    None => &mut empty,
                };

                /*
                  Conceal defective spectral data
                */
                let conceal_apply_return_code = {
                    let last_lpd_mode = sci.last_lpd_mode;
                    cconcealment_apply(
                        ci,
                        sci,
                        &self_.sampling_rate_info[stream_index],
                        self_.stream_info.aac_samples_per_frame,
                        last_lpd_mode,
                        (self_.frame_ok != 0 && (flags & AACDEC_CONCEAL) == 0) as i32,
                        self_.flags[stream_index],
                        &self_.conceal_common_data,
                        spec,
                    )
                };
                if conceal_apply_return_code == -1 {
                    frame_ok_but_conceal = 1;
                }

                if flags & AACDEC_INTR != 0 {
                    /* Reset DRC control data for this channel */
                    aac_decoder_drc_init_channel_data(Some(&mut sci.drc_data));
                }
                if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
                    /* Reset DRC control data for this channel */
                    aac_decoder_drc_init_channel_data(Some(&mut sci.drc_data));
                }

                /* The DRC module demands to be called with the gain field holding the gain
                 * scale. */
                self_.ext_gain[0] = AACDEC_DRC_GAIN_SCALING as FixpDbl;

                /* DRC processing */
                if let Some(drc) = self_.h_drc_info.as_deref_mut() {
                    let mut drc_data = sci.drc_data;
                    aac_decoder_drc_apply(
                        drc,
                        self_.h_sbr_decoder.as_deref_mut(),
                        ci,
                        &mut drc_data,
                        Some(&mut self_.ext_gain[0]),
                        c,
                        self_.stream_info.aac_samples_per_frame,
                        self_.sbr_enabled as i32,
                        spec,
                    );
                    sci.drc_data = drc_data;
                }

                if time_data_size < time_data_channel_offset.wrapping_mul(self_.aac_channels) {
                    error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                    break 'els;
                }
                let out = tail_mut(p_time_data, offset.max(0) as usize);
                if self_.flush_status != 0 && (self_.flush_cnt > 0) && (flags & AACDEC_CONCEAL) == 0
                {
                    let n =
                        (self_.stream_info.aac_samples_per_frame.max(0) as usize).min(out.len());
                    out[..n].fill(0);
                } else {
                    match ci.render_mode {
                        AACDEC_RENDER_IMDCT => {
                            let mut tmp_wb: Option<Box<CWorkBufferCore1>> = None;
                            let wb1: &mut CWorkBufferCore1 =
                                match self_.p_work_buffer_core1.as_deref_mut() {
                                    Some(w) => w,
                                    None => tmp_wb.insert(Box::<CWorkBufferCore1>::default()),
                                };
                            cblock_frequency_to_time(
                                sci,
                                ci,
                                out,
                                self_.stream_info.aac_samples_per_frame as i16,
                                (self_.frame_ok != 0
                                    && (flags & AACDEC_CONCEAL) == 0
                                    && frame_ok_but_conceal == 0)
                                    as i32,
                                &mut wb1.buf[..], // pWorkBuffer1: the whole union (see the callee)
                                self_.aac_out_data_headroom,
                                self_.el_flags[el],
                                el_ch,
                                spec,
                            );

                            self_.ext_gain_delay = self_.stream_info.aac_samples_per_frame as u32;
                        }
                        AACDEC_RENDER_ELDFB => {
                            cblock_frequency_to_time_low_delay(
                                sci,
                                ci,
                                out,
                                self_.stream_info.aac_samples_per_frame as i16,
                                spec,
                            );
                            self_.ext_gain_delay = ((self_.stream_info.aac_samples_per_frame * 2
                                - self_.stream_info.aac_samples_per_frame / 2
                                - 1)
                                / 2) as u32;
                        }
                        AACDEC_RENDER_LPD => {
                            let mut tmp_wb: Option<Box<CWorkBufferCore1>> = None;
                            let wb1: &mut CWorkBufferCore1 =
                                match self_.p_work_buffer_core1.as_deref_mut() {
                                    Some(w) => w,
                                    None => tmp_wb.insert(Box::<CWorkBufferCore1>::default()),
                                };
                            clpd_render_time_signal(
                                sci,
                                ci,
                                out,
                                self_.stream_info.aac_samples_per_frame,
                                &self_.sampling_rate_info[stream_index],
                                (self_.frame_ok != 0
                                    && (flags & AACDEC_CONCEAL) == 0
                                    && frame_ok_but_conceal == 0)
                                    as u32,
                                self_.aac_out_data_headroom,
                                flags,
                                self_.flags[stream_index],
                                spec,
                                wb1,
                            );

                            self_.ext_gain_delay = self_.stream_info.aac_samples_per_frame as u32;
                        }
                        _ => {
                            error_status = AAC_DEC_UNKNOWN;
                        }
                    }
                }
                /* TimeDomainFading */
                if cconceal_td_fading_applied[cu] == 0 {
                    if let Some(sci_c) = self_.p_aac_decoder_static_channel_info[cu].as_deref_mut()
                    {
                        cconceal_td_fading_applied[cu] = cconcealment_td_fading(
                            self_.stream_info.aac_samples_per_frame,
                            sci_c,
                            &self_.conceal_common_data,
                            self_.aac_out_data_headroom,
                            tail_mut(p_time_data, offset.max(0) as usize),
                        );
                    }
                    if c + 1 < 8 && c < aac_channels - 1 {
                        /* update next TDNoise Seed to avoid muting in case of Parametric Stereo */
                        let seed = self_.p_aac_decoder_static_channel_info[cu]
                            .as_deref()
                            .map(|s| s.concealment_info.td_noise_seed);
                        if let (Some(seed), Some(next)) =
                            (seed, self_.p_aac_decoder_static_channel_info[cu + 1].as_deref_mut())
                        {
                            next.concealment_info.td_noise_seed = seed;
                        }
                    }
                }

                c += 1;
                c_idx += 1;
                el_ch += 1;
            }
        }

        if self_.flags[stream_index] & AC_USAC != 0 {
            let mut bs_pseudo_lr = 0;
            mpeg_surround_decoder_is_pseudo_lr(
                self_.p_mpeg_surround_decoder.as_deref(),
                &mut bs_pseudo_lr,
            );
            /* ISO/IEC 23003-3, 7.11.2.6 Modification of core decoder output (pseudo LR) */
            if (aac_channels == 2) && bs_pseudo_lr != 0 {
                let inv_sqrt2: FixpSgl = fl2fxconst_sgl(0.707106781186547f32 as f64);
                let offset2 = time_data_channel_offset.max(0) as usize;

                for i in 0..self_.stream_info.aac_samples_per_frame.max(0) as usize {
                    let (Some(&l0), Some(&r0)) = (p_time_data.get(i), p_time_data.get(i + offset2))
                    else {
                        break;
                    };
                    let l = f_mult(l0, inv_sqrt2);
                    let r = f_mult(r0, inv_sqrt2);
                    p_time_data[i] = l.wrapping_add(r);
                    p_time_data[i + offset2] = l.wrapping_sub(r);
                }
            }
        }

        /* Extract DRC control data and map it to channels (with bitstream delay) */
        let mapped = aac_decoder_drc_epilog(
            self_.h_drc_info.as_deref_mut(),
            bs,
            &mut self_.p_aac_decoder_static_channel_info,
            self_.pce.element_instance_tag,
            &drc_ch_map,
            aac_channels,
        );
        if mapped > 0 {
            if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) == 0 {
                /* If at least one DRC thread has been mapped to a channel there was DRC data in
                 * the bitstream. */
                self_.flags[stream_index] |= AC_DRC_PRESENT;
            } else {
                error_status = AAC_DEC_UNSUPPORTED_FORMAT;
            }
        }
        if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) != 0 {
            if let Some(d) = self_.h_drc_info.as_deref_mut() {
                aac_decoder_drc_disable(d);
            }
        }
    }

    /* Add additional concealment delay */
    self_.stream_info.output_delay = self_.stream_info.output_delay.wrapping_add(
        cconcealment_get_delay(Some(&self_.conceal_common_data))
            .wrapping_mul(self_.stream_info.aac_samples_per_frame as u32),
    );

    /* Map DRC data to StreamInfo structure */
    aac_decoder_drc_get_info(
        self_.h_drc_info.as_deref(),
        Some(&mut self_.stream_info.drc_pres_mode),
        Some(&mut self_.stream_info.drc_prog_ref_lev),
    );

    // (The channel type tables were reordered and the frame counter incremented by stage A.)

    error_status
}

/// `aacDecoder_DecodeFrame()` after `CAacDecoder_DecodeFrame()` and
/// `transportDec_EndAccessUnit()`, for a call of one access unit (see the module
/// documentation).
#[allow(clippy::cognitive_complexity, unused_assignments)]
fn lib_b(
    self_: &mut AacDecoderInstance,
    ho: &mut HandOver,
    td2: &mut [PcmDec],
    p_time_data: &mut [IntPcm],
    core_err: AacDecoderError,
    cap: usize,
) -> AacDecoderError {
    let flags = ho.call_flags;
    let time_data_size = ho.time_data_size;
    let mut error_status: AacDecoderError = core_err;
    let access_unit: u32 = 0;
    let num_preroll_au: u32 = 0;
    let mut apply_crossfade = 1;
    let mut pcm_limiter_scale: i32 = 0;
    let mut interleaved: i32 = 0;

    let time_data2_size: i32 = self_.time_data2_size / core::mem::size_of::<PcmDec>() as i32;
    let time_data3_size: i32 = self_.time_data2_size / core::mem::size_of::<i32>() as i32;

    'bail: {
        let mut time_data_headroom = self_.aac_out_data_headroom;

        /* if flushing for USAC DASH IPF was not possible go on with decoding preroll */
        if (self_.flags[0] & AC_USAC) != 0
            && (self_.flush_status == AACDEC_USAC_DASH_IPF_FLUSH_ON)
            && (flags & AACDEC_CONCEAL) == 0
            && (error_status != AAC_DEC_OK)
        {
            apply_crossfade = 0;
        } else
        /* USAC DASH IPF flushing possible begin */
        {
            // (transportDec_EndAccessUnit() ran in stage A)

            /* If the current pTimeData2 does not contain a valid signal, there nothing else
             * we can do, so bail. */
            if is_output_valid(error_status) == 0 {
                break 'bail;
            }

            {
                self_.stream_info.sample_rate = self_.stream_info.aac_sample_rate;
                self_.stream_info.frame_size = self_.stream_info.aac_samples_per_frame;
            }

            self_.stream_info.num_channels = self_.stream_info.aac_num_channels;

            {
                let off = (self_.stream_info.aac_samples_per_frame.max(0) as usize).min(td2.len());
                fdk_delay_apply(
                    &mut self_.usac_residual_delay,
                    &mut td2[off..],
                    self_.stream_info.frame_size as u32,
                    0,
                );
            }

            /* Setting of internal MPS state; may be reset in CAacDecoder_SyncQmfMode if
            decoder is unable to decode with user defined qmfMode */
            if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_ELD) == 0 {
                self_.mps_enable_curr = (self_.mps_enable_user != 0
                    && crate::aacdecoder_lib::is_supported_mps_config(
                        self_.stream_info.aot,
                        self_.stream_info.num_channels as u32,
                        ((self_.flags[0] & AC_MPS_PRESENT) != 0) as u32,
                    ) != 0) as u8;
            }

            if self_.qmf_domain.global_conf.qmf_domain_explicit_config == 0
                && self_.mps_enable_curr != 0
            {
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

            caac_decoder_sync_qmf_mode(self_);

            if self_.qmf_domain.global_conf.qmf_domain_explicit_config == 0
                && self_.mps_enable_curr != 0
            {
                let sac_interface = if self_.sbr_enabled != 0 && self_.h_sbr_decoder.is_some() {
                    SAC_INTERFACE_QMF
                } else {
                    SAC_INTERFACE_TIME
                };
                /* needs to be done before first SBR apply. */
                mpeg_surround_decoder_configure_qmf_domain(
                    self_.p_mpeg_surround_decoder.as_deref_mut(),
                    &mut self_.qmf_domain,
                    sac_interface,
                    self_.stream_info.aac_sample_rate as u32,
                    self_.stream_info.aot,
                );
                let gc = &mut self_.qmf_domain.global_conf;
                if gc.n_bands_analysis_requested > 0 {
                    gc.n_qmf_time_slots_requested = (self_.stream_info.aac_samples_per_frame
                        / gc.n_bands_analysis_requested as i32)
                        as u8;
                } else {
                    gc.n_qmf_time_slots_requested = 0;
                }
            }

            match fdk_qmf_domain_configure(&mut self_.qmf_domain) {
                QMF_DOMAIN_OK => {}
                QMF_DOMAIN_OUT_OF_MEMORY => {
                    error_status = AAC_DEC_OUT_OF_MEMORY;
                    break 'bail;
                }
                /* QMF_DOMAIN_INIT_ERROR and default */
                _ => {
                    let _ = QMF_DOMAIN_INIT_ERROR;
                    error_status = AAC_DEC_UNKNOWN;
                    break 'bail;
                }
            }

            /* sbr decoder */

            if (error_status != AAC_DEC_OK)
                || (flags & AACDEC_CONCEAL) != 0
                || self_.p_aac_decoder_static_channel_info[0]
                    .as_deref()
                    .map_or(0, |s| s.concealment_info.conceal_state)
                    > ConcealState_FadeIn
            {
                self_.frame_ok = 0; /* if an error has occured do concealment in the SBR
                                    decoder too */
            }

            if self_.sbr_enabled != 0 && (self_.flags[0] & AC_USAC_SCFGI3) == 0 {
                let num_core_channel = self_.stream_info.num_channels;

                /* set params */
                sbr_decoder_set_param(
                    self_.h_sbr_decoder.as_deref_mut(),
                    SBR_SYSTEM_BITSTREAM_DELAY,
                    self_.sbr_params.bs_delay,
                );
                sbr_decoder_set_param(
                    self_.h_sbr_decoder.as_deref_mut(),
                    SBR_FLUSH_DATA,
                    ((flags & AACDEC_FLUSH)
                        | if self_.flush_status != 0 && (flags & AACDEC_CONCEAL) == 0 {
                            AACDEC_FLUSH
                        } else {
                            0
                        }) as i32,
                );

                if self_.stream_info.aot == AOT_ER_AAC_ELD {
                    /* Configure QMF */
                    sbr_decoder_set_param(
                        self_.h_sbr_decoder.as_deref_mut(),
                        SBR_LD_QMF_TIME_ALIGN,
                        ((self_.flags[0] & AC_MPS_PRESENT) != 0) as i32,
                    );
                }

                {
                    let mut max_out_ch: i32 = 0;

                    let dmx_err = pcm_dmx_get_param(
                        self_.h_pcm_utils.as_deref(),
                        MAX_NUMBER_OF_OUTPUT_CHANNELS,
                        Some(&mut max_out_ch),
                    );
                    if (dmx_err == PCMDMX_OK) && (max_out_ch == 1) {
                        /* Disable PS processing if we have to create a mono output signal. */
                        self_.ps_possible = 0;
                    }
                }

                sbr_decoder_set_param(
                    self_.h_sbr_decoder.as_deref_mut(),
                    SBR_SKIP_QMF,
                    if self_.mps_enable_curr != 0 { 2 } else { 0 },
                );

                let n = (self_.stream_info.num_channels.wrapping_mul(self_.stream_info.frame_size))
                    .max(0) as usize;
                let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                let n = n.min(wb2.len()).min(td2.len());
                wb2[..n].copy_from_slice(&td2[..n]);

                /* apply SBR processing */
                // (the channels of a stereo element on two threads, see super::chan)
                let par = super::chan::sbr_par(&mut self_.chan_par, cap);
                let sbr_error = match self_.h_sbr_decoder.as_deref_mut() {
                    Some(sbr) => sbr_decoder_apply_pool(
                        sbr,
                        &mut self_.qmf_domain,
                        wb2,
                        td2,
                        time_data3_size,
                        &mut self_.stream_info.num_channels,
                        &mut self_.stream_info.sample_rate,
                        Some(&self_.map_descr),
                        self_.ch_map_index as i32,
                        self_.frame_ok as i32,
                        &mut self_.ps_possible,
                        self_.aac_out_data_headroom,
                        &mut time_data_headroom,
                        super::chan::as_sbr_par(&par),
                    ),
                    // (sbrDecoder_Apply() with a NULL handle)
                    None => SBRDEC_INVALID_ARGUMENT,
                };

                if sbr_error == SBRDEC_OK {
                    /* Update data in streaminfo structure. Assume that the SBR upsampling
                    factor is either 1, 2, 8/3 or 4. Maximum upsampling factor is 4 (CELP+SBR
                    or USAC 4:1 SBR) */
                    self_.flags[0] |= AC_SBR_PRESENT;
                    let si = &mut self_.stream_info;
                    if si.aac_sample_rate != si.sample_rate {
                        if si.aac_sample_rate >> 2 == si.sample_rate {
                            si.frame_size = si.aac_samples_per_frame >> 2;
                            si.output_delay >>= 2;
                        } else if si.aac_sample_rate >> 1 == si.sample_rate {
                            si.frame_size = si.aac_samples_per_frame >> 1;
                            si.output_delay >>= 1;
                        } else if si.aac_sample_rate.wrapping_shl(1) == si.sample_rate {
                            si.frame_size = si.aac_samples_per_frame << 1;
                            si.output_delay <<= 1;
                        } else if si.aac_sample_rate.wrapping_shl(2) == si.sample_rate {
                            si.frame_size = si.aac_samples_per_frame << 2;
                            si.output_delay <<= 2;
                        } else if si.frame_size == 768 {
                            si.frame_size = (si.aac_samples_per_frame << 3) / 3;
                            si.output_delay = (si.output_delay << 3) / 3;
                        } else {
                            error_status = AAC_DEC_SET_PARAM_FAIL;
                            break 'bail;
                        }
                    } else {
                        si.frame_size = si.aac_samples_per_frame;
                    }
                    self_.stream_info.output_delay = self_
                        .stream_info
                        .output_delay
                        .wrapping_add(sbr_decoder_get_delay(self_.h_sbr_decoder.as_deref()));

                    if self_.ps_possible != 0 {
                        self_.flags[0] |= AC_PS_PRESENT;
                    }
                    for ch_idx in num_core_channel..self_.stream_info.num_channels {
                        if (0..8).contains(&ch_idx) {
                            self_.stream_info.p_channel_type[ch_idx as usize] = ACT_FRONT;
                            self_.stream_info.p_channel_indices[ch_idx as usize] = ch_idx as u8;
                        }
                    }
                }
                if sbr_error == SBRDEC_OUTPUT_BUFFER_TOO_SMALL {
                    error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                    break 'bail;
                }
            }

            if self_.mps_enable_curr != 0 {
                let mut n_channels = self_.stream_info.num_channels;
                let mut frame_size = self_.stream_info.frame_size;
                let mut sac_interface = SAC_INTERFACE_TIME;

                if self_.sbr_enabled != 0 && self_.h_sbr_decoder.is_some() {
                    sac_interface = SAC_INTERFACE_QMF;
                }
                if self_.stream_info.aot == AOT_USAC && (self_.flags[0] & AC_USAC_SCFGI3) != 0 {
                    sac_interface = SAC_INTERFACE_TIME;
                }
                let mut err = mpeg_surround_decoder_set_param(
                    self_.p_mpeg_surround_decoder.as_deref_mut(),
                    SACDEC_INTERFACE,
                    sac_interface,
                );

                if err == 0 {
                    let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                    err = mpeg_surround_decoder_apply(
                        self_.p_mpeg_surround_decoder.as_deref_mut(),
                        &mut self_.qmf_domain,
                        wb2,
                        td2,
                        time_data3_size,
                        self_.stream_info.aac_samples_per_frame,
                        &mut n_channels,
                        &mut frame_size,
                        self_.stream_info.sample_rate,
                        self_.stream_info.aot,
                        Some(&mut self_.stream_info.p_channel_type),
                        Some(&mut self_.stream_info.p_channel_indices),
                        Some(&self_.map_descr),
                        self_.aac_out_data_headroom,
                        &mut time_data_headroom,
                    );
                }

                if err == MPS_OUTPUT_BUFFER_TOO_SMALL {
                    error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                    break 'bail;
                }
                if err == 0 {
                    /* Update output parameter */
                    self_.stream_info.num_channels = n_channels;
                    self_.stream_info.frame_size = frame_size;
                    self_.stream_info.output_delay = self_.stream_info.output_delay.wrapping_add(
                        mpeg_surround_decoder_get_delay(self_.p_mpeg_surround_decoder.as_deref()),
                    );
                    /* Save current parameter for possible concealment of next frame */
                    self_.mps_out_channels_last = n_channels;
                    self_.mps_frame_size_last = frame_size;
                } else if (self_.mps_out_channels_last > 0) && (self_.mps_frame_size_last > 0) {
                    /* Restore parameters of last frame ... */
                    self_.stream_info.num_channels = self_.mps_out_channels_last;
                    self_.stream_info.frame_size = self_.mps_frame_size_last;
                    /* ... and clear output buffer so that potentially corrupted data does
                     * not reach the framework. */
                    let n = (self_.mps_out_channels_last.wrapping_mul(self_.mps_frame_size_last))
                        .max(0) as usize;
                    let n = n.min(td2.len());
                    td2[..n].fill(0);
                    /* Additionally proclaim that this frame had errors during decoding. */
                    error_status = AAC_DEC_DECODE_FRAME_ERROR;
                } else {
                    error_status = AAC_DEC_UNKNOWN; /* no output */
                }
            }

            /* SBR decoder for Unified Stereo Config (stereoConfigIndex == 3) */

            if self_.sbr_enabled != 0 && (self_.flags[0] & AC_USAC_SCFGI3) != 0 {
                /* set params */
                sbr_decoder_set_param(
                    self_.h_sbr_decoder.as_deref_mut(),
                    SBR_SYSTEM_BITSTREAM_DELAY,
                    self_.sbr_params.bs_delay,
                );
                sbr_decoder_set_param(
                    self_.h_sbr_decoder.as_deref_mut(),
                    SBR_FLUSH_DATA,
                    ((flags & AACDEC_FLUSH)
                        | if self_.flush_status != 0 && (flags & AACDEC_CONCEAL) == 0 {
                            AACDEC_FLUSH
                        } else {
                            0
                        }) as i32,
                );

                sbr_decoder_set_param(self_.h_sbr_decoder.as_deref_mut(), SBR_SKIP_QMF, 1);

                /* apply SBR processing (input == timeData == pTimeData3 in C; with
                SBR_SKIP_QMF the SBR decoder does not read the input: an empty slice) */
                let sbr_error = match self_.h_sbr_decoder.as_deref_mut() {
                    Some(sbr) => sbr_decoder_apply(
                        sbr,
                        &mut self_.qmf_domain,
                        &[],
                        td2,
                        time_data3_size,
                        &mut self_.stream_info.num_channels,
                        &mut self_.stream_info.sample_rate,
                        Some(&self_.map_descr),
                        self_.ch_map_index as i32,
                        self_.frame_ok as i32,
                        &mut self_.ps_possible,
                        self_.aac_out_data_headroom,
                        &mut time_data_headroom,
                    ),
                    // (sbrDecoder_Apply() with a NULL handle)
                    None => SBRDEC_INVALID_ARGUMENT,
                };

                if sbr_error == SBRDEC_OK {
                    /* Update data in streaminfo structure. Assume that the SBR upsampling
                     * factor is either 1,2 or 4 */
                    self_.flags[0] |= AC_SBR_PRESENT;
                    let si = &mut self_.stream_info;
                    if si.aac_sample_rate != si.sample_rate {
                        if si.frame_size == 768 {
                            si.frame_size = (si.aac_samples_per_frame * 8) / 3;
                        } else if si.aac_sample_rate.wrapping_shl(2) == si.sample_rate {
                            si.frame_size = si.aac_samples_per_frame << 2;
                        } else {
                            si.frame_size = si.aac_samples_per_frame << 1;
                        }
                    }

                    self_.flags[0] &= !AC_PS_PRESENT;
                }
                if sbr_error == SBRDEC_OUTPUT_BUFFER_TOO_SMALL {
                    error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                    break 'bail;
                }
            }

            if PCM_OUT_HEADROOM != time_data_headroom {
                let n = self_.stream_info.frame_size.wrapping_mul(self_.stream_info.num_channels);
                // for (i = n - 1; i >= 0; i--) pTimeData3[i] >>= ...: the elements 0..n of
                // the buffer, each shifted once (the order does not matter); `n - 1` wraps
                // for n == INT_MIN, where C runs over the whole buffer.
                let end =
                    if n == i32::MIN { td2.len() } else { (n.max(0) as usize).min(td2.len()) };
                // (PCM_DEC)pTimeData3[i] >> (PCM_OUT_HEADROOM - timeDataHeadroom): the
                // amount is 5 (headroom 3) in the library.
                let sh = (PCM_OUT_HEADROOM - time_data_headroom) & 31;
                for x in &mut td2[..end] {
                    *x >>= sh;
                }
            }

            {
                if fdk_drc_dec_get_param(self_.h_uni_drc_decoder.as_deref(), DRC_DEC_IS_ACTIVE) != 0
                    && (self_.flags[0] & AC_USAC) != 0
                {
                    /* Apply DRC gains*/
                    let mut drc_delay: i32 = 0;
                    let mut needs_deinterleaving = 0;
                    let channel_gain = [0 as FixpDbl; 8];
                    let mut reverse_in_channel_map = [0i32; 8];
                    let mut reverse_out_channel_map = [0i32; 8];
                    for ch in 0..8 {
                        reverse_in_channel_map[ch] = ch as i32;
                        reverse_out_channel_map[ch] = ch as i32;
                    }

                    /* Update sampleRate and frameSize. This may be necessary in case of
                     * implicit SBR signaling */
                    fdk_drc_dec_set_param(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        DRC_DEC_SAMPLE_RATE,
                        self_.stream_info.sample_rate,
                    );
                    fdk_drc_dec_set_param(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        DRC_DEC_FRAME_SIZE,
                        self_.stream_info.frame_size,
                    );

                    /* If SBR and/or MPS is active, the DRC gains are aligned to the QMF
                    domain signal before the QMF synthesis. Therefore the DRC gains need to
                    be delayed by the QMF synthesis delay. */
                    if self_.sbr_enabled != 0 {
                        drc_delay = 257;
                    }
                    if self_.mps_enable_curr != 0 {
                        drc_delay = 257;
                    }
                    /* Take into account concealment delay */
                    drc_delay = drc_delay.wrapping_add(
                        (cconcealment_get_delay(Some(&self_.conceal_common_data)) as i32)
                            .wrapping_mul(self_.stream_info.frame_size),
                    );

                    let nch = self_.stream_info.num_channels;
                    let fs = self_.stream_info.frame_size;
                    /* The output of SBR and MPS is interleaved. Deinterleaving may be
                     * necessary for FDK_drcDec_ProcessTime, which accepts deinterleaved
                     * audio only. */
                    let work_off: usize;
                    if (nch > 1) && (self_.sbr_enabled != 0 || self_.mps_enable_curr != 0) {
                        /* interleaving/deinterleaving is performed on upper part of
                         * pTimeData2. Check if this buffer is large enough. */
                        if time_data2_size < 2 * nch * fs {
                            error_status = AAC_DEC_UNKNOWN;
                            break 'bail;
                        }
                        needs_deinterleaving = 1;
                        work_off = (nch * fs) as usize;
                        let (lo, hi) = td2.split_at_mut(work_off.min(td2.len()));
                        fdk_deinterleave(lo, hi, nch as u32, fs as u32, fs as u32);
                    } else {
                        work_off = 0;
                    }
                    let drc_work_buffer = tail_mut_dbl(td2, work_off);

                    /* prepare Loudness Normalisation gain */
                    fdk_drc_dec_set_param(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        DRC_DEC_TARGET_LOUDNESS,
                        (-(self_.default_target_loudness as i32))
                            .wrapping_mul(fl2fxconst_dbl(1.0 / (1 << 9) as f64)),
                    );
                    fdk_drc_dec_set_channel_gains(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        nch,
                        fs,
                        &channel_gain,
                        drc_work_buffer,
                        fs,
                    );
                    fdk_drc_dec_preprocess(self_.h_uni_drc_decoder.as_deref_mut());

                    /* apply DRC1 gain sequence */
                    fdk_drc_dec_process_time(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        drc_delay,
                        DRC_DEC_DRC1,
                        0,
                        0,
                        nch,
                        drc_work_buffer,
                        fs,
                    );
                    /* apply downmix */
                    fdk_drc_dec_apply_downmix(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        Some(&reverse_in_channel_map),
                        Some(&reverse_out_channel_map),
                        Some(drc_work_buffer),
                        &mut self_.stream_info.num_channels,
                    ); /* self->streamInfo.numChannels may change here */
                    /* apply DRC2/3 gain sequence */
                    fdk_drc_dec_process_time(
                        self_.h_uni_drc_decoder.as_deref_mut(),
                        drc_delay,
                        DRC_DEC_DRC2_DRC3,
                        0,
                        0,
                        self_.stream_info.num_channels,
                        drc_work_buffer,
                        fs,
                    );

                    if needs_deinterleaving != 0 {
                        let (lo, hi) = td2.split_at_mut(work_off.min(td2.len()));
                        fdk_interleave(
                            hi,
                            lo,
                            self_.stream_info.num_channels as u32,
                            fs as u32,
                            fs as u32,
                        );
                    }
                }
            }
            if fdk_drc_dec_get_param(self_.h_uni_drc_decoder.as_deref(), DRC_DEC_IS_ACTIVE) != 0 {
                /* return output loudness information for MPEG-D DRC */
                let output_loudness = fdk_drc_dec_get_param(
                    self_.h_uni_drc_decoder.as_deref(),
                    DRC_DEC_OUTPUT_LOUDNESS,
                );
                if output_loudness == DRC_DEC_LOUDNESS_NOT_PRESENT {
                    /* no valid MPEG-D DRC loudness value contained */
                    self_.stream_info.output_loudness = -1;
                } else if output_loudness > 0 {
                    /* positive output loudness values (very unusual) are limited to 0 dB */
                    self_.stream_info.output_loudness = 0;
                } else {
                    self_.stream_info.output_loudness = output_loudness.wrapping_neg() >> 22;
                    /* negate and scale from e = 7 to e = (31-2) */
                }
            } else {
                /* return output loudness information for MPEG-4 DRC */
                if self_.stream_info.drc_prog_ref_lev < 0 {
                    /* no MPEG-4 DRC loudness metadata contained */
                    self_.stream_info.output_loudness = -1;
                } else if self_.default_target_loudness < 0 {
                    /* loudness normalization is off */
                    self_.stream_info.output_loudness = self_.stream_info.drc_prog_ref_lev as i32;
                } else {
                    self_.stream_info.output_loudness = self_.default_target_loudness as i32;
                }
            }

            if self_.stream_info.ext_aot != AOT_AAC_SLS {
                interleaved = 0;
                interleaved |= (self_.sbr_enabled != 0) as i32;
                interleaved |= (self_.mps_enable_curr != 0) as i32;
                if (flags & AACDEC_INTR) != 0 && (access_unit == 0) {
                    /* delete data from the past (e.g. mixdown coeficients) */
                    pcm_dmx_reset(self_.h_pcm_utils.as_deref_mut(), PCMDMX_RESET_BS_DATA);
                }
                if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
                    /* delete data from the past (e.g. mixdown coeficients) */
                    pcm_dmx_reset(self_.h_pcm_utils.as_deref_mut(), PCMDMX_RESET_BS_DATA);
                }

                /* do PCM post processing */
                let dmx_err = pcm_dmx_apply_frame(
                    self_.h_pcm_utils.as_deref_mut(),
                    Some(td2),
                    time_data2_size,
                    self_.stream_info.frame_size as u32,
                    Some(&mut self_.stream_info.num_channels),
                    interleaved,
                    Some(&mut self_.stream_info.p_channel_type),
                    Some(&mut self_.stream_info.p_channel_indices),
                    Some(&self_.map_descr),
                    Some(&mut pcm_limiter_scale),
                );
                if dmx_err == PCMDMX_OUTPUT_BUFFER_TOO_SMALL {
                    error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                    break 'bail;
                }
                if (error_status == AAC_DEC_OK) && (dmx_err == PCMDMX_INVALID_MODE) {
                    /* Announce the framework that the current combination of channel
                    configuration and downmix settings are not know to produce a predictable
                    behavior and thus maybe produce strange output. */
                    error_status = AAC_DEC_DECODE_FRAME_ERROR;
                }
            }

            if self_.flags[0] & AC_USAC != 0 {
                if self_.flush_status == AACDEC_USAC_DASH_IPF_FLUSH_ON
                    && (flags & AACDEC_CONCEAL) == 0
                {
                    caac_decoder_prepare_cross_fade(
                        td2,
                        &mut self_.p_time_data_flush,
                        self_.stream_info.num_channels,
                        self_.stream_info.frame_size,
                        interleaved,
                    );
                }

                /* prepare crossfade buffer for fade in */
                if apply_crossfade == 0
                    && (self_.apply_crossfade != AACDEC_CROSSFADE_BITMASK_OFF)
                    && (flags & AACDEC_CONCEAL) == 0
                {
                    for ch in 0..self_.stream_info.num_channels.max(0) as usize {
                        // pTimeDataFlush[ch] is allocated for the flushed channels (C
                        // dereferences it).
                        if let Some(f) =
                            self_.p_time_data_flush.get_mut(ch).and_then(|f| f.as_deref_mut())
                        {
                            for i in 0..(TIME_DATA_FLUSH_SIZE as usize).min(f.len()) {
                                f[i] = 0;
                            }
                        }
                    }
                    apply_crossfade = 1;
                }
                let _ = apply_crossfade;

                if apply_crossfade != 0
                    && (self_.apply_crossfade != AACDEC_CROSSFADE_BITMASK_OFF)
                    && access_unit >= num_preroll_au
                    && (self_.build_up_status == crate::aacdecoder::AACDEC_USAC_BUILD_UP_ON)
                {
                    caac_decoder_apply_cross_fade(
                        td2,
                        &self_.p_time_data_flush,
                        self_.stream_info.num_channels,
                        self_.stream_info.frame_size,
                        interleaved,
                    );
                    self_.apply_crossfade = AACDEC_CROSSFADE_BITMASK_OFF; /* disable cross-fade
                                                                          between frames at
                                                                          nect config
                                                                          change */
                }
            }

            /* Signal interruption to take effect in next frame. */
            if ((flags & AACDEC_FLUSH) != 0 || self_.flush_status != 0)
                && (flags & AACDEC_CONCEAL) == 0
            {
                aac_decoder_signal_interruption(self_);
            }

            /* Update externally visible copy of flags */
            self_.stream_info.flags = self_.flags[0];
        } /* USAC DASH IPF flushing possible end */

        // (the end of the access unit loop: one access unit without pre-roll, no build-up)

        if self_.stream_info.ext_aot != AOT_AAC_SLS {
            pcm_limiter_scale += PCM_OUT_HEADROOM;

            if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
                /* Reset DRC data */
                if let Some(d) = self_.h_drc_info.as_deref_mut() {
                    aac_decoder_drc_reset(d);
                }
                /* Delete the delayed signal. */
                pcm_limiter_reset(self_.h_limiter.as_deref_mut());
            }

            /* Set applyExtGain if DRC processing is enabled and if progRefLevelPresent is
            present for the first time. Consequences: The headroom of the output signal can be
            set to AACDEC_DRC_GAIN_SCALING only for audio formats which support legacy DRC Level
            Normalization. For all other audio formats the headroom of the output signal is set
            to PCM_OUT_HEADROOM. */
            if let Some(d) = self_.h_drc_info.as_deref_mut() {
                if d.enable != 0 && (d.prog_ref_level_present == 1) {
                    d.apply_ext_gain |= 1;
                }
            }

            let nch = self_.stream_info.num_channels;
            let fs = self_.stream_info.frame_size;

            /* Check whether time data buffer is large enough. */
            if time_data_size < nch.wrapping_mul(fs) {
                error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                break 'bail;
            }

            let n = (nch.wrapping_mul(fs)).max(0) as usize;
            let drc_ext =
                self_.h_drc_info.as_deref().is_some_and(|d| d.enable != 0 && d.apply_ext_gain != 0);

            if self_.limiter_enable_curr != 0 {
                /* use workBufferCore2 buffer for interleaving */
                let block_length = fs;

                /* Set actual signal parameters */
                pcm_limiter_set_n_channels(self_.h_limiter.as_deref_mut(), nch as u32);
                pcm_limiter_set_sample_rate(
                    self_.h_limiter.as_deref_mut(),
                    self_.stream_info.sample_rate as u32,
                );

                let use_td2 = (nch == 1) || self_.sbr_enabled != 0 || self_.mps_enable_curr != 0;
                if !use_td2 {
                    /* applyLimiter requests for interleaved data */
                    /* Interleave ouput buffer */
                    let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                    fdk_interleave(td2, wb2, nch as u32, block_length as u32, fs as u32);
                }

                let mut p_gain_per_sample: Option<&mut [FixpDbl]> = None;

                if drc_ext {
                    if (crate::aac_ram::get_required_mem_work_buffer_core1() as i32)
                        < fs.wrapping_mul(core::mem::size_of::<FixpDbl>() as i32)
                    {
                        error_status = AAC_DEC_UNKNOWN;
                        break 'bail;
                    }
                    let gain = self_.work_buffer_core1.as_deref_mut().map(|w| &mut w.buf[..]);
                    let interleave_buffer: &mut [FixpDbl] = if use_td2 {
                        &mut *td2
                    } else {
                        self_.work_buffer_core2.as_deref_mut().unwrap_or(&mut [])
                    };

                    if let Some(drc) = self_.h_drc_info.as_deref_mut() {
                        pcm_limiter_scale = apply_drc_level_normalization(
                            drc,
                            interleave_buffer,
                            &self_.ext_gain,
                            gain,
                            pcm_limiter_scale,
                            self_.ext_gain_delay,
                            fs as u32,
                            nch as u32,
                            1,
                            1,
                        );
                    }
                    p_gain_per_sample =
                        self_.work_buffer_core1.as_deref_mut().map(|w| &mut w.buf[..]);
                }

                let interleave_buffer: &[FixpDbl] =
                    if use_td2 { &*td2 } else { self_.work_buffer_core2.as_deref().unwrap_or(&[]) };
                pcm_limiter_apply(
                    self_.h_limiter.as_deref_mut(),
                    interleave_buffer,
                    p_time_data,
                    p_gain_per_sample.as_deref(),
                    pcm_limiter_scale,
                    fs as u32,
                );

                {
                    /* Announce the additional limiter output delay */
                    if let Some(l) = self_.h_limiter.as_deref() {
                        self_.stream_info.output_delay =
                            self_.stream_info.output_delay.wrapping_add(pcm_limiter_get_delay(l));
                    }
                }
            } else {
                if drc_ext {
                    let stride = if interleaved != 0 || (nch == 1) { 1 } else { fs as u32 };
                    if let Some(drc) = self_.h_drc_info.as_deref_mut() {
                        pcm_limiter_scale = apply_drc_level_normalization(
                            drc,
                            td2,
                            &self_.ext_gain,
                            None,
                            pcm_limiter_scale,
                            self_.ext_gain_delay,
                            fs as u32,
                            nch as u32,
                            stride,
                            0,
                        );
                    }
                }

                /* If numChannels = 1 we do not need interleaving. The same applies if SBR or MPS
                are used, since their output is interleaved already (resampled or not) */
                if (nch == 1) || self_.sbr_enabled != 0 || self_.mps_enable_curr != 0 {
                    let m = n.min(p_time_data.len()).min(td2.len());
                    scale_values_saturate_dbl_to_sgl(
                        &mut p_time_data[..m],
                        &td2[..m],
                        pcm_limiter_scale,
                    );
                } else {
                    let wb2 = self_.work_buffer_core2.get_or_insert_with(Vec::new);
                    let pcm = as_int_pcm(wb2);
                    let m = n.min(pcm.len()).min(td2.len());
                    scale_values_saturate_dbl_to_sgl(&mut pcm[..m], &td2[..m], pcm_limiter_scale);
                    /* Interleave ouput buffer */
                    fdk_interleave_sgl(&pcm[..m], p_time_data, nch as u32, fs as u32, fs as u32);
                }
            }
        } /* if (self->streamInfo.extAot != AOT_AAC_SLS)*/
    }

    /* bail: */

    // (f_end_au_not_adjusted: only with pre-roll access units)

    /* Update Statistics */
    update_counters(&mut self_.stream_info, ho.n_bits_consumed, error_status);
    if ((self_.stream_info.num_channels <= 0)
        || (self_.stream_info.frame_size <= 0)
        || (self_.stream_info.sample_rate <= 0))
        && is_output_valid(error_status) != 0
    {
        /* Ensure consistency of IS_OUTPUT_VALID() macro. */
        error_status = AAC_DEC_UNKNOWN;
    }

    error_status
}

/// `aacDecoder_UpdateBitStreamCounters(pSi, hBs, nBits, ErrorStatus)` with the bits the
/// transport decoder consumed (`nBits - FDKgetValidBits(hBs)`).
pub(crate) fn update_counters(p_si: &mut CStreamInfo, n_bits: i32, error_status: AacDecoderError) {
    /* Calc bitrate. */
    if p_si.frame_size > 0 {
        /* bitRate = nBits * sampleRate / frameSize */
        let mut ratio_e: i32 = 0;
        let ratio_m = f_div_norm(p_si.sample_rate, p_si.frame_size, &mut ratio_e);
        p_si.bit_rate = f_mult_norm_to(n_bits, DFRACT_BITS - 1, ratio_m, ratio_e, DFRACT_BITS - 1);
    }

    /* bit/byte counters */
    {
        let n_bytes = n_bits >> 3;
        p_si.num_total_bytes = p_si.num_total_bytes.wrapping_add(n_bytes as i64);
        if is_output_valid(error_status) != 0 {
            p_si.num_total_access_units += 1;
        }
        if crate::aacdecoder_lib::is_decode_error(error_status) != 0 {
            p_si.num_bad_bytes = p_si.num_bad_bytes.wrapping_add(n_bytes as i64);
            p_si.num_bad_access_units += 1;
        }
    }
}

#[allow(unused_imports)]
use {
    fdk_aac_rs_fdk::common_fix::FRACT_BITS as _, AACDEC_DRC_GAIN_SCALING as _,
    AAC_DEC_UNSUPPORTED_FORMAT as _,
};

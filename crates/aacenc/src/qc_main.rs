// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/qc_main.h, libAACenc/src/qc_main.cpp.

//! Port of `qc_main.h`: quantizing & coding.
//!
//! The channels of element `i` of a [`QcOut`] / [`PsyOut`] are the contiguous slices that start
//! at `qc_element[i].qc_out_channel[0]` / `psy_out_element[i].psy_out_channel[0]` (see
//! [`qc_data`](crate::qc_data) and [`interface`](crate::interface)).
//!
//! # Calls into other modules
//!
//! `FDKaacEnc_ChannelElementWrite()` and `FDKaacEnc_writeExtensionData()` (counting mode, from
//! [`bitenc`](crate::bitenc)) and `transportEnc_GetStaticBits()` (libMpegTPEnc) are called
//! through the wrappers at the end of this file.

use crate::aac_enc_rom::{FDK_AAC_ENC_SIDE_INFO_TAB_LONG, FDK_AAC_ENC_SIDE_INFO_TAB_SHORT};
use crate::aacenc::*;
use crate::adj_thr::*;
use crate::adj_thr_data::*;
use crate::bit_cnt::{fdk_aac_enc_count_values, CODE_BOOK_PNS_NO};
use crate::channel_map::fdk_aac_enc_init_element_bits;
use crate::dyn_bits::{fdk_aac_enc_bc_new, fdk_aac_enc_dyn_bit_count};
use crate::interface::*;
use crate::psy_const::*;
use crate::qc_data::*;
use crate::quantize::fdk_aac_enc_quantize_spectrum;
use crate::sf_estim::{
    fdk_aac_enc_calc_form_factor, fdk_aac_enc_estimate_scale_factors_channel,
    fdk_aac_enc_estimate_scale_factors_views,
};
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::f_mult_i;

/// `AACENC_DZQ_BR_THR`: Dead zone quantizer bitrate threshold
const AACENC_DZQ_BR_THR: i32 = 32000;

/// `MAX_QUANT` (`quantize.h`)
const MAX_QUANT: i32 = 8191;

/// `TAB_VBR_QUAL_FACTOR`
#[derive(Clone, Copy)]
struct TabVbrQualFactor {
    bitrate_mode: QcdataBrMode,
    vbr_qual_factor: i32,
}

/// `tableVbrQualFactor`
static TABLE_VBR_QUAL_FACTOR: [TabVbrQualFactor; 5] = [
    // Approx.  32 kbps mono   AAC-LC + SBR + PS
    TabVbrQualFactor {
        bitrate_mode: QCDATA_BR_MODE_VBR_1,
        vbr_qual_factor: fl2fxconst_dbl(0.150f32 as f64),
    },
    // Approx.  64 kbps stereo AAC-LC + SBR
    TabVbrQualFactor {
        bitrate_mode: QCDATA_BR_MODE_VBR_2,
        vbr_qual_factor: fl2fxconst_dbl(0.162f32 as f64),
    },
    // Approx.  96 kbps stereo AAC-LC
    TabVbrQualFactor {
        bitrate_mode: QCDATA_BR_MODE_VBR_3,
        vbr_qual_factor: fl2fxconst_dbl(0.176f32 as f64),
    },
    // Approx. 128 kbps stereo AAC-LC
    TabVbrQualFactor {
        bitrate_mode: QCDATA_BR_MODE_VBR_4,
        vbr_qual_factor: fl2fxconst_dbl(0.120f32 as f64),
    },
    // Approx. 192 kbps stereo AAC-LC
    TabVbrQualFactor {
        bitrate_mode: QCDATA_BR_MODE_VBR_5,
        vbr_qual_factor: fl2fxconst_dbl(0.070f32 as f64),
    },
];

/// `isConstantBitrateMode(bitrateMode)`
fn is_constant_bitrate_mode(bitrate_mode: QcdataBrMode) -> i32 {
    if (bitrate_mode == QCDATA_BR_MODE_CBR)
        || (bitrate_mode == QCDATA_BR_MODE_SFR)
        || (bitrate_mode == QCDATA_BR_MODE_FF)
    {
        1
    } else {
        0
    }
}

/// `FRAME_LEN_RESULT_MODE`
type FrameLenResultMode = i32;
const FRAME_LEN_BYTES_MODULO: FrameLenResultMode = 1;
const FRAME_LEN_BYTES_INT: FrameLenResultMode = 2;

/// `isAudioElement(elType)`
fn is_audio_element(el_type: Mp4ElementId) -> bool {
    (el_type == ID_SCE) || (el_type == ID_CPE) || (el_type == ID_LFE)
}

/// `FDKaacEnc_calcFrameLen(bitRate, sampleRate, granuleLength, mode)`
fn fdk_aac_enc_calc_frame_len(
    bit_rate: i32,
    sample_rate: i32,
    granule_length: i32,
    mode: FrameLenResultMode,
) -> i32 {
    let mut result: i32 = (granule_length >> 3).wrapping_mul(bit_rate);

    match mode {
        FRAME_LEN_BYTES_MODULO => result = if sample_rate != 0 { result % sample_rate } else { 0 },
        FRAME_LEN_BYTES_INT => result = if sample_rate != 0 { result / sample_rate } else { 0 },
        _ => {}
    }
    result
}

/// `FDKaacEnc_framePadding(bitRate, sampleRate, granuleLength, paddingRest)`: calculates if
/// padding is needed for actual frame.
fn fdk_aac_enc_frame_padding(
    bit_rate: i32,
    sample_rate: i32,
    granule_length: i32,
    padding_rest: &mut i32,
) -> i32 {
    let mut padding_on = 0;

    let difference =
        fdk_aac_enc_calc_frame_len(bit_rate, sample_rate, granule_length, FRAME_LEN_BYTES_MODULO);
    *padding_rest = padding_rest.wrapping_sub(difference);

    if *padding_rest <= 0 {
        padding_on = 1;
        *padding_rest = padding_rest.wrapping_add(sample_rate);
    }

    padding_on
}

/// `FDKaacEnc_QCOutNew(phQC, nElements, nChannels, nSubFrames, dynamic_RAM)`
pub fn fdk_aac_enc_qc_out_new(
    ph_qc: &mut [Option<Box<QcOut>>],
    n_elements: i32,
    n_channels: i32,
    n_sub_frames: i32,
) -> AacEncoderError {
    for n in 0..n_sub_frames.max(0) as usize {
        ph_qc[n] = Some(Box::new(QcOut {
            qc_element: vec![QcOutElement::default(); n_elements.max(0) as usize],
            p_qc_out_channels: vec![QcOutChannel::default(); n_channels.max(0) as usize],
            ..Default::default()
        }));
    }
    AAC_ENC_OK
}

/// `FDKaacEnc_QCOutInit(phQC, nSubFrames, cm)`
pub fn fdk_aac_enc_qc_out_init(
    ph_qc: &mut [Option<Box<QcOut>>],
    n_sub_frames: i32,
    cm: &ChannelMapping,
) -> AacEncoderError {
    for n in 0..n_sub_frames.max(0) as usize {
        let Some(qc) = ph_qc[n].as_deref_mut() else { continue };
        let mut ch_inc: usize = 0;
        for i in 0..cm.n_elements as usize {
            for ch in 0..cm.el_info[i].n_channels_in_el as usize {
                qc.qc_element[i].qc_out_channel[ch] = ch_inc;
                ch_inc += 1;
            }
        }
    }
    AAC_ENC_OK
}

/// `FDKaacEnc_QCNew(phQC, nElements, dynamic_RAM)`
pub fn fdk_aac_enc_qc_new(ph_qc: &mut Option<Box<QcState>>, n_elements: i32) -> AacEncoderError {
    let mut h_qc = Box::<QcState>::default();

    if fdk_aac_enc_adj_thr_new(&mut h_qc.h_adj_thr, n_elements) != 0 {
        *ph_qc = None;
        return AAC_ENC_NO_MEMORY;
    }

    if fdk_aac_enc_bc_new(&mut h_qc.h_bit_counter) != 0 {
        *ph_qc = None;
        return AAC_ENC_NO_MEMORY;
    }

    h_qc.element_bits = vec![ElementBits::default(); n_elements.max(0) as usize];

    *ph_qc = Some(h_qc);
    AAC_ENC_OK
}

/// `FDKaacEnc_QCInit(hQC, init, initFlags)`
pub fn fdk_aac_enc_qc_init(h_qc: &mut QcState, init: &QcInit, init_flags: u32) -> AacEncoderError {
    h_qc.max_bits_per_frame = init.max_bits;
    h_qc.min_bits_per_frame = init.min_bits;
    h_qc.n_elements = init.channel_mapping.n_elements;
    if (init_flags != 0)
        || ((init.bitrate_mode != QCDATA_BR_MODE_FF) && (h_qc.bit_res_tot_max != init.bit_res))
    {
        h_qc.bit_res_tot = init.bit_res;
    }
    h_qc.bit_res_tot_max = init.bit_res;
    h_qc.max_bit_fac = init.max_bit_fac;
    h_qc.bitrate_mode = init.bitrate_mode;
    h_qc.inv_quant = init.inv_quant;
    h_qc.max_iterations = init.max_iterations;

    // 0: full bitreservoir, 1: reduced bitreservoir, 2: disabled bitreservoir
    h_qc.bit_res_mode = init.bit_res_mode;

    h_qc.padding.padding_rest = init.padding.padding_rest;

    h_qc.glob_hdr_bits = init.static_bits; // Bit overhead due to transport

    let cm = &init.channel_mapping;
    let n_sub = init.n_sub_frames;
    let n_eff = cm.n_channels_eff;
    let err = fdk_aac_enc_init_element_bits(
        h_qc,
        cm,
        init.bitrate,
        (if n_sub != 0 { init.average_bits.wrapping_div(n_sub) } else { 0 })
            .wrapping_sub(h_qc.glob_hdr_bits),
        if n_eff != 0 { h_qc.max_bits_per_frame.wrapping_div(n_eff) } else { 0 },
    );
    if err != AAC_ENC_OK {
        return err;
    }

    h_qc.vbr_qual_factor = fl2fxconst_dbl(0.0);
    for t in TABLE_VBR_QUAL_FACTOR.iter() {
        if h_qc.bitrate_mode == t.bitrate_mode {
            h_qc.vbr_qual_factor = t.vbr_qual_factor;
            break;
        }
    }

    // watch out here: init->bitrate is the bitrate "minus" the standard SBR bitrate
    // (=2500kbps) --> for the FDK the OFFSTE tuning should start somewhere below
    // 32000kbps-2500kbps ... so everything is fine here
    if cm.n_channels_eff == 1
        && init.bitrate.wrapping_div(cm.n_channels_eff) < AACENC_DZQ_BR_THR
        && init.is_low_delay != 0
    {
        h_qc.d_zone_quant_enable = 1;
    } else {
        h_qc.d_zone_quant_enable = 0;
    }

    let d_zone = h_qc.d_zone_quant_enable;
    let inv_quant = h_qc.inv_quant;
    let vbr_qual_factor = h_qc.vbr_qual_factor;
    fdk_aac_enc_adj_thr_init(
        &mut h_qc.h_adj_thr,
        init.mean_pe,
        inv_quant,
        cm,
        init.sample_rate,  // output sample rate
        init.bitrate,      // total bitrate
        init.is_low_delay, // if set, calc bits2PE factor depending on samplerate
        init.bit_res_mode, // for a small bitreservoir, the pe correction is calc'd differently
        d_zone,
        init.bit_distribution_mode,
        vbr_qual_factor,
    );

    AAC_ENC_OK
}

/// `FDKaacEnc_QCMainPrepare(elInfo, adjThrStateElement, psyOutElement, qcOutElement, aot,
/// syntaxFlags, epConfig)`
///
/// `psy_out_channel` / `qc_out_channel` are the channels of the element.
pub fn fdk_aac_enc_qc_main_prepare(
    el_info: &ElementInfo,
    adj_thr_state_element: &mut AtsElement,
    psy_out_element: &mut PsyOutElement,
    psy_out_channel: &mut [PsyOutChannel],
    qc_out_element: &mut QcOutElement,
    qc_out_channel: &mut [QcOutChannel],
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
) -> AacEncoderError {
    let n_channels = el_info.n_channels_in_el;

    fdk_aac_enc_calc_form_factor(qc_out_channel, psy_out_channel, n_channels);

    // prepare and calculate PE without reduction
    fdk_aac_enc_pe_calculation(
        &mut qc_out_element.pe_data,
        psy_out_channel,
        qc_out_channel,
        &psy_out_element.tools_info,
        adj_thr_state_element,
        n_channels,
    );

    channel_element_write_bits(
        el_info,
        psy_out_element,
        psy_out_channel,
        syntax_flags,
        aot,
        ep_config,
        &mut qc_out_element.static_bits_used,
        0,
    )
}

/// `FDKaacEnc_AdjustBitrate(hQC, cm, avgTotalBits, bitRate, sampleRate, granuleLength)`:
/// adjusts framelength via padding on a frame to frame basis, to achieve a bitrate that demands
/// a non byte aligned framelength.
pub fn fdk_aac_enc_adjust_bitrate(
    h_qc: &mut QcState,
    _cm: &ChannelMapping,
    avg_total_bits: &mut i32,
    bit_rate: i32,
    sample_rate: i32,
    granule_length: i32,
) -> AacEncoderError {
    // Do we need an extra padding byte?
    let padding_on = fdk_aac_enc_frame_padding(
        bit_rate,
        sample_rate,
        granule_length,
        &mut h_qc.padding.padding_rest,
    );

    let frame_len = padding_on
        + fdk_aac_enc_calc_frame_len(bit_rate, sample_rate, granule_length, FRAME_LEN_BYTES_INT);

    *avg_total_bits = frame_len.wrapping_shl(3);

    AAC_ENC_OK
}

/// `FDKaacEnc_distributeElementDynBits(hQC, qcElement, cm, codeBits)`: distributes all bits over
/// all elements. The relative bit distibution is described in the ELEMENT_INFO of the
/// appropriate element. The bit distribution table is initialized in
/// FDKaacEnc_InitChannelMapping().
fn fdk_aac_enc_distribute_element_dyn_bits(
    h_qc: &QcState,
    qc_element: &mut [QcOutElement],
    cm: &ChannelMapping,
    code_bits: i32,
) -> AacEncoderError {
    let mut total_bits: i32 = 0; // sum of bits over all elements

    for i in (0..cm.n_elements as usize).rev() {
        if is_audio_element(cm.el_info[i].el_type) {
            qc_element[i].granted_dyn_bits =
                f_max(0, f_mult_i(h_qc.element_bits[i].relative_bits_el, code_bits));
            total_bits += qc_element[i].granted_dyn_bits;
        }
    }

    // Due to inaccuracies with the multiplication, codeBits may differ from totalBits. For that
    // case, the difference must be added/substracted again to/from one element, i.e:
    // Negative differences are substracted from the element with the most bits.
    // Positive differences are added to the element with the least bits.
    if code_bits != total_bits {
        let mut el_max_bits = (cm.n_elements - 1) as usize; // element with the most bits
        let mut el_min_bits = (cm.n_elements - 1) as usize; // element with the least bits

        // Search for biggest and smallest audio element
        for i in (0..cm.n_elements as usize).rev() {
            if is_audio_element(cm.el_info[i].el_type) {
                if qc_element[i].granted_dyn_bits > qc_element[el_max_bits].granted_dyn_bits {
                    el_max_bits = i;
                }
                if qc_element[i].granted_dyn_bits < qc_element[el_min_bits].granted_dyn_bits {
                    el_min_bits = i;
                }
            }
        }
        // Compensate for bit distibution difference
        if code_bits - total_bits > 0 {
            qc_element[el_min_bits].granted_dyn_bits += code_bits - total_bits;
        } else {
            qc_element[el_max_bits].granted_dyn_bits += code_bits - total_bits;
        }
    }

    AAC_ENC_OK
}

/// `checkMinFrameBitsDemand(qcOut, minBitsPerFrame, nSubFrames)`: verify whether
/// minBitsPerFrame criterion can be satisfied (always 1 in this configuration).
fn check_min_frame_bits_demand(_min_bits_per_frame: i32, _n_sub_frames: i32) -> i32 {
    1 // all fine
}

/// `FDKaacEnc_getMinimalStaticBitdemand(cm, psyOut)`: calculate minmal size of static bits by
/// reduction, to zero spectrum and deactivating tns and MS.
fn fdk_aac_enc_get_minimal_static_bitdemand(cm: &ChannelMapping, psy_out: &mut PsyOut) -> i32 {
    let aot: AudioObjectType = AOT_AAC_LC;
    let syntax_flags: u32 = 0;
    let ep_config: i8 = -1;
    let mut bitcount = 0;

    for i in 0..cm.n_elements as usize {
        let el_info = cm.el_info[i];

        if (el_info.el_type == ID_SCE) || (el_info.el_type == ID_CPE) || (el_info.el_type == ID_LFE)
        {
            let mut min_el_bits = 0;
            let first = psy_out.psy_out_element[i].psy_out_channel[0];
            let n = el_info.n_channels_in_el.max(0) as usize;
            let (elements, channels) =
                (&mut psy_out.psy_out_element, &mut psy_out.p_psy_out_channels);
            channel_element_write_bits(
                &el_info,
                &mut elements[i],
                &mut channels[first..first + n],
                syntax_flags,
                aot,
                ep_config,
                &mut min_el_bits,
                1,
            );
            bitcount += min_el_bits;
        }
    }

    bitcount
}

/// `FDKaacEnc_prepareBitDistribution(hQC, psyOut, qcOut, cm, qcElement, avgTotalBits,
/// totalAvailableBits, avgTotalDynBits)` (one sub frame; `qcElement[0]` is
/// `qc_out.qc_element`).
fn fdk_aac_enc_prepare_bit_distribution(
    h_qc: &mut QcState,
    psy_out: &mut PsyOut,
    qc_out: &mut QcOut,
    cm: &ChannelMapping,
    avg_total_bits: i32,
    total_available_bits: &mut i32,
    avg_total_dyn_bits: &mut i32,
) -> AacEncoderError {
    // get maximal allowed dynamic bits
    qc_out.granted_dyn_bits =
        (fix_min(h_qc.max_bits_per_frame, avg_total_bits) - h_qc.glob_hdr_bits) & !7;
    qc_out.granted_dyn_bits -=
        qc_out.global_ext_bits + qc_out.static_bits + qc_out.element_ext_bits;
    qc_out.max_dyn_bits = (h_qc.max_bits_per_frame & !7)
        - (qc_out.global_ext_bits + qc_out.static_bits + qc_out.element_ext_bits);
    // assure that enough bits are available
    if (qc_out.granted_dyn_bits + h_qc.bit_res_tot) < 0 {
        // crash recovery allows to reduce static bits to a minimum
        if (qc_out.granted_dyn_bits + h_qc.bit_res_tot)
            < (fdk_aac_enc_get_minimal_static_bitdemand(cm, psy_out) - qc_out.static_bits)
        {
            return AAC_ENC_BITRES_TOO_LOW;
        }
    }

    // distribute dynamic bits to each element
    let granted = qc_out.granted_dyn_bits;
    fdk_aac_enc_distribute_element_dyn_bits(h_qc, &mut qc_out.qc_element, cm, granted);

    *avg_total_dyn_bits = 0; // frameDynBits;

    *total_available_bits = avg_total_bits;

    // sum up corrected granted PE
    qc_out.total_granted_pe_corr = 0;

    for i in 0..cm.n_elements as usize {
        let el_info = cm.el_info[i];
        let n_channels = el_info.n_channels_in_el;

        if (el_info.el_type == ID_SCE) || (el_info.el_type == ID_CPE) || (el_info.el_type == ID_LFE)
        {
            // for ( all sub frames ) ...
            let pe = &psy_out.psy_out_element[i];
            let first = pe.psy_out_channel[0];
            let n = n_channels.max(0) as usize;
            let qe = &mut qc_out.qc_element[i];
            let eb = h_qc.element_bits[i];
            let mut granted_pe = qe.granted_pe;
            let mut granted_pe_corr = qe.granted_pe_corr;
            fdk_aac_enc_distribute_bits(
                &mut h_qc.h_adj_thr,
                i,
                &psy_out.p_psy_out_channels[first..first + n],
                &mut qe.pe_data,
                &mut granted_pe,
                &mut granted_pe_corr,
                n_channels,
                pe.common_window,
                qe.granted_dyn_bits,
                eb.bit_res_level_el,
                eb.max_bit_res_bits_el,
                h_qc.max_bit_fac,
                h_qc.bit_res_mode,
            );
            qe.granted_pe = granted_pe;
            qe.granted_pe_corr = granted_pe_corr;

            *total_available_bits += eb.bit_res_level_el;
            // get total corrected granted PE
            qc_out.total_granted_pe_corr += qe.granted_pe_corr;
        } // -end- if(ID_SCE || ID_CPE || ID_LFE)
    } // -end- element loop

    *total_available_bits = f_min(h_qc.max_bits_per_frame, *total_available_bits);

    AAC_ENC_OK
}

/// `FDKaacEnc_updateUsedDynBits(sumDynBitsConsumed, qcElement, cm)`
fn fdk_aac_enc_update_used_dyn_bits(
    sum_dyn_bits_consumed: &mut i32,
    qc_element: &[QcOutElement],
    cm: &ChannelMapping,
) -> AacEncoderError {
    *sum_dyn_bits_consumed = 0;

    for i in 0..cm.n_elements as usize {
        let el_info = &cm.el_info[i];

        if (el_info.el_type == ID_SCE) || (el_info.el_type == ID_CPE) || (el_info.el_type == ID_LFE)
        {
            // sum up bits consumed
            *sum_dyn_bits_consumed += qc_element[i].dyn_bits_used;
        }
    }

    AAC_ENC_OK
}

/// `FDKaacEnc_getTotalConsumedDynBits(qcOut, nSubFrames)` (one sub frame)
fn fdk_aac_enc_get_total_consumed_dyn_bits(qc_out: &QcOut) -> i32 {
    // bit consumption not valid if dynamic bits not available in one sub frame
    if qc_out.used_dyn_bits == -1 {
        return -1;
    }
    qc_out.used_dyn_bits
}

/// `FDKaacEnc_getTotalConsumedBits(qcOut, qcElement, cm, globHdrBits, nSubFrames)` (one sub
/// frame)
fn fdk_aac_enc_get_total_consumed_bits(
    qc_out: &QcOut,
    cm: &ChannelMapping,
    glob_hdr_bits: i32,
) -> i32 {
    let mut total_used_bits = 0;

    let mut data_bits = 0;
    for i in 0..cm.n_elements as usize {
        if is_audio_element(cm.el_info[i].el_type) {
            let e = &qc_out.qc_element[i];
            data_bits += e.dyn_bits_used + e.static_bits_used + e.ext_bits_used;
        }
    }
    data_bits += qc_out.global_ext_bits;

    total_used_bits += (8 - data_bits % 8) % 8;
    total_used_bits += data_bits + glob_hdr_bits; // header bits for every frame
    total_used_bits
}

/// `FDKaacEnc_BitResRedistribution(hQC, cm, avgTotalBits)`
fn fdk_aac_enc_bit_res_redistribution(
    h_qc: &mut QcState,
    cm: &ChannelMapping,
    avg_total_bits: i32,
) -> AacEncoderError {
    // check bitreservoir fill level
    if h_qc.bit_res_tot < 0 {
        return AAC_ENC_BITRES_TOO_LOW;
    } else if h_qc.bit_res_tot > h_qc.bit_res_tot_max {
        return AAC_ENC_BITRES_TOO_HIGH;
    }
    let mut total_bits = 0;
    let mut total_bits_max = 0;

    let total_bitreservoir = f_min(h_qc.bit_res_tot, h_qc.max_bits_per_frame - avg_total_bits);
    let total_bitreservoir_max =
        f_min(h_qc.bit_res_tot_max, h_qc.max_bits_per_frame - avg_total_bits);

    for i in (0..cm.n_elements as usize).rev() {
        if is_audio_element(cm.el_info[i].el_type) {
            let eb = &mut h_qc.element_bits[i];
            eb.bit_res_level_el = f_mult_i(eb.relative_bits_el, total_bitreservoir);
            total_bits += eb.bit_res_level_el;

            eb.max_bit_res_bits_el = f_mult_i(eb.relative_bits_el, total_bitreservoir_max);
            total_bits_max += eb.max_bit_res_bits_el;
        }
    }
    for i in 0..cm.n_elements as usize {
        if is_audio_element(cm.el_info[i].el_type) {
            let eb = &mut h_qc.element_bits[i];
            let mut delta_bits = f_max(total_bitreservoir - total_bits, -eb.bit_res_level_el);
            eb.bit_res_level_el += delta_bits;
            total_bits += delta_bits;

            delta_bits = f_max(total_bitreservoir_max - total_bits_max, -eb.max_bit_res_bits_el);
            eb.max_bit_res_bits_el += delta_bits;
            total_bits_max += delta_bits;
        }
    }

    AAC_ENC_OK
}

/// `FDKaacEnc_QCMain(hQC, psyOut, qcOut, avgTotalBits, cm, aot, syntaxFlags, epConfig)` (one
/// sub frame: `psyOut[0]`, `qcOut[0]`).
pub fn fdk_aac_enc_qc_main(
    h_qc: &mut QcState,
    psy_out: &mut PsyOut,
    qc_out: &mut QcOut,
    avg_total_bits: i32,
    cm: &ChannelMapping,
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
) -> AacEncoderError {
    fdk_aac_enc_qc_main_pool(
        h_qc,
        psy_out,
        qc_out,
        avg_total_bits,
        cm,
        aot,
        syntax_flags,
        ep_config,
        None,
    )
}

/// The results of the parallel first pass of [`fdk_aac_enc_qc_main_pool`] per element and
/// channel.
#[derive(Clone, Copy, Default)]
struct FirstPass {
    /// Whether the first quantization exceeded `MAX_QUANT`, and the scratch slot with its
    /// `quantSpec` and `maxValueInSfb` (`None`: not computed or used).
    exceeded: [[Option<(bool, usize)>; 2]; 8],
    /// The speculative bit count of the first quantization: the bits and the scratch slot with
    /// its section data (`None`: not computed, used or discarded).
    dyn_bits: [[Option<(i32, usize)>; 2]; 8],
}

/// [`fdk_aac_enc_qc_main`] with the opt-in multi-threading: with `par`, the scale factor
/// estimation, the first quantization and a speculative first bit count of every channel of every
/// audio element run as one task per channel on the pool (`qc_first_pass_parallel`); the
/// sequential element loops then use these results instead of computing them, in the same order.
/// Without `par` this is the straight port.
///
/// Why the result is the sequential one:
///
/// - `FDKaacEnc_EstimateScaleFactors()` of element `i` reads the `PSY_OUT_CHANNEL`s of the element
///   and writes its `QC_OUT_CHANNEL`s and nothing else; the element loop around it does nothing
///   else, so all channels can be estimated in any order.
/// - In the first pass of the quantization loop every element starts with
///   `constraintsFulfilled[i] == 1` and `calculateQuant[i][ch] == 1`: its first inner iteration
///   quantizes every channel without calling `FDKaacEnc_reduceBitConsumption()`, from the channel's
///   own estimation (spectrum, global gain, scale factors). Nothing that runs before it in the
///   sequential order (earlier elements: their own channels, `decreaseBitConsumption`, the bit
///   counter scratch) is read by it. The side effects of a channel exceeding `MAX_QUANT` are
///   applied when the sequential loop reaches it.
/// - If no channel of the element exceeded `MAX_QUANT`, the sequential code calls
///   `FDKaacEnc_dynBitCount()` next for every channel, with exactly the inputs the task had
///   (quantized spectrum, maximum values, scale factors, the element's `PSY_OUT_CHANNEL`s). The
///   task counted on a copy of the channel's section data, which is swapped in, so the section
///   data is the one an in-place count leaves, unwritten entries included. The bit counter state
///   (`bitLookUp`, `mergeGainLookUp`) is scratch: every row and entry it reads in a call is
///   written earlier in the same call (`FDKaacEnc_buildBitLookUp()` writes rows `0..maxSfb` of
///   the group; stage 2 writes `mergeGainLookUp` at every section start before
///   `FDKaacEnc_findMaxMerge()` reads section starts), so a per-task state counts the same.
///   Otherwise the speculative counts are discarded and the sequential code counts.
/// - Everything else (reductions, crash recovery, later passes) runs sequentially as before.
/// - Errors: `FDKaacEnc_reduceBitConsumption()` can make the frame fail at element `i` before
///   the later elements are quantized. The tasks therefore quantize into copies of `quantSpec`
///   and `maxValueInSfb`, which the sequential loop copies into the channel when it reaches its
///   first quantization; a failing frame leaves the later channels as the sequential code does.
///   (The scale factor estimation of all elements precedes the quantization loop in the
///   sequential code as well.)
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_qc_main_pool(
    h_qc: &mut QcState,
    psy_out: &mut PsyOut,
    qc_out: &mut QcOut,
    avg_total_bits: i32,
    cm: &ChannelMapping,
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
    par: Option<&mut crate::par::EncPar<'_>>,
) -> AacEncoderError {
    let mut error_status: AacEncoderError;
    let mut avg_total_dyn_bits: i32 = 0; // maximal allowed dynamic bits for all frames
    let mut total_available_bits: i32 = 0;
    let n_sub_frames: i32 = 1;
    let is_cbr_adjustment = if (is_constant_bitrate_mode(h_qc.bitrate_mode) != 0)
        || (h_qc.bit_res_mode != AACENC_BR_MODE_FULL)
    {
        1
    } else {
        0
    };

    // redistribute total bitreservoir to elements
    let max_bits_per_frame = h_qc.max_bits_per_frame;
    error_status = fdk_aac_enc_bit_res_redistribution(
        h_qc,
        cm,
        if is_cbr_adjustment == 0 { max_bits_per_frame } else { avg_total_bits },
    );
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    // fastenc needs one time threshold simulation, in case of multiple frames, one more guess
    // has to be calculated

    // helper pointer: qcElement[0][i] = qcOut[0]->qcElement[i]

    // calc granted dynamic bits for sub frame and distribute it to each element
    error_status = fdk_aac_enc_prepare_bit_distribution(
        h_qc,
        psy_out,
        qc_out,
        cm,
        if is_cbr_adjustment == 0 { max_bits_per_frame } else { avg_total_bits },
        &mut total_available_bits,
        &mut avg_total_dyn_bits,
    );

    if error_status != AAC_ENC_OK {
        return error_status;
    }

    // for ( all sub frames ) ...
    // for CBR and VBR mode
    fdk_aac_enc_adjust_thresholds(&mut h_qc.h_adj_thr, qc_out, psy_out, is_cbr_adjustment, cm);

    let mut iterations = [0i32; 8];
    let mut ch_constraints_fulfilled = [[0i32; 2]; 8];
    let mut calculate_quant = [[0i32; 2]; 8];
    let mut constraints_fulfilled = [0i32; 8];

    let mut first_pass: Option<FirstPass> = None;
    let mut first_pass_scratch: Option<&mut [crate::par::ChScratch]> = None;
    if let Some(par) = par {
        first_pass =
            qc_first_pass_parallel(h_qc, psy_out, qc_out, cm, syntax_flags, par.pool, par.scratch);
        if first_pass.is_some() {
            first_pass_scratch = Some(&mut par.scratch[..]);
        }
    }

    // for ( all sub frames ) ...
    for i in 0..cm.n_elements as usize {
        let el_info = cm.el_info[i];
        let n_channels = el_info.n_channels_in_el;

        if (el_info.el_type == ID_SCE) || (el_info.el_type == ID_CPE) || (el_info.el_type == ID_LFE)
        {
            // Turn thresholds into scalefactors, optimize bit consumption and verify
            // conformance
            let pf = psy_out.psy_out_element[i].psy_out_channel[0];
            let qf = qc_out.qc_element[i].qc_out_channel[0];
            let n = n_channels.max(0) as usize;
            if first_pass.is_none() {
                let views =
                    crate::adj_thr::psy_array_views(psy_out, qc_out, pf..pf + n, qf..qf + n);
                fdk_aac_enc_estimate_scale_factors_views(
                    &psy_out.p_psy_out_channels[pf..pf + n],
                    &mut qc_out.p_qc_out_channels[qf..qf + n],
                    &views,
                    h_qc.inv_quant,
                    h_qc.d_zone_quant_enable,
                    n_channels,
                );
            }

            constraints_fulfilled[i] = 1;
            iterations[i] = 0;

            for ch in 0..n {
                ch_constraints_fulfilled[i][ch] = 1;
                calculate_quant[i][ch] = 1;
            }
        } // -end- if(ID_SCE || ID_CPE || ID_LFE)
    } // -end- element loop

    qc_out.used_dyn_bits = -1;

    let mut quantization_done: i32;
    let mut sum_dyn_bits_consumed_total: i32;
    let mut decrease_bit_consumption: i32 = -1; // no direction yet!

    // -start- Quantization loop ...
    loop {
        // until max allowed bits per frame and maxDynBits!=-1
        quantization_done = 0;

        for i in 0..cm.n_elements as usize {
            let el_info = cm.el_info[i];
            let n_channels = el_info.n_channels_in_el;
            let n = n_channels.max(0) as usize;

            if (el_info.el_type == ID_SCE)
                || (el_info.el_type == ID_CPE)
                || (el_info.el_type == ID_LFE)
            {
                let pf = psy_out.psy_out_element[i].psy_out_channel[0];
                let qf = qc_out.qc_element[i].qc_out_channel[0];
                loop {
                    // until element bits < nChannels*MIN_BUFSIZE_PER_EFF_CHAN
                    loop {
                        // until spectral values < MAX_QUANT
                        if constraints_fulfilled[i] == 0 {
                            error_status = fdk_aac_enc_reduce_bit_consumption(
                                &mut iterations[i],
                                h_qc.max_iterations,
                                if decrease_bit_consumption != 0 { 1 } else { -1 },
                                &ch_constraints_fulfilled[i],
                                &mut calculate_quant[i],
                                n_channels,
                                psy_out,
                                i,
                                qc_out,
                                i,
                                &h_qc.element_bits[i],
                                aot,
                                syntax_flags,
                                ep_config,
                            );
                            if error_status != AAC_ENC_OK {
                                return error_status;
                            }
                        }

                        constraints_fulfilled[i] = 1;

                        // quantize spectrum (per each channel)
                        for ch in 0..n {
                            ch_constraints_fulfilled[i][ch] = 1;

                            if calculate_quant[i][ch] != 0 {
                                let qc_out_ch = &mut qc_out.p_qc_out_channels[qf + ch];
                                let psy_out_ch = &psy_out.p_psy_out_channels[pf + ch];

                                // calculate quantization only if necessary
                                calculate_quant[i][ch] = 0;

                                // (the first quantization of the pass, done in parallel)
                                let pre =
                                    first_pass.as_mut().and_then(|f| f.exceeded[i][ch].take());
                                let pre = pre.zip(first_pass_scratch.as_deref_mut());
                                let exceeded = match pre {
                                    Some(((exceeded, slot), scratch)) => {
                                        // the quantization as the sequential code writes it
                                        let s = &scratch[slot];
                                        qc_out_ch.quant_spec = s.quant_spec;
                                        qc_out_ch.max_value_in_sfb = s.max_value_in_sfb;
                                        exceeded
                                    }
                                    None => {
                                        let global_gain = qc_out_ch.global_gain;
                                        let scf = qc_out_ch.scf;
                                        fdk_aac_enc_quantize_spectrum(
                                            psy_out_ch.sfb_cnt,
                                            psy_out_ch.max_sfb_per_group,
                                            psy_out_ch.sfb_per_group,
                                            &psy_out_ch.sfb_offsets,
                                            &qc_out_ch.mdct_spectrum,
                                            global_gain,
                                            &scf,
                                            &mut qc_out_ch.quant_spec,
                                            h_qc.d_zone_quant_enable,
                                        );

                                        fdk_aac_enc_calc_max_value_in_sfb(
                                            psy_out_ch.sfb_cnt,
                                            psy_out_ch.max_sfb_per_group,
                                            psy_out_ch.sfb_per_group,
                                            &psy_out_ch.sfb_offsets,
                                            &qc_out_ch.quant_spec,
                                            &mut qc_out_ch.max_value_in_sfb,
                                        ) > MAX_QUANT
                                    }
                                };
                                if exceeded {
                                    // the speculative bit counts of the element are not used
                                    if let Some(f) = first_pass.as_mut() {
                                        f.dyn_bits[i] = [None; 2];
                                    }
                                    ch_constraints_fulfilled[i][ch] = 0;
                                    constraints_fulfilled[i] = 0;
                                    // if quanizted value out of range; increase global gain!
                                    decrease_bit_consumption = 1;
                                }
                            } // if calculate_quant[i][ch]
                        } // channel loop

                        if constraints_fulfilled[i] != 0 {
                            break;
                        }
                    } // does not regard bit consumption

                    qc_out.qc_element[i].dyn_bits_used = 0; // reset dynamic bits

                    // quantization valid in current channel!
                    for ch in 0..n {
                        let qc_out_ch = &mut qc_out.p_qc_out_channels[qf + ch];
                        let psy_out_ch = &psy_out.p_psy_out_channels[pf + ch];

                        // count dynamic bits (the speculative count of the first pass if it
                        // counted the same quantization)
                        let pre = first_pass.as_mut().and_then(|f| f.dyn_bits[i][ch].take());
                        let ch_dyn_bits = match (pre, first_pass_scratch.as_deref_mut()) {
                            (Some((bits, slot)), Some(scratch)) => {
                                core::mem::swap(
                                    &mut qc_out_ch.section_data,
                                    &mut scratch[slot].section_data,
                                );
                                bits
                            }
                            _ => fdk_aac_enc_dyn_bit_count(
                                &mut h_qc.h_bit_counter,
                                &qc_out_ch.quant_spec,
                                &qc_out_ch.max_value_in_sfb,
                                Some(&qc_out_ch.scf[..]),
                                psy_out_ch.last_window_sequence,
                                psy_out_ch.sfb_cnt,
                                psy_out_ch.max_sfb_per_group,
                                psy_out_ch.sfb_per_group,
                                &psy_out_ch.sfb_offsets,
                                &mut qc_out_ch.section_data,
                                &psy_out_ch.noise_nrg,
                                &psy_out_ch.is_book,
                                &psy_out_ch.is_scale,
                                syntax_flags,
                            ),
                        };

                        // sum up dynamic channel bits
                        qc_out.qc_element[i].dyn_bits_used += ch_dyn_bits;
                    }

                    // save dynBitsUsed for correction of bits2pe relation
                    if h_qc.h_adj_thr.adj_thr_state_elem[i].dyn_bits_last == -1 {
                        h_qc.h_adj_thr.adj_thr_state_elem[i].dyn_bits_last =
                            qc_out.qc_element[i].dyn_bits_used;
                    }

                    // hold total bit consumption in present element below maximum allowed
                    let qe = &qc_out.qc_element[i];
                    if qe.dyn_bits_used
                        > ((n_channels * MIN_BUFSIZE_PER_EFF_CHAN)
                            - qe.static_bits_used
                            - qe.ext_bits_used)
                    {
                        constraints_fulfilled[i] = 0;
                    }

                    if constraints_fulfilled[i] != 0 {
                        break;
                    }
                }
            } // -end- if(ID_SCE || ID_CPE || ID_LFE)
        } // -end- element loop

        // update dynBits of current subFrame
        let mut used = 0;
        fdk_aac_enc_update_used_dyn_bits(&mut used, &qc_out.qc_element, cm);
        qc_out.used_dyn_bits = used;

        // get total consumed bits, dyn bits in all sub frames have to be valid
        sum_dyn_bits_consumed_total = fdk_aac_enc_get_total_consumed_dyn_bits(qc_out);

        if sum_dyn_bits_consumed_total == -1 {
            quantization_done = 0; // bit consumption not valid in all sub frames
        } else {
            let sum_bits_consumed_total =
                fdk_aac_enc_get_total_consumed_bits(qc_out, cm, h_qc.glob_hdr_bits);

            // in all frames are valid dynamic bits
            if ((sum_bits_consumed_total < total_available_bits)
                || sum_dyn_bits_consumed_total == 0)
                && (decrease_bit_consumption == 1)
                && check_min_frame_bits_demand(h_qc.min_bits_per_frame, n_sub_frames) != 0
            {
                quantization_done = 1; // exit bit adjustment
            }
            if sum_bits_consumed_total > total_available_bits && (decrease_bit_consumption == 0) {
                quantization_done = 0; // reset!
            }
        }

        let mut emergency_iterations = 1;
        let mut dyn_bits_overshoot = 0;

        for i in 0..cm.n_elements as usize {
            if is_audio_element(cm.el_info[i].el_type) {
                // iteration limitation
                emergency_iterations &= if iterations[i] < h_qc.max_iterations { 0 } else { 1 };
            }
        }
        // detection if used dyn bits exceeds the maximal allowed criterion
        dyn_bits_overshoot |= if qc_out.used_dyn_bits > qc_out.max_dyn_bits { 1 } else { 0 };

        if quantization_done == 0 || dyn_bits_overshoot != 0 {
            let sum_bits_consumed_total =
                fdk_aac_enc_get_total_consumed_bits(qc_out, cm, h_qc.glob_hdr_bits);

            if (sum_dyn_bits_consumed_total >= avg_total_dyn_bits)
                || (sum_dyn_bits_consumed_total == 0)
            {
                quantization_done = 1;
            }
            if emergency_iterations != 0 && (sum_bits_consumed_total < total_available_bits) {
                quantization_done = 1;
            }
            if (sum_bits_consumed_total > total_available_bits)
                || check_min_frame_bits_demand(h_qc.min_bits_per_frame, n_sub_frames) == 0
            {
                quantization_done = 0;
            }
            if (sum_bits_consumed_total < total_available_bits)
                && check_min_frame_bits_demand(h_qc.min_bits_per_frame, n_sub_frames) != 0
            {
                decrease_bit_consumption = 0;
            } else {
                decrease_bit_consumption = 1;
            }

            if dyn_bits_overshoot != 0 {
                quantization_done = 0;
                decrease_bit_consumption = 1;
            }

            // reset constraints fullfilled flags
            constraints_fulfilled = [0; 8];
            ch_constraints_fulfilled = [[0; 2]; 8];
        } // quantizationDone

        if quantization_done != 0 {
            break;
        }
    }
    // ... -end- Quantization loop

    AAC_ENC_OK
}

/// The parallel first pass of [`fdk_aac_enc_qc_main_pool`] (see there): for every channel of
/// every audio element, `FDKaacEnc_EstimateScaleFactors()`, the first quantization
/// (`FDKaacEnc_QuantizeSpectrum()`, `FDKaacEnc_calcMaxValueInSfb()`) and, if it does not exceed
/// `MAX_QUANT`, `FDKaacEnc_dynBitCount()` on a copy of the section data in `scratch`.
///
/// Returns `None` (nothing done) for a channel mapping the sequential code would not run with
/// independent channels (more than 8 elements or 2 channels per element, a channel shared by
/// two elements, an index out of range): the sequential code then runs as it is.
fn qc_first_pass_parallel(
    h_qc: &QcState,
    psy_out: &PsyOut,
    qc_out: &mut QcOut,
    cm: &ChannelMapping,
    syntax_flags: u32,
    pool: &crate::par::ThreadPool,
    scratch: &mut Vec<crate::par::ChScratch>,
) -> Option<FirstPass> {
    // the element and channel of every QC_OUT_CHANNEL, with its PSY_OUT_CHANNEL
    let n_qc = qc_out.p_qc_out_channels.len();
    let n_psy = psy_out.p_psy_out_channels.len();
    let mut owner: [Option<(usize, usize, usize)>; 8] = [None; 8];
    if !(0..=8).contains(&cm.n_elements) || n_qc > owner.len() {
        return None;
    }
    let mut n_tasks = 0;
    for i in 0..cm.n_elements as usize {
        let el_info = cm.el_info[i];
        if !((el_info.el_type == ID_SCE)
            || (el_info.el_type == ID_CPE)
            || (el_info.el_type == ID_LFE))
        {
            continue;
        }
        let n = el_info.n_channels_in_el.max(0) as usize;
        if n > 2 {
            return None;
        }
        let pf = psy_out.psy_out_element[i].psy_out_channel[0];
        let qf = qc_out.qc_element[i].qc_out_channel[0];
        for ch in 0..n {
            let (q, p) = (qf + ch, pf + ch);
            if q >= n_qc || p >= n_psy || owner[q].is_some() {
                return None;
            }
            // a PSY_OUT_CHANNEL whose arrays are another element's (adj_thr::PsyArrayViews):
            // left to the sequential code
            if psy_out.p_array_qc.get(p).is_some_and(|&a| a != q && a < n_qc) {
                return None;
            }
            owner[q] = Some((i, ch, p));
            n_tasks += 1;
        }
    }

    struct Task<'a> {
        qc: &'a mut QcOutChannel,
        psy: &'a PsyOutChannel,
        scratch: &'a mut crate::par::ChScratch,
        slot: usize,
        el: usize,
        ch: usize,
        exceeded: bool,
        dyn_bits: Option<i32>,
    }
    if scratch.len() < n_tasks {
        scratch.resize_with(n_tasks, Default::default);
    }
    let mut tasks: Vec<Task<'_>> = Vec::with_capacity(n_tasks);
    let mut scratch_iter = scratch.iter_mut().enumerate();
    for (q, qc) in qc_out.p_qc_out_channels.iter_mut().enumerate() {
        if let Some((el, ch, p)) = owner[q] {
            let (slot, scratch) = scratch_iter.next()?;
            tasks.push(Task {
                qc,
                psy: &psy_out.p_psy_out_channels[p],
                scratch,
                slot,
                el,
                ch,
                exceeded: false,
                dyn_bits: None,
            });
        }
    }

    let (inv_quant, d_zone_quant_enable) = (h_qc.inv_quant, h_qc.d_zone_quant_enable);
    crate::par::for_each_mut(Some(pool), &mut tasks, |_, t| {
        let (qc, psy) = (&mut *t.qc, t.psy);
        fdk_aac_enc_estimate_scale_factors_channel(qc, psy, inv_quant, d_zone_quant_enable);

        // the first quantization, on copies of quantSpec and maxValueInSfb (the sequential loop
        // copies them back when it reaches the channel)
        let s = &mut *t.scratch;
        s.quant_spec = qc.quant_spec;
        s.max_value_in_sfb = qc.max_value_in_sfb;
        let global_gain = qc.global_gain;
        let scf = qc.scf;
        fdk_aac_enc_quantize_spectrum(
            psy.sfb_cnt,
            psy.max_sfb_per_group,
            psy.sfb_per_group,
            &psy.sfb_offsets,
            &qc.mdct_spectrum,
            global_gain,
            &scf,
            &mut s.quant_spec,
            d_zone_quant_enable,
        );
        t.exceeded = fdk_aac_enc_calc_max_value_in_sfb(
            psy.sfb_cnt,
            psy.max_sfb_per_group,
            psy.sfb_per_group,
            &psy.sfb_offsets,
            &s.quant_spec,
            &mut s.max_value_in_sfb,
        ) > MAX_QUANT;

        t.dyn_bits = None;
        if !t.exceeded {
            s.section_data = qc.section_data;
            t.dyn_bits = Some(fdk_aac_enc_dyn_bit_count(
                &mut s.bit_counter,
                &s.quant_spec,
                &s.max_value_in_sfb,
                Some(&qc.scf[..]),
                psy.last_window_sequence,
                psy.sfb_cnt,
                psy.max_sfb_per_group,
                psy.sfb_per_group,
                &psy.sfb_offsets,
                &mut s.section_data,
                &psy.noise_nrg,
                &psy.is_book,
                &psy.is_scale,
                syntax_flags,
            ));
        }
    });

    let mut first = FirstPass::default();
    for t in &tasks {
        first.exceeded[t.el][t.ch] = Some((t.exceeded, t.slot));
        first.dyn_bits[t.el][t.ch] = t.dyn_bits.map(|b| (b, t.slot));
    }
    Some(first)
}

/// `FDKaacEnc_reduceBitConsumption(iterations, maxIterations, gainAdjustment,
/// chConstraintsFulfilled, calculateQuant, nChannels, psyOutElement, qcOut, qcOutElement, elBits,
/// aot, syntaxFlags, epConfig)`: `psy_el` / `qc_el` are the indexes of `psyOutElement` /
/// `qcOutElement`.
fn fdk_aac_enc_reduce_bit_consumption(
    iterations: &mut i32,
    max_iterations: i32,
    gain_adjustment: i32,
    ch_constraints_fulfilled: &[i32; 2],
    calculate_quant: &mut [i32; 2],
    n_channels: i32,
    psy_out: &mut PsyOut,
    psy_el: usize,
    qc_out: &mut QcOut,
    qc_el: usize,
    el_bits: &ElementBits,
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
) -> AacEncoderError {
    let n = n_channels.max(0) as usize;
    let qf = qc_out.qc_element[qc_el].qc_out_channel[0];

    // SOLVING PROBLEM
    if *iterations < max_iterations {
        // increase gain (+ next iteration)
        for ch in 0..n {
            if ch_constraints_fulfilled[ch] == 0 {
                let g = &mut qc_out.p_qc_out_channels[qf + ch].global_gain;
                *g = g.wrapping_add(gain_adjustment);
                // global gain has changed, recalculate quantization in next iteration!
                calculate_quant[ch] = 1;
            }
        }
    } else if *iterations == max_iterations {
        let qe = qc_out.qc_element[qc_el];
        if qe.dyn_bits_used == 0 {
            return AAC_ENC_QUANT_ERROR;
        } else {
            // crash recovery
            let bits_to_save = fix_max(
                (qe.dyn_bits_used + 8) - (el_bits.bit_res_level_el + qe.granted_dyn_bits),
                (qe.dyn_bits_used + qe.static_bits_used + 8) - el_bits.max_bits_el,
            );
            if bits_to_save > 0 {
                fdk_aac_enc_crash_recovery(
                    n_channels,
                    psy_out,
                    psy_el,
                    qc_out,
                    qc_el,
                    bits_to_save,
                    aot,
                    syntax_flags,
                    ep_config,
                );
            } else {
                for ch in 0..n {
                    let g = &mut qc_out.p_qc_out_channels[qf + ch].global_gain;
                    *g = g.wrapping_add(1);
                }
            }
            for ch in 0..n {
                calculate_quant[ch] = 1;
            }
        }
    } else {
        // (*iterations) > maxIterations
        return AAC_ENC_QUANT_ERROR;
    }
    *iterations += 1;

    AAC_ENC_OK
}

/// `FDKaacEnc_updateFillBits(cm, qcKernel, elBits, qcOut)` (one sub frame)
pub fn fdk_aac_enc_update_fill_bits(
    _cm: &ChannelMapping,
    qc_kernel: &QcState,
    qc_out: &mut QcOut,
) -> AacEncoderError {
    // The bit counts cannot overflow in the encoder; the arithmetic wraps as the reference
    // builds do for arbitrary values.
    let total = |q: &QcOut| {
        q.static_bits
            .wrapping_add(q.used_dyn_bits)
            .wrapping_add(q.tot_fill_bits)
            .wrapping_add(q.element_ext_bits)
            .wrapping_add(q.global_ext_bits)
    };
    match qc_kernel.bitrate_mode {
        QCDATA_BR_MODE_SFR | QCDATA_BR_MODE_FF => {}
        QCDATA_BR_MODE_VBR_1 | QCDATA_BR_MODE_VBR_2 | QCDATA_BR_MODE_VBR_3
        | QCDATA_BR_MODE_VBR_4 | QCDATA_BR_MODE_VBR_5 => {
            // precalculate alignment bits
            qc_out.tot_fill_bits = qc_out.granted_dyn_bits.wrapping_sub(qc_out.used_dyn_bits) & 7;
            qc_out.total_bits = total(qc_out);
            qc_out.tot_fill_bits = qc_out.tot_fill_bits.wrapping_add(
                fix_max(0, qc_kernel.min_bits_per_frame.wrapping_sub(qc_out.total_bits))
                    .wrapping_add(7)
                    & !7,
            );
        }
        // QCDATA_BR_MODE_CBR, QCDATA_BR_MODE_INVALID, default
        _ => {
            let bit_res_space = qc_kernel.bit_res_tot_max.wrapping_sub(qc_kernel.bit_res_tot);
            // processing fill-bits
            let delta_bit_res = qc_out.granted_dyn_bits.wrapping_sub(qc_out.used_dyn_bits);
            qc_out.tot_fill_bits = fix_max(
                delta_bit_res & 7,
                delta_bit_res.wrapping_sub(fix_max(0, bit_res_space.wrapping_sub(7)) & !7),
            );
            qc_out.total_bits = total(qc_out);
            qc_out.tot_fill_bits = qc_out.tot_fill_bits.wrapping_add(
                fix_max(0, qc_kernel.min_bits_per_frame.wrapping_sub(qc_out.total_bits))
                    .wrapping_add(7)
                    & !7,
            );
        }
    } // switch (qcKernel->bitrateMode)

    AAC_ENC_OK
}

/// `FDKaacEnc_calcMaxValueInSfb(sfbCnt, maxSfbPerGroup, sfbPerGroup, sfbOffset, quantSpectrum,
/// maxValue)`
fn fdk_aac_enc_calc_max_value_in_sfb(
    sfb_cnt: i32,
    max_sfb_per_group: i32,
    sfb_per_group: i32,
    sfb_offset: &[i32],
    quant_spectrum: &[i16],
    max_value: &mut [u32],
) -> i32 {
    let mut max_value_all: i32 = 0;

    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let k = (sfb_offs + sfb) as usize;
            let (lo, hi) = (sfb_offset[k], sfb_offset[k + 1]);
            let lines = if lo < hi { &quant_spectrum[lo as usize..hi as usize] } else { &[] };
            // fixp_abs(SHORT) is computed in INT: fixp_abs(-32768) is 32768. (The maximum does
            // not depend on the order.)
            let max_this_sfb: i32 = lines.iter().fold(0, |m, &q| fix_max((q as i32).abs(), m));

            max_value[k] = max_this_sfb as u32;
            max_value_all = fix_max(max_this_sfb, max_value_all);
        }
        if sfb_per_group <= 0 {
            // (sfbPerGroup <= 0 is not defined by the reference)
            break;
        }
        sfb_offs += sfb_per_group;
    }
    max_value_all
}

/// `FDKaacEnc_updateBitres(cm, qcKernel, qcOut)` (one sub frame)
pub fn fdk_aac_enc_update_bitres(_cm: &ChannelMapping, qc_kernel: &mut QcState, qc_out: &QcOut) {
    match qc_kernel.bitrate_mode {
        QCDATA_BR_MODE_VBR_1 | QCDATA_BR_MODE_VBR_2 | QCDATA_BR_MODE_VBR_3
        | QCDATA_BR_MODE_VBR_4 | QCDATA_BR_MODE_VBR_5 => {
            // variable bitrate
            qc_kernel.bit_res_tot = f_min(qc_kernel.max_bits_per_frame, qc_kernel.bit_res_tot_max);
        }
        // QCDATA_BR_MODE_CBR, QCDATA_BR_MODE_SFR, QCDATA_BR_MODE_INVALID, default
        _ => {
            // constant bitrate
            qc_kernel.bit_res_tot = qc_kernel.bit_res_tot.wrapping_add(
                qc_out.granted_dyn_bits.wrapping_sub(
                    qc_out
                        .used_dyn_bits
                        .wrapping_add(qc_out.tot_fill_bits)
                        .wrapping_add(qc_out.align_bits),
                ),
            );
        }
    }
}

/// `FDKaacEnc_FinalizeBitConsumption(cm, qcKernel, qcOut, qcElement, hTpEnc, aot, syntaxFlags,
/// epConfig)`
pub fn fdk_aac_enc_finalize_bit_consumption(
    _cm: &ChannelMapping,
    qc_kernel: &mut QcState,
    qc_out: &mut QcOut,
    h_tp_enc: &mut TransportEnc,
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
) -> AacEncoderError {
    // Get total consumed bits in AU
    qc_out.total_bits = qc_out.static_bits
        + qc_out.used_dyn_bits
        + qc_out.tot_fill_bits
        + qc_out.element_ext_bits
        + qc_out.global_ext_bits;

    if qc_kernel.bitrate_mode == QCDATA_BR_MODE_CBR {
        // Now we can get the exact transport bit amount, and hopefully it is equal to the
        // estimated value
        let exact_tp_bits = transport_enc_get_static_bits(h_tp_enc, qc_out.total_bits);

        if exact_tp_bits != qc_kernel.glob_hdr_bits {
            // How many bits can be take by bitreservoir
            let bitres_space = qc_kernel.bit_res_tot_max
                - (qc_kernel.bit_res_tot
                    + (qc_out.granted_dyn_bits - (qc_out.used_dyn_bits + qc_out.tot_fill_bits)));

            // Number of bits which can be moved to bitreservoir.
            let bits_to_bitres = qc_kernel.glob_hdr_bits - exact_tp_bits;
            fdk_assert!(bits_to_bitres >= 0); // is always positive

            // If bitreservoir can not take all bits, move ramaining bits to fillbits
            let mut diff_fill_bits = f_max(0, bits_to_bitres - bitres_space);

            // Assure previous alignment
            diff_fill_bits = (diff_fill_bits + 7) & !7;

            // Move as many bits as possible to bitreservoir
            qc_kernel.bit_res_tot += bits_to_bitres - diff_fill_bits;

            // Write remaing bits as fill bits
            qc_out.tot_fill_bits += diff_fill_bits;
            qc_out.total_bits += diff_fill_bits;
            qc_out.granted_dyn_bits += diff_fill_bits;

            // Get new header bits
            qc_kernel.glob_hdr_bits = transport_enc_get_static_bits(h_tp_enc, qc_out.total_bits);

            if qc_kernel.glob_hdr_bits != exact_tp_bits {
                // In previous step, fill bits and corresponding total bits were changed when
                // bitreservoir was completely filled. Now we can take the too much taken bits
                // caused by header overhead from bitreservoir.
                qc_kernel.bit_res_tot -= qc_kernel.glob_hdr_bits - exact_tp_bits;
            }
        }
    } // MODE_CBR

    // Update exact number of consumed header bits.
    qc_kernel.glob_hdr_bits = transport_enc_get_static_bits(h_tp_enc, qc_out.total_bits);

    // Save total fill bits and distribut to alignment and fill bits
    let tot_fill_bits = qc_out.tot_fill_bits;

    // fake a fill extension payload
    let mut fill_ext_payload = QcOutExtension::default();
    fill_ext_payload.type_ = EXT_FILL_DATA;
    fill_ext_payload.n_payload_bits = tot_fill_bits;

    // ask bitstream encoder how many of that bits can be written in a fill extension data
    // entity
    qc_out.tot_fill_bits =
        write_extension_data_bits(&fill_ext_payload, 0, 0, syntax_flags, aot, ep_config);

    // now distribute extra fillbits and alignbits
    let align_bits = 7
        - (qc_out.static_bits
            + qc_out.used_dyn_bits
            + qc_out.element_ext_bits
            + qc_out.tot_fill_bits
            + qc_out.global_ext_bits
            - 1)
            % 8;

    // Maybe we could remove this
    if ((align_bits + qc_out.tot_fill_bits - tot_fill_bits) == 8) && (qc_out.tot_fill_bits > 8) {
        qc_out.tot_fill_bits -= 8;
    }

    qc_out.total_bits = qc_out.static_bits
        + qc_out.used_dyn_bits
        + qc_out.tot_fill_bits
        + align_bits
        + qc_out.element_ext_bits
        + qc_out.global_ext_bits;

    if (qc_out.total_bits > qc_kernel.max_bits_per_frame)
        || (qc_out.total_bits < qc_kernel.min_bits_per_frame)
    {
        return AAC_ENC_QUANT_ERROR;
    }

    qc_out.align_bits = align_bits;

    AAC_ENC_OK
}

/// `FDKaacEnc_crashRecovery(nChannels, psyOutElement, qcOut, qcElement, bitsToSave, aot,
/// syntaxFlags, epConfig)`: fulfills constraints by means of brute force... => bits are saved by
/// cancelling out spectral lines!! (beginning at the highest frequencies). `psy_el` / `qc_el`
/// are the indexes of `psyOutElement` / `qcElement`.
fn fdk_aac_enc_crash_recovery(
    n_channels: i32,
    psy_out: &mut PsyOut,
    psy_el: usize,
    qc_out: &mut QcOut,
    qc_el: usize,
    bits_to_save: i32,
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
) {
    let mut saved_bits: i32 = 0;
    let mut bits_per_scf = [[0i32; MAX_GROUPED_SFB]; 2];
    let mut section_to_scf = [[0i32; MAX_GROUPED_SFB]; 2];
    let n = n_channels.max(0) as usize;
    let pf = psy_out.psy_out_element[psy_el].psy_out_channel[0];
    let qf = qc_out.qc_element[qc_el].qc_out_channel[0];

    // create a table which converts frq-bins to bit-demand...    [bitsPerScf]
    // ...and another one which holds the corresponding sections [sectionToScf]
    for ch in 0..n {
        let psy_ch = &psy_out.p_psy_out_channels[pf + ch];
        let qc_ch = &qc_out.p_qc_out_channels[qf + ch];
        let sfb_offset = &psy_ch.sfb_offsets;

        for sect in 0..qc_ch.section_data.no_of_sections {
            let hs = qc_ch.section_data.huffsection[sect as usize];
            let code_book = hs.code_book;

            for sfb in hs.sfb_start..hs.sfb_start + hs.sfb_cnt {
                bits_per_scf[ch][sfb as usize] = 0;
                if code_book != CODE_BOOK_PNS_NO {
                    let sfb_start_line = sfb_offset[sfb as usize];
                    let no_of_lines = sfb_offset[sfb as usize + 1] - sfb_start_line;
                    let s = sfb_start_line as usize;
                    bits_per_scf[ch][sfb as usize] =
                        fdk_aac_enc_count_values(&qc_ch.quant_spec[s..], no_of_lines, code_book);
                }
                section_to_scf[ch][sfb as usize] = sect;
            }
        }
    }

    // LOWER [maxSfb] IN BOTH CHANNELS!!
    // Attention: in case of stereo: maxSfbL == maxSfbR, GroupingL == GroupingR ;

    let mut sfb = qc_out.p_qc_out_channels[qf].section_data.max_sfb_per_group - 1;
    while sfb >= 0 {
        let psy0 = &psy_out.p_psy_out_channels[pf];
        let (sfb_cnt0, sfb_per_group0) = (psy0.sfb_cnt, psy0.sfb_per_group);
        let mut sfb_grp = 0;
        while sfb_grp < sfb_cnt0 {
            for ch in 0..n {
                let k = (sfb_grp + sfb) as usize;
                let sect = section_to_scf[ch][k] as usize;
                let last_ws = psy_out.p_psy_out_channels[pf + ch].last_window_sequence;
                let qc_ch = &mut qc_out.p_qc_out_channels[qf + ch];
                qc_ch.section_data.huffsection[sect].sfb_cnt -= 1;
                saved_bits += bits_per_scf[ch][k];

                if qc_ch.section_data.huffsection[sect].sfb_cnt == 0 {
                    saved_bits += if last_ws != SHORT_WINDOW {
                        FDK_AAC_ENC_SIDE_INFO_TAB_LONG[0] as i32
                    } else {
                        FDK_AAC_ENC_SIDE_INFO_TAB_SHORT[0] as i32
                    };
                }
            }
            if sfb_per_group0 <= 0 {
                break;
            }
            sfb_grp += sfb_per_group0;
        }

        // ...have enough bits been saved?
        if saved_bits >= bits_to_save {
            break;
        }
        sfb -= 1;
    } // sfb loop

    // if not enough bits saved, clean whole spectrum and remove side info overhead
    if sfb == -1 {
        sfb = 0;
    }

    for ch in 0..n {
        qc_out.p_qc_out_channels[qf + ch].section_data.max_sfb_per_group = sfb;
        let psy_ch = &mut psy_out.p_psy_out_channels[pf + ch];
        psy_ch.max_sfb_per_group = sfb;
        // when no spectrum is coded save tools info in bitstream
        if sfb == 0 {
            psy_ch.tns_info = Default::default();
            psy_out.psy_out_element[psy_el].tools_info = ToolsInfo::default();
        }
    }
    // dynamic bits will be updated in iteration loop

    let mut stat_bits_new: i32 = 0;
    {
        // if stop sfb has changed save bits in side info, e.g. MS or TNS coding
        let el_info = ElementInfo {
            n_channels_in_el: n_channels,
            el_type: if n_channels == 2 { ID_CPE } else { ID_SCE },
            ..Default::default()
        };

        let (elements, channels) = (&mut psy_out.psy_out_element, &mut psy_out.p_psy_out_channels);
        channel_element_write_bits(
            &el_info,
            &mut elements[psy_el],
            &mut channels[pf..pf + n],
            syntax_flags,
            aot,
            ep_config,
            &mut stat_bits_new,
            0,
        );
    }

    let qe = &mut qc_out.qc_element[qc_el];
    saved_bits = qe.static_bits_used - stat_bits_new;

    // update static and dynamic bits
    qe.static_bits_used -= saved_bits;
    qe.granted_dyn_bits += saved_bits;

    qc_out.static_bits -= saved_bits;
    qc_out.granted_dyn_bits += saved_bits;
    qc_out.max_dyn_bits += saved_bits;
}

/// `FDKaacEnc_QCClose(phQCstate, phQC)`
pub fn fdk_aac_enc_qc_close(
    ph_qc_state: Option<&mut Option<Box<QcState>>>,
    ph_qc: Option<&mut [Option<Box<QcOut>>]>,
) {
    if let Some(p) = ph_qc {
        for q in p.iter_mut().take(1) {
            *q = None;
        }
    }
    if let Some(p) = ph_qc_state {
        *p = None;
    }
}

// ------------------------------------------------------------------------------------------
// Calls into the bitstream writer (bitenc) and the transport encoder (libMpegTPEnc).
// ------------------------------------------------------------------------------------------

/// `HANDLE_TRANSPORTENC`
pub use fdk_aac_rs_tpenc::tpenc_lib::TransportEnc;

use fdk_aac_rs_tpenc::tpenc_lib::transport_enc_get_static_bits;

thread_local! {
    static CHANNEL_ELEMENT_WRITE_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Number of calls of `FDKaacEnc_ChannelElementWrite()` (counting mode) from this module on this
/// thread: `FDKaacEnc_QCMainPrepare()`, `FDKaacEnc_getMinimalStaticBitdemand()` and
/// `FDKaacEnc_crashRecovery()`. Lets tests tell which frames reached the latter two.
pub fn channel_element_write_calls() -> u64 {
    CHANNEL_ELEMENT_WRITE_CALLS.with(|c| c.get())
}

/// Adds to the counter of [`channel_element_write_calls`] (the carry of the thread pool).
pub(crate) fn add_channel_element_write_calls(n: u64) {
    CHANNEL_ELEMENT_WRITE_CALLS.with(|c| c.set(c.get().wrapping_add(n)));
}

/// `FDKaacEnc_ChannelElementWrite(NULL, elInfo, NULL, psyOutElement, psyOutChannel, syntaxFlags,
/// aot, epConfig, pBitDemand, minCnt)`: counts the static bits of an element without writing.
#[allow(clippy::too_many_arguments)]
fn channel_element_write_bits(
    el_info: &ElementInfo,
    psy_out_element: &mut PsyOutElement,
    psy_out_channel: &mut [PsyOutChannel],
    syntax_flags: u32,
    aot: AudioObjectType,
    ep_config: i8,
    p_bit_demand: &mut i32,
    min_cnt: u8,
) -> AacEncoderError {
    CHANNEL_ELEMENT_WRITE_CALLS.with(|c| c.set(c.get() + 1));
    crate::bitenc::fdk_aac_enc_channel_element_write(
        None,
        el_info,
        None,
        psy_out_element,
        psy_out_channel,
        syntax_flags,
        aot,
        ep_config,
        Some(p_bit_demand),
        min_cnt,
    )
}

/// `FDKaacEnc_writeExtensionData(NULL, pExtension, elInstanceTag, alignAnchor, syntaxFlags, aot,
/// epConfig)`: counts the bits of an extension payload without writing.
fn write_extension_data_bits(
    p_extension: &QcOutExtension,
    el_instance_tag: i32,
    align_anchor: u32,
    syntax_flags: u32,
    aot: AudioObjectType,
    ep_config: i8,
) -> i32 {
    // SAFETY: without a transport encoder the payload is not dereferenced.
    unsafe {
        crate::bitenc::fdk_aac_enc_write_extension_data(
            None,
            p_extension,
            el_instance_tag,
            align_anchor,
            syntax_flags,
            aot,
            ep_config,
        )
    }
}

// FDKaacEnc_calcMaxValueInSfb with the band folded in any order against the straight loop.
#[cfg(test)]
mod tests {
    use super::*;

    fn calc_max_value_in_sfb_straight(
        sfb_cnt: i32,
        max_sfb_per_group: i32,
        sfb_per_group: i32,
        sfb_offset: &[i32],
        quant_spectrum: &[i16],
        max_value: &mut [u32],
    ) -> i32 {
        let mut max_value_all: i32 = 0;
        let mut sfb_offs = 0;
        while sfb_offs < sfb_cnt {
            for sfb in 0..max_sfb_per_group {
                let mut max_this_sfb: i32 = 0;
                let k = (sfb_offs + sfb) as usize;
                for line in sfb_offset[k]..sfb_offset[k + 1] {
                    let tmp = (quant_spectrum[line as usize] as i32).abs();
                    max_this_sfb = fix_max(tmp, max_this_sfb);
                }
                max_value[k] = max_this_sfb as u32;
                max_value_all = fix_max(max_this_sfb, max_value_all);
            }
            if sfb_per_group <= 0 {
                break;
            }
            sfb_offs += sfb_per_group;
        }
        max_value_all
    }

    #[test]
    fn calc_max_value_in_sfb_vs_straight() {
        let mut seed = 0x0f0f_1234_5678_9abcu64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        for _ in 0..20_000 {
            let per = 1 + (next() % 49) as i32;
            let groups = 1 + (next() % 3) as i32;
            let sfb_cnt = per * groups;
            let max_sfb = (next() % (per as u32 + 1)) as i32;
            let mut offsets = vec![0i32; sfb_cnt as usize + 1];
            for b in 1..offsets.len() {
                offsets[b] = (offsets[b - 1] + (next() % 20) as i32).min(1024);
            }
            let q: Vec<i16> = (0..1024)
                .map(|_| match next() % 4 {
                    0 => next() as i16,
                    1 => [0, 1, -1, i16::MIN, i16::MAX][(next() % 5) as usize],
                    _ => (next() as i16) >> (next() % 16),
                })
                .collect();
            let (mut a, mut b) = (vec![7u32; 64 * 3], vec![7u32; 64 * 3]);
            let ra = fdk_aac_enc_calc_max_value_in_sfb(sfb_cnt, max_sfb, per, &offsets, &q, &mut a);
            let rb = calc_max_value_in_sfb_straight(sfb_cnt, max_sfb, per, &offsets, &q, &mut b);
            assert_eq!((ra, &a), (rb, &b));
        }
    }
}

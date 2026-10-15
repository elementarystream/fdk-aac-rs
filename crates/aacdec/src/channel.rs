// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/channel.h, libAACdec/src/channel.cpp.

//! Port of `channel.h`: reading (`CChannelElement_Read()`) and decoding
//! (`CChannelElement_Decode()`) of a channel element.
//!
//! # Memory
//!
//! The functions follow the rules of [`crate::channelinfo`]: `p` holds the channels of the
//! element, `spec` the spectrum of the element (channel `i` at `spec[i * 1024..]`), `com` its
//! `pComData`, `com_static` its `pComStaticData`, `cpe` the `pCpeStaticData` of its first static
//! channel info (`None` for `NULL`).
//!
//! `com` is the `pComData` of the first channel. The decoder allocates one `pComData` per channel
//! element of its configuration, so the channels of a CPE in the bitstream have different
//! `pComData` where the configuration has single channel elements in their place (a CPE decoded
//! into the channels of two SCEs). C then reaches the second channel's `pComData` wherever it
//! goes through `pAacDecoderChannelInfo[ch]` (the PNS inter channel data and random seeds, the
//! RVLC and HCR scratch data); the port passes it as `com_r` (`None` where both channels share
//! `com`). The joint stereo data is always the first channel's.
//!
//! The bitstream `hBs` of `CChannelElement_Read()` is the bitstream of the transport decoder
//! (`transportDec_GetBitstream(hInput, 0)`) in the library. The port passes it separately from
//! the transport decoder; the CRC functions of the transport decoder, which read the position of
//! its bitstream, get the current position copied in and out (`FDK_BITSTREAM` is a value).
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `channel.cpp` apart from
//! the expansion of `FDK_ASSERT` and the shared header primitives.

use fdk_aac_rs_fdk::audio::{
    AudioObjectType, AC_ELD, AC_EL_LFE, AC_EL_USAC_CP_POSSIBLE, AC_EL_USAC_NOISE, AC_ER_HCR,
    AC_ER_RVLC, AC_HDAAC, AC_RSV603DA, AC_RSVD50, AC_SCALABLE, AC_USAC, ID_CPE, ID_SCE,
};
use fdk_aac_rs_fdk::bitstream::{fdk_read_bit, fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::mdct::BLOCK_LONG;
use fdk_aac_rs_fdk::tools_rom::{get_bitstream_element_list, ElementList, RbdIdT};
use fdk_aac_rs_tpdec::tpdec_lib::{
    transport_dec_crc_end_reg, transport_dec_crc_start_reg, TransportDec,
};

use crate::aac_rom::AAC_CODE_BOOK_DESCRIPTION_TABLE;
use crate::aacdec_hcr::chcr_read;
use crate::aacdec_pns::{cpns_is_pns_used, cpns_set_correlation, cpns_update_noise_state};
use crate::aacdec_tns::{
    ctns_read, ctns_read_data_present_flag, ctns_read_data_present_usac, ctns_reset,
};
use crate::block::{
    apply_tools, cblock_apply_noise, cblock_decode_huffman_word,
    cblock_inverse_quantize_spectral_data_fast, cblock_read_ac_spectral_data,
    cblock_read_scale_factor_data_fast, cblock_read_section_data_fast,
    cblock_read_spectral_data_fast, cblock_scale_spectral_data_fast,
};
use crate::channelinfo::*;
use crate::pulsedata::cpulse_data_read;
use crate::rvlc::{crvlc_decode, crvlc_element_check, crvlc_read};
use crate::stereo::{cjoint_stereo_apply_is, cjoint_stereo_apply_ms, cjoint_stereo_read};
use crate::usacdec_const::LFAC;
use crate::usacdec_fac::clpd_fac_read;
use crate::usacdec_lpd::{
    clpd_channel_stream_decode, clpd_channel_stream_read, clpd_fac_get_length,
};

/// `L`
const L: usize = 0;
/// `R`
const R: usize = 1;

/// The spectrum of channel `ch` of the element (`pSpectralCoefficient`).
#[inline]
fn ch_spec(spec: &mut [FixpDbl], ch: usize) -> &mut [FixpDbl] {
    let off = (ch * 1024).min(spec.len());
    &mut spec[off..]
}

/// `pAacDecoderChannelInfo[ch]->pComData`: `com_r` for the second channel where it has a
/// `pComData` of its own, else `com` (see the [module documentation](self)).
fn ch_com<'a>(
    com: &'a mut CAacDecoderCommonData,
    com_r: &'a mut Option<&mut CAacDecoderCommonData>,
    ch: usize,
) -> &'a mut CAacDecoderCommonData {
    match com_r {
        Some(c) if ch == R => c,
        _ => com,
    }
}

/// `MapMidSideMaskToPnsCorrelation(pAacDecoderChannelInfo[2])` (static in C). Writes the PNS
/// inter channel data of the element's `pComData` (`com`).
pub fn map_mid_side_mask_to_pns_correlation(
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    com: &mut CAacDecoderCommonData,
) {
    for group in 0..p_aac_decoder_channel_info[L].ics_info.window_groups as i32 {
        let group_mask: u8 = (1i32 << (group & 31)) as u8;

        let mut band: u8 = 0;
        while band < p_aac_decoder_channel_info[L].ics_info.max_sf_bands {
            let ms = com.joint_stereo_data.ms_used.get(band as usize).copied().unwrap_or(0);
            if ms & group_mask != 0 {
                /* channels are correlated */
                cpns_set_correlation(&mut com.pns_inter_channel_data, group, band as i32, 0);

                if cpns_is_pns_used(
                    &p_aac_decoder_channel_info[L].data.aac.pns_data,
                    group,
                    band as i32,
                ) != 0
                    && cpns_is_pns_used(
                        &p_aac_decoder_channel_info[R].data.aac.pns_data,
                        group,
                        band as i32,
                    ) != 0
                {
                    com.joint_stereo_data.ms_used[band as usize] ^= group_mask; /* clear the groupMask-bit */
                }
            }
            band = band.wrapping_add(1);
            if band == 0 {
                break;
            }
        }
    }
}

/// `Clean_Complex_Prediction_coefficients(pJointStereoPersistentData, windowGroups, low_limit,
/// high_limit)` (static in C)
pub fn clean_complex_prediction_coefficients(
    p_joint_stereo_persistent_data: &mut CJointStereoPersistentData,
    window_groups: i32,
    low_limit: i32,
    high_limit: i32,
) {
    for group in 0..window_groups.clamp(0, JointStereoMaximumGroups) {
        for sfb in low_limit.max(0)..high_limit.min(JointStereoMaximumBands) {
            p_joint_stereo_persistent_data.alpha_q_re_prev[group as usize][sfb as usize] = 0;
            p_joint_stereo_persistent_data.alpha_q_im_prev[group as usize][sfb as usize] = 0;
        }
    }
}

/// `CChannelElement_Decode(pAacDecoderChannelInfo[2], pAacDecoderStaticChannelInfo[2],
/// pSamplingRateInfo, flags, elFlags, el_channels)`: decode a channel element. To be called after
/// [`c_channel_element_read`].
///
/// `p` and `ps` hold the `el_channels` channels of the element. Further memory (see the
/// [module documentation](self)): `spec`, `com`, `com_r`, `com_static`, `cpe`, `scratch`, the joint
/// stereo scratch buffer (`jointStereoPersistentData.scratchBuffer`, the decoder's time data
/// buffer, at least 2048 values), and `current_seed`, the PNS seed of the element
/// (`*pAacDecoderChannelInfo[0]->data.aac.PnsData.currentSeed`: the `pnsCurrentSeed` of the
/// static channel info that [`crate::channelinfo::CPnsData::current_seed`] of the first channel
/// names; see [`crate::channelinfo::aacdec_pns_types`]).
pub fn c_channel_element_decode(
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    p_aac_decoder_static_channel_info: &mut [&mut CAacDecoderStaticChannelInfo],
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    el_flags: u32,
    el_channels: i32,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
    mut com_r: Option<&mut CAacDecoderCommonData>,
    com_static: &mut CAacDecoderCommonStaticData,
    mut cpe: Option<&mut CpePersistentData>,
    scratch: &mut [FixpDbl],
    current_seed: &mut i32,
) {
    let p = p_aac_decoder_channel_info;
    let ps = p_aac_decoder_static_channel_info;
    let el_channels = el_channels.clamp(0, p.len().min(ps.len()) as i32);
    let usac = flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA) != 0;

    let mut max_sf_bands_l: i32 = 0;
    let mut max_sf_bands_r: i32 = 0;
    let mut maybe_jstereo: i32 = (el_channels > 1) as i32;

    if usac && el_channels == 2 {
        if p[L].data.usac.core_mode != 0 || p[R].data.usac.core_mode != 0 {
            maybe_jstereo = 0;
        }
    }

    if maybe_jstereo != 0 {
        max_sf_bands_l = get_scale_factor_bands_transmitted(&p[L].ics_info) as i32;
        max_sf_bands_r = get_scale_factor_bands_transmitted(&p[R].ics_info) as i32;

        /* apply ms */
        if p[L].p_dyn_data.raw_data_info.common_window != 0 {
            if !usac {
                if p[L].data.aac.pns_data.pns_active != 0 || p[R].data.aac.pns_data.pns_active != 0
                {
                    map_mid_side_mask_to_pns_correlation(p, com);
                }
            }
            /* if tns_on_lr == 1 run MS */
            if (usac && (p[L].p_dyn_data.specific_to.usac.tns_on_lr == 1)) || !usac {
                let max_sfb_ste = p[L].ics_info.max_sfb_ste as i32;
                let offsets = get_scale_factor_band_offsets(&p[L].ics_info, p_sampling_rate_info);
                let wgl = *get_window_group_length_table(&p[L].ics_info);
                let groups = get_window_groups(&p[L].ics_info) as i32;

                cjoint_stereo_apply_ms(
                    &mut p[..2],
                    &mut ps[..2],
                    offsets,
                    &wgl,
                    groups,
                    max_sfb_ste,
                    max_sf_bands_l,
                    max_sf_bands_r,
                    1,
                    spec,
                    com,
                    com_static,
                    cpe.as_deref_mut(),
                    scratch,
                );
            } /* if ( ((elFlags & AC_EL_USAC_CP_POSSIBLE).... */
        } /* if (pAacDecoderChannelInfo[L]->pDynData->RawDataInfo.CommonWindow)*/

        /* apply intensity stereo */
        /* modifies pAacDecoderChannelInfo[]->aSpecSfb */
        if !usac {
            if (p[L].p_dyn_data.raw_data_info.common_window == 1) && (el_channels == 2) {
                let offsets = get_scale_factor_band_offsets(&p[L].ics_info, p_sampling_rate_info);
                let wgl = *get_window_group_length_table(&p[L].ics_info);
                let groups = get_window_groups(&p[L].ics_info) as i32;
                let sfbt = get_scale_factor_bands_transmitted(&p[L].ics_info) as i32;
                cjoint_stereo_apply_is(&mut p[..2], offsets, &wgl, groups, sfbt, spec, com);
            }
        }
    } /* maybe_stereo */

    for ch in 0..el_channels as usize {
        if p[ch].render_mode == AACDEC_RENDER_LPD {
            /* Decode LPD data */
            clpd_channel_stream_decode(&mut *p[ch], &mut *ps[ch], flags, ch_spec(spec, ch));
        } else {
            let mut no_sfbs: u8 = get_scale_factor_bands_transmitted(&p[ch].ics_info);
            /* For USAC common window: max_sfb of both channels may differ (common_max_sfb ==
             * 0). */
            if (maybe_jstereo == 1) && (p[L].p_dyn_data.raw_data_info.common_window == 1) {
                no_sfbs = max_sf_bands_l.max(max_sf_bands_r) as u8;
            }
            let mut cp_active: i32 = 0;
            if el_flags & AC_EL_USAC_CP_POSSIBLE != 0 {
                cp_active = com.joint_stereo_data.cplx_pred_flag as i32;
            }

            /* Omit writing of pAacDecoderChannelInfo[ch]->specScale for complex stereo
            prediction since scaling has already been carried out. */
            let max_sfb_ste = p[L].ics_info.max_sfb_ste as i32;

            let tns_on_lr = p[L].p_dyn_data.specific_to.usac.tns_on_lr;
            if !(cp_active != 0 && (max_sfb_ste == no_sfbs as i32))
                || !(cp_active != 0 && p[ch].p_dyn_data.tns_data.active == 0)
                || (usac && (tns_on_lr == 0))
            {
                cblock_scale_spectral_data_fast(
                    &mut *p[ch],
                    no_sfbs,
                    p_sampling_rate_info,
                    ch_spec(spec, ch),
                );

                /*Active for the case of TNS applied before MS/CP*/
                if usac && (tns_on_lr == 0) {
                    let pc = &mut *p[ch];
                    if is_long_block(&pc.ics_info) != 0 {
                        for i in 0..(no_sfbs as usize).min(128) {
                            pc.p_dyn_data.a_sfb_scale[i] = pc.spec_scale[0];
                        }
                    } else {
                        for i in 0..8 {
                            for j in 0..no_sfbs as usize {
                                if let Some(s) = pc.p_dyn_data.a_sfb_scale.get_mut(i * 16 + j) {
                                    *s = pc.spec_scale[i];
                                }
                            }
                        }
                    }
                }
            }
        }
    } /* End "for (ch = 0; ch < el_channels; ch++)" */

    if maybe_jstereo != 0 {
        /* apply ms */
        if p[L].p_dyn_data.raw_data_info.common_window != 0 {
        }
        /* CommonWindow */
        else {
            if el_flags & AC_EL_USAC_CP_POSSIBLE != 0 {
                // pCpeStaticData is allocated for these elements (C dereferences it).
                if let Some(c) = cpe.as_deref_mut() {
                    c.joint_stereo_persistent_data.alpha_q_re_prev =
                        [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize];
                    c.joint_stereo_persistent_data.alpha_q_im_prev =
                        [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize];
                }
            }
        }
    } /* if (maybe_jstereo) */

    for ch in 0..el_channels as usize {
        if p[ch].render_mode == AACDEC_RENDER_LPD {
        } else {
            if !usac {
                /* Use same seed for coupled channels (CPE) */
                let pns_ch = if ch > 0 { L } else { ch };
                // The seeds are passed to ApplyTools() below (the current seed of the element's
                // first channel, the channel's pComData->pnsRandomSeed).
                let seed = p[pns_ch].data.aac.pns_data.current_seed;
                cpns_update_noise_state(&mut p[ch].data.aac.pns_data, seed);
            }

            if (flags & AC_USAC) == 0
                || ((flags & AC_USAC) != 0 && (p[L].p_dyn_data.specific_to.usac.tns_active == 1))
                || (maybe_jstereo == 0)
            {
                let common_window = p[L].p_dyn_data.raw_data_info.common_window as i32;
                apply_tools(
                    &mut p[..],
                    p_sampling_rate_info,
                    flags,
                    el_flags,
                    ch as i32,
                    common_window,
                    spec,
                    ch_com(com, &mut com_r, ch),
                    current_seed,
                );
            }
        } /* End "} else" */
    } /* End "for (ch = 0; ch < el_channels; ch++)" */

    if maybe_jstereo != 0 {
        /* apply ms */
        if p[L].p_dyn_data.raw_data_info.common_window != 0 {
            /* if tns_on_lr == 0 run MS */
            if usac && (p[L].p_dyn_data.specific_to.usac.tns_on_lr == 0) {
                let max_sfb_ste = p[L].ics_info.max_sfb_ste as i32;
                let offsets = get_scale_factor_band_offsets(&p[L].ics_info, p_sampling_rate_info);
                let wgl = *get_window_group_length_table(&p[L].ics_info);
                let groups = get_window_groups(&p[L].ics_info) as i32;

                cjoint_stereo_apply_ms(
                    &mut p[..2],
                    &mut ps[..2],
                    offsets,
                    &wgl,
                    groups,
                    max_sfb_ste,
                    max_sf_bands_l,
                    max_sf_bands_r,
                    1,
                    spec,
                    com,
                    com_static,
                    cpe.as_deref_mut(),
                    scratch,
                );
            }
        } /* if (pAacDecoderChannelInfo[L]->pDynData->RawDataInfo.CommonWindow) */
    } /* if (maybe_jstereo) */

    for _ch in 0..el_channels {
        if el_flags & AC_EL_USAC_CP_POSSIBLE != 0 {
            if let Some(c) = cpe.as_deref_mut() {
                c.joint_stereo_persistent_data.clear_spectral_coeffs = 0;
            }
        }
    }

    crvlc_element_check(&mut p[..], Some(&mut ps[..]), flags, el_channels, com);
}

/// `CChannel_CodebookTableInit(pAacDecoderChannelInfo)`: init the codebook of the SFBs (section
/// data) with `ESCBOOK`, for bitstreams without section data. This has the effect that up to
/// the amount of transmitted SFBs are treated as non-zero.
pub fn c_channel_codebook_table_init(p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo) {
    let p = p_aac_decoder_channel_info;
    let max_sfb = get_scale_factor_bands_transmitted(&p.ics_info) as usize;
    let (max_bands, max_windows) = if is_long_block(&p.ics_info) != 0 { (64, 1) } else { (16, 8) };

    let cb = &mut p.p_dyn_data.a_code_book;
    let mut base = 0usize;
    for _w in 0..max_windows {
        let mut b = 0;
        while b < max_sfb {
            // max_sfb is at most 63 (long) / 15 (short); writes outside the array (no defined
            // reference result) are dropped.
            if let Some(c) = cb.get_mut(base + b) {
                *c = ESCBOOK;
            }
            b += 1;
        }
        while b < max_bands {
            if let Some(c) = cb.get_mut(base + b) {
                *c = ZERO_HCB;
            }
            b += 1;
        }
        base += max_bands;
    }
}

/// `transportDec_CrcStartReg(pTpDec, mBits)` on the transport decoder whose bitstream is `h_bs`
/// (see the [module documentation](self)).
fn crc_start_reg(tp: &mut TransportDec, h_bs: &mut FdkBitStream, m_bits: i32) -> i32 {
    tp.bit_stream[0] = h_bs.clone();
    let r = transport_dec_crc_start_reg(tp, m_bits);
    *h_bs = tp.bit_stream[0].clone();
    r
}

/// `transportDec_CrcEndReg(pTpDec, reg)` on the transport decoder whose bitstream is `h_bs`.
fn crc_end_reg(tp: &mut TransportDec, h_bs: &mut FdkBitStream, reg: i32) {
    tp.bit_stream[0] = h_bs.clone();
    transport_dec_crc_end_reg(tp, reg);
    *h_bs = tp.bit_stream[0].clone();
}

/// `CChannelElement_Read(hBs, pAacDecoderChannelInfo[], pAacDecoderStaticChannelInfo[], aot,
/// pSamplingRateInfo, flags, elFlags, frame_length, numberOfChannels, epConfig, pTpDec)`: read a
/// channel element of the given type from the bitstream (arbitrary order bitstream parser).
///
/// `p` holds the channels of the element; `ps` the static channel infos (`None` for `NULL`,
/// which the decoder passes for the CCE dummy channel); `p_tp_dec` the transport decoder (`None`
/// for `NULL`), whose bitstream `h_bs` is (see the [module documentation](self)). Further memory:
/// `spec`, `com`, `com_r`, `com_static`, `cpe`.
///
/// Without a static channel info or `pCpeStaticData` where a list item needs one, or where an
/// element list ends a `link_sequence` without a successor, the reference has no defined result;
/// the port returns `AAC_DEC_UNSUPPORTED_FORMAT` or skips the access.
pub fn c_channel_element_read(
    h_bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    p_aac_decoder_static_channel_info: Option<&mut [&mut CAacDecoderStaticChannelInfo]>,
    aot: AudioObjectType,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    el_flags: u32,
    frame_length: u32,
    number_of_channels: u8,
    ep_config: i8,
    p_tp_dec: Option<&mut TransportDec>,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
    com_r: Option<&mut CAacDecoderCommonData>,
    com_static: &mut CAacDecoderCommonStaticData,
    cpe: Option<&mut CpePersistentData>,
) -> AacDecoderError {
    let mut deferred = false;
    c_channel_element_read_impl::<true>(
        h_bs,
        p_aac_decoder_channel_info,
        p_aac_decoder_static_channel_info,
        aot,
        p_sampling_rate_info,
        flags,
        el_flags,
        frame_length,
        number_of_channels,
        ep_config,
        p_tp_dec,
        spec,
        com,
        com_r,
        com_static,
        cpe,
        &mut deferred,
    )
}

/// [`c_channel_element_read`] for the frame pipeline (`crate::pipeline`): the inverse
/// quantization at its end is left to [`c_channel_element_inverse_quantize`] (stage B), with
/// `*deferred` set, when it cannot fail. It fails only for a band with a quantized value above
/// `MAX_QUANTIZED_VALUE`, which only the escape codebook (and the codebook numbers above it) can
/// give: the others give at most 12, and pulse data adds at most 4 x 15. Otherwise (and for
/// USAC noise filling, which this decoder does not pipeline) it runs here, as in C (including
/// the early return of its error, which skips the end of the CRC regions).
#[allow(clippy::too_many_arguments)]
pub(crate) fn c_channel_element_read_deferred_iq(
    h_bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    p_aac_decoder_static_channel_info: Option<&mut [&mut CAacDecoderStaticChannelInfo]>,
    aot: AudioObjectType,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    el_flags: u32,
    frame_length: u32,
    number_of_channels: u8,
    ep_config: i8,
    p_tp_dec: Option<&mut TransportDec>,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
    com_r: Option<&mut CAacDecoderCommonData>,
    com_static: &mut CAacDecoderCommonStaticData,
    cpe: Option<&mut CpePersistentData>,
    deferred: &mut bool,
) -> AacDecoderError {
    *deferred = false;
    c_channel_element_read_impl::<false>(
        h_bs,
        p_aac_decoder_channel_info,
        p_aac_decoder_static_channel_info,
        aot,
        p_sampling_rate_info,
        flags,
        el_flags,
        frame_length,
        number_of_channels,
        ep_config,
        p_tp_dec,
        spec,
        com,
        com_r,
        com_static,
        cpe,
        deferred,
    )
}

/// The inverse quantization at the end of `CChannelElement_Read()` for the channels of an
/// element whose reading deferred it ([`c_channel_element_read_deferred_iq`]); it does not
/// fail.
pub(crate) fn c_channel_element_inverse_quantize(
    p: &mut [&mut CAacDecoderChannelInfo],
    p_sampling_rate_info: &SamplingRateInfo,
    spec: &mut [FixpDbl],
) {
    for ch in 0..p.len() {
        if p[ch].render_mode == AACDEC_RENDER_IMDCT || p[ch].render_mode == AACDEC_RENDER_ELDFB {
            /* Shows which bands are empty. */
            p[ch].p_dyn_data.band_is_noise = [1; 8 * 16];

            let error = cblock_inverse_quantize_spectral_data_fast(
                &mut *p[ch],
                p_sampling_rate_info,
                1,
                ch_spec(spec, ch),
            );
            debug_assert!(error == AAC_DEC_OK);
        }
    }
}

/// Whether the inverse quantization of the channels would fail (a band of the escape codebook
/// or above with a value of magnitude above `MAX_QUANTIZED_VALUE`; see
/// [`c_channel_element_read_deferred_iq`]). The bands and lines are those
/// `CBlock_InverseQuantizeSpectralData()` visits.
fn inverse_quantization_fails(
    p: &[&mut CAacDecoderChannelInfo],
    p_sampling_rate_info: &SamplingRateInfo,
    spec: &[FixpDbl],
) -> bool {
    for ch in 0..p.len() {
        let c = &*p[ch];
        if c.render_mode != AACDEC_RENDER_IMDCT && c.render_mode != AACDEC_RENDER_ELDFB {
            continue;
        }
        let ch_off = ch * 1024;
        let sfb_tx = get_scale_factor_bands_transmitted(&c.ics_info) as i32;
        let band_offsets = get_scale_factor_band_offsets(&c.ics_info, p_sampling_rate_info);
        let mut window: i32 = 0;
        for group in 0..get_window_groups(&c.ics_info) as i32 {
            for _w in
                0..c.ics_info.window_group_length.get(group as usize).copied().unwrap_or(0) as i32
            {
                let win_off = crate::overlapadd::spec(window, c.granule_length) as usize;
                for band in 0..sfb_tx {
                    let bnds = (group * 16 + band) as usize;
                    let sfb = (window * 16 + band) as usize;
                    if bnds >= 128 || sfb >= 128 {
                        continue;
                    }
                    let cb = c.p_dyn_data.a_code_book[bnds];
                    if cb < ESCBOOK
                        || cb == NOISE_HCB
                        || cb == INTENSITY_HCB
                        || cb == INTENSITY_HCB2
                    {
                        continue;
                    }
                    let off0 = band_offsets.get(band as usize).copied().unwrap_or(0) as i32;
                    let off1 = band_offsets.get(band as usize + 1).copied().unwrap_or(0) as i32;
                    let begin = ch_off + win_off + off0.max(0) as usize;
                    let end = (begin + (off1 - off0).max(0) as usize).min(spec.len());
                    if begin < end
                        && spec[begin..end].iter().any(|&x| x.wrapping_abs() > MAX_QUANTIZED_VALUE)
                    {
                        return true;
                    }
                }
                window += 1;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn c_channel_element_read_impl<const IQ: bool>(
    h_bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    mut p_aac_decoder_static_channel_info: Option<&mut [&mut CAacDecoderStaticChannelInfo]>,
    aot: AudioObjectType,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    el_flags: u32,
    frame_length: u32,
    number_of_channels: u8,
    ep_config: i8,
    mut p_tp_dec: Option<&mut TransportDec>,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
    mut com_r: Option<&mut CAacDecoderCommonData>,
    com_static: &mut CAacDecoderCommonStaticData,
    mut cpe: Option<&mut CpePersistentData>,
    deferred: &mut bool,
) -> AacDecoderError {
    let p = p_aac_decoder_channel_info;
    let mut error: AacDecoderError = AAC_DEC_OK;
    let mut crc_reg1: i32 = -1;
    let mut crc_reg2: i32 = -1;
    let mut ind_sw_cce_flag: i32 = 0;
    let mut num_gain_element_lists: i32 = 0;
    let usac = flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA) != 0;

    fdk_assert!((number_of_channels == 1) || (number_of_channels == 2));
    let nch = (number_of_channels as usize).min(p.len());

    'bail: {
        /* Get channel element sequence table */
        let Some(mut list): Option<&'static ElementList> =
            get_bitstream_element_list(aot, ep_config, number_of_channels, 0, el_flags)
        else {
            error = AAC_DEC_UNSUPPORTED_FORMAT;
            break 'bail;
        };

        ctns_reset(&mut p[0].p_dyn_data.tns_data);
        /* Set common window to 0 by default. If signalized in the bit stream it will be
         * overwritten later explicitely */
        p[0].p_dyn_data.raw_data_info.common_window = 0;
        if usac {
            p[0].p_dyn_data.specific_to.usac.tns_active = 0;
            p[0].p_dyn_data.specific_to.usac.tns_on_lr = 0;
        }
        if number_of_channels == 2 && nch == 2 {
            ctns_reset(&mut p[1].p_dyn_data.tns_data);
            p[1].p_dyn_data.raw_data_info.common_window = 0;
        }

        let mut cplx_pred: i32 = 0;
        if p_aac_decoder_static_channel_info.is_some() {
            if el_flags & AC_EL_USAC_CP_POSSIBLE != 0 {
                com.joint_stereo_data.cplx_pred_flag = 0;
                cplx_pred = 1;
            }
        }

        if flags & (AC_ELD | AC_SCALABLE) != 0 {
            p[0].p_dyn_data.raw_data_info.common_window = 1;
            if number_of_channels == 2 && nch == 2 {
                p[1].p_dyn_data.raw_data_info.common_window =
                    p[0].p_dyn_data.raw_data_info.common_window;
            }
        }

        /* Iterate through sequence table */
        let mut i: i32 = 0;
        let mut ch: usize = 0;
        let mut decision_bit: u32 = 0;
        loop {
            let id = list.id.get(i as usize).copied().unwrap_or(RbdIdT::end_of_sequence);
            match id {
                RbdIdT::element_instance_tag => {
                    p[0].element_instance_tag = fdk_read_bits(h_bs, 4) as u8;
                    if number_of_channels == 2 && nch == 2 {
                        p[1].element_instance_tag = p[0].element_instance_tag;
                    }
                }
                RbdIdT::common_window => {
                    let v = fdk_read_bits(h_bs, 1);
                    p[ch].p_dyn_data.raw_data_info.common_window = v as u8;
                    decision_bit = v as u8 as u32;
                    if number_of_channels == 2 && nch == 2 {
                        p[1].p_dyn_data.raw_data_info.common_window =
                            p[0].p_dyn_data.raw_data_info.common_window;
                    }
                }
                RbdIdT::ics_info => {
                    /* store last window sequence (utilized in complex stereo prediction) before
                     * reading new channel-info */
                    if cplx_pred != 0 {
                        if p[0].p_dyn_data.raw_data_info.common_window != 0 {
                            if let Some(c) = cpe.as_deref_mut() {
                                c.joint_stereo_persistent_data.win_seq_prev =
                                    p[0].ics_info.window_sequence as u8;
                                c.joint_stereo_persistent_data.win_shape_prev =
                                    p[0].ics_info.window_shape;
                            }
                        }
                    }
                    /* Read individual channel info */
                    error = ics_read(h_bs, &mut p[ch].ics_info, p_sampling_rate_info, flags);

                    if el_flags & AC_EL_LFE != 0
                        && get_window_sequence(&p[ch].ics_info) != BLOCK_LONG
                    {
                        error = AAC_DEC_PARSE_ERROR;
                    } else if number_of_channels == 2
                        && nch == 2
                        && p[0].p_dyn_data.raw_data_info.common_window != 0
                    {
                        p[1].ics_info = p[0].ics_info;
                    }
                }
                RbdIdT::common_max_sfb => {
                    if fdk_read_bit(h_bs) == 0 && nch == 2 {
                        error = ics_read_max_sfb(h_bs, &mut p[1].ics_info, p_sampling_rate_info);
                    }
                }
                RbdIdT::ltp_data_present => {
                    if fdk_read_bits(h_bs, 1) != 0 {
                        error = AAC_DEC_UNSUPPORTED_PREDICTION;
                    }
                }
                RbdIdT::ms => {
                    let max_sfb_ste: i32 = if nch == 2 {
                        get_scale_max_factor_bands_transmitted(&p[0].ics_info, &p[1].ics_info)
                            as i32
                    } else {
                        get_scale_factor_bands_transmitted(&p[0].ics_info) as i32
                    };

                    let max_sfb_ste_clear: i32 = 64;

                    p[0].ics_info.max_sfb_ste = max_sfb_ste as u8;
                    if nch == 2 {
                        p[1].ics_info.max_sfb_ste = max_sfb_ste as u8;
                    }

                    if flags & (AC_USAC | AC_RSV603DA) != 0
                        && p[ch].p_dyn_data.raw_data_info.common_window == 0
                    {
                        if let Some(c) = cpe.as_deref_mut() {
                            clean_complex_prediction_coefficients(
                                &mut c.joint_stereo_persistent_data,
                                get_window_groups(&p[0].ics_info) as i32,
                                0,
                                64,
                            );
                        }
                    }

                    /* jointStereoPersistentData and cplxPredictionData are only
                    available/allocated if cplxPred is active. */
                    let persistent =
                        if cplx_pred == 0 || p_aac_decoder_static_channel_info.is_none() {
                            None
                        } else {
                            cpe.as_deref_mut().map(|c| &mut c.joint_stereo_persistent_data)
                        };
                    let cplx_data = if cplx_pred == 0 {
                        None
                    } else {
                        com_static.cplx_prediction_data.as_deref_mut()
                    };
                    if cjoint_stereo_read(
                        h_bs,
                        &mut com.joint_stereo_data,
                        get_window_groups(&p[0].ics_info) as i32,
                        max_sfb_ste,
                        max_sfb_ste_clear,
                        persistent,
                        cplx_data,
                        cplx_pred,
                        get_scale_factor_bands_total(&p[0].ics_info) as i32,
                        get_window_sequence(&p[0].ics_info),
                        flags,
                    ) != 0
                    {
                        error = AAC_DEC_PARSE_ERROR;
                    }
                }
                RbdIdT::global_gain => {
                    p[ch].p_dyn_data.raw_data_info.global_gain = fdk_read_bits(h_bs, 8) as u8;
                }
                RbdIdT::section_data => {
                    error = cblock_read_section_data_fast(
                        h_bs,
                        &mut *p[ch],
                        p_sampling_rate_info,
                        flags,
                    );
                }
                RbdIdT::scale_factor_data_usac | RbdIdT::scale_factor_data => {
                    if id == RbdIdT::scale_factor_data_usac {
                        p[ch].curr_aliasing_symmetry = 0;
                        /* Set active sfb codebook indexes to HCB_ESC to make them "active" */
                        c_channel_codebook_table_init(&mut *p[ch]); /*  equals ReadSectionData(self, bs) in float soft. block.c line: ~599 */
                        /* Note: The missing "break" is intentional here, since we need to call
                         * CBlock_ReadScaleFactorData(). */
                    }
                    if flags & AC_ER_RVLC != 0 {
                        /* read RVLC data from bitstream (error sens. cat. 1) */
                        crvlc_read(&mut *p[ch], h_bs, ch_com(com, &mut com_r, ch));
                    } else {
                        error = cblock_read_scale_factor_data_fast(&mut *p[ch], h_bs, flags);
                    }
                }
                RbdIdT::pulse => {
                    let pc = &mut *p[ch];
                    if cpulse_data_read(
                        h_bs,
                        &mut pc.p_dyn_data.specific_to.aac.pulse_data,
                        p_sampling_rate_info.scale_factor_bands_long, /* pulse data is only allowed to be present in long blocks! */
                        &pc.ics_info,
                        frame_length as i16,
                    ) != 0
                    {
                        error = AAC_DEC_DECODE_FRAME_ERROR;
                    }
                }
                RbdIdT::tns_data_present => {
                    ctns_read_data_present_flag(h_bs, &mut p[ch].p_dyn_data.tns_data);
                    if el_flags & AC_EL_LFE != 0 && p[ch].p_dyn_data.tns_data.data_present != 0 {
                        error = AAC_DEC_PARSE_ERROR;
                    }
                }
                RbdIdT::tns_data => {
                    /* tns_data_present is checked inside CTns_Read(). */
                    let pc = &mut *p[ch];
                    error = ctns_read(h_bs, &mut pc.p_dyn_data.tns_data, &pc.ics_info, flags);
                }
                RbdIdT::gain_control_data => {}
                RbdIdT::gain_control_data_present => {
                    if fdk_read_bits(h_bs, 1) != 0 {
                        error = AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA;
                    }
                }
                RbdIdT::tw_data => {}
                RbdIdT::common_tw => {}
                RbdIdT::tns_data_present_usac => {
                    if p[0].p_dyn_data.specific_to.usac.tns_active != 0 {
                        if nch == 2 {
                            let (a, b) = p.split_at_mut(1);
                            let p0 = &mut *a[0];
                            let p1 = &mut *b[0];
                            let common_window = p0.p_dyn_data.raw_data_info.common_window as i32;
                            let ics = p0.ics_info;
                            let mut tns_on_lr = p0.p_dyn_data.specific_to.usac.tns_on_lr;
                            ctns_read_data_present_usac(
                                h_bs,
                                &mut p0.p_dyn_data.tns_data,
                                &mut p1.p_dyn_data.tns_data,
                                &mut tns_on_lr,
                                &ics,
                                flags,
                                el_flags,
                                common_window,
                            );
                            p0.p_dyn_data.specific_to.usac.tns_on_lr = tns_on_lr;
                        }
                    } else {
                        p[0].p_dyn_data.specific_to.usac.tns_on_lr = 1;
                    }
                }
                RbdIdT::core_mode => {
                    decision_bit = fdk_read_bits(h_bs, 1);
                    p[ch].data.usac.core_mode = decision_bit as u8;
                    if (ch == 1) && (p[0].data.usac.core_mode != p[1].data.usac.core_mode) {
                        /* StereoCoreToolInfo(core_mode[ch] ) */
                        p[0].p_dyn_data.raw_data_info.common_window = 0;
                        p[1].p_dyn_data.raw_data_info.common_window = 0;
                    }
                }
                RbdIdT::tns_active => {
                    p[0].p_dyn_data.specific_to.usac.tns_active = fdk_read_bit(h_bs) as u8;
                }
                RbdIdT::noise => {
                    if el_flags & AC_EL_USAC_NOISE != 0 {
                        p[ch].p_dyn_data.specific_to.usac.fd_noise_level_and_offset =
                            fdk_read_bits(h_bs, 3 + 5) as u8; /* Noise level */
                    }
                }
                RbdIdT::lpd_channel_stream => {
                    match p_aac_decoder_static_channel_info.as_deref_mut() {
                        Some(ps) if ch < ps.len() => {
                            error = clpd_channel_stream_read(
                                h_bs,
                                &mut *p[ch],
                                &mut *ps[ch],
                                p_sampling_rate_info,
                                flags,
                                ch_spec(spec, ch),
                            );
                        }
                        _ => {
                            error = AAC_DEC_UNSUPPORTED_FORMAT;
                        }
                    }

                    p[ch].render_mode = AACDEC_RENDER_LPD;
                }
                RbdIdT::fac_data => {
                    let f_fac_dat_present = fdk_read_bit(h_bs);

                    /* Wee need a valid fac_data[0] even if no FAC data is present (as temporal
                     * buffer) */
                    p[ch].data.usac.fac_data[0] = FacDataPtr::FacData0;

                    let Some(ps) = p_aac_decoder_static_channel_info.as_deref_mut() else {
                        error = AAC_DEC_UNSUPPORTED_FORMAT;
                        break 'bail;
                    };
                    let Some(psc) = ps.get_mut(ch) else {
                        error = AAC_DEC_UNSUPPORTED_FORMAT;
                        break 'bail;
                    };
                    if f_fac_dat_present != 0 {
                        if el_flags & AC_EL_LFE != 0 {
                            error = AAC_DEC_PARSE_ERROR;
                        } else {
                            /* FAC data present, this frame is FD, so the last mode had to be
                             * ACELP. */
                            if psc.last_core_mode != LPD || psc.last_lpd_mode != 0 {
                                p[ch].data.usac.core_mode_last = LPD;
                                p[ch].data.usac.lpd_mode_last = 0;
                                /* We can't change the past! So look to the future and go ahead! */
                            }
                            let pc = &mut *p[ch];
                            let len = clpd_fac_get_length(
                                is_long_block(&pc.ics_info) as i32,
                                pc.granule_length,
                            );
                            clpd_fac_read(
                                h_bs,
                                &mut pc.data.usac.fac_data0,
                                &mut pc.data.usac.fac_data_e,
                                len,
                                1,
                                0,
                            );
                        }
                    } else {
                        if psc.last_core_mode == LPD && psc.last_lpd_mode == 0 {
                            /* ACELP to FD transitons without FAC are possible. That is why we
                            zero it out (i.e FAC will not be considered in the subsequent
                            calculations */
                            p[ch].data.usac.fac_data0 = [0; LFAC as usize];
                        }
                    }
                }
                RbdIdT::esc2_rvlc => {
                    if flags & AC_ER_RVLC != 0 {
                        match p_aac_decoder_static_channel_info.as_deref_mut() {
                            Some(ps) if ch < ps.len() => {
                                crvlc_decode(
                                    &mut *p[ch],
                                    &mut *ps[ch],
                                    h_bs,
                                    ch_com(com, &mut com_r, ch),
                                );
                            }
                            _ => {
                                error = AAC_DEC_UNSUPPORTED_FORMAT;
                            }
                        }
                    }
                }
                RbdIdT::esc1_hcr => {
                    if flags & AC_ER_HCR != 0 {
                        chcr_read(
                            h_bs,
                            &mut *p[ch],
                            if number_of_channels == 2 { ID_CPE } else { ID_SCE },
                        );
                    }
                }
                RbdIdT::spectral_data => {
                    error = cblock_read_spectral_data_fast(
                        h_bs,
                        &mut *p[ch],
                        p_sampling_rate_info,
                        flags,
                        ch_spec(spec, ch),
                        ch_com(com, &mut com_r, ch),
                    );
                    if flags & AC_ELD != 0 {
                        p[ch].render_mode = AACDEC_RENDER_ELDFB;
                    } else {
                        if flags & AC_HDAAC != 0 {
                            p[ch].render_mode = AACDEC_RENDER_INTIMDCT;
                        } else {
                            p[ch].render_mode = AACDEC_RENDER_IMDCT;
                        }
                    }
                }
                RbdIdT::ac_spectral_data => {
                    match p_aac_decoder_static_channel_info.as_deref_mut() {
                        Some(ps) if ch < ps.len() => {
                            error = cblock_read_ac_spectral_data(
                                h_bs,
                                &mut *p[ch],
                                &mut *ps[ch],
                                p_sampling_rate_info,
                                frame_length,
                                flags,
                                ch_spec(spec, ch),
                            );
                        }
                        _ => {
                            error = AAC_DEC_UNSUPPORTED_FORMAT;
                        }
                    }
                    p[ch].render_mode = AACDEC_RENDER_IMDCT;
                }
                RbdIdT::coupled_elements => {
                    ind_sw_cce_flag = fdk_read_bit(h_bs) as i32;
                    let num_coupled_elements = fdk_read_bits(h_bs, 3) as i32;

                    for _c in 0..(num_coupled_elements + 1) {
                        num_gain_element_lists += 1;
                        let cc_target_is_cpe = fdk_read_bit(h_bs); /* cc_target_is_cpe[c] */
                        fdk_read_bits(h_bs, 4); /* cc_target_tag_select[c] */

                        if cc_target_is_cpe != 0 {
                            let cc_l = fdk_read_bit(h_bs); /* cc_l[c] */
                            let cc_r = fdk_read_bit(h_bs); /* cc_r[c] */

                            if cc_l != 0 && cc_r != 0 {
                                num_gain_element_lists += 1;
                            }
                        }
                    }
                    fdk_read_bit(h_bs); /* cc_domain */
                    fdk_read_bit(h_bs); /* gain_element_sign  */
                    fdk_read_bits(h_bs, 2); /* gain_element_scale */
                }
                RbdIdT::gain_element_lists => {
                    let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[BOOKSCL as usize];
                    let pc = &*p[ch];

                    for _c in 1..num_gain_element_lists {
                        let cge = if ind_sw_cce_flag != 0 {
                            1
                        } else {
                            fdk_read_bits(h_bs, 1) /* common_gain_element_present[c] */
                        };
                        if cge != 0 {
                            /* Huffman */
                            cblock_decode_huffman_word(h_bs, hcb); /* hcod_sf[common_gain_element[c]] 1..19 */
                        } else {
                            for _g in 0..get_window_groups(&pc.ics_info) {
                                for sfb in 0..get_scale_factor_bands_transmitted(&pc.ics_info) {
                                    // pCodeBook[sfb]: the codebooks of the first group only, for
                                    // every group (as in C).
                                    if pc
                                        .p_dyn_data
                                        .a_code_book
                                        .get(sfb as usize)
                                        .copied()
                                        .unwrap_or(0)
                                        != ZERO_HCB
                                    {
                                        /* Huffman */
                                        cblock_decode_huffman_word(h_bs, hcb); /* hcod_sf[dpcm_gain_element[c][g][sfb]] 1..19 */
                                    }
                                }
                            }
                        }
                    }
                }

                /* CRC handling */
                RbdIdT::adtscrc_start_reg1 => {
                    if let Some(tp) = p_tp_dec.as_deref_mut() {
                        crc_reg1 = crc_start_reg(tp, h_bs, 192);
                    }
                }
                RbdIdT::adtscrc_start_reg2 => {
                    if let Some(tp) = p_tp_dec.as_deref_mut() {
                        crc_reg2 = crc_start_reg(tp, h_bs, 128);
                    }
                }
                RbdIdT::adtscrc_end_reg1 | RbdIdT::drmcrc_end_reg => {
                    if let Some(tp) = p_tp_dec.as_deref_mut() {
                        crc_end_reg(tp, h_bs, crc_reg1);
                        crc_reg1 = -1;
                    }
                }
                RbdIdT::adtscrc_end_reg2 => {
                    if crc_reg1 != -1 {
                        error = AAC_DEC_DECODE_FRAME_ERROR;
                    } else if let Some(tp) = p_tp_dec.as_deref_mut() {
                        crc_end_reg(tp, h_bs, crc_reg2);
                        crc_reg2 = -1;
                    }
                }
                RbdIdT::drmcrc_start_reg => {
                    if let Some(tp) = p_tp_dec.as_deref_mut() {
                        crc_reg1 = crc_start_reg(tp, h_bs, 0);
                    }
                }

                /* Non data cases */
                RbdIdT::next_channel => {
                    ch = (ch + 1) % number_of_channels.max(1) as usize;
                    if ch >= nch {
                        // More channels than the caller passed.
                        error = AAC_DEC_UNSUPPORTED_FORMAT;
                    }
                }
                RbdIdT::link_sequence => match list.next[(decision_bit & 1) as usize] {
                    Some(next) => {
                        list = next;
                        i = -1;
                    }
                    None => {
                        error = AAC_DEC_UNSUPPORTED_FORMAT;
                    }
                },

                _ => {
                    error = AAC_DEC_UNSUPPORTED_FORMAT;
                }
            }

            if error != AAC_DEC_OK {
                break 'bail;
            }

            i += 1;

            if list.id.get(i as usize).copied().unwrap_or(RbdIdT::end_of_sequence)
                == RbdIdT::end_of_sequence
            {
                break;
            }
        }

        if !IQ
            && el_flags & AC_EL_USAC_NOISE == 0
            && !inverse_quantization_fails(&p[..nch], p_sampling_rate_info, spec)
        {
            // (stage B of the frame pipeline does the inverse quantization)
            *deferred = true;
        } else {
            for ch in 0..nch {
                if p[ch].render_mode == AACDEC_RENDER_IMDCT
                    || p[ch].render_mode == AACDEC_RENDER_ELDFB
                {
                    /* Shows which bands are empty. */
                    p[ch].p_dyn_data.band_is_noise = [1; 8 * 16];

                    error = cblock_inverse_quantize_spectral_data_fast(
                        &mut *p[ch],
                        p_sampling_rate_info,
                        1,
                        ch_spec(spec, ch),
                    );
                    if error != AAC_DEC_OK {
                        return error;
                    }

                    if el_flags & AC_EL_USAC_NOISE != 0 {
                        match p_aac_decoder_static_channel_info.as_deref_mut() {
                            Some(ps) if ch < ps.len() => {
                                cblock_apply_noise(
                                    &mut *p[ch],
                                    p_sampling_rate_info,
                                    &mut ps[ch].nf_random_seed,
                                    ch_spec(spec, ch),
                                );
                            }
                            _ => {}
                        }
                    } /* if (elFlags & AC_EL_USAC_NOISE) */
                }
            }
        }
    }

    /* bail: */
    if crc_reg1 != -1 || crc_reg2 != -1 {
        if error == AAC_DEC_OK {
            error = AAC_DEC_DECODE_FRAME_ERROR;
        }
        if let Some(tp) = p_tp_dec {
            if crc_reg1 != -1 {
                crc_end_reg(tp, h_bs, crc_reg1);
            }
            if crc_reg2 != -1 {
                crc_end_reg(tp, h_bs, crc_reg2);
            }
        }
    }
    error
}

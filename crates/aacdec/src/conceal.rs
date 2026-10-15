// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/conceal.h, libAACdec/src/conceal.cpp.

//! Port of `conceal.h`: independent channel concealment (muting, noise substitution, energy
//! interpolation, time domain fading).
//!
//! # Memory
//!
//! - `CConcealmentInfo::pConcealParams` is not stored ([`crate::conceal_types`]): the functions
//!   that use it take `&CConcealParams` (the decoder's `concealCommonData`).
//! - `CConcealment_Store()` and `CConcealment_Apply()` take `hConcealmentInfo` and the static
//!   channel info, and the library always passes the static channel info's own
//!   `concealmentInfo` (`&pAacDecoderStaticChannelInfo->concealmentInfo`); the port drops the
//!   parameter and uses that field.
//! - `spec` is the spectrum of the channel (`pSpectralCoefficient`, at least 1024 values).
//!
//! The functions with a `NULL` check in C take `Option`s.
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `conceal.cpp` apart from
//! the expansion of `FDK_ASSERT` (the `x86` build declares the variable that an assertion of
//! `CConcealment_ApplyInter()` uses) and the shared header primitives.

use fdk_aac_rs_fdk::common_fix::{
    cnt_leading_zeros, f_ld_pow_noexp, f_mult, f_pow2_div2, fixp_abs, fl2fxconst_dbl,
    fl2fxconst_sgl, fx_dbl2fx_sgl, fx_sgl2fx_dbl, scale_value, FixpDbl, FixpSgl, DFRACT_BITS,
    MAXVAL_DBL, MAXVAL_SGL, MINVAL_DBL,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::mdct::{BlockType, BLOCK_LONG, BLOCK_SHORT, BLOCK_START, BLOCK_STOP};

use crate::aac_rom::{
    fixp_dbl2pcm_dec, PcmDec, AAC_DEC_RANDOM_SIGN, AAC_NF_NO_RANDOM_VAL, MAXVAL_PCM_DEC,
    MINVAL_PCM_DEC,
};
use crate::channelinfo::*;
pub use crate::conceal_types::*;
use crate::usacdec_const::NB_DIV;
use crate::usacdec_lpc::clpc_conceal;

/// `AACDEC_CONCEAL_PARAM_NOT_SPECIFIED`
pub const AACDEC_CONCEAL_PARAM_NOT_SPECIFIED: i32 = 0xFFFE;

/// `CONCEAL_DFLT_COMF_NOISE_LEVEL`
const CONCEAL_DFLT_COMF_NOISE_LEVEL: FixpDbl = 0x100000;

/// `CONCEAL_NOT_DEFINED`: `((UCHAR)-1)`
const CONCEAL_NOT_DEFINED: u8 = 255;

/* default settings */
/// `CONCEAL_DFLT_FADEOUT_FRAMES`
const CONCEAL_DFLT_FADEOUT_FRAMES: i32 = 6;
/// `CONCEAL_DFLT_FADEIN_FRAMES`
const CONCEAL_DFLT_FADEIN_FRAMES: i32 = 5;
/// `CONCEAL_DFLT_MUTE_RELEASE_FRAMES`
const CONCEAL_DFLT_MUTE_RELEASE_FRAMES: i32 = 0;

/// `CONCEAL_DFLT_FADE_FACTOR` (a `float` literal: `0.707106781186548f`, 1/sqrt(2))
const CONCEAL_DFLT_FADE_FACTOR: f32 = 0.707106781186548f32;

/* For parameter conversion */
/// `CONCEAL_PARAMETER_BITS`
const CONCEAL_PARAMETER_BITS: i32 = 8;
/// `CONCEAL_MAX_QUANT_FACTOR`
const CONCEAL_MAX_QUANT_FACTOR: i32 = (1 << CONCEAL_PARAMETER_BITS) - 1;
/// `CONCEAL_MIN_ATTENUATION_FACTOR_025_LD` (-0.25 dB)
const CONCEAL_MIN_ATTENUATION_FACTOR_025_LD: FixpDbl =
    fl2fxconst_dbl(-0.041524101186092029596853445212299);

/// `CConcealmentExpandType`
type CConcealmentExpandType = i32;
/// `CConcealment_NoExpand`
const CConcealment_NoExpand: CConcealmentExpandType = 0;
/// `CConcealment_Expand`
const CConcealment_Expand: CConcealmentExpandType = 1;
/// `CConcealment_Compress`
#[allow(unused)]
const CConcealment_Compress: CConcealmentExpandType = 2;

/// `facMod4Table[4]` (static in C): `2^-(1-0.00)`, `2^-(1-0.25)`, `2^-(1-0.50)`, `2^-(1-0.75)`
///
/// `scripts/dump_table.py libAACdec/src/conceal.cpp facMod4Table SHORT --hex`
pub static FAC_MOD4_TABLE: [FixpSgl; 4] = [0x4000, 0x4C1C, 0x5A82, 0x6BA2];

/// `pConcealParams->fadeOutFactor[idx]` (`which == 0`) or `fadeInFactor[idx]` (`which == 1`)
/// with the memory layout of `CConcealParams`: an index past `fadeOutFactor` reaches the next
/// field, `fadeInFactor`; reproduced from the C layout. Indexes outside both arrays read 0.
fn fade_factor(params: &CConcealParams, which: usize, idx: i32) -> FixpSgl {
    let i = idx as isize + which as isize * CONCEAL_MAX_NUM_FADE_FACTORS as isize;
    if i < 0 {
        return 0;
    }
    let i = i as usize;
    let n = CONCEAL_MAX_NUM_FADE_FACTORS as usize;
    if i < n {
        params.fade_out_factor[i]
    } else if i < 2 * n {
        params.fade_in_factor[i - n]
    } else {
        0
    }
}

/// `CConcealment_GetWinSeq(prevWinSeq)` (static in C)
fn cconcealment_get_win_seq(prev_win_seq: i32) -> BlockType {
    let mut new_win_seq = BLOCK_LONG;

    /* Try to have only long blocks */
    if prev_win_seq == BLOCK_START || prev_win_seq == BLOCK_SHORT {
        new_win_seq = BLOCK_STOP;
    }

    new_win_seq
}

/// `CConcealment_InitCommonData(pConcealCommonData)`: init the common concealment information
/// (`None` for `NULL`: nothing).
pub fn cconcealment_init_common_data(p_conceal_common_data: Option<&mut CConcealParams>) {
    if let Some(p) = p_conceal_common_data {
        /* Set default error concealment technique */
        p.method = ConcealMethodInter;

        p.num_fade_out_frames = CONCEAL_DFLT_FADEOUT_FRAMES;
        p.num_fade_in_frames = CONCEAL_DFLT_FADEIN_FRAMES;
        p.num_mute_release_frames = CONCEAL_DFLT_MUTE_RELEASE_FRAMES;

        p.comfort_noise_level = CONCEAL_DFLT_COMF_NOISE_LEVEL;

        /* Init fade factors (symetric) */
        p.fade_out_factor[0] = fl2fxconst_sgl(CONCEAL_DFLT_FADE_FACTOR as f64);
        p.fade_in_factor[0] = p.fade_out_factor[0];

        for i in 1..CONCEAL_MAX_NUM_FADE_FACTORS as usize {
            p.fade_out_factor[i] = fx_dbl2fx_sgl(f_mult(
                p.fade_out_factor[i - 1],
                fl2fxconst_sgl(CONCEAL_DFLT_FADE_FACTOR as f64),
            ));
            p.fade_in_factor[i] = p.fade_out_factor[i];
        }
    }
}

/// `CConcealment_GetMethod(pConcealCommonData)`: the current concealment method
/// (`ConcealMethodNone` for `NULL`).
pub fn cconcealment_get_method(
    p_conceal_common_data: Option<&CConcealParams>,
) -> CConcealmentMethod {
    let mut method = ConcealMethodNone;

    if let Some(p) = p_conceal_common_data {
        method = p.method;
    }

    method
}

/// `CConcealment_InitChannelData(pConcealChannelInfo, pConcealCommonData, initRenderMode,
/// samplesPerFrame)`: init the concealment information of a channel. The pointer to the common
/// data is not stored (see the [module documentation](self)).
pub fn cconcealment_init_channel_data(
    p_conceal_channel_info: &mut CConcealmentInfo,
    init_render_mode: AacdecRenderMode,
    _samples_per_frame: i32,
) {
    let c = p_conceal_channel_info;
    c.td_noise_seed = 0;
    c.td_noise_states = [0; 3];
    c.td_noise_coef[0] = fl2fxconst_sgl(0.05f32 as f64);
    c.td_noise_coef[1] = fl2fxconst_sgl(0.5f32 as f64);
    c.td_noise_coef[2] = fl2fxconst_sgl(0.45f32 as f64);

    c.last_render_mode = init_render_mode as i8;

    c.window_shape = CONCEAL_NOT_DEFINED;
    c.window_sequence = BLOCK_LONG; /* default type */
    c.last_win_grp_len = 1;

    c.conceal_state = ConcealState_Ok;

    c.spectral_coefficient = [0; 1024];

    c.spec_scale = [0; 8];

    c.i_random_phase = 0;

    c.prev_frame_ok[0] = 1;
    c.prev_frame_ok[1] = 1;

    c.cnt_fade_frames = 0;
    c.cnt_valid_frames = 0;
    c.fade_old = MAXVAL_DBL;
    c.win_grp_offset[0] = 0;
    c.win_grp_offset[1] = 0;
    c.att_grp_offset[0] = 0;
    c.att_grp_offset[1] = 0;
}

/// `CConcealment_SetParams(concealParams, method, fadeOutSlope, fadeInSlope, muteRelease,
/// comfNoiseLevel)`: set the error concealment parameters (`None` for `NULL`).
pub fn cconcealment_set_params(
    mut conceal_params: Option<&mut CConcealParams>,
    method: i32,
    fade_out_slope: i32,
    fade_in_slope: i32,
    mute_release: i32,
    comf_noise_level: FixpDbl,
) -> AacDecoderError {
    /* set concealment technique */
    if method != AACDEC_CONCEAL_PARAM_NOT_SPECIFIED {
        match method {
            ConcealMethodMute | ConcealMethodNoise | ConcealMethodInter => {
                /* Be sure to enable delay adjustment of SBR decoder! */
                match conceal_params.as_deref_mut() {
                    None => return AAC_DEC_INVALID_HANDLE,
                    Some(p) => {
                        /* set param */
                        p.method = method;
                    }
                }
            }
            _ => return AAC_DEC_SET_PARAM_FAIL,
        }
    }

    /* set number of frames for fade-out slope */
    if fade_out_slope != AACDEC_CONCEAL_PARAM_NOT_SPECIFIED {
        if (fade_out_slope < CONCEAL_MAX_NUM_FADE_FACTORS) && (fade_out_slope >= 0) {
            match conceal_params.as_deref_mut() {
                None => return AAC_DEC_INVALID_HANDLE,
                Some(p) => {
                    /* set param */
                    p.num_fade_out_frames = fade_out_slope;
                }
            }
        } else {
            return AAC_DEC_SET_PARAM_FAIL;
        }
    }

    /* set number of frames for fade-in slope */
    if fade_in_slope != AACDEC_CONCEAL_PARAM_NOT_SPECIFIED {
        if (fade_in_slope < CONCEAL_MAX_NUM_FADE_FACTORS) && (fade_in_slope >= 0) {
            match conceal_params.as_deref_mut() {
                None => return AAC_DEC_INVALID_HANDLE,
                Some(p) => {
                    /* set param */
                    p.num_fade_in_frames = fade_in_slope;
                }
            }
        } else {
            return AAC_DEC_SET_PARAM_FAIL;
        }
    }

    /* set number of error-free frames after which the muting will be released */
    if mute_release != AACDEC_CONCEAL_PARAM_NOT_SPECIFIED {
        if (mute_release < (CONCEAL_MAX_NUM_FADE_FACTORS << 1)) && (mute_release >= 0) {
            match conceal_params.as_deref_mut() {
                None => return AAC_DEC_INVALID_HANDLE,
                Some(p) => {
                    /* set param */
                    p.num_mute_release_frames = mute_release;
                }
            }
        } else {
            return AAC_DEC_SET_PARAM_FAIL;
        }
    }

    /* set confort noise level which will be inserted while in state 'muting' */
    if comf_noise_level != AACDEC_CONCEAL_PARAM_NOT_SPECIFIED {
        if (comf_noise_level < 0) || (comf_noise_level > MAXVAL_DBL) {
            return AAC_DEC_SET_PARAM_FAIL;
        }
        match conceal_params {
            None => return AAC_DEC_INVALID_HANDLE,
            Some(p) => {
                p.comfort_noise_level = comf_noise_level;
            }
        }
    }

    AAC_DEC_OK
}

/// `CConcealment_SetAttenuation(concealParams, fadeOutAttenuationVector,
/// fadeInAttenuationVector)`: set the fade-out/in attenuation factor vectors (32 quantized
/// factors each; `None` for `NULL`). Returns `AAC_DEC_OK` on success.
///
/// A vector shorter than 32 values (no defined reference result) is read as if it were padded
/// with zeros.
pub fn cconcealment_set_attenuation(
    conceal_params: Option<&mut CConcealParams>,
    fade_out_attenuation_vector: Option<&[i16]>,
    fade_in_attenuation_vector: Option<&[i16]>,
) -> AacDecoderError {
    if fade_out_attenuation_vector.is_none() && fade_in_attenuation_vector.is_none() {
        return AAC_DEC_SET_PARAM_FAIL;
    }
    let n = CONCEAL_MAX_NUM_FADE_FACTORS as usize;
    let get = |v: &[i16], i: usize| -> i32 { v.get(i).copied().unwrap_or(0) as i32 };
    let mut conceal_params = conceal_params;

    /* Fade-out factors */
    if let Some(v) = fade_out_attenuation_vector {
        /* check quantized factors first */
        for i in 0..n {
            if (get(v, i) < 0) || (get(v, i) > CONCEAL_MAX_QUANT_FACTOR) {
                return AAC_DEC_SET_PARAM_FAIL;
            }
        }
        let Some(p) = conceal_params.as_deref_mut() else {
            return AAC_DEC_INVALID_HANDLE;
        };

        /* now dequantize factors */
        for i in 0..n {
            p.fade_out_factor[i] = fx_dbl2fx_sgl(f_ld_pow_noexp(
                CONCEAL_MIN_ATTENUATION_FACTOR_025_LD,
                0,
                (fl2fxconst_dbl(1.0 / 2.0) >> (CONCEAL_PARAMETER_BITS - 1)).wrapping_mul(get(v, i)),
                CONCEAL_PARAMETER_BITS,
            ));
        }
    }

    /* Fade-in factors */
    if let Some(v) = fade_in_attenuation_vector {
        /* check quantized factors first */
        for i in 0..n {
            if (get(v, i) < 0) || (get(v, i) > CONCEAL_MAX_QUANT_FACTOR) {
                return AAC_DEC_SET_PARAM_FAIL;
            }
        }
        let Some(p) = conceal_params else {
            return AAC_DEC_INVALID_HANDLE;
        };

        /* now dequantize factors */
        for i in 0..n {
            p.fade_in_factor[i] = fx_dbl2fx_sgl(f_ld_pow_noexp(
                CONCEAL_MIN_ATTENUATION_FACTOR_025_LD,
                0,
                (fl2fxconst_dbl(1.0) >> CONCEAL_PARAMETER_BITS).wrapping_mul(get(v, i)),
                CONCEAL_PARAMETER_BITS,
            ));
        }
    }

    AAC_DEC_OK
}

/// `CConcealment_GetState(pConcealChannelInfo)`: the concealment state (`ConcealState_Ok` for
/// `NULL`).
pub fn cconcealment_get_state(
    p_conceal_channel_info: Option<&CConcealmentInfo>,
) -> CConcealmentState {
    let mut state = ConcealState_Ok;

    if let Some(c) = p_conceal_channel_info {
        state = c.conceal_state;
    }

    state
}

/// `CConcealment_Store(hConcealmentInfo, pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo)`:
/// store data for the concealment of later frames. `hConcealmentInfo` is the static channel
/// info's `concealmentInfo` (see the [module documentation](self)).
///
/// `lastWinGrpLen` is `WindowGroupLength[WindowGroups - 1]`; index -1 is the byte in front of
/// `icsInfo`, the high byte of `specScale[7]`, reproduced from the C layout of
/// `CAacDecoderChannelInfo` (checked by a `cref` test).
pub fn cconcealment_store(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    params: &CConcealParams,
    spec: &mut [FixpDbl],
) {
    let p = p_aac_decoder_channel_info;
    let ps = p_aac_decoder_static_channel_info;
    let nb_div = NB_DIV as usize;

    if !(p.render_mode == AACDEC_RENDER_LPD && p.data.usac.r#mod[nb_div - 1] == 0) {
        let h = &mut ps.concealment_info;

        /* store old window infos for swapping */
        let t_window_sequence = h.window_sequence;
        let t_window_shape = h.window_shape;

        /* store old scale factors for swapping */
        let t_spec_scale = h.spec_scale;

        /* store new window infos */
        h.window_sequence = get_window_sequence(&p.ics_info);
        h.window_shape = get_window_shape(&p.ics_info);
        let groups = get_window_groups(&p.ics_info) as i32;
        h.last_win_grp_len = if groups >= 1 {
            p.ics_info.window_group_length.get(groups as usize - 1).copied().unwrap_or(0)
        } else {
            // *(WindowGroupLength - 1): the high byte of specScale[7] (little-endian).
            ((p.spec_scale[7] as u16) >> 8) as u8
        };

        /* store new scale factors */
        h.spec_scale = p.spec_scale;

        let n = spec.len().min(1024);
        if params.method < ConcealMethodInter {
            /* store new spectral bins */
            h.spectral_coefficient[..n].copy_from_slice(&spec[..n]);
        } else {
            /* swap spectral data */
            spec[..n].swap_with_slice(&mut h.spectral_coefficient[..n]);

            /* complete swapping of window infos */
            p.ics_info.window_sequence = t_window_sequence;
            p.ics_info.window_shape = t_window_shape;

            /* complete swapping of scale factors */
            p.spec_scale = t_spec_scale;
        }
    }

    if p.render_mode == AACDEC_RENDER_LPD {
        /* Store LSF4 */
        ps.concealment_info.lsf4 = ps.lpc4_lsf;
        /* Store TCX gain */
        ps.concealment_info.last_tcx_gain = ps.last_tcx_gain;
        ps.concealment_info.last_tcx_gain_e = ps.last_tcx_gain_e;
    }
}

/// `CConcealment_Apply(hConcealmentInfo, pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo,
/// pSamplingRateInfo, samplesPerFrame, lastLpdMode, frameOk, flags)`: apply the concealment.
/// Returns -1 during an active mute release, otherwise 1 if concealment processing was
/// applied, 0 if not.
///
/// `hConcealmentInfo` is the static channel info's `concealmentInfo`; `params` is
/// `pConcealParams`; `spec` the spectrum of the channel (see the
/// [module documentation](self)).
pub fn cconcealment_apply(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    samples_per_frame: i32,
    _last_lpd_mode: u8,
    frame_ok: i32,
    flags: u32,
    params: &CConcealParams,
    spec: &mut [FixpDbl],
) -> i32 {
    let p = p_aac_decoder_channel_info;
    let ps = p_aac_decoder_static_channel_info;
    let mut applied_processing = 0;
    let mute_release_active: i32 = (frame_ok != 0
        && (ps.concealment_info.conceal_state >= ConcealState_Mute)
        && (ps.concealment_info.cnt_valid_frames + 1 <= params.num_mute_release_frames))
        as i32;

    if ps.concealment_info.window_shape == CONCEAL_NOT_DEFINED {
        /* Initialize window_shape with same value as in the current (parsed) frame. Because
        section 4.6.11.3.2 (Windowing and block switching) of ISO/IEC 14496-3:2009 says: For the
        first raw_data_block() to be decoded the window_shape of the left and right half of the
        window are identical. */
        ps.concealment_info.window_shape = p.ics_info.window_shape;
    }

    if frame_ok != 0 && mute_release_active == 0 {
        /* Update render mode if frameOk except for ongoing mute release state. */
        ps.concealment_info.last_render_mode = p.render_mode as i8;

        /* Rescue current data for concealment in future frames */
        cconcealment_store(p, ps, params, spec);
        /* Reset index to random sign vector to make sign calculation frame agnostic (only
        depends on number of subsequently concealed spectral blocks) */
        ps.concealment_info.i_random_phase = 0;
    } else {
        if ps.concealment_info.last_render_mode as i32 == AACDEC_RENDER_INVALID {
            ps.concealment_info.last_render_mode = AACDEC_RENDER_IMDCT as i8;
        }
        p.render_mode = ps.concealment_info.last_render_mode as AacdecRenderMode;
    }

    /* hand current frame status to the state machine */
    cconcealment_update_state(ps, frame_ok, samples_per_frame, p, params);

    {
        if frame_ok == 0 && p.render_mode == AACDEC_RENDER_IMDCT {
            /* LPC extrapolation */
            let first = (ps.concealment_info.last_render_mode as i32 == AACDEC_RENDER_IMDCT) as i32;
            clpc_conceal(
                &mut p.data.usac.lsp_coeff,
                &mut ps.lpc4_lsf,
                &ps.lsf_adaptive_mean,
                first,
            );
            ps.concealment_info.lsf4 = ps.lpc4_lsf;
        }

        /* Create data for signal rendering according to the selected concealment method and
         * decoder operating mode. */

        if (frame_ok == 0 || mute_release_active != 0) && (p.render_mode == AACDEC_RENDER_LPD) {
            /* Restore old LSF4 */
            ps.lpc4_lsf = ps.concealment_info.lsf4;
            /* Restore old TCX gain */
            ps.last_tcx_gain = ps.concealment_info.last_tcx_gain;
            ps.last_tcx_gain_e = ps.concealment_info.last_tcx_gain_e;
        }

        if !(p.render_mode == AACDEC_RENDER_LPD && ps.last_lpd_mode == 0) {
            match params.method {
                ConcealMethodNoise => {
                    /* Noise substitution error concealment technique */
                    applied_processing = cconcealment_apply_noise(
                        ps,
                        p,
                        p_sampling_rate_info,
                        samples_per_frame,
                        flags,
                        params,
                        spec,
                    );
                }
                ConcealMethodInter => {
                    /* Energy interpolation concealment based on 3GPP */
                    applied_processing = cconcealment_apply_inter(
                        &mut ps.concealment_info,
                        p,
                        p_sampling_rate_info,
                        samples_per_frame,
                        0, /* don't use tonal improvement */
                        frame_ok,
                        mute_release_active,
                        spec,
                    );
                }
                _ => {
                    /* default: case ConcealMethodMute: */
                    if frame_ok == 0 {
                        /* Mute spectral data in case of errors */
                        let n = (samples_per_frame.max(0) as usize).min(spec.len());
                        spec[..n].fill(0);
                        /* Set last window shape */
                        p.ics_info.window_shape = ps.concealment_info.window_shape;
                        applied_processing = 1;
                    }
                }
            }
        } else if frame_ok == 0 || mute_release_active != 0 {
            /* simply restore the buffer */
            let h = &ps.concealment_info;

            /* restore window infos (gri) do we need that? */
            p.ics_info.window_sequence = h.window_sequence;
            p.ics_info.window_shape = h.window_shape;

            let n = spec.len().min(1024);
            if h.conceal_state != ConcealState_Mute {
                /* restore scale factors */
                p.spec_scale = h.spec_scale;

                /* restore spectral bins */
                spec[..n].copy_from_slice(&h.spectral_coefficient[..n]);
            } else {
                /* clear scale factors */
                p.spec_scale = [0; 8];

                /* clear buffer */
                spec[..n].fill(0);
            }
        }
    }
    /* update history */
    ps.concealment_info.prev_frame_ok[0] = ps.concealment_info.prev_frame_ok[1];
    ps.concealment_info.prev_frame_ok[1] = frame_ok;

    if mute_release_active != 0 {
        -1
    } else {
        applied_processing
    }
}

/// `CConcealment_ApplyNoise(pConcealmentInfo, pAacDecoderChannelInfo,
/// pAacDecoderStaticChannelInfo, pSamplingRateInfo, samplesPerFrame, flags)` (static in C):
/// noise substitution. `pConcealmentInfo` is the static channel info's `concealmentInfo`.
fn cconcealment_apply_noise(
    ps: &mut CAacDecoderStaticChannelInfo,
    p: &mut CAacDecoderChannelInfo,
    _p_sampling_rate_info: &SamplingRateInfo,
    samples_per_frame: i32,
    _flags: u32,
    params: &CConcealParams,
    spec: &mut [FixpDbl],
) -> i32 {
    let mut applied_processing = 0;

    fdk_assert!((samples_per_frame >= 120) && (samples_per_frame <= 1024));

    match ps.concealment_info.conceal_state {
        ConcealState_Ok => { /* Nothing to do here! */ }
        ConcealState_Single | ConcealState_FadeOut => {
            applied_processing =
                cconcealment_apply_fade_out(1, ps, samples_per_frame, p, params, spec);
        }
        ConcealState_Mute => {
            let c = &mut ps.concealment_info;
            /* set dummy window parameters */
            p.ics_info.valid = 0; /* Trigger the generation of a consitent IcsInfo */
            p.ics_info.window_shape = c.window_shape; /* Prevent an invalid WindowShape (required for F/T transform) */
            p.ics_info.window_sequence = cconcealment_get_win_seq(c.window_sequence);
            c.window_sequence = p.ics_info.window_sequence; /* Store for next frame (spectrum in concealment buffer can't be used at all) */

            /* mute spectral data */
            let n = (samples_per_frame.max(0) as usize).min(spec.len()).min(1024);
            spec[..n].fill(0);
            c.spectral_coefficient[..n].fill(0);

            applied_processing = 1;
        }
        ConcealState_FadeIn => {
            /* TimeDomainFading:                                        */
            /* Attenuation of signal is done in CConcealment_TDFading() */

            applied_processing = 1;
        }
        _ => {
            /* we shouldn't come here anyway */
            fdk_assert!(false);
        }
    }

    applied_processing
}

/// `CConcealment_ApplyInter(pConcealmentInfo, pAacDecoderChannelInfo, pSamplingRateInfo,
/// samplesPerFrame, improveTonal, frameOk, mute_release_active)` (static in C): swap the data of
/// the current and the previous frame; interpolate a missing frame; fade in and out.
fn cconcealment_apply_inter(
    c: &mut CConcealmentInfo,
    p: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    samples_per_frame: i32,
    _improve_tonal: i32,
    frame_ok: i32,
    mute_release_active: i32,
    spec: &mut [FixpDbl],
) -> i32 {
    let mut sfb_energy_prev = [0i32; 64];
    let mut sfb_energy_act = [0i32; 64];

    let mut applied_processing = 0;
    let spf = (samples_per_frame.max(0) as usize).min(1024).min(spec.len());

    if frame_ok == 0 || mute_release_active != 0 {
        /* Restore last frame from concealment buffer */
        p.ics_info.window_shape = c.window_shape;
        p.ics_info.window_sequence = c.window_sequence;

        /* Restore spectral data */
        spec[..spf].copy_from_slice(&c.spectral_coefficient[..spf]);

        /* Restore scale factors */
        p.spec_scale = c.spec_scale;
    }

    /* if previous frame was not ok */
    if c.prev_frame_ok[1] == 0 || mute_release_active != 0 {
        /* if current frame (f_n) is ok and the last but one frame (f_(n-2)) was ok, too, then
        interpolate both frames in order to generate the current output frame (f_(n-1)).
        Otherwise, use the last stored frame (f_(n-2) or f_(n-3) or ...). */
        if frame_ok != 0 && c.prev_frame_ok[0] != 0 && mute_release_active == 0 {
            applied_processing = 1;

            /* Interpolate both frames in order to generate the current output frame (f_(n-1)). */
            if p.ics_info.window_sequence == BLOCK_SHORT {
                /* f_(n-2) == BLOCK_SHORT */
                /* short--??????--short, short--??????--long interpolation */
                /* short--short---short, short---long---long interpolation */

                if c.window_sequence == BLOCK_SHORT {
                    /* f_n == BLOCK_SHORT */
                    /* short--short---short interpolation */

                    let scale_factor_bands_total =
                        p_sampling_rate_info.number_of_scale_factor_bands_short as i32;
                    let p_sfb_offset = p_sampling_rate_info.scale_factor_bands_short;
                    p.ics_info.window_shape = if samples_per_frame <= 512 { 2 } else { 1 };
                    p.ics_info.window_sequence = BLOCK_SHORT;

                    let wl = (samples_per_frame / 8).max(0) as usize;
                    for wnd in 0..8usize {
                        let o = (wnd * wl).min(spec.len());
                        cconcealment_calc_band_energy(
                            &spec[o..], /* spec_(n-2) */
                            p_sampling_rate_info,
                            BLOCK_SHORT,
                            CConcealment_NoExpand,
                            &mut sfb_energy_prev,
                        );

                        let oc = (wnd * wl).min(1024);
                        cconcealment_calc_band_energy(
                            &c.spectral_coefficient[oc..], /* spec_n */
                            p_sampling_rate_info,
                            BLOCK_SHORT,
                            CConcealment_NoExpand,
                            &mut sfb_energy_act,
                        );

                        p.spec_scale[wnd] = cconcealment_interpolate_buffer(
                            &mut spec[o..], /* spec_(n-1) */
                            p.spec_scale[wnd],
                            c.spec_scale[wnd],
                            &sfb_energy_prev,
                            &sfb_energy_act,
                            scale_factor_bands_total,
                            p_sfb_offset,
                        );
                    }
                } else {
                    /* f_n != BLOCK_SHORT */
                    /* short---long---long interpolation */

                    let scale_factor_bands_total =
                        p_sampling_rate_info.number_of_scale_factor_bands_long as i32;
                    let p_sfb_offset = p_sampling_rate_info.scale_factor_bands_long;

                    let o = ((samples_per_frame - (samples_per_frame / 8)).max(0) as usize)
                        .min(spec.len());
                    cconcealment_calc_band_energy(
                        &spec[o..], /* [wnd] spec_(n-2) */
                        p_sampling_rate_info,
                        BLOCK_SHORT,
                        CConcealment_Expand,
                        &mut sfb_energy_act,
                    );

                    cconcealment_calc_band_energy(
                        &c.spectral_coefficient[..], /* spec_n */
                        p_sampling_rate_info,
                        BLOCK_LONG,
                        CConcealment_NoExpand,
                        &mut sfb_energy_prev,
                    );

                    p.ics_info.window_shape = 0;
                    p.ics_info.window_sequence = BLOCK_STOP;

                    spec[..spf].copy_from_slice(&c.spectral_coefficient[..spf]); /* spec_n */

                    for i in 0..8 {
                        /* search for max(specScale) */
                        if p.spec_scale[i] > p.spec_scale[0] {
                            p.spec_scale[0] = p.spec_scale[i];
                        }
                    }

                    let spec_scale_out = cconcealment_interpolate_buffer(
                        spec, /* spec_(n-1) */
                        c.spec_scale[0],
                        p.spec_scale[0],
                        &sfb_energy_prev,
                        &sfb_energy_act,
                        scale_factor_bands_total,
                        p_sfb_offset,
                    );

                    p.spec_scale[0] = spec_scale_out;
                }
            } else {
                /* long--??????--short, long--??????--long interpolation */
                /* long---long---short, long---long---long interpolation */

                let scale_factor_bands_total =
                    p_sampling_rate_info.number_of_scale_factor_bands_long as i32;
                let p_sfb_offset = p_sampling_rate_info.scale_factor_bands_long;
                let mut spec_scale_act: i16 = c.spec_scale[0];

                cconcealment_calc_band_energy(
                    spec, /* spec_(n-2) */
                    p_sampling_rate_info,
                    BLOCK_LONG,
                    CConcealment_NoExpand,
                    &mut sfb_energy_prev,
                );

                if c.window_sequence == BLOCK_SHORT {
                    /* f_n == BLOCK_SHORT */
                    /* long---long---short interpolation */

                    p.ics_info.window_shape = if samples_per_frame <= 512 { 2 } else { 1 };
                    p.ics_info.window_sequence = BLOCK_START;

                    for i in 1..8 {
                        /* search for max(specScale) */
                        if c.spec_scale[i] > spec_scale_act {
                            spec_scale_act = c.spec_scale[i];
                        }
                    }

                    /* Expand first short spectrum */
                    cconcealment_calc_band_energy(
                        &c.spectral_coefficient[..], /* spec_n */
                        p_sampling_rate_info,
                        BLOCK_SHORT,
                        CConcealment_Expand, /* !!! */
                        &mut sfb_energy_act,
                    );
                } else {
                    /* long---long---long interpolation */

                    p.ics_info.window_shape = 0;
                    p.ics_info.window_sequence = BLOCK_LONG;

                    cconcealment_calc_band_energy(
                        &c.spectral_coefficient[..], /* spec_n */
                        p_sampling_rate_info,
                        BLOCK_LONG,
                        CConcealment_NoExpand,
                        &mut sfb_energy_act,
                    );
                }

                p.spec_scale[0] = cconcealment_interpolate_buffer(
                    spec, /* spec_(n-1) */
                    p.spec_scale[0],
                    spec_scale_act,
                    &sfb_energy_prev,
                    &sfb_energy_act,
                    scale_factor_bands_total,
                    p_sfb_offset,
                );
            }
        }

        /* Noise substitution of sign of the output spectral coefficients */
        cconcealment_apply_random_sign(c.i_random_phase, &mut spec[..spf], samples_per_frame);
        /* Increment random phase index to avoid repetition artifacts. */
        c.i_random_phase = (c.i_random_phase + 1) & (AAC_NF_NO_RANDOM_VAL - 1);
    }

    /* scale spectrum according to concealment state */
    match c.conceal_state {
        ConcealState_Single => {
            applied_processing = 1;
        }
        ConcealState_FadeOut => {
            fdk_assert!(c.cnt_fade_frames >= 0);
            fdk_assert!(c.cnt_fade_frames < CONCEAL_MAX_NUM_FADE_FACTORS);

            /* TimeDomainFading:                                        */
            /* Attenuation of signal is done in CConcealment_TDFading() */

            applied_processing = 1;
        }
        ConcealState_FadeIn => {
            fdk_assert!(c.cnt_fade_frames >= 0);
            fdk_assert!(c.cnt_fade_frames < CONCEAL_MAX_NUM_FADE_FACTORS);

            /* TimeDomainFading:                                        */
            /* Attenuation of signal is done in CConcealment_TDFading() */

            applied_processing = 1;
        }
        ConcealState_Mute => {
            /* set dummy window parameters */
            p.ics_info.valid = 0; /* Trigger the generation of a consitent IcsInfo */
            p.ics_info.window_shape = c.window_shape; /* Prevent an invalid WindowShape (required for F/T transform) */
            p.ics_info.window_sequence = cconcealment_get_win_seq(c.window_sequence);
            c.window_sequence = p.ics_info.window_sequence; /* Store for next frame (spectrum in concealment buffer can't be used at all) */

            /* mute spectral data */
            spec[..spf].fill(0);

            applied_processing = 1;
        }
        _ => { /* nothing to do here */ }
    }

    applied_processing
}

/// `CConcealment_CalcBandEnergy(spectrum, pSamplingRateInfo, blockType, expandType, sfbEnergy)`
/// (static in C): the band-wise spectral energy used for the frame interpolation. Lines outside
/// `spectrum` read 0 (no defined reference result).
fn cconcealment_calc_band_energy(
    spectrum: &[FixpDbl],
    p_sampling_rate_info: &SamplingRateInfo,
    block_type: i32,
    expand_type: CConcealmentExpandType,
    sfb_energy: &mut [i32; 64],
) {
    /* In the following calculations, enAccu is initialized with LSB-value in order to avoid
     * zero energy-level */
    let sp = |i: i32| -> FixpDbl { spectrum.get(i as usize).copied().unwrap_or(0) };
    let off = |t: &[i16], i: i32| -> i32 { t.get(i as usize).copied().unwrap_or(0) as i32 };
    let sfb_scale_of = |t: &[i16], sfb: i32| -> i32 {
        (core::mem::size_of::<i32>() as i32 * 8)
            - cnt_leading_zeros(off(t, sfb + 1) - off(t, sfb))
            - 1
    };
    // An sfb index of 64 or more (impossible with the tables of the library) is not written.
    let mut store = |sfb: i32, v: i32| {
        if let Some(e) = sfb_energy.get_mut(sfb as usize) {
            *e = v;
        }
    };

    let mut line: i32 = 0;

    match block_type {
        BLOCK_LONG | BLOCK_START | BLOCK_STOP => {
            if expand_type == CConcealment_NoExpand {
                /* standard long calculation */
                let total = p_sampling_rate_info.number_of_scale_factor_bands_long as i32;
                let t = p_sampling_rate_info.scale_factor_bands_long;

                for sfb in 0..total {
                    let mut en_accu: FixpDbl = 1;
                    let sfb_scale = sfb_scale_of(t, sfb);
                    /* scaling depends on sfb width. */
                    while line < off(t, sfb + 1) {
                        en_accu = en_accu
                            .wrapping_add(f_pow2_div2(sp(line)).wrapping_shr(sfb_scale as u32));
                        line += 1;
                    }
                    store(sfb, cnt_leading_zeros(en_accu) - 1);
                }
            } else {
                /* compress long to short */
                let total = p_sampling_rate_info.number_of_scale_factor_bands_short as i32;
                let t = p_sampling_rate_info.scale_factor_bands_short;

                for sfb in 0..total {
                    let mut en_accu: FixpDbl = 1;
                    let sfb_scale = sfb_scale_of(t, sfb);
                    /* scaling depends on sfb width. */
                    while line < off(t, sfb + 1) << 3 {
                        en_accu = en_accu.wrapping_add(
                            en_accu
                                .wrapping_add(f_pow2_div2(sp(line)).wrapping_shr(sfb_scale as u32))
                                >> 3,
                        );
                        line += 1;
                    }
                    store(sfb, cnt_leading_zeros(en_accu) - 1);
                }
            }
        }
        BLOCK_SHORT => {
            if expand_type == CConcealment_NoExpand {
                /*   standard short calculation */
                let total = p_sampling_rate_info.number_of_scale_factor_bands_short as i32;
                let t = p_sampling_rate_info.scale_factor_bands_short;

                for sfb in 0..total {
                    let mut en_accu: FixpDbl = 1;
                    let sfb_scale = sfb_scale_of(t, sfb);
                    /* scaling depends on sfb width. */
                    while line < off(t, sfb + 1) {
                        en_accu = en_accu
                            .wrapping_add(f_pow2_div2(sp(line)).wrapping_shr(sfb_scale as u32));
                        line += 1;
                    }
                    store(sfb, cnt_leading_zeros(en_accu) - 1);
                }
            } else {
                /*  expand short to long spectrum */
                let total = p_sampling_rate_info.number_of_scale_factor_bands_long as i32;
                let t = p_sampling_rate_info.scale_factor_bands_long;

                for sfb in 0..total {
                    let mut en_accu: FixpDbl = 1;
                    let sfb_scale = sfb_scale_of(t, sfb);
                    /* scaling depends on sfb width. */
                    while line < off(t, sfb + 1) {
                        en_accu = en_accu.wrapping_add(
                            f_pow2_div2(sp(line >> 3)).wrapping_shr(sfb_scale as u32),
                        );
                        line += 1;
                    }
                    store(sfb, cnt_leading_zeros(en_accu) - 1);
                }
            }
        }
        _ => {}
    }
}

/// `CConcealment_InterpolateBuffer(spectrum, pSpecScalePrv, pSpecScaleAct, pSpecScaleOut, enPrv,
/// enAct, sfbCnt, pSfbOffset)` (static in C): create the interpolated spectral data according to
/// the energy of the last good frame and the current (good) frame. Returns `*pSpecScaleOut`
/// (written after the loop in C, so its aliasing with `pSpecScalePrv` does not matter).
fn cconcealment_interpolate_buffer(
    spectrum: &mut [FixpDbl],
    p_spec_scale_prv: i16,
    p_spec_scale_act: i16,
    en_prv: &[i32; 64],
    en_act: &[i32; 64],
    sfb_cnt: i32,
    p_sfb_offset: &[i16],
) -> i16 {
    let mut line: i32 = 0;
    let prv = p_spec_scale_prv as i32;
    let act = p_spec_scale_act as i32;

    for sfb in 0..sfb_cnt.clamp(0, 64) {
        let s = sfb as usize;
        let mut fac_shift = en_prv[s] - en_act[s] + ((act - prv) << 1);
        let fac_mod = (fac_shift & 3) as usize;
        fac_shift = (fac_shift >> 2) + 1;
        fac_shift += prv - prv.max(act);
        fac_shift = fac_shift.min(DFRACT_BITS - 1).max(-(DFRACT_BITS - 1));

        let end = p_sfb_offset.get(s + 1).copied().unwrap_or(0) as i32;
        while line < end {
            if let Some(v) = spectrum.get_mut(line as usize) {
                let accu = f_mult(*v, FAC_MOD4_TABLE[fac_mod]);
                *v = scale_value(accu, fac_shift);
            }
            line += 1;
        }
    }
    prv.max(act) as i16
}

/// `findEquiFadeFrame(pConcealCommonData, actFadeIndex, direction)` (static in C): find the next
/// fading frame in case of a change of the fading direction (`direction` 0: from fade-out to
/// fade-in, 1: from fade-in to fade-out).
fn find_equi_fade_frame(params: &CConcealParams, act_fade_index: i32, direction: i32) -> i32 {
    let reference_val: FixpSgl;
    let mut min_diff: FixpSgl = MAXVAL_SGL as FixpSgl;

    let mut next_fade_index: i32 = 0;

    /* init depending on direction */
    let (ref_which, factor_which) = if direction == 0 { (0usize, 1usize) } else { (1, 0) };
    if act_fade_index < 0 {
        reference_val = MAXVAL_SGL as FixpSgl;
    } else {
        reference_val = fade_factor(params, ref_which, act_fade_index) >> 1;
    }
    let p_factor = |i: i32| fade_factor(params, factor_which, i);

    /* search for minimum difference */
    for i in 0..CONCEAL_MAX_NUM_FADE_FACTORS {
        let diff: FixpSgl = fixp_abs((p_factor(i) >> 1) as i32 - reference_val as i32) as FixpSgl;
        if diff < min_diff {
            min_diff = diff;
            next_fade_index = i;
        }
    }

    /* check and adjust depending on direction */
    if direction == 0 {
        /* FADE-OUT => FADE-IN */
        if next_fade_index > params.num_fade_in_frames {
            next_fade_index = (params.num_fade_in_frames - 1).max(0);
        }
        if ((p_factor(next_fade_index) >> 1) <= reference_val) && (next_fade_index > 0) {
            next_fade_index -= 1;
        }
    } else {
        /* FADE-IN => FADE-OUT */
        if ((p_factor(next_fade_index) >> 1) >= reference_val)
            && (next_fade_index < CONCEAL_MAX_NUM_FADE_FACTORS - 1)
        {
            next_fade_index += 1;
        }
    }

    next_fade_index
}

/// `CConcealment_UpdateState(pConcealmentInfo, frameOk, pAacDecoderStaticChannelInfo,
/// samplesPerFrame, pAacDecoderChannelInfo)` (static in C): the concealment state machine
/// (states mute, fade-in, fade-out, interpolate and frame-ok). `pConcealmentInfo` is the static
/// channel info's `concealmentInfo`.
fn cconcealment_update_state(
    ps: &mut CAacDecoderStaticChannelInfo,
    frame_ok: i32,
    samples_per_frame: i32,
    p: &mut CAacDecoderChannelInfo,
    params: &CConcealParams,
) {
    match params.method {
        ConcealMethodNoise => {
            if ps.concealment_info.conceal_state != ConcealState_Ok {
                /* count the valid frames during concealment process */
                if frame_ok != 0 {
                    ps.concealment_info.cnt_valid_frames += 1;
                } else {
                    ps.concealment_info.cnt_valid_frames = 0;
                }
            }

            /* -- STATE MACHINE for Noise Substitution -- */
            match ps.concealment_info.conceal_state {
                ConcealState_Ok => {
                    if frame_ok == 0 {
                        ps.concealment_info.cnt_fade_frames = 0;
                        ps.concealment_info.cnt_valid_frames = 0;
                        ps.concealment_info.att_grp_offset[0] = 0;
                        ps.concealment_info.att_grp_offset[1] = 0;
                        ps.concealment_info.win_grp_offset[0] = 0;
                        ps.concealment_info.win_grp_offset[1] = 0;
                        if params.num_fade_out_frames > 0 {
                            /* change to state SINGLE-FRAME-LOSS */
                            ps.concealment_info.conceal_state = ConcealState_Single;
                            /* mode 0 just updates the Fading counter */
                            cconcealment_apply_fade_out(
                                0,
                                ps,
                                samples_per_frame,
                                p,
                                params,
                                &mut [],
                            );
                        } else {
                            /* change to state MUTE */
                            ps.concealment_info.conceal_state = ConcealState_Mute;
                        }
                    }
                }
                ConcealState_Single => {
                    /* Just a pre-stage before fade-out begins. Stay here only one frame! */
                    if frame_ok != 0 {
                        /* change to state OK */
                        ps.concealment_info.conceal_state = ConcealState_Ok;
                    } else {
                        if ps.concealment_info.cnt_fade_frames >= params.num_fade_out_frames {
                            /* change to state MUTE */
                            ps.concealment_info.conceal_state = ConcealState_Mute;
                        } else {
                            /* change to state FADE-OUT */
                            ps.concealment_info.conceal_state = ConcealState_FadeOut;
                            /* mode 0 just updates the Fading counter */
                            cconcealment_apply_fade_out(
                                0,
                                ps,
                                samples_per_frame,
                                p,
                                params,
                                &mut [],
                            );
                        }
                    }
                }
                ConcealState_FadeOut => {
                    if ps.concealment_info.cnt_valid_frames > params.num_mute_release_frames {
                        if params.num_fade_in_frames > 0 {
                            /* change to state FADE-IN */
                            ps.concealment_info.conceal_state = ConcealState_FadeIn;
                            ps.concealment_info.cnt_fade_frames = find_equi_fade_frame(
                                params,
                                ps.concealment_info.cnt_fade_frames,
                                0, /* FadeOut -> FadeIn */
                            );
                        } else {
                            /* change to state OK */
                            ps.concealment_info.conceal_state = ConcealState_Ok;
                        }
                    } else {
                        if frame_ok != 0 {
                            /* we have good frame information but stay fully in concealment -
                             * reset winGrpOffset/attGrpOffset */
                            ps.concealment_info.win_grp_offset[0] = 0;
                            ps.concealment_info.win_grp_offset[1] = 0;
                            ps.concealment_info.att_grp_offset[0] = 0;
                            ps.concealment_info.att_grp_offset[1] = 0;
                        }
                        if ps.concealment_info.cnt_fade_frames >= params.num_fade_out_frames {
                            /* change to state MUTE */
                            ps.concealment_info.conceal_state = ConcealState_Mute;
                        } else {
                            /* Stay in FADE-OUT */
                            /* mode 0 just updates the Fading counter */
                            cconcealment_apply_fade_out(
                                0,
                                ps,
                                samples_per_frame,
                                p,
                                params,
                                &mut [],
                            );
                        }
                    }
                }
                ConcealState_Mute => {
                    if ps.concealment_info.cnt_valid_frames > params.num_mute_release_frames {
                        if params.num_fade_in_frames > 0 {
                            /* change to state FADE-IN */
                            ps.concealment_info.conceal_state = ConcealState_FadeIn;
                            ps.concealment_info.cnt_fade_frames = params.num_fade_in_frames - 1;
                        } else {
                            /* change to state OK */
                            ps.concealment_info.conceal_state = ConcealState_Ok;
                        }
                    } else {
                        if frame_ok != 0 {
                            /* we have good frame information but stay fully in concealment -
                             * reset winGrpOffset/attGrpOffset */
                            ps.concealment_info.win_grp_offset[0] = 0;
                            ps.concealment_info.win_grp_offset[1] = 0;
                            ps.concealment_info.att_grp_offset[0] = 0;
                            ps.concealment_info.att_grp_offset[1] = 0;
                        }
                    }
                }
                ConcealState_FadeIn => {
                    ps.concealment_info.cnt_fade_frames -= 1;
                    if frame_ok != 0 {
                        if ps.concealment_info.cnt_fade_frames < 0 {
                            /* change to state OK */
                            ps.concealment_info.conceal_state = ConcealState_Ok;
                        }
                    } else {
                        if params.num_fade_out_frames > 0 {
                            /* change to state FADE-OUT */
                            ps.concealment_info.conceal_state = ConcealState_FadeOut;
                            ps.concealment_info.cnt_fade_frames = find_equi_fade_frame(
                                params,
                                ps.concealment_info.cnt_fade_frames + 1,
                                1, /* FadeIn -> FadeOut */
                            );
                            ps.concealment_info.win_grp_offset[0] = 0;
                            ps.concealment_info.win_grp_offset[1] = 0;
                            ps.concealment_info.att_grp_offset[0] = 0;
                            ps.concealment_info.att_grp_offset[1] = 0;

                            ps.concealment_info.cnt_fade_frames -= 1; /* decrease because CConcealment_ApplyFadeOut() will increase, accordingly */
                            /* mode 0 just updates the Fading counter */
                            cconcealment_apply_fade_out(
                                0,
                                ps,
                                samples_per_frame,
                                p,
                                params,
                                &mut [],
                            );
                        } else {
                            /* change to state MUTE */
                            ps.concealment_info.conceal_state = ConcealState_Mute;
                        }
                    }
                }
                _ => {
                    fdk_assert!(false);
                }
            }
        }

        ConcealMethodInter | ConcealMethodTonal => {
            let frame_ok_like = ps.concealment_info.prev_frame_ok[1] != 0
                || (ps.concealment_info.prev_frame_ok[0] != 0
                    && ps.concealment_info.prev_frame_ok[1] == 0
                    && frame_ok != 0);
            if ps.concealment_info.conceal_state != ConcealState_Ok {
                /* count the valid frames during concealment process */
                if frame_ok_like {
                    /* The frame is OK even if it can be estimated by the energy interpolation
                     * algorithm */
                    ps.concealment_info.cnt_valid_frames += 1;
                } else {
                    ps.concealment_info.cnt_valid_frames = 0;
                }
            }

            /* -- STATE MACHINE for energy interpolation -- */
            match ps.concealment_info.conceal_state {
                ConcealState_Ok => {
                    if !frame_ok_like {
                        if params.num_fade_out_frames > 0 {
                            /* Fade out only if the energy interpolation algorithm can not be
                             * applied! */
                            ps.concealment_info.conceal_state = ConcealState_FadeOut;
                        } else {
                            /* change to state MUTE */
                            ps.concealment_info.conceal_state = ConcealState_Mute;
                        }
                        ps.concealment_info.cnt_fade_frames = 0;
                        ps.concealment_info.cnt_valid_frames = 0;
                    }
                }
                ConcealState_Single => {
                    ps.concealment_info.conceal_state = ConcealState_Ok;
                }
                ConcealState_FadeOut => {
                    ps.concealment_info.cnt_fade_frames += 1;

                    if ps.concealment_info.cnt_valid_frames > params.num_mute_release_frames {
                        if params.num_fade_in_frames > 0 {
                            /* change to state FADE-IN */
                            ps.concealment_info.conceal_state = ConcealState_FadeIn;
                            ps.concealment_info.cnt_fade_frames = find_equi_fade_frame(
                                params,
                                ps.concealment_info.cnt_fade_frames - 1,
                                0, /* FadeOut -> FadeIn */
                            );
                        } else {
                            /* change to state OK */
                            ps.concealment_info.conceal_state = ConcealState_Ok;
                        }
                    } else {
                        if ps.concealment_info.cnt_fade_frames >= params.num_fade_out_frames {
                            /* change to state MUTE */
                            ps.concealment_info.conceal_state = ConcealState_Mute;
                        }
                    }
                }
                ConcealState_Mute => {
                    if ps.concealment_info.cnt_valid_frames > params.num_mute_release_frames {
                        if params.num_fade_in_frames > 0 {
                            /* change to state FADE-IN */
                            ps.concealment_info.conceal_state = ConcealState_FadeIn;
                            ps.concealment_info.cnt_fade_frames = params.num_fade_in_frames - 1;
                        } else {
                            /* change to state OK */
                            ps.concealment_info.conceal_state = ConcealState_Ok;
                        }
                    }
                }
                ConcealState_FadeIn => {
                    ps.concealment_info.cnt_fade_frames -= 1; /* used to address the fade-in factors */

                    if frame_ok != 0 || ps.concealment_info.prev_frame_ok[1] != 0 {
                        if ps.concealment_info.cnt_fade_frames < 0 {
                            /* change to state OK */
                            ps.concealment_info.conceal_state = ConcealState_Ok;
                        }
                    } else {
                        if params.num_fade_out_frames > 0 {
                            /* change to state FADE-OUT */
                            ps.concealment_info.conceal_state = ConcealState_FadeOut;
                            ps.concealment_info.cnt_fade_frames = find_equi_fade_frame(
                                params,
                                ps.concealment_info.cnt_fade_frames + 1,
                                1, /* FadeIn -> FadeOut */
                            );
                        } else {
                            /* change to state MUTE */
                            ps.concealment_info.conceal_state = ConcealState_Mute;
                        }
                    }
                }
                _ => {}
            } /* End switch(pConcealmentInfo->concealState) */
        }

        _ => { /* Don't need a state machine for other concealment methods. */ }
    }
}

/// `CConcealment_ApplyRandomSign(iRandomPhase, spec, samplesPerFrame)` (static in C): toggle the
/// sign of the spectral data randomly.
fn cconcealment_apply_random_sign(random_phase: i32, spec: &mut [FixpDbl], samples_per_frame: i32) {
    let mut random_phase = random_phase;
    let sign =
        |rp: i32| -> u16 { AAC_DEC_RANDOM_SIGN.get((rp >> 4) as usize).copied().unwrap_or(0) };

    /* random table 512x16bit has been reduced to 512 packed sign bits = 32x16 bit */

    /* read current packed sign word */
    let mut packed_sign: u16 = sign(random_phase);
    packed_sign >>= random_phase & 0xf;

    for i in 0..samples_per_frame.max(0) as usize {
        if (random_phase & 0xf) == 0 {
            packed_sign = sign(random_phase);
        }

        if packed_sign & 0x1 != 0 {
            if let Some(s) = spec.get_mut(i) {
                *s = -(*s).max(MINVAL_DBL + 1);
            }
        }
        packed_sign >>= 1;

        random_phase = (random_phase + 1) & (AAC_NF_NO_RANDOM_VAL - 1);
    }
}

/// `CConcealment_GetLastFrameOk(hConcealmentInfo, fBeforeApply)`: the frame OK flag of the
/// previous frame (1 for `NULL`). Called before `CConcealment_Apply()`, `f_before_apply` must
/// be set to get the correct value.
pub fn cconcealment_get_last_frame_ok(
    h_concealment_info: Option<&CConcealmentInfo>,
    f_before_apply: i32,
) -> i32 {
    let mut prev_frame_ok = 1;

    if let Some(c) = h_concealment_info {
        prev_frame_ok = c.prev_frame_ok[(f_before_apply & 0x1) as usize];
    }

    prev_frame_ok
}

/// `CConcealment_GetDelay(pConcealCommonData)`: the number of delay frames introduced by the
/// concealment technique (0 for `NULL`).
pub fn cconcealment_get_delay(p_conceal_common_data: Option<&CConcealParams>) -> u32 {
    let mut frame_delay: u32 = 0;

    if let Some(p) = p_conceal_common_data {
        match p.method {
            ConcealMethodTonal | ConcealMethodInter => {
                frame_delay = 1;
            }
            _ => {}
        }
    }

    frame_delay
}

/// `CConcealment_ApplyFadeOut(mode, pConcealmentInfo, pAacDecoderStaticChannelInfo,
/// samplesPerFrame, pAacDecoderChannelInfo)` (static in C): mode 1 restores the spectrum from the
/// concealment buffer with a random sign (and mutes it if necessary), mode 0 only updates the
/// fading counter. `pConcealmentInfo` is the static channel info's `concealmentInfo`; `spec` is
/// only used in mode 1.
///
/// Window and group offsets that leave the concealment buffer or the spectrum (impossible in
/// the library: the offsets are reset whenever the window sequence can change) read 0 and are
/// not written.
fn cconcealment_apply_fade_out(
    mode: i32,
    ps: &mut CAacDecoderStaticChannelInfo,
    samples_per_frame: i32,
    p: &mut CAacDecoderChannelInfo,
    params: &CConcealParams,
    spec: &mut [FixpDbl],
) -> i32 {
    /* mode 1 = apply RandomSign and mute spectral coefficients if necessary, *
     * mode 0 = Update cntFadeFrames                                          */

    /* restore frequency coefficients from buffer with a specific muting */
    let mut num_windows: i32 = 1;
    let mut window_len: i32 = samples_per_frame;
    let mut src_grp_start: i32 = 0;
    let mut win_idx_stride: i32 = 1;
    let m = (mode & 1) as usize;

    let c = &mut ps.concealment_info;

    /* set old window parameters */
    if c.last_render_mode as i32 == AACDEC_RENDER_LPD {
        match ps.last_lpd_mode {
            1 => {
                num_windows = 4;
                src_grp_start = 3;
                window_len = samples_per_frame >> 2;
            }
            2 => {
                num_windows = 2;
                src_grp_start = 1;
                window_len = samples_per_frame >> 1;
                win_idx_stride = 2;
            }
            3 => {
                num_windows = 1;
                src_grp_start = 0;
                window_len = samples_per_frame;
                win_idx_stride = 4;
            }
            _ => {}
        }
        c.last_win_grp_len = 1;
    } else {
        p.ics_info.window_shape = c.window_shape;
        p.ics_info.window_sequence = c.window_sequence;

        if c.window_sequence == BLOCK_SHORT {
            /* short block handling */
            num_windows = 8;
            window_len = samples_per_frame >> 3;
            src_grp_start = num_windows - c.last_win_grp_len as i32;
        }
    }

    let att_idx_stride: i32 = 1.max(num_windows / (c.last_win_grp_len as i32 + 1));

    /* load last state */
    let mut att_idx = c.cnt_fade_frames;
    let mut num_win_grp_per_fac = c.att_grp_offset[m];
    let mut src_win = src_grp_start + c.win_grp_offset[m];

    fdk_assert!((src_grp_start * window_len + window_len) <= samples_per_frame);
    fdk_assert!((src_win * window_len + window_len) <= 1024);

    let wl = window_len.max(0) as usize;
    for dst_win in 0..num_windows {
        if mode == 1 {
            let src = src_win as isize * wl as isize;
            let dst = dst_win as usize * wl;

            /* mute if attIdx gets large enaugh */
            if att_idx > params.num_fade_out_frames {
                for i in 0..wl {
                    if let Some(v) = usize::try_from(src + i as isize)
                        .ok()
                        .and_then(|k| c.spectral_coefficient.get_mut(k))
                    {
                        *v = 0;
                    }
                }
            }

            /* restore frequency coefficients from buffer - attenuation is done later */
            for i in 0..wl {
                let v = usize::try_from(src + i as isize)
                    .ok()
                    .and_then(|k| c.spectral_coefficient.get(k).copied())
                    .unwrap_or(0);
                if let Some(o) = spec.get_mut(dst + i) {
                    *o = v;
                }
            }

            /* apply random change of sign for spectral coefficients */
            let o = dst.min(spec.len());
            let e = (dst + wl).min(spec.len());
            cconcealment_apply_random_sign(c.i_random_phase, &mut spec[o..e], window_len);

            /* Increment random phase index to avoid repetition artifacts. */
            c.i_random_phase = (c.i_random_phase + 1) & (AAC_NF_NO_RANDOM_VAL - 1);

            /* set old scale factors */
            let di = (dst_win * win_idx_stride) as usize;
            let si = (src_win * win_idx_stride) as usize;
            if di < 8 && si < 8 {
                p.spec_scale[di] = c.spec_scale[si];
            }
        }

        src_win += 1;

        if src_win >= num_windows {
            /* end of sequence -> rewind to first window of group */
            src_win = src_grp_start;
            num_win_grp_per_fac += 1;
            if num_win_grp_per_fac >= att_idx_stride {
                num_win_grp_per_fac = 0;
                att_idx += 1;
            }
        }
    }

    /* store current state */

    c.win_grp_offset[m] = src_win - src_grp_start;
    fdk_assert!((c.win_grp_offset[m] >= 0) && (c.win_grp_offset[m] < 8));
    c.att_grp_offset[m] = num_win_grp_per_fac;
    fdk_assert!((c.att_grp_offset[m] >= 0) && (c.att_grp_offset[m] < att_idx_stride));

    if mode == 0 {
        c.cnt_fade_frames = att_idx;
    }

    1
}

/// `CConcealment_TDFading(len, ppAacDecoderStaticChannelInfo, aacOutDataHeadroom, pcmdata,
/// pcmdata_1)`: time domain fading of the rendered time signal (`len` samples of `pcmdata`)
/// according to the concealment state. `p_aac_decoder_static_channel_info` is
/// `*ppAacDecoderStaticChannelInfo`; `pcmdata_1` is unused in C. Returns 1.
pub fn cconcealment_td_fading(
    len: i32,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    params: &CConcealParams,
    aac_out_data_headroom: i32,
    pcmdata: &mut [PcmDec],
) -> i32 {
    /* Do the fading in Time domain based on concealment states and core mode */
    let c = &mut p_aac_decoder_static_channel_info.concealment_info;
    let att_mute: FixpDbl = 0;
    let mut idx: i32 = 0;
    let conceal_state = c.conceal_state;
    let fading_type: TDfadingType;
    let mut fading_stations: [FixpDbl; 9] = [0; 9];
    let mut fading_steps: [i32; 8] = [0; 8];
    let fade_start: FixpDbl = c.fade_old; /* start fading at last end-of-frame attenuation */
    let mut fade_factor_which: usize = 0; /* fadeOutFactor */
    let cnt_fade_frames = c.cnt_fade_frames;
    let td_fade_out_stop_before_mute = 1;
    let td_fade_in_stop_before_full_level = 1;

    /* determine Fading behaviour (end-of-frame attenuation and fading type) (1.) */

    match conceal_state {
        ConcealState_Single | ConcealState_Mute | ConcealState_FadeOut => {
            idx = if params.method == ConcealMethodNoise {
                cnt_fade_frames - 1
            } else {
                cnt_fade_frames
            };
            if conceal_state == ConcealState_Mute
                || (cnt_fade_frames + td_fade_out_stop_before_mute) > params.num_fade_out_frames
            {
                fading_type = FADE_TIMEDOMAIN_TOSPECTRALMUTE;
            } else {
                fading_type = FADE_TIMEDOMAIN;
            }
        }
        ConcealState_FadeIn | ConcealState_Ok => {
            if conceal_state == ConcealState_FadeIn {
                idx = cnt_fade_frames;
                idx -= td_fade_in_stop_before_full_level;
            }
            fade_factor_which = 1; /* fadeInFactor */
            idx = if conceal_state == ConcealState_Ok { -1 } else { idx };
            fading_type = if c.conceal_state_old == ConcealState_Mute {
                FADE_TIMEDOMAIN_FROMSPECTRALMUTE
            } else {
                FADE_TIMEDOMAIN
            };
        }
        _ => {
            fdk_assert!(false);
            fading_type = FADE_TIMEDOMAIN_TOSPECTRALMUTE;
        }
    }

    /* determine Target end-of-frame fading level and fading slope */
    let fade_stop: FixpDbl = match fading_type {
        FADE_TIMEDOMAIN_FROMSPECTRALMUTE => {
            let fs = if idx < 0 {
                MAXVAL_DBL
            } else {
                fx_sgl2fx_dbl(fade_factor(params, fade_factor_which, idx))
            };
            if params.num_fade_in_frames == 0 {
                /* do step as fast as possible */
                fading_steps[0] = 1;
            } else {
                cconcealment_td_fading_do_linear_fading_steps(&mut fading_steps);
            }
            fs
        }
        FADE_TIMEDOMAIN => {
            let fs = if idx < 0 {
                MAXVAL_DBL
            } else {
                fx_sgl2fx_dbl(fade_factor(params, fade_factor_which, idx))
            };
            cconcealment_td_fading_do_linear_fading_steps(&mut fading_steps);
            fs
        }
        _ => {
            /* FADE_TIMEDOMAIN_TOSPECTRALMUTE */
            if params.num_fade_out_frames == 0 {
                /* do step as fast as possible */
                fading_steps[0] = 1;
            } else {
                cconcealment_td_fading_do_linear_fading_steps(&mut fading_steps);
            }
            att_mute
        }
    };

    /* Render fading levels within current frame and do the final fading (2.) */

    let len = len >> 3;
    cconcealment_td_fade_fill_fading_stations(
        &mut fading_stations,
        &fading_steps,
        fade_stop,
        fade_start,
        fading_type,
    );

    if fading_stations.iter().any(|&s| s != MAXVAL_DBL) {
        /* if there's something to fade */
        let mut start = 0;
        for ii in 0..8 {
            cconcealment_td_fade_pcm_att(
                start,
                len,
                fading_stations[ii],
                fading_stations[ii + 1],
                pcmdata,
            );
            start += len;
        }
    }
    cconcealment_td_noise_apply(c, params, len, aac_out_data_headroom, pcmdata);

    /* Save end-of-frame attenuation and fading type */
    c.last_fading_type = fading_type;
    c.fade_old = fade_stop;
    c.conceal_state_old = conceal_state;

    1
}

/// `CConcealment_TDFadePcmAtt(start, len, fadeStart, fadeStop, pcmdata)` (static in C):
/// attenuate the samples `start..start + len` in the time domain fading.
///
/// x86 division by zero: the arm64 result (`sdiv` gives 0) on every variant.
fn cconcealment_td_fade_pcm_att(
    start: i32,
    len: i32,
    fade_start: FixpDbl,
    fade_stop: FixpDbl,
    pcmdata: &mut [PcmDec],
) {
    /* set start energy */
    let mut d_gain = fade_start;
    /* determine energy steps from sample to sample */
    let num = (fade_start >> 1).wrapping_sub(fade_stop >> 1);
    let d_step: FixpDbl = if len == 0 { 0 } else { num.wrapping_div(len) }.wrapping_shl(1);

    for i in start..(start + len) {
        d_gain = d_gain.wrapping_sub(d_step);
        /* prevent gain from getting negative due to possible fixpoint inaccuracies */
        let d_gain_apply = d_gain.max(0);
        /* finally, attenuate samples */
        if let Some(s) = pcmdata.get_mut(i as usize) {
            *s = fixp_dbl2pcm_dec(f_mult(*s, d_gain_apply));
        }
    }
}

/// `CConcealment_TDFadeFillFadingStations(fadingStations, fadingSteps, fadeStop, fadeStart,
/// fadingType)` (static in C): the attenuation factors of the 8 parts of a frame.
fn cconcealment_td_fade_fill_fading_stations(
    fading_stations: &mut [FixpDbl; 9],
    fading_steps: &[i32; 8],
    fade_stop: FixpDbl,
    fade_start: FixpDbl,
    _fading_type: TDfadingType,
) {
    let fading_steps_sum: i32 = fading_steps.iter().sum();
    let fade_diff: i32 = fade_stop.wrapping_sub(fade_start) / fading_steps_sum.max(1);
    fading_stations[0] = fade_start;
    for i in 1..8 {
        fading_stations[i] =
            fading_stations[i - 1].wrapping_add(fade_diff.wrapping_mul(fading_steps[i - 1]));
    }
    fading_stations[8] = fade_stop;
}

/// `CConcealment_TDFading_doLinearFadingSteps(fadingSteps)` (static in C)
fn cconcealment_td_fading_do_linear_fading_steps(fading_steps: &mut [i32; 8]) {
    *fading_steps = [1; 8];
}

/// `CConcealment_TDNoise_Random(seed)` (static in C): derived from `int UsacRandomSign()`.
fn cconcealment_td_noise_random(seed: &mut u32) -> i32 {
    *seed = seed.wrapping_mul(69069).wrapping_add(5);
    *seed as i32
}

/// `CConcealment_TDNoise_Apply(pConcealmentInfo, len, aacOutDataHeadroom, pcmdata)` (static in
/// C): add comfort noise while concealing.
fn cconcealment_td_noise_apply(
    c: &mut CConcealmentInfo,
    params: &CConcealParams,
    len: i32,
    aac_out_data_headroom: i32,
    pcmdata: &mut [PcmDec],
) {
    let mut seed: u32 = {
        let r = cconcealment_td_noise_random(&mut c.td_noise_seed) as u32;
        c.td_noise_seed = r.wrapping_add(1);
        c.td_noise_seed
    };

    let td_noise_att: FixpDbl = params.comfort_noise_level;

    if (c.conceal_state != ConcealState_Ok || c.conceal_state_old != ConcealState_Ok)
        && td_noise_att != 0
    {
        let states = &mut c.td_noise_states;
        let coef = &c.td_noise_coef;
        for ii in 0..(len.max(0) << 3) as usize {
            /* create filtered noise */
            states[2] = states[1];
            states[1] = states[0];
            states[0] = fixp_dbl2pcm_dec(cconcealment_td_noise_random(&mut seed));
            let noise_val_long: FixpDbl = f_mult(states[0], coef[0])
                .wrapping_add(f_mult(states[1], coef[1]))
                .wrapping_add(f_mult(states[2], coef[2]));
            // >> aacOutDataHeadroom: the headroom is a small configuration value (asr with a
            // register amount).
            let mut noise_val: PcmDec = fixp_dbl2pcm_dec(
                f_mult(noise_val_long, td_noise_att).wrapping_shr(aac_out_data_headroom as u32),
            );

            let Some(s) = pcmdata.get_mut(ii) else { break };
            /* add filtered noise - check for clipping, before */
            if noise_val > 0 && *s > MAXVAL_PCM_DEC.wrapping_sub(noise_val) {
                noise_val = noise_val.wrapping_mul(-1);
            } else if noise_val < 0 && *s < MINVAL_PCM_DEC.wrapping_sub(noise_val) {
                noise_val = noise_val.wrapping_mul(-1);
            }

            *s = s.wrapping_add(noise_val);
        }
    }
}

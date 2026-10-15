// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_dec_conceal.h, libSACdec/src/sac_dec_conceal.cpp.

//! Port of `sac_dec_conceal.h` and `sac_dec_conceal.cpp`: the error concealment of the spatial
//! parameters (keep, fade to the default parameters, fade back).

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

use crate::sac_dec_interface::*;

/// `SAC_DEC_CONCEAL_PARAM`: the dynamic parameters of the module.
pub type SacDecConcealParam = i32;
pub const SAC_DEC_CONCEAL_METHOD: SacDecConcealParam = 0;
pub const SAC_DEC_CONCEAL_NUM_KEEP_FRAMES: SacDecConcealParam = 1;
pub const SAC_DEC_CONCEAL_FADE_OUT_SLOPE_LENGTH: SacDecConcealParam = 2;
pub const SAC_DEC_CONCEAL_FADE_IN_SLOPE_LENGTH: SacDecConcealParam = 3;
pub const SAC_DEC_CONCEAL_NUM_RELEASE_FRAMES: SacDecConcealParam = 4;

/// `SpatialDecConcealmentMethod`
pub type SpatialDecConcealmentMethod = i32;
pub const SAC_DEC_CONCEAL_WITH_ZERO_VALUED_OUTPUT: SpatialDecConcealmentMethod = 0;
pub const SAC_DEC_CONCEAL_BY_FADING_PARAMETERS: SpatialDecConcealmentMethod = 1;

/// `MPEGS_CONCEAL_DEFAULT_METHOD`
pub const MPEGS_CONCEAL_DEFAULT_METHOD: SpatialDecConcealmentMethod =
    SAC_DEC_CONCEAL_BY_FADING_PARAMETERS;
/// `MPEGS_CONCEAL_DEFAULT_NUM_KEEP_FRAMES`
pub const MPEGS_CONCEAL_DEFAULT_NUM_KEEP_FRAMES: u32 = 10;
/// `MPEGS_CONCEAL_DEFAULT_FADE_OUT_SLOPE_LENGTH`
pub const MPEGS_CONCEAL_DEFAULT_FADE_OUT_SLOPE_LENGTH: u32 = 5;
/// `MPEGS_CONCEAL_DEFAULT_FADE_IN_SLOPE_LENGTH`
pub const MPEGS_CONCEAL_DEFAULT_FADE_IN_SLOPE_LENGTH: u32 = 5;
/// `MPEGS_CONCEAL_DEFAULT_NUM_RELEASE_FRAMES`
pub const MPEGS_CONCEAL_DEFAULT_NUM_RELEASE_FRAMES: u32 = 3;

/// `SpatialDecConcealmentState`
pub type SpatialDecConcealmentState = i32;
pub const SpatialDecConcealState_Init: SpatialDecConcealmentState = 0;
pub const SpatialDecConcealState_Ok: SpatialDecConcealmentState = 1;
pub const SpatialDecConcealState_Keep: SpatialDecConcealmentState = 2;
pub const SpatialDecConcealState_FadeToDefault: SpatialDecConcealmentState = 3;
pub const SpatialDecConcealState_Default: SpatialDecConcealmentState = 4;
pub const SpatialDecConcealState_FadeFromDefault: SpatialDecConcealmentState = 5;

/// `SpatialDecConcealmentParams`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialDecConcealmentParams {
    /// `method`
    pub method: SpatialDecConcealmentMethod,
    /// `numKeepFrames`
    pub num_keep_frames: u32,
    /// `numFadeOutFrames`
    pub num_fade_out_frames: u32,
    /// `numFadeInFrames`
    pub num_fade_in_frames: u32,
    /// `numReleaseFrames`
    pub num_release_frames: u32,
}

/// `SpatialDecConcealmentInfo`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialDecConcealmentInfo {
    /// `concealParams`: user set params
    pub conceal_params: SpatialDecConcealmentParams,
    /// `concealState`: state of the internal state machine (fade-in/out etc)
    pub conceal_state: SpatialDecConcealmentState,
    /// `cntStateFrames`: counter for fade-in/out handling
    pub cnt_state_frames: u32,
    /// `cntValidFrames`: counter for the number of consecutive good frames
    pub cnt_valid_frames: u32,
}

/// `MPEGS_CONCEAL_RESET_STATE`
pub const MPEGS_CONCEAL_RESET_STATE: u32 = 0x01;
/// `MPEGS_CONCEAL_RESET_PARAMETER`
pub const MPEGS_CONCEAL_RESET_PARAMETER: u32 = 0x02;
/// `MPEGS_CONCEAL_RESET_ALL`
pub const MPEGS_CONCEAL_RESET_ALL: u32 = 0xFF;

/// `SpatialDecConcealment_Init(info, resetFlags)`
pub fn spatial_dec_concealment_init(info: &mut SpatialDecConcealmentInfo, reset_flags: u32) {
    if reset_flags & MPEGS_CONCEAL_RESET_STATE != 0 {
        info.conceal_state = SpatialDecConcealState_Init;
        // Frame counters will be initialized implicitly in function
        // SpatialDecConcealment_UpdateState().
    }

    if reset_flags & MPEGS_CONCEAL_RESET_PARAMETER != 0 {
        // Set default params
        info.conceal_params.method = MPEGS_CONCEAL_DEFAULT_METHOD;
        info.conceal_params.num_keep_frames = MPEGS_CONCEAL_DEFAULT_NUM_KEEP_FRAMES;
        info.conceal_params.num_fade_out_frames = MPEGS_CONCEAL_DEFAULT_FADE_OUT_SLOPE_LENGTH;
        info.conceal_params.num_fade_in_frames = MPEGS_CONCEAL_DEFAULT_FADE_IN_SLOPE_LENGTH;
        info.conceal_params.num_release_frames = MPEGS_CONCEAL_DEFAULT_NUM_RELEASE_FRAMES;
    }
}

/// `fDivNorm(UINT a + 1, UINT b + 1)` of the fades: the `UINT` sums are converted to
/// `FIXP_DBL` (two's complement wrap).
fn fade_factor(a: u32, b: u32) -> FixpDbl {
    f_div_norm_noexp(a.wrapping_add(1) as i32, b.wrapping_add(1) as i32)
}

/// `SpatialDecConcealment_Apply(info, cmpIdxData, diffIdxData, idxPrev, bsXXXDataMode,
/// startBand, stopBand, defaultValue, paramType, numParamSets)`: applies the concealment to the
/// index data of one parameter (`diff_idx_data` is `None` for `NULL`). Returns whether
/// processing was applied.
///
/// Deviation: `cmpIdxData[numParamSets - 1]` outside of the array (the library never calls the
/// function with `numParamSets` outside of `1..=9`) reads 0.
pub fn spatial_dec_concealment_apply(
    info: &SpatialDecConcealmentInfo,
    cmp_idx_data: &[[i8; MAX_PARAMETER_BANDS]],
    mut diff_idx_data: Option<&mut [[i8; MAX_PARAMETER_BANDS]]>,
    idx_prev: &mut [i8; MAX_PARAMETER_BANDS],
    bs_xxx_data_mode: &mut [i8],
    start_band: i32,
    stop_band: i32,
    default_value: i8,
    _param_type: i32,
    num_param_sets: i32,
) -> i32 {
    let mut applied_processing = 0;
    let mut data_mode: i32 = -1;
    let bands = start_band.max(0) as usize..stop_band.clamp(0, MAX_PARAMETER_BANDS as i32) as usize;

    // Processing depends only on the internal state
    match info.conceal_state {
        SpatialDecConcealState_Init => {
            data_mode = 0; // default
        }
        SpatialDecConcealState_Ok => {
            // Nothing to do
        }
        SpatialDecConcealState_Keep => {
            data_mode = 1; // keep
        }
        SpatialDecConcealState_FadeToDefault => {
            // Start simple fade out
            let fac = fade_factor(info.cnt_state_frames, info.conceal_params.num_fade_out_frames);

            for band in bands.clone() {
                // idxPrev = fac * defaultValue + (1-fac) * idxPrev;
                let p = idx_prev[band] as i32;
                idx_prev[band] =
                    f_mult_i(fac, (default_value as i32).wrapping_sub(p)).wrapping_add(p) as i8;
            }
            data_mode = 1; // keep
            applied_processing = 1;
        }
        SpatialDecConcealState_Default => {
            for band in bands.clone() {
                idx_prev[band] = default_value;
            }
            data_mode = 1; // keep
            applied_processing = 1;
        }
        SpatialDecConcealState_FadeFromDefault => {
            let fac = fade_factor(info.cnt_valid_frames, info.conceal_params.num_fade_in_frames);
            let row = cmp_idx_data.get((num_param_sets - 1) as usize);

            for band in bands.clone() {
                // idxPrev = fac * cmpIdxData + (1-fac) * defaultValue;
                let c = row.map_or(0, |r| r[band] as i32);
                idx_prev[band] = f_mult_i(fac, c.wrapping_sub(default_value as i32))
                    .wrapping_add(default_value as i32) as i8;
            }
            data_mode = 1; // keep
            applied_processing = 1;
        }
        _ => {
            fdk_assert!(false); // All valid states shall be handled above.
        }
    }

    if data_mode >= 0 {
        for i in 0..num_param_sets.max(0) as usize {
            if let Some(m) = bs_xxx_data_mode.get_mut(i) {
                *m = data_mode as i8;
            }
            if let Some(diff) = diff_idx_data.as_deref_mut() {
                if let Some(row) = diff.get_mut(i) {
                    for band in bands.clone() {
                        row[band] = 0;
                    }
                }
            }
        }
    }

    applied_processing
}

/// `SpatialDecConcealment_UpdateState(info, frameOk)`
pub fn spatial_dec_concealment_update_state(info: &mut SpatialDecConcealmentInfo, frame_ok: i32) {
    if frame_ok != 0 {
        info.cnt_valid_frames = info.cnt_valid_frames.wrapping_add(1);
    } else {
        info.cnt_valid_frames = 0;
    }

    match info.conceal_state {
        SpatialDecConcealState_Init => {
            if frame_ok != 0 {
                // NEXT STATE: Ok
                info.conceal_state = SpatialDecConcealState_Ok;
                info.cnt_state_frames = 0;
            }
        }
        SpatialDecConcealState_Ok => {
            if frame_ok == 0 {
                // NEXT STATE: Keep
                info.conceal_state = SpatialDecConcealState_Keep;
                info.cnt_state_frames = 0;
            }
        }
        SpatialDecConcealState_Keep => {
            info.cnt_state_frames = info.cnt_state_frames.wrapping_add(1);
            if frame_ok != 0 {
                // NEXT STATE: Ok
                info.conceal_state = SpatialDecConcealState_Ok;
            } else if info.cnt_state_frames >= info.conceal_params.num_keep_frames {
                if info.conceal_params.num_fade_out_frames == 0 {
                    // NEXT STATE: Default
                    info.conceal_state = SpatialDecConcealState_Default;
                } else {
                    // NEXT STATE: Fade to default
                    info.conceal_state = SpatialDecConcealState_FadeToDefault;
                    info.cnt_state_frames = 0;
                }
            }
        }
        SpatialDecConcealState_FadeToDefault => {
            info.cnt_state_frames = info.cnt_state_frames.wrapping_add(1);
            if info.cnt_valid_frames > 0 {
                // NEXT STATE: Fade in from default
                info.conceal_state = SpatialDecConcealState_FadeFromDefault;
                info.cnt_state_frames = 0;
            } else if info.cnt_state_frames >= info.conceal_params.num_fade_out_frames {
                // NEXT STATE: Default
                info.conceal_state = SpatialDecConcealState_Default;
            }
        }
        SpatialDecConcealState_Default => {
            if info.cnt_valid_frames > 0 {
                if info.conceal_params.num_fade_in_frames == 0 {
                    // NEXT STATE: Ok
                    info.conceal_state = SpatialDecConcealState_Ok;
                } else {
                    // NEXT STATE: Fade in from default
                    info.conceal_state = SpatialDecConcealState_FadeFromDefault;
                    info.cnt_valid_frames = 0;
                }
            }
        }
        SpatialDecConcealState_FadeFromDefault => {
            info.cnt_valid_frames = info.cnt_valid_frames.wrapping_add(1);
            if frame_ok != 0 {
                if info.cnt_valid_frames >= info.conceal_params.num_fade_in_frames {
                    // NEXT STATE: Ok
                    info.conceal_state = SpatialDecConcealState_Ok;
                }
            } else {
                // NEXT STATE: Fade to default
                info.conceal_state = SpatialDecConcealState_FadeToDefault;
                info.cnt_state_frames = 0;
            }
        }
        _ => {
            fdk_assert!(false); // All valid states should be handled above!
        }
    }
}

/// `SpatialDecConcealment_SetParam(self, param, value)` (`self` is `None` for `NULL`).
pub fn spatial_dec_concealment_set_param(
    self_: Option<&mut SpatialDecConcealmentInfo>,
    param: SacDecConcealParam,
    value: i32,
) -> SacdecError {
    match param {
        SAC_DEC_CONCEAL_METHOD => {
            match value {
                SAC_DEC_CONCEAL_WITH_ZERO_VALUED_OUTPUT | SAC_DEC_CONCEAL_BY_FADING_PARAMETERS => {}
                _ => return MPS_INVALID_PARAMETER,
            }
            match self_ {
                // store parameter value
                Some(s) => s.conceal_params.method = value,
                None => return MPS_INVALID_HANDLE,
            }
        }
        SAC_DEC_CONCEAL_NUM_KEEP_FRAMES
        | SAC_DEC_CONCEAL_FADE_OUT_SLOPE_LENGTH
        | SAC_DEC_CONCEAL_FADE_IN_SLOPE_LENGTH
        | SAC_DEC_CONCEAL_NUM_RELEASE_FRAMES => {
            if value < 0 {
                return MPS_INVALID_PARAMETER;
            }
            let Some(s) = self_ else {
                return MPS_INVALID_HANDLE;
            };
            // store parameter value
            let p = &mut s.conceal_params;
            match param {
                SAC_DEC_CONCEAL_NUM_KEEP_FRAMES => p.num_keep_frames = value as u32,
                SAC_DEC_CONCEAL_FADE_OUT_SLOPE_LENGTH => p.num_fade_out_frames = value as u32,
                SAC_DEC_CONCEAL_FADE_IN_SLOPE_LENGTH => p.num_fade_in_frames = value as u32,
                _ => p.num_release_frames = value as u32,
            }
        }
        _ => return MPS_INVALID_PARAMETER,
    }
    MPS_OK
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/conceal_types.h.

//! Port of `conceal_types.h`: error concealment structs and types.
//!
//! `CConcealmentInfo::pConcealParams` always points to the decoder's single `concealCommonData`
//! (`CConcealment_InitChannelData()`); it is not stored, the concealment functions take the
//! [`CConcealParams`] as a parameter (see [`crate::channelinfo`]).

use fdk_aac_rs_fdk::common_fix::{FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::lpc::FixpLpc;
use fdk_aac_rs_fdk::mdct::BlockType;

use crate::aac_rom::PcmDec;
use crate::rvlc_info::RVLC_MAX_SFB;
use crate::usacdec_const::M_LP_FILTER_ORDER;

/// `CONCEAL_MAX_NUM_FADE_FACTORS`
pub const CONCEAL_MAX_NUM_FADE_FACTORS: i32 = 32;

/// `FIXP_CNCL`
pub type FixpCncl = FixpDbl;
/// `CNCL_FRACT_BITS`
pub const CNCL_FRACT_BITS: i32 = fdk_aac_rs_fdk::common_fix::DFRACT_BITS;

/// `CConcealmentMethod`. Warning: Do not ever change these values.
pub type CConcealmentMethod = i32;
/// `ConcealMethodNone`
pub const ConcealMethodNone: CConcealmentMethod = -1;
/// `ConcealMethodMute`
pub const ConcealMethodMute: CConcealmentMethod = 0;
/// `ConcealMethodNoise`
pub const ConcealMethodNoise: CConcealmentMethod = 1;
/// `ConcealMethodInter`
pub const ConcealMethodInter: CConcealmentMethod = 2;
/// `ConcealMethodTonal`
pub const ConcealMethodTonal: CConcealmentMethod = 3;

/// `CConcealmentState`
pub type CConcealmentState = i32;
/// `ConcealState_Ok`
pub const ConcealState_Ok: CConcealmentState = 0;
/// `ConcealState_Single`
pub const ConcealState_Single: CConcealmentState = 1;
/// `ConcealState_FadeIn`
pub const ConcealState_FadeIn: CConcealmentState = 2;
/// `ConcealState_Mute`
pub const ConcealState_Mute: CConcealmentState = 3;
/// `ConcealState_FadeOut`
pub const ConcealState_FadeOut: CConcealmentState = 4;

/// `CConcealParams`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CConcealParams {
    /// `fadeOutFactor`
    pub fade_out_factor: [FixpSgl; CONCEAL_MAX_NUM_FADE_FACTORS as usize],
    /// `fadeInFactor`
    pub fade_in_factor: [FixpSgl; CONCEAL_MAX_NUM_FADE_FACTORS as usize],

    /// `method`
    pub method: CConcealmentMethod,

    /// `numFadeOutFrames`
    pub num_fade_out_frames: i32,
    /// `numFadeInFrames`
    pub num_fade_in_frames: i32,
    /// `numMuteReleaseFrames`
    pub num_mute_release_frames: i32,
    /// `comfortNoiseLevel`
    pub comfort_noise_level: FixpDbl,
}

impl Default for CConcealParams {
    fn default() -> Self {
        CConcealParams {
            fade_out_factor: [0; CONCEAL_MAX_NUM_FADE_FACTORS as usize],
            fade_in_factor: [0; CONCEAL_MAX_NUM_FADE_FACTORS as usize],
            method: 0,
            num_fade_out_frames: 0,
            num_fade_in_frames: 0,
            num_mute_release_frames: 0,
            comfort_noise_level: 0,
        }
    }
}

/// `TDfadingType`
pub type TDfadingType = i32;
/// `FADE_TIMEDOMAIN_TOSPECTRALMUTE`
pub const FADE_TIMEDOMAIN_TOSPECTRALMUTE: TDfadingType = 1;
/// `FADE_TIMEDOMAIN_FROMSPECTRALMUTE`
pub const FADE_TIMEDOMAIN_FROMSPECTRALMUTE: TDfadingType = 2;
/// `FADE_TIMEDOMAIN`
pub const FADE_TIMEDOMAIN: TDfadingType = 3;

/// `CConcealmentInfo`. `pConcealParams` is not stored, see the [module documentation](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CConcealmentInfo {
    /// `spectralCoefficient`
    pub spectral_coefficient: [FixpCncl; 1024],
    /// `specScale`
    pub spec_scale: [i16; 8],

    /// `iRandomPhase`
    pub i_random_phase: i32,
    /// `prevFrameOk`
    pub prev_frame_ok: [i32; 2],
    /// `cntValidFrames`
    pub cnt_valid_frames: i32,
    /// `cntFadeFrames`: state for signal fade-in/out
    pub cnt_fade_frames: i32,
    /// `winGrpOffset`: state for signal fade-out of frames with more than one window/subframe
    /// (`[0]` used by Update CntFadeFrames mode of `CConcealment_ApplyFadeOut`, `[1]` used by
    /// FadeOut mode)
    pub win_grp_offset: [i32; 2],
    /// `attGrpOffset`: state for faster signal fade-out of frames with transient signal parts
    pub att_grp_offset: [i32; 2],

    /// `lastRenderMode`
    pub last_render_mode: i8,

    /// `windowShape`
    pub window_shape: u8,
    /// `windowSequence`
    pub window_sequence: BlockType,
    /// `lastWinGrpLen`
    pub last_win_grp_len: u8,

    /// `concealState`
    pub conceal_state: CConcealmentState,
    /// `concealState_old`
    pub conceal_state_old: CConcealmentState,
    /// `fade_old`: last fading factor
    pub fade_old: FixpDbl,
    /// `lastFadingType`: last fading type
    pub last_fading_type: TDfadingType,

    /// `aRvlcPreviousScaleFactor`: needed once per channel
    pub a_rvlc_previous_scale_factor: [i16; RVLC_MAX_SFB as usize],
    /// `aRvlcPreviousCodebook`: needed once per channel
    pub a_rvlc_previous_codebook: [u8; RVLC_MAX_SFB as usize],
    /// `rvlcPreviousScaleFactorOK`
    pub rvlc_previous_scale_factor_ok: i8,
    /// `rvlcPreviousBlockType`
    pub rvlc_previous_block_type: i8,

    /// `lsf4`
    pub lsf4: [FixpLpc; M_LP_FILTER_ORDER as usize],
    /// `last_tcx_gain`
    pub last_tcx_gain: FixpDbl,
    /// `last_tcx_gain_e`
    pub last_tcx_gain_e: i32,
    /// `TDNoiseSeed`
    pub td_noise_seed: u32,
    /// `TDNoiseStates`
    pub td_noise_states: [PcmDec; 3],
    /// `TDNoiseCoef`
    pub td_noise_coef: [FixpSgl; 3],
    /// `TDNoiseAtt`
    pub td_noise_att: FixpSgl,
}

impl Default for CConcealmentInfo {
    fn default() -> Self {
        CConcealmentInfo {
            spectral_coefficient: [0; 1024],
            spec_scale: [0; 8],
            i_random_phase: 0,
            prev_frame_ok: [0; 2],
            cnt_valid_frames: 0,
            cnt_fade_frames: 0,
            win_grp_offset: [0; 2],
            att_grp_offset: [0; 2],
            last_render_mode: 0,
            window_shape: 0,
            window_sequence: 0,
            last_win_grp_len: 0,
            conceal_state: 0,
            conceal_state_old: 0,
            fade_old: 0,
            last_fading_type: 0,
            a_rvlc_previous_scale_factor: [0; RVLC_MAX_SFB as usize],
            a_rvlc_previous_codebook: [0; RVLC_MAX_SFB as usize],
            rvlc_previous_scale_factor_ok: 0,
            rvlc_previous_block_type: 0,
            lsf4: [0; M_LP_FILTER_ORDER as usize],
            last_tcx_gain: 0,
            last_tcx_gain_e: 0,
            td_noise_seed: 0,
            td_noise_states: [0; 3],
            td_noise_coef: [0; 3],
            td_noise_att: 0,
        }
    }
}

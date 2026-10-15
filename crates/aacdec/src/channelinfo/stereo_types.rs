// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/stereo.h (types and constants).

//! The types and constants of `stereo.h`, which `channelinfo.h` contains by value. They are
//! defined here; `crate::stereo` re-exports them
//! (`pub use crate::channelinfo::stereo_types::*;`) and adds the functions.
//!
//! # Pointers of [`CJointStereoPersistentData`]
//!
//! `CAacDecoder_Init()` and `CAacDecoder_DecodeFrame()` set them; the port does not store them:
//!
//! | C | Points to | Port |
//! |---|---|---|
//! | `spectralCoeffs[ch2]` | `pAacDecoderStaticChannelInfo[ch]->concealmentInfo.spectralCoefficient` of channel `ch2` of the element | taken from the static channel info of that channel |
//! | `specScale[ch2]` | `pAacDecoderStaticChannelInfo[ch]->concealmentInfo.specScale` of channel `ch2` of the element | taken from the static channel info of that channel |
//! | `scratchBuffer` | the decoder's `pTimeData2` (`CAacDecoder_Init()`), then the time data buffer of `CAacDecoder_DecodeFrame()` (set before every `CChannelElement_Decode()` of a USAC element) | passed explicitly to the functions that use it (`CJointStereo_ApplyMS()`) |

use fdk_aac_rs_fdk::common_fix::FixpDbl;

/// `SFB_PER_PRED_BAND`
pub const SFB_PER_PRED_BAND: i32 = 2;

/// `SR_FNA_OUT`: additional scaling of the `CJointStereo_filterAndAdd()` output to avoid
/// overflows.
pub const SR_FNA_OUT: i32 = 0;

/// `JointStereoMaximumGroups`
pub const JointStereoMaximumGroups: i32 = 8;
/// `JointStereoMaximumBands`
pub const JointStereoMaximumBands: i32 = 64;

/// `CCplxPredictionData`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CCplxPredictionData {
    /// `pred_dir`: 0 = prediction from mid to side channel, 1 = vice versa
    pub pred_dir: u8,
    /// `igf_pred_dir`: 0 = prediction from mid to side channel, 1 = vice versa
    pub igf_pred_dir: u8,
    /// `complex_coef`: 0 = `alpha_q_im[x]` is 0 for all prediction bands, 1 = `alpha_q_im[x]` is
    /// transmitted via bitstream
    pub complex_coef: u8,
    /// `use_prev_frame`: 0 = use current frame for MDST estimation, 1 = use current and
    /// previous frame
    pub use_prev_frame: u8,

    /// `alpha_q_re`
    pub alpha_q_re: [[i16; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize],
    /// `alpha_q_im`
    pub alpha_q_im: [[i16; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize],
}

impl Default for CCplxPredictionData {
    fn default() -> Self {
        CCplxPredictionData {
            pred_dir: 0,
            igf_pred_dir: 0,
            complex_coef: 0,
            use_prev_frame: 0,
            alpha_q_re: [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize],
            alpha_q_im: [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize],
        }
    }
}

/// `CJointStereoData`: joint stereo scratch memory (valid for this frame).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CJointStereoData {
    /// `MsMaskPresent`
    pub ms_mask_present: u8,
    /// `MsUsed`: every array element contains flags for up to 8 groups. This array is also
    /// utilized for complex stereo prediction.
    pub ms_used: [u8; JointStereoMaximumBands as usize],
    /// `IGF_MsMaskPresent`
    pub igf_ms_mask_present: u8,

    /// `cplx_pred_flag`: stereo complex prediction was signalled for this frame
    pub cplx_pred_flag: u8,
    /// `igf_cplx_pred_flag`
    pub igf_cplx_pred_flag: u8,

    /* The following array and variable are needed for the case when INF is active */
    /// `store_dmx_re_prev`
    pub store_dmx_re_prev: [FixpDbl; 1024],
    /// `store_dmx_re_prev_e`
    pub store_dmx_re_prev_e: i16,
}

impl Default for CJointStereoData {
    fn default() -> Self {
        CJointStereoData {
            ms_mask_present: 0,
            ms_used: [0; JointStereoMaximumBands as usize],
            igf_ms_mask_present: 0,
            cplx_pred_flag: 0,
            igf_cplx_pred_flag: 0,
            store_dmx_re_prev: [0; 1024],
            store_dmx_re_prev_e: 0,
        }
    }
}

/// `CJointStereoPersistentData`: joint stereo persistent memory. The pointers `scratchBuffer`,
/// `spectralCoeffs[2]` and `specScale[2]` are not stored, see the
/// [module documentation](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CJointStereoPersistentData {
    /// `clearSpectralCoeffs`: indicates that the spectral coeffs must be cleared because the
    /// transform splitting active flag of the left and right channel was different
    pub clear_spectral_coeffs: u8,

    /// `alpha_q_re_prev`
    pub alpha_q_re_prev:
        [[i16; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize],
    /// `alpha_q_im_prev`
    pub alpha_q_im_prev:
        [[i16; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize],

    /// `winSeqPrev`
    pub win_seq_prev: u8,
    /// `winShapePrev`
    pub win_shape_prev: u8,
    /// `winGroupsPrev`
    pub win_groups_prev: u8,
}

impl Default for CJointStereoPersistentData {
    fn default() -> Self {
        CJointStereoPersistentData {
            clear_spectral_coeffs: 0,
            alpha_q_re_prev: [[0; JointStereoMaximumBands as usize];
                JointStereoMaximumGroups as usize],
            alpha_q_im_prev: [[0; JointStereoMaximumBands as usize];
                JointStereoMaximumGroups as usize],
            win_seq_prev: 0,
            win_shape_prev: 0,
            win_groups_prev: 0,
        }
    }
}

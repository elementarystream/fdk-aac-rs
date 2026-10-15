// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/psy_const.h.

//! Port of `psy_const.h`: constants of the psychoacoustic model.

/// `TRUE`
pub const TRUE: i32 = 1;
/// `FALSE`
pub const FALSE: i32 = 0;

/// `TRANS_FAC`: encoder short long ratio
pub const TRANS_FAC: i32 = 8;

/// `FRAME_MAXLEN_SHORT`
pub const FRAME_MAXLEN_SHORT: i32 = 1024 / TRANS_FAC;
/// `FRAME_LEN_SHORT_128`
pub const FRAME_LEN_SHORT_128: i32 = 1024 / TRANS_FAC;

/// `FB_TYPE`: filterbank type
pub type FbType = i32;
/// `FB_LC`
pub const FB_LC: FbType = 0;
/// `FB_LD`
pub const FB_LD: FbType = 1;
/// `FB_ELD`
pub const FB_ELD: FbType = 2;

/// `N_BLOCKTYPES`
pub const N_BLOCKTYPES: i32 = 6;
/// `LONG_WINDOW`
pub const LONG_WINDOW: i32 = 0;
/// `START_WINDOW`
pub const START_WINDOW: i32 = 1;
/// `SHORT_WINDOW`
pub const SHORT_WINDOW: i32 = 2;
/// `STOP_WINDOW`
pub const STOP_WINDOW: i32 = 3;
/// `_LOWOV_WINDOW`: do not use this block type outside of block_switch
pub const _LOWOV_WINDOW: i32 = 4;
/// `WRONG_WINDOW`
pub const WRONG_WINDOW: i32 = 5;

/// `SINE_WINDOW`
pub const SINE_WINDOW: i32 = 0;
/// `KBD_WINDOW`
pub const KBD_WINDOW: i32 = 1;
/// `LOL_WINDOW`: Low OverLap window shape for AAC-LD
pub const LOL_WINDOW: i32 = 2;

/// `SI_MS_MASK_NONE`
pub const SI_MS_MASK_NONE: i32 = 0;
/// `SI_MS_MASK_SOME`
pub const SI_MS_MASK_SOME: i32 = 1;
/// `SI_MS_MASK_ALL`
pub const SI_MS_MASK_ALL: i32 = 2;

/// `MAX_NO_OF_GROUPS`
pub const MAX_NO_OF_GROUPS: usize = 4;
/// `MAX_SFB_LONG`: 51 for a memory optimized implementation
pub const MAX_SFB_LONG: usize = 51;
/// `MAX_SFB_SHORT`: 15 for a memory optimized implementation
pub const MAX_SFB_SHORT: usize = 15;
/// `MAX_SFB` (= 51)
pub const MAX_SFB: usize = if MAX_SFB_SHORT > MAX_SFB_LONG { MAX_SFB_SHORT } else { MAX_SFB_LONG };
/// `MAX_GROUPED_SFB` (= 60)
pub const MAX_GROUPED_SFB: usize = if MAX_NO_OF_GROUPS * MAX_SFB_SHORT > MAX_SFB_LONG {
    MAX_NO_OF_GROUPS * MAX_SFB_SHORT
} else {
    MAX_SFB_LONG
};

/// `MAX_INPUT_BUFFER_SIZE` (2048)
pub const MAX_INPUT_BUFFER_SIZE: usize = 2 * 1024;

/// `PCM_LEVEL`
pub const PCM_LEVEL: f32 = 1.0;
/// `NORM_PCM`
pub const NORM_PCM: f32 = PCM_LEVEL / 32768.0;
/// `NORM_PCM_ENERGY`
pub const NORM_PCM_ENERGY: f32 = NORM_PCM * NORM_PCM;
/// `LOG_NORM_PCM`
pub const LOG_NORM_PCM: i32 = -15;

/// `TNS_PREDGAIN_SCALE`
pub const TNS_PREDGAIN_SCALE: i32 = 1000;

/// `LFE_LOWPASS_LINE`
pub const LFE_LOWPASS_LINE: i32 = 12;
/// `LFE_LOWPASS_LINE_MIN`
pub const LFE_LOWPASS_LINE_MIN: i32 = 4;

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_staticgain.h, libSACenc/src/sacenc_staticgain.cpp.

//! Port of `sacenc_staticgain.h` and `sacenc_staticgain.cpp`: the static pre and post gains
//! of the encoder.
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds apart from the expansion of
//! `SATURATE_LEFT_SHIFT` (same values); the results differ through `fMult`.
//!
//! The pre gain factor (`preGainFactorDb`, always 0 in the library) and the downmix gain index
//! select table entries. Bounded: values outside the tables read 0.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::machine_type::IntPcm;

use crate::sacenc_bitstream::*;
use crate::sacenc_const::SACENC_MAX_INPUT_CHANNELS;
use crate::sacenc_lib::*;

mod tables;
pub use tables::*;

/// `FIXP_GAIN`
pub type FixpGain = FixpDbl;
/// `MAXVAL_GAIN`
pub const MAXVAL_GAIN: FixpGain = MAXVAL_DBL;

/// `MP4SPACEENC_DMX_GAIN_DEFAULT`
const MP4SPACEENC_DMX_GAIN_DEFAULT: Mp4SpaceEncDmxGain = SACENC_DMXGAIN_3_dB;
/// `GAINCF_SF`
const GAINCF_SF: i32 = 4;

/// `struct STATIC_GAIN_CONFIG`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticGainConfig {
    pub enc_mode: Mp4SpaceEncMode,
    pub fixed_gain_dmx: Mp4SpaceEncDmxGain,
    pub pre_gain_factor_db: i32,
}

/// `HANDLE_STATIC_GAIN_CONFIG`
pub type HandleStaticGainConfig = Option<Box<StaticGainConfig>>;

/// `struct STATIC_GAIN`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticGain {
    // External Config Values
    pub enc_mode: Mp4SpaceEncMode,
    pub fixed_gain_dmx: Mp4SpaceEncDmxGain,
    pub pre_gain_factor_db: i32,

    // Internal Values
    pub post_gain_fdk: FixpGain,
    pub p_pre_gain_fdk: [FixpGain; SACENC_MAX_INPUT_CHANNELS],
}

/// `HANDLE_STATIC_GAIN`
pub type HandleStaticGain = Option<Box<StaticGain>>;

/// `fdk_sacenc_staticGain_OpenConfig(phStaticGainConfig)`: allocates a zeroed configuration.
pub fn fdk_sacenc_static_gain_open_config(
    ph_static_gain_config: Option<&mut HandleStaticGainConfig>,
) -> FdkSacencError {
    match ph_static_gain_config {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = Some(Box::default());
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_staticGain_InitDefaultConfig(hStaticGainConfig)`
pub fn fdk_sacenc_static_gain_init_default_config(
    h_static_gain_config: Option<&mut StaticGainConfig>,
) -> FdkSacencError {
    match h_static_gain_config {
        None => SACENC_INVALID_HANDLE,
        Some(c) => {
            // Necessary Input Variables
            c.enc_mode = SACENC_INVALID_MODE;

            // Optional Configs Set to Default Values
            c.fixed_gain_dmx = MP4SPACEENC_DMX_GAIN_DEFAULT;
            c.pre_gain_factor_db = 0;
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_staticGain_CloseConfig(phStaticGainConfig)`
pub fn fdk_sacenc_static_gain_close_config(
    ph_static_gain_config: Option<&mut HandleStaticGainConfig>,
) -> FdkSacencError {
    match ph_static_gain_config {
        Some(h) if h.is_some() => {
            *h = None;
            SACENC_OK
        }
        _ => SACENC_INVALID_HANDLE,
    }
}

/// `fdk_sacenc_staticGain_Open(phStaticGain)`: allocates a zeroed instance.
pub fn fdk_sacenc_static_gain_open(
    ph_static_gain: Option<&mut HandleStaticGain>,
) -> FdkSacencError {
    match ph_static_gain {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = Some(Box::default());
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_staticGain_Init(hStaticGain, hStaticGainConfig, scale)`
pub fn fdk_sacenc_static_gain_init(
    h_static_gain: &mut StaticGain,
    h_static_gain_config: &StaticGainConfig,
    scale: &mut i32,
) -> FdkSacencError {
    h_static_gain.enc_mode = h_static_gain_config.enc_mode;
    h_static_gain.fixed_gain_dmx = h_static_gain_config.fixed_gain_dmx;
    h_static_gain.pre_gain_factor_db = h_static_gain_config.pre_gain_factor_db;

    if (h_static_gain.pre_gain_factor_db < -20) || (h_static_gain.pre_gain_factor_db > 20) {
        return SACENC_INVALID_CONFIG;
    }

    let mut f_pre_gain_factor_fdk: FixpDbl;

    if h_static_gain.pre_gain_factor_db == 0 {
        f_pre_gain_factor_fdk = MAXVAL_DBL;
        *scale = 0;
    } else {
        f_pre_gain_factor_fdk =
            PRE_GAIN_FACTOR_TABLE_FDK[(h_static_gain.pre_gain_factor_db + 20) as usize];
        let s = fix_max(0, cnt_leading_zeros(f_pre_gain_factor_fdk) - 1);
        f_pre_gain_factor_fdk <<= s;
        *scale = GAINCF_SF - s;
    }

    if h_static_gain.fixed_gain_dmx == 0 {
        h_static_gain.post_gain_fdk = MAXVAL_GAIN;
    } else {
        // Outside of the table: see the module documentation.
        h_static_gain.post_gain_fdk = DMX_GAIN_TABLE_FDK
            .get(h_static_gain.fixed_gain_dmx.wrapping_sub(1) as u32 as usize)
            .copied()
            .unwrap_or(0);
    }

    // zero all input channels
    h_static_gain.p_pre_gain_fdk = [0; SACENC_MAX_INPUT_CHANNELS];

    // Configure PreGain-Vector
    if h_static_gain.enc_mode == SACENC_212 {
        h_static_gain.p_pre_gain_fdk[0] = f_pre_gain_factor_fdk; // L
        h_static_gain.p_pre_gain_fdk[1] = f_pre_gain_factor_fdk; // R
        SACENC_OK
    } else {
        SACENC_INVALID_CONFIG
    }
}

/// `fdk_sacenc_staticGain_Close(phStaticGain)`
pub fn fdk_sacenc_static_gain_close(
    ph_static_gain: Option<&mut HandleStaticGain>,
) -> FdkSacencError {
    match ph_static_gain {
        Some(h) if h.is_some() => {
            *h = None;
            SACENC_OK
        }
        _ => SACENC_INVALID_HANDLE,
    }
}

/// `fdk_sacenc_staticPostGain_ApplyFDK(hStaticGain, pOutputSamples, nOutputSamples, scale)`:
/// multiplies the output samples with the post gain.
pub fn fdk_sacenc_static_post_gain_apply_fdk(
    h_static_gain: &StaticGain,
    p_output_samples: &mut [IntPcm],
    n_output_samples: i32,
    scale: i32,
) -> FdkSacencError {
    let post_gain = h_static_gain.post_gain_fdk;
    let out = &mut p_output_samples[..n_output_samples.max(0) as usize];

    if scale < 0 {
        let s = scale.wrapping_neg() as u32;
        if post_gain == MAXVAL_GAIN {
            for v in out.iter_mut() {
                *v = (*v as i32).wrapping_shr(s) as IntPcm;
            }
        } else {
            for v in out.iter_mut() {
                *v = fx_dbl2fx_pcm(f_mult(post_gain, fx_pcm2fx_dbl(*v)).wrapping_shr(s));
            }
        }
    } else if post_gain == MAXVAL_GAIN {
        for v in out.iter_mut() {
            *v = fx_dbl2fx_pcm(saturate_left_shift(fx_pcm2fx_dbl(*v), scale, DFRACT_BITS));
        }
    } else {
        for v in out.iter_mut() {
            *v = fx_dbl2fx_pcm(saturate_left_shift(
                f_mult(post_gain, fx_pcm2fx_dbl(*v)),
                scale,
                DFRACT_BITS,
            ));
        }
    }
    SACENC_OK
}

/// `fdk_sacenc_getPreGainPtrFDK(hStaticGain)`: the pre gains of the input channels.
pub fn fdk_sacenc_get_pre_gain_ptr_fdk(
    h_static_gain: &StaticGain,
) -> &[FixpGain; SACENC_MAX_INPUT_CHANNELS] {
    &h_static_gain.p_pre_gain_fdk
}

/// `fdk_sacenc_getPostGainFDK(hStaticGain)`
pub fn fdk_sacenc_get_post_gain_fdk(h_static_gain: &StaticGain) -> FixpGain {
    h_static_gain.post_gain_fdk
}

/// `fdk_sacenc_staticGain_GetDmxGain(hStaticGain)`: the fixed downmix gain as bitstream value.
pub fn fdk_sacenc_static_gain_get_dmx_gain(h_static_gain: &StaticGain) -> FixedGainDmxConfig {
    match h_static_gain.fixed_gain_dmx {
        0 => FIXEDGAINDMX_0,
        1 => FIXEDGAINDMX_1,
        2 => FIXEDGAINDMX_2,
        3 => FIXEDGAINDMX_3,
        4 => FIXEDGAINDMX_4,
        5 => FIXEDGAINDMX_5,
        6 => FIXEDGAINDMX_6,
        7 => FIXEDGAINDMX_7,
        _ => FIXEDGAINDMX_INVALID,
    }
}

/// `fdk_sacenc_staticGain_SetDmxGain(hStaticGainCfg, dmxGain)`
pub fn fdk_sacenc_static_gain_set_dmx_gain(
    h_static_gain_cfg: Option<&mut StaticGainConfig>,
    dmx_gain: Mp4SpaceEncDmxGain,
) -> FdkSacencError {
    match h_static_gain_cfg {
        None => SACENC_INVALID_HANDLE,
        Some(c) => {
            c.fixed_gain_dmx = dmx_gain;
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_staticGain_SetEncMode(hStaticGainCfg, encMode)`
pub fn fdk_sacenc_static_gain_set_enc_mode(
    h_static_gain_cfg: Option<&mut StaticGainConfig>,
    enc_mode: Mp4SpaceEncMode,
) -> FdkSacencError {
    match h_static_gain_cfg {
        None => SACENC_INVALID_HANDLE,
        Some(c) => {
            c.enc_mode = enc_mode;
            SACENC_OK
        }
    }
}

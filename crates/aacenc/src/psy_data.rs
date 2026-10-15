// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/psy_data.h.

//! Port of `psy_data.h`: psychoacoustic data.
//!
//! # Unions
//!
//! `SFB_THRESHOLD`, `SFB_ENERGY`, `SFB_LD_ENERGY` and `SFB_MAX_SCALE` are unions of
//! `Long[MAX_GROUPED_SFB]` and `Short[TRANS_FAC][MAX_SFB_SHORT]`. The C code writes the `.Short`
//! member and reads the `.Long` member (`FDKaacEnc_groupShortData()` regroups the short values
//! into `.Long` in place, `FDKaacEnc_psyMain()` passes `.Long` of short blocks on), so each union
//! is one flat array of `SFB_UNION_LEN` words: `Long[sfb]` is element `sfb`, `Short[w][sfb]` is
//! element `w * MAX_SFB_SHORT + sfb`.

use crate::block_switch::BlockSwitchingControl;
use crate::psy_const::{MAX_GROUPED_SFB, MAX_INPUT_BUFFER_SIZE, MAX_SFB, MAX_SFB_SHORT, TRANS_FAC};
use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::mdct::MdctT;

/// Number of words of the `SFB_*` unions (`Short[TRANS_FAC][MAX_SFB_SHORT]` is the larger
/// member).
pub const SFB_UNION_LEN: usize = if TRANS_FAC as usize * MAX_SFB_SHORT > MAX_GROUPED_SFB {
    TRANS_FAC as usize * MAX_SFB_SHORT
} else {
    MAX_GROUPED_SFB
};

/// Index of `.Short[w][0]` in an `SFB_*` union.
pub const fn sfb_short(w: usize) -> usize {
    w * MAX_SFB_SHORT
}

/// `SFB_THRESHOLD` (union, see the [module documentation](self)).
pub type SfbThreshold = [FixpDbl; SFB_UNION_LEN];
/// `SFB_ENERGY` (union, see the [module documentation](self)).
pub type SfbEnergy = [FixpDbl; SFB_UNION_LEN];
/// `SFB_LD_ENERGY` (union, see the [module documentation](self)).
pub type SfbLdEnergy = [FixpDbl; SFB_UNION_LEN];
/// `SFB_MAX_SCALE` (union, see the [module documentation](self)).
pub type SfbMaxScale = [i32; SFB_UNION_LEN];

/// Size of `PSY_STATIC::overlapAddBuffer`.
pub const OVERLAP_ADD_BUFFER_SIZE: usize = 3 * 512 / 2;

/// `PSY_STATIC`
///
/// `psyInputBuffer` points to a separately allocated buffer of `MAX_INPUT_BUFFER_SIZE` samples
/// in C (`GetRam_aacEnc_PsyInputBuffer`); here the buffer is owned. The overlap buffer of
/// `mdctPers` is owned by [`MdctT`] (the encoder initializes it with size 0).
#[derive(Clone, Debug)]
pub struct PsyStatic {
    pub psy_input_buffer: Vec<IntPcm>,
    pub overlap_add_buffer: [FixpDbl; OVERLAP_ADD_BUFFER_SIZE],
    /// MDCT persistent data
    pub mdct_pers: MdctT,
    /// block switching
    pub block_switching_control: BlockSwitchingControl,
    /// FDKaacEnc_PreEchoControl
    pub sfb_thresholdnm1: [FixpDbl; MAX_SFB],
    /// scale of last block's mdct (FDKaacEnc_PreEchoControl)
    pub mdct_scalenm1: i32,
    pub calc_pre_echo: i32,
    pub is_lfe: i32,
}

impl Default for PsyStatic {
    fn default() -> Self {
        PsyStatic {
            psy_input_buffer: vec![0; MAX_INPUT_BUFFER_SIZE],
            overlap_add_buffer: [0; OVERLAP_ADD_BUFFER_SIZE],
            mdct_pers: MdctT::default(),
            block_switching_control: BlockSwitchingControl::default(),
            sfb_thresholdnm1: [0; MAX_SFB],
            mdct_scalenm1: 0,
            calc_pre_echo: 0,
            is_lfe: 0,
        }
    }
}

/// `PSY_DATA` without its first member `mdctSpectrum`.
///
/// In C, `mdctSpectrum` points to `QC_OUT_CHANNEL::mdctSpectrum` of the channel (set by
/// `FDKaacEnc_psyMain()` from `PSY_OUT_CHANNEL::mdctSpectrum`, which `FDKaacEnc_EncodeFrame()`
/// points there); the port passes that spectrum separately. The remaining members have the
/// layout of C (from byte offset 8 of the C struct on 64-bit targets).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsyData {
    /// adapt
    pub sfb_threshold: SfbThreshold,
    /// sfb energies
    pub sfb_energy: SfbEnergy,
    /// sfb energies in ldData format
    pub sfb_energy_ld_data: SfbLdEnergy,
    pub sfb_max_scale_spec: SfbMaxScale,
    /// mid/side sfb energies
    pub sfb_energy_ms: SfbEnergy,
    /// mid/side sfb energies in ldData format
    pub sfb_energy_ms_ld_data: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_spread_energy: SfbEnergy,
    /// exponent of data in mdctSpectrum
    pub mdct_scale: i32,
    pub grouped_sfb_offset: [i32; MAX_GROUPED_SFB + 1],
    pub sfb_active: i32,
    pub lowpass_line: i32,
}

impl Default for PsyData {
    fn default() -> Self {
        PsyData {
            sfb_threshold: [0; SFB_UNION_LEN],
            sfb_energy: [0; SFB_UNION_LEN],
            sfb_energy_ld_data: [0; SFB_UNION_LEN],
            sfb_max_scale_spec: [0; SFB_UNION_LEN],
            sfb_energy_ms: [0; SFB_UNION_LEN],
            sfb_energy_ms_ld_data: [0; MAX_GROUPED_SFB],
            sfb_spread_energy: [0; SFB_UNION_LEN],
            mdct_scale: 0,
            grouped_sfb_offset: [0; MAX_GROUPED_SFB + 1],
            sfb_active: 0,
            lowpass_line: 0,
        }
    }
}

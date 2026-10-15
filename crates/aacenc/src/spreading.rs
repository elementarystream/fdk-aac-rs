// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/spreading.h, libAACenc/src/spreading.cpp.

//! Port of `spreading.h`: spreading of energy.
//!
//! The `aarch64` and `x86` translation units of `spreading.cpp` are identical; the variants
//! differ only in `fMult`.

use fdk_aac_rs_fdk::common_fix::*;

/// `FDKaacEnc_SpreadingMax()`: spreads `pb_spread_energy[0..pb_cnt]` in place with the slopes
/// `mask_high_factor` (to higher frequencies) and `mask_low_factor` (to lower frequencies).
///
/// The callers pass `pbCnt >= 1`; where the reference has no defined result, the port reads
/// element 0.
pub fn fdk_aac_enc_spreading_max(
    pb_cnt: i32,
    mask_low_factor: &[FixpDbl],
    mask_high_factor: &[FixpDbl],
    pb_spread_energy: &mut [FixpDbl],
) {
    // slope to higher frequencies
    let mut delay: FixpDbl = pb_spread_energy[0];
    for i in 1..pb_cnt.max(0) as usize {
        delay = fix_max(pb_spread_energy[i], f_mult(mask_high_factor[i], delay));
        pb_spread_energy[i] = delay;
    }

    // slope to lower frequencies
    delay = pb_spread_energy[(pb_cnt - 1).max(0) as usize];
    for i in (0..(pb_cnt - 1).max(0) as usize).rev() {
        delay = fix_max(pb_spread_energy[i], f_mult(mask_low_factor[i], delay));
        pb_spread_energy[i] = delay;
    }
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/quantize.h, libAACenc/src/quantize.cpp.

//! Port of `quantize.h`: quantization and inverse quantization of the spectrum.
//!
//! # Compiled behavior
//!
//! The function bodies are the same in the `aarch64` and `x86` translation units, but the static
//! helpers `FDKaacEnc_quantizeLines()` and `FDKaacEnc_invQuantizeLines()` and the distortion
//! computation are inlined into each caller, and some of their operations are undefined for
//! out-of-range inputs (gains that do not fit the spectrum). The port reproduces the code the
//! reference builds generated (checked in `work/fdk-aac-asm/ref-arm64`, `ref-x86_64` and
//! `ref-i686`):
//!
//! - quantization: the shift `accu >>= fixMin(totalShift, DFRACT_BITS - 1)` with a negative
//!   `totalShift` is a register shift, i.e. by the amount modulo 32 (both builds);
//! - inverse quantization: both shift directions are register shifts (amount modulo 32);
//! - distortion `diff = fixp_abs(fixp_abs(invQuantSpec) - fixp_abs(mdctSpectrum[i] >> 1))`
//!   followed by `CountLeadingBits()`/`scaleValue()`: the compilers assume that `fixp_abs()`
//!   does not return a negative value; they differ where `invQuantSpec == MINVAL_DBL` (see
//!   `dist_line`);
//! - `fAbs(quantSpectrum[i]) > MAX_QUANT` is evaluated in 16 bits; the inverse quantization
//!   tables `FDKaacEnc_specExpMantTableCombElc` and `FDKaacEnc_specExpTableComb` are read with
//!   the table layout of the reference reproduced (see `spec_exp_mant_table_comb_elc`).

use crate::aac_enc_rom::{
    ELD_ANALYSIS_512, FDK_AAC_ENC_M_TAB_3_4, FDK_AAC_ENC_M_TAB_4_3_ELC, FDK_AAC_ENC_QUANT_TABLE_E,
    FDK_AAC_ENC_QUANT_TABLE_Q, FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC,
    FDK_AAC_ENC_SPEC_EXP_TABLE_COMB, MANT_DIGITS, MANT_SIZE,
};
use fdk_aac_rs_fdk::common_fix::{
    f_mult, f_mult_div2, f_pow2, fix_min, fl2fxconst_dbl, FixpDbl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::fixpoint_math::calc_ld_data;
use fdk_aac_rs_fdk::{Arch, ARCH};

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

/// `MAX_QUANT`
pub const MAX_QUANT: i32 = 8191;

/// `FDKaacEnc_specExpMantTableCombElc[mod][specExp]`, read as the flat array of the reference.
///
/// The port reproduces the table layout of the reference: a read past a row reaches the next
/// row, and past the last row the next table in read-only data, `FDKaacEnc_specExpTableComb` in
/// both reference builds (the port reads its bytes, little endian).
#[inline]
fn spec_exp_mant_table_comb_elc(m: i32, spec_exp: i32) -> FixpDbl {
    let f = (m * 14 + spec_exp) as usize;
    if f < 56 {
        FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC[f / 14][f % 14]
    } else {
        let b = |i: usize| spec_exp_table_comb(i as i32 / 14, i as i32 % 14) as u8;
        let i = (f - 56) * 4;
        i32::from_le_bytes([b(i), b(i + 1), b(i + 2), b(i + 3)])
    }
}

/// `FDKaacEnc_specExpTableComb[mod][specExp]`, read as the flat array of the reference.
///
/// Past the table, modelled per variant: in the arm64 and i686 builds the next object is
/// `ELDAnalysis512` (the port reads its bytes, little endian), in the x86-64 build 8 bytes of
/// alignment padding (0).
#[inline]
fn spec_exp_table_comb(m: i32, spec_exp: i32) -> i32 {
    let f = (m * 14 + spec_exp) as usize;
    if f < 56 {
        FDK_AAC_ENC_SPEC_EXP_TABLE_COMB[f / 14][f % 14] as i32
    } else {
        let i = f - 56;
        match ARCH {
            Arch::X86 => 0,
            _ => ELD_ANALYSIS_512[i / 2].to_le_bytes()[i % 2] as i32,
        }
    }
}

/// `FDKaacEnc_quantizeLines`: quantizes spectrum lines.
///
/// input: global gain, number of lines to process, spectral data;
/// output: quantized spectrum
fn fdk_aac_enc_quantize_lines(
    gain: i32,
    no_of_lines: i32,
    mdct_spectrum: &[FixpDbl],
    qua_spectrum: &mut [i16],
    d_zone_quant_enable: i32,
) {
    let quantizer = FDK_AAC_ENC_QUANT_TABLE_Q[(gain.wrapping_neg() & 3) as usize];
    let quantizershift = (gain.wrapping_neg() >> 2) + 1;
    const K_SHIFT: i32 = 16;

    let k: FixpDbl = if d_zone_quant_enable != 0 {
        fl2fxconst_dbl(0.23f32 as f64) >> K_SHIFT
    } else {
        fl2fxconst_dbl((-0.0946f32 + 0.5f32) as f64) >> K_SHIFT
    };

    for line in 0..no_of_lines.max(0) as usize {
        let mut accu: FixpDbl = f_mult_div2(mdct_spectrum[line], quantizer);

        if accu != 0 {
            let neg = accu < 0;
            if neg {
                accu = -accu;
            }
            // normalize
            let accu_shift = accu.leading_zeros() as i32 - 1; // CountLeadingBits() is not
                                                              // necessary here since test
                                                              // value is always > 0
            accu <<= accu_shift;
            let tab_index = (accu >> (DFRACT_BITS - 2 - MANT_DIGITS)) & !MANT_SIZE;
            let mut total_shift = quantizershift.wrapping_sub(accu_shift).wrapping_add(1);
            accu = f_mult_div2(
                FDK_AAC_ENC_M_TAB_3_4[tab_index as usize],
                FDK_AAC_ENC_QUANT_TABLE_E[(total_shift & 3) as usize],
            );
            // C: totalShift = (16 - 4) - (3 * (totalShift >> 2));
            //    accu >>= fixMin(totalShift, DFRACT_BITS - 1);
            // FDK_ASSERT(totalShift >= 0) (MAX_QUANT_VIOLATION) is violated by gains that do not
            // fit the spectrum; both builds compute min(-3 * (totalShift >> 2), 19) + 12 and shift
            // by it modulo 32 (asr / sar with a register amount).
            total_shift = fix_min(3i32.wrapping_mul(total_shift >> 2).wrapping_neg(), 19) + 12;
            fdk_aac_rs_fdk::fdk_assert!(total_shift >= 0);
            accu = accu.wrapping_shr(total_shift as u32);
            let q = (k.wrapping_add(accu) >> (DFRACT_BITS - 1 - 16)) as i16;
            qua_spectrum[line] = if neg { q.wrapping_neg() } else { q };
        } else {
            qua_spectrum[line] = 0;
        }
    }
}

/// `FDKaacEnc_invQuantizeLines` for one line: `mdctSpectrum = iquaSpectrum^4/3 *2^(0.25*gain)`.
#[inline]
fn fdk_aac_enc_inv_quantize_line(gain: i32, quant_spectrum: i16) -> FixpDbl {
    let iquantizermod = gain & 3;
    let iquantizershift = gain >> 2;

    if quant_spectrum == 0 {
        return fl2fxconst_dbl(0.0);
    }
    let mut accu: FixpDbl = (quant_spectrum as i32).abs();

    // ex = CountLeadingBits(accu) (accu > 0)
    let ex = accu.leading_zeros() as i32 - 1;
    accu <<= ex;
    let spec_exp = (DFRACT_BITS - 1) - ex;

    // this fails if abs(value) > 8191
    fdk_aac_rs_fdk::fdk_assert!(spec_exp < 14);

    let tab_index = (accu >> (DFRACT_BITS - 2 - MANT_DIGITS)) & !MANT_SIZE;

    // calculate "mantissa" ^4/3
    let s = FDK_AAC_ENC_M_TAB_4_3_ELC[tab_index as usize];

    // get approperiate exponent multiplier for specExp^3/4 combined with scfMod
    let t = spec_exp_mant_table_comb_elc(iquantizermod, spec_exp);

    // multiply "mantissa" ^4/3 with exponent multiplier
    accu = f_mult(s, t);

    // get approperiate exponent shifter
    // C: specExp = FDKaacEnc_specExpTableComb[iquantizermod][specExp] - 1;
    //    if ((-iquantizershift - specExp) < 0) accu <<= -(-iquantizershift - specExp);
    //    else accu >>= -iquantizershift - specExp;
    // Both builds compare iquantizershift + table entry with 2 and shift by register amounts
    // (modulo 32).
    let st = iquantizershift.wrapping_add(spec_exp_table_comb(iquantizermod, spec_exp));
    if st >= 2 {
        accu = accu.wrapping_shl(st.wrapping_sub(1) as u32);
    } else {
        accu = accu.wrapping_shr(1i32.wrapping_sub(st) as u32);
    }

    if quant_spectrum < 0 {
        accu.wrapping_neg()
    } else {
        accu
    }
}

/// The distortion of one line as `FDKaacEnc_calcSfbDist()` and
/// `FDKaacEnc_calcSfbQuantEnergyAndDist()` compute it:
///
/// ```c
/// diff = fixp_abs(fixp_abs(invQuantSpec) - fixp_abs(mdctSpectrum[i] >> 1));
/// scale = CountLeadingBits(diff);
/// diff = scaleValue(diff, scale);
/// diff = fPow2(diff);
/// scale = fixMin(2 * (scale - 1), DFRACT_BITS - 1);
/// diff = scaleValue(diff, -scale);
/// ```
///
/// All builds treat `diff` as non-negative (`CountLeadingBits()` becomes `clz - 1`, the right
/// shift of `scaleValue()` a logical one). The difference is the absolute value, which only
/// matters for `invQuantSpec == MINVAL_DBL` (`fixp_abs()` returns `MINVAL_DBL`):
/// - `aarch64` (ref-arm64): `diff = |a - b|` of the wrapped difference, normalized with `lsr` by
///   1 if bit 31 is set;
/// - `x86` and `x86_32` (ref-x86_64 and ref-i686, Debian clang 19, both functions): `diff =
///   |a - b|` of the wrapped difference (`neg`/`cmovs`), normalized with `shl` by
///   `(clz - 1) mod 32` (by 31 if bit 31 is set); the final scale is
///   `min((unsigned)(2 * scale), 33) - 2`, so `scale == -1` (bit 31 set) shifts right by 31
///   (ref-x86_64: `FDKaacEnc_calcSfbDist` 0x240-0x286 and 0x499-0x4d0,
///   `FDKaacEnc_calcSfbQuantEnergyAndDist` 0x690-0x6d1 and 0x7f3-0x836).
///
/// Apple clang for x86-64 (ref-x86_64-mac) computes `diff = a > b ? a - b : b - a` (signed
/// comparison) instead; the port does not reproduce that build.
#[inline]
fn dist_line(inv_quant_spec: FixpDbl, mdct_spectrum: FixpDbl) -> FixpDbl {
    let a = inv_quant_spec.wrapping_abs();
    let b = (mdct_spectrum >> 1).wrapping_abs();
    let raw = a.wrapping_sub(b);
    if raw == 0 {
        return 0;
    }
    let x86 = matches!(ARCH, Arch::X86 | Arch::X86_32);
    let d: u32 = raw.wrapping_abs() as u32;
    let clz = d.leading_zeros() as i32;
    let n: u32 = if x86 {
        if clz != 1 {
            d.wrapping_shl((clz - 1) as u32)
        } else {
            d
        }
    } else if clz > 1 {
        d << (clz - 1)
    } else {
        d >> (1 - clz)
    };
    let p = f_pow2(n as FixpDbl);
    if x86 && clz == 0 {
        return p >> 31;
    }
    let e = 2 * (clz - 2);
    let m = fix_min(e, DFRACT_BITS - 1);
    if e < 0 {
        p.wrapping_shl(m.wrapping_neg() as u32)
    } else {
        p >> m
    }
}

/// `FDKaacEnc_QuantizeSpectrum`: quantizes the entire spectrum (straight port, kept as the
/// reference of [`fdk_aac_enc_quantize_spectrum`]).
///
/// input: number of scalefactor bands to be quantized, ...;
/// output: quantized spectrum
#[allow(clippy::too_many_arguments)]
#[doc(hidden)]
pub fn fdk_aac_enc_quantize_spectrum_straight(
    sfb_cnt: i32,
    max_sfb_per_group: i32,
    sfb_per_group: i32,
    sfb_offset: &[i32],
    mdct_spectrum: &[FixpDbl],
    global_gain: i32,
    scalefactors: &[i32],
    quantized_spectrum: &mut [i16],
    d_zone_quant_enable: i32,
) {
    // in FDKaacEnc_quantizeLines quaSpectrum is calculated with:
    //    spec^(3/4) * 2^(-3/16*QSS) * 2^(3/4*scale) + k
    // simplify scaling calculation and reduce QSS before:
    //    spec^(3/4) * 2^(-3/16*(QSS - 4*scale))
    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let i = (sfb_offs + sfb) as usize;
            let scalefactor = scalefactors[i];
            let start = sfb_offset[i] as usize;

            fdk_aac_enc_quantize_lines(
                global_gain.wrapping_sub(scalefactor), // QSS
                sfb_offset[i + 1] - sfb_offset[i],
                &mdct_spectrum[start..],
                &mut quantized_spectrum[start..],
                d_zone_quant_enable,
            );
        }
        sfb_offs += sfb_per_group;
    }
}

/// `FDKaacEnc_calcSfbDist`: calculates distortion of quantized values (straight port, kept as
/// the reference of [`fdk_aac_enc_calc_sfb_dist`]).
///
/// Writes the quantized lines to `quant_spectrum`; if a line exceeds `MAX_QUANT`, returns 0 and
/// leaves the following lines unwritten.
#[doc(hidden)]
pub fn fdk_aac_enc_calc_sfb_dist_straight(
    mdct_spectrum: &[FixpDbl],
    quant_spectrum: &mut [i16],
    no_of_lines: i32,
    gain: i32,
    d_zone_quant_enable: i32,
) -> FixpDbl {
    let mut xfsf: FixpDbl = fl2fxconst_dbl(0.0);

    for i in 0..no_of_lines.max(0) as usize {
        // quantization
        fdk_aac_enc_quantize_lines(
            gain,
            1,
            &mdct_spectrum[i..],
            &mut quant_spectrum[i..],
            d_zone_quant_enable,
        );

        // C: fAbs(quantSpectrum[i]) > MAX_QUANT, evaluated in 16 bits (-32768 passes)
        if quant_spectrum[i].wrapping_abs() as i32 > MAX_QUANT {
            return fl2fxconst_dbl(0.0);
        }
        // inverse quantization
        let inv_quant_spec = fdk_aac_enc_inv_quantize_line(gain, quant_spectrum[i]);

        // dist
        xfsf = xfsf.wrapping_add(dist_line(inv_quant_spec, mdct_spectrum[i]));
    }

    calc_ld_data(xfsf)
}

/// `FDKaacEnc_calcSfbQuantEnergyAndDist`: calculates energy and distortion of quantized values
/// (straight port, kept as the reference of [`fdk_aac_enc_calc_sfb_quant_energy_and_dist`]).
#[doc(hidden)]
pub fn fdk_aac_enc_calc_sfb_quant_energy_and_dist_straight(
    mdct_spectrum: &[FixpDbl],
    quant_spectrum: &[i16],
    no_of_lines: i32,
    gain: i32,
    en: &mut FixpDbl,
    dist: &mut FixpDbl,
) {
    let mut energy: FixpDbl = fl2fxconst_dbl(0.0);
    let mut distortion: FixpDbl = fl2fxconst_dbl(0.0);

    for i in 0..no_of_lines.max(0) as usize {
        // C: fAbs(quantSpectrum[i]) > MAX_QUANT, evaluated in 16 bits (-32768 passes)
        if quant_spectrum[i].wrapping_abs() as i32 > MAX_QUANT {
            *en = fl2fxconst_dbl(0.0);
            *dist = fl2fxconst_dbl(0.0);
            return;
        }

        // inverse quantization
        let inv_quant_spec = fdk_aac_enc_inv_quantize_line(gain, quant_spectrum[i]);

        // energy
        energy = energy.wrapping_add(f_pow2(inv_quant_spec));

        // dist
        distortion = distortion.wrapping_add(dist_line(inv_quant_spec, mdct_spectrum[i]));
    }

    *en = calc_ld_data(energy).wrapping_add(fl2fxconst_dbl(0.03125f32 as f64));
    *dist = calc_ld_data(distortion);
}

// Optimized versions.
//
// The functions below compute the same values as the straight port above for every input
// (including every `gain` and the undefined shifts), but per line without data-dependent
// branches: the reference's tests `accu != 0`, `quantSpectrum != 0` and `diff != 0` become
// selects, the gain-dependent constants are computed once per call, and the distortion is
// normalized with one shift pair. The rare inputs whose straight code takes another path (a
// quantized value of -32768, a distortion difference of 2^31) call the straight helpers.
// `quantize_fast` tests these functions against the straight ones on random and extreme inputs.

/// The gain-dependent constants of `FDKaacEnc_quantizeLines()` and
/// `FDKaacEnc_invQuantizeLines()` for one gain.
#[derive(Clone, Copy)]
struct GainConsts {
    /// `quantizer << 16` (`fMultDiv2(FIXP_DBL, FIXP_SGL)` multiplies by it and keeps the high
    /// word)
    quantizer: i64,
    /// `quantizershift`
    quantizershift: i32,
    /// the rounding offset `k`
    k: FixpDbl,
    /// `iquantizermod`
    iquantizermod: usize,
    /// `iquantizershift`
    iquantizershift: i32,
}

impl GainConsts {
    #[inline(always)]
    fn new(gain: i32, d_zone_quant_enable: i32) -> Self {
        const K_SHIFT: i32 = 16;
        GainConsts {
            quantizer: (FDK_AAC_ENC_QUANT_TABLE_Q[(gain.wrapping_neg() & 3) as usize] as i64) << 16,
            quantizershift: (gain.wrapping_neg() >> 2) + 1,
            k: if d_zone_quant_enable != 0 {
                fl2fxconst_dbl(0.23f32 as f64) >> K_SHIFT
            } else {
                fl2fxconst_dbl((-0.0946f32 + 0.5f32) as f64) >> K_SHIFT
            },
            iquantizermod: (gain & 3) as usize,
            iquantizershift: gain >> 2,
        }
    }
}

/// One line of `FDKaacEnc_quantizeLines()`, without branches.
///
/// `accu = fMultDiv2(x, quantizer)` is below 2^30 in magnitude, so its absolute value never
/// wraps and normalizes with a shift of 1 to 31 (31 for `accu == 0`, whose result is replaced by
/// 0 as in the reference).
#[inline(always)]
fn quantize_line_fast(x: FixpDbl, c: &GainConsts) -> i16 {
    let accu = ((x as i64 * c.quantizer) >> 32) as i32;
    let a = accu.wrapping_abs();
    let accu_shift = a.leading_zeros() as i32 - 1;
    let an = a << accu_shift;
    let tab_index = ((an >> (DFRACT_BITS - 2 - MANT_DIGITS)) & (MANT_SIZE - 1)) as usize;
    let total_shift = c.quantizershift.wrapping_sub(accu_shift).wrapping_add(1);
    let m = FDK_AAC_ENC_M_TAB_3_4[tab_index] as i32
        * FDK_AAC_ENC_QUANT_TABLE_E[(total_shift & 3) as usize] as i32;
    let total_shift = fix_min(3i32.wrapping_mul(total_shift >> 2).wrapping_neg(), 19) + 12;
    fdk_aac_rs_fdk::fdk_assert!(accu == 0 || total_shift >= 0);
    let q = (c.k.wrapping_add(m.wrapping_shr(total_shift as u32)) >> (DFRACT_BITS - 1 - 16)) as i16;
    let q = if accu < 0 { q.wrapping_neg() } else { q };
    if accu == 0 {
        0
    } else {
        q
    }
}

/// One line of `FDKaacEnc_invQuantizeLines()` for `|q| <= MAX_QUANT`, without branches.
#[inline(always)]
fn inv_quantize_line_fast(q: i16, c: &GainConsts) -> FixpDbl {
    let qa = (q as i32).abs();
    debug_assert!(qa <= MAX_QUANT);
    let ex = qa.leading_zeros() as i32 - 1;
    let an = qa << ex;
    // 0 for q == 0 (the result is replaced by 0), else 1..=13
    let spec_exp = ((DFRACT_BITS - 1) - ex) as usize;
    fdk_aac_rs_fdk::fdk_assert!(q == 0 || spec_exp < 14);
    let tab_index = ((an >> (DFRACT_BITS - 2 - MANT_DIGITS)) & (MANT_SIZE - 1)) as usize;
    let s = FDK_AAC_ENC_M_TAB_4_3_ELC[tab_index];
    let t = FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC[c.iquantizermod][spec_exp.min(13)];
    let accu = f_mult(s, t);
    let st = c
        .iquantizershift
        .wrapping_add(FDK_AAC_ENC_SPEC_EXP_TABLE_COMB[c.iquantizermod][spec_exp.min(13)] as i32);
    let accu = if st >= 2 {
        accu.wrapping_shl(st.wrapping_sub(1) as u32)
    } else {
        accu.wrapping_shr(1i32.wrapping_sub(st) as u32)
    };
    let accu = if q < 0 { accu.wrapping_neg() } else { accu };
    if q == 0 {
        0
    } else {
        accu
    }
}

/// [`dist_line`] without branches.
///
/// `d = |(|invQuantSpec|) - |mdctSpectrum >> 1||` is below 2^31 except for `invQuantSpec ==
/// MINVAL_DBL` with `mdctSpectrum >> 1 == 0`, which takes the straight code. Below 2^31, all
/// variants normalize `d` to `(d << clz) >> 1` (0 for `d == 0`, where the reference skips the
/// line) and scale `fPow2()` of it by `2 * (clz - 2)`: left by 2 for `clz == 1`, right by at
/// most 31 otherwise, i.e. `(p << 4) >> min(2 * clz, 35)` in 64 bits (`p < 2^31`).
#[inline(always)]
fn dist_line_fast(inv_quant_spec: FixpDbl, mdct_spectrum: FixpDbl) -> FixpDbl {
    let a = inv_quant_spec.wrapping_abs();
    let b = (mdct_spectrum >> 1).wrapping_abs();
    let d = a.wrapping_sub(b).wrapping_abs() as u32;
    if d >= 0x8000_0000 {
        return dist_line(inv_quant_spec, mdct_spectrum);
    }
    let clz = d.leading_zeros();
    let n = (d.wrapping_shl(clz) >> 1) as FixpDbl;
    let p = f_pow2(n) as u32 as u64;
    ((p << 4) >> (2 * clz).min(35)) as u32 as FixpDbl
}

/// `FDKaacEnc_QuantizeSpectrum`: quantizes the entire spectrum.
///
/// input: number of scalefactor bands to be quantized, ...;
/// output: quantized spectrum
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_quantize_spectrum(
    sfb_cnt: i32,
    max_sfb_per_group: i32,
    sfb_per_group: i32,
    sfb_offset: &[i32],
    mdct_spectrum: &[FixpDbl],
    global_gain: i32,
    scalefactors: &[i32],
    quantized_spectrum: &mut [i16],
    d_zone_quant_enable: i32,
) {
    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let i = (sfb_offs + sfb) as usize;
            let c = GainConsts::new(global_gain.wrapping_sub(scalefactors[i]), d_zone_quant_enable);
            let start = sfb_offset[i] as usize;
            let n = (sfb_offset[i + 1] - sfb_offset[i]).max(0) as usize;
            let spec = &mdct_spectrum[start..start + n];
            let quant = &mut quantized_spectrum[start..start + n];
            quantize_lines_vec(spec, quant, &c);
        }
        sfb_offs += sfb_per_group;
    }
}

/// `FDKaacEnc_calcSfbDist()` up to the final `CalcLdData()` (scalar version): the sum of the
/// distortions, or `None` for the early return (a line above `MAX_QUANT`).
fn calc_sfb_dist_sum_scalar(
    spec: &[FixpDbl],
    quant: &mut [i16],
    gain: i32,
    c: &GainConsts,
) -> Option<FixpDbl> {
    let mut xfsf: FixpDbl = fl2fxconst_dbl(0.0);
    for (q_out, &x) in quant.iter_mut().zip(spec) {
        let q = quantize_line_fast(x, c);
        *q_out = q;
        let inv_quant_spec = if (q as i32).abs() > MAX_QUANT {
            // C: fAbs(quantSpectrum[i]) > MAX_QUANT, evaluated in 16 bits (-32768 passes)
            if q != i16::MIN {
                return None;
            }
            fdk_aac_enc_inv_quantize_line(gain, q)
        } else {
            inv_quantize_line_fast(q, c)
        };
        xfsf = xfsf.wrapping_add(dist_line_fast(inv_quant_spec, x));
    }
    Some(xfsf)
}

/// `FDKaacEnc_calcSfbQuantEnergyAndDist()` up to the final `CalcLdData()` (scalar version): the
/// sums of the energies and the distortions, or `None` for the early return (a line above
/// `MAX_QUANT`).
fn calc_sfb_quant_energy_and_dist_sums_scalar(
    spec: &[FixpDbl],
    quant: &[i16],
    gain: i32,
    c: &GainConsts,
) -> Option<(FixpDbl, FixpDbl)> {
    let mut energy: FixpDbl = fl2fxconst_dbl(0.0);
    let mut distortion: FixpDbl = fl2fxconst_dbl(0.0);
    for (&q, &x) in quant.iter().zip(spec) {
        let inv_quant_spec = if (q as i32).abs() > MAX_QUANT {
            // C: fAbs(quantSpectrum[i]) > MAX_QUANT, evaluated in 16 bits (-32768 passes)
            if q != i16::MIN {
                return None;
            }
            fdk_aac_enc_inv_quantize_line(gain, q)
        } else {
            inv_quantize_line_fast(q, c)
        };
        energy = energy.wrapping_add(f_pow2(inv_quant_spec));
        distortion = distortion.wrapping_add(dist_line_fast(inv_quant_spec, x));
    }
    Some((energy, distortion))
}

/// `FDKaacEnc_quantizeLines()` of a band (scalar version).
fn quantize_lines_scalar(spec: &[FixpDbl], quant: &mut [i16], c: &GainConsts) {
    for (q, &x) in quant.iter_mut().zip(spec) {
        *q = quantize_line_fast(x, c);
    }
}

#[cfg(target_arch = "aarch64")]
use neon::{
    calc_sfb_dist_sum as calc_sfb_dist_sum_vec,
    calc_sfb_quant_energy_and_dist_sums as calc_sfb_quant_energy_and_dist_sums_vec,
    quantize_lines as quantize_lines_vec,
};
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
use {
    calc_sfb_dist_sum_scalar as calc_sfb_dist_sum_vec,
    calc_sfb_quant_energy_and_dist_sums_scalar as calc_sfb_quant_energy_and_dist_sums_vec,
    quantize_lines_scalar as quantize_lines_vec,
};

/// The AVX2 kernels (`x86.rs`) if [`crate::simd::avx2`], else the scalar ones.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn calc_sfb_dist_sum_vec(
    spec: &[FixpDbl],
    quant: &mut [i16],
    gain: i32,
    c: &GainConsts,
) -> Option<FixpDbl> {
    if crate::simd::avx2() {
        // SAFETY: the CPU supports AVX2.
        unsafe { x86::calc_sfb_dist_sum(spec, quant, gain, c) }
    } else {
        calc_sfb_dist_sum_scalar(spec, quant, gain, c)
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn calc_sfb_quant_energy_and_dist_sums_vec(
    spec: &[FixpDbl],
    quant: &[i16],
    gain: i32,
    c: &GainConsts,
) -> Option<(FixpDbl, FixpDbl)> {
    if crate::simd::avx2() {
        // SAFETY: the CPU supports AVX2.
        unsafe { x86::calc_sfb_quant_energy_and_dist_sums(spec, quant, gain, c) }
    } else {
        calc_sfb_quant_energy_and_dist_sums_scalar(spec, quant, gain, c)
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn quantize_lines_vec(spec: &[FixpDbl], quant: &mut [i16], c: &GainConsts) {
    if crate::simd::avx2() {
        // SAFETY: the CPU supports AVX2.
        unsafe { x86::quantize_lines(spec, quant, c) }
    } else {
        quantize_lines_scalar(spec, quant, c)
    }
}

/// `FDKaacEnc_calcSfbDist`: calculates distortion of quantized values.
///
/// Writes the quantized lines to `quant_spectrum`; if a line exceeds `MAX_QUANT`, returns 0 and
/// leaves the following lines unwritten.
pub fn fdk_aac_enc_calc_sfb_dist(
    mdct_spectrum: &[FixpDbl],
    quant_spectrum: &mut [i16],
    no_of_lines: i32,
    gain: i32,
    d_zone_quant_enable: i32,
) -> FixpDbl {
    let n = no_of_lines.max(0) as usize;
    let c = GainConsts::new(gain, d_zone_quant_enable);
    match calc_sfb_dist_sum_vec(&mdct_spectrum[..n], &mut quant_spectrum[..n], gain, &c) {
        Some(xfsf) => calc_ld_data(xfsf),
        None => fl2fxconst_dbl(0.0),
    }
}

/// `FDKaacEnc_calcSfbQuantEnergyAndDist`: calculates energy and distortion of quantized values.
pub fn fdk_aac_enc_calc_sfb_quant_energy_and_dist(
    mdct_spectrum: &[FixpDbl],
    quant_spectrum: &[i16],
    no_of_lines: i32,
    gain: i32,
    en: &mut FixpDbl,
    dist: &mut FixpDbl,
) {
    let n = no_of_lines.max(0) as usize;
    let c = GainConsts::new(gain, 0);
    match calc_sfb_quant_energy_and_dist_sums_vec(
        &mdct_spectrum[..n],
        &quant_spectrum[..n],
        gain,
        &c,
    ) {
        Some((energy, distortion)) => {
            *en = calc_ld_data(energy).wrapping_add(fl2fxconst_dbl(0.03125f32 as f64));
            *dist = calc_ld_data(distortion);
        }
        None => {
            *en = fl2fxconst_dbl(0.0);
            *dist = fl2fxconst_dbl(0.0);
        }
    }
}

/// The per-line kernels, for the tests of the optimized functions against the straight port.
#[doc(hidden)]
pub mod testing {
    use super::*;

    /// `FDKaacEnc_quantizeLines()` of one line: straight port and optimized version.
    pub fn quantize_line(gain: i32, x: FixpDbl, d_zone_quant_enable: i32) -> (i16, i16) {
        let mut q = [0x5555i16];
        fdk_aac_enc_quantize_lines(gain, 1, &[x], &mut q, d_zone_quant_enable);
        (q[0], quantize_line_fast(x, &GainConsts::new(gain, d_zone_quant_enable)))
    }

    /// `FDKaacEnc_invQuantizeLines()` of one line (`|q| <= MAX_QUANT`): straight port and
    /// optimized version.
    pub fn inv_quantize_line(gain: i32, q: i16) -> (FixpDbl, FixpDbl) {
        let c = GainConsts::new(gain, 0);
        (fdk_aac_enc_inv_quantize_line(gain, q), inv_quantize_line_fast(q, &c))
    }

    /// `FDKaacEnc_quantizeLines()` of a band with the scalar kernel (on AArch64 the public
    /// functions use the NEON kernels, on x86 with AVX2 the AVX2 kernels).
    pub fn quantize_lines_scalar(gain: i32, spec: &[FixpDbl], quant: &mut [i16], dz: i32) {
        super::quantize_lines_scalar(spec, quant, &GainConsts::new(gain, dz));
    }

    /// `FDKaacEnc_calcSfbDist()` with the scalar kernels (on AArch64 the public function uses
    /// the NEON kernels, on x86 with AVX2 the AVX2 kernels).
    pub fn calc_sfb_dist_scalar(
        mdct_spectrum: &[FixpDbl],
        quant_spectrum: &mut [i16],
        no_of_lines: i32,
        gain: i32,
        d_zone_quant_enable: i32,
    ) -> FixpDbl {
        let n = no_of_lines.max(0) as usize;
        let c = GainConsts::new(gain, d_zone_quant_enable);
        match calc_sfb_dist_sum_scalar(&mdct_spectrum[..n], &mut quant_spectrum[..n], gain, &c) {
            Some(xfsf) => calc_ld_data(xfsf),
            None => fl2fxconst_dbl(0.0),
        }
    }

    /// `FDKaacEnc_calcSfbQuantEnergyAndDist()` with the scalar kernels: `(en, dist)`.
    pub fn calc_sfb_quant_energy_and_dist_scalar(
        mdct_spectrum: &[FixpDbl],
        quant_spectrum: &[i16],
        no_of_lines: i32,
        gain: i32,
    ) -> (FixpDbl, FixpDbl) {
        let n = no_of_lines.max(0) as usize;
        let c = GainConsts::new(gain, 0);
        match calc_sfb_quant_energy_and_dist_sums_scalar(
            &mdct_spectrum[..n],
            &quant_spectrum[..n],
            gain,
            &c,
        ) {
            Some((energy, distortion)) => (
                calc_ld_data(energy).wrapping_add(fl2fxconst_dbl(0.03125f32 as f64)),
                calc_ld_data(distortion),
            ),
            None => (0, 0),
        }
    }

    /// The distortion of one line: straight port and optimized version.
    pub fn dist_line(inv_quant_spec: FixpDbl, mdct_spectrum: FixpDbl) -> (FixpDbl, FixpDbl) {
        let s = super::dist_line(inv_quant_spec, mdct_spectrum);
        (s, dist_line_fast(inv_quant_spec, mdct_spectrum))
    }
}

// The optimized functions against the straight port, without the C++ reference: these also run
// with `FDK_AAC_RS_ARCH=x86|x86_32|generic` on an AArch64 host, where the NEON kernels follow the
// `fMult` of those variants (the cref tests `quantize_fast` need the matching reference).
#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 32) as u32
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() as u64 % (hi as i64 - lo as i64 + 1) as u64) as i32
        }
        fn spec(&mut self) -> FixpDbl {
            match self.range(0, 7) {
                0 => [0, i32::MIN, i32::MAX, 1, -1, i32::MIN + 1][self.range(0, 5) as usize],
                1..=4 => (self.next() as i32) >> self.range(0, 31),
                _ => self.next() as i32,
            }
        }
        fn gain(&mut self) -> i32 {
            match self.range(0, 7) {
                0 => self.next() as i32,
                1 => self.range(-400, 400),
                _ => self.range(-120, 120),
            }
        }
    }

    #[test]
    fn vector_kernels_vs_straight() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for k in 0..300_000 {
            // up to 40 lines, every 16th band up to 160 (the two-pass kernels take up to 128)
            let n = if k % 16 == 0 { rng.range(0, 160) } else { rng.range(0, 40) };
            let spec: Vec<FixpDbl> = (0..n).map(|_| rng.spec()).collect();
            let q0: Vec<i16> = (0..n).map(|_| rng.next() as i16).collect();
            let (g, dz) = (rng.gain(), rng.range(0, 1));
            let (mut qs, mut qf) = (q0.clone(), q0.clone());
            let s = fdk_aac_enc_calc_sfb_dist_straight(&spec, &mut qs, n, g, dz);
            let f = fdk_aac_enc_calc_sfb_dist(&spec, &mut qf, n, g, dz);
            assert_eq!((s, &qs), (f, &qf), "calcSfbDist n {n} gain {g} dz {dz}");

            let q: Vec<i16> = (0..n)
                .map(|_| match rng.range(0, 40) {
                    0 => rng.next() as i16,
                    1 => i16::MIN,
                    2..=9 => 0,
                    _ => (rng.range(-MAX_QUANT, MAX_QUANT) >> rng.range(0, 13)) as i16,
                })
                .collect();
            let (mut es, mut ds, mut ef, mut df) = (1, 2, 1, 2);
            fdk_aac_enc_calc_sfb_quant_energy_and_dist_straight(&spec, &q, n, g, &mut es, &mut ds);
            fdk_aac_enc_calc_sfb_quant_energy_and_dist(&spec, &q, n, g, &mut ef, &mut df);
            assert_eq!((es, ds), (ef, df), "calcSfbQuantEnergyAndDist n {n} gain {g}");

            let offsets = [0, n];
            let (mut qs, mut qf) = (q0.clone(), q0.clone());
            fdk_aac_enc_quantize_spectrum_straight(1, 1, 1, &offsets, &spec, g, &[0], &mut qs, dz);
            fdk_aac_enc_quantize_spectrum(1, 1, 1, &offsets, &spec, g, &[0], &mut qf, dz);
            assert_eq!(qs, qf, "QuantizeSpectrum n {n} gain {g} dz {dz}");
        }
    }
}

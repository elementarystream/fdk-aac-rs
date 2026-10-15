// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/quantize.cpp (optimized; no counterpart in the reference).

//! AArch64 NEON versions of `FDKaacEnc_calcSfbDist()` and `FDKaacEnc_calcSfbQuantEnergyAndDist()`
//! (four lines at a time).
//!
//! Every lane computes exactly what the scalar kernels of the parent module compute (which equal
//! the straight port):
//!
//! - `fMultDiv2(x, quantizer)` = `(x * (quantizer << 16)) >> 32` is `sqdmulh(x, quantizer << 15)`:
//!   the doubling product never saturates (`quantizer << 15 < 2^30`);
//! - `fMult(s, t)` of the inverse quantization (both positive) is `sqdmulh(s, t)` = `(s * t) >> 31`
//!   on `aarch64`, and with bit 0 cleared `((s * t) >> 32) << 1` on the other variants;
//! - `fPow2(n)` = `((n * n) >> 32) << 1` is `sqdmulh(n, n)` with bit 0 cleared (`n < 2^31`);
//! - shifts by a register amount modulo 32 take the amount `& 31` (`sshl`/`ushl` with a negative
//!   amount shift right; the reference's amounts of 32 or more never reach `sshl`);
//! - the table lookups by the line's own index are lane loads; the 4- and 14-entry rows are `tbl`
//!   lookups (out-of-range indices give 0 only in lanes whose result is discarded); the inverse
//!   quantization of a group whose values are all below 16 in magnitude reads the 15 entries of
//!   `FDKaacEnc_mTab_4_3Elc` it can use from a 16-entry copy (`tbl`).
//!
//! A group of four lines with a quantized value whose 32-bit magnitude exceeds `MAX_QUANT`
//! (including -32768) or a distortion difference of 2^31 is recomputed by the scalar code, which
//! also handles the early return; the scalar code also takes the last `n % 4` lines.

use super::{
    dist_line_fast, fdk_aac_enc_inv_quantize_line, inv_quantize_line_fast, quantize_line_fast,
    GainConsts, MAX_QUANT,
};
use crate::aac_enc_rom::{
    FDK_AAC_ENC_M_TAB_3_4, FDK_AAC_ENC_M_TAB_4_3_ELC, FDK_AAC_ENC_QUANT_TABLE_E,
    FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC, FDK_AAC_ENC_SPEC_EXP_TABLE_COMB,
};
use core::arch::aarch64::*;
use fdk_aac_rs_fdk::common_fix::{f_pow2, FixpDbl};
use fdk_aac_rs_fdk::{Arch, ARCH};

/// The per-call constants of the vector kernels.
struct VecConsts {
    /// `quantizer << 15`
    quantizer: i32,
    /// `quantizershift + 1`
    quantizershift1: int32x4_t,
    k: int32x4_t,
    /// `FDKaacEnc_quantTableE` as 16 bytes (4 little-endian words)
    quant_table_e: uint8x16_t,
    /// row `iquantizermod` of `FDKaacEnc_specExpMantTableCombElc` as 64 bytes (14 words, 0)
    mant_row: uint8x16x4_t,
    /// row `iquantizermod` of `FDKaacEnc_specExpTableComb` (14 bytes, 0)
    exp_row: uint8x16_t,
    iquantizershift: int32x4_t,
    /// [`M_TAB_4_3_SMALL_BYTES`]
    m43_small: uint8x16x4_t,
}

/// `FDKaacEnc_mTab_4_3Elc[]` of the normalized magnitudes `1..=15` (index 0: 0), the entry the
/// inverse quantization reads for `|q| < 16`, as 64 bytes (16 little-endian words).
static M_TAB_4_3_SMALL_BYTES: [u8; 64] = {
    let mut b = [0u8; 64];
    let mut q = 1;
    while q < 16 {
        let an = (q as i32) << ((q as i32).leading_zeros() - 1);
        let w = FDK_AAC_ENC_M_TAB_4_3_ELC[((an >> 21) & 511) as usize].to_le_bytes();
        let mut j = 0;
        while j < 4 {
            b[4 * q + j] = w[j];
            j += 1;
        }
        q += 1;
    }
    b
};

/// `FDKaacEnc_quantTableE` as 16 bytes (4 little-endian words).
static QUANT_TABLE_E_BYTES: [u8; 16] = {
    let mut b = [0u8; 16];
    let mut i = 0;
    while i < 4 {
        let w = (FDK_AAC_ENC_QUANT_TABLE_E[i] as i32).to_le_bytes();
        let mut j = 0;
        while j < 4 {
            b[4 * i + j] = w[j];
            j += 1;
        }
        i += 1;
    }
    b
};

/// The rows of `FDKaacEnc_specExpMantTableCombElc` as 64 bytes each (14 little-endian words, 0).
static MANT_ROW_BYTES: [[u8; 64]; 4] = {
    let mut b = [[0u8; 64]; 4];
    let mut r = 0;
    while r < 4 {
        let mut i = 0;
        while i < 14 {
            let w = FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC[r][i].to_le_bytes();
            let mut j = 0;
            while j < 4 {
                b[r][4 * i + j] = w[j];
                j += 1;
            }
            i += 1;
        }
        r += 1;
    }
    b
};

/// The rows of `FDKaacEnc_specExpTableComb` as 16 bytes each (14 bytes, 0).
static EXP_ROW_BYTES: [[u8; 16]; 4] = {
    let mut b = [[0u8; 16]; 4];
    let mut r = 0;
    while r < 4 {
        let mut i = 0;
        while i < 14 {
            b[r][i] = FDK_AAC_ENC_SPEC_EXP_TABLE_COMB[r][i];
            i += 1;
        }
        r += 1;
    }
    b
};

impl VecConsts {
    #[inline(always)]
    fn new(c: &GainConsts) -> Self {
        // SAFETY: NEON is part of the AArch64 baseline; the loads read arrays of the loaded size.
        unsafe {
            VecConsts {
                quantizer: (c.quantizer >> 1) as i32,
                quantizershift1: vdupq_n_s32(c.quantizershift.wrapping_add(1)),
                k: vdupq_n_s32(c.k),
                quant_table_e: vld1q_u8(QUANT_TABLE_E_BYTES.as_ptr()),
                mant_row: vld1q_u8_x4(MANT_ROW_BYTES[c.iquantizermod].as_ptr()),
                exp_row: vld1q_u8(EXP_ROW_BYTES[c.iquantizermod].as_ptr()),
                iquantizershift: vdupq_n_s32(c.iquantizershift),
                m43_small: vld1q_u8_x4(M_TAB_4_3_SMALL_BYTES.as_ptr()),
            }
        }
    }
}

/// Byte indices for a `tbl` lookup of the 32-bit words `w` (`4 * w + [0, 1, 2, 3]`).
#[inline(always)]
unsafe fn word_bytes(w: uint32x4_t) -> uint8x16_t {
    vreinterpretq_u8_u32(vorrq_u32(vmulq_n_u32(w, 0x0404_0404), vdupq_n_u32(0x0302_0100)))
}

/// `FDKaacEnc_quantizeLines()` of four lines, sign-extended to 32 bits.
#[inline(always)]
unsafe fn quantize4(x: int32x4_t, v: &VecConsts) -> int32x4_t {
    let accu = vqdmulhq_n_s32(x, v.quantizer);
    let a = vabsq_s32(accu);
    let accu_shift = vsubq_s32(vclzq_s32(a), vdupq_n_s32(1));
    let an = vreinterpretq_u32_s32(vshlq_s32(a, accu_shift));
    let idx = vandq_u32(vshrq_n_u32::<21>(an), vdupq_n_u32(511));
    let m34 = gather_i16(&FDK_AAC_ENC_M_TAB_3_4, idx);
    let total_shift = vsubq_s32(v.quantizershift1, accu_shift);
    let e_idx = word_bytes(vandq_u32(vreinterpretq_u32_s32(total_shift), vdupq_n_u32(3)));
    let qe = vreinterpretq_s32_u8(vqtbl1q_u8(v.quant_table_e, e_idx));
    let m = vmulq_s32(m34, qe);
    // fixMin(-3 * (totalShift >> 2), 19) + 12, as a right shift modulo 32
    let sh = vaddq_s32(
        vminq_s32(vmulq_n_s32(vshrq_n_s32::<2>(total_shift), -3), vdupq_n_s32(19)),
        vdupq_n_s32(12),
    );
    let m = vshlq_s32(m, vnegq_s32(vandq_s32(sh, vdupq_n_s32(31))));
    let q = vmovn_s32(vshrq_n_s32::<15>(vaddq_s32(v.k, m)));
    let q = vbsl_s16(vmovn_u32(vcltzq_s32(accu)), vneg_s16(q), q);
    let q = vbic_s16(q, vreinterpret_s16_u16(vmovn_u32(vceqzq_s32(accu))));
    vmovl_s16(q)
}

/// `FDKaacEnc_invQuantizeLines()` of four lines; lanes with `|q| > MAX_QUANT` are undefined.
#[inline(always)]
unsafe fn inv_quantize4(q: int32x4_t, v: &VecConsts) -> int32x4_t {
    inv_quantize4_with(q, v, vmaxvq_s32(vabsq_s32(q)) < 16)
}

/// [`inv_quantize4`] with the test `max |q| < 16` given (`small` must imply it).
#[inline(always)]
unsafe fn inv_quantize4_with(q: int32x4_t, v: &VecConsts, small: bool) -> int32x4_t {
    let qa = vabsq_s32(q);
    let clz = vclzq_s32(qa);
    let an = vreinterpretq_u32_s32(vshlq_s32(qa, vsubq_s32(clz, vdupq_n_s32(1))));
    // 0 for q == 0 (the result is replaced by 0), else 1..=13
    let spec_exp = vreinterpretq_u32_s32(vsubq_s32(vdupq_n_s32(32), clz));
    let s = if small {
        // all |q| < 16 (the common case): the table entries of 1..=15 by tbl
        vreinterpretq_s32_u8(vqtbl4q_u8(v.m43_small, word_bytes(vreinterpretq_u32_s32(qa))))
    } else {
        let idx = vandq_u32(vshrq_n_u32::<21>(an), vdupq_n_u32(511));
        gather_i32(&FDK_AAC_ENC_M_TAB_4_3_ELC, idx)
    };
    let t = vreinterpretq_s32_u8(vqtbl4q_u8(v.mant_row, word_bytes(spec_exp)));
    let accu = vqdmulhq_s32(s, t);
    let accu = match ARCH {
        Arch::Aarch64 => accu,
        _ => vandq_s32(accu, vdupq_n_s32(!1)),
    };
    let e = vqtbl1q_u8(v.exp_row, vreinterpretq_u8_u32(vorrq_u32(spec_exp, vdupq_n_u32(!0xFF))));
    let st = vaddq_s32(v.iquantizershift, vreinterpretq_s32_u8(e));
    // st >= 2: left by (st - 1) & 31, else right by (1 - st) & 31
    let one = vdupq_n_s32(1);
    let mask31 = vdupq_n_s32(31);
    let left = vandq_s32(vsubq_s32(st, one), mask31);
    let right = vnegq_s32(vandq_s32(vsubq_s32(one, st), mask31));
    let accu = vshlq_s32(accu, vbslq_s32(vcgtq_s32(st, one), left, right));
    let accu = vbslq_s32(vcltzq_s32(q), vnegq_s32(accu), accu);
    vbicq_s32(accu, vreinterpretq_s32_u32(vceqzq_s32(q)))
}

/// The distortion of four lines (see `dist_line_fast()`) and a mask of the lanes whose
/// difference is 2^31 (which need the straight code).
#[inline(always)]
unsafe fn dist4(inv: int32x4_t, x: int32x4_t) -> (uint32x4_t, uint32x4_t) {
    let a = vabsq_s32(inv);
    let b = vabsq_s32(vshrq_n_s32::<1>(x));
    let d = vabsq_s32(vsubq_s32(a, b));
    let bad = vcltzq_s32(d);
    let d = vreinterpretq_u32_s32(d);
    let clz = vreinterpretq_s32_u32(vclzq_u32(d));
    let n = vreinterpretq_s32_u32(vshrq_n_u32::<1>(vshlq_u32(d, clz)));
    let p = vreinterpretq_u32_s32(vandq_s32(vqdmulhq_s32(n, n), vdupq_n_s32(!1)));
    let amount = vsubq_s32(vdupq_n_s32(4), vminq_s32(vshlq_n_s32::<1>(clz), vdupq_n_s32(35)));
    (vshlq_u32(p, amount), bad)
}

#[inline(always)]
unsafe fn gather_i16(tab: &[i16; 512], idx: uint32x4_t) -> int32x4_t {
    let p = tab.as_ptr();
    let mut v = vdup_n_s16(0);
    v = vld1_lane_s16::<0>(p.add(vgetq_lane_u32::<0>(idx) as usize), v);
    v = vld1_lane_s16::<1>(p.add(vgetq_lane_u32::<1>(idx) as usize), v);
    v = vld1_lane_s16::<2>(p.add(vgetq_lane_u32::<2>(idx) as usize), v);
    v = vld1_lane_s16::<3>(p.add(vgetq_lane_u32::<3>(idx) as usize), v);
    vmovl_s16(v)
}

#[inline(always)]
unsafe fn gather_i32(tab: &[i32; 512], idx: uint32x4_t) -> int32x4_t {
    let p = tab.as_ptr();
    let mut v = vdupq_n_s32(0);
    v = vld1q_lane_s32::<0>(p.add(vgetq_lane_u32::<0>(idx) as usize), v);
    v = vld1q_lane_s32::<1>(p.add(vgetq_lane_u32::<1>(idx) as usize), v);
    v = vld1q_lane_s32::<2>(p.add(vgetq_lane_u32::<2>(idx) as usize), v);
    v = vld1q_lane_s32::<3>(p.add(vgetq_lane_u32::<3>(idx) as usize), v);
    v
}

/// Whether `|q| > MAX_QUANT` (in 32 bits) in a lane.
#[inline(always)]
unsafe fn over_max_quant(q: int32x4_t) -> uint32x4_t {
    vcgtq_s32(vabsq_s32(q), vdupq_n_s32(MAX_QUANT))
}

/// The inverse quantization and distortion of one line (`|q| <= MAX_QUANT` or -32768), as the
/// scalar functions compute it.
#[inline(always)]
fn inv_dist_line(q: i16, x: FixpDbl, gain: i32, c: &GainConsts) -> (FixpDbl, FixpDbl) {
    let inv = if q == i16::MIN {
        fdk_aac_enc_inv_quantize_line(gain, q)
    } else {
        inv_quantize_line_fast(q, c)
    };
    (inv, dist_line_fast(inv, x))
}

/// The lines of a band [`calc_sfb_dist_sum`] processes in two passes (wider bands take
/// [`calc_sfb_dist_sum_groups`]).
const TWO_PASS_MAX: usize = 128;

/// `FDKaacEnc_calcSfbDist()` up to the final `CalcLdData()`: the sum of the distortions, or
/// `None` for the early return (a line above `MAX_QUANT`).
///
/// Two passes over the groups of 4 lines: the quantization into a local buffer,
/// with the band's maximum magnitude, then the inverse quantization and distortion, with the
/// table lookup of `|q| < 16` for the whole band if its maximum allows. Only if no line exceeds
/// `MAX_QUANT` (in 32 bits, which includes -32768) and no distortion difference is 2^31 are the
/// quantized lines stored and the sum returned (each lane computes what the scalar kernels
/// compute, see the module documentation); otherwise [`calc_sfb_dist_sum_groups`] computes the
/// band from the start (nothing has been written), with the early return of the straight port.
pub(super) fn calc_sfb_dist_sum(
    spec: &[FixpDbl],
    quant: &mut [i16],
    gain: i32,
    c: &GainConsts,
) -> Option<FixpDbl> {
    debug_assert_eq!(spec.len(), quant.len());
    let n = spec.len();
    let g = n / 4;
    if g == 0 || n > TWO_PASS_MAX {
        return calc_sfb_dist_sum_groups(spec, quant, gain, c);
    }
    let v = VecConsts::new(c);
    let mut qb = [0i16; TWO_PASS_MAX];
    // SAFETY: NEON is part of the AArch64 baseline; the loads and stores access the groups of 4
    // lines of `spec`, `qb` and `quant` (4 * g <= n <= TWO_PASS_MAX); the gathers read indices
    // below 512.
    unsafe {
        let mut qmax = vdupq_n_s32(0);
        for (k, sg) in spec.chunks_exact(4).enumerate() {
            let q = quantize4(vld1q_s32(sg.as_ptr()), &v);
            qmax = vmaxq_s32(qmax, vabsq_s32(q));
            vst1_s16(qb.as_mut_ptr().add(4 * k), vmovn_s32(q));
        }
        let qmax = vmaxvq_s32(qmax);
        if qmax > MAX_QUANT {
            return calc_sfb_dist_sum_groups(spec, quant, gain, c);
        }
        let small = qmax < 16;
        let mut sum = vdupq_n_u32(0);
        let mut bad = vdupq_n_u32(0);
        for (k, sg) in spec.chunks_exact(4).enumerate() {
            let x = vld1q_s32(sg.as_ptr());
            let q = vmovl_s16(vld1_s16(qb.as_ptr().add(4 * k)));
            let inv = inv_quantize4_with(q, &v, small);
            let (d, bad_d) = dist4(inv, x);
            bad = vorrq_u32(bad, bad_d);
            sum = vaddq_u32(sum, d);
        }
        if vmaxvq_u32(bad) != 0 {
            return calc_sfb_dist_sum_groups(spec, quant, gain, c);
        }
        // quant[..4 g] = qb[..4 g] in 8- and 4-line pieces (without a call of memcpy)
        let (dst, src) = (quant.as_mut_ptr(), qb.as_ptr());
        let mut i = 0;
        while i + 8 <= 4 * g {
            vst1q_s16(dst.add(i), vld1q_s16(src.add(i)));
            i += 8;
        }
        if i < 4 * g {
            vst1_s16(dst.add(i), vld1_s16(src.add(i)));
        }
        let mut xfsf = vaddvq_u32(sum) as FixpDbl;
        let tail = 4 * g;
        for (q_out, &x) in quant[tail..].iter_mut().zip(&spec[tail..]) {
            let q = quantize_line_fast(x, c);
            *q_out = q;
            if (q as i32).abs() > MAX_QUANT && q != i16::MIN {
                return None;
            }
            xfsf = xfsf.wrapping_add(inv_dist_line(q, x, gain, c).1);
        }
        Some(xfsf)
    }
}

/// [`calc_sfb_dist_sum`] group by group (its fallback): every group of 4 lines that has no line
/// above `MAX_QUANT` and no distortion difference of 2^31 is stored and added at once, the
/// others take the scalar code with the early return.
pub(super) fn calc_sfb_dist_sum_groups(
    spec: &[FixpDbl],
    quant: &mut [i16],
    gain: i32,
    c: &GainConsts,
) -> Option<FixpDbl> {
    debug_assert_eq!(spec.len(), quant.len());
    let v = VecConsts::new(c);
    let mut xfsf: FixpDbl = 0;
    let mut sum = unsafe { vdupq_n_u32(0) };
    let groups = spec.len() / 4;
    for (sg, qg) in spec.chunks_exact(4).zip(quant.chunks_exact_mut(4)) {
        // SAFETY: NEON is part of the AArch64 baseline; the loads and stores access the chunks
        // of 4 lines; the gathers read indices below 512.
        unsafe {
            let x = vld1q_s32(sg.as_ptr());
            let q = quantize4(x, &v);
            let inv = inv_quantize4(q, &v);
            let (d, bad_d) = dist4(inv, x);
            if vmaxvq_u32(vorrq_u32(over_max_quant(q), bad_d)) == 0 {
                vst1_s16(qg.as_mut_ptr(), vmovn_s32(q));
                sum = vaddq_u32(sum, d);
                continue;
            }
        }
        for (q_out, &x) in qg.iter_mut().zip(sg) {
            let q = quantize_line_fast(x, c);
            *q_out = q;
            if (q as i32).abs() > MAX_QUANT && q != i16::MIN {
                return None;
            }
            xfsf = xfsf.wrapping_add(inv_dist_line(q, x, gain, c).1);
        }
    }
    xfsf = xfsf.wrapping_add(unsafe { vaddvq_u32(sum) } as FixpDbl);
    let tail = groups * 4;
    for (q_out, &x) in quant[tail..].iter_mut().zip(&spec[tail..]) {
        let q = quantize_line_fast(x, c);
        *q_out = q;
        if (q as i32).abs() > MAX_QUANT && q != i16::MIN {
            return None;
        }
        xfsf = xfsf.wrapping_add(inv_dist_line(q, x, gain, c).1);
    }
    Some(xfsf)
}

/// `FDKaacEnc_calcSfbQuantEnergyAndDist()` up to the final `CalcLdData()`: the sums of the
/// energies and the distortions, or `None` for the early return (a line above `MAX_QUANT`).
pub(super) fn calc_sfb_quant_energy_and_dist_sums(
    spec: &[FixpDbl],
    quant: &[i16],
    gain: i32,
    c: &GainConsts,
) -> Option<(FixpDbl, FixpDbl)> {
    debug_assert_eq!(spec.len(), quant.len());
    let v = VecConsts::new(c);
    let mut energy: FixpDbl = 0;
    let mut distortion: FixpDbl = 0;
    let mut en_sum = unsafe { vdupq_n_s32(0) };
    let mut dist_sum = unsafe { vdupq_n_u32(0) };
    let groups = spec.len() / 4;
    for (sg, qg) in spec.chunks_exact(4).zip(quant.chunks_exact(4)) {
        // SAFETY: as in `calc_sfb_dist_sum()`.
        unsafe {
            let x = vld1q_s32(sg.as_ptr());
            let q = vmovl_s16(vld1_s16(qg.as_ptr()));
            let inv = inv_quantize4(q, &v);
            let (d, bad_d) = dist4(inv, x);
            // fPow2(MINVAL_DBL) wraps to MINVAL_DBL, sqdmulh saturates
            let bad = vorrq_u32(bad_d, vceqq_s32(inv, vdupq_n_s32(i32::MIN)));
            if vmaxvq_u32(vorrq_u32(over_max_quant(q), bad)) == 0 {
                let p = vandq_s32(vqdmulhq_s32(inv, inv), vdupq_n_s32(!1));
                en_sum = vaddq_s32(en_sum, p);
                dist_sum = vaddq_u32(dist_sum, d);
                continue;
            }
        }
        for (&q, &x) in qg.iter().zip(sg) {
            if (q as i32).abs() > MAX_QUANT && q != i16::MIN {
                return None;
            }
            let (inv, d) = inv_dist_line(q, x, gain, c);
            energy = energy.wrapping_add(f_pow2(inv));
            distortion = distortion.wrapping_add(d);
        }
    }
    unsafe {
        energy = energy.wrapping_add(vaddvq_s32(en_sum));
        distortion = distortion.wrapping_add(vaddvq_u32(dist_sum) as FixpDbl);
    }
    let tail = groups * 4;
    for (&q, &x) in quant[tail..].iter().zip(&spec[tail..]) {
        if (q as i32).abs() > MAX_QUANT && q != i16::MIN {
            return None;
        }
        let (inv, d) = inv_dist_line(q, x, gain, c);
        energy = energy.wrapping_add(f_pow2(inv));
        distortion = distortion.wrapping_add(d);
    }
    Some((energy, distortion))
}

/// `FDKaacEnc_quantizeLines()` of a band.
pub(super) fn quantize_lines(spec: &[FixpDbl], quant: &mut [i16], c: &GainConsts) {
    debug_assert_eq!(spec.len(), quant.len());
    let v = VecConsts::new(c);
    let groups = spec.len() / 4;
    for (sg, qg) in spec.chunks_exact(4).zip(quant.chunks_exact_mut(4)) {
        // SAFETY: as in `calc_sfb_dist_sum()`.
        unsafe {
            let q = quantize4(vld1q_s32(sg.as_ptr()), &v);
            vst1_s16(qg.as_mut_ptr(), vmovn_s32(q));
        }
    }
    let tail = groups * 4;
    for (q, &x) in quant[tail..].iter_mut().zip(&spec[tail..]) {
        *q = quantize_line_fast(x, c);
    }
}

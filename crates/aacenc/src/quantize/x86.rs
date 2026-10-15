// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/quantize.cpp (optimized; no counterpart in the reference).

//! x86 AVX2 versions of `FDKaacEnc_calcSfbDist()`, `FDKaacEnc_calcSfbQuantEnergyAndDist()` and
//! `FDKaacEnc_quantizeLines()` (eight lines at a time), selected at run time
//! ([`crate::simd::avx2`]).
//!
//! Every lane computes exactly what the scalar kernels of the parent module compute (which equal
//! the straight port):
//!
//! - `fMultDiv2(x, quantizer)` = `floor(x * quantizer / 2^16)` (`quantizer` a positive 16-bit
//!   value) as `(x >> 16) * quantizer + (((x & 0xFFFF) * quantizer) >> 16)` in 32-bit lanes (no
//!   intermediate value exceeds 31 bits);
//! - the leading zeros of a value `0 <= a < 2^31` from the exponent of the `float` conversion of
//!   `a & !(a >> 1)`: clearing the bit below the leading one keeps the rounding from carrying
//!   into the next power of two, so the exponent is exactly `31 - clz(a)` (0 gives 32 after the
//!   clamp);
//! - the 64-bit products of `fMult(s, t)` (per variant: `(s * t) >> 31` on `aarch64`,
//!   `((s * t) >> 32) << 1` on the others) and of `fPow2(n)` = `((n * n) >> 32) << 1` with
//!   `vpmuldq` on the even and odd lanes;
//! - shifts by a register amount modulo 32 take the amount `& 31`; `vpsllvd`/`vpsrlvd` give 0
//!   for amounts of 32 or more, which the distortion uses for the shift direction it does not
//!   take;
//! - the tables indexed by the line's own value are gathers (`FDKaacEnc_mTab_3_4` widened to 32
//!   bits), the 4- and 14-entry rows `vpermd` and `vpshufb` lookups; the inverse quantization of
//!   a group whose values are all below 16 in magnitude reads the 15 entries of
//!   `FDKaacEnc_mTab_4_3Elc` it can use from a 16-entry copy (`vpermd`);
//! - `vpsignd` negates the lanes with a negative operand and clears those with 0, which is the
//!   select of the reference's sign and zero tests.
//!
//! A group of eight lines with a quantized value whose magnitude exceeds `MAX_QUANT` (including
//! -32768) or a distortion difference of 2^31 is recomputed by the scalar code, which also
//! handles the early return. The last `n % 8` lines run through the same kernel padded with
//! zero lines, which contribute 0 to every sum.

use super::{
    dist_line_fast, fdk_aac_enc_inv_quantize_line, inv_quantize_line_fast, quantize_line_fast,
    GainConsts, MAX_QUANT,
};
use crate::aac_enc_rom::{
    FDK_AAC_ENC_M_TAB_3_4, FDK_AAC_ENC_M_TAB_4_3_ELC, FDK_AAC_ENC_QUANT_TABLE_E,
    FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC, FDK_AAC_ENC_SPEC_EXP_TABLE_COMB,
};
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use fdk_aac_rs_fdk::common_fix::{f_pow2, FixpDbl};
use fdk_aac_rs_fdk::{Arch, ARCH};

/// `FDKaacEnc_mTab_3_4` widened to 32 bits (for the gather).
static M_TAB_3_4_I32: [i32; 512] = {
    let mut t = [0i32; 512];
    let mut i = 0;
    while i < 512 {
        t[i] = FDK_AAC_ENC_M_TAB_3_4[i] as i32;
        i += 1;
    }
    t
};

/// `FDKaacEnc_mTab_4_3Elc[]` of the normalized magnitudes `1..=15` (index 0: 0), the entry the
/// inverse quantization reads for `|q| < 16`.
static M_TAB_4_3_SMALL: [i32; 16] = {
    let mut t = [0i32; 16];
    let mut q = 1;
    while q < 16 {
        let an = (q as i32) << ((q as i32).leading_zeros() - 1);
        t[q] = FDK_AAC_ENC_M_TAB_4_3_ELC[((an >> 21) & 511) as usize];
        q += 1;
    }
    t
};

/// The rows of `FDKaacEnc_specExpMantTableCombElc` padded to 16 words (0).
static MANT_ROWS: [[i32; 16]; 4] = {
    let mut t = [[0i32; 16]; 4];
    let mut r = 0;
    while r < 4 {
        let mut i = 0;
        while i < 14 {
            t[r][i] = FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC[r][i];
            i += 1;
        }
        r += 1;
    }
    t
};

/// The rows of `FDKaacEnc_specExpTableComb` as 16 bytes (14 bytes, 0), twice (both 128-bit
/// halves of a `vpshufb` table).
static EXP_ROWS: [[u8; 32]; 4] = {
    let mut t = [[0u8; 32]; 4];
    let mut r = 0;
    while r < 4 {
        let mut i = 0;
        while i < 14 {
            t[r][i] = FDK_AAC_ENC_SPEC_EXP_TABLE_COMB[r][i];
            t[r][16 + i] = FDK_AAC_ENC_SPEC_EXP_TABLE_COMB[r][i];
            i += 1;
        }
        r += 1;
    }
    t
};

/// `FDKaacEnc_quantTableE` twice (a `vpermd` table indexed by `totalShift & 3`).
static QUANT_TABLE_E8: [i32; 8] = {
    let mut t = [0i32; 8];
    let mut i = 0;
    while i < 8 {
        t[i] = FDK_AAC_ENC_QUANT_TABLE_E[i & 3] as i32;
        i += 1;
    }
    t
};

/// The per-call constants of the vector kernels.
struct VecConsts {
    /// `quantizer` (the 16-bit table value)
    quantizer: __m256i,
    /// `quantizershift + 1`
    quantizershift1: __m256i,
    k: __m256i,
    quant_table_e: __m256i,
    /// row `iquantizermod` of `FDKaacEnc_specExpMantTableCombElc`, entries 0..8 and 8..16
    mant_lo: __m256i,
    mant_hi: __m256i,
    /// row `iquantizermod` of `FDKaacEnc_specExpTableComb`
    exp_row: __m256i,
    iquantizershift: __m256i,
    /// [`M_TAB_4_3_SMALL`] entries 0..8 and 8..16
    m43_lo: __m256i,
    m43_hi: __m256i,
}

impl VecConsts {
    #[inline(always)]
    unsafe fn new(c: &GainConsts) -> Self {
        let r = c.iquantizermod;
        VecConsts {
            quantizer: _mm256_set1_epi32((c.quantizer >> 16) as i32),
            quantizershift1: _mm256_set1_epi32(c.quantizershift.wrapping_add(1)),
            k: _mm256_set1_epi32(c.k),
            quant_table_e: _mm256_loadu_si256(QUANT_TABLE_E8.as_ptr() as *const __m256i),
            mant_lo: _mm256_loadu_si256(MANT_ROWS[r].as_ptr() as *const __m256i),
            mant_hi: _mm256_loadu_si256(MANT_ROWS[r].as_ptr().add(8) as *const __m256i),
            exp_row: _mm256_loadu_si256(EXP_ROWS[r].as_ptr() as *const __m256i),
            iquantizershift: _mm256_set1_epi32(c.iquantizershift),
            m43_lo: _mm256_loadu_si256(M_TAB_4_3_SMALL.as_ptr() as *const __m256i),
            m43_hi: _mm256_loadu_si256(M_TAB_4_3_SMALL.as_ptr().add(8) as *const __m256i),
        }
    }
}

/// The leading zeros of every lane `0 <= a < 2^31` (32 for 0).
#[inline(always)]
unsafe fn clz31(a: __m256i) -> __m256i {
    let y = _mm256_andnot_si256(_mm256_srli_epi32::<1>(a), a);
    let e = _mm256_srli_epi32::<23>(_mm256_castps_si256(_mm256_cvtepi32_ps(y)));
    _mm256_min_epi32(_mm256_sub_epi32(_mm256_set1_epi32(158), e), _mm256_set1_epi32(32))
}

/// The high words of the 64-bit products `a * b` of every lane (`(a * b) >> 32`).
#[inline(always)]
unsafe fn mul_hi(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
    let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// `(a * b) >> 31` of every lane (the `aarch64` `fMult()`).
#[inline(always)]
unsafe fn mul_hi31(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<31>(_mm256_mul_epi32(a, b));
    let odd = _mm256_slli_epi64::<1>(_mm256_mul_epi32(
        _mm256_srli_epi64::<32>(a),
        _mm256_srli_epi64::<32>(b),
    ));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// `fMult(FIXP_DBL, FIXP_DBL)` of the variant.
#[inline(always)]
unsafe fn f_mult(a: __m256i, b: __m256i) -> __m256i {
    match ARCH {
        Arch::Aarch64 => mul_hi31(a, b),
        _ => _mm256_slli_epi32::<1>(mul_hi(a, b)),
    }
}

/// `fPow2()` (every variant).
#[inline(always)]
unsafe fn f_pow2_v(a: __m256i) -> __m256i {
    _mm256_slli_epi32::<1>(mul_hi(a, a))
}

/// The low 16 bits of every lane, sign-extended.
#[inline(always)]
unsafe fn sext16(a: __m256i) -> __m256i {
    _mm256_srai_epi32::<16>(_mm256_slli_epi32::<16>(a))
}

/// `FDKaacEnc_quantizeLines()` of eight lines, sign-extended to 32 bits.
#[inline(always)]
unsafe fn quantize8(x: __m256i, v: &VecConsts) -> __m256i {
    let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), v.quantizer);
    let lo = _mm256_mullo_epi32(_mm256_and_si256(x, _mm256_set1_epi32(0xFFFF)), v.quantizer);
    let accu = _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(lo));
    let a = _mm256_abs_epi32(accu);
    let accu_shift = _mm256_sub_epi32(clz31(a), _mm256_set1_epi32(1));
    let an = _mm256_sllv_epi32(a, accu_shift);
    let idx = _mm256_and_si256(_mm256_srli_epi32::<21>(an), _mm256_set1_epi32(511));
    let m34 = _mm256_i32gather_epi32::<4>(M_TAB_3_4_I32.as_ptr(), idx);
    let total_shift = _mm256_sub_epi32(v.quantizershift1, accu_shift);
    let qe = _mm256_permutevar8x32_epi32(v.quant_table_e, total_shift);
    let m = _mm256_mullo_epi32(m34, qe);
    // fixMin(-3 * (totalShift >> 2), 19) + 12, as a right shift modulo 32
    let t = _mm256_srai_epi32::<2>(total_shift);
    let t3 = _mm256_add_epi32(t, _mm256_add_epi32(t, t));
    let sh = _mm256_add_epi32(
        _mm256_min_epi32(_mm256_sub_epi32(_mm256_setzero_si256(), t3), _mm256_set1_epi32(19)),
        _mm256_set1_epi32(12),
    );
    let m = _mm256_srav_epi32(m, _mm256_and_si256(sh, _mm256_set1_epi32(31)));
    let q = _mm256_srai_epi32::<15>(_mm256_add_epi32(v.k, m));
    sext16(_mm256_sign_epi32(q, accu))
}

/// `FDKaacEnc_invQuantizeLines()` of eight lines; lanes with `|q| > MAX_QUANT` are undefined.
#[inline(always)]
unsafe fn inv_quantize8(q: __m256i, v: &VecConsts) -> __m256i {
    let qa = _mm256_abs_epi32(q);
    let small =
        _mm256_movemask_ps(_mm256_castsi256_ps(_mm256_cmpgt_epi32(qa, _mm256_set1_epi32(15)))) == 0;
    inv_quantize8_with(q, v, small)
}

/// [`inv_quantize8`] with the test `max |q| < 16` given (`small` must imply it).
#[inline(always)]
unsafe fn inv_quantize8_with(q: __m256i, v: &VecConsts, small: bool) -> __m256i {
    let qa = _mm256_abs_epi32(q);
    let clz = clz31(qa);
    let an = _mm256_sllv_epi32(qa, _mm256_sub_epi32(clz, _mm256_set1_epi32(1)));
    // 0 for q == 0 (the result is replaced by 0), else 1..=13
    let spec_exp = _mm256_sub_epi32(_mm256_set1_epi32(32), clz);
    let s = if small {
        // all |q| < 16 (the common case): the table entries of 1..=15 by vpermd
        _mm256_blendv_epi8(
            _mm256_permutevar8x32_epi32(v.m43_lo, qa),
            _mm256_permutevar8x32_epi32(v.m43_hi, qa),
            _mm256_cmpgt_epi32(qa, _mm256_set1_epi32(7)),
        )
    } else {
        let idx = _mm256_and_si256(_mm256_srli_epi32::<21>(an), _mm256_set1_epi32(511));
        _mm256_i32gather_epi32::<4>(FDK_AAC_ENC_M_TAB_4_3_ELC.as_ptr(), idx)
    };
    let t = _mm256_blendv_epi8(
        _mm256_permutevar8x32_epi32(v.mant_lo, spec_exp),
        _mm256_permutevar8x32_epi32(v.mant_hi, spec_exp),
        _mm256_cmpgt_epi32(spec_exp, _mm256_set1_epi32(7)),
    );
    let accu = f_mult(s, t);
    // byte 0 of every lane selects the row entry, the other bytes give 0
    let e = _mm256_shuffle_epi8(
        v.exp_row,
        _mm256_or_si256(spec_exp, _mm256_set1_epi32(0x8080_8000u32 as i32)),
    );
    let st = _mm256_add_epi32(v.iquantizershift, e);
    // st >= 2: left by (st - 1) & 31, else right by (1 - st) & 31
    let one = _mm256_set1_epi32(1);
    let mask31 = _mm256_set1_epi32(31);
    let left = _mm256_sllv_epi32(accu, _mm256_and_si256(_mm256_sub_epi32(st, one), mask31));
    let right = _mm256_srav_epi32(accu, _mm256_and_si256(_mm256_sub_epi32(one, st), mask31));
    let accu = _mm256_blendv_epi8(right, left, _mm256_cmpgt_epi32(st, one));
    _mm256_sign_epi32(accu, q)
}

/// The distortion of eight lines (see `dist_line_fast()`) and a mask of the lanes whose
/// difference is 2^31 (which need the straight code).
#[inline(always)]
unsafe fn dist8(inv: __m256i, x: __m256i) -> (__m256i, __m256i) {
    let a = _mm256_abs_epi32(inv);
    let b = _mm256_abs_epi32(_mm256_srai_epi32::<1>(x));
    let d = _mm256_abs_epi32(_mm256_sub_epi32(a, b));
    let clz = clz31(d);
    let n = _mm256_srli_epi32::<1>(_mm256_sllv_epi32(d, clz));
    let p = f_pow2_v(n);
    // (p << 4) >> min(2 * clz, 35) with p < 2^31: left by 2 for clz == 1, else right by
    // 2 * clz - 4 (amounts of 31 or more give 0); the other direction's amount is >= 32
    let clz2 = _mm256_add_epi32(clz, clz);
    let right = _mm256_srlv_epi32(p, _mm256_sub_epi32(clz2, _mm256_set1_epi32(4)));
    let left = _mm256_sllv_epi32(p, _mm256_sub_epi32(_mm256_set1_epi32(4), clz2));
    (_mm256_or_si256(right, left), d)
}

/// Whether `|q| > MAX_QUANT` (in 32 bits) in a lane.
#[inline(always)]
unsafe fn over_max_quant(q: __m256i) -> __m256i {
    _mm256_cmpgt_epi32(_mm256_abs_epi32(q), _mm256_set1_epi32(MAX_QUANT))
}

/// Stores the low 16 bits of the eight lanes of `q` (sign-extended values) to `out`.
#[inline(always)]
unsafe fn store_q8(out: *mut i16, q: __m256i) {
    let p = _mm256_permute4x64_epi64::<0b1000>(_mm256_packs_epi32(q, q));
    _mm_storeu_si128(out as *mut __m128i, _mm256_castsi256_si128(p));
}

/// Loads `n <= 8` lines, padded with zeros (4 lines, the width of the narrowest bands, with one
/// 128-bit load).
#[inline(always)]
unsafe fn load8(s: &[FixpDbl]) -> __m256i {
    match s.len() {
        8 => _mm256_loadu_si256(s.as_ptr() as *const __m256i),
        4 => _mm256_zextsi128_si256(_mm_loadu_si128(s.as_ptr() as *const __m128i)),
        _ => {
            let mut b = [0i32; 8];
            b[..s.len()].copy_from_slice(s);
            _mm256_loadu_si256(b.as_ptr() as *const __m256i)
        }
    }
}

/// Loads `n <= 8` quantized lines sign-extended to 32 bits, padded with zeros.
#[inline(always)]
unsafe fn load_q8(s: &[i16]) -> __m256i {
    match s.len() {
        8 => _mm256_cvtepi16_epi32(_mm_loadu_si128(s.as_ptr() as *const __m128i)),
        4 => _mm256_cvtepi16_epi32(_mm_loadl_epi64(s.as_ptr() as *const __m128i)),
        _ => {
            let mut b = [0i16; 8];
            b[..s.len()].copy_from_slice(s);
            _mm256_cvtepi16_epi32(_mm_loadu_si128(b.as_ptr() as *const __m128i))
        }
    }
}

/// Stores the low 16 bits of the first `qg.len() <= 8` lanes of `q` (sign-extended values).
#[inline(always)]
unsafe fn store_q(qg: &mut [i16], q: __m256i) {
    match qg.len() {
        8 => store_q8(qg.as_mut_ptr(), q),
        4 => {
            let p = _mm256_permute4x64_epi64::<0b1000>(_mm256_packs_epi32(q, q));
            _mm_storel_epi64(qg.as_mut_ptr() as *mut __m128i, _mm256_castsi256_si128(p));
        }
        n => {
            let mut b = [0i16; 8];
            store_q8(b.as_mut_ptr(), q);
            qg.copy_from_slice(&b[..n]);
        }
    }
}

/// The wrapping sum of the eight lanes.
#[inline(always)]
unsafe fn hsum(v: __m256i) -> i32 {
    let s = _mm_add_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b0100_1110>(s));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b1011_0001>(s));
    _mm_cvtsi128_si32(s)
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

/// The horizontal maximum of eight lanes.
#[inline(always)]
unsafe fn hmax(v: __m256i) -> i32 {
    let s = _mm_max_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
    let s = _mm_max_epi32(s, _mm_shuffle_epi32::<0b0100_1110>(s));
    let s = _mm_max_epi32(s, _mm_shuffle_epi32::<0b1011_0001>(s));
    _mm_cvtsi128_si32(s)
}

/// `FDKaacEnc_calcSfbDist()` up to the final `CalcLdData()`: the sum of the distortions, or
/// `None` for the early return (a line above `MAX_QUANT`).
///
/// Two passes over the groups of 8 lines (the NEON version explains the method):
/// the quantization into a local buffer with the band's maximum magnitude, then the inverse
/// quantization (one `|q| < 16` decision per band) and distortion; the quantized lines are
/// stored only if no line exceeds `MAX_QUANT` and no distortion difference is 2^31, otherwise
/// [`calc_sfb_dist_sum_groups`] computes the band from the start. The lanes past the band are
/// zero lines, which contribute 0.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn calc_sfb_dist_sum(
    spec: &[FixpDbl],
    quant: &mut [i16],
    gain: i32,
    c: &GainConsts,
) -> Option<FixpDbl> {
    debug_assert_eq!(spec.len(), quant.len());
    let n = spec.len();
    if n == 0 || n > TWO_PASS_MAX {
        return calc_sfb_dist_sum_groups(spec, quant, gain, c);
    }
    let v = VecConsts::new(c);
    let mut qb = [0i16; TWO_PASS_MAX];
    // the lines of the band in groups of 8, the last group loaded with a mask (the lanes past
    // the band are 0); no branch per group
    let groups = n.div_ceil(8);
    let tail_mask = _mm256_cmpgt_epi32(
        _mm256_set1_epi32((n - 8 * (groups - 1)) as i32),
        _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7),
    );
    let sp = spec.as_ptr();
    let mut qmax = _mm256_setzero_si256();
    for g in 0..groups {
        let x = if g + 1 < groups {
            _mm256_loadu_si256(sp.add(8 * g) as *const __m256i)
        } else {
            _mm256_maskload_epi32(sp.add(8 * g), tail_mask)
        };
        let q = quantize8(x, &v);
        qmax = _mm256_max_epi32(qmax, _mm256_abs_epi32(q));
        store_q8(qb.as_mut_ptr().add(8 * g), q);
    }
    let qmax = hmax(qmax);
    if qmax > MAX_QUANT {
        return calc_sfb_dist_sum_groups(spec, quant, gain, c);
    }
    let small = qmax < 16;
    let mut sum = _mm256_setzero_si256();
    let mut bad = _mm256_setzero_si256();
    for g in 0..groups {
        let x = if g + 1 < groups {
            _mm256_loadu_si256(sp.add(8 * g) as *const __m256i)
        } else {
            _mm256_maskload_epi32(sp.add(8 * g), tail_mask)
        };
        let q = _mm256_cvtepi16_epi32(_mm_loadu_si128(qb.as_ptr().add(8 * g) as *const __m128i));
        let (d, bad_d) = dist8(inv_quantize8_with(q, &v, small), x);
        bad = _mm256_or_si256(bad, bad_d);
        sum = _mm256_add_epi32(sum, d);
    }
    if _mm256_movemask_ps(_mm256_castsi256_ps(bad)) != 0 {
        return calc_sfb_dist_sum_groups(spec, quant, gain, c);
    }
    // quant = qb[..n] in 8- and 4-line pieces (without a call of memcpy)
    let (dst, src) = (quant.as_mut_ptr(), qb.as_ptr());
    let mut i = 0;
    while i + 8 <= n {
        _mm_storeu_si128(dst.add(i) as *mut __m128i, _mm_loadu_si128(src.add(i) as *const __m128i));
        i += 8;
    }
    if i + 4 <= n {
        _mm_storel_epi64(dst.add(i) as *mut __m128i, _mm_loadl_epi64(src.add(i) as *const __m128i));
        i += 4;
    }
    // the last 0..=3 lines one by one (a copy_from_slice here is a memcpy call, also for none)
    for k in 0..3 {
        if i + k < n {
            quant[i + k] = qb[i + k];
        }
    }
    Some(hsum(sum))
}

/// [`calc_sfb_dist_sum`] group by group (its fallback).
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn calc_sfb_dist_sum_groups(
    spec: &[FixpDbl],
    quant: &mut [i16],
    gain: i32,
    c: &GainConsts,
) -> Option<FixpDbl> {
    debug_assert_eq!(spec.len(), quant.len());
    let v = VecConsts::new(c);
    let mut xfsf: FixpDbl = 0;
    let mut sum = _mm256_setzero_si256();
    for (sg, qg) in spec.chunks(8).zip(quant.chunks_mut(8)) {
        let x = load8(sg);
        let q = quantize8(x, &v);
        let inv = inv_quantize8(q, &v);
        let (d, bad_d) = dist8(inv, x);
        if _mm256_movemask_ps(_mm256_castsi256_ps(_mm256_or_si256(over_max_quant(q), bad_d))) == 0 {
            store_q(qg, q);
            sum = _mm256_add_epi32(sum, d);
            continue;
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
    Some(xfsf.wrapping_add(hsum(sum)))
}

/// `FDKaacEnc_calcSfbQuantEnergyAndDist()` up to the final `CalcLdData()`: the sums of the
/// energies and the distortions, or `None` for the early return (a line above `MAX_QUANT`).
///
/// `fPow2(MINVAL_DBL)` wraps to `MINVAL_DBL` in the vector form as in the scalar one.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn calc_sfb_quant_energy_and_dist_sums(
    spec: &[FixpDbl],
    quant: &[i16],
    gain: i32,
    c: &GainConsts,
) -> Option<(FixpDbl, FixpDbl)> {
    debug_assert_eq!(spec.len(), quant.len());
    let v = VecConsts::new(c);
    let mut energy: FixpDbl = 0;
    let mut distortion: FixpDbl = 0;
    let mut en_sum = _mm256_setzero_si256();
    let mut dist_sum = _mm256_setzero_si256();
    for (sg, qg) in spec.chunks(8).zip(quant.chunks(8)) {
        let x = load8(sg);
        let q = load_q8(qg);
        let inv = inv_quantize8(q, &v);
        let (d, bad_d) = dist8(inv, x);
        if _mm256_movemask_ps(_mm256_castsi256_ps(_mm256_or_si256(over_max_quant(q), bad_d))) == 0 {
            en_sum = _mm256_add_epi32(en_sum, f_pow2_v(inv));
            dist_sum = _mm256_add_epi32(dist_sum, d);
            continue;
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
    Some((energy.wrapping_add(hsum(en_sum)), distortion.wrapping_add(hsum(dist_sum))))
}

/// `FDKaacEnc_quantizeLines()` of a band.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn quantize_lines(spec: &[FixpDbl], quant: &mut [i16], c: &GainConsts) {
    debug_assert_eq!(spec.len(), quant.len());
    let v = VecConsts::new(c);
    for (sg, qg) in spec.chunks(8).zip(quant.chunks_mut(8)) {
        store_q(qg, quantize8(load8(sg), &v));
    }
}

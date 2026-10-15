// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized versions of functions of libSBRdec/src/env_calc.cpp (x86 hosts).

//! AVX2 kernels of the envelope adjustment (x86 hosts, selected at run time with
//! `fdk_aac_rs_fdk::simd_x86::avx2()`, so `FDK_AAC_RS_NO_SIMD` disables them): the same
//! operations as the NEON kernels of `env_calc.rs`, eight subbands at a time. All products are
//! the same on every variant and computed exactly:
//!
//! * `fMultDiv2(FIXP_DBL, FIXP_DBL)`: the high halves of the 64 bit products (`vpmuldq` of the
//!   even and of the odd lanes);
//! * `fMultDiv2(FIXP_SGL a, FIXP_DBL x)` = `(a * x) >> 16`: `(x >> 16) * a + (((x & 0xFFFF) *
//!   a) >> 16)` in 32 bit lanes (`|a * (x & 0xFFFF)| < 2^31`); `fMult(FIXP_SGL, FIXP_DBL)` is
//!   that doubled (`fixmul_SD` is `fixmuldiv2_SD << 1` on every variant);
//! * per-lane shifts with `vpsllvd`/`vpsravd`, uniform ones with `vpslld`/`vpsrad` (amounts
//!   `0..32`).

use super::*;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[inline(always)]
unsafe fn ld(x: &[FixpDbl], k: usize) -> __m256i {
    debug_assert!(k + 8 <= x.len());
    _mm256_loadu_si256(x.as_ptr().add(k) as *const __m256i)
}

#[inline(always)]
unsafe fn st(x: &mut [FixpDbl], k: usize, v: __m256i) {
    debug_assert!(k + 8 <= x.len());
    _mm256_storeu_si256(x.as_mut_ptr().add(k) as *mut __m256i, v)
}

/// The lanes `l` with `0 <= start + l < n` (all ones) and the others (0).
#[inline(always)]
unsafe fn in_range(start: isize, n: usize) -> __m256i {
    let idx = _mm256_add_epi32(
        _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7),
        _mm256_set1_epi32(start as i32),
    );
    _mm256_andnot_si256(
        _mm256_cmpgt_epi32(_mm256_setzero_si256(), idx),
        _mm256_cmpgt_epi32(_mm256_set1_epi32(n as i32), idx),
    )
}

/// `x[start + l]` in the lanes with `0 <= start + l < n` (`n <= x.len()`), 0 in the others
/// (masked load: those lanes are not accessed).
#[inline(always)]
unsafe fn ldm(x: &[FixpDbl], start: isize, n: usize) -> __m256i {
    debug_assert!(n <= x.len());
    _mm256_maskload_epi32(x.as_ptr().wrapping_offset(start), in_range(start, n))
}

#[inline(always)]
unsafe fn arr(v: [i32; 8]) -> __m256i {
    _mm256_loadu_si256(v.as_ptr() as *const __m256i)
}

/// `fMultDiv2(FIXP_DBL, FIXP_DBL)` per lane.
#[inline(always)]
unsafe fn dd(a: __m256i, b: __m256i) -> __m256i {
    let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
    let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
    _mm256_blend_epi32::<0b1010_1010>(even, odd)
}

/// `fMultDiv2(FIXP_SGL a, FIXP_DBL x)` per lane (`a` sign-extended in the lanes).
#[inline(always)]
unsafe fn sd(x: __m256i, a: __m256i) -> __m256i {
    let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), a);
    let lo = _mm256_and_si256(x, _mm256_set1_epi32(0xFFFF));
    _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, a)))
}

#[inline(always)]
unsafe fn clamp(v: __m256i, maxv: __m256i, minv: __m256i) -> __m256i {
    _mm256_max_epi32(_mm256_min_epi32(v, maxv), minv)
}

#[inline(always)]
unsafe fn select(mask: __m256i, a: __m256i, b: __m256i) -> __m256i {
    _mm256_blendv_epi8(b, a, mask)
}

/// `x ^ (x >> 31)` ORed over `x` (the magnitude accumulator of `maxSubbandSample()`).
///
/// # Safety
/// The CPU must have AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn or_mag(acc: &mut __m256i, x: &[FixpDbl]) -> FixpDbl {
    let n8 = x.len() & !7;
    let mut k = 0;
    while k < n8 {
        let v = ld(x, k);
        *acc = _mm256_or_si256(*acc, _mm256_xor_si256(v, _mm256_srai_epi32::<31>(v)));
        k += 8;
    }
    let mut r = 0;
    for &t in &x[n8..] {
        r |= t ^ (t >> 31);
    }
    r
}

/// A zero accumulator for [`or_mag`].
///
/// # Safety
/// The CPU must have AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn zero() -> __m256i {
    _mm256_setzero_si256()
}

/// The OR of the lanes.
///
/// # Safety
/// The CPU must have AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn or_lanes(v: __m256i) -> FixpDbl {
    let mut a = [0i32; 8];
    _mm256_storeu_si256(a.as_mut_ptr() as *mut __m256i, v);
    a.iter().fold(0, |r, &x| r | x)
}

/// [`super::calc_nrg_per_subband`] for the subbands `lo..lo + n8` (`n8` a multiple of 8); every
/// row covers them (checked by the caller).
///
/// # Safety
/// The CPU must have AVX2.
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2")]
pub(super) unsafe fn per_subband(
    re: &[&mut [FixpDbl]],
    im: Option<&[&mut [FixpDbl]]>,
    lo: usize,
    n8: usize,
    slots: core::ops::Range<usize>,
    frame_exp: i32,
    inv_width: FixpSgl,
    nrg_est: &mut [FixpDbl],
    nrg_est_e: &mut [i8],
) {
    let rows_re = &re[slots.clone()];
    let rows_im = im.map(|im| &im[slots.clone()]);
    for k in (0..n8).step_by(8) {
        let b = lo + k;
        assert!(rows_re.iter().all(|r| r.len() >= b + 8));
        assert!(rows_im.is_none_or(|rows| rows.iter().all(|r| r.len() >= b + 8)));
        let mag = |v: __m256i| _mm256_xor_si256(v, _mm256_srai_epi32::<31>(v));
        let mut mx = _mm256_setzero_si256();
        for r in rows_re {
            mx = _mm256_or_si256(mx, mag(ld(r, b)));
        }
        if let Some(rows) = rows_im {
            for r in rows {
                mx = _mm256_or_si256(mx, mag(ld(r, b)));
            }
        }
        let mut mxa = [0i32; 8];
        _mm256_storeu_si256(mxa.as_mut_ptr() as *mut __m256i, mx);
        let ps = mxa.map(|m| {
            if m == 0 {
                0
            } else {
                f_min((cnt_leading_zeros(m) - 1) as i8 - SHIFT_BEFORE_SQUARE as i8, 25) as i32
            }
        });
        // `x << ps` for ps >= 0, `x >> -ps` otherwise
        let psl = arr(ps.map(|p| p.max(0)));
        let psr = arr(ps.map(|p| (-p).max(0)));
        let sq = |v: __m256i| {
            let y = _mm256_srav_epi32(_mm256_sllv_epi32(v, psl), psr);
            dd(y, y)
        };
        let mut acc = _mm256_setzero_si256();
        for r in rows_re {
            acc = _mm256_add_epi32(acc, sq(ld(r, b)));
        }
        if let Some(rows) = rows_im {
            for r in rows {
                acc = _mm256_add_epi32(acc, sq(ld(r, b)));
            }
        }
        let mut acca = [0i32; 8];
        _mm256_storeu_si256(acca.as_mut_ptr() as *mut __m256i, acc);
        for j in 0..8 {
            let (nrg, e) = nrg_of_subband(mxa[j], |_| acca[j], frame_exp, inv_width, im.is_none());
            nrg_est[k + j] = nrg;
            nrg_est_e[k + j] = e;
        }
    }
}

/// The noise phases (`FIXP_SGL`, sign-extended) of the subbands `index + 0..8`.
#[inline(always)]
unsafe fn phases(index: i32, part: usize) -> __m256i {
    arr(core::array::from_fn(|l| {
        let i = (index + l as i32) & (SBR_NF_NO_RANDOM_VAL - 1);
        FDK_SBR_DECODER_SBR_RANDOM_PHASE[i as usize][part] as i32
    }))
}

/// [`super::adjust_time_slot_eld_grid`]: all `ns` subbands eight at a time (the AVX2 form of
/// `neon_eld::all`, see there): `slots`, `sine` have `ns <= MAX_FREQ_COEFFS` values, `gain` and
/// `noise` are the full tables.
///
/// # Safety
/// The CPU must have AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn eld_all(
    slots: &mut [FixpDbl],
    gain: &[FixpDbl; MAX_FREQ_COEFFS],
    noise: &[FixpDbl; MAX_FREQ_COEFFS],
    sine: &[FixpDbl],
    e: &AdjEld,
) {
    let ns = slots.len();
    assert!(sine.len() == ns && ns <= MAX_FREQ_COEFFS && MAX_FREQ_COEFFS % 8 == 0);
    // The values sine[k - 1 + l] (zero before and after the ns sines) and the slot values of a
    // block come from masked loads (lanes outside 0..ns read as 0 and are not accessed), and
    // the last block is stored masked: no padded copies of the slices.
    let iota = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
    let (maxv, minv) = (_mm256_set1_epi32(e.max_val), _mm256_set1_epi32(e.min_val));
    let sc = _mm_cvtsi32_si128(e.sc);
    let hp0 = _mm256_set1_epi32(e.hp0);
    let zero = _mm256_setzero_si256();
    let noise_on = _mm256_set1_epi32(if e.noise_on { -1 } else { 0 });
    // harmonicPhaseX of the lower and the upper neighbour per lane (lane l is subband k + l
    // with k a multiple of 8: the parity of lo + l)
    let pxl = arr(core::array::from_fn(|l| e.hpx[(e.lo + l) & 1]));
    let pxu = arr(core::array::from_fn(|l| e.hpx[(e.lo + 1 + l) & 1]));
    let mut k = 0;
    while k < ns {
        // k + 8 <= MAX_FREQ_COEFFS for the tables
        let ki = k as isize;
        let s = ldm(sine, ki, ns);
        let x = if k + 8 <= ns { ld(slots, k) } else { ldm(slots, ki, ns) };
        let mut sig = _mm256_sll_epi32(clamp(dd(x, ld(gain, k)), maxv, minv), sc);
        let nterm =
            _mm256_slli_epi32::<1>(sd(ld(noise, k), phases(e.phase_index + k as i32 + 1, 0)));
        let use_noise = _mm256_and_si256(_mm256_cmpeq_epi32(s, zero), noise_on);
        sig = _mm256_add_epi32(sig, _mm256_and_si256(use_noise, nterm));
        sig = _mm256_add_epi32(sig, _mm256_mullo_epi32(s, hp0));
        let nb = _mm256_add_epi32(dd(ldm(sine, ki - 1, ns), pxl), dd(ldm(sine, ki + 1, ns), pxu));
        // k + l < k_tone (both below 2^31)
        let on = _mm256_cmpgt_epi32(
            _mm256_set1_epi32(e.k_tone as i32),
            _mm256_add_epi32(iota, _mm256_set1_epi32(k as i32)),
        );
        let out = _mm256_add_epi32(sig, _mm256_and_si256(on, nb));
        if k + 8 <= ns {
            st(slots, k, out);
        } else {
            _mm256_maskstore_epi32(slots.as_mut_ptr().add(k), in_range(ki, ns), out);
        }
        k += 8;
    }
}

/// [`super::adjust_time_slot_lc`]: middle subbands `1..j_end` eight at a time (`j_end - 1` a
/// multiple of 8, `j_end <= ns - 1`); returns `j_end`. All slices have `ns >= 3` values.
///
/// # Safety
/// The CPU must have AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn lc_mid(
    slots: &mut [FixpDbl],
    gain: &[FixpDbl],
    noise: &[FixpDbl],
    sine: &[FixpDbl],
    m: &AdjLcMid,
) -> usize {
    let ns = slots.len();
    assert!(gain.len() == ns && noise.len() == ns && sine.len() == ns && ns >= 3);
    let j_end = 1 + ((ns - 2) & !7);
    let (maxv, minv) = (_mm256_set1_epi32(m.max_val), _mm256_set1_epi32(m.min_val));
    let sc = _mm_cvtsi32_si128(m.sc);
    let c1 = _mm256_set1_epi32(C1 as i32);
    let zero = _mm256_setzero_si256();
    let noise_on = _mm256_set1_epi32(if m.noise_on { -1 } else { 0 });
    let lanes = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
    let j_tone = _mm256_set1_epi32(m.j_tone as i32);
    let mut j = 1;
    while j < j_end {
        let s = ld(sine, j);
        let adjusted = _mm256_sll_epi32(clamp(dd(ld(slots, j), ld(gain, j)), maxv, minv), sc);
        let nterm = _mm256_slli_epi32::<1>(sd(ld(noise, j), phases(m.index + j as i32, 0)));
        let no_sine = _mm256_cmpeq_epi32(s, zero);
        let use_noise = _mm256_and_si256(no_sine, noise_on);
        let out = if !m.odd {
            let sig = if m.sine_sign { _mm256_sub_epi32(zero, s) } else { s };
            _mm256_add_epi32(select(use_noise, nterm, sig), adjusted)
        } else {
            let sig = _mm256_add_epi32(adjusted, _mm256_and_si256(use_noise, nterm));
            let add = sd(_mm256_sub_epi32(ld(sine, j - 1), ld(sine, j + 1)), c1);
            // inv for subband j + l: freq_inv1 ^ ((j + l - 1) odd)
            let sign = arr(core::array::from_fn(|l| {
                if m.freq_inv1 ^ ((j + l - 1) & 1 != 0) {
                    -1
                } else {
                    1
                }
            }));
            let add = _mm256_mullo_epi32(add, sign);
            let on =
                _mm256_cmpgt_epi32(j_tone, _mm256_add_epi32(lanes, _mm256_set1_epi32(j as i32)));
            _mm256_add_epi32(sig, _mm256_and_si256(on, add))
        };
        st(slots, j, out);
        j += 8;
    }
    j_end
}

/// [`super::adjust_time_slot_hq_fast`]: subbands `0..k_end` (a multiple of 8; all slices have at
/// least `k_end` values).
///
/// # Safety
/// The CPU must have AVX2.
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2")]
pub(super) unsafe fn hq(
    re: &mut [FixpDbl],
    im: &mut [FixpDbl],
    gain: &[FixpDbl],
    noise: &[FixpDbl],
    sine: &[FixpDbl],
    fb: &[FixpDbl],
    fbn: &[FixpDbl],
    k_end: usize,
    p: &AdjHq,
) {
    assert!([re.len(), im.len(), gain.len(), noise.len(), sine.len(), fb.len(), fbn.len()]
        .iter()
        .all(|&l| k_end <= l));
    let sr = _mm256_set1_epi32(p.smooth_ratio as i32);
    let dr = _mm256_set1_epi32(p.direct_ratio as i32);
    let (maxv, minv) = (_mm256_set1_epi32(p.max_val), _mm256_set1_epi32(p.min_val));
    let (maxn, minn) = (_mm256_set1_epi32(p.max_val_noise), _mm256_set1_epi32(p.min_val_noise));
    let (maxh, minh) = (_mm256_set1_epi32(MAXVAL_DBL / 2), _mm256_set1_epi32(MINVAL_DBL / 2));
    let nsh = _mm_cvtsi32_si128(p.shift);
    let sc = _mm_cvtsi32_si128(p.scale_change_amt);
    let zero = _mm256_setzero_si256();
    // harmonics: the sine is added to the real part with the sign sr, to the imaginary part
    // with the sign si (per lane; k is a multiple of 8, so the parity of the lane is the
    // parity of k)
    let lane_inv: [i32; 8] = core::array::from_fn(|j| (p.freq_inv0 + j as i32) & 1);
    let (hr, hi): ([i32; 8], [i32; 8]) = match p.harm_index() {
        Some(0) => ([1; 8], [0; 8]),
        Some(2) => ([-1; 8], [0; 8]),
        Some(1) => ([0; 8], lane_inv.map(|v| if v != 0 { -1 } else { 1 })),
        Some(_) => ([0; 8], lane_inv.map(|v| if v != 0 { 1 } else { -1 })),
        None => ([0; 8], [0; 8]),
    };
    let (hr, hi) = (arr(hr), arr(hi));
    let noise_on = _mm256_set1_epi32(if p.noise_on { -1 } else { 0 });

    let mut k = 0;
    while k < k_end {
        let (g, nl) = (ld(gain, k), ld(noise, k));
        let (sg, sn) = if p.smooth_ratio > 0 {
            let sg = _mm256_add_epi32(
                _mm256_slli_epi32::<1>(sd(ld(fb, k), sr)),
                _mm256_slli_epi32::<1>(sd(g, dr)),
            );
            let t = sd(ld(fbn, k), sr);
            let t = if p.noise_shift_neg {
                _mm256_sra_epi32(t, nsh)
            } else {
                _mm256_sll_epi32(clamp(t, maxn, minn), nsh)
            };
            let sn = _mm256_add_epi32(t, _mm256_slli_epi32::<1>(sd(nl, dr)));
            (sg, clamp(sn, maxh, minh))
        } else {
            (g, nl)
        };
        let adj = |x: __m256i| _mm256_sll_epi32(clamp(dd(x, sg), maxv, minv), sc);
        let sig_r = adj(ld(re, k));
        let sig_i = adj(ld(im, k));

        let s = ld(sine, k);
        let no_sine = _mm256_cmpeq_epi32(s, zero);
        let index = p.index0 + k as i32 + 1;
        let nr = _mm256_slli_epi32::<1>(sd(sn, phases(index, 0)));
        let ni = _mm256_slli_epi32::<1>(sd(sn, phases(index, 1)));
        let add_noise = _mm256_and_si256(noise_on, no_sine);
        let add_r = select(no_sine, _mm256_and_si256(add_noise, nr), _mm256_mullo_epi32(s, hr));
        let add_i = select(no_sine, _mm256_and_si256(add_noise, ni), _mm256_mullo_epi32(s, hi));
        st(re, k, _mm256_add_epi32(sig_r, add_r));
        st(im, k, _mm256_add_epi32(sig_i, add_i));
        k += 8;
    }
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/src/autocorr2nd.cpp and libFDK/src/scale.cpp (optimized; no counterpart in
// the reference).

//! `autoCorr2nd_cplx()` and the scale factor of `FDKsbrEnc_CalculateTonalityQuotas()` for
//! several bands at once, one band per vector lane: NEON (4 lanes) and AVX2 (8 lanes).
//!
//! The lane operations compute per lane exactly what the scalar operations of the straight code
//! compute: wrapping additions, `fMultDiv2(a, b)` = `floor(a * b / 2^32)` as the high word of the
//! exact 64-bit product (`smull`/`vpmuldq`; no saturating form), arithmetic shifts, `fAbs()` with
//! wrap-around (`abs(MINVAL_DBL) = MINVAL_DBL`), the leading zeros of the 32-bit pattern, and left
//! shifts by per-lane amounts in `0..=31` (the straight code's shifts by `mScale = -1` take the
//! amount modulo 32, i.e. 31).

/// The lane operations.
pub(super) trait Lanes: Copy {
    /// The number of lanes.
    const N: usize;
    unsafe fn splat(x: i32) -> Self;
    /// Loads `N` values.
    unsafe fn load(p: *const i32) -> Self;
    /// Stores `N` values.
    unsafe fn store(self, p: *mut i32);
    unsafe fn add(self, b: Self) -> Self;
    unsafe fn sub(self, b: Self) -> Self;
    unsafe fn or(self, b: Self) -> Self;
    unsafe fn xor(self, b: Self) -> Self;
    /// Arithmetic right shift of every lane by `n` (`0 <= n < 32`).
    unsafe fn sra(self, n: i32) -> Self;
    /// `floor(a * b / 2^32)` of every lane.
    unsafe fn mulhi(self, b: Self) -> Self;
    /// Absolute value with wrap-around.
    unsafe fn abs(self) -> Self;
    /// The leading zeros of every lane's 32-bit pattern (32 for 0).
    unsafe fn clz(self) -> Self;
    /// Left shift of every lane by its amount in `cnt` (`0..=31`).
    unsafe fn shl_lanes(self, cnt: Self) -> Self;
    unsafe fn min(self, b: Self) -> Self;
    unsafe fn max(self, b: Self) -> Self;
    unsafe fn and(self, b: Self) -> Self;

    /// `fixMax(0, fixMin(getScalefactor(re), getScalefactor(im)) - 1)` of every lane, where
    /// `getScalefactor(x) = fixMax(0, fixnormz_D(OR(x ^ (x >> 31))) - 1)`.
    #[inline(always)]
    unsafe fn scalefactor_pair(re: &[Self], im: &[Self]) -> Self {
        // (no closures here: on x86 the lane operations must be inlined into the AVX2 caller)
        let zero = Self::splat(0);
        let one = Self::splat(1);
        let mut mr = zero;
        for &v in re {
            mr = mr.or(v.xor(v.sra(31)));
        }
        let mut mi = zero;
        for &v in im {
            mi = mi.or(v.xor(v.sra(31)));
        }
        let gr = mr.clz().sub(one).max(zero);
        let gi = mi.clz().sub(one).max(zero);
        gr.min(gi).sub(one).max(zero)
    }
}

/// The result of [`acorr`]: per lane `r00r, r11r, r22r, r01r, r02r, r12r, r01i, r02i, r12i, det,
/// det_scale` and the returned scaling of `autoCorr2nd_cplx()`.
pub(super) struct AcorrLanes<V> {
    pub values: [V; 12],
}

/// `autoCorr2nd_cplx(ac, reBuffer, imBuffer, len)` of every lane: `re[j]`, `im[j]` hold
/// `reBuffer[j - LPC_ORDER]` of the lanes' bands, `len + LPC_ORDER` slots (`len >= 1`). The same
/// expressions in the same order as `fdk_aac_rs_fdk::autocorr2nd::auto_corr2nd_cplx`.
#[inline(always)]
pub(super) unsafe fn acorr<V: Lanes>(re: &[V], im: &[V], len: i32) -> AcorrLanes<V> {
    debug_assert!(len >= 1 && re.len() >= len as usize + 2 && im.len() >= len as usize + 2);
    let len_scale = (32 - (len as u32).leading_zeros() as i32).max(1);
    let ls = len_scale;
    let n = len as usize;
    // (no closures here: on x86 the lane operations must be inlined into the AVX2 caller)
    // realBuf[k] = re[k + 2]: realBuf[-2] = re[0], realBuf[-1] = re[1], realBuf[len - 2] =
    // re[n], realBuf[len - 1] = re[n + 1]

    // pReBuf = realBuf - 2
    let mut accu7 = re[2].mulhi(re[0]).add(im[2].mulhi(im[0])).sra(ls);
    let mut accu8 = im[2].mulhi(re[0]).sub(re[2].mulhi(im[0])).sra(ls);
    let mut accu1 = V::splat(0);
    let mut accu3 = V::splat(0);
    let mut accu5 = V::splat(0);
    // pReBuf = realBuf - 1, len - 1 iterations
    for p in 1..n {
        let (r0, r1, r2) = (re[p], re[p + 1], re[p + 2]);
        let (i0, i1, i2) = (im[p], im[p + 1], im[p + 2]);
        accu1 = accu1.add(r0.mulhi(r0).add(i0.mulhi(i0)).sra(ls));
        accu3 = accu3.add(r0.mulhi(r1).add(i0.mulhi(i1)).sra(ls));
        accu5 = accu5.add(i1.mulhi(r0).sub(r1.mulhi(i0)).sra(ls));
        accu7 = accu7.add(r2.mulhi(r0).add(i2.mulhi(i0)).sra(ls));
        accu8 = accu8.add(i2.mulhi(r0).sub(r2.mulhi(i0)).sra(ls));
    }
    let (rm2, rm1, rl2, rl1) = (re[0], re[1], re[n], re[n + 1]);
    let (im2, im1, il2, il1) = (im[0], im[1], im[n], im[n + 1]);
    let mut accu2 = rm2.mulhi(rm2).add(im2.mulhi(im2)).sra(ls);
    accu2 = accu2.add(accu1);
    accu1 = accu1.add(rl2.mulhi(rl2).add(il2.mulhi(il2)).sra(ls));
    let mut accu0 =
        rl1.mulhi(rl1).add(il1.mulhi(il1)).sra(ls).sub(rm1.mulhi(rm1).add(im1.mulhi(im1)).sra(ls));
    accu0 = accu0.add(accu1);
    let mut accu4 = rm1.mulhi(rm2).add(im1.mulhi(im2)).sra(ls);
    accu4 = accu4.add(accu3);
    accu3 = accu3.add(rl1.mulhi(rl2).add(il1.mulhi(il2)).sra(ls));
    let mut accu6 = im1.mulhi(rm2).sub(rm1.mulhi(im2)).sra(ls);
    accu6 = accu6.add(accu5);
    accu5 = accu5.add(il1.mulhi(rl2).sub(rl1.mulhi(il2)).sra(ls));

    let one = V::splat(1);
    let m31 = V::splat(31);
    let all = accu0
        .or(accu1)
        .or(accu2)
        .or(accu3.abs())
        .or(accu4.abs())
        .or(accu5.abs())
        .or(accu6.abs())
        .or(accu7.abs())
        .or(accu8.abs());
    let m_scale = all.clz().sub(one);
    let scaling = m_scale.sub(one).sub(V::splat(ls));
    // shifts by mScale modulo 32
    let sh = m_scale.and(m31);
    let r00r = accu0.shl_lanes(sh);
    let r11r = accu1.shl_lanes(sh);
    let r22r = accu2.shl_lanes(sh);
    let r01r = accu3.shl_lanes(sh);
    let r12r = accu4.shl_lanes(sh);
    let r01i = accu5.shl_lanes(sh);
    let r12i = accu6.shl_lanes(sh);
    let r02r = accu7.shl_lanes(sh);
    let r02i = accu8.shl_lanes(sh);

    let det = r11r.mulhi(r22r).sra(1).sub(r12r.mulhi(r12r).add(r12i.mulhi(r12i)).sra(1));
    let m2 = det.abs().clz().sub(one);
    let det = det.shl_lanes(m2.and(m31));
    let det_scale = m2.sub(V::splat(2));
    AcorrLanes {
        values: [r00r, r11r, r22r, r01r, r02r, r12r, r01i, r02i, r12i, det, det_scale, scaling],
    }
}

/// Four lanes of NEON.
#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
pub(super) struct Neon(core::arch::aarch64::int32x4_t);

#[cfg(target_arch = "aarch64")]
impl Lanes for Neon {
    const N: usize = 4;
    #[inline(always)]
    unsafe fn splat(x: i32) -> Self {
        Neon(core::arch::aarch64::vdupq_n_s32(x))
    }
    #[inline(always)]
    unsafe fn load(p: *const i32) -> Self {
        Neon(core::arch::aarch64::vld1q_s32(p))
    }
    #[inline(always)]
    unsafe fn store(self, p: *mut i32) {
        core::arch::aarch64::vst1q_s32(p, self.0)
    }
    #[inline(always)]
    unsafe fn add(self, b: Self) -> Self {
        Neon(core::arch::aarch64::vaddq_s32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn sub(self, b: Self) -> Self {
        Neon(core::arch::aarch64::vsubq_s32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn or(self, b: Self) -> Self {
        Neon(core::arch::aarch64::vorrq_s32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn xor(self, b: Self) -> Self {
        Neon(core::arch::aarch64::veorq_s32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn sra(self, n: i32) -> Self {
        use core::arch::aarch64::*;
        Neon(vshlq_s32(self.0, vdupq_n_s32(-n)))
    }
    #[inline(always)]
    unsafe fn mulhi(self, b: Self) -> Self {
        use core::arch::aarch64::*;
        let lo = vmull_s32(vget_low_s32(self.0), vget_low_s32(b.0));
        let hi = vmull_high_s32(self.0, b.0);
        Neon(vuzp2q_s32(vreinterpretq_s32_s64(lo), vreinterpretq_s32_s64(hi)))
    }
    #[inline(always)]
    unsafe fn abs(self) -> Self {
        Neon(core::arch::aarch64::vabsq_s32(self.0))
    }
    #[inline(always)]
    unsafe fn clz(self) -> Self {
        Neon(core::arch::aarch64::vclzq_s32(self.0))
    }
    #[inline(always)]
    unsafe fn shl_lanes(self, cnt: Self) -> Self {
        Neon(core::arch::aarch64::vshlq_s32(self.0, cnt.0))
    }
    #[inline(always)]
    unsafe fn min(self, b: Self) -> Self {
        Neon(core::arch::aarch64::vminq_s32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn max(self, b: Self) -> Self {
        Neon(core::arch::aarch64::vmaxq_s32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn and(self, b: Self) -> Self {
        Neon(core::arch::aarch64::vandq_s32(self.0, b.0))
    }
}

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Eight lanes of AVX2 (only used from functions compiled with AVX2).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[derive(Clone, Copy)]
pub(super) struct Avx2(__m256i);

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
impl Lanes for Avx2 {
    const N: usize = 8;
    #[inline(always)]
    unsafe fn splat(x: i32) -> Self {
        Avx2(_mm256_set1_epi32(x))
    }
    #[inline(always)]
    unsafe fn load(p: *const i32) -> Self {
        Avx2(_mm256_loadu_si256(p as *const __m256i))
    }
    #[inline(always)]
    unsafe fn store(self, p: *mut i32) {
        _mm256_storeu_si256(p as *mut __m256i, self.0)
    }
    #[inline(always)]
    unsafe fn add(self, b: Self) -> Self {
        Avx2(_mm256_add_epi32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn sub(self, b: Self) -> Self {
        Avx2(_mm256_sub_epi32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn or(self, b: Self) -> Self {
        Avx2(_mm256_or_si256(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn xor(self, b: Self) -> Self {
        Avx2(_mm256_xor_si256(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn sra(self, n: i32) -> Self {
        Avx2(_mm256_sra_epi32(self.0, _mm_cvtsi32_si128(n)))
    }
    #[inline(always)]
    unsafe fn mulhi(self, b: Self) -> Self {
        let (a, b) = (self.0, b.0);
        let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
        let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
        Avx2(_mm256_blend_epi32::<0b1010_1010>(even, odd))
    }
    #[inline(always)]
    unsafe fn abs(self) -> Self {
        Avx2(_mm256_abs_epi32(self.0))
    }
    #[inline(always)]
    unsafe fn clz(self) -> Self {
        // 0 <= a < 2^31: the exponent of the float conversion of a & !(a >> 1) (clearing the bit
        // below the leading one keeps the rounding from carrying into the next power of two),
        // 32 for 0; lanes with bit 31 set have no leading zeros.
        let a = self.0;
        let y = _mm256_andnot_si256(_mm256_srli_epi32::<1>(a), a);
        let e = _mm256_srli_epi32::<23>(_mm256_castps_si256(_mm256_cvtepi32_ps(y)));
        let c =
            _mm256_min_epi32(_mm256_sub_epi32(_mm256_set1_epi32(158), e), _mm256_set1_epi32(32));
        Avx2(_mm256_blendv_epi8(c, _mm256_setzero_si256(), _mm256_srai_epi32::<31>(a)))
    }
    #[inline(always)]
    unsafe fn shl_lanes(self, cnt: Self) -> Self {
        Avx2(_mm256_sllv_epi32(self.0, cnt.0))
    }
    #[inline(always)]
    unsafe fn min(self, b: Self) -> Self {
        Avx2(_mm256_min_epi32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn max(self, b: Self) -> Self {
        Avx2(_mm256_max_epi32(self.0, b.0))
    }
    #[inline(always)]
    unsafe fn and(self, b: Self) -> Self {
        Avx2(_mm256_and_si256(self.0, b.0))
    }
}

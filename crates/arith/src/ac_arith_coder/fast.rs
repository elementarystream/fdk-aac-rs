// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized version of decode2() of libArithCoding/src/ac_arith_coder.cpp.

//! The arithmetic decoder of [`super::decode2`] (an optimization of the port; the straight port
//! is [`super::decode2_straight`]). The decoder is latency bound: the symbol search, the interval
//! update, the renormalization, the bit reader and the next context form one dependency chain.
//!
//! * **MSB symbols** (17 entry tables): the first entry is tested first (the symbol is 0 in
//!   about half of the cases), the rest of the binary search of the C code is replaced by a
//!   count of the table entries `cf` with `cf * range > c`. The rows of `ari_pk` are
//!   non-increasing and `range >= 1`, so these comparisons form a prefix of ones and the binary
//!   search returns their count (entry 16 is 0 and never compared). The count is vectorized
//!   (SSE2 on x86, NEON on AArch64) for `range < 2^16`, where the products of the 16 bit
//!   entries and the range are below `2^30` and exact in 32 bits; `range == 2^16` (`low == 0`,
//!   `high == 0xFFFF`) takes the search of the C code (as would any other range).
//! * **LSB symbols** (4 entry tables): the count of the three compared entries, branch free;
//!   the row is prefixed by `2^14`, so that `high` is updated without a branch for symbol 0
//!   (`(range * 2^14) >> 14 == range` keeps `high = low + range - 1`).
//! * **Renormalization**: the closed form of [`super::ari_renormalize`] with the E3 toggles
//!   applied after the shift: toggling bits `14 .. 15 - n3` and shifting by `n3` leaves only the
//!   toggle of bit 15 within the 16 bits that are kept, and only if `n3 > 0`.
//! * **Context**: the context of a 2-tuple is `base + c_1` (plus `0x10000` for small
//!   amplitudes), where `base` depends on the context of the previous 2-tuple and the previous
//!   frame only. The 16 + 16 possible entries of the `get_pk_v2()` table are loaded one 2-tuple
//!   ahead, and the entry is selected in registers when `c_1` is known.
//! * **Signs**: branch free; reading 0 bits leaves the bitstream unchanged and negating 0 has no
//!   effect.
//!
//! Why `range >= 1`: the decoder starts with `low = 0`, `high = 0xFFFF` and a 16 bit `value`,
//! so `low <= value <= high`. The search chooses the symbol with `cf[s] * range <= c <
//! cf[s - 1] * range` for `c = ((value - low + 1) << 14) - 1`, which keeps `low <= value <=
//! high` for the new interval, and the renormalization shifts, or subtracts 0x4000 from, all
//! three alike. So `value` stays in the interval, which is never empty.

use super::*;
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;
#[cfg(all(target_arch = "x86", target_feature = "sse2"))]
use core::arch::x86::*;
#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
use core::arch::x86_64::*;

/// The row `row` with `2^14` prepended (see the module documentation).
const fn with_full_range<const N: usize, const M: usize>(row: [i16; N]) -> [i16; M] {
    let mut out = [0i16; M];
    out[0] = 1 << stat_bitsnew;
    let mut i = 0;
    while i < N {
        out[i + 1] = row[i];
        i += 1;
    }
    out
}

static ARI_LSB2_X: [[i16; 5]; 3] = [
    with_full_range::<4, 5>(ARI_LSB2[0]),
    with_full_range::<4, 5>(ARI_LSB2[1]),
    with_full_range::<4, 5>(ARI_LSB2[2]),
];

/// The number of the first 16 entries `cf` of `row` with `cf * range > c`, for `1 <= range <
/// 2^16` (the products of the 16 bit lanes are exact: `pmullw` gives the low and `pmulhuw` the
/// high half of the unsigned products, which are below `2^30`).
#[cfg(all(any(target_arch = "x86", target_arch = "x86_64"), target_feature = "sse2"))]
#[inline(always)]
pub(super) fn count_gt16(row: &[i16; 17], c: i32, range: i32) -> usize {
    debug_assert!((1..0x10000).contains(&range));
    // SAFETY: SSE2 is enabled for the target; the two loads read entries 0..16 of the row.
    unsafe {
        let r = _mm_set1_epi16(range as i16);
        let cv = _mm_set1_epi32(c);
        let lo = _mm_loadu_si128(row.as_ptr() as *const __m128i);
        let hi = _mm_loadu_si128(row.as_ptr().add(8) as *const __m128i);
        let (l0, l1) = (_mm_mullo_epi16(lo, r), _mm_mulhi_epu16(lo, r));
        let (h0, h1) = (_mm_mullo_epi16(hi, r), _mm_mulhi_epu16(hi, r));
        let p0 = _mm_unpacklo_epi16(l0, l1);
        let p1 = _mm_unpackhi_epi16(l0, l1);
        let p2 = _mm_unpacklo_epi16(h0, h1);
        let p3 = _mm_unpackhi_epi16(h0, h1);
        let m = _mm_packs_epi16(
            _mm_packs_epi32(_mm_cmpgt_epi32(p0, cv), _mm_cmpgt_epi32(p1, cv)),
            _mm_packs_epi32(_mm_cmpgt_epi32(p2, cv), _mm_cmpgt_epi32(p3, cv)),
        );
        // the comparison results are a prefix of ones
        (!(_mm_movemask_epi8(m) as u32)).trailing_zeros() as usize
    }
}

/// [`count_gt16`] for AArch64: `umull` gives the exact products, the comparison results (-1 or
/// 0 per lane) are added up.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub(super) fn count_gt16(row: &[i16; 17], c: i32, range: i32) -> usize {
    debug_assert!((1..0x10000).contains(&range));
    // SAFETY: NEON is part of AArch64; the two loads read entries 0..16 of the row.
    unsafe {
        let r = vdupq_n_u16(range as u16);
        let cv = vdupq_n_s32(c);
        let lo = vld1q_u16(row.as_ptr() as *const u16);
        let hi = vld1q_u16(row.as_ptr().add(8) as *const u16);
        let gt = |p: uint32x4_t| vcgtq_s32(vreinterpretq_s32_u32(p), cv);
        let m0 = gt(vmull_u16(vget_low_u16(lo), vget_low_u16(r)));
        let m1 = gt(vmull_high_u16(lo, r));
        let m2 = gt(vmull_u16(vget_low_u16(hi), vget_low_u16(r)));
        let m3 = gt(vmull_high_u16(hi, r));
        vaddvq_u32(vaddq_u32(vaddq_u32(m0, m1), vaddq_u32(m2, m3))).wrapping_neg() as usize
    }
}

/// [`count_gt16`] for other targets.
#[cfg(not(any(
    target_arch = "aarch64",
    all(any(target_arch = "x86", target_arch = "x86_64"), target_feature = "sse2")
)))]
#[inline(always)]
pub(super) fn count_gt16(row: &[i16; 17], c: i32, range: i32) -> usize {
    row[..16].iter().map(|&cf| (cf as i32 * range > c) as usize).sum()
}

/// `ari_decode_14bits(hBs, s, c_freq, VAL_ESC + 1)` (see the module documentation).
#[inline(always)]
fn decode_msb(h_bs: &mut FdkBitStream, s: &mut Tastat, row: &[i16; 17]) -> i32 {
    let low = s.low;
    let mut high = s.high;
    let value = s.vobf;
    let range = high - low + 1;
    let c = ((value - low + 1) << stat_bitsnew) - 1;
    let f = |i: usize| row[i] as i32 * range;
    let mut symbol = 0;
    if f(0) > c {
        if (1..0x10000).contains(&range) {
            symbol = count_gt16(row, c, range);
        } else {
            // the search of the C code: `p[k]` is `c_freq[p + k - 1]`, here `f(symbol + k - 1)`
            symbol = 1;
            if f(symbol + 7) > c {
                symbol += 8;
            }
            if f(symbol + 3) > c {
                symbol += 4;
            }
            if f(symbol + 1) > c {
                symbol += 2;
            }
            if f(symbol) > c {
                symbol += 1;
            }
        }
        high = low + mul_sbc_14bits(range, row[symbol - 1] as i32) - 1;
    }
    let low = low + mul_sbc_14bits(range, row[symbol] as i32);
    renormalize(h_bs, s, low as u16 as u32, high as u16 as u32, value as u32);
    symbol as i32
}

/// `ari_decode_14bits(hBs, s, c_freq, 4)` with the row of `ari_lsb2` with `2^14` prepended.
#[inline(always)]
fn decode_lsb(h_bs: &mut FdkBitStream, s: &mut Tastat, row: &[i16; 5]) -> i32 {
    let low = s.low;
    let high = s.high;
    let value = s.vobf;
    let range = high - low + 1;
    let c = ((value - low + 1) << stat_bitsnew) - 1;
    let f = |i: usize| row[i + 1] as i32 * range;
    debug_assert!(range >= 1);
    let symbol = (f(0) > c) as usize + (f(1) > c) as usize + (f(2) > c) as usize;
    let high = low + mul_sbc_14bits(range, row[symbol] as i32) - 1;
    let low = low + mul_sbc_14bits(range, row[symbol + 1] as i32);
    renormalize(h_bs, s, low as u16 as u32, high as u16 as u32, value as u32);
    symbol as i32
}

/// [`super::ari_renormalize`] with the toggles applied after the shift (see the module
/// documentation): `n1` shifts while `high` has bit 15 clear or `low` has it set, then `n3`
/// E3 steps; the bits of `low & !high` that the E3 steps test are those of `(low & !high) <<
/// n1` (the bits shifted into `low` are 0).
#[inline(always)]
pub(super) fn renormalize<B: AriBits>(
    h_bs: &mut B,
    s: &mut Tastat,
    low: u32,
    high: u32,
    value: u32,
) {
    let n1 = (!((!high | low) << 16)).leading_zeros(); // 0..=16
    let n3 = (!(((low & !high) << n1) << 17)).leading_zeros(); // 0..=15
    let n = n1 + n3;
    let t15 = ((n3 != 0) as u32) << 15;
    let bits = h_bs.bits(n);
    s.low = (((low << n) & 0xFFFF) ^ t15) as i32;
    s.high = ((((high << n) | ((1 << n) - 1)) & 0xFFFF) ^ t15) as i32;
    s.vobf = ((((value << n) | bits) & 0xFFFF) ^ t15) as i32;
}

/// `decode2()` (see the module documentation). Requires `c_prev.len() > max(n, 0)`,
/// `p_spectral_coefficient.len() >= 2 * max(n, 0)` and `c_prev.len() >= nt`; the caller checks.
#[inline(always)]
pub(super) fn decode2_impl(
    bbuf: &mut FdkBitStream,
    c_prev: &mut [u8],
    p_spectral_coefficient: &mut [FixpDbl],
    n: i32,
    nt: i32,
) -> ArithCodingError {
    let mut r: i32;

    let mut c_3: i32 = 0; /* context of current frame 3 time steps ago */
    let mut c_2: i32 = 0; /* context of current frame 2 time steps ago */
    let mut c_1: i32 = 0; /* context of current frame 1 time steps ago */
    let mut c_0: i32; /* context of current frame to be calculated */

    /* ari_start_decoding_14bits */
    let mut as_ =
        Tastat { low: 0, high: ari_q4new, vobf: fdk_read_bits(bbuf, cbitsnew as u32) as i32 };

    /* arith_map_context */
    let mut state_inc: u16 = ((c_prev[0] as i32) << 12) as u16;
    let pk = pk_table();

    let nu = n.max(0) as usize;
    let (c_prev_n, spec_n) = (&mut c_prev[..nu + 1], &mut p_spectral_coefficient[..2 * nu]);

    // The table entries for the contexts base + 0..16 and base + 0x10000 + 0..16, or None for
    // contexts beyond the table (only invalid contexts of the previous frame give them).
    let chunks = |base: usize| -> Option<[u64; 4]> {
        if base + 0x10010 > PK_TABLE_LEN {
            return None;
        }
        let w = |p: usize| u64::from_le_bytes(pk[p..p + 8].try_into().unwrap());
        Some([w(base), w(base + 8), w(base + 0x10000), w(base + 0x10008)])
    };
    let mut next = chunks(((state_inc >> 8) as usize + ((c_prev_n[1.min(nu)] as usize) << 8)) << 4);

    let mut i: i32 = 0;
    while i < n {
        let iu = i as usize;
        /* arith_get_context */
        let mut s: u32 = (state_inc >> 8) as u32;
        s += (c_prev_n[iu + 1] as u32) << 8;
        s = (s << 4) + c_1 as u32;

        state_inc = s as u16;

        /* Cumulative amplitude below 2 */
        let small = i > 3 && (c_1 + c_2 + c_3) < 5;
        if small {
            s += 0x10000;
        }

        // get_pk_v2(s) from the entries loaded for this 2-tuple, then the loads for the next.
        let mut pki = match next {
            Some(ch) => {
                let (lo, hi) = if small { (ch[2], ch[3]) } else { (ch[0], ch[1]) };
                let word = if c_1 < 8 { lo } else { hi };
                (word >> (8 * (c_1 & 7))) as u32 & 0xFF
            }
            None => get_pk_v2_fast(s),
        };
        debug_assert_eq!(pki, get_pk_v2(s));
        next =
            chunks(((state_inc >> 8) as usize + ((c_prev_n[(iu + 2).min(nu)] as usize) << 8)) << 4);

        /* MSBs decoding */
        let mut lev: i32 = 0;
        let mut esc_nb: i32 = 0;
        loop {
            r = decode_msb(bbuf, &mut as_, &ARI_PK[pki as usize & 0x3F]);
            if r < VAL_ESC {
                break;
            }

            lev += 1;

            if lev > 23 {
                return ARITH_CODER_ERROR;
            }

            if esc_nb < 7 {
                esc_nb += 1;
            }
            pki = get_pk_v2_fast(s.wrapping_add((esc_nb << (VAL_ESC + 1)) as u32));
        }

        /* Stop symbol */
        if r == 0 {
            if esc_nb > 0 {
                break; /* Stop symbol */
            }
            c_0 = 1;
        } else {
            let mut b = r >> 2;
            let mut a = r & 0x3;

            /* LSBs decoding */
            for _ in 0..lev {
                let pidx = if a == 0 {
                    1
                } else if b == 0 {
                    0
                } else {
                    2
                };
                r = decode_lsb(bbuf, &mut as_, &ARI_LSB2_X[pidx]);
                a = (a << 1) | (r & 1);
                b = (b << 1) | (r >> 1);
            }

            spec_n[2 * iu] = a as FixpDbl;
            spec_n[2 * iu + 1] = b as FixpDbl;

            c_0 = (a + b + 1).min(0xF);
        }

        /* arith_update_context */
        c_3 = c_2;
        c_2 = c_1;
        c_1 = c_0;
        c_prev_n[iu] = c_0 as u8;

        i += 1;
    }

    fdk_push_back(bbuf, (cbitsnew - 2) as u32);

    /* We need to run only from 0 to i-1 since all other q[i][1].a,b will be cleared later */
    let j = i as usize;
    // Branch-free form of: bits = (a != 0) + (b != 0); if bits != 0 { r = FDKreadBits(bits);
    // if a != 0 && (r >> (bits - 1)) == 0 { a = -a } if b != 0 && (r & 1) == 0 { b = -b } }.
    for q in spec_n[..2 * j].chunks_exact_mut(2) {
        let bits = (q[0] != 0) as u32 + (q[1] != 0) as u32;
        let r = fdk_read_bits(bbuf, bits);
        if (r >> (bits.wrapping_sub(1) & 31)) & 1 == 0 {
            q[0] = q[0].wrapping_neg();
        }
        if r & 1 == 0 {
            q[1] = q[1].wrapping_neg();
        }
    }

    if nt as usize > j && nt > i {
        c_prev[j..nt as usize].fill(1);
    }

    ARITH_CODER_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pat {
        pat: u64,
        pos: u32,
    }

    impl AriBits for Pat {
        fn bit(&mut self) -> u32 {
            let b = (self.pat >> (63 - self.pos)) & 1;
            self.pos += 1;
            b as u32
        }
        fn bits(&mut self, n: u32) -> u32 {
            if n == 0 {
                return 0;
            }
            let b = (self.pat >> (64 - self.pos - n)) & ((1u64 << n) - 1);
            self.pos += n;
            b as u32
        }
    }

    fn check(low: u16, high: u16, value: i32, pat: u64) {
        let (mut a, mut b) = (Tastat::default(), Tastat::default());
        let (mut pa, mut pb) = (Pat { pat, pos: 0 }, Pat { pat, pos: 0 });
        ari_renormalize_straight(&mut pa, &mut a, low, high, value);
        // the callers pass the 16 bit value of the decoder state
        renormalize(&mut pb, &mut b, low as u32, high as u32, (value & 0xFFFF) as u32);
        assert!(
            a == b && pa.pos == pb.pos,
            "low {low:#06x} high {high:#06x} value {value:#x}: {a:?}/{} vs {b:?}/{}",
            pa.pos,
            pb.pos
        );
    }

    /// The toggle-after-shift renormalization against the bit-by-bit loop of the C code.
    #[test]
    fn renormalize_equals_loop_sample() {
        let mut x: u64 = 0x0123_4567_89AB_CDEF;
        for _ in 0..2_000_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            for pat in [0u64, !0u64, x.rotate_left(29)] {
                check(x as u16, (x >> 16) as u16, (x >> 32) as i32, pat);
            }
        }
        // every (low, high) pair of the E3 window region
        for low in (0x3F00u32..0x4200).chain(0x7F00..0x8100) {
            for high in (0x7E00u32..0x8200).chain(0xBE00..0xC200) {
                check(low as u16, high as u16, 0x5A5A, 0x9E37_79B9_7F4A_7C15);
            }
        }
    }

    #[test]
    #[ignore = "exhaustive (a few minutes in release)"]
    fn renormalize_equals_loop_exhaustive() {
        for high in 0..=0xFFFFu32 {
            for low in 0..=0xFFFFu32 {
                let h = (low << 16 | high).wrapping_mul(0x9E37_79B9);
                let pat = ((h as u64) << 32 | (high << 16 | low) as u64)
                    .wrapping_mul(0x2545_F491_4F6C_DD1D);
                check(low as u16, high as u16, (h ^ (h >> 13)) as i32, pat);
            }
        }
    }

    /// The binary search of `ari_decode_14bits()` for 17 symbol tables (after the test of the
    /// first entry), as in C.
    fn search(row: &[i16; 17], c: i32, range: i32) -> usize {
        let f = |i: usize| row[i] as i32 * range;
        let mut p = 0;
        if f(p) > c {
            p = 1;
            for k in [8, 4, 2, 1] {
                if f(p + k - 1) > c {
                    p += k;
                }
            }
        }
        p
    }

    #[test]
    fn rows_are_non_increasing() {
        for row in ARI_PK.iter() {
            assert!(row.windows(2).all(|w| w[0] >= w[1]) && row[16] == 0, "{row:?}");
            assert!(row.iter().all(|&v| (0..1 << 14).contains(&v)));
        }
        for row in ARI_LSB2.iter() {
            assert!(row.windows(2).all(|w| w[0] >= w[1]) && row[3] == 0, "{row:?}");
        }
    }

    /// The vectorized count equals the binary search for every row, every range below 2^16 and
    /// the values of `c` at and next to every product (the boundaries of the comparisons).
    #[test]
    fn count_equals_search() {
        for row in ARI_PK.iter() {
            for range in (1..0x10000).step_by(7).chain(0xFFF0..0x10000) {
                for &cf in row.iter() {
                    let p = cf as i32 * range;
                    for c in [p - 1, p, p + 1, 0x3FFF, (range << 14) - 1] {
                        if row[0] as i32 * range > c {
                            assert_eq!(
                                count_gt16(row, c, range),
                                search(row, c, range),
                                "{range} {c}"
                            );
                        }
                    }
                }
            }
        }
    }
}

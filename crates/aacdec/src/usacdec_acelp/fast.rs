// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized versions of Syn_filt(), Deemph(), BuildAdaptiveExcitation() and the energy loop of
// calc_period_factor() of libAACdec/src/usacdec_acelp.cpp.

//! Optimizations of the ACELP decoder (the straight ports are [`super::syn_filt_ref`],
//! [`super::deemph_ref`], [`super::build_adaptive_excitation_ref`] and
//! [`super::calc_period_factor_ref`]).
//!
//! **`Syn_filt()`** with a short recursion: output `i` is
//! `fAddSaturate(scaleValue(-sum_j t_j, a_exp + LP_FILTER_SCALE), x[i])` with
//! `t_j = fMultDiv2(a[j], y[i - 1 - j]) >> (LP_FILTER_SCALE - 1)`, which is
//! `(a[j] * y[i - 1 - j]) >> 19` on every variant (the floor of a floor is the floor of the
//! quotient). The sum wraps, so its order does not matter: here the 15 terms of the older outputs
//! are summed first and the term of `y[i - 1]` last, so that only one product, one subtraction,
//! the scaling and the saturation lie between two outputs (the straight order puts all 16
//! subtractions there). `y[i - 1]` is kept in a register. The shift direction of `scaleValue()`
//! is the same for every output and chosen once; its amount is taken modulo 32 as in the
//! straight port.
//!
//! **`Deemph()`**: `SATURATE_LEFT_SHIFT(x, 1, 32)` is the saturating doubling `x + x` in both
//! forms of the macro (`arm/scale_arm.h`: `|x|' > MAXVAL_DBL >> 1` gives `sign ^ MAXVAL_DBL`;
//! generic: `x > MAXVAL_DBL >> 1` gives `MAXVAL_DBL`, `x < -2^30` gives `MINVAL_DBL`), which the
//! compiler evaluates with the overflow flag of one addition, a shorter recursion.
//!
//! **`BuildAdaptiveExcitation()`**: the loop of the straight port carries values from one
//! iteration to the next, but each is a function of the inputs: with `t[i] = fMultDiv2(exc[i],
//! gain_pit) << 1`, `cs[i] = fMultDiv2(code[i], gain_code_smoothed) << SF` and `ccs[i] =
//! fMultDiv2(cpe, cs[i])` it writes `exc[i] = (t[i] + (fMultDiv2(code[i], gain_code) << SF)) << 1`
//! and `exc2[i] = (t[i] + cs[i] - ccs[i - 1] - ccs[i + 1]) << 1` (the terms outside of `0..64`
//! omitted), with wrapping sums, whose order does not matter. Computed in that form, the loops
//! have independent iterations. Every product is the same on every variant.
//!
//! **Energy of the pitch excitation** (`calc_period_factor()`): the terms
//! `fPow2Div2(exc[i]) = (exc[i]^2) >> 32` are not negative, so the running sum only grows. If the
//! sum of all 64 terms (in 64 bits) is below `0.5` (`2^30`), the halving of the straight loop is
//! never taken (`s` stays 0) and its 32-bit sum equals this sum; otherwise the straight loop runs.

use super::*;

/// `(a * y) >> 19`: `fMultDiv2(FIXP_LPC a, FIXP_DBL y) >> (LP_FILTER_SCALE - 1)`.
#[inline(always)]
fn term(a: i32, y: FixpDbl) -> i32 {
    ((a as i64 * y as i64) >> (16 + LP_FILTER_SCALE - 1)) as i32
}

/// See the module documentation; the straight port where the accesses leave the buffers.
pub(super) fn syn_filt(
    a: &[FixpLpc],
    a_exp: i32,
    length: i32,
    x: &[FixpDbl],
    y_buf: &mut [FixpDbl],
    y_off: usize,
) {
    let n = length.max(0) as usize;
    if a.len() < M || y_off < M || y_off + n > y_buf.len() || x.len() < n {
        return syn_filt_ref(a, a_exp, length, x, y_buf, y_off);
    }
    let sf = a_exp + LP_FILTER_SCALE;
    let y = &mut y_buf[y_off - M..y_off + n];
    let x = &x[..n];
    if sf > 0 {
        kernel::<true>(a, sf as u32, x, y);
    } else {
        kernel::<false>(a, sf.wrapping_neg() as u32, x, y);
    }
}

/// `x` unchanged, through an empty `asm!` block that the compiler cannot look into: it keeps the
/// compiler from reassociating the sum of the older terms with the term of `y[i - 1]`, which
/// would put more operations on the recursion. No instruction is emitted.
#[inline(always)]
fn opaque(x: i32) -> i32 {
    #[allow(unused_mut)]
    let mut x = x;
    // SAFETY: empty assembly blocks (comments) that only name the register of x.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("/* {0:w} */", inout(reg) x, options(pure, nomem, nostack))
    };
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    unsafe {
        core::arch::asm!("/* {0:e} */", inout(reg) x, options(pure, nomem, nostack))
    };
    x
}

/// `y[..M]` is the filter memory, `y[M..]` the output; `scaleValue()` shifts left by `sh` for
/// `LEFT`, otherwise right by `sh` (both modulo 32).
#[inline(always)]
fn kernel<const LEFT: bool>(a: &[FixpLpc], sh: u32, x: &[FixpDbl], y: &mut [FixpDbl]) {
    let mut c = [0i32; M];
    for (c, &a) in c.iter_mut().zip(a) {
        *c = a as i32;
    }
    let mut prev = y[M - 1];
    for (i, &xi) in x.iter().enumerate() {
        // w[M - 1 - j] = y[i - 1 - j]
        let w: &[FixpDbl; M] = (&y[i..i + M]).try_into().unwrap();
        let mut p: i32 = 0;
        for j in (1..M).rev() {
            p = p.wrapping_sub(term(c[j], w[M - 1 - j]));
        }
        let l_tmp = if LEFT {
            // (p - t0) << sh == (p << sh) - (t0 << sh) modulo 2^32: the shift of p leaves the
            // recursion
            opaque(p.wrapping_shl(sh)).wrapping_sub(term(c[0], prev).wrapping_shl(sh))
        } else {
            opaque(p).wrapping_sub(term(c[0], prev)).wrapping_shr(sh)
        };
        let v = f_add_saturate(l_tmp, xi);
        y[i + M] = v;
        prev = v;
    }
}

/// `Deemph(x, y, L, mem)` (see the module documentation); the straight port where the accesses
/// leave the buffers.
pub(super) fn deemph(x: &[FixpDbl], y: &mut [FixpDbl], l: i32, mem: &mut FixpDbl) {
    let n = l.max(0) as usize;
    if x.len() < n || y.len() < n {
        return deemph_ref(x, y, l, mem);
    }
    let fac = preemph_fac();
    let mut yi = *mem;
    for (o, &xv) in y[..n].iter_mut().zip(&x[..n]) {
        let xi = f_mult_add_div2(xv >> 1, fac, yi);
        yi = xi.saturating_add(xi);
        *o = yi;
    }
    *mem = yi;
}

/// `BuildAdaptiveExcitation()` with independent iterations (see the module documentation); the
/// straight port where the accesses leave the buffers.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_adaptive_excitation(
    code: &[FixpCod; L_SUBFR_U],
    exc_buf: &mut [FixpDbl],
    exc_off: usize,
    gain_pit: FixpSgl,
    gain_code: FixpDbl,
    gain_code_smoothed: FixpDbl,
    period_fac: FixpDbl,
    exc2: &mut [FixpDbl],
) {
    const SF_HEADROOM: i32 = 1;
    const SF: i32 = SF_CODE + SF_GAIN_C + 1 - SF_EXC - SF_HEADROOM;
    const SF_GAIN_P2: i32 = SF_GAIN_P - SF_HEADROOM;
    if exc_off.checked_add(L_SUBFR_U).is_none_or(|e| e > exc_buf.len()) || exc2.len() < L_SUBFR_U {
        return build_adaptive_excitation_ref(
            code,
            exc_buf,
            exc_off,
            gain_pit,
            gain_code,
            gain_code_smoothed,
            period_fac,
            exc2,
        );
    }
    let exc: &mut [FixpDbl; L_SUBFR_U] =
        (&mut exc_buf[exc_off..exc_off + L_SUBFR_U]).try_into().unwrap();
    let exc2: &mut [FixpDbl; L_SUBFR_U] = (&mut exc2[..L_SUBFR_U]).try_into().unwrap();

    // cpe = (1+r_v)/8 * 2 ; ( SF = -1)
    let cpe = (period_fac >> (2 - SF_PFAC)).wrapping_add(fl2fxconst_dbl(0.25));

    let mut t = [0 as FixpDbl; L_SUBFR_U];
    let mut cs = [0 as FixpDbl; L_SUBFR_U];
    let mut ccs = [0 as FixpDbl; L_SUBFR_U];
    for i in 0..L_SUBFR_U {
        t[i] = f_mult_div2(exc[i], gain_pit) << (SF_GAIN_P2 + 1); // v(i)*g_p
        cs[i] = f_mult_div2(code[i], gain_code_smoothed) << SF; // c(i) * g_sc
        ccs[i] = f_mult_div2(cpe, cs[i]);
        exc[i] = t[i].wrapping_add(f_mult_div2(code[i], gain_code) << SF) << SF_HEADROOM;
    }
    exc2[0] = t[0].wrapping_add(cs[0]).wrapping_sub(ccs[1]) << SF_HEADROOM;
    for i in 1..L_SUBFR_U - 1 {
        exc2[i] = t[i].wrapping_add(cs[i]).wrapping_sub(ccs[i - 1]).wrapping_sub(ccs[i + 1])
            << SF_HEADROOM;
    }
    let l = L_SUBFR_U - 1;
    exc2[l] = t[l].wrapping_add(cs[l]).wrapping_sub(ccs[l - 1]) << SF_HEADROOM;
}

/// The energy of the pitch excitation and its scaling of `calc_period_factor()` (see the module
/// documentation).
pub(super) fn pitch_energy(exc: &[FixpDbl]) -> (FixpDbl, i32) {
    let total: i64 = exc[..L_SUBFR_U].iter().map(|&v| f_pow2_div2(v) as i64).sum();
    if total < fl2fxconst_dbl(0.5) as i64 {
        (total as FixpDbl, 0)
    } else {
        pitch_energy_ref(exc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u32) as i32
        }
        fn val(&mut self, mode: u32) -> i32 {
            match mode {
                0 => [i32::MIN, i32::MIN + 1, i32::MAX, i32::MAX - 1, 0, -1, 1 << 30, -(1 << 30)]
                    [(self.next() % 8) as usize],
                1 => self.next() as i32,
                n => (self.next() as i32) >> (n * 3),
            }
        }
        fn any(&mut self) -> i32 {
            let m = self.next() % 6;
            self.val(m)
        }
    }

    /// Against the straight port: random and extreme coefficients, memories and inputs, every
    /// exponent (also outside of the range the decoder produces), lengths and offsets, and
    /// buffers too short for the fast path.
    #[test]
    fn syn_filt_matches_straight() {
        let mut r = Rng(0x2468_1357_fedc_ba98);
        for it in 0..60_000 {
            let mode = r.next() % 6;
            let a: Vec<i16> = (0..M)
                .map(|_| match r.next() % 4 {
                    0 => [i16::MIN, i16::MAX, 0, -1][(r.next() % 4) as usize],
                    1 => r.next() as i16,
                    _ => (r.next() as i16) >> r.range(0, 8),
                })
                .collect();
            let a_exp = if it % 4 == 0 { r.range(-40, 40) } else { r.range(-4, 4) };
            let length = if it % 8 == 0 { r.range(-2, 70) } else { 64 };
            let len = if it % 16 == 1 { r.range(0, 100) as usize } else { 272 };
            let y_off = if it % 16 == 2 {
                r.range(0, 30) as usize
            } else {
                16 + 64 * (r.next() % 4) as usize
            };
            let mut buf = Vec::with_capacity(len);
            for _ in 0..len {
                let m = if mode == 5 { r.next() % 5 } else { mode };
                buf.push(r.val(m));
            }
            let mut x = Vec::with_capacity(64);
            for _ in 0..64 {
                let m = r.next() % 5;
                x.push(r.val(m));
            }
            let fast = std::panic::catch_unwind(|| {
                let mut y = buf.clone();
                syn_filt(&a, a_exp, length, &x, &mut y, y_off);
                y
            });
            let straight = std::panic::catch_unwind(|| {
                let mut y = buf.clone();
                syn_filt_ref(&a, a_exp, length, &x, &mut y, y_off);
                y
            });
            match (fast, straight) {
                (Ok(p), Ok(q)) => {
                    assert_eq!(p, q, "it {it} a_exp {a_exp} length {length} off {y_off}")
                }
                (Err(_), Err(_)) => {}
                (p, q) => panic!("it {it}: fast {:?} straight {:?}", p.is_ok(), q.is_ok()),
            }
        }
    }

    fn vec(r: &mut Rng, n: usize) -> Vec<i32> {
        let mode = r.next() % 7;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let m = if mode == 6 { r.next() % 6 } else { mode };
            v.push(r.val(m));
        }
        v
    }

    fn sgl(r: &mut Rng) -> i16 {
        match r.next() % 4 {
            0 => [i16::MIN, i16::MAX, 0, -1, 1][(r.next() % 5) as usize],
            1 => r.next() as i16,
            _ => (r.next() as i16) >> r.range(0, 12),
        }
    }

    /// Deemph against the straight port, with every memory, random and extreme inputs, and short
    /// buffers.
    #[test]
    fn deemph_matches_straight() {
        let mut r = Rng(0x1111_2222_3333_4444);
        for it in 0..40_000 {
            let n = if it % 8 == 0 { r.range(0, 300) as usize } else { 256 };
            let x = vec(&mut r, n);
            let l = if it % 8 == 1 { r.range(-1, 300) } else { n as i32 };
            let mem = r.any();
            let ylen = if it % 8 == 2 { r.range(0, 300) as usize } else { n };
            let fast = std::panic::catch_unwind(|| {
                let (mut y, mut m) = (vec![0; ylen], mem);
                deemph(&x, &mut y, l, &mut m);
                (y, m)
            });
            let straight = std::panic::catch_unwind(|| {
                let (mut y, mut m) = (vec![0; ylen], mem);
                deemph_ref(&x, &mut y, l, &mut m);
                (y, m)
            });
            match (fast, straight) {
                (Ok(p), Ok(q)) => assert_eq!(p, q, "it {it}"),
                (Err(_), Err(_)) => {}
                (p, q) => panic!("it {it}: fast {:?} straight {:?}", p.is_ok(), q.is_ok()),
            }
        }
    }

    /// BuildAdaptiveExcitation against the straight port: random and extreme codes, gains,
    /// excitations and period factors, every offset of the decoder's excitation buffer.
    #[test]
    fn build_adaptive_excitation_matches_straight() {
        let mut r = Rng(0x5555_6666_7777_8888);
        for it in 0..40_000 {
            let mut code = [0i16; L_SUBFR_U];
            for c in code.iter_mut() {
                *c = sgl(&mut r);
            }
            let len = if it % 16 == 0 { r.range(0, 700) as usize } else { 685 };
            let buf = vec(&mut r, len);
            let off = if it % 16 == 1 {
                r.range(0, 700) as usize
            } else {
                428 + 64 * (r.next() % 4) as usize
            };
            let gain_pit = sgl(&mut r);
            let gain_code = r.any();
            let gcs = r.any();
            let pf = r.any();
            let x2len = if it % 16 == 2 { r.range(0, 64) as usize } else { 64 };
            let fast = std::panic::catch_unwind(|| {
                let (mut e, mut x2) = (buf.clone(), vec![0; x2len]);
                build_adaptive_excitation(
                    &code, &mut e, off, gain_pit, gain_code, gcs, pf, &mut x2,
                );
                (e, x2)
            });
            let straight = std::panic::catch_unwind(|| {
                let (mut e, mut x2) = (buf.clone(), vec![0; x2len]);
                build_adaptive_excitation_ref(
                    &code, &mut e, off, gain_pit, gain_code, gcs, pf, &mut x2,
                );
                (e, x2)
            });
            match (fast, straight) {
                (Ok(p), Ok(q)) => assert_eq!(p, q, "it {it}"),
                (Err(_), Err(_)) => {}
                (p, q) => panic!("it {it}: fast {:?} straight {:?}", p.is_ok(), q.is_ok()),
            }
        }
    }

    /// calc_period_factor against the straight port, with excitations around the energy limit
    /// of the fast path (sums just below and above 0.5) and random gains.
    #[test]
    fn calc_period_factor_matches_straight() {
        let mut r = Rng(0x9999_aaaa_bbbb_cccc);
        let mut fast_path = 0;
        for it in 0..60_000 {
            let exc: Vec<i32> = match it % 4 {
                // sqrt(2^30 / 64 * 2^32) = 2^29: values near it put the sum near 0.5
                0 => (0..64).map(|_| (1 << 29) + r.range(-(1 << 20), 1 << 20)).collect(),
                1 => (0..64).map(|_| r.range(-(1 << 29) - 4, (1 << 29) + 4)).collect(),
                _ => vec(&mut r, 64),
            };
            let gain_pit = sgl(&mut r);
            let gain_code = r.any();
            let ener_code = r.any();
            let ener_code_e = r.range(-40, 40);
            if pitch_energy(&exc).1 == 0 {
                fast_path += 1;
            }
            let p = calc_period_factor(&exc, gain_pit, gain_code, ener_code, ener_code_e);
            let q = calc_period_factor_ref(&exc, gain_pit, gain_code, ener_code, ener_code_e);
            assert_eq!(p, q, "it {it}");
            assert_eq!(pitch_energy(&exc), pitch_energy_ref(&exc), "it {it}");
        }
        assert!(fast_path > 10_000 && fast_path < 50_000, "{fast_path}");
    }
}

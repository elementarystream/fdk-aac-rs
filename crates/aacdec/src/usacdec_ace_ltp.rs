// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_ace_ltp.h, libAACdec/src/usacdec_ace_ltp.cpp.

//! Port of `usacdec_ace_ltp.h`: the USAC ACELP LTP filter (adaptive codebook).
//!
//! The preprocessed `usacdec_ace_ltp.cpp` of the `aarch64` and `x86` builds are identical; the
//! variants differ only in the primitives (`fMult`).
//!
//! The excitation pointer `exc` of C points into an excitation buffer and is accessed at
//! negative offsets. The port passes the buffer and the index of `exc[0]`. Accesses outside of
//! the buffer have no defined reference result; the port reads 0 and drops writes there.

use fdk_aac_rs_fdk::common_fix::{
    f_add_saturate, f_mult, f_mult_add_div2, f_mult_div2, fl2fx_sgl, FixpDbl, FixpSgl,
};

mod fast;

const UP_SAMP: i32 = 4;
const L_INTERPOL2: isize = 16;
const L_SUBFR: isize = 64;

/// `A2`: `FL2FX_SGL(2 * 0.18f)`
#[inline(always)]
fn a2() -> FixpSgl {
    fl2fx_sgl(2.0f32 * 0.18f32)
}

/// `B`: `FL2FX_SGL(0.64f)`
#[inline(always)]
fn b() -> FixpSgl {
    fl2fx_sgl(0.64f32)
}

/// `Pred_lt4_inter4_2[UP_SAMP][L_INTERPOL2]`
static PRED_LT4_INTER4_2: [[i32; L_INTERPOL2 as usize]; UP_SAMP as usize] = [
    [
        0x0000FFFCu32 as i32,
        0x0008FFFCu32 as i32,
        0xFFEB004Cu32 as i32,
        0xFF50014Au32 as i32,
        0xFDD90351u32 as i32,
        0xFB2A06CDu32 as i32,
        0xF6920D46u32 as i32,
        0xEBB42B35u32 as i32,
        0x6D9EEF39u32 as i32,
        0x0618FE0Fu32 as i32,
        0xFFE00131u32 as i32,
        0xFE5501C5u32 as i32,
        0xFE5E015Du32 as i32,
        0xFEF700B6u32 as i32,
        0xFF920037u32 as i32,
        0xFFEC0003u32 as i32,
    ],
    [
        0x0002FFF2u32 as i32,
        0x0026FFBDu32 as i32,
        0x005DFF98u32 as i32,
        0x0055FFEFu32 as i32,
        0xFF89015Fu32 as i32,
        0xFD3A04E5u32 as i32,
        0xF7D90DAAu32 as i32,
        0xE67A50EEu32 as i32,
        0x50EEE67Au32 as i32,
        0x0DAAF7D9u32 as i32,
        0x04E5FD3Au32 as i32,
        0x015FFF89u32 as i32,
        0xFFEF0055u32 as i32,
        0xFF98005Du32 as i32,
        0xFFBD0026u32 as i32,
        0xFFF20002u32 as i32,
    ],
    [
        0x0003FFECu32 as i32,
        0x0037FF92u32 as i32,
        0x00B6FEF7u32 as i32,
        0x015DFE5Eu32 as i32,
        0x01C5FE55u32 as i32,
        0x0131FFE0u32 as i32,
        0xFE0F0618u32 as i32,
        0xEF396D9Eu32 as i32,
        0x2B35EBB4u32 as i32,
        0x0D46F692u32 as i32,
        0x06CDFB2Au32 as i32,
        0x0351FDD9u32 as i32,
        0x014AFF50u32 as i32,
        0x004CFFEBu32 as i32,
        0xFFFC0008u32 as i32,
        0xFFFC0000u32 as i32,
    ],
    [
        0x0002FFF2u32 as i32,
        0x002BFF9Eu32 as i32,
        0x00B9FECEu32 as i32,
        0x01CFFD75u32 as i32,
        0x035EFBC1u32 as i32,
        0x0521FA0Cu32 as i32,
        0x06AAF8C9u32 as i32,
        0x07907852u32 as i32,
        0x0790F8C9u32 as i32,
        0x06AAFA0Cu32 as i32,
        0x0521FBC1u32 as i32,
        0x035EFD75u32 as i32,
        0x01CFFECEu32 as i32,
        0x00B9FF9Eu32 as i32,
        0x002BFFF2u32 as i32,
        0x00020000u32 as i32,
    ],
];

/// `exc[i]` with `exc[0]` at `buf[off]`; 0 outside of the buffer.
#[inline(always)]
fn rd(buf: &[FixpDbl], idx: isize) -> FixpDbl {
    if idx >= 0 {
        buf.get(idx as usize).copied().unwrap_or(0)
    } else {
        0
    }
}

/// `exc[i] = v` with `exc[0]` at `buf[off]`; dropped outside of the buffer.
#[inline(always)]
fn wr(buf: &mut [FixpDbl], idx: isize, v: FixpDbl) {
    if idx >= 0 {
        if let Some(p) = buf.get_mut(idx as usize) {
            *p = v;
        }
    }
}

/// `Pred_lt4(exc, T0, frac)`: compute the initial adaptive codebook excitation v'(n) by
/// interpolating the past excitation vector u'(n).
///
/// `exc` (`exc_buf[exc_off..]`) points to the adaptive codebook of the current subframe
/// (input/output); `L_SUBFR + 1` values are written. `t0` is the integer part of the decoded
/// pitch lag, `frac` its fractional part (0..3).
///
/// Computes several outputs at once with SIMD where that gives the same values (see
/// `fast`); otherwise, and on other targets, the straight port [`pred_lt4_ref`].
pub fn pred_lt4(exc_buf: &mut [FixpDbl], exc_off: usize, t0: i32, frac: i32) {
    if !fast::pred_lt4(exc_buf, exc_off, t0, frac) {
        pred_lt4_ref(exc_buf, exc_off, t0, frac);
    }
}

/// The remapped `frac` (the row of `Pred_lt4_inter4_2`) and the index of the first value read
/// (`x` of the C code) of `Pred_lt4(exc, T0, frac)`.
#[inline(always)]
fn pred_lt4_setup(exc_off: usize, t0: i32, frac: i32) -> (isize, usize) {
    let mut frac = frac;
    let mut x = exc_off as isize - t0 as isize - L_INTERPOL2 + 1;
    frac -= 1;
    if frac < 0 {
        frac += UP_SAMP;
    } else {
        x -= 1;
    }
    (x, (frac as usize).min(UP_SAMP as usize - 1))
}

/// The straight port of `Pred_lt4(exc, T0, frac)` (see [`pred_lt4`]).
pub fn pred_lt4_ref(exc_buf: &mut [FixpDbl], exc_off: usize, t0: i32, frac: i32) {
    let mut frac = frac;
    let mut x = exc_off as isize - t0 as isize - L_INTERPOL2 + 1;

    // remap frac and x:
    //   0 -> 3   x (unchanged)
    //   1 -> 0   x--
    //   2 -> 1   x--
    //   3 -> 2   x--
    frac -= 1;
    if frac < 0 {
        frac += UP_SAMP;
    } else {
        x -= 1;
    }
    // frac is in 0..=3 for every frac in 0..=3; other values (no defined reference result) are
    // clamped.
    let interpol_tab = &PRED_LT4_INTER4_2[(frac as usize).min(UP_SAMP as usize - 1)];

    let mut exc = exc_off as isize;
    let fast = x >= 0 && (x + L_SUBFR + 2 * L_INTERPOL2) as usize <= exc_buf.len();
    for _ in 0..(L_SUBFR + 1) {
        let mut xi = x;
        x += 1;
        let mut l_sumt;
        let mut l_sumb;
        if fast {
            let w = &exc_buf[xi as usize..xi as usize + 2 * L_INTERPOL2 as usize];
            let filt = interpol_tab[0];
            l_sumt = f_mult_div2(w[0], (filt >> 16) as i16);
            l_sumb = f_mult_div2(w[1], filt as i16);
            for k in 1..L_INTERPOL2 as usize {
                let filt = interpol_tab[k];
                l_sumt = f_mult_add_div2(l_sumt, w[2 * k], (filt >> 16) as i16);
                l_sumb = f_mult_add_div2(l_sumb, w[2 * k + 1], filt as i16);
            }
        } else {
            let filt = interpol_tab[0];
            let x0 = rd(exc_buf, xi);
            let x1 = rd(exc_buf, xi + 1);
            xi += 2;
            l_sumt = f_mult_div2(x0, (filt >> 16) as i16);
            l_sumb = f_mult_div2(x1, filt as i16);
            for k in 1..L_INTERPOL2 as usize {
                let filt = interpol_tab[k];
                let x0 = rd(exc_buf, xi);
                let x1 = rd(exc_buf, xi + 1);
                xi += 2;
                l_sumt = f_mult_add_div2(l_sumt, x0, (filt >> 16) as i16);
                l_sumb = f_mult_add_div2(l_sumb, x1, filt as i16);
            }
        }

        l_sumb <<= 1;
        l_sumb = f_add_saturate(l_sumt << 1, l_sumb);
        wr(exc_buf, exc, l_sumb);
        exc += 1;
    }
}

/// `Pred_lt4_postfilter(exc)`: compute the adaptive codebook excitation v(n) in case of
/// `ltp_filtering_flag == 0`. `exc` (`exc_buf[exc_off..]`) points to the adaptive codebook of
/// the current subframe (input/output); `exc[-1]` and `exc[L_SUBFR]` are read.
pub fn pred_lt4_postfilter(exc_buf: &mut [FixpDbl], exc_off: usize) {
    // exc[i]   = A*exc[i-1] + B*exc[i] + A*exc[i+1]
    // exc[i+1] =              A*exc[i] + B*exc[i+1] + A*exc[i+2] ; i = 0:2:62
    let e = exc_off as isize;
    let a2 = a2();
    let b = b();
    let mut a_exc0 = f_mult_div2(a2, rd(exc_buf, e - 1));
    let mut a_exc1 = f_mult_div2(a2, rd(exc_buf, e));

    let mut i = 0isize;
    while i < L_SUBFR {
        let sum0 = a_exc0.wrapping_add(f_mult(b, rd(exc_buf, e + i)));
        let sum1 = a_exc1.wrapping_add(f_mult(b, rd(exc_buf, e + i + 1)));
        a_exc0 = f_mult_div2(a2, rd(exc_buf, e + i + 1));
        a_exc1 = f_mult_div2(a2, rd(exc_buf, e + i + 2));
        wr(exc_buf, e + i, sum0.wrapping_add(a_exc0));
        wr(exc_buf, e + i + 1, sum1.wrapping_add(a_exc1));
        i += 2;
    }
}

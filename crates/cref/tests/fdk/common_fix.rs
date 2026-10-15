// Differential tests of fdk_aac_rs_fdk::common_fix against the C++ reference, and the input
// generators shared by the tests of the arithmetic primitives.

use fdk_aac_rs_cref::fdk_common_fix::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;

/// Number of random inputs per function (in addition to the edge cases).
pub const N: usize = 100_000;

/// Edge cases of `FIXP_DBL` / `INT` operands.
pub const EDGES_D: &[i32] = &[
    0,
    1,
    -1,
    2,
    -2,
    3,
    0x7FFF_FFFF,
    0x7FFF_FFFE,
    i32::MIN,
    i32::MIN + 1,
    i32::MIN + 2,
    0x4000_0000,
    -0x4000_0000,
    0x3FFF_FFFF,
    -0x3FFF_FFFF,
    0x4000_0001,
    0x2000_0000,
    0x0000_7FFF,
    0x0000_8000,
    -0x0000_8000,
    0x0000_FFFF,
    0x0001_0000,
    0x5A82_7999,
    0x1234_5678,
    -0x1234_5678,
];

/// Edge cases of `FIXP_SGL` / `SHORT` operands.
pub const EDGES_S: &[i16] =
    &[0, 1, -1, 2, -2, 0x7FFF, 0x7FFE, i16::MIN, i16::MIN + 1, 0x4000, -0x4000, 0x3FFF, 0x5A82];

/// Shift amounts and exponents: in range, at and beyond the limits.
pub const EDGES_SHIFT: &[i32] = &[
    0,
    1,
    -1,
    2,
    -2,
    14,
    15,
    16,
    -15,
    -16,
    30,
    31,
    32,
    33,
    -30,
    -31,
    -32,
    -33,
    40,
    -40,
    63,
    -63,
    64,
    100,
    -100,
    i32::MAX,
    i32::MIN,
    i32::MIN + 1,
];

/// Calls `f` for every edge case and `n` random values.
pub fn each_d(seed: u64, n: usize, mut f: impl FnMut(i32)) {
    for &a in EDGES_D {
        f(a);
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i32());
    }
}

/// Calls `f` for every edge case and `n` random values.
pub fn each_s(seed: u64, n: usize, mut f: impl FnMut(i16)) {
    for a in i16::MIN..=i16::MAX {
        f(a);
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i16());
    }
}

/// Calls `f` for every pair of edge cases and `n` random pairs.
pub fn each_dd(seed: u64, n: usize, mut f: impl FnMut(i32, i32)) {
    for &a in EDGES_D {
        for &b in EDGES_D {
            f(a, b);
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i32(), r.i32());
    }
}

/// Calls `f` for every pair of edge cases and `n` random pairs.
pub fn each_ss(seed: u64, n: usize, mut f: impl FnMut(i16, i16)) {
    for &a in EDGES_S {
        for &b in EDGES_S {
            f(a, b);
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i16(), r.i16());
    }
}

/// Calls `f` for every pair of edge cases and `n` random pairs.
pub fn each_sd(seed: u64, n: usize, mut f: impl FnMut(i16, i32)) {
    for &a in EDGES_S {
        for &b in EDGES_D {
            f(a, b);
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i16(), r.i32());
    }
}

/// Calls `f` for every triple of edge cases and `n` random triples.
pub fn each_ddd(seed: u64, n: usize, mut f: impl FnMut(i32, i32, i32)) {
    for &x in EDGES_D {
        for &a in EDGES_D {
            for &b in EDGES_D {
                f(x, a, b);
            }
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i32(), r.i32(), r.i32());
    }
}

/// Calls `f` for every triple of edge cases and `n` random triples.
pub fn each_dsd(seed: u64, n: usize, mut f: impl FnMut(i32, i16, i32)) {
    for &x in EDGES_D {
        for &a in EDGES_S {
            for &b in EDGES_D {
                f(x, a, b);
            }
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i32(), r.i16(), r.i32());
    }
}

/// Calls `f` for every triple of edge cases and `n` random triples.
pub fn each_dss(seed: u64, n: usize, mut f: impl FnMut(i32, i16, i16)) {
    for &x in EDGES_D {
        for &a in EDGES_S {
            for &b in EDGES_S {
                f(x, a, b);
            }
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(r.i32(), r.i16(), r.i16());
    }
}

/// Calls `f` for every value with every edge-case shift and `n` random pairs (random shifts in
/// -40..=40).
pub fn each_d_shift(seed: u64, n: usize, mut f: impl FnMut(i32, i32)) {
    for &a in EDGES_D {
        for &s in EDGES_SHIFT {
            f(a, s);
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        let s = if r.next_u32() % 8 == 0 { r.i32() } else { r.range(-40, 40) };
        f(r.i32(), s);
    }
}

#[test]
fn f_mult_overloads() {
    each_dd(1, N, |a, b| {
        assert_eq!(f_mult(a, b), unsafe { cref_fMult_DD(a, b) }, "fMult(DBL {a:#x}, DBL {b:#x})");
        assert_eq!(f_mult_div2(a, b), unsafe { cref_fMultDiv2_DD(a, b) }, "fMultDiv2 DD {a} {b}");
        assert_eq!(f_mult_bit_exact(a, b), unsafe { cref_fMultBitExact_DD(a, b) }, "{a} {b}");
        assert_eq!(f_mult_div2_bit_exact(a, b), unsafe { cref_fMultDiv2BitExact_DD(a, b) });
    });
    each_sd(2, N, |a, b| {
        assert_eq!(f_mult(a, b), unsafe { cref_fMult_SD(a, b) }, "fMult(SGL {a}, DBL {b})");
        assert_eq!(f_mult(b, a), unsafe { cref_fMult_DS(b, a) }, "fMult(DBL {b}, SGL {a})");
        assert_eq!(f_mult_div2(a, b), unsafe { cref_fMultDiv2_SD(a, b) });
        assert_eq!(f_mult_div2(b, a), unsafe { cref_fMultDiv2_DS(b, a) });
        assert_eq!(f_mult_bit_exact(a, b), unsafe { cref_fMultBitExact_SD(a, b) });
        assert_eq!(f_mult_bit_exact(b, a), unsafe { cref_fMultBitExact_DS(b, a) });
        assert_eq!(f_mult_div2_bit_exact(a, b), unsafe { cref_fMultDiv2BitExact_SD(a, b) });
        assert_eq!(f_mult_div2_bit_exact(b, a), unsafe { cref_fMultDiv2BitExact_DS(b, a) });
    });
    each_ss(3, N, |a, b| {
        assert_eq!(f_mult(a, b), unsafe { cref_fMult_SS(a, b) }, "fMult(SGL {a}, SGL {b})");
        assert_eq!(f_mult_div2(a, b), unsafe { cref_fMultDiv2_SS(a, b) });
    });
}

#[test]
fn f_pow2_overloads() {
    each_d(4, N, |a| {
        assert_eq!(f_pow2(a), unsafe { cref_fPow2_D(a) }, "fPow2(DBL {a:#x})");
        assert_eq!(f_pow2_div2(a), unsafe { cref_fPow2Div2_D(a) }, "fPow2Div2(DBL {a:#x})");
    });
    each_s(5, N, |a| {
        assert_eq!(f_pow2(a), unsafe { cref_fPow2_S(a) }, "fPow2(SGL {a})");
        assert_eq!(f_pow2_div2(a), unsafe { cref_fPow2Div2_S(a) }, "fPow2Div2(SGL {a})");
    });
    each_dd(6, N, |x, a| {
        assert_eq!(f_pow2_add(x, a), unsafe { cref_fPow2Add_D(x, a) });
        assert_eq!(f_pow2_add_div2(x, a), unsafe { cref_fPow2AddDiv2_D(x, a) });
    });
    each_sd(7, N, |a, x| {
        assert_eq!(f_pow2_add(x, a), unsafe { cref_fPow2Add_S(x, a) });
        assert_eq!(f_pow2_add_div2(x, a), unsafe { cref_fPow2AddDiv2_S(x, a) });
    });
}

#[test]
fn f_abs_f_norm_overloads() {
    each_d(8, N, |x| {
        assert_eq!(f_abs(x), unsafe { cref_fAbs_D(x) }, "fAbs(DBL {x:#x})");
        assert_eq!(fixp_abs(x), unsafe { cref_fixp_abs_D(x) });
        assert_eq!(f_normz(x), unsafe { cref_fNormz_D(x) }, "fNormz(DBL {x:#x})");
        assert_eq!(f_norm(x), unsafe { cref_fNorm_D(x) }, "fNorm(DBL {x:#x})");
        assert_eq!(cnt_leading_zeros(x), unsafe { cref_CntLeadingZeros(x) });
        assert_eq!(count_leading_bits(x), unsafe { cref_CountLeadingBits(x) });
    });
    each_s(9, N, |x| {
        assert_eq!(f_abs(x), unsafe { cref_fAbs_S(x) }, "fAbs(SGL {x})");
        assert_eq!(f_normz(x), unsafe { cref_fNormz_S(x) }, "fNormz(SGL {x})");
        assert_eq!(f_norm(x), unsafe { cref_fNorm_S(x) }, "fNorm(SGL {x})");
    });
    let mut r = Rng::new(10);
    for i in 0..N + 200 {
        let x = match i {
            0..=63 => 1i64 << i,
            64..=127 => (1i64 << (i - 64)).wrapping_neg(),
            128..=191 => (1i64 << (i - 128)).wrapping_sub(1),
            192 => 0,
            193 => i64::MIN,
            194 => i64::MAX,
            _ => r.i64(),
        };
        assert_eq!(f_normz(x), unsafe { cref_fNormz_Q(x) }, "fNormz(INT64 {x:#x})");
    }
}

#[test]
fn f_mult_add_overloads() {
    each_ddd(11, N, |x, a, b| {
        assert_eq!(f_mult_add(x, a, b), unsafe { cref_fMultAdd_DD(x, a, b) }, "{x} {a} {b}");
        assert_eq!(f_mult_add_div2(x, a, b), unsafe { cref_fMultAddDiv2_DD(x, a, b) });
        assert_eq!(f_mult_sub(x, a, b), unsafe { cref_fMultSub_DD(x, a, b) });
        assert_eq!(f_mult_sub_div2(x, a, b), unsafe { cref_fMultSubDiv2_DD(x, a, b) });
        assert_eq!(f_mult_add_div2_bit_exact(x, a, b), unsafe {
            cref_fMultAddDiv2BitExact_DD(x, a, b)
        });
        assert_eq!(f_mult_sub_div2_bit_exact(x, a, b), unsafe {
            cref_fMultSubDiv2BitExact_DD(x, a, b)
        });
    });
    each_dsd(12, N, |x, a, b| {
        assert_eq!(f_mult_add(x, a, b), unsafe { cref_fMultAdd_SD(x, a, b) });
        assert_eq!(f_mult_add(x, b, a), unsafe { cref_fMultAdd_DS(x, b, a) });
        assert_eq!(f_mult_add_div2(x, a, b), unsafe { cref_fMultAddDiv2_SD(x, a, b) });
        assert_eq!(f_mult_add_div2(x, b, a), unsafe { cref_fMultAddDiv2_DS(x, b, a) });
        assert_eq!(f_mult_sub(x, a, b), unsafe { cref_fMultSub_SD(x, a, b) });
        assert_eq!(f_mult_sub(x, b, a), unsafe { cref_fMultSub_DS(x, b, a) });
        assert_eq!(f_mult_sub_div2(x, a, b), unsafe { cref_fMultSubDiv2_SD(x, a, b) });
        assert_eq!(f_mult_sub_div2(x, b, a), unsafe { cref_fMultSubDiv2_DS(x, b, a) });
        assert_eq!(f_mult_add_div2_bit_exact(x, a, b), unsafe {
            cref_fMultAddDiv2BitExact_SD(x, a, b)
        });
        assert_eq!(f_mult_add_div2_bit_exact(x, b, a), unsafe {
            cref_fMultAddDiv2BitExact_DS(x, b, a)
        });
        assert_eq!(f_mult_sub_div2_bit_exact(x, a, b), unsafe {
            cref_fMultSubDiv2BitExact_SD(x, a, b)
        });
        assert_eq!(f_mult_sub_div2_bit_exact(x, b, a), unsafe {
            cref_fMultSubDiv2BitExact_DS(x, b, a)
        });
    });
    each_dss(13, N, |x, a, b| {
        assert_eq!(f_mult_add(x, a, b), unsafe { cref_fMultAdd_SS(x, a, b) });
        assert_eq!(f_mult_add_div2(x, a, b), unsafe { cref_fMultAddDiv2_SS(x, a, b) });
        assert_eq!(f_mult_sub(x, a, b), unsafe { cref_fMultSub_SS(x, a, b) });
        assert_eq!(f_mult_sub_div2(x, a, b), unsafe { cref_fMultSubDiv2_SS(x, a, b) });
    });
}

#[test]
fn f_min_f_max_overloads() {
    each_dd(14, N, |a, b| {
        assert_eq!(f_min(a, b), unsafe { cref_fMin_D(a, b) });
        assert_eq!(f_max(a, b), unsafe { cref_fMax_D(a, b) });
        assert_eq!(fix_min(a, b), unsafe { cref_fixMin(a, b) });
        assert_eq!(fix_max(a, b), unsafe { cref_fixMax(a, b) });
        let (ua, ub) = (a as u32, b as u32);
        assert_eq!(f_min(ua, ub), unsafe { cref_fMin_UI(ua, ub) });
        assert_eq!(f_max(ua, ub), unsafe { cref_fMax_UI(ua, ub) });
        let (ca, cb) = (a as u8, b as u8);
        assert_eq!(f_min(ca, cb), unsafe { cref_fMin_UC(ca, cb) });
        assert_eq!(f_max(ca, cb), unsafe { cref_fMax_UC(ca, cb) });
    });
    each_ss(15, N, |a, b| {
        assert_eq!(f_min(a, b), unsafe { cref_fMin_S(a, b) });
        assert_eq!(f_max(a, b), unsafe { cref_fMax_S(a, b) });
    });
}

#[test]
fn conversion_macros() {
    each_d(16, N, |v| {
        assert_eq!(fx_dbl2fxconst_sgl(v), unsafe { cref_FX_DBL2FXCONST_SGL(v) }, "{v:#x}");
        assert_eq!(fx_dbl2fx_sgl(v), unsafe { cref_FX_DBL2FX_SGL(v) });
        assert_eq!(fx_dbl2fx_pcm(v), unsafe { cref_FX_DBL2FX_PCM(v) });
        assert_eq!(fx_dbl2fl(v).to_bits(), unsafe { cref_FX_DBL2FL(v) }.to_bits());
        assert_eq!(fx_dbl2double(v).to_bits(), unsafe { cref_FX_DBL2DOUBLE(v) }.to_bits());
    });
    each_s(17, N, |v| {
        assert_eq!(fx_sgl2fx_dbl(v), unsafe { cref_FX_SGL2FX_DBL(v) });
        assert_eq!(fx_pcm2fx_dbl(v), unsafe { cref_FX_PCM2FX_DBL(v) });
    });
    // Constants in -2.0..2.0 (the macros convert constants; out-of-range float conversions are
    // undefined in C).
    let mut r = Rng::new(18);
    let mut values: Vec<f64> = vec![0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 0.25, 0.999_999_999, -0.999_999_999];
    values.extend((0..=64).map(|i| i as f64 / 32.0 - 1.0));
    for _ in 0..N {
        values.push((r.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * 4.0 - 2.0);
    }
    for &v in &values {
        assert_eq!(fl2fxconst_sgl(v), unsafe { cref_FL2FXCONST_SGL(v) }, "FL2FXCONST_SGL({v})");
        assert_eq!(fl2fxconst_dbl(v), unsafe { cref_FL2FXCONST_DBL(v) }, "FL2FXCONST_DBL({v})");
        assert_eq!(fl2fxconst_spc(v), unsafe { cref_FL2FXCONST_SPC(v) });
        // FL2FX_*: only values whose result fits (no overflow check in C).
        let f = v as f32;
        if f.abs() < 0.999 {
            assert_eq!(fl2fx_sgl(f), unsafe { cref_FL2FX_SGL(f) }, "FL2FX_SGL({f})");
            assert_eq!(fl2fx_dbl(f), unsafe { cref_FL2FX_DBL(f) }, "FL2FX_DBL({f})");
            assert_eq!(fl2fx_spc(f), unsafe { cref_FL2FX_DBL(f) });
        }
    }
}

#[test]
fn constants() {
    let rust: [i64; 18] = [
        FRACT_BITS as i64,
        DFRACT_BITS as i64,
        ACCU_BITS as i64,
        SGL_MASK as i64,
        MAX_SHIFT_SGL as i64,
        MAX_SHIFT_DBL as i64,
        FRACT_FIX_SCALE,
        DFRACT_FIX_SCALE,
        MAXVAL_SGL as i64,
        MINVAL_SGL as i64,
        MAXVAL_DBL as i64,
        MINVAL_DBL as i64,
        MINVAL_DBL_CONST as i64,
        MINVAL_SGL_CONST as i64,
        MAXVAL_FIXP_PCM as i64,
        MINVAL_FIXP_PCM as i64,
        core::mem::size_of::<FixpSpk>() as i64,
        core::mem::size_of::<FixpDpk>() as i64,
    ];
    for (i, &v) in rust.iter().enumerate() {
        assert_eq!(v, unsafe { cref_common_fix_const(i as i32) }, "constant {i}");
    }
}

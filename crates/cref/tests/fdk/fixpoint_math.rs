// Differential tests of fdk_aac_rs_fdk::fixpoint_math against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_fixpoint_math::*;
use fdk_aac_rs_cref::fdk_fixpoint_math_tables::cref_fixpoint_math_table;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::fixpoint_math::*;

/// A positive `FIXP_DBL` of random magnitude.
fn pos(r: &mut Rng) -> i32 {
    let v = (r.next_u32() >> 1) >> (r.next_u32() % 31);
    if v == 0 {
        1
    } else {
        v as i32
    }
}

/// An exponent: mostly small, sometimes large.
fn exponent(r: &mut Rng) -> i32 {
    match r.next_u32() % 8 {
        0 => r.range(-1000, 1000),
        1 => EDGES_SHIFT[(r.next_u32() as usize) % EDGES_SHIFT.len()],
        _ => r.range(-40, 40),
    }
}

/// Positive values: edge cases and random magnitudes.
fn each_pos(seed: u64, n: usize, mut f: impl FnMut(i32)) {
    for v in [1, 2, 3, 4, 0x7FFF_FFFF, 0x7FFF_FFFE, 0x4000_0000, 0x3FFF_FFFF, 0x4000_0001, 0x10000, 0xFFFF, 0x7F] {
        f(v);
    }
    for i in 0..31 {
        f(1 << i);
        f((1 << i) + 1);
        f(((1u32 << (i + 1)) - 1) as i32);
    }
    let mut r = Rng::new(seed);
    for _ in 0..n {
        f(pos(&mut r));
    }
}

#[test]
fn tables() {
    unsafe {
        assert_eq!(&LD_COEFF[..], core::slice::from_raw_parts(cref_ldCoeff(), 10));
        assert_eq!(&EXP2_TAB_LONG[..], core::slice::from_raw_parts(cref_exp2_tab_long(), 32));
        assert_eq!(&EXP2W_TAB_LONG[..], core::slice::from_raw_parts(cref_exp2w_tab_long(), 32));
        assert_eq!(&EXP2X_TAB_LONG[..], core::slice::from_raw_parts(cref_exp2x_tab_long(), 32));
        assert_eq!(&INV_SQRT_TAB[..], core::slice::from_raw_parts(cref_invSqrtTab(), 130));
        assert_eq!(&SQRT_TAB[..], core::slice::from_raw_parts(cref_sqrt_tab(), 49));
        assert_eq!(&INV_COUNT[..], core::slice::from_raw_parts(cref_invCount(), 80));
    }
}

#[test]
fn fIsLessThan() {
    let mut r = Rng::new(801);
    each_dd(802, N, |a, b| {
        let (ae, be) = (exponent(&mut r), exponent(&mut r));
        assert_eq!(f_is_less_than(a, ae, b, be), unsafe { cref_fIsLessThan_D(a, ae, b, be) }, "{a} {ae} {b} {be}");
        let (sa, sb) = (a as i16, b as i16);
        assert_eq!(f_is_less_than_sgl(sa, ae, sb, be), unsafe {
            cref_fIsLessThan_S(sa, ae, sb, be)
        }, "{sa} {ae} {sb} {be}");
    });
}

#[test]
fn CalcInvLdData_CalcLdData() {
    each_d(803, N, |x| {
        assert_eq!(calc_inv_ld_data(x), unsafe { cref_CalcInvLdData(x) }, "CalcInvLdData({x:#x})");
        assert_eq!(calc_ld_data(x), unsafe { cref_CalcLdData(x) }, "CalcLdData({x:#x})");
    });
    for x in (i32::MIN..=i32::MAX).step_by(997) {
        assert_eq!(calc_inv_ld_data(x), unsafe { cref_CalcInvLdData(x) }, "CalcInvLdData({x:#x})");
    }
}

#[test]
fn CalcLdInt() {
    for i in -10..300 {
        assert_eq!(calc_ld_int(i), unsafe { cref_CalcLdInt(i) }, "CalcLdInt({i})");
    }
    for i in [i32::MIN, i32::MAX] {
        assert_eq!(calc_ld_int(i), unsafe { cref_CalcLdInt(i) });
    }
}

#[test]
fn sqrtFixp_lookup() {
    // x == 0 has no defined result in the reference, so it is not tested.
    each_d(804, N, |x| {
        if x != 0 {
            assert_eq!(sqrt_fixp_lookup(x), unsafe { cref_sqrtFixp_lookup(x) }, "{x:#x}");
        }
    });
    let mut r = Rng::new(805);
    each_d(806, N, |x| {
        let e0 = exponent(&mut r);
        let (mut e1, mut e2) = (e0, e0);
        let a = sqrt_fixp_lookup_e(x, &mut e1);
        let b = unsafe { cref_sqrtFixp_lookup_e(x, &mut e2) };
        assert_eq!((a, e1), (b, e2), "sqrtFixp_lookup({x:#x}, {e0})");
    });
}

#[test]
fn invSqrtNorm2_sqrtFixp_invFixp() {
    let mut check = |op: i32| {
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = inv_sqrt_norm2(op, &mut e1);
        let b = unsafe { cref_invSqrtNorm2(op, &mut e2) };
        assert_eq!((a, e1), (b, e2), "invSqrtNorm2({op:#x})");
        assert_eq!(sqrt_fixp(op), unsafe { cref_sqrtFixp(op) }, "sqrtFixp({op:#x})");
        assert_eq!(inv_fixp(op), unsafe { cref_invFixp(op) }, "invFixp({op:#x})");
    };
    check(0);
    each_pos(807, N, &mut check);
    // The step covers every exponent and many mantissas.
    for op in (1..=i32::MAX).step_by(4099) {
        check(op);
    }
}

/// Negative inputs are outside of the domain (FDK_ASSERT); the C++ code shifts by -1
/// (`aarch64`) or takes the square root of a negative number (`x86`). The port's debug_assert!
/// of the FDK_ASSERT stops this test in the `checked` profile.
#[test]
#[cfg_attr(debug_assertions, ignore = "outside of the FDK_ASSERT domain")]
fn invSqrtNorm2_sqrtFixp_invFixp_negative() {
    each_d(808, N, |op| {
        let op = if op > 0 { op.wrapping_neg() } else { op };
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = inv_sqrt_norm2(op, &mut e1);
        let b = unsafe { cref_invSqrtNorm2(op, &mut e2) };
        assert_eq!((a, e1), (b, e2), "invSqrtNorm2({op:#x})");
        assert_eq!(sqrt_fixp(op), unsafe { cref_sqrtFixp(op) }, "sqrtFixp({op:#x})");
        assert_eq!(inv_fixp(op), unsafe { cref_invFixp(op) }, "invFixp({op:#x})");
    });
}

#[test]
fn invFixp_e() {
    let mut r = Rng::new(809);
    each_d(810, N, |op| {
        // Negative values are outside of the FDK_ASSERT domain of the integer variant; the
        // debug_assert! of the port would stop the `checked` profile.
        if cfg!(debug_assertions) && op < 0 {
            return;
        }
        let e0 = exponent(&mut r);
        let (mut e1, mut e2) = (e0, e0);
        let a = inv_fixp_e(op, &mut e1);
        let b = unsafe { cref_invFixp_e(op, &mut e2) };
        assert_eq!((a, e1), (b, e2), "invFixp({op:#x}, {e0})");
    });
}

#[test]
fn schur_div() {
    let mut r = Rng::new(811);
    let check = |num: i32, denum: i32, count: i32| {
        assert_eq!(
            fdk_aac_rs_fdk::fixpoint_math::schur_div(num, denum, count),
            unsafe { cref_schur_div(num, denum, count) },
            "schur_div({num:#x}, {denum:#x}, {count})"
        );
    };
    for count in 1..=32 {
        for (num, denum) in [(0, 1), (1, 1), (1, 2), (0x3FFF_FFFF, 0x4000_0000), (0x7FFF_FFFF, 0x7FFF_FFFF), (1, 0x7FFF_FFFF)] {
            check(num, denum, count);
        }
    }
    for _ in 0..N {
        let denum = pos(&mut r);
        let num = (r.next_u64() % (denum as u64 + 1)) as i32;
        let count = if r.next_u32() % 2 == 0 { 16 } else { r.range(1, 32) };
        check(num, denum, count);
    }
}

#[test]
fn fMultNorm() {
    let mut r = Rng::new(812);
    each_dd(813, N, |f1, f2| {
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_mult_norm(f1, f2, &mut e1);
        let b = unsafe { cref_fMultNorm_e(f1, f2, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fMultNorm({f1:#x}, {f2:#x})");
        assert_eq!(f_mult_norm_noexp(f1, f2), unsafe { cref_fMultNorm(f1, f2) });
        let (x1, x2, xr) = (exponent(&mut r), exponent(&mut r), exponent(&mut r));
        assert_eq!(f_mult_norm_to(f1, x1, f2, x2, xr), unsafe {
            cref_fMultNorm_to(f1, x1, f2, x2, xr)
        }, "fMultNorm({f1:#x}, {x1}, {f2:#x}, {x2}, {xr})");
        assert_eq!(f_mult_i(f1, f2), unsafe { cref_fMultI(f1, f2) }, "fMultI({f1:#x}, {f2})");
        assert_eq!(f_mult_i_floor(f1, f2), unsafe { cref_fMultIfloor(f1, f2) });
        assert_eq!(f_mult_i_ceil(f1, f2), unsafe { cref_fMultIceil(f1, f2) }, "fMultIceil({f1:#x}, {f2})");
        let b16 = f2 >> 16;
        assert_eq!(f_mult_i(f1, b16), unsafe { cref_fMultI(f1, b16) });
        assert_eq!(f_mult_i_floor(f1, b16), unsafe { cref_fMultIfloor(f1, b16) });
        assert_eq!(f_mult_i_ceil(f1, b16), unsafe { cref_fMultIceil(f1, b16) });
    });
}

#[test]
fn fDivNorm() {
    let mut r = Rng::new(814);
    for _ in 0..N + 1000 {
        let denom = pos(&mut r);
        let num = if r.next_u32() % 2 == 0 { (r.next_u64() % (denom as u64 + 1)) as i32 } else { pos(&mut r) };
        let num = if r.next_u32() % 50 == 0 { 0 } else { num };
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_div_norm(num, denom, &mut e1);
        let b = unsafe { cref_fDivNorm_e(num, denom, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fDivNorm({num:#x}, {denom:#x})");
        if num <= denom {
            assert_eq!(f_div_norm_noexp(num, denom), unsafe { cref_fDivNorm(num, denom) }, "fDivNorm({num:#x}, {denom:#x})");
        }
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_div_norm_high_prec(num, denom, &mut e1);
        let b = unsafe { cref_fDivNormHighPrec(num, denom, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fDivNormHighPrec({num:#x}, {denom:#x})");
    }
}

#[test]
fn fDivNormSigned() {
    each_dd(815, N, |num, denom| {
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_div_norm_signed(num, denom, &mut e1);
        let b = unsafe { cref_fDivNormSigned_e(num, denom, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fDivNormSigned({num:#x}, {denom:#x})");
        assert_eq!(f_div_norm_signed_noexp(num, denom), unsafe { cref_fDivNormSigned(num, denom) }, "fDivNormSigned({num:#x}, {denom:#x})");
    });
}

#[test]
fn fAdjust_fAddNorm() {
    let mut r = Rng::new(816);
    each_dd(817, N, |a_m, b_m| {
        let (a_e, b_e, res_e) = (exponent(&mut r), exponent(&mut r), exponent(&mut r));
        let (mut e1, mut e2) = (a_e, a_e);
        assert_eq!(f_adjust(a_m, &mut e1), unsafe { cref_fAdjust(a_m, &mut e2) });
        assert_eq!(e1, e2);
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_add_norm(a_m, a_e, b_m, b_e, &mut e1);
        let b = unsafe { cref_fAddNorm_e(a_m, a_e, b_m, b_e, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fAddNorm({a_m:#x}, {a_e}, {b_m:#x}, {b_e})");
        assert_eq!(f_add_norm_to(a_m, a_e, b_m, b_e, res_e), unsafe {
            cref_fAddNorm_to(a_m, a_e, b_m, b_e, res_e)
        }, "fAddNorm({a_m:#x}, {a_e}, {b_m:#x}, {b_e}, {res_e})");
    });
}

#[test]
fn fLog2() {
    let mut r = Rng::new(818);
    each_d(819, N, |x_m| {
        let x_e = if r.next_u32() % 4 == 0 { 0 } else { r.range(-(1 << 20), 1 << 20) >> r.range(0, 20) };
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_log2(x_m, x_e, &mut e1);
        let b = unsafe { cref_fLog2_e(x_m, x_e, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fLog2({x_m:#x}, {x_e})");
        let (mut e1, mut e2) = (0x5555, 0x5555);
        assert_eq!(calc_log2(x_m, x_e, &mut e1), unsafe { cref_CalcLog2(x_m, x_e, &mut e2) });
        assert_eq!(e1, e2);
        assert_eq!(f_log2_noexp(x_m, x_e), unsafe { cref_fLog2(x_m, x_e) }, "fLog2({x_m:#x}, {x_e})");
    });
    let src: Vec<i32> = (0..1000).map(|_| r.i32()).collect();
    let (mut a, mut b) = (vec![0; 1000], vec![0; 1000]);
    ld_data_vector(&src, &mut a);
    let mut src2 = src.clone();
    unsafe { cref_LdDataVector(src2.as_mut_ptr(), b.as_mut_ptr(), 1000) };
    assert_eq!(a, b);
}

/// fLog2 with exponents of 2^30 and more: `x_e << (31 - enorm)` shifts by -1.
#[test]
fn fLog2_large_exponent() {
    each_d(820, 2000, |x_m| {
        for x_e in [1 << 30, (1 << 30) + 12345, i32::MAX, -(1 << 30) - 1, i32::MIN] {
            let (mut e1, mut e2) = (0x5555, 0x5555);
            let a = f_log2(x_m, x_e, &mut e1);
            let b = unsafe { cref_fLog2_e(x_m, x_e, &mut e2) };
            assert_eq!((a, e1), (b, e2), "fLog2({x_m:#x}, {x_e})");
        }
    });
}

#[test]
fn f2Pow_fPow_fLdPow() {
    let mut r = Rng::new(821);
    each_dd(822, N, |m, n| {
        let e = r.range(-40, 40);
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f2_pow(m, e, &mut e1);
        let b = unsafe { cref_f2Pow_e(m, e, &mut e2) };
        assert_eq!((a, e1), (b, e2), "f2Pow({m:#x}, {e})");
        assert_eq!(f2_pow_noexp(m, e), unsafe { cref_f2Pow(m, e) }, "f2Pow({m:#x}, {e})");

        let (be, ee) = (r.range(-40, 40), r.range(-40, 40));
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_pow(m, be, n, ee, &mut e1);
        let b = unsafe { cref_fPow(m, be, n, ee, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fPow({m:#x}, {be}, {n:#x}, {ee})");
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_ld_pow(m, be, n, ee, &mut e1);
        let b = unsafe { cref_fLdPow_e(m, be, n, ee, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fLdPow({m:#x}, {be}, {n:#x}, {ee})");
        assert_eq!(f_ld_pow_noexp(m, be, n, ee), unsafe { cref_fLdPow(m, be, n, ee) }, "fLdPow({m:#x}, {be}, {n:#x}, {ee})");
    });
}

#[test]
fn fPowInt() {
    let mut r = Rng::new(823);
    for i in 0..N {
        let base_m = if i < 100 { (i as i32) << 24 } else { pos(&mut r) };
        let base_m = if i % 50 == 0 { 0 } else { base_m };
        let base_e = r.range(-40, 40);
        let n = r.range(-20, 20);
        let (mut e1, mut e2) = (0x5555, 0x5555);
        let a = f_pow_int(base_m, base_e, n, &mut e1);
        let b = unsafe { cref_fPowInt(base_m, base_e, n, &mut e2) };
        assert_eq!((a, e1), (b, e2), "fPowInt({base_m:#x}, {base_e}, {n})");
    }
}

#[test]
fn fAddSaturate() {
    each_dd(824, N, |a, b| {
        assert_eq!(f_add_saturate(a, b), unsafe { cref_fAddSaturate_D(a, b) }, "{a:#x} {b:#x}");
    });
    each_ss(825, N, |a, b| {
        assert_eq!(f_add_saturate_sgl(a, b), unsafe { cref_fAddSaturate_S(a, b) }, "{a} {b}");
    });
}

#[test]
fn fixp_floor_ceil_truncate_round() {
    each_d(826, N / 4, |f| {
        for sf in 0..=31 {
            assert_eq!(fixp_floor_to_int(f, sf), unsafe { cref_fixp_floorToInt(f, sf) });
            assert_eq!(fixp_floor(f, sf), unsafe { cref_fixp_floor(f, sf) }, "fixp_floor({f:#x}, {sf})");
            assert_eq!(fixp_ceil_to_int(f, sf), unsafe { cref_fixp_ceilToInt(f, sf) }, "fixp_ceilToInt({f:#x}, {sf})");
            assert_eq!(fixp_ceil(f, sf), unsafe { cref_fixp_ceil(f, sf) }, "fixp_ceil({f:#x}, {sf})");
            assert_eq!(fixp_truncate_to_int(f, sf), unsafe { cref_fixp_truncateToInt(f, sf) }, "fixp_truncateToInt({f:#x}, {sf})");
            assert_eq!(fixp_truncate(f, sf), unsafe { cref_fixp_truncate(f, sf) }, "fixp_truncate({f:#x}, {sf})");
        }
        // sf == 31: sx == 0 shifts by -1 (undefined); the out-of-line library functions are
        // what every caller runs. Beyond (sf > 31 or < 0, FDK_ASSERT(sf >= 0)) every shift
        // amount is out of range; the three reference builds compile the same instructions
        // (amounts modulo 32), which the port reproduces.
        for sf in (-33..=64).chain([i32::MIN, i32::MAX]) {
            assert_eq!(fixp_round_to_int(f, sf), unsafe { cref_fixp_roundToInt(f, sf) }, "fixp_roundToInt({f:#x}, {sf})");
            assert_eq!(fixp_round(f, sf), unsafe { cref_fixp_round(f, sf) }, "fixp_round({f:#x}, {sf})");
        }
    });
}

#[test]
fn GetInvInt() {
    for i in -100..200 {
        assert_eq!(get_inv_int(i), unsafe { cref_GetInvInt(i) });
    }
    for i in [i32::MIN, i32::MAX] {
        assert_eq!(get_inv_int(i), unsafe { cref_GetInvInt(i) });
    }
}

/// The file-static table `name` of fixpoint_math.cpp (a copy compiled into the shim).
fn c_table<T: Copy>(name: &str) -> Vec<T> {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut bytes = 0;
    let p = unsafe { cref_fixpoint_math_table(cname.as_ptr(), &mut bytes) };
    assert!(!p.is_null(), "{name}");
    assert_eq!(bytes as usize % core::mem::size_of::<T>(), 0, "{name}");
    unsafe { core::slice::from_raw_parts(p as *const T, bytes as usize / core::mem::size_of::<T>()) }.to_vec()
}

/// POW2_COEFF (`pow2Coeff`) and LD_INT_COEFF (`ldIntCoeff`), element by element.
#[test]
fn file_static_tables() {
    assert_eq!(c_table::<i16>("pow2Coeff"), POW2_COEFF.to_vec(), "pow2Coeff");
    assert_eq!(c_table::<i32>("ldIntCoeff"), LD_INT_COEFF.to_vec(), "ldIntCoeff");
}

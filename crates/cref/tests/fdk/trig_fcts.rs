// Differential tests of fdk_aac_rs_fdk::trig_fcts against the C++ reference.

use fdk_aac_rs_cref::fdk_trig_fcts as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::trig_fcts::*;

/// `MINVAL_DBL` makes the reference pass a negative operand to the division functions, whose
/// `FDK_ASSERT`s are `debug_assert!`s in the port: only tested without debug assertions.
fn input(rng: &mut Rng) -> i32 {
    loop {
        let v = rng.i32();
        if v != i32::MIN || !cfg!(debug_assertions) {
            return v;
        }
    }
}

#[test]
fn f_atan_expand_range() {
    let mut len = 0;
    let p = unsafe { c::cref_trig_fcts_f_atan_expand_range(&mut len) };
    let t = unsafe { std::slice::from_raw_parts(p, len as usize) };
    assert_eq!(t, &F_ATAN_EXPAND_RANGE[..]);
}

#[test]
fn fixp_atan_matches() {
    let mut rng = Rng::new(1);
    let mut edge: Vec<i32> = vec![0, 1, -1, i32::MAX, -i32::MAX, 1 << 25, (1 << 25) - 1];
    edge.extend([0x28F5C28, 0x28F5C29, 0x28F5C27, -(1 << 25), -0x28F5C28, -0x28F5C29]);
    if !cfg!(debug_assertions) {
        edge.push(i32::MIN);
    }
    for &x in &edge {
        assert_eq!(fixp_atan(x), unsafe { c::cref_trig_fcts_fixp_atan(x) }, "fixp_atan({x:#x})");
    }
    for _ in 0..200_000 {
        let x = input(&mut rng);
        assert_eq!(fixp_atan(x), unsafe { c::cref_trig_fcts_fixp_atan(x) }, "fixp_atan({x:#x})");
    }
}

#[test]
fn fixp_atan2_matches() {
    let mut rng = Rng::new(2);
    let mut edge: Vec<i32> = vec![0, 1, -1, i32::MAX, -i32::MAX, 3, -3, 1 << 20, -(1 << 20)];
    if !cfg!(debug_assertions) {
        edge.push(i32::MIN);
    }
    for &y in &edge {
        for &x in &edge {
            let e = unsafe { c::cref_trig_fcts_fixp_atan2(y, x) };
            assert_eq!(fixp_atan2(y, x), e, "fixp_atan2({y:#x}, {x:#x})");
        }
    }
    for _ in 0..200_000 {
        let (y, x) = (input(&mut rng), input(&mut rng));
        // Ratios of all sizes: scale one operand down.
        let x = if rng.next_u32() % 2 == 0 { x >> (rng.next_u32() % 31) } else { x };
        let e = unsafe { c::cref_trig_fcts_fixp_atan2(y, x) };
        assert_eq!(fixp_atan2(y, x), e, "fixp_atan2({y:#x}, {x:#x})");
    }
}

#[test]
fn fixp_sin_cos_matches() {
    let mut rng = Rng::new(3);
    for i in 0..300_000 {
        let x = rng.i32();
        let x2 = rng.i32();
        // 31 - scale - LD - 1 must be a valid shift amount: scale 0..=21.
        let scale = (i % 22) as i32;
        assert_eq!(fixp_cos(x, scale), unsafe { c::cref_trig_fcts_fixp_cos(x, scale) });
        assert_eq!(fixp_sin(x, scale), unsafe { c::cref_trig_fcts_fixp_sin(x, scale) });
        let mut out = [0i32; 2];
        unsafe { c::cref_trig_fcts_fixp_cos_sin(x, scale, out.as_mut_ptr()) };
        assert_eq!(fixp_cos_sin(x, scale), (out[0], out[1]), "fixp_cos_sin({x}, {scale})");
        let r = unsafe { c::cref_trig_fcts_fixp_sin_cos_residual_inline(x, scale, out.as_mut_ptr()) };
        assert_eq!(fixp_sin_cos_residual_inline(x, scale), (r, out[0], out[1]));
        let mut out4 = [0i32; 4];
        unsafe { c::cref_trig_fcts_inline_fixp_cos_sin(x, x2, scale, out4.as_mut_ptr()) };
        assert_eq!(inline_fixp_cos_sin(x, x2, scale), out4, "inline_fixp_cos_sin({x}, {x2}, {scale})");
    }
}

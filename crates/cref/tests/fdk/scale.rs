// Differential tests of fdk_aac_rs_fdk::scale against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_scale::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::scale::*;

const VEC_CASES: usize = 20_000;

fn vec_d(r: &mut Rng, n: usize) -> Vec<i32> {
    (0..n).map(|_| r.i32()).collect()
}

fn vec_s(r: &mut Rng, n: usize) -> Vec<i16> {
    (0..n).map(|_| r.i16()).collect()
}

/// A scalefactor: every edge case in turn, then random values (mostly in -40..=40).
fn scalefactor(r: &mut Rng, i: usize) -> i32 {
    if i < EDGES_SHIFT.len() {
        EDGES_SHIFT[i]
    } else if r.next_u32() % 16 == 0 {
        r.i32()
    } else {
        r.range(-40, 40)
    }
}

#[test]
fn scaleValue() {
    each_d_shift(701, N, |v, s| {
        assert_eq!(scale_value(v, s), unsafe { cref_scaleValue_D(v, s) }, "scaleValue({v:#x}, {s})");
        assert_eq!(scale_value_saturate(v, s), unsafe { cref_scaleValueSaturate(v, s) }, "{v:#x} {s}");
        let mut x = v;
        scale_value_in_place(&mut x, s);
        assert_eq!(x, unsafe { cref_scaleValueInPlace(v, s) }, "scaleValueInPlace({v:#x}, {s})");
    });
    each_s(702, 0, |v| {
        for &s in EDGES_SHIFT {
            assert_eq!(scale_value_sgl(v, s), unsafe { cref_scaleValue_S(v, s) }, "{v} {s}");
        }
    });
}

#[test]
fn SATURATE_SHIFT() {
    let mut r = Rng::new(703);
    let check = |src: i32, scale: i32, dbits: i32| {
        assert_eq!(
            saturate_right_shift(src, scale, dbits),
            unsafe { cref_SATURATE_RIGHT_SHIFT(src, scale, dbits) },
            "SATURATE_RIGHT_SHIFT({src:#x}, {scale}, {dbits})"
        );
        assert_eq!(
            saturate_left_shift(src, scale, dbits),
            unsafe { cref_SATURATE_LEFT_SHIFT(src, scale, dbits) },
            "SATURATE_LEFT_SHIFT({src:#x}, {scale}, {dbits})"
        );
        assert_eq!(saturate_shift(src, scale, dbits), unsafe {
            cref_SATURATE_SHIFT(src, scale, dbits)
        });
        assert_eq!(saturate_shift(src, -scale, dbits), unsafe {
            cref_SATURATE_SHIFT(src, -scale, dbits)
        });
        assert_eq!(
            saturate_left_shift_alt(src, scale, dbits),
            unsafe { cref_SATURATE_LEFT_SHIFT_ALT(src, scale, dbits) },
            "SATURATE_LEFT_SHIFT_ALT({src:#x}, {scale}, {dbits})"
        );
        assert_eq!(saturate_right_shift_alt(src, scale, dbits), unsafe {
            cref_SATURATE_RIGHT_SHIFT_ALT(src, scale, dbits)
        });
    };
    // Shift amounts 0..=31 and dBits 1..=32.
    for &src in EDGES_D {
        for scale in 0..32 {
            for dbits in [1, 8, 15, 16, 17, 24, 31, 32] {
                check(src, scale, dbits);
            }
        }
    }
    for _ in 0..N {
        check(r.i32(), r.range(0, 31), r.range(1, 32));
    }
    each_d(704, N, |src| {
        let scale = (src as u32 % 32) as i32;
        assert_eq!(saturate_int_pcm_right_shift(src, scale), unsafe {
            cref_SATURATE_INT_PCM_RIGHT_SHIFT(src, scale)
        });
        assert_eq!(saturate_int_pcm_left_shift(src, scale), unsafe {
            cref_SATURATE_INT_PCM_LEFT_SHIFT(src, scale)
        });
    });
}

/// The shift amounts the macros get at the library's call sites can be out of range (e.g.
/// `fLdPow`); with a run-time amount the reference shifts modulo 32.
#[test]
fn SATURATE_SHIFT_out_of_range() {
    each_d_shift(705, N, |src, scale| {
        assert_eq!(saturate_shift(src, scale, 32), unsafe { cref_SATURATE_SHIFT_32(src, scale) }, "SATURATE_SHIFT({src:#x}, {scale}, 32)");
        assert_eq!(saturate_right_shift(src, scale, 16), unsafe { cref_SATURATE_RIGHT_SHIFT_16(src, scale) }, "SATURATE_RIGHT_SHIFT({src:#x}, {scale}, 16)");
        assert_eq!(saturate_left_shift(src, scale, 16), unsafe { cref_SATURATE_LEFT_SHIFT_16(src, scale) }, "SATURATE_LEFT_SHIFT({src:#x}, {scale}, 16)");
        assert_eq!(saturate_right_shift(src, scale, 32), unsafe { cref_SATURATE_RIGHT_SHIFT_32(src, scale) }, "SATURATE_RIGHT_SHIFT({src:#x}, {scale}, 32)");
        assert_eq!(saturate_left_shift(src, scale, 32), unsafe { cref_SATURATE_LEFT_SHIFT_32(src, scale) }, "SATURATE_LEFT_SHIFT({src:#x}, {scale}, 32)");
    });
}

#[test]
fn scaleValues() {
    let mut r = Rng::new(706);
    for i in 0..VEC_CASES {
        let n = r.range(0, 37) as usize;
        let sf = scalefactor(&mut r, i);
        // FIXP_DBL in place
        let src = vec_d(&mut r, n);
        let mut a = src.clone();
        let mut b = src.clone();
        scale_values(&mut a, sf);
        unsafe { cref_scaleValues_D(b.as_mut_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValues(FIXP_DBL*, {n}, {sf})");
        // FIXP_DBL dst, src
        let mut a = vec![0x5555_5555; n + 2];
        let mut b = a.clone();
        scale_values_to(&mut a, &src, sf);
        unsafe { cref_scaleValues_DD(b.as_mut_ptr(), src.as_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValues(FIXP_DBL*, const FIXP_DBL*, {n}, {sf})");
        // FIXP_PCM dst, FIXP_DBL src
        let mut a = vec![0x5555i16; n + 2];
        let mut b = a.clone();
        scale_values_pcm_to(&mut a, &src, sf);
        unsafe { cref_scaleValues_PCMD(b.as_mut_ptr(), src.as_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValues(FIXP_PCM*, const FIXP_DBL*, {n}, {sf})");
        // FIXP_SGL in place
        let src = vec_s(&mut r, n);
        let mut a = src.clone();
        let mut b = src.clone();
        scale_values_sgl(&mut a, sf);
        unsafe { cref_scaleValues_S(b.as_mut_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValues(FIXP_SGL*, {n}, {sf})");
        // FIXP_SGL dst, src: the source has 2 * n values, because the reference copies 2 * n
        // values for sf == 0.
        let src2 = vec_s(&mut r, 2 * n);
        let mut a = vec![0x5555i16; 2 * n + 2];
        let mut b = a.clone();
        scale_values_sgl_to(&mut a, &src2, n, sf);
        unsafe { cref_scaleValues_SS(b.as_mut_ptr(), src2.as_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValues(FIXP_SGL*, const FIXP_SGL*, {n}, {sf})");
    }
}

#[test]
fn scaleValues_SGL_SGL_copies_2_len() {
    let src: Vec<i16> = (1..=8).collect();
    let mut a = vec![0i16; 8];
    scale_values_sgl_to(&mut a, &src, 3, 0);
    let mut b = vec![0i16; 8];
    unsafe { cref_scaleValues_SS(b.as_mut_ptr(), src.as_ptr(), 3, 0) };
    assert_eq!(b, [1, 2, 3, 4, 5, 6, 0, 0]);
    assert_eq!(a, b);
}

#[test]
fn scaleValuesWithFactor() {
    let mut r = Rng::new(707);
    for i in 0..VEC_CASES {
        let n = r.range(0, 37) as usize;
        let sf = scalefactor(&mut r, i);
        let factor = if i % 7 == 0 { EDGES_D[i / 7 % EDGES_D.len()] } else { r.i32() };
        let src = vec_d(&mut r, n);
        let mut a = src.clone();
        let mut b = src.clone();
        scale_values_with_factor(&mut a, factor, sf);
        unsafe { cref_scaleValuesWithFactor(b.as_mut_ptr(), factor, n as i32, sf) };
        assert_eq!(a, b, "scaleValuesWithFactor({n}, factor {factor:#x}, sf {sf})");
    }
}

/// The variant difference: arm/scale_arm.cpp does not limit right shifts to 31 (the amount is
/// taken modulo 32 by `asr`), scale.cpp limits them.
#[test]
fn scaleValuesWithFactor_variant() {
    let mut v = [0x4000_0000i32; 1];
    // fMultDiv2(0x40000000, MAXVAL_DBL) = 0x1FFFFFFF; shift = -(-40 + 1) = 39.
    unsafe { cref_scaleValuesWithFactor(v.as_mut_ptr(), i32::MAX, 1, -40) };
    match fdk_aac_rs_fdk::ARCH {
        fdk_aac_rs_fdk::Arch::Aarch64 => assert_eq!(v[0], 0x1FFF_FFFF >> (39 % 32)),
        _ => assert_eq!(v[0], 0x1FFF_FFFF >> 31),
    }
}

#[test]
fn scaleValuesSaturate() {
    let mut r = Rng::new(708);
    for i in 0..VEC_CASES {
        let n = r.range(0, 37) as usize;
        let sf = scalefactor(&mut r, i);
        let src = vec_d(&mut r, n);
        let mut a = src.clone();
        let mut b = src.clone();
        scale_values_saturate(&mut a, sf);
        unsafe { cref_scaleValuesSaturate_D(b.as_mut_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValuesSaturate(FIXP_DBL*, {n}, {sf})");
        let mut a = vec![0x5555_5555; n + 2];
        let mut b = a.clone();
        scale_values_saturate_to(&mut a, &src, sf);
        unsafe { cref_scaleValuesSaturate_DD(b.as_mut_ptr(), src.as_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValuesSaturate(FIXP_DBL*, const FIXP_DBL*, {n}, {sf})");
        let mut a = vec![0x5555i16; n + 2];
        let mut b = a.clone();
        scale_values_saturate_dbl_to_sgl(&mut a, &src, sf);
        unsafe { cref_scaleValuesSaturate_SD(b.as_mut_ptr(), src.as_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValuesSaturate(FIXP_SGL*, const FIXP_DBL*, {n}, {sf})");
        let src = vec_s(&mut r, n);
        let mut a = src.clone();
        let mut b = src.clone();
        scale_values_saturate_sgl(&mut a, sf);
        unsafe { cref_scaleValuesSaturate_S(b.as_mut_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValuesSaturate(FIXP_SGL*, {n}, {sf})");
        let mut a = vec![0x5555i16; n + 2];
        let mut b = a.clone();
        scale_values_saturate_sgl_to(&mut a, &src, sf);
        unsafe { cref_scaleValuesSaturate_SS(b.as_mut_ptr(), src.as_ptr(), n as i32, sf) };
        assert_eq!(a, b, "scaleValuesSaturate(FIXP_SGL*, const FIXP_SGL*, {n}, {sf})");
    }
}

#[test]
fn getScalefactor() {
    let mut r = Rng::new(709);
    for i in 0..VEC_CASES {
        let n = r.range(0, 37) as usize;
        // Values of every magnitude.
        let shift = r.range(0, 31) as u32;
        let d: Vec<i32> = (0..n).map(|_| r.i32() >> shift).collect();
        assert_eq!(get_scalefactor(&d), unsafe { cref_getScalefactor_D(d.as_ptr(), n as i32) });
        let s: Vec<i16> = (0..n).map(|_| r.i16() >> (shift % 16)).collect();
        let s = if i == 0 { vec![-1i16, 0, -1] } else { s };
        let n = s.len();
        assert_eq!(get_scalefactor_sgl(&s), unsafe { cref_getScalefactor_S(s.as_ptr(), n as i32) });
        assert_eq!(get_scalefactor_short(&s), unsafe {
            cref_getScalefactorShort(s.as_ptr(), n as i32)
        });
        let stride = r.range(1, 4) as usize;
        let len = if n == 0 { 0 } else { (n - 1) / stride + 1 };
        assert_eq!(get_scalefactor_pcm(&s, len, stride), unsafe {
            cref_getScalefactorPCM(s.as_ptr(), len as i32, stride as i32)
        });
    }
}

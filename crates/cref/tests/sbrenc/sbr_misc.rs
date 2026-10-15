// Differential tests of fdk_aac_rs_sbrenc::sbr_misc against the C++ reference.
//
// The sorting routines on random vectors (with duplicates and edge values), the vector helpers
// on random contents, and FDKsbrEnc_LSI_divide_scale_fract() on random and edge operands.

use fdk_aac_rs_cref::sbrenc_sbr_misc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrenc::sbr_misc::*;

#[test]
fn shellsort() {
    let mut rng = Rng::new(0x5eed_0001);
    for _ in 0..3000 {
        let n = rng.range(0, 64);
        let mut a: Vec<i32> = (0..n.max(1)).map(|_| if rng.range(0, 3) == 0 { rng.range(-4, 4) } else { rng.i32() }).collect();
        let mut b = a.clone();
        unsafe { c::cref_sbr_misc_shellsort_fract(a.as_mut_ptr(), n) };
        fdk_sbr_enc_shellsort_fract(&mut b, n);
        assert_eq!(a, b, "Shellsort_fract n={n}");
        let mut a: Vec<i32> = (0..n.max(1)).map(|_| rng.range(-100, 100)).collect();
        let mut b = a.clone();
        unsafe { c::cref_sbr_misc_shellsort_int(a.as_mut_ptr(), n) };
        fdk_sbr_enc_shellsort_int(&mut b, n);
        assert_eq!(a, b, "Shellsort_int n={n}");
    }
}

#[test]
fn vector_helpers() {
    let mut rng = Rng::new(0x5eed_0002);
    for _ in 0..2000 {
        let len = rng.range(0, 10);
        let n = rng.range(0, 10);
        let mut a: Vec<i32> = (0..24).map(|_| rng.range(-50, 50)).collect();
        let src: Vec<i32> = (0..12).map(|_| rng.range(-50, 50)).collect();
        let mut b = a.clone();
        let (mut la, mut lb) = (len, len);
        match rng.range(0, 3) {
            0 => {
                let v = rng.i32();
                unsafe { c::cref_sbr_misc_add_left(a.as_mut_ptr(), &mut la, v) };
                fdk_sbr_enc_add_left(&mut b, &mut lb, v);
            }
            1 => {
                let v = rng.i32();
                unsafe { c::cref_sbr_misc_add_right(a.as_mut_ptr(), &mut la, v) };
                fdk_sbr_enc_add_right(&mut b, &mut lb, v);
            }
            2 => {
                let mut s = src.clone();
                unsafe { c::cref_sbr_misc_add_vec_left(a.as_mut_ptr(), &mut la, s.as_mut_ptr(), n) };
                fdk_sbr_enc_add_vec_left(&mut b, &mut lb, &src, n);
            }
            _ => {
                let mut s = src.clone();
                unsafe { c::cref_sbr_misc_add_vec_right(a.as_mut_ptr(), &mut la, s.as_mut_ptr(), n) };
                fdk_sbr_enc_add_vec_right(&mut b, &mut lb, &src, n);
            }
        }
        assert_eq!((a, la), (b, lb));
    }
}

#[test]
fn lsi_divide_scale_fract() {
    let mut rng = Rng::new(0x5eed_0003);
    // The library calls it with num >= 0, denom > 0 and scale = SFM_SCALE (mh_det.cpp: the
    // geometric and arithmetic means of non-negative tonality values). Outside of that domain
    // schur_div() asserts (num >= 0, denum > 0), and denum == 0 has no defined result on x86.
    let edge = [0, 1, 2, 3, i32::MAX, i32::MAX - 1, 0x4000_0000, 0x2000_0000, 0x1fff_ffff, 0x1000];
    let pos = |rng: &mut Rng| rng.i32().wrapping_abs().max(0);
    for k in 0..200_000 {
        let (num, denom, scale) = if k < 1000 {
            (edge[k % 10], edge[(k / 10) % 10].max(1), edge[(k / 100) % 10].max(1))
        } else {
            let s = if rng.range(0, 1) == 0 { i32::MAX >> 2 } else { pos(&mut rng).max(1) };
            (pos(&mut rng), pos(&mut rng).max(1), s)
        };
        let r = unsafe { c::cref_sbr_misc_lsi_divide_scale_fract(num, denom, scale) };
        let p = fdk_sbr_enc_lsi_divide_scale_fract(num, denom, scale);
        assert_eq!(r, p, "num={num:#x} denom={denom:#x} scale={scale:#x}");
    }
}

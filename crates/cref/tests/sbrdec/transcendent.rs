// Differential tests of fdk_aac_rs_sbrdec::transcendent against the C++ reference.
//
// The mantissa/exponent helpers of transcendent.h with random (edge-biased) mantissas over the
// whole range and exponents over the whole SCHAR range; FDK_getNumOctavesDiv8 over a range of
// band numbers.

use fdk_aac_rs_cref::sbrdec_transcendent::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::transcendent::*;

fn exponent(rng: &mut Rng) -> i8 {
    match rng.next_u32() % 4 {
        0 => rng.next_u32() as i8,
        _ => rng.range(-40, 40) as i8,
    }
}

#[test]
fn get_num_octaves_div8() {
    for a in -3..=260 {
        for b in [-1, 0, 1, 2, 7, 13, 32, 63, 64, 100, 192, 193, 256, a + 1, a + 5] {
            let r = unsafe { cref_sbrdec_get_num_octaves_div8(a, b) };
            assert_eq!(fdk_get_num_octaves_div8(a, b), r, "a {a} b {b}");
        }
    }
}

#[test]
fn add_mant_exp() {
    let mut rng = Rng::new(0x7a11);
    for it in 0..200_000 {
        let (a_e, b_e) = (exponent(&mut rng), exponent(&mut rng));
        let mut out = [0i32; 2];
        let (a, b) = (rng.i16(), rng.i16());
        unsafe { cref_sbrdec_add_mant_exp_sgl(a, a_e, b, b_e, out.as_mut_ptr()) };
        let (m, e) = fdk_add_mant_exp_sgl(a, a_e, b, b_e);
        assert_eq!([m as i32, e as i32], out, "{it}: sgl {a} {a_e} {b} {b_e}");
        let (a, b) = (rng.i32(), rng.i32());
        unsafe { cref_sbrdec_add_mant_exp(a, a_e, b, b_e, out.as_mut_ptr()) };
        let (m, e) = fdk_add_mant_exp(a, a_e, b, b_e);
        assert_eq!([m, e as i32], out, "{it}: dbl {a} {a_e} {b} {b_e}");
    }
}

#[test]
fn divide_mant_exp() {
    let mut rng = Rng::new(0xd1f);
    for it in 0..200_000 {
        let (a_e, b_e) = (exponent(&mut rng), exponent(&mut rng));
        let mut out = [0i32; 2];
        let (a, b) = (rng.i16(), rng.i16());
        unsafe { cref_sbrdec_divide_mant_exp_sgl(a, a_e, b, b_e, out.as_mut_ptr()) };
        let (m, e) = fdk_divide_mant_exp_sgl(a, a_e, b, b_e);
        assert_eq!([m as i32, e as i32], out, "{it}: sgl {a} {a_e} {b} {b_e}");
        let (a, b) = (rng.i32(), rng.i32());
        unsafe { cref_sbrdec_divide_mant_exp(a, a_e, b, b_e, out.as_mut_ptr()) };
        let (m, e) = fdk_divide_mant_exp(a, a_e, b, b_e);
        assert_eq!([m, e as i32], out, "{it}: dbl {a} {a_e} {b} {b_e}");
    }
}

#[test]
fn sqrt_mant_exp() {
    let mut rng = Rng::new(0x5947);
    for it in 0..200_000 {
        let m = rng.i32();
        let e = exponent(&mut rng);
        let same = rng.next_u32() % 2 == 0;
        let dest = exponent(&mut rng);
        let mut out = [0i32; 2];
        unsafe { cref_sbrdec_sqrt_mant_exp(m, e, same as i32, dest, out.as_mut_ptr()) };
        let (mut pm, mut pe) = (m, e);
        fdk_sqrt_mant_exp(&mut pm, &mut pe, if same { None } else { Some(dest) });
        assert_eq!([pm, pe as i32], out, "{it}: {m} {e} same {same} dest {dest}");
    }
}

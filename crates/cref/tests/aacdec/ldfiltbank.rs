// Differential tests of fdk_aac_rs_aacdec::ldfiltbank against the C++ reference.
//
// multE2_DinvF_fdk() on random inputs and InvMdctTransformLowDelay_fdk() on sequences of frames
// (the filter state carries over) for every frame length of the low delay filter bank, with
// spectra of every magnitude and exponents around the saturation.

use fdk_aac_rs_aacdec::aac_rom::*;
use fdk_aac_rs_aacdec::ldfiltbank::*;
use fdk_aac_rs_cref::aacdec_ldfiltbank as c;
use fdk_aac_rs_cref::testutil::Rng;

const LENS: [i32; 7] = [512, 480, 256, 240, 160, 128, 120];

fn window(n: i32) -> &'static [i16] {
    match n {
        256 => &LOW_DELAY_SYNTHESIS_256,
        240 => &LOW_DELAY_SYNTHESIS_240,
        160 => &LOW_DELAY_SYNTHESIS_160,
        128 => &LOW_DELAY_SYNTHESIS_128,
        120 => &LOW_DELAY_SYNTHESIS_120,
        512 => &LOW_DELAY_SYNTHESIS_512,
        _ => &LOW_DELAY_SYNTHESIS_480,
    }
}

#[test]
fn mult_e2_dinv_f() {
    let mut rng = Rng::new(0x1dfb_0001);
    for _ in 0..3000 {
        let n = LENS[(rng.next_u32() % 7) as usize];
        let nu = n as usize;
        let sh = rng.next_u32() % 32;
        let x: Vec<i32> = (0..nu).map(|_| rng.i32() >> sh).collect();
        let z: Vec<i32> = (0..nu * 3 / 2).map(|_| rng.i32() >> sh).collect();
        let mut cout = vec![0; nu];
        let mut cz = z.clone();
        let mut cx = x.clone();
        unsafe { c::cref_ldfb_mult_e2(cout.as_mut_ptr(), cx.as_mut_ptr(), cz.as_mut_ptr(), n) };
        let mut rout = vec![0; nu];
        let mut rz = z.clone();
        mult_e2_dinv_f_fdk(&mut rout, &x, window(n), &mut rz, n);
        assert_eq!((&rout, &rz), (&cout, &cz), "multE2_DinvF_fdk({n})");
    }
}

#[test]
fn inv_mdct_transform_low_delay() {
    let mut rng = Rng::new(0x1dfb_0002);
    for seq in 0..400 {
        let n = LENS[(rng.next_u32() % 7) as usize];
        let nu = n as usize;
        let mut cfs = vec![0i32; 768];
        let mut rfs = vec![0i32; 768];
        for frame in 0..6 {
            let sh = rng.next_u32() % 32;
            let spec: Vec<i32> = (0..nu).map(|_| rng.i32() >> sh).collect();
            let e = rng.range(-8, 24);
            let mut cs = spec.clone();
            let mut cout = vec![0; nu];
            let cr = unsafe { c::cref_ldfb_inv_mdct(cs.as_mut_ptr(), e, cout.as_mut_ptr(), cfs.as_mut_ptr(), n) };
            let mut rs = spec.clone();
            let mut rout = vec![0; nu];
            let rr = inv_mdct_transform_low_delay_fdk(&mut rs, e, &mut rout, &mut rfs, n);
            let what = format!("InvMdctTransformLowDelay_fdk({n}) seq {seq} frame {frame} e {e}");
            assert_eq!(rr, cr, "{what}: return");
            assert_eq!(rout, cout, "{what}: output");
            assert_eq!(rs, cs, "{what}: spectrum");
            assert_eq!(rfs, cfs, "{what}: filter state");
        }
    }
}

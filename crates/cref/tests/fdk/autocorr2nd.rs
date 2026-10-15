// Differential tests of fdk_aac_rs_fdk::autocorr2nd against the C++ reference.

use fdk_aac_rs_cref::fdk_autocorr2nd as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::autocorr2nd::*;

fn to_array(ac: &AcorrCoefs) -> [i32; 11] {
    [
        ac.r00r, ac.r11r, ac.r22r, ac.r01r, ac.r02r, ac.r12r, ac.r01i, ac.r02i, ac.r12i, ac.det,
        ac.det_scale,
    ]
}

/// A buffer of QMF values: all of one magnitude class, so that every scaling path is reached
/// (including overflowing sums and all-zero input).
fn buffer(rng: &mut Rng, n: usize, class: u32) -> Vec<i32> {
    (0..n)
        .map(|_| match class {
            0 => 0,
            1 => rng.i32(),
            2 => [i32::MIN, i32::MAX, -i32::MAX][(rng.next_u32() % 3) as usize],
            3 => (rng.next_u32() % 3) as i32 - 1,
            _ => rng.i32() >> (rng.next_u32() % 31),
        })
        .collect()
}

#[test]
fn auto_corr2nd_real_matches() {
    let mut rng = Rng::new(20);
    for iter in 0..100_000 {
        let len = match iter % 4 {
            0 => rng.range(2, 6),
            _ => rng.range(2, 80),
        };
        let class = rng.next_u32() % 6;
        let re = buffer(&mut rng, len as usize + LPC_ORDER, class);
        let mut ac = AcorrCoefs { r00r: rng.i32(), r01i: rng.i32(), ..Default::default() };
        let mut ac_c = to_array(&ac);
        let r = auto_corr2nd_real(&mut ac, &re, len);
        let e = unsafe { c::cref_autocorr2nd_real(ac_c.as_mut_ptr(), re.as_ptr(), len) };
        assert_eq!(r, e, "scaling, len {len} re {re:?}");
        assert_eq!(to_array(&ac), ac_c, "coefficients, len {len} re {re:?}");
    }
}

#[test]
fn auto_corr2nd_cplx_matches() {
    let mut rng = Rng::new(21);
    for iter in 0..100_000 {
        let len = match iter % 4 {
            0 => rng.range(1, 6),
            _ => rng.range(1, 80),
        };
        let class = rng.next_u32() % 6;
        let re = buffer(&mut rng, len as usize + LPC_ORDER, class);
        let im = buffer(&mut rng, len as usize + LPC_ORDER, if iter % 3 == 0 { class } else { 4 });
        let mut ac = AcorrCoefs::default();
        let mut ac_c = to_array(&ac);
        let r = auto_corr2nd_cplx(&mut ac, &re, &im, len);
        let e = unsafe {
            c::cref_autocorr2nd_cplx(ac_c.as_mut_ptr(), re.as_ptr(), im.as_ptr(), len)
        };
        assert_eq!(r, e, "scaling, len {len} re {re:?} im {im:?}");
        assert_eq!(to_array(&ac), ac_c, "coefficients, len {len} re {re:?} im {im:?}");
    }
}

// Differential tests of fdk_aac_rs_fdk::scramble against the C++ reference.

use fdk_aac_rs_cref::fdk_scramble::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::scramble::*;

#[test]
fn scramble_all_lengths() {
    let mut r = Rng::new(0x5c4a_0001);
    for ld in 0..=11 {
        let n = 1i32 << ld;
        for _ in 0..20 {
            let x: Vec<i32> = (0..2 * n as usize).map(|_| r.i32()).collect();
            let mut a = x.clone();
            let mut b = x.clone();
            scramble(&mut a, n);
            unsafe { cref_scramble_scramble(b.as_mut_ptr(), n) };
            assert_eq!(a, b, "scramble length {n}");
        }
    }
}

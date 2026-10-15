// Differential tests of fdk_aac_rs_sbrdec::sbr_dec against the C++ reference.
//
// copyHarmonicSpectrum (file-static, reached through a namespaced copy of sbr_dec.cpp) with
// random cross-over bands of the 4:1 transposer (up to 5 patches; the source patch at least 2
// bands wide, narrower ones have no defined result in the reference), QMF data, overlaps and all
// keep-states modes. sbr_dec(), createSbrDec(), deleteSbrDec() and resetSbrDec() are compared through the
// SBR decoder API with real and random data: tests/sbrdec/e2e.rs and api_fuzz.rs.

use fdk_aac_rs_cref::sbrdec_sbr_dec::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::hbe::{MAX_NUM_PATCHES_HBE, MAX_STRETCH_HBE};
use fdk_aac_rs_sbrdec::sbr_dec::copy_harmonic_spectrum;

#[test]
fn copy_harmonic() {
    let mut rng = Rng::new(0xc0b);
    for it in 0..50_000 {
        let mut x = [0i32; MAX_NUM_PATCHES_HBE];
        let n = rng.range(2, MAX_NUM_PATCHES_HBE as i32) as usize;
        x[0] = rng.range(4, 16);
        for k in 1..n {
            x[k] = (x[k - 1] + rng.range(1, 16)).min(64);
        }
        // (the source patch xOverQmf[MAX_STRETCH_HBE - 2..=MAX_STRETCH_HBE - 1])
        if n > MAX_STRETCH_HBE && x[MAX_STRETCH_HBE - 1] - x[MAX_STRETCH_HBE - 2] < 2 {
            continue;
        }
        let no_cols = [32, 64][rng.range(0, 1) as usize];
        let overlap = [6, 12][rng.range(0, 1) as usize];
        let n_rows = (overlap + no_cols + 2) as usize;
        let mode = rng.range(0, 3);
        let re: Vec<i32> = (0..n_rows * 64).map(|_| rng.i32()).collect();
        let im: Vec<i32> = (0..n_rows * 64).map(|_| rng.i32()).collect();
        let (mut rc, mut ic) = (re.clone(), im.clone());
        let mut xc = x;
        unsafe { cref_sbrdec_sbr_dec_copy_harmonic(xc.as_mut_ptr(), rc.as_mut_ptr(), ic.as_mut_ptr(), n_rows as i32, no_cols, overlap, mode) };
        let (mut rp, mut ip) = (re.clone(), im.clone());
        {
            let mut r: Vec<&mut [i32]> = rp.chunks_mut(64).collect();
            let mut i: Vec<&mut [i32]> = ip.chunks_mut(64).collect();
            copy_harmonic_spectrum(&x, &mut r, &mut i, no_cols, overlap, mode);
        }
        assert!(rp == rc && ip == ic, "{it}: xOverQmf {x:?} cols {no_cols} overlap {overlap} mode {mode}");
    }
}

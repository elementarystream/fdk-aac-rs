// Differential tests of fdk_aac_rs_sbrdec::hfgen_pre_flat against the C++ reference.
//
// The file-static solver functions (forward and backward substitution, Cholesky solver,
// polynomial fit and evaluation; reached through a namespaced copy of HFgen_preFlat.cpp) with
// random values and exponents for all band counts of the solver tables, and
// sbrDecoder_calculateGainVec with random QMF data (also silent bands), exponents, overlaps and
// slot ranges for 1..=32 bands. The low power case (no imaginary data) is not compared: the
// reference has no defined result there (see imag_at()). Real data: tests/sbrdec/e2e.rs.

use fdk_aac_rs_cref::sbrdec_hfgen_pre_flat::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::hfgen_pre_flat::*;

fn vals(rng: &mut Rng) -> ([i32; 4], [i32; 4]) {
    let mut v = [0i32; 4];
    let mut e = [0i32; 4];
    for k in 0..4 {
        v[k] = rng.i32();
        e[k] = rng.range(-40, 40);
    }
    (v, e)
}

#[test]
fn solver() {
    let mut rng = Rng::new(0x501);
    for it in 0..100_000 {
        let nb = rng.range(5, 32);
        let (b, _) = vals(&mut rng);
        let (x0, e0) = vals(&mut rng);
        let (mut xc, mut ec) = (x0, e0);
        let (mut xp, mut ep) = (x0, e0);
        let mut bc = b;
        unsafe { cref_sbrdec_hfgen_backsubst_fw(nb, bc.as_mut_ptr(), xc.as_mut_ptr(), ec.as_mut_ptr()) };
        backsubst_fw(nb, &b, &mut xp, &mut ep);
        assert_eq!((xp, ep), (xc, ec), "{it}: backsubst_fw {nb}");

        let (mut xc, mut ec) = (x0, e0);
        let (mut xp, mut ep) = (x0, e0);
        unsafe { cref_sbrdec_hfgen_backsubst_bw(nb, bc.as_mut_ptr(), xc.as_mut_ptr(), ec.as_mut_ptr()) };
        backsubst_bw(nb, &b, &mut xp, &mut ep);
        assert_eq!((xp, ep), (xc, ec), "{it}: backsubst_bw {nb}");

        let (mut xc, mut ec) = (x0, e0);
        let (mut xp, mut ep) = (x0, e0);
        unsafe { cref_sbrdec_hfgen_cholesky_solve(nb, xc.as_mut_ptr(), ec.as_mut_ptr()) };
        cholesky_solve(nb, &mut xp, &mut ep);
        assert_eq!((xp, ep), (xc, ec), "{it}: cholesky_solve {nb}");

        let mut y: Vec<i32> = (0..32).map(|_| rng.i32() >> rng.range(0, 8)).collect();
        if rng.next_u32() % 8 == 0 {
            y.iter_mut().for_each(|v| *v = 0);
        }
        let y_sf = rng.range(-10, 20);
        let (mut pc, mut sc) = ([0i32; 4], [0i32; 4]);
        let (mut pp, mut sp) = ([0i32; 4], [0i32; 4]);
        unsafe { cref_sbrdec_hfgen_polyfit(nb, y.as_mut_ptr(), y_sf, pc.as_mut_ptr(), sc.as_mut_ptr()) };
        polyfit(nb, &y, y_sf, &mut pp, &mut sp);
        assert_eq!((pp, sp), (pc, sc), "{it}: polyfit {nb} {y_sf}");

        for x in [0, 1, rng.range(0, 31), 31] {
            let (mut oc, mut op) = (0, 0);
            let rc = unsafe { cref_sbrdec_hfgen_polyval(pc.as_mut_ptr(), sc.as_mut_ptr(), x, &mut oc) };
            let rp = polyval(&pp, &sp, x, &mut op);
            assert_eq!((rp, op), (rc, oc), "{it}: polyval of the fit at {x}");
        }
        let (p, e) = vals(&mut rng);
        let (mut pc, mut sc) = (p, e);
        let x = rng.range(0, 31);
        let (mut oc, mut op) = (0, 0);
        let rc = unsafe { cref_sbrdec_hfgen_polyval(pc.as_mut_ptr(), sc.as_mut_ptr(), x, &mut oc) };
        let rp = polyval(&p, &e, x, &mut op);
        assert_eq!((rp, op), (rc, oc), "{it}: polyval {p:?} {e:?} {x}");
    }
}

#[test]
fn calculate_gain_vec() {
    let mut rng = Rng::new(0x9a1);
    for it in 0..20_000 {
        let overlap = [0, 6, 8, 12, 16][rng.range(0, 4) as usize];
        let n_slots = overlap + rng.range(1, 64);
        let start = rng.range(0, overlap.min(4));
        let stop = rng.range(start + 1, n_slots);
        let nb = rng.range(1, 32);
        let shift = rng.range(0, 31);
        let mut re: Vec<i32> = (0..n_slots * 64).map(|_| rng.i32() >> shift).collect();
        let mut im: Vec<i32> = (0..n_slots * 64).map(|_| rng.i32() >> shift).collect();
        // silent bands
        for b in 0..64 {
            if rng.next_u32() % 6 == 0 {
                for s in 0..n_slots as usize {
                    re[s * 64 + b] = 0;
                    im[s * 64 + b] = 0;
                }
            }
        }
        let e_ov = rng.range(-20, 20);
        let e_cur = if rng.next_u32() % 2 == 0 { e_ov } else { rng.range(-20, 20) };
        let (mut gc, mut gec) = ([0i32; 64], [0i32; 64]);
        let (mut gp, mut gep) = ([0i32; 64], [0i32; 64]);
        let (mut rc, mut ic) = (re.clone(), im.clone());
        unsafe {
            cref_sbrdec_hfgen_gain_vec(
                rc.as_mut_ptr(),
                ic.as_mut_ptr(),
                n_slots,
                e_ov,
                e_cur,
                overlap,
                gc.as_mut_ptr(),
                gec.as_mut_ptr(),
                nb,
                start,
                stop,
            )
        };
        let r: Vec<&mut [i32]> = re.chunks_mut(64).collect();
        let i: Vec<&mut [i32]> = im.chunks_mut(64).collect();
        sbr_decoder_calculate_gain_vec(&r, &i, e_ov, e_cur, overlap, &mut gp, &mut gep, nb, start, stop);
        assert_eq!((gp, gep), (gc, gec), "{it}: bands {nb} overlap {overlap} slots {start}..{stop} e {e_ov} {e_cur}");
    }
}

// Differential tests of fdk_aac_rs_sacenc::sacenc_vectorfunctions against the C++ reference.
//
// Every function of sacenc_vectorfunctions.cpp on random complex vectors (values of every
// magnitude, including MINVAL_DBL and MAXVAL_DBL), random dimensions and scale factors in and
// beyond the range the library uses (the shift amounts reach 32..62).

use fdk_aac_rs_cref::sacenc_sacenc_vectorfunctions as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::FixpDpk;
use fdk_aac_rs_sacenc::sacenc_vectorfunctions as r;

/// A random value: full range, small, or headroom-limited like hybrid data.
pub fn val(rng: &mut Rng, mag: u32) -> i32 {
    match rng.next_u32() % 4 {
        0 => rng.i32(),
        _ => (rng.next_u32() as i32) >> mag,
    }
}

pub fn cvec(rng: &mut Rng, n: usize) -> Vec<FixpDpk> {
    let mag = rng.next_u32() % 32;
    (0..n).map(|_| FixpDpk::new(val(rng, mag), val(rng, mag))).collect()
}

pub fn cmat(rng: &mut Rng, rows: usize, cols: usize) -> Vec<Vec<FixpDpk>> {
    let mag = rng.next_u32() % 32;
    (0..rows).map(|_| (0..cols).map(|_| FixpDpk::new(val(rng, mag), val(rng, mag))).collect()).collect()
}

pub fn flat(m: &[Vec<FixpDpk>]) -> Vec<i32> {
    m.iter().flat_map(|r| r.iter().flat_map(|v| [v.re, v.im])).collect()
}

fn flat1(v: &[FixpDpk]) -> Vec<i32> {
    v.iter().flat_map(|v| [v.re, v.im]).collect()
}

fn scale(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 4 {
        0 => rng.range(-40, 40),
        _ => rng.range(-31, 10),
    }
}

#[test]
fn sum_up_cplx_pow2() {
    let mut rng = Rng::new(1);
    for it in 0..20000 {
        let n = rng.range(0, 70) as usize;
        let x = cvec(&mut rng, n.max(1));
        let mode = rng.range(0, 1);
        let sf = scale(&mut rng);
        let (mut oc, mut or) = (0, 0);
        let rc = unsafe {
            c::cref_sacenc_vectorfunctions_sumUpCplxPow2(flat1(&x).as_ptr(), mode, sf, &mut oc, n as i32)
        };
        let rr = r::sum_up_cplx_pow2(&x, mode, sf, &mut or, n as i32);
        assert_eq!((rc, oc), (rr, or), "it {it} n {n} mode {mode} sf {sf}");
    }
}

#[test]
fn sum_up_cplx_pow2_dim2() {
    let mut rng = Rng::new(2);
    for it in 0..10000 {
        let rows = rng.range(1, 20) as usize;
        let cols = rng.range(1, 40) as usize;
        let m = cmat(&mut rng, rows, cols);
        let mode = rng.range(0, 1);
        let sf = scale(&mut rng);
        let s1 = rng.range(0, rows as i32);
        let n1 = rng.range(s1, rows as i32);
        let s2 = rng.range(0, cols as i32);
        let n2 = rng.range(s2, cols as i32);
        let (mut oc, mut or) = (0, 0);
        let rc = unsafe {
            c::cref_sacenc_vectorfunctions_sumUpCplxPow2Dim2(
                flat(&m).as_ptr(),
                rows as i32,
                cols as i32,
                mode,
                sf,
                &mut oc,
                s1,
                n1,
                s2,
                n2,
            )
        };
        let rr = r::sum_up_cplx_pow2_dim2(&m, mode, sf, &mut or, s1, n1, s2, n2);
        assert_eq!((rc, oc), (rr, or), "it {it}");
    }
}

#[test]
fn copy_and_set_cplx_vec() {
    let mut rng = Rng::new(3);
    for _ in 0..2000 {
        let n = rng.range(1, 70) as usize;
        let x = cvec(&mut rng, n);
        let z0 = cvec(&mut rng, n);
        let k = rng.range(0, n as i32);
        let mut zc = flat1(&z0);
        unsafe { c::cref_sacenc_vectorfunctions_copyCplxVec(zc.as_mut_ptr(), flat1(&x).as_ptr(), k) };
        let mut zr = z0.clone();
        r::copy_cplx_vec(&mut zr, &x, k);
        assert_eq!(zc, flat1(&zr));

        let a = rng.i32();
        let mut zc = flat1(&z0);
        unsafe { c::cref_sacenc_vectorfunctions_setCplxVec(zc.as_mut_ptr(), a, k) };
        let mut zr = z0.clone();
        r::set_cplx_vec(&mut zr, a, k);
        assert_eq!(zc, flat1(&zr));
    }
}

#[test]
fn cplx_scalar_product() {
    let mut rng = Rng::new(4);
    for it in 0..20000 {
        let rows = rng.range(1, 20) as usize;
        let cols = rng.range(1, 40) as usize;
        let x = cmat(&mut rng, rows, cols);
        let y = cmat(&mut rng, rows, cols);
        let sx = scale(&mut rng);
        let sy = scale(&mut rng);
        let s1 = rng.range(0, rows as i32);
        let n1 = rng.range(s1, rows as i32);
        let s2 = rng.range(0, cols as i32);
        let n2 = rng.range(s2, cols as i32);
        let mut zc = [rng.i32(), rng.i32()];
        let mut zr = FixpDpk::new(zc[0], zc[1]);
        let (mut scc, mut scr) = (0, 0);
        unsafe {
            c::cref_sacenc_vectorfunctions_cplxScalarProduct(
                zc.as_mut_ptr(),
                flat(&x).as_ptr(),
                flat(&y).as_ptr(),
                rows as i32,
                cols as i32,
                sx,
                sy,
                &mut scc,
                s1,
                n1,
                s2,
                n2,
            )
        };
        r::cplx_cplx_scalar_product(&mut zr, &x, &y, sx, sy, &mut scr, s1, n1, s2, n2);
        assert_eq!((zc[0], zc[1], scc), (zr.re, zr.im, scr), "it {it} sx {sx} sy {sy}");
    }
}

#[test]
fn correlation_and_coherence() {
    let mut rng = Rng::new(5);
    for it in 0..20000 {
        let n = rng.range(0, 28) as usize;
        let mag = rng.next_u32() % 32;
        let v = |rng: &mut Rng| -> Vec<i32> { (0..n.max(1)).map(|_| val(rng, mag)).collect() };
        let pr12 = v(&mut rng);
        let pi12 = v(&mut rng);
        let p1: Vec<i32> = v(&mut rng).iter().map(|x| if it % 3 == 0 { *x } else { x.wrapping_abs() }).collect();
        let p2: Vec<i32> = v(&mut rng).iter().map(|x| if it % 5 == 0 { *x } else { x.wrapping_abs() }).collect();
        let z0: Vec<i32> = (0..n.max(1)).map(|_| rng.i32()).collect();

        let mut zc = z0.clone();
        let mut zr = z0.clone();
        unsafe {
            c::cref_sacenc_vectorfunctions_FDKcalcCorrelationVec(
                zc.as_mut_ptr(),
                pr12.as_ptr(),
                p1.as_ptr(),
                p2.as_ptr(),
                n as i32,
            )
        };
        r::fdk_calc_correlation_vec(&mut zr, &pr12, &p1, &p2, n as i32);
        assert_eq!(zc, zr, "correlation it {it}");

        let sp12 = if it % 2 == 0 { 0 } else { rng.range(-40, 40) };
        let sp = if it % 2 == 0 { 0 } else { rng.range(-40, 40) };
        let mut zc = z0.clone();
        let mut zr = z0.clone();
        unsafe {
            c::cref_sacenc_vectorfunctions_calcCoherenceVec(
                zc.as_mut_ptr(),
                pr12.as_ptr(),
                pi12.as_ptr(),
                p1.as_ptr(),
                p2.as_ptr(),
                sp12,
                sp,
                n as i32,
            )
        };
        r::calc_coherence_vec(&mut zr, &pr12, &pi12, &p1, &p2, sp12, sp, n as i32);
        assert_eq!(zc, zr, "coherence it {it}");
    }
}

/// A random parameter band offset table (increasing, last entry <= max).
pub fn offsets(rng: &mut Rng, n_pb: usize, max: usize) -> Vec<u8> {
    let mut o: Vec<u8> = (0..n_pb).map(|_| rng.range(0, max as i32) as u8).collect();
    o.sort();
    o
}

#[test]
fn add_weighted_cplx_vec() {
    let mut rng = Rng::new(6);
    for it in 0..5000 {
        let bands = rng.range(1, 40) as usize;
        let slots = rng.range(1, 20) as usize;
        let x = cmat(&mut rng, bands, slots);
        let y = cmat(&mut rng, bands, slots);
        let z0 = cmat(&mut rng, bands, slots);
        let n_pb = rng.range(0, 28) as usize;
        let off = offsets(&mut rng, n_pb, bands);
        let a: Vec<i32> = (0..n_pb.max(1)).map(|_| rng.i32()).collect();
        let b: Vec<i32> = (0..n_pb.max(1)).map(|_| rng.i32()).collect();
        let sc = rng.range(-2, 33);
        let st = rng.range(0, slots as i32);
        let nt = rng.range(st, slots as i32);
        let sch2 = rng.range(-40, 40);
        let mut s1c = rng.range(-40, 40);
        let mut s1r = s1c;
        let mut zc = flat(&z0);
        unsafe {
            c::cref_sacenc_vectorfunctions_addWeightedCplxVec(
                zc.as_mut_ptr(),
                a.as_ptr(),
                flat(&x).as_ptr(),
                b.as_ptr(),
                flat(&y).as_ptr(),
                bands as i32,
                slots as i32,
                sc,
                &mut s1c,
                sch2,
                off.as_ptr(),
                n_pb as i32,
                nt,
                st,
            )
        };
        let mut zr = z0.clone();
        r::add_weighted_cplx_vec(&mut zr, &a, &x, &b, &y, sc, &mut s1r, sch2, &off, n_pb as i32, nt, st);
        assert_eq!((zc, s1c), (flat(&zr), s1r), "it {it}");
    }
}

#[test]
fn calc_pb_scale_factor() {
    let mut rng = Rng::new(7);
    for it in 0..10000 {
        let slots = rng.range(1, 20) as usize;
        let bands = rng.range(1, 64) as usize;
        let x = cmat(&mut rng, slots, bands);
        let n_pb = rng.range(0, 28) as usize;
        let off = offsets(&mut rng, n_pb, bands);
        let st = rng.range(0, slots as i32);
        let nt = rng.range(st, slots as i32);
        let mut oc = vec![77; 28];
        let mut or = oc.clone();
        unsafe {
            c::cref_sacenc_vectorfunctions_FDKcalcPbScaleFactor(
                flat(&x).as_ptr(),
                slots as i32,
                bands as i32,
                off.as_ptr(),
                oc.as_mut_ptr(),
                st,
                nt,
                n_pb as i32,
            )
        };
        r::fdk_calc_pb_scale_factor(&x, &off, &mut or, st, nt, n_pb as i32);
        assert_eq!(oc, or, "it {it}");
    }
}

#[test]
fn calc_scale_factor() {
    let mut rng = Rng::new(8);
    for it in 0..10000 {
        let n = rng.range(0, 64) as usize;
        let mag = rng.next_u32() % 33;
        let x: Vec<i32> =
            (0..n.max(1)).map(|_| if mag == 32 { 0 } else { val(&mut rng, mag) }).collect();
        let y: Vec<i32> = (0..n.max(1)).map(|_| val(&mut rng, mag.min(31))).collect();
        let which = rng.range(0, 3);
        let xp = if which & 1 != 0 { x.as_ptr() } else { core::ptr::null() };
        let yp = if which & 2 != 0 { y.as_ptr() } else { core::ptr::null() };
        let rc = unsafe { c::cref_sacenc_vectorfunctions_FDKcalcScaleFactor(xp, yp, n as i32) };
        let rr = r::fdk_calc_scale_factor(
            (which & 1 != 0).then_some(&x[..]),
            (which & 2 != 0).then_some(&y[..]),
            n as i32,
        );
        assert_eq!(rc, rr, "it {it} n {n} which {which} x {:?} y {:?}", &x[..n], &y[..n]);

        let v = cvec(&mut rng, n.max(1));
        let sb = rng.range(0, n as i32);
        let nb = rng.range(0, n as i32);
        let rc = unsafe {
            c::cref_sacenc_vectorfunctions_FDKcalcScaleFactorDPK(flat1(&v).as_ptr(), n.max(1) as i32, sb, nb)
        };
        assert_eq!(rc, r::fdk_calc_scale_factor_dpk(&v, sb, nb), "dpk it {it}");
    }
}

#[test]
fn memcpy_and_memset_flex() {
    let mut rng = Rng::new(9);
    for _ in 0..5000 {
        let src: Vec<i32> = (0..64).map(|_| rng.i32()).collect();
        let dst0: Vec<i32> = (0..64).map(|_| rng.i32()).collect();
        let n = rng.range(0, 10);
        let ds = rng.range(-3, 3);
        let ss = rng.range(-3, 3);
        let doff = 32 + rng.range(-2, 2);
        let soff = 32 + rng.range(-2, 2);
        let mut dc = dst0.clone();
        unsafe { c::cref_sacenc_vectorfunctions_memcpy_flex(dc.as_mut_ptr(), doff, ds, src.as_ptr(), soff, ss, n) };
        let mut dr = dst0.clone();
        r::fdk_memcpy_flex(&mut dr, doff as usize, ds, &src, soff as usize, ss, n);
        assert_eq!(dc, dr);
        let v = rng.i32();
        let n = rng.range(-2, 64);
        let mut dc = dst0.clone();
        unsafe { c::cref_sacenc_vectorfunctions_memset_flex(dc.as_mut_ptr(), v, n) };
        let mut dr = dst0.clone();
        r::fdk_memset_flex(&mut dr, v, n);
        assert_eq!(dc, dr);
    }
}

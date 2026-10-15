// Differential tests of fdk_aac_rs_sbrdec::hbe against the C++ reference.
//
// The file-static helpers (the inverse root functions, scaleUp, calculateCenterFIXP,
// getHBEScaleFactorFrame, addHighBandPart; reached through a namespaced copy of hbe.cpp) with
// random values and exponents, and the transposer itself: QmfTransposerCreate for the frame
// sizes of 8:3, 2:1 and 4:1 SBR (with and without cross products), QmfTransposerReInit with the
// frequency band tables of random USAC headers, then sequences of QmfTransposerApply with
// random QMF data, LPC filter states, pitches, scales, time steps, borders and all
// keep-states modes; the complete transposer state (the state dump of tests/sbrdec/e2e.rs),
// the QMF slots, the filter states and the scale are compared after every call. Real data:
// tests/sbrdec/e2e.rs (USAC eSBR streams).

use super::e2e::{dump_hbe_reference, first_diff, Dump};
use super::env_extr::random_header;
use fdk_aac_rs_cref::sbrdec_hbe::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::env_extr::*;
use fdk_aac_rs_sbrdec::hbe::*;

#[test]
fn helpers() {
    let mut rng = Rng::new(0x4be);
    for it in 0..200_000 {
        let which = rng.range(2, 4);
        let m = (rng.i32() & 0x7FFF_FFFF).max(1);
        let e0 = rng.range(-40, 40);
        let (mut ec, mut ep) = (e0, e0);
        let rc = unsafe { cref_sbrdec_hbe_inv_root(which, m, &mut ec) };
        let rp = match which {
            2 => inv_fourth_root_norm2(m, &mut ep),
            3 => inv_cube_root_norm2(m, &mut ep),
            _ => inv3_eigth_root_norm2(m, &mut ep),
        };
        assert_eq!((rp, ep), (rc, ec), "{it}: root {which} of {m} {e0}");

        let (re, im, e) = (rng.i32(), rng.i32(), rng.range(0, 40));
        let (mut rc, mut ic, mut ec) = (re, im, e);
        let (mut rp, mut ip, mut ep) = (re, im, e);
        unsafe { cref_sbrdec_hbe_scale_up(&mut rc, &mut ic, &mut ec) };
        scale_up(&mut rp, &mut ip, &mut ep);
        assert_eq!((rp, ip, ep), (rc, ic, ec), "{it}: scaleUp {re} {im} {e}");

        let shift = rng.range(0, 31);
        let (gr, gi) = (rng.i32() >> shift, rng.i32() >> shift);
        let (stretch, mult) = (rng.range(2, 4), rng.range(0, 2));
        let e = rng.range(0, 30);
        let mut cc = [0i32; 2];
        let mut ec = e;
        unsafe { cref_sbrdec_hbe_center(gr, gi, cc.as_mut_ptr(), &mut ec, stretch, mult) };
        let (mut cr, mut ci, mut ep) = (0, 0, e);
        calculate_center_fixp(gr, gi, &mut cr, &mut ci, &mut ep, stretch, mult);
        assert_eq!(([cr, ci], ep), (cc, ec), "{it}: calculateCenterFIXP {gr} {gi} {e} {stretch} {mult}");

        let (b41, ms, pitch) = (rng.range(0, 1), rng.range(2, 4), rng.range(0, 127));
        assert_eq!(get_hbe_scale_factor_frame(b41, ms, pitch), unsafe { cref_sbrdec_hbe_scale_factor_frame(b41, ms, pitch) });

        let (gr, gi, ge) = (rng.i32() >> rng.range(0, 31), rng.i32() >> rng.range(0, 31), rng.range(0, 30));
        let mult = rng.i32();
        let (cr, ci, ce) = (rng.i32() >> rng.range(0, 8), rng.i32() >> rng.range(0, 8), rng.range(0, 30));
        let sf = rng.range(20, 30);
        let acc0 = [rng.i32() >> 4, rng.i32() >> 4];
        let mut ac = acc0;
        unsafe { cref_sbrdec_hbe_add_high_band(gr, gi, ge, mult, cr, ci, ce, stretch, sf, ac.as_mut_ptr()) };
        let mut ap = acc0;
        let (a0, a1) = ap.split_at_mut(1);
        add_high_band_part(gr, gi, ge, mult, cr, ci, ce, stretch, sf, &mut a0[0], &mut a1[0]);
        assert_eq!(ap, ac, "{it}: addHighBandPart");
    }
}

fn compare_state(h: &HbeTransposer, what: &str) {
    let mut d = Dump(Vec::new());
    d.hbe(Some(h));
    let c = dump_hbe_reference(unsafe { cref_sbrdec_hbe_handle() });
    if let Some(e) = first_diff(&d.0, &c) {
        panic!("{what}: state: {e}");
    }
}

#[test]
fn transposer() {
    let _lock = super::e2e::harness_lock(); // (the transposer of shim/sbrdec_hbe.cpp)
    let mut rng = Rng::new(0x7a5);
    let mut applies = 0;
    for it in 0..400 {
        let (frame, b41) = [(768, 0), (1024, 0), (1024, 1)][rng.range(0, 2) as usize];
        let disable = rng.range(0, 1);
        let mut flags = SBRDEC_SYNTAX_USAC | SBRDEC_USAC_HARMONICSBR;
        if b41 != 0 {
            flags |= SBRDEC_QUAD_RATE;
        }
        let Some(hd) = random_header(&mut rng, flags) else {
            continue;
        };
        let mut port: Option<Box<HbeTransposer>> = None;
        let rp = qmf_transposer_create(&mut port, frame, disable, b41);
        let rc = unsafe { cref_sbrdec_hbe_create(frame, disable, b41) };
        assert_eq!(rp, rc, "{it}: QmfTransposerCreate({frame}, {disable}, {b41})");
        let h = port.as_deref_mut().unwrap();
        compare_state(h, &format!("{it}: create"));

        let fbd = &hd.freq_band_data;
        let (mut lo, mut hi) = (fbd.freq_band_table_lo, fbd.freq_band_table_hi);
        let rc = unsafe { cref_sbrdec_hbe_reinit(lo.as_mut_ptr(), hi.as_mut_ptr(), fbd.n_sfb[0], fbd.n_sfb[1]) };
        let rp = qmf_transposer_re_init(Some(&mut *h), [&lo[..], &hi[..]], fbd.n_sfb);
        assert_eq!(rp, rc, "{it}: QmfTransposerReInit");
        compare_state(h, &format!("{it}: reinit"));
        let mut xc = [0i32; 8];
        let gc = unsafe { cref_sbrdec_hbe_get(xc.as_mut_ptr()) };
        let xp = getx_over_band_qmf_transposer(Some(&*h)).copied().unwrap();
        assert_eq!(get41_sbr_qmf_transposer(Some(&*h)), gc, "{it}: Get41SbrQmfTransposer");
        assert_eq!(&xp[..], &xc[..xp.len()], "{it}: GetxOverBandQmfTransposer");
        if rc != 0 {
            unsafe { cref_sbrdec_hbe_close() };
            qmf_transposer_close(&mut port);
            continue;
        }

        let n_cols = h.no_cols;
        let ov = if b41 != 0 { 12 } else { 6 };
        let time_step = if b41 != 0 { 4 } else { 2 };
        let mut scale_hb = rng.range(-5, 20);
        let mut lpc_re: Vec<i32> = (0..14 * 64).map(|_| rng.i32() >> 6).collect();
        let mut lpc_im: Vec<i32> = (0..14 * 64).map(|_| rng.i32() >> 6).collect();
        for f in 0..12 {
            let what = format!("{it}/{f}: frame {frame} 4:1 {b41}");
            let n_rows = (ov + 2 * n_cols) as usize;
            let codec_offset = if b41 != 0 && rng.next_u32() % 2 == 0 { ov + n_cols - 32 } else { ov + n_cols };
            let shift = rng.range(2, 20);
            let mut re: Vec<i32> = (0..n_rows * 64).map(|_| rng.i32() >> shift).collect();
            let mut im: Vec<i32> = (0..n_rows * 64).map(|_| rng.i32() >> shift).collect();
            if rng.next_u32() % 8 == 0 {
                re.iter_mut().chain(im.iter_mut()).for_each(|v| *v = 0);
            }
            let pitch = if rng.next_u32() % 3 == 0 { 0 } else { rng.range(0, 127) };
            let (scale_lb, scale_hbe) = (rng.range(-5, 20), rng.range(-5, 20));
            let first = rng.range(0, 3);
            let mode = rng.range(0, 3);
            let (mut rc_, mut ic_) = (re.clone(), im.clone());
            let (mut lrc, mut lic) = (lpc_re.clone(), lpc_im.clone());
            let mut shc = scale_hb;
            unsafe {
                cref_sbrdec_hbe_apply(
                    rc_.as_mut_ptr(),
                    ic_.as_mut_ptr(),
                    n_rows as i32,
                    codec_offset,
                    n_cols,
                    lrc.as_mut_ptr(),
                    lic.as_mut_ptr(),
                    pitch,
                    scale_lb,
                    scale_hbe,
                    &mut shc,
                    time_step,
                    first,
                    ov,
                    mode,
                )
            };
            {
                let mut r: Vec<&mut [i32]> = re.chunks_mut(64).collect();
                let mut i: Vec<&mut [i32]> = im.chunks_mut(64).collect();
                qmf_transposer_apply(
                    h,
                    &mut r,
                    &mut i,
                    codec_offset as usize,
                    n_cols,
                    Some((&mut lpc_re, &mut lpc_im)),
                    pitch,
                    scale_lb,
                    scale_hbe,
                    &mut scale_hb,
                    time_step,
                    first,
                    ov,
                    mode,
                );
            }
            applies += 1;
            let w = format!("{what}: pitch {pitch} scales {scale_lb} {scale_hbe} first {first} mode {mode} offset {codec_offset}");
            assert_eq!(scale_hb, shc, "{w}: scale_hb");
            if let Some(k) = (0..re.len()).find(|&k| re[k] != rc_[k] || im[k] != ic_[k]) {
                panic!("{w}: slot {} band {}: port {} {}, reference {} {}", k / 64, k % 64, re[k], im[k], rc_[k], ic_[k]);
            }
            assert!(lpc_re == lrc && lpc_im == lic, "{w}: LPC filter states");
            compare_state(h, &w);
        }
        unsafe { cref_sbrdec_hbe_close() };
        qmf_transposer_close(&mut port);
    }
    assert!(applies > 2000, "{applies} applies");
}

// Differential tests of fdk_aac_rs_sbrdec::lpp_tran against the C++ reference.
//
// createLppTransposer/resetLppTransposer on random headers (all syntaxes, sample rates and
// frequency tables, see tests/sbrdec/env_extr.rs) and on random master tables; lppTransposer
// on random QMF data of all magnitudes with random filter states, inverse filtering modes,
// pre-whitening, slot offsets, low power and high quality, carried over several frames. The
// settings, the transposer state, the QMF data, the scale factors and degreeAlias are compared.

use super::env_extr::{ptr, random_flags, random_header};
use fdk_aac_rs_cref::sbrdec_lpp_tran::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::qmf::QmfScaleFactor;
use fdk_aac_rs_sbrdec::lpp_tran::*;
use fdk_aac_rs_sbrdec::sbr_ram::{pod_bytes, pod_bytes_mut};

#[test]
fn reset_random_headers() {
    // Frequency tables of random headers (all syntaxes and rates, see tests/sbrdec/env_extr.rs)
    // with random cross-over bands, sample rates and slot counts. (Arbitrary master and noise
    // tables can have no defined result in the reference.)
    let mut rng = Rng::new(41);
    let mut runs = 0;
    for it in 0..20000 {
        let (flags, overlap) = random_flags(&mut rng);
        let Some(h) = random_header(&mut rng, flags) else {
            continue;
        };
        let f = &h.freq_band_data;
        let mut vk = f.v_k_master;
        let mut nt = f.freq_band_table_noise;
        let hb = vk[rng.range(0, (f.num_master as i32).min(7)) as usize] as i32;
        // fs 0 only on AArch64 (no defined result on x86)
        let fs_first = if cfg!(target_arch = "aarch64") { 0 } else { 1 };
        let fs = [0u32, 8000, 16000, 22050, 32000, 44100, 48000, 96000, h.sbr_proc_smpl_rate]
            [rng.range(fs_first, 8) as usize];
        let mut sc = TransposerSettings::ZERO;
        sc.n_cols = (h.number_time_slots * h.time_step) as u8;
        sc.overlap = overlap as u8;
        let mut sp = sc;
        let rc = unsafe {
            cref_sbrdec_lpp_tran_reset(
                ptr(&mut sc),
                hb,
                vk.as_mut_ptr(),
                f.num_master as i32,
                nt.as_mut_ptr(),
                f.n_nfb as i32,
                f.high_subband as i32,
                fs,
            )
        };
        let rp = reset_lpp_transposer(&mut sp, hb as u8, &vk, f.num_master, &nt, f.n_nfb, f.high_subband, fs);
        assert_eq!(rp, rc, "resetLppTransposer {it}");
        assert_eq!(pod_bytes(&sp), pod_bytes(&sc), "resetLppTransposer settings {it}");
        runs += 1;
    }
    assert!(runs > 5000, "runs {runs}");
}

#[test]
fn transposer_random() {
    let mut rng = Rng::new(42);
    let mut runs = 0;
    for it in 0..4000 {
        let (flags, overlap) = random_flags(&mut rng);
        let Some(h) = random_header(&mut rng, flags) else {
            continue;
        };
        let f = &h.freq_band_data;
        let time_slots = h.number_time_slots as i32;
        let n_cols = time_slots * h.time_step as i32;
        let mut sc = TransposerSettings::ZERO;
        let mut hc = SbrLppTrans::ZERO;
        let mut vk = f.v_k_master;
        let mut nt = f.freq_band_table_noise;
        let rc = unsafe {
            cref_sbrdec_lpp_tran_create(
                ptr(&mut hc),
                ptr(&mut sc),
                f.low_subband as i32,
                vk.as_mut_ptr(),
                f.num_master as i32,
                f.high_subband as i32,
                time_slots,
                n_cols,
                nt.as_mut_ptr(),
                f.n_nfb as i32,
                h.sbr_proc_smpl_rate,
                0,
                overlap,
            )
        };
        let mut sp = TransposerSettings::ZERO;
        let mut hp = SbrLppTrans::ZERO;
        let rp = create_lpp_transposer(
            &mut hp,
            &mut sp,
            f.low_subband as i32,
            &f.v_k_master,
            f.num_master as i32,
            f.high_subband as i32,
            time_slots,
            n_cols,
            &f.freq_band_table_noise,
            f.n_nfb as i32,
            h.sbr_proc_smpl_rate,
            0,
            overlap,
        );
        assert_eq!(rp, rc, "createLppTransposer {it}");
        assert_eq!(pod_bytes(&sp), pod_bytes(&sc), "createLppTransposer settings {it}");
        if rc != 0 {
            continue;
        }
        // random filter states
        for v in hc.lpc.iter_mut() {
            *v = (rng.next_u32() as i32) >> rng.range(0, 31);
        }
        for v in hc.bw_vector_old.iter_mut() {
            *v = (rng.next_u32() >> 1) as i32;
        }
        let mut hp = hc;
        let use_lp = flags & SBRDEC_SYNTAX_USAC_FLAG == 0 && rng.next_u32() % 2 == 0;
        let n_slots = 128usize;
        let mut invf_prev = [0i32; MAX_INVF_BANDS];
        for fr in 0..3 {
            let class = rng.next_u32() % 4;
            let mut re: Vec<i32> = (0..n_slots * 64)
                .map(|_| match class {
                    0 => 0,
                    1 => rng.i32(),
                    _ => (rng.next_u32() as i32) >> rng.range(2, 30),
                })
                .collect();
            let mut im: Vec<i32> = re.iter().map(|&x| x.rotate_left(11)).collect();
            let mut alias: Vec<i32> = (0..64).map(|_| rng.i32()).collect();
            let (mut re2, mut im2, mut alias2) = (re.clone(), im.clone(), alias.clone());
            let mut scale = [rng.range(-10, 20), rng.range(-10, 20), rng.range(-10, 20), rng.range(-10, 20)];
            let mut invf = [0i32; MAX_INVF_BANDS];
            for v in invf.iter_mut() {
                *v = rng.range(0, 3);
            }
            let pre = rng.range(0, 1);
            // slot offsets of the frame grids: firstSlotOffs * timeStep and lastSlotOffs *
            // timeStep stay within the overlap (offsets with a defined reference result)
            let max_off = sp.overlap as i32 / h.time_step as i32;
            let first = rng.range(0, max_off);
            let last = rng.range(0, max_off);
            let ninvf = f.n_invf_bands as i32;
            let mut ip = invf_prev;
            let mut i2 = invf;
            let mut sf = QmfScaleFactor { lb_scale: scale[0], ov_lb_scale: scale[1], hb_scale: scale[2], ov_hb_scale: scale[3] };
            unsafe {
                cref_sbrdec_lpp_tran_apply(
                    ptr(&mut hc),
                    ptr(&mut sc),
                    scale.as_mut_ptr(),
                    re.as_mut_ptr(),
                    alias.as_mut_ptr(),
                    im.as_mut_ptr(),
                    n_slots as i32,
                    use_lp as i32,
                    pre,
                    f.v_k_master[0] as i32,
                    h.time_step as i32,
                    first,
                    last,
                    ninvf,
                    i2.as_mut_ptr(),
                    ip.as_mut_ptr(),
                )
            };
            {
                let mut r: Vec<&mut [i32]> = re2.chunks_mut(64).collect();
                let mut i: Vec<&mut [i32]> = im2.chunks_mut(64).collect();
                lpp_transposer(
                    &mut hp,
                    &sp,
                    &mut sf,
                    &mut r,
                    &mut alias2,
                    &mut i,
                    use_lp,
                    pre,
                    f.v_k_master[0] as i32,
                    h.time_step as i32,
                    first,
                    last,
                    ninvf,
                    &invf,
                    &invf_prev,
                );
            }
            let what = format!("lppTransposer {it}/{fr} lp {use_lp} flags {flags:#x}");
            assert_eq!([sf.lb_scale, sf.ov_lb_scale, sf.hb_scale, sf.ov_hb_scale], scale, "{what}: scale");
            assert_eq!(pod_bytes(&hp), pod_bytes(&hc), "{what}: state");
            if let Some(k) = (0..re.len()).find(|&k| re[k] != re2[k]) {
                panic!(
                    "{what}: real slot {} band {}: port {} reference {} (lsb {} usb {} first {first} last {last} ts {} ov {})",
                    k / 64,
                    k % 64,
                    re2[k],
                    re[k],
                    f.low_subband,
                    f.high_subband,
                    h.time_step,
                    sp.overlap
                );
            }
            if !use_lp {
                assert!(im2 == im, "{what}: imag");
            } else {
                let lo = f.low_subband as usize;
                let hi = (f.high_subband as usize).min(64);
                assert_eq!(alias2[lo..hi.max(lo)], alias[lo..hi.max(lo)], "{what}: degreeAlias");
            }
            invf_prev = invf;
            runs += 1;
        }
        let _ = pod_bytes_mut(&mut hp);
    }
    assert!(runs > 1000, "runs {runs}");
}

const SBRDEC_SYNTAX_USAC_FLAG: u32 = fdk_aac_rs_sbrdec::env_extr::SBRDEC_SYNTAX_USAC;

/// The band-parallel loop of lppTransposer (low power) against the straight loop
/// (`lpp_transposer_straight`), on the transposers of random headers with random states, QMF
/// data of all magnitudes (full scale gives dynamicScale 0), scales, slot offsets and inverse
/// filtering modes; every eighth case with patch parameters changed so that targets overlap
/// sources or each other (the band loop must leave these to the straight loop).
#[test]
fn transposer_bands_match_straight() {
    let mut rng = Rng::new(43);
    let mut runs = 0;
    for it in 0..12000 {
        let (flags, overlap) = random_flags(&mut rng);
        let Some(h) = random_header(&mut rng, flags) else {
            continue;
        };
        let f = &h.freq_band_data;
        let time_slots = h.number_time_slots as i32;
        let n_cols = time_slots * h.time_step as i32;
        let mut sp = TransposerSettings::ZERO;
        let mut hp = SbrLppTrans::ZERO;
        let rp = create_lpp_transposer(
            &mut hp,
            &mut sp,
            f.low_subband as i32,
            &f.v_k_master,
            f.num_master as i32,
            f.high_subband as i32,
            time_slots,
            n_cols,
            &f.freq_band_table_noise,
            f.n_nfb as i32,
            h.sbr_proc_smpl_rate,
            0,
            overlap,
        );
        if rp != 0 {
            continue;
        }
        if it % 8 == 7 && sp.no_of_patches > 1 {
            let p = rng.range(0, sp.no_of_patches as i32 - 1) as usize;
            let q = rng.range(0, sp.no_of_patches as i32 - 1) as usize;
            sp.patch_param[p].target_band_offs = sp.patch_param[q].target_band_offs
                .wrapping_add(rng.range(-3, 3) as u8)
                .min(40);
        }
        for v in hp.lpc.iter_mut() {
            *v = (rng.next_u32() as i32) >> rng.range(0, 31);
        }
        for v in hp.bw_vector_old.iter_mut() {
            *v = (rng.next_u32() >> 1) as i32;
        }
        let mut hs = hp;
        let n_slots = 128usize;
        let mut invf_prev = [0i32; MAX_INVF_BANDS];
        for fr in 0..3 {
            let class = rng.next_u32() % 5;
            let re: Vec<i32> = (0..n_slots * 64)
                .map(|_| match class {
                    0 => 0,
                    1 => rng.i32(),
                    2 => [i32::MIN, i32::MAX, -1, 1][rng.range(0, 3) as usize],
                    _ => (rng.next_u32() as i32) >> rng.range(2, 30),
                })
                .collect();
            let alias: Vec<i32> = (0..64).map(|_| rng.i32()).collect();
            let (mut re_b, mut alias_b) = (re.clone(), alias.clone());
            let (mut re_s, mut alias_s) = (re, alias);
            let scale = [rng.range(-10, 20), rng.range(-10, 20), rng.range(-10, 20), rng.range(-10, 20)];
            let mut invf = [0i32; MAX_INVF_BANDS];
            for v in invf.iter_mut() {
                *v = rng.range(0, 3);
            }
            let pre = rng.range(0, 1);
            let max_off = sp.overlap as i32 / h.time_step as i32;
            let first = rng.range(0, max_off);
            let last = rng.range(0, max_off);
            let ninvf = f.n_invf_bands as i32;
            let mk = || QmfScaleFactor {
                lb_scale: scale[0],
                ov_lb_scale: scale[1],
                hb_scale: scale[2],
                ov_hb_scale: scale[3],
            };
            let (mut sf_b, mut sf_s) = (mk(), mk());
            {
                let mut r: Vec<&mut [i32]> = re_b.chunks_mut(64).collect();
                lpp_transposer(
                    &mut hp, &sp, &mut sf_b, &mut r, &mut alias_b, &mut [], true, pre,
                    f.v_k_master[0] as i32, h.time_step as i32, first, last, ninvf, &invf,
                    &invf_prev,
                );
                let mut r: Vec<&mut [i32]> = re_s.chunks_mut(64).collect();
                lpp_transposer_straight(
                    &mut hs, &sp, &mut sf_s, &mut r, &mut alias_s, &mut [], true, pre,
                    f.v_k_master[0] as i32, h.time_step as i32, first, last, ninvf, &invf,
                    &invf_prev,
                );
            }
            let what = format!("lppTransposer bands {it}/{fr} flags {flags:#x}");
            assert!(sf_b == sf_s, "{what}: scale");
            assert_eq!(pod_bytes(&hp), pod_bytes(&hs), "{what}: state");
            if let Some(k) = (0..re_b.len()).find(|&k| re_b[k] != re_s[k]) {
                panic!("{what}: slot {} band {}: {} vs {}", k / 64, k % 64, re_b[k], re_s[k]);
            }
            assert_eq!(alias_b, alias_s, "{what}: degreeAlias");
            invf_prev = invf;
            runs += 1;
        }
    }
    assert!(runs > 3000, "runs {runs}");
}

/// lppTransposerHBE with the transposer of random USAC headers (2:1, 8:3 and 4:1) and random
/// filter states, QMF data, scales, borders and inverse filtering modes.
#[test]
fn transposer_hbe_random() {
    use fdk_aac_rs_cref::sbrdec_hbe::*;
    use fdk_aac_rs_sbrdec::env_extr::{SBRDEC_QUAD_RATE, SBRDEC_SYNTAX_USAC, SBRDEC_USAC_HARMONICSBR};
    use fdk_aac_rs_sbrdec::hbe::*;
    let _lock = super::e2e::harness_lock(); // (the transposer of shim/sbrdec_hbe.cpp)
    let mut rng = Rng::new(43);
    let mut runs = 0;
    for it in 0..2000 {
        let quad = rng.next_u32() % 3 == 0;
        let flags = SBRDEC_SYNTAX_USAC | SBRDEC_USAC_HARMONICSBR | if quad { SBRDEC_QUAD_RATE } else { 0 };
        let overlap = if quad { 12 } else { 6 };
        let Some(h) = random_header(&mut rng, flags) else {
            continue;
        };
        let f = &h.freq_band_data;
        let time_slots = h.number_time_slots as i32;
        let n_cols = time_slots * h.time_step as i32;
        let mut sc = TransposerSettings::ZERO;
        let mut hc = SbrLppTrans::ZERO;
        let mut vk = f.v_k_master;
        let mut nt = f.freq_band_table_noise;
        let rc = unsafe {
            cref_sbrdec_lpp_tran_create(
                ptr(&mut hc),
                ptr(&mut sc),
                f.low_subband as i32,
                vk.as_mut_ptr(),
                f.num_master as i32,
                f.high_subband as i32,
                time_slots,
                n_cols,
                nt.as_mut_ptr(),
                f.n_nfb as i32,
                h.sbr_proc_smpl_rate,
                0,
                overlap,
            )
        };
        let mut sp = TransposerSettings::ZERO;
        let mut hp = SbrLppTrans::ZERO;
        let rp = create_lpp_transposer(
            &mut hp,
            &mut sp,
            f.low_subband as i32,
            &f.v_k_master,
            f.num_master as i32,
            f.high_subband as i32,
            time_slots,
            n_cols,
            &f.freq_band_table_noise,
            f.n_nfb as i32,
            h.sbr_proc_smpl_rate,
            0,
            overlap,
        );
        assert_eq!((rp, pod_bytes(&sp)), (rc, pod_bytes(&sc)), "createLppTransposer {it}");
        if rc != 0 {
            continue;
        }
        // the transposer (as createSbrDec()/resetSbrDec() set it up)
        let frame = if n_cols == 24 || h.number_time_slots == 12 { 768 } else { 1024 };
        let mut hbe: Option<Box<HbeTransposer>> = None;
        let b41 = quad as i32;
        assert_eq!(qmf_transposer_create(&mut hbe, frame, 0, b41), unsafe { cref_sbrdec_hbe_create(frame, 0, b41) });
        let (mut lo, mut hi) = (f.freq_band_table_lo, f.freq_band_table_hi);
        let rc = unsafe { cref_sbrdec_hbe_reinit(lo.as_mut_ptr(), hi.as_mut_ptr(), f.n_sfb[0], f.n_sfb[1]) };
        let hb = hbe.as_deref_mut().unwrap();
        let rp = qmf_transposer_re_init(Some(&mut *hb), [&lo[..], &hi[..]], f.n_sfb);
        assert_eq!(rp, rc, "QmfTransposerReInit {it}");
        if rc != 0 {
            unsafe { cref_sbrdec_hbe_close() };
            continue;
        }
        for v in hc.lpc.iter_mut() {
            *v = (rng.next_u32() as i32) >> rng.range(0, 31);
        }
        for v in hc.bw_vector_old.iter_mut() {
            *v = (rng.next_u32() >> 1) as i32;
        }
        let mut hp = hc;
        let n_slots = 128usize;
        let mut invf_prev = [0i32; MAX_INVF_BANDS];
        for fr in 0..3 {
            let class = rng.next_u32() % 4;
            let mut re: Vec<i32> = (0..n_slots * 64)
                .map(|_| match class {
                    0 => 0,
                    1 => rng.i32(),
                    _ => (rng.next_u32() as i32) >> rng.range(2, 30),
                })
                .collect();
            let mut im: Vec<i32> = re.iter().map(|&x| x.rotate_left(11)).collect();
            let (mut re2, mut im2) = (re.clone(), im.clone());
            let mut scale = [rng.range(-10, 20), rng.range(-10, 20), rng.range(-10, 20), rng.range(-10, 20)];
            let mut invf = [0i32; MAX_INVF_BANDS];
            for v in invf.iter_mut() {
                *v = rng.range(0, 3);
            }
            let max_off = sp.overlap as i32 / h.time_step as i32;
            let first = rng.range(0, max_off);
            let last = rng.range(0, max_off);
            let ninvf = f.n_invf_bands as i32;
            let mut ip = invf_prev;
            let mut i2 = invf;
            let mut sf = QmfScaleFactor { lb_scale: scale[0], ov_lb_scale: scale[1], hb_scale: scale[2], ov_hb_scale: scale[3] };
            unsafe {
                cref_sbrdec_lpp_tran_apply_hbe(
                    ptr(&mut hc),
                    ptr(&mut sc),
                    cref_sbrdec_hbe_handle(),
                    scale.as_mut_ptr(),
                    re.as_mut_ptr(),
                    im.as_mut_ptr(),
                    n_slots as i32,
                    h.time_step as i32,
                    first,
                    last,
                    ninvf,
                    i2.as_mut_ptr(),
                    ip.as_mut_ptr(),
                )
            };
            {
                let mut r: Vec<&mut [i32]> = re2.chunks_mut(64).collect();
                let mut i: Vec<&mut [i32]> = im2.chunks_mut(64).collect();
                lpp_transposer_hbe(
                    &mut hp,
                    &sp,
                    hb,
                    &mut sf,
                    &mut r,
                    &mut i,
                    h.time_step as i32,
                    first,
                    last,
                    ninvf,
                    &invf,
                    &invf_prev,
                );
            }
            let what = format!("lppTransposerHBE {it}/{fr} flags {flags:#x} first {first} last {last}");
            assert_eq!([sf.lb_scale, sf.ov_lb_scale, sf.hb_scale, sf.ov_hb_scale], scale, "{what}: scale");
            assert_eq!(pod_bytes(&hp), pod_bytes(&hc), "{what}: state");
            if let Some(k) = (0..re.len()).find(|&k| re[k] != re2[k] || im[k] != im2[k]) {
                panic!("{what}: slot {} band {}: port {} {}, reference {} {}", k / 64, k % 64, re2[k], im2[k], re[k], im[k]);
            }
            invf_prev = invf;
            runs += 1;
        }
        unsafe { cref_sbrdec_hbe_close() };
    }
    assert!(runs > 1000, "runs {runs}");
}

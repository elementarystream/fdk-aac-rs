// Differential tests of fdk_aac_rs_sbrenc::ton_corr (with invf_est, nf_est and mh_det)
// against the C++ reference.
//
// The tonality correction module of the library and of the port is initialized with the
// frequency tables of random SBR configurations (sampling rates, start and stop frequencies,
// frequency scales, 32 and 64 QMF bands, HE-AAC and low delay, speech and music tuning, noise
// band counts and maximum levels), then run on sequences of frames: the tonality quotas of
// random QMF data (tonal, noisy, silent and full-scale), and the parameter extraction with the
// grids of the frame generator and random transient information (which runs the inverse
// filtering detector, the missing harmonics detector and the noise floor estimation). The
// outputs and the complete SBR_TON_CORR_EST are compared after every call; resets with new
// tables are mixed in.

use fdk_aac_rs_cref::sbrenc_ton_corr as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrenc::env_est::{V_TUNING_ELD, V_TUNING_HEAAC};
use fdk_aac_rs_sbrenc::fram_gen::*;
use fdk_aac_rs_sbrenc::mh_det::ENV_DATA_TAIL_BYTES;
use fdk_aac_rs_sbrenc::sbr_def::*;
use fdk_aac_rs_sbrenc::sbr_encoder::*;
use fdk_aac_rs_sbrenc::sbrenc_freq_sca::*;
use fdk_aac_rs_sbrenc::ton_corr::*;

use super::sbr_encoder_dump::{assert_dump_eq, frame_info, ton_corr, Dump};

type H = *mut core::ffi::c_void;

/// Frequency tables of a random valid configuration: (config data, start band).
pub fn random_tables(rng: &mut Rng, fs_sbr: i32, no_qmf: i32, dsf: i32) -> Option<SbrConfigData> {
    let mut cfg = SbrConfigData::default();
    let fs_core = fs_sbr >> (dsf - 1);
    let start = rng.range(0, 15);
    let stop = rng.range(0, 13);
    let (mut k0, mut k2) = (0, 0);
    if fdk_sbr_enc_find_start_and_stop_band(fs_sbr, fs_core, no_qmf, start, stop, &mut k0, &mut k2) != 0 {
        return None;
    }
    let mut num_master = 0;
    if fdk_sbr_enc_update_freq_scale(&mut cfg.v_k_master, &mut num_master, k0, k2, rng.range(1, 3), rng.range(0, 1)) != 0
        || num_master > 48
    {
        return None;
    }
    let mut xover = 0;
    let mut n_hi = 0;
    fdk_sbr_enc_update_hi_res(&mut cfg.freq_band_table_hi, &mut n_hi, &cfg.v_k_master, num_master, &mut xover);
    if n_hi > 48 || n_hi < 1 {
        return None;
    }
    let mut n_lo = 0;
    fdk_sbr_enc_update_lo_res(&mut cfg.freq_band_table_lo, &mut n_lo, &cfg.freq_band_table_hi, n_hi);
    cfg.num_master = num_master;
    cfg.n_sfb = [n_lo, n_hi];
    cfg.sample_freq = fs_sbr;
    cfg.no_qmf_bands = no_qmf;
    Some(cfg)
}

/// QMF data of one frame (32 rows of 64), `style` selects the signal.
fn qmf(rng: &mut Rng, style: i32) -> (Vec<i32>, Vec<i32>) {
    let mut re = vec![0i32; 32 * 64];
    let mut im = vec![0i32; 32 * 64];
    let amp = rng.range(0, 30);
    let f0 = rng.range(1, 60) as f64 + 0.3;
    for t in 0..32 {
        for b in 0..64 {
            let (x, y) = match style {
                0 => (rng.i32() >> amp.max(1), rng.i32() >> amp.max(1)),
                1 => {
                    // a tone in some bands, noise elsewhere
                    let ph = (t as f64) * (f0 + b as f64 * 0.1);
                    let a = if (b as f64 - f0 / 2.0).abs() < 3.0 { 1.0e9 } else { 1.0e6 };
                    ((a * ph.cos()) as i32 >> amp.min(20), (a * ph.sin()) as i32 >> amp.min(20))
                }
                2 => (0, 0),
                3 => (if (t + b) % 2 == 0 { i32::MAX } else { i32::MIN }, rng.i32()),
                _ => (rng.range(-3, 3), rng.range(-3, 3)),
            };
            re[t * 64 + b] = x;
            im[t * 64 + b] = y;
        }
    }
    (re, im)
}

fn rows(v: &[i32]) -> Vec<&[i32]> {
    v.chunks(64).collect()
}

fn check(hc: H, r: &SbrTonCorrEst, what: &str) {
    let mut buf = vec![0i32; 8192];
    let n = unsafe { c::cref_ton_corr_dump(hc, buf.as_mut_ptr(), buf.len() as i32) };
    let mut d = Dump::default();
    ton_corr(&mut d, r);
    assert_dump_eq(&buf[..n as usize], &d.0, what);
}

fn cfg_args(cfg: &SbrConfigData) -> [i32; 7] {
    [
        cfg.sbr_syntax_flags as i32,
        cfg.no_qmf_slots,
        cfg.sample_freq,
        cfg.no_qmf_bands,
        cfg.num_master,
        cfg.n_sfb[0],
        cfg.n_sfb[1],
    ]
}

#[test]
fn env_data_layout() {
    // The port's image of the SBR_ENV_DATA members after addHarmonic assumes this layout of
    // both reference builds (LP64): addHarmonic at 312, 192 bytes up to currentAmpResFF.
    let mut v = [0i32; 2];
    unsafe { c::cref_ton_corr_env_data_layout(v.as_mut_ptr()) };
    assert!(v[0] as usize + 4 * MAX_FREQ_COEFFS <= v[1] as usize, "{v:?}");
}

#[test]
fn sequences() {
    let mut rng = Rng::new(0x5eed_0401);
    let mut runs = 0;
    let (mut harm, mut comp, mut invf) = (0, 0, 0);
    while runs < 160 {
        let ld = rng.range(0, 2) == 0;
        let (fs_sbr, no_qmf, dsf) = match rng.range(0, 5) {
            0 => (44100, 64, 2),
            1 => (48000, 64, 2),
            2 => (32000, 64, 2),
            3 => (24000, 32, 1),
            4 => (22050, 64, 2),
            _ => (48000, 32, 1),
        };
        let Some(mut cfg) = random_tables(&mut rng, fs_sbr, no_qmf, dsf) else {
            continue;
        };
        runs += 1;
        let (time_slots, frame_size) = if ld {
            if rng.range(0, 1) == 0 { (16, 1024) } else { (15, 960) }
        } else if rng.range(0, 3) == 0 {
            (15, 1920)
        } else {
            (16, 2048)
        };
        cfg.sbr_syntax_flags = if ld { SBR_SYNTAX_LOW_DELAY } else { 0 };
        cfg.no_qmf_slots = if ld { time_slots } else { 2 * time_slots };
        let xpos_ctrl = if rng.range(0, 4) == 0 { 1 } else { 2 };
        let ana_max_level = [6, 3, -3, 0][rng.range(0, 3) as usize];
        let noise_bands = rng.range(0, 3);
        let nfo = rng.range(0, 3);
        let speech = rng.range(0, 1) as u32;

        let hc = unsafe { c::cref_ton_corr_new(0) };
        let mut r = SbrTonCorrEst::default();
        assert_eq!(fdk_sbr_enc_create_ton_corr_param_extr(&mut r, 0), 0);
        let mut a = cfg_args(&cfg);
        let mut vkm = cfg.v_k_master;
        let mut lo = cfg.freq_band_table_lo;
        let mut hi = cfg.freq_band_table_hi;
        let rc = unsafe {
            c::cref_ton_corr_init(
                hc, frame_size, a.as_mut_ptr(), vkm.as_mut_ptr(), lo.as_mut_ptr(), hi.as_mut_ptr(), time_slots,
                xpos_ctrl, ana_max_level, noise_bands, nfo, speech,
            )
        };
        let rr = fdk_sbr_enc_init_ton_corr_param_extr(
            frame_size, &mut r, &cfg, time_slots, xpos_ctrl, ana_max_level, noise_bands, nfo, speech,
        );
        assert_eq!(rc, rr, "init");
        check(hc, &r, "init");
        if rc != 0 {
            unsafe { c::cref_ton_corr_free(hc) };
            continue;
        }
        // switchInverseFilt is set by the encoder after the init (initEnvChannel()).
        let sw = (rng.range(0, 4) != 0) as i32;
        unsafe { c::cref_ton_corr_set_switch(hc, sw) };
        r.switch_inverse_filt = sw;

        let mut fg = SbrEnvelopeFrame::default();
        let fr = [1, 1];
        fdk_sbr_enc_init_frame_info_generator(&mut fg, 1, 1, 0, time_slots, &fr, 0, ld as i32);
        let tuning: &'static [i32] = if ld { &V_TUNING_ELD } else { &V_TUNING_HEAAC };
        let mut pre = [0u8; 2];
        let mut add_harm = [0u8; 4 * MAX_FREQ_COEFFS];
        for (i, b) in add_harm.iter_mut().enumerate().skip(MAX_FREQ_COEFFS) {
            *b = (i * 7) as u8;
        }
        let style = rng.range(0, 4);
        for f in 0..30 {
            let st = if rng.range(0, 5) == 0 { rng.range(0, 4) } else { style };
            let (mut re, mut im) = qmf(&mut rng, st);
            let usb = cfg.freq_band_table_hi[cfg.n_sfb[1] as usize] as i32;
            let qmf_scale = rng.range(0, 20);
            unsafe { c::cref_ton_corr_quotas(hc, re.as_mut_ptr(), im.as_mut_ptr(), usb, qmf_scale) };
            fdk_sbr_enc_calculate_tonality_quotas(&mut r, &rows(&re), &rows(&im), usb, qmf_scale);
            check(hc, &r, &format!("quotas frame {f}"));

            let tran = [rng.range(0, time_slots - 1) as u8, (rng.range(0, 3) == 0) as u8, rng.range(0, 1) as u8];
            fdk_sbr_enc_frame_info_generator(&mut fg, &tran, 0, &pre, ld as i32, tuning);
            pre = [tran[0], tran[1]];
            let mut fi = Dump::default();
            frame_info(&mut fi, &fg.sbr_frame_info);

            let xpos = if rng.range(0, 5) == 0 { XPOS_MDCT } else { XPOS_LC };
            let mut inf_c = [0i32; MAX_NUM_NOISE_VALUES];
            let mut inf_r = inf_c;
            let mut nl_c = [0i32; MAX_NUM_NOISE_VALUES];
            let mut nl_r = nl_c;
            let (mut fl_c, mut fl_r) = (7, 7);
            let mut ah_c = add_harm;
            let mut ah_r = add_harm;
            let mut ec_c = [0u8; MAX_FREQ_COEFFS];
            let mut ec_r = ec_c;
            let mut tran_c = tran;
            let mut fbt = cfg.freq_band_table_hi;
            let mut tail = [0u8; ENV_DATA_TAIL_BYTES];
            tail.copy_from_slice(&add_harm[MAX_FREQ_COEFFS..]);
            unsafe {
                c::cref_ton_corr_extr(
                    hc, inf_c.as_mut_ptr(), nl_c.as_mut_ptr(), &mut fl_c, ah_c.as_mut_ptr(), ec_c.as_mut_ptr(),
                    fi.0.as_mut_ptr(), tran_c.as_mut_ptr(), fbt.as_mut_ptr(), cfg.n_sfb[1], xpos,
                    cfg.sbr_syntax_flags,
                )
            };
            fdk_sbr_enc_ton_corr_param_extr(
                &mut r,
                &mut inf_r,
                &mut nl_r,
                &mut fl_r,
                &mut ah_r[..MAX_FREQ_COEFFS],
                &mut ec_r,
                &fg.sbr_frame_info,
                &tran,
                &cfg.freq_band_table_hi,
                cfg.n_sfb[1],
                xpos,
                cfg.sbr_syntax_flags,
                &tail,
            );
            assert_eq!((inf_c, nl_c, fl_c), (inf_r, nl_r, fl_r), "extr outputs frame {f}");
            assert_eq!(ah_c, ah_r, "addHarmonic frame {f}");
            assert_eq!(ec_c, ec_r, "envelopeCompensation frame {f}");
            check(hc, &r, &format!("extr frame {f}"));
            add_harm = ah_c;
            harm += (fl_c != 0) as i32;
            comp += ec_c.iter().any(|&x| x != 0) as i32;
            invf += inf_c.iter().any(|&x| x != 0) as i32;

            if rng.range(0, 15) == 0 {
                // reset with new tables (dynamic bandwidth)
                // Only resets that do not increase the number of high resolution bands: with
                // more bands, the missing harmonics detector reads tonality differences beyond
                // the old count, which have no defined value in C (see the port, mh_det.rs).
                if let Some(mut c2) = random_tables(&mut rng, fs_sbr, no_qmf, dsf)
                    .filter(|c2| c2.n_sfb[1] <= cfg.n_sfb[1])
                {
                    c2.sbr_syntax_flags = cfg.sbr_syntax_flags;
                    c2.no_qmf_slots = cfg.no_qmf_slots;
                    let mut vkm = c2.v_k_master;
                    let mut lo = c2.freq_band_table_lo;
                    let mut hi = c2.freq_band_table_hi;
                    let mut nsfb = c2.n_sfb;
                    let hb = c2.freq_band_table_hi[0] as i32;
                    let rc = unsafe {
                        c::cref_ton_corr_reset(
                            hc, xpos_ctrl, hb, vkm.as_mut_ptr(), c2.num_master, fs_sbr, lo.as_mut_ptr(),
                            hi.as_mut_ptr(), nsfb.as_mut_ptr(), no_qmf,
                        )
                    };
                    let rr = fdk_sbr_enc_reset_ton_corr_param_extr(
                        &mut r,
                        xpos_ctrl,
                        hb,
                        &c2.v_k_master,
                        c2.num_master,
                        fs_sbr,
                        &c2.freq_band_table_lo,
                        &c2.n_sfb,
                        no_qmf,
                    );
                    assert_eq!(rc, rr, "reset");
                    check(hc, &r, "reset");
                    if rc != 0 {
                        break;
                    }
                    cfg = c2;
                }
            }
        }
        unsafe { c::cref_ton_corr_free(hc) };
    }
    // coverage of the detectors
    assert!(harm > 20 && comp > 0 && invf > 100, "harm {harm} comp {comp} invf {invf}");
}

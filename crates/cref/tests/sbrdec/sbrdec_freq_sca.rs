// Differential tests of fdk_aac_rs_sbrdec::sbrdec_freq_sca against the C++ reference.
//
// sbrdecUpdateFreqScale for all start/stop frequencies, frequency scales and alter scales at
// standard and other sampling rates, for the AAC, USAC and quad rate syntaxes; the file-static
// helpers (start and stop band, band factor, number of bands, band widths, cumulated sum, band
// modification, high and low resolution tables) and sbrdecDownSampleLoRes and shellsort with
// random inputs in the ranges their callers use. resetFreqBandTables is also covered by
// tests/sbrdec/env_extr.rs and env_calc.rs; real data by tests/sbrdec/e2e.rs.

use super::env_extr::ptr;
use fdk_aac_rs_cref::sbrdec_env_extr::cref_sbrdec_env_extr_update_freq_scale;
use fdk_aac_rs_cref::sbrdec_sbrdec_freq_sca::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::env_extr::*;
use fdk_aac_rs_sbrdec::lpp_tran::MAX_FREQ_COEFFS;
use fdk_aac_rs_sbrdec::sbrdec_freq_sca::*;

// (no rate 0: the division by the rate in getStopBand() has no defined result on x86)
const RATES: [u32; 19] = [
    4000, 8000, 11025, 12000, 16000, 22050, 24000, 32000, 40000, 44100, 46009, 48000, 64000,
    88200, 96000, 128000, 176400, 192000, 42000,
];

fn random_flags(rng: &mut Rng) -> u32 {
    let mut f = 0;
    if rng.next_u32() % 2 == 0 {
        f |= SBRDEC_SYNTAX_USAC;
        if rng.next_u32() % 2 == 0 {
            f |= SBRDEC_QUAD_RATE;
        }
    }
    if rng.next_u32() % 8 == 0 {
        f |= SBRDEC_SYNTAX_RSVD50;
    }
    if rng.next_u32() % 8 == 0 {
        f |= SBRDEC_QUAD_RATE;
    }
    f
}

fn random_rate(rng: &mut Rng) -> u32 {
    if rng.next_u32() % 4 == 0 {
        rng.range(2, 200_000) as u32 // (at least 1 after the halving of quad rate)
    } else {
        RATES[rng.range(0, RATES.len() as i32 - 1) as usize]
    }
}

#[test]
fn update_freq_scale() {
    let mut rng = Rng::new(0xf5ca);
    let mut ok = 0;
    for it in 0..100_000 {
        let flags = random_flags(&mut rng);
        let fs = random_rate(&mut rng);
        let mut h = SbrHeaderData::ZERO;
        h.bs_data.start_freq = rng.range(0, 15) as u8;
        h.bs_data.stop_freq = rng.range(0, 15) as u8;
        h.bs_data.freq_scale = rng.range(0, 3) as u8;
        h.bs_data.alter_scale = rng.range(0, 1) as u8;
        let mut vc = [0u8; MAX_FREQ_COEFFS + 1];
        let mut nc = rng.next_u32() as u8;
        let mut vp = vc;
        let mut np = nc;
        let mut hc = h;
        let rc = unsafe { cref_sbrdec_env_extr_update_freq_scale(vc.as_mut_ptr(), &mut nc, fs, ptr(&mut hc), flags) };
        let rp = sbrdec_update_freq_scale(&mut vp, &mut np, fs, &h, flags);
        assert_eq!((rp, np, vp), (rc, nc, vc), "{it}: fs {fs} flags {flags:#x} {:?}", h.bs_data);
        ok += (rc == 0) as usize;
    }
    assert!(ok > 10_000, "{ok} valid configurations");
}

#[test]
fn start_and_stop_band() {
    let mut rng = Rng::new(0x57a);
    for it in 0..100_000 {
        let flags = random_flags(&mut rng);
        let fs = random_rate(&mut rng);
        let f = rng.range(0, 15) as u8;
        let k0 = rng.range(0, 64) as u8;
        assert_eq!(get_start_band(fs, f, flags), unsafe { cref_sbrdec_freq_sca_start_band(fs, f, flags) }, "{it}");
        assert_eq!(get_stop_band(fs, f, flags, k0), unsafe { cref_sbrdec_freq_sca_stop_band(fs, f, flags, k0) }, "{it}");
    }
}

#[test]
fn band_helpers() {
    let mut rng = Rng::new(0xba7d);
    for it in 0..50_000 {
        let start = rng.range(1, 63);
        let stop = rng.range(start + 1, 64);
        let n = rng.range(1, 48);
        assert_eq!(calc_factor_per_band(start, stop, n), unsafe { cref_sbrdec_freq_sca_factor_per_band(start, stop, n) }, "{it}: factor {start} {stop} {n}");

        let bpo = match rng.next_u32() % 4 {
            0 => 0x6000,
            1 => 0x5000,
            2 => 0x4000,
            _ => ((rng.range(0, 63) & 0xfe) << 11) as i16 as i32,
        } as i16;
        let warp = rng.range(0, 1);
        assert_eq!(number_of_bands(bpo, start, stop, warp), unsafe { cref_sbrdec_freq_sca_number_of_bands(bpo, start, stop, warp) }, "{it}: bands {bpo} {start} {stop} {warp}");

        let nb = rng.range(1, 49) as u8;
        let mut dc = [0u8; 64];
        let mut dp = dc;
        unsafe { cref_sbrdec_freq_sca_calc_bands(dc.as_mut_ptr(), start as u8, stop as u8, nb) };
        calc_bands(&mut dp, start as u8, stop as u8, nb);
        assert_eq!(dp, dc, "{it}: calc_bands {start} {stop} {nb}");

        let len = rng.range(0, 48) as u8;
        let diff: Vec<u8> = (0..64).map(|_| rng.range(0, 12) as u8).collect();
        let sv = rng.range(0, 64) as u8;
        let (mut oc, mut op) = ([0u8; 64], [0u8; 64]);
        let mut dcc = diff.clone();
        unsafe { cref_sbrdec_freq_sca_cum_sum(sv, dcc.as_mut_ptr(), len, oc.as_mut_ptr()) };
        cum_sum(sv, &diff, len, &mut op);
        assert_eq!(op, oc, "{it}: cum_sum");

        let len = rng.range(1, 48) as u8;
        let mut dc: Vec<u8> = (0..64).map(|_| rng.range(1, 12) as u8).collect();
        dc[..len as usize].sort();
        let mut dp = dc.clone();
        let mb = rng.range(0, 16) as u8;
        let rc = unsafe { cref_sbrdec_freq_sca_modify_bands(mb, dc.as_mut_ptr(), len) };
        let rp = modify_bands(mb, &mut dp, len);
        assert_eq!((rp, &dp), (rc, &dc), "{it}: modify_bands {mb} {len}");
    }
}

#[test]
fn hi_and_lo_res() {
    let mut rng = Rng::new(0x41);
    for it in 0..50_000 {
        let nb = rng.range(1, MAX_FREQ_COEFFS as i32) as u8;
        let xover = rng.range(0, nb as i32 - 1) as u8;
        let mut vk: Vec<u8> = (0..=MAX_FREQ_COEFFS).map(|_| rng.range(0, 64) as u8).collect();
        vk.sort();
        let (mut hc, mut hp) = ([0u8; MAX_FREQ_COEFFS + 1], [0u8; MAX_FREQ_COEFFS + 1]);
        let (mut nc, mut np) = (0u8, 0u8);
        unsafe { cref_sbrdec_freq_sca_hi_res(hc.as_mut_ptr(), &mut nc, vk.as_mut_ptr(), nb, xover) };
        sbrdec_update_hi_res(&mut hp, &mut np, &vk, nb, xover);
        assert_eq!((np, hp), (nc, hc), "{it}: hi res {nb} {xover}");

        let (mut lc, mut lp) = ([0u8; MAX_FREQ_COEFFS + 1], [0u8; MAX_FREQ_COEFFS + 1]);
        let (mut mc, mut mp) = (0u8, 0u8);
        unsafe { cref_sbrdec_freq_sca_lo_res(lc.as_mut_ptr(), &mut mc, hc.as_mut_ptr(), nc) };
        sbrdec_update_lo_res(&mut lp, &mut mp, &hp, np);
        assert_eq!((mp, lp), (mc, lc), "{it}: lo res {np}");

        // (num_result is the number of noise bands, at most 5; from 28 on the reference has no
        // defined result, the port stops)
        let num_ref = rng.range(1, MAX_FREQ_COEFFS as i32 / 2) as u8;
        let num_result = rng.range(1, num_ref.min(27) as i32) as u8;
        let (mut rc, mut rp) = ([0u8; MAX_FREQ_COEFFS + 1], [0u8; MAX_FREQ_COEFFS + 1]);
        let mut refc = vk.clone();
        unsafe { cref_sbrdec_freq_sca_down_sample_lo_res(rc.as_mut_ptr(), num_result, refc.as_mut_ptr(), num_ref) };
        sbrdec_down_sample_lo_res(&mut rp, num_result, &vk, num_ref);
        assert_eq!(rp, rc, "{it}: down sample {num_result} {num_ref}");

        let n = rng.range(0, 64) as u8;
        let mut sc: Vec<u8> = (0..64).map(|_| rng.next_u32() as u8).collect();
        let mut sp = sc.clone();
        unsafe { cref_sbrdec_freq_sca_shellsort(sc.as_mut_ptr(), n) };
        shellsort(&mut sp, n);
        assert_eq!(sp, sc, "{it}: shellsort {n}");
    }
}

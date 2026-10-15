// Differential tests of fdk_aac_rs_sbrenc::sbrenc_freq_sca against the C++ reference.
//
// The start and stop frequencies for every core sampling rate and index (including invalid
// ones), FDKsbrEnc_FindStartAndStopBand() for every combination of rates, start and stop
// indices and band counts, and the master, high and low resolution tables for every valid
// (k0, k2) pair, frequency scale, alter scale and crossover band.

use fdk_aac_rs_cref::sbrenc_sbrenc_freq_sca as c;
use fdk_aac_rs_sbrenc::sbrenc_freq_sca::*;

/// The core sampling rates with tables, then rates the functions treat as illegal.
const RATES: [i32; 10] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 96000];
const OTHER_RATES: [i32; 5] = [7350, 64000, 88200, 0, 12345];

#[test]
fn start_stop_freq() {
    for &fs in RATES.iter().chain(OTHER_RATES.iter()) {
        for start in -2..=17 {
            let r = unsafe { c::cref_freq_sca_start_freq_raw(start, fs) };
            assert_eq!(r, fdk_sbr_enc_get_sbr_start_freq_raw(start, fs), "start {start} fs {fs}");
        }
    }
    // Only the rates with a defined reference result of getStopFreq() are compared.
    for &fs in RATES.iter() {
        for stop in -2..=15 {
            let r = unsafe { c::cref_freq_sca_stop_freq_raw(stop, fs) };
            assert_eq!(r, fdk_sbr_enc_get_sbr_stop_freq_raw(stop, fs), "stop {stop} fs {fs}");
        }
    }
}

#[test]
fn find_start_and_stop_band() {
    for &sr_core in RATES.iter() {
        for &dsf in &[1, 2] {
            let sr_sbr = sr_core * dsf;
            for &no_channels in &[16, 32, 64] {
                for start in 0..=15 {
                    for stop in 0..=15 {
                        let (mut k0c, mut k2c, mut k0r, mut k2r) = (-7, -7, -7, -7);
                        let rc = unsafe {
                            c::cref_freq_sca_find_start_and_stop_band(
                                sr_sbr, sr_core, no_channels, start, stop, &mut k0c, &mut k2c,
                            )
                        };
                        let rr = fdk_sbr_enc_find_start_and_stop_band(
                            sr_sbr, sr_core, no_channels, start, stop, &mut k0r, &mut k2r,
                        );
                        assert_eq!((rc, k0c, k2c), (rr, k0r, k2r), "{sr_core} {dsf} {no_channels} {start} {stop}");
                    }
                }
            }
        }
    }
}

#[test]
fn freq_scale_and_tables() {
    let mut cases = 0;
    for k0 in 1..=48 {
        for k2 in k0..=64 {
            for freq_scale in 0..=3 {
                for alter_scale in 0..=1 {
                    if k2 - k0 > 48 {
                        continue;
                    }
                    if freq_scale > 0 {
                        // Inputs without a defined reference result are skipped (see the
                        // port), with a margin.
                        if 10 * k2 < 11 * k0 + 10 {
                            continue;
                        }
                    } else if k2 - k0 < 2 * (alter_scale + 1) {
                        // Inputs without a defined reference result are skipped.
                        continue;
                    }
                    let mut mc = [0xaau8; 49];
                    let mut mr = mc;
                    let (mut nc, mut nr) = (-1, -1);
                    let rc = unsafe {
                        c::cref_freq_sca_update_freq_scale(mc.as_mut_ptr(), &mut nc, k0, k2, freq_scale, alter_scale)
                    };
                    let rr = fdk_sbr_enc_update_freq_scale(&mut mr, &mut nr, k0, k2, freq_scale, alter_scale);
                    assert_eq!((rc, nc, mc), (rr, nr, mr), "k0 {k0} k2 {k2} fs {freq_scale} as {alter_scale}");
                    if rc != 0 || nc > 48 {
                        continue;
                    }
                    cases += 1;
                    for xover in 0..=nc {
                        let mut hc = [0x55u8; 49];
                        let mut hr = hc;
                        let (mut nhc, mut nhr) = (-1, -1);
                        let (mut xc, mut xr) = (xover, xover);
                        let rc = unsafe {
                            c::cref_freq_sca_update_hi_res(hc.as_mut_ptr(), &mut nhc, mc.as_mut_ptr(), nc, &mut xc)
                        };
                        let rr = fdk_sbr_enc_update_hi_res(&mut hr, &mut nhr, &mr, nr, &mut xr);
                        assert_eq!((rc, nhc, xc, hc), (rr, nhr, xr, hr), "hires {k0} {k2} {xover}");
                        if nhc < 0 || nhc > 48 {
                            continue;
                        }
                        let mut lc = [0x33u8; 49];
                        let mut lr = lc;
                        let (mut nlc, mut nlr) = (-1, -1);
                        unsafe { c::cref_freq_sca_update_lo_res(lc.as_mut_ptr(), &mut nlc, hc.as_mut_ptr(), nhc) };
                        fdk_sbr_enc_update_lo_res(&mut lr, &mut nlr, &hr, nhr);
                        assert_eq!((nlc, lc), (nlr, lr), "lores {k0} {k2} {xover}");
                    }
                }
            }
        }
    }
    assert!(cases > 1000, "{cases}");
}

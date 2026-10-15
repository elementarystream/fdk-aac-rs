// Differential tests of fdk_aac_rs_aacenc::spreading against the C++ reference.
//
// FDKaacEnc_SpreadingMax is compared on random energies and slopes (including extreme and
// negative values) and on realistic data: the spreading slopes of real configurations
// (FDKaacEnc_InitPsyConfiguration() of the reference) applied to thresholds and energies of MDCT
// spectra of real and synthetic PCM, as in FDKaacEnc_psyMain().

use crate::band_nrg::{configs, frames, normalize};
use fdk_aac_rs_aacenc::band_nrg::fdk_aac_enc_calc_band_energy_optim_long;
use fdk_aac_rs_aacenc::band_nrg::fdk_aac_enc_calc_band_energy_optim_short;
use fdk_aac_rs_aacenc::psy_configuration::C_RATIO;
use fdk_aac_rs_aacenc::spreading::*;
use fdk_aac_rs_cref::aacenc_spreading as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;

fn check(pb_cnt: i32, low: &[i32], high: &[i32], energy: &[i32], what: &str) {
    let mut a = energy.to_vec();
    let mut b = energy.to_vec();
    fdk_aac_enc_spreading_max(pb_cnt, low, high, &mut a);
    unsafe { c::cref_spreading_SpreadingMax(pb_cnt, low.as_ptr(), high.as_ptr(), b.as_mut_ptr()) };
    assert_eq!(a, b, "{what}");
}

#[test]
fn spreading_max_random() {
    let mut r = Rng::new(41);
    for it in 0..20000 {
        let n = r.range(1, 60);
        let mask = |r: &mut Rng| -> Vec<i32> {
            (0..64)
                .map(|_| match r.next_u32() % 4 {
                    0 => r.i32(),
                    _ => (r.next_u32() >> 1) as i32,
                })
                .collect()
        };
        let low = mask(&mut r);
        let high = mask(&mut r);
        let energy: Vec<i32> = (0..64)
            .map(|_| match r.next_u32() % 4 {
                0 => r.i32(),
                1 => 0,
                _ => (r.next_u32() >> (1 + r.next_u32() % 31)) as i32,
            })
            .collect();
        check(n, &low, &high, &energy, &format!("iteration {it}"));
    }
}

#[test]
fn spreading_max_realistic() {
    let mut count = 0;
    for conf in configs() {
        for mut frame in frames(&conf, 8) {
            let pc = *frame.conf(&conf);
            let (nw, wl) = frame.windows(&conf);
            let scale = normalize(&mut frame, &conf);
            let max_sfb = if frame.short { 15 } else { 60 };
            for ch in 0..2 {
                for w in 0..nw {
                    let spec = &frame.spectrum[ch][w * wl..];
                    let sc = &scale[ch][w * max_sfb..];
                    let mut nrg = vec![0i32; 64];
                    let mut ld = vec![0i32; 64];
                    if frame.short {
                        fdk_aac_enc_calc_band_energy_optim_short(spec, sc, &pc.sfb_offset, pc.sfb_active, &mut nrg);
                    } else {
                        fdk_aac_enc_calc_band_energy_optim_long(spec, sc, &pc.sfb_offset, pc.sfb_active, &mut nrg, &mut ld);
                    }
                    let thr: Vec<i32> = nrg.iter().map(|&e| f_mult(e, C_RATIO)).collect();
                    let what = format!("{} short {}", conf.name, frame.short);
                    check(pc.sfb_active, &pc.sfb_mask_low_factor, &pc.sfb_mask_high_factor, &thr, &what);
                    check(pc.sfb_active, &pc.sfb_mask_low_factor_spr_en, &pc.sfb_mask_high_factor_spr_en, &nrg, &what);
                    count += 1;
                }
            }
        }
    }
    assert!(count > 1000);
}

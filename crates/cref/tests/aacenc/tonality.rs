// Differential tests of fdk_aac_rs_aacenc::tonality against the C++ reference.
//
// FDKaacEnc_CalculateFullTonality (with and without PNS) and its static kernel
// FDKaacEnc_CalcSfbTonality are compared on random data and on MDCT spectra of real and
// synthetic PCM with the band layouts, headrooms and energies of FDKaacEnc_psyMain().

use crate::band_nrg::{configs, frames, normalize, random_offsets, random_spectrum};
use fdk_aac_rs_aacenc::band_nrg::{
    fdk_aac_enc_calc_band_energy_optim_long, fdk_aac_enc_calc_sfb_max_scale_spec,
};
use fdk_aac_rs_aacenc::tonality::*;
use fdk_aac_rs_cref::aacenc_tonality as c;
use fdk_aac_rs_cref::testutil::Rng;

#[allow(clippy::too_many_arguments)]
fn check_full(spec: &[i32], scale: &[i32], ld: &[i32], n: i32, offsets: &[i32], use_pns: i32, what: &str) {
    let mut s = spec.to_vec();
    s.resize(1024, 0);
    let (mut sc, mut l) = (scale.to_vec(), ld.to_vec());
    let mut a = vec![-3i16; 64];
    let mut b = a.clone();
    fdk_aac_enc_calculate_full_tonality(&s, scale, ld, &mut a, n, offsets, use_pns);
    unsafe {
        c::cref_tonality_CalculateFullTonality(
            s.as_mut_ptr(),
            sc.as_mut_ptr(),
            l.as_mut_ptr(),
            b.as_mut_ptr(),
            n,
            offsets.as_ptr(),
            use_pns,
        )
    };
    assert_eq!(a, b, "{what}: CalculateFullTonality");
}

#[test]
fn normlog() {
    assert_eq!(NORMLOG, unsafe { c::cref_tonality_normlog() });
}

#[test]
fn calc_sfb_tonality_random() {
    let mut r = Rng::new(71);
    for it in 0..4000 {
        let nb = r.range(0, 51) as usize;
        let offsets = random_offsets(&mut r, nb, 32);
        let mut spec = random_spectrum(&mut r, &offsets);
        spec.resize(1100, 0);
        let mut scale = vec![0i32; nb.max(1)];
        fdk_aac_enc_calc_sfb_max_scale_spec(&spec, &offsets, &mut scale, nb as i32);
        for x in scale.iter_mut() {
            if r.next_u32() % 4 == 0 {
                *x = r.range(0, 30);
            }
        }
        let mut cm: Vec<i32> = (0..1100).map(|_| (r.next_u32() >> 1) as i32).collect();
        let mut ld: Vec<i32> = (0..64).map(|_| r.i32()).collect();
        let mut a = vec![-3i16; 64];
        let mut b = a.clone();
        let mut sc = scale.clone();
        fdk_aac_enc_calc_sfb_tonality(&spec, &scale, &cm, &mut a, nb as i32, &offsets, &ld);
        unsafe {
            c::cref_tonality_CalcSfbTonality(
                spec.as_mut_ptr(),
                sc.as_mut_ptr(),
                cm.as_mut_ptr(),
                b.as_mut_ptr(),
                nb as i32,
                offsets.as_ptr(),
                ld.as_mut_ptr(),
            )
        };
        assert_eq!(a, b, "iteration {it}: CalcSfbTonality");
        if *offsets.last().unwrap() >= 5 && *offsets.last().unwrap() <= 1024 {
            // inputs without a defined reference result are skipped
            check_full(&spec, &scale, &ld, nb as i32, &offsets, r.range(0, 1), &format!("iteration {it}"));
        }
    }
}

#[test]
fn full_tonality_realistic() {
    let mut count = 0;
    for conf in configs() {
        for mut frame in frames(&conf, 8) {
            if frame.short {
                continue;
            }
            let pc = conf.long;
            let scale = normalize(&mut frame, &conf);
            for ch in 0..2 {
                let mut nrg = vec![0i32; 64];
                let mut ld = vec![0i32; 64];
                fdk_aac_enc_calc_band_energy_optim_long(
                    &frame.spectrum[ch],
                    &scale[ch],
                    &pc.sfb_offset,
                    pc.sfb_active,
                    &mut nrg,
                    &mut ld,
                );
                for use_pns in [0, 1] {
                    check_full(&frame.spectrum[ch], &scale[ch], &ld, pc.sfb_active, &pc.sfb_offset, use_pns, &conf.name);
                }
                count += 1;
            }
        }
    }
    assert!(count > 300);
}

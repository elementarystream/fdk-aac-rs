// Differential tests of fdk_aac_rs_aacenc::noisedet against the C++ reference.
//
// FDKaacEnc_noiseDetect runs with the PNS parameters of the encoder (FDKaacEnc_GetPnsParam on the
// band tables of FDKaacEnc_InitPsyConfiguration) on MDCT spectra of real PCM and on random
// spectra; the fuzzy measures are compared.

use crate::aacenc_tns_spectra::spectra;
use crate::psy_configuration::{init_psy_checked, RATES};
use core::ffi::c_void;
use fdk_aac_rs_aacenc::noisedet::*;
use fdk_aac_rs_aacenc::pnsparam::*;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_cref::aacenc_noisedet as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;

#[test]
fn fuzzy_is_smaller() {
    let mut r = Rng::new(0x7a52_0001);
    for _ in 0..200_000 {
        let (t, rf, lo, hi) = (r.i32(), r.i32(), r.i32(), r.i32());
        let cc = unsafe { c::cref_noisedet_fuzzyIsSmaller(t, rf, lo, hi) };
        assert_eq!(fdk_aac_enc_fuzzy_is_smaller(t, rf, lo, hi), cc, "{t} {rf} {lo} {hi}");
    }
}

/// `FDKaacEnc_CalcSfbMaxScaleSpec` of the encoder (the input of the noise detection).
pub fn sfb_max_scale_spec(spec: &[i32], off: &[i32], n: usize) -> Vec<i32> {
    (0..n)
        .map(|i| {
            let mut m = 0;
            for j in off[i]..off[i + 1] {
                m = fix_max(m, fixp_abs(spec[j as usize]));
            }
            fix_min(DFRACT_BITS - 2, cnt_leading_zeros(m) - 1)
        })
        .collect()
}

/// The NOISEPARAMS of the encoder for a band table (PNS active).
pub fn noise_params(r: &mut Rng, sr: i32, cnt: i32, off: &[i32]) -> Option<NoiseParams> {
    for _ in 0..20 {
        let mut np = NoiseParams::default();
        let mut use_pns = 1;
        let br = [16000, 24000, 28000, 32000, 40000, 48000, 56000][r.range(0, 6) as usize];
        fdk_aac_enc_get_pns_param(&mut np, br, sr, cnt, off, &mut use_pns, r.range(1, 2), r.range(0, 1));
        if use_pns > 0 {
            return Some(np);
        }
    }
    None
}

fn check(spec: &[i32], ssc: &[i32], active: i32, off: &[i32], np: &NoiseParams, ton: &[i16], what: &str) {
    let mut np_c = *np;
    let fill = 0x1234;
    let (mut fc, mut fr) = ([fill as i16; MAX_GROUPED_SFB], [fill as i16; MAX_GROUPED_SFB]);
    let (mut sc, mut ssc_c, mut tc) = (spec.to_vec(), ssc.to_vec(), ton.to_vec());
    unsafe {
        c::cref_noisedet_noiseDetect(
            sc.as_mut_ptr(),
            ssc_c.as_mut_ptr(),
            active,
            off.as_ptr(),
            fc.as_mut_ptr(),
            &mut np_c as *mut NoiseParams as *mut c_void,
            tc.as_mut_ptr(),
        )
    };
    fdk_aac_enc_noise_detect(spec, ssc, active, off, &mut fr, np, ton);
    assert_eq!(fr, fc, "{what}: noiseFuzzyMeasure");
}

#[test]
fn noise_detect() {
    let mut r = Rng::new(0x7a52_0002);
    let mut tonal = [0usize; 2];
    for (k, &sr) in RATES.iter().enumerate() {
        for &(gl, fb) in &[(1024, FB_LC), (512, FB_LD), (480, FB_ELD)] {
            let (_, p) = init_psy_checked(64000, sr, sr / 2, LONG_WINDOW, gl, 0, 0, fb);
            let Some(np0) = noise_params(&mut r, sr, p.sfb_cnt, &p.sfb_offset) else { continue };
            let frames = spectra(&mut r, k, gl, gl <= 512, 16);
            for (i, f) in frames.iter().enumerate() {
                if f.block_type == SHORT_WINDOW {
                    continue;
                }
                let mut np = np0;
                if i % 4 == 3 {
                    np.detection_algorithm_flags = r.range(0, 63) as u16;
                    np.start_sfb = r.range(0, 20) as i16;
                    np.min_sfb_width = r.range(0, 32);
                    np.ref_tonality = r.range(0, 0x7fff) << 16;
                }
                let spec: Vec<i32> = f.spectrum.iter().map(|&x| x >> r.range(0, 3)).collect();
                let active = p.sfb_active;
                let ssc = sfb_max_scale_spec(&spec, &p.sfb_offset, active as usize);
                let ton: Vec<i16> = (0..MAX_GROUPED_SFB).map(|_| r.range(0, 0x7fff) as i16).collect();
                check(&spec, &ssc, active, &p.sfb_offset, &np, &ton, &format!("sr {sr} gl {gl} frame {i}"));
                let mut fm = [0i16; MAX_GROUPED_SFB];
                fdk_aac_enc_noise_detect(&spec, &ssc, active, &p.sfb_offset, &mut fm, &np, &ton);
                for v in &fm[..active as usize] {
                    tonal[(*v > 0x4000) as usize] += 1;
                }
            }
        }
    }
    assert!(tonal[0] > 0 && tonal[1] > 0, "{tonal:?}");

    // random spectra, scale values and parameters (sfbMaxScaleSpec up to 40: shifts by 32 or
    // more are taken modulo 32 by both reference builds)
    for it in 0..5000 {
        let sr = RATES[r.range(0, 11) as usize];
        let (_, p) = init_psy_checked(64000, sr, sr / 2, LONG_WINDOW, 1024, 0, 0, FB_LC);
        let mut np = noise_params(&mut r, sr, p.sfb_cnt, &p.sfb_offset).unwrap_or_default();
        np.detection_algorithm_flags = r.range(0, 63) as u16;
        if it % 3 == 0 {
            for x in np.pow_dist_psd_curve.iter_mut() {
                *x = r.i16();
            }
            np.ref_tonality = r.i32();
        }
        let spec: Vec<i32> = (0..1024).map(|_| r.i32() >> r.range(0, 31)).collect();
        let ssc: Vec<i32> = if it % 2 == 0 {
            sfb_max_scale_spec(&spec, &p.sfb_offset, p.sfb_active as usize)
        } else {
            (0..p.sfb_active).map(|_| r.range(-5, 40)).collect()
        };
        let ton: Vec<i16> = (0..MAX_GROUPED_SFB).map(|_| r.i16()).collect();
        check(&spec, &ssc, p.sfb_active, &p.sfb_offset, &np, &ton, &format!("it {it}"));
    }
}

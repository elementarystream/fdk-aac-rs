// Differential tests of fdk_aac_rs_aacenc::sf_estim against the C++ reference.
//
// FDKaacEnc_CalcFormFactor and FDKaacEnc_EstimateScaleFactors (and through them every static
// helper: CalcFormFactorChannel, calcSfbRelevantLines, countSingleScfBits, calcSingleSpecPe,
// countScfBitsDiff, calcSpecPeDiff, improveScf, assimilateSingleScf, assimilateMultipleScf,
// assimilateMultipleScf2, EstimateScaleFactorsChannel) on one and two channels, invQuant 0, 1
// and 2, with and without the dead zone quantizer. The complete QC_OUT_CHANNEL and
// PSY_OUT_CHANNEL (without its pointers) are compared after every call; the channels start with
// random contents in the fields the functions do not set (the persistent QC_OUT_CHANNEL memory
// of C). Inputs: realistic channels (sf_estim_data) and random ones.

use crate::sf_estim_data::{channel, ChannelData};
use fdk_aac_rs_aacenc::interface::PsyOutChannel;
use fdk_aac_rs_aacenc::qc_data::QcOutChannel;
use fdk_aac_rs_aacenc::sf_estim::*;
use fdk_aac_rs_cref::aacenc_sf_estim as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};
use std::mem::size_of;

/// Fills a plain-data struct with random words.
fn randomize<T>(rng: &mut Rng, t: &mut T) {
    let p = t as *mut T as *mut u32;
    for k in 0..size_of::<T>() / 4 {
        unsafe { *p.add(k) = rng.next_u32() };
    }
}

fn make(rng: &mut Rng, d: &ChannelData) -> (PsyOutChannel, Box<QcOutChannel>) {
    let mut psy = PsyOutChannel::default();
    randomize(rng, &mut psy);
    psy.sfb_cnt = d.sfb_cnt;
    psy.sfb_per_group = d.sfb_per_group;
    psy.max_sfb_per_group = d.max_sfb_per_group;
    psy.sfb_offsets = d.sfb_offsets;
    let mut qc = Box::<QcOutChannel>::default();
    randomize(rng, &mut *qc);
    // quantized values from an earlier frame stay within the quantizer's range
    for q in qc.quant_spec.iter_mut() {
        *q %= 8192;
    }
    qc.mdct_spectrum = d.spectrum;
    qc.sfb_energy = d.energy;
    qc.sfb_energy_ld_data = d.energy_ld;
    qc.sfb_threshold_ld_data = d.thr_ld;
    qc.sfb_min_snr_ld_data = d.min_snr_ld;
    qc.sfb_spread_energy = d.spread_energy;
    (psy, qc)
}

fn check_form_factor(what: &str, psy: &mut [PsyOutChannel], qc: &mut [QcOutChannel]) {
    let n = psy.len() as i32;
    let mut psy_c = psy.to_vec();
    let mut qc_c = qc.to_vec();
    fdk_aac_enc_calc_form_factor(qc, psy, n);
    unsafe {
        c::cref_sf_estim_CalcFormFactor(
            qc_c.as_mut_ptr() as *mut _,
            psy_c.as_mut_ptr() as *mut _,
            size_of::<PsyOutChannel>() as i32,
            n,
        )
    };
    for ch in 0..psy.len() {
        assert!(psy[ch] == psy_c[ch], "{what}: CalcFormFactor: PSY_OUT_CHANNEL {ch}");
        assert!(
            qc[ch].sfb_form_factor_ld_data == qc_c[ch].sfb_form_factor_ld_data,
            "{what}: CalcFormFactor: sfbFormFactorLdData {ch}: {:?} != {:?}",
            qc[ch].sfb_form_factor_ld_data,
            qc_c[ch].sfb_form_factor_ld_data
        );
        assert!(qc[ch] == qc_c[ch], "{what}: CalcFormFactor: QC_OUT_CHANNEL {ch}");
    }
}

fn check_estimate(what: &str, psy: &[PsyOutChannel], qc: &[QcOutChannel], inv_quant: i32, dz: i32) {
    let n = psy.len() as i32;
    let mut qc_r = qc.to_vec();
    let mut psy_c = psy.to_vec();
    let mut qc_c = qc.to_vec();
    fdk_aac_enc_estimate_scale_factors(psy, &mut qc_r, inv_quant, dz, n);
    unsafe {
        c::cref_sf_estim_EstimateScaleFactors(
            psy_c.as_mut_ptr() as *mut _,
            qc_c.as_mut_ptr() as *mut _,
            size_of::<PsyOutChannel>() as i32,
            inv_quant,
            dz,
            n,
        )
    };
    for ch in 0..psy.len() {
        assert!(psy[ch] == psy_c[ch], "{what}: PSY_OUT_CHANNEL {ch}");
        let (r, cc) = (&qc_r[ch], &qc_c[ch]);
        assert_eq!(
            (r.global_gain, &r.scf[..]),
            (cc.global_gain, &cc.scf[..]),
            "{what} (invQuant {inv_quant}, dz {dz}): globalGain, scf {ch}"
        );
        assert!(r.quant_spec == cc.quant_spec, "{what} (invQuant {inv_quant}): quantSpec {ch}");
        assert!(r.mdct_spectrum == cc.mdct_spectrum, "{what}: mdctSpectrum {ch}");
        assert!(r == cc, "{what}: QC_OUT_CHANNEL {ch}");
    }
}

#[test]
fn layout() {
    assert_eq!(unsafe { c::cref_sf_estim_psy_prefix_size() } as usize, size_of::<PsyOutChannel>());
}

#[test]
fn realistic() {
    let mut rng = Rng::new(0x5f);
    fdk_aac_rs_fdk::fdk_assert::reset_violations();
    for k in 0..240 {
        let n = 1 + (k % 2);
        let short = (k / 2) % 3 == 2;
        let mut psy = vec![];
        let mut qc = vec![];
        for _ in 0..n {
            let mut d = channel(&mut rng, short);
            // more or less demanding thresholds (as the threshold adjustment sets them)
            let lift = [0, 1 << 20, 1 << 22, -(1 << 21)][rng.range(0, 3) as usize];
            for x in d.thr_ld.iter_mut() {
                *x = x.saturating_add(lift).min(0);
            }
            let (p, q) = make(&mut rng, &d);
            psy.push(p);
            qc.push(*q);
        }
        check_form_factor(&format!("realistic {k}"), &mut psy, &mut qc);
        for inv_quant in 0..3 {
            for dz in 0..2 {
                check_estimate(&format!("realistic {k}"), &psy, &qc, inv_quant, dz);
            }
        }
    }
    eprintln!(
        "sf_estim realistic: {} FDK_ASSERT violations (checked profile only)",
        fdk_aac_rs_fdk::fdk_assert::violations()
    );
}

fn random_value(rng: &mut Rng) -> FixpDbl {
    match rng.range(0, 7) {
        0 => [0, MINVAL_DBL, MAXVAL_DBL, 1, -1][rng.range(0, 4) as usize],
        1..=3 => rng.i32() >> rng.range(0, 31),
        _ => rng.i32(),
    }
}

/// Random spectra (with the edge values) and random energies/thresholds in the ld data range
/// (energies also outside it; see the comment on the thresholds).
#[test]
fn random() {
    let mut rng = Rng::new(0x60);
    for k in 0..300 {
        let n = 1 + (k % 2);
        let mut psy = vec![];
        let mut qc = vec![];
        for _ in 0..n {
            let mut d = channel(&mut rng, k % 3 == 2);
            for x in d.spectrum.iter_mut() {
                if rng.range(0, 3) == 0 {
                    *x = random_value(&mut rng);
                }
            }
            for i in 0..60 {
                if rng.range(0, 2) == 0 {
                    d.energy_ld[i] = -((rng.next_u32() >> 1) as i32);
                    d.thr_ld[i] = -((rng.next_u32() >> 1) as i32);
                }
                if rng.range(0, 20) == 0 {
                    d.energy_ld[i] = random_value(&mut rng);
                    // Thresholds stay in the encoder's domain (not positive). With a positive
                    // threshold, FDKaacEnc_improveScf can copy quantSpecTmp lines without a
                    // defined value in the reference (the sentinel -32768 in the port).
                    let t = random_value(&mut rng);
                    d.thr_ld[i] = if t > 0 { -t } else { t };
                }
            }
            let (p, q) = make(&mut rng, &d);
            psy.push(p);
            qc.push(*q);
        }
        check_form_factor(&format!("random {k}"), &mut psy, &mut qc);
        for q in qc.iter_mut() {
            for x in q.sfb_form_factor_ld_data.iter_mut() {
                if rng.range(0, 10) == 0 {
                    *x = random_value(&mut rng);
                }
            }
        }
        for inv_quant in 0..3 {
            check_estimate(&format!("random {k}"), &psy, &qc, inv_quant, rng.range(0, 1));
        }
    }
}

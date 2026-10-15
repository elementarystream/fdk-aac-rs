// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/ms_stereo.h, libAACenc/src/ms_stereo.cpp.

//! Port of `ms_stereo.h`: MS stereo processing.
//!
//! The `aarch64` and `x86` translation units of `ms_stereo.cpp` are identical.
//!
//! # Memory
//!
//! C takes `PSY_DATA *psyData[2]` and `PSY_OUT_CHANNEL *psyOutChannel[2]`. The arrays the
//! function reads and modifies are:
//!
//! | C | port |
//! |---|---|
//! | `psyData[ch]->mdctSpectrum` (= `psyOutChannel[ch]->mdctSpectrum`) | `qc_out_channel[ch].mdct_spectrum` |
//! | `psyOutChannel[ch]->sfbEnergyLdData` | `qc_out_channel[ch].sfb_energy_ld_data` |
//! | `psyOutChannel[ch]->sfbThresholdLdData` | `qc_out_channel[ch].sfb_threshold_ld_data` |
//! | `psyData[ch]->sfbEnergy.Long`, `sfbThreshold.Long`, `sfbSpreadEnergy.Long` | `psy_data[ch].sfb_energy`, ... |
//! | `psyData[0]->sfbEnergyMS.Long` (mid), `psyData[1]->sfbEnergyMS.Long` (side) | `psy_data[ch].sfb_energy_ms` |
//! | `psyData[0]->sfbEnergyMSLdData` (mid), `psyData[1]->sfbEnergyMSLdData` (side) | `psy_data[ch].sfb_energy_ms_ld_data` |
//!
//! `psy_data` and `qc_out_channel` hold the two channels of the pair (`QC_OUT_CHANNEL`s of an
//! element are consecutive, see [`qc_data`](crate::qc_data)). `sfb_offset` is
//! `psyOutChannel[0]->sfbOffsets` in `FDKaacEnc_psyMain()`.

use crate::psy_const::{SI_MS_MASK_ALL, SI_MS_MASK_NONE, SI_MS_MASK_SOME};
use crate::psy_data::PsyData;
use crate::qc_data::QcOutChannel;
use fdk_aac_rs_fdk::common_fix::*;

/// Applies M/S coding to the lines of band `b` and copies the mid/side energies and the minimum
/// thresholds into the left/right arrays (the body shared by both loops of C; `min_threshold_ld`
/// is the minimum of the left/right threshold ld values).
fn apply_ms(
    psy_data: &mut [PsyData],
    qc_out_channel: &mut [QcOutChannel],
    sfb_offset: &[i32],
    b: usize,
    min_threshold_ld_data: FixpDbl,
) {
    let (q0, q1) = qc_out_channel.split_at_mut(1);
    let (q0, q1) = (&mut q0[0], &mut q1[0]);
    let (lo, hi) = (sfb_offset[b] as usize, sfb_offset[b + 1] as usize);
    if lo < hi {
        // (line by line over the band's slices)
        let left = &mut q0.mdct_spectrum[lo..hi];
        let right = &mut q1.mdct_spectrum[lo..hi];
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let spec_l: FixpDbl = *l >> 1;
            let spec_r: FixpDbl = *r >> 1;
            *l = spec_l.wrapping_add(spec_r);
            *r = spec_l.wrapping_sub(spec_r);
        }
    }
    let (p0, p1) = psy_data.split_at_mut(1);
    let (p0, p1) = (&mut p0[0], &mut p1[0]);
    let min_threshold: FixpDbl = fix_min(p0.sfb_threshold[b], p1.sfb_threshold[b]);
    p0.sfb_threshold[b] = min_threshold;
    p1.sfb_threshold[b] = min_threshold;
    q0.sfb_threshold_ld_data[b] = min_threshold_ld_data;
    q1.sfb_threshold_ld_data[b] = min_threshold_ld_data;
    p0.sfb_energy[b] = p0.sfb_energy_ms[b];
    p1.sfb_energy[b] = p1.sfb_energy_ms[b];
    q0.sfb_energy_ld_data[b] = p0.sfb_energy_ms_ld_data[b];
    q1.sfb_energy_ld_data[b] = p1.sfb_energy_ms_ld_data[b];

    let spread = fix_min(p0.sfb_spread_energy[b], p1.sfb_spread_energy[b]) >> 1;
    p0.sfb_spread_energy[b] = spread;
    p1.sfb_spread_energy[b] = spread;
}

/// `FDKaacEnc_MsStereoProcessing()`: decides M/S per band (`ms_mask`, `ms_digest`) and applies
/// it. See the [module documentation](self) for the mapping of the C arguments; `is_book` is
/// `NULL` (`None`) or the intensity codebooks.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_ms_stereo_processing(
    psy_data: &mut [PsyData],
    qc_out_channel: &mut [QcOutChannel],
    is_book: Option<&[i32]>,
    ms_digest: &mut i32,
    ms_mask: &mut [i32],
    allow_ms: i32,
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
    sfb_offset: &[i32],
) {
    let mut ms_mask_true_somewhere: i32 = 0; // to determine msDigest
    let mut num_ms_mask_false: i32 = 0; // number of non-intensity bands where L/R coding is used

    let no_is = |b: usize| is_book.is_none_or(|ib| ib[b] == 0);
    // C loops `for (sfb = 0; sfb < sfbCnt; sfb += sfbPerGroup)`; where the reference has no
    // defined result, the port steps by 1.
    let step = sfb_per_group.max(1) as usize;

    for sfb in (0..sfb_cnt.max(0) as usize).step_by(step) {
        for sfboffs in 0..max_sfb_per_group.max(0) as usize {
            let b = sfb + sfboffs;
            if no_is(b) {
                let thr_ld_l = qc_out_channel[0].sfb_threshold_ld_data[b];
                let thr_ld_r = qc_out_channel[1].sfb_threshold_ld_data[b];
                let nrg_ld_l = qc_out_channel[0].sfb_energy_ld_data[b];
                let nrg_ld_r = qc_out_channel[1].sfb_energy_ld_data[b];

                // minThreshold=min(sfbThresholdLeft[sfb+sfboffs],
                // sfbThresholdRight[sfb+sfboffs])*scaleMinThres; pnlr =
                // (sfbThresholdLeft[sfb+sfboffs]/
                // max(sfbEnergyLeft[sfb+sfboffs],sfbThresholdLeft[sfb+sfboffs]))*
                // (sfbThresholdRight[sfb+sfboffs]/
                // max(sfbEnergyRight[sfb+sfboffs],sfbThresholdRight[sfb+sfboffs]));
                // pnms = (minThreshold/max(sfbEnergyMid[sfb+sfboffs],minThreshold))*
                // (minThreshold/max(sfbEnergySide[sfb+sfboffs],minThreshold));
                // useMS = (pnms > pnlr);

                // we assume that scaleMinThres == 1.0f and we can drop it
                let min_threshold_ld_data: FixpDbl = fix_min(thr_ld_l, thr_ld_r);

                // pnlrLdData = sfbThresholdLeftLdData[sfb+sfboffs] -
                // max(sfbEnergyLeftLdData[sfb+sfboffs], sfbThresholdLeftLdData[sfb+sfboffs]) +
                // sfbThresholdRightLdData[sfb+sfboffs] -
                // max(sfbEnergyRightLdData[sfb+sfboffs], sfbThresholdRightLdData[sfb+sfboffs]);
                let mut tmp: FixpDbl = fix_max(nrg_ld_l, thr_ld_l);
                let mut pnlr_ld_data: FixpDbl = (thr_ld_l >> 1).wrapping_sub(tmp >> 1);
                pnlr_ld_data = pnlr_ld_data.wrapping_add(thr_ld_r >> 1);
                tmp = fix_max(nrg_ld_r, thr_ld_r);
                pnlr_ld_data = pnlr_ld_data.wrapping_sub(tmp >> 1);

                // pnmsLdData = minThresholdLdData - max(sfbEnergyMidLdData[sfb+sfboffs],
                // minThresholdLdData) + minThresholdLdData -
                // max(sfbEnergySideLdData[sfb+sfboffs], minThresholdLdData);
                tmp = fix_max(psy_data[0].sfb_energy_ms_ld_data[b], min_threshold_ld_data);
                let mut pnms_ld_data: FixpDbl = min_threshold_ld_data.wrapping_sub(tmp >> 1);
                tmp = fix_max(psy_data[1].sfb_energy_ms_ld_data[b], min_threshold_ld_data);
                pnms_ld_data = pnms_ld_data.wrapping_sub(tmp >> 1);
                let use_ms = (allow_ms != 0) && (pnms_ld_data > pnlr_ld_data);

                if use_ms {
                    ms_mask[b] = 1;
                    ms_mask_true_somewhere = 1;
                    apply_ms(psy_data, qc_out_channel, sfb_offset, b, min_threshold_ld_data);
                } else {
                    ms_mask[b] = 0;
                    num_ms_mask_false += 1;
                } // useMS
            }
            // isBook
            else {
                // keep mDigest from IS module
                if ms_mask[b] != 0 {
                    ms_mask_true_somewhere = 1;
                }
                // prohibit MS_MASK_ALL in combination with IS
                num_ms_mask_false = 9;
            } // isBook
        } // sfboffs
    } // sfb

    if ms_mask_true_somewhere == 1 {
        if (num_ms_mask_false == 0)
            || ((num_ms_mask_false < max_sfb_per_group) && (num_ms_mask_false < 9))
        {
            *ms_digest = SI_MS_MASK_ALL;
            // loop through M/S bands; if msMask==0, set it to 1 and apply M/S
            for sfb in (0..sfb_cnt.max(0) as usize).step_by(step) {
                for sfboffs in 0..max_sfb_per_group.max(0) as usize {
                    let b = sfb + sfboffs;
                    if no_is(b) && (ms_mask[b] == 0) {
                        ms_mask[b] = 1;
                        // apply M/S coding
                        let min_threshold_ld_data = fix_min(
                            qc_out_channel[0].sfb_threshold_ld_data[b],
                            qc_out_channel[1].sfb_threshold_ld_data[b],
                        );
                        apply_ms(psy_data, qc_out_channel, sfb_offset, b, min_threshold_ld_data);
                    }
                }
            }
        } else {
            *ms_digest = SI_MS_MASK_SOME;
        }
    } else {
        *ms_digest = SI_MS_MASK_NONE;
    }
}

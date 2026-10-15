// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/grp_data.h, libAACenc/src/grp_data.cpp.

//! Port of `grp_data.h`: short block grouping.
//!
//! The `aarch64` and `x86` translation units of `grp_data.cpp` are identical.
//!
//! # Memory
//!
//! The `SFB_*` unions are flat arrays (see [`psy_data`](crate::psy_data)): the function reads
//! `.Short[w][sfb]` (element `w * MAX_SFB_SHORT + sfb`) and writes the grouped values to
//! `.Long[i]` (element `i`) of the same array, in the order of C, so that a write can overwrite
//! a value that a later group reads, exactly as in C.
//!
//! Entries of the outputs that C skips (`groupedSfbOffset`, `groupedSfbMinSnrLdData` and the
//! `.Long` arrays between `sfbActive` and `sfbCnt` of every group) keep their previous contents.
//!
//! The regrouped spectrum is assembled in a scratch buffer (`C_ALLOC_SCRATCH_START`) and copied
//! back for the whole granule. The reference does not define the lines of the bands from
//! `sfbActive` to `sfbCnt` of every window; **the port writes 0 there** (deviation).

use crate::psy_const::{MAX_SFB_SHORT, TRANS_FAC};
use crate::psy_data::{sfb_short, SfbEnergy, SfbThreshold, SFB_UNION_LEN};
use fdk_aac_rs_fdk::common_fix::*;

/// `nrgAddSaturate()` (static in C): Don't use fAddSaturate2() because it looses one bit
/// accuracy which is usefull for quality.
#[inline]
pub fn nrg_add_saturate(a: FixpDbl, b: FixpDbl) -> FixpDbl {
    // `(FIXP_DBL)MAXVAL_DBL - b` wraps (both builds compute it as `b ^ MAXVAL_DBL`)
    if a >= MAXVAL_DBL.wrapping_sub(b) {
        MAXVAL_DBL
    } else {
        a.wrapping_add(b)
    }
}

/// Groups the `.Short` values of an `SFB_*` union into its `.Long` values (one of the four
/// "sum up" loops of `FDKaacEnc_groupShortData()`).
///
/// The windows of the groups add up to `TRANS_FAC`; a larger sum (never produced by the block
/// switching) reads past the union in C (the next member of `PSY_DATA`); the port reads 0 there.
fn sum_up(
    arr: &mut [FixpDbl; SFB_UNION_LEN],
    sfb_cnt: i32,
    sfb_active: i32,
    no_of_groups: i32,
    group_len: &[i32],
) {
    let short = |arr: &[FixpDbl; SFB_UNION_LEN], w: i32, sfb: i32| -> FixpDbl {
        usize::try_from(w as i64 * MAX_SFB_SHORT as i64 + sfb as i64)
            .ok()
            .and_then(|k| arr.get(k))
            .copied()
            .unwrap_or(0)
    };
    let mut wnd: i32 = 0;
    let mut i: i32 = 0;
    for grp in 0..no_of_groups.max(0) as usize {
        let mut sfb: i32 = 0;
        while sfb < sfb_active {
            let mut x = short(arr, wnd, sfb);
            for j in 1..group_len[grp] {
                x = nrg_add_saturate(x, short(arr, wnd + j, sfb));
            }
            arr[i as usize] = x;
            i += 1;
            sfb += 1;
        }
        i += sfb_cnt - sfb;
        wnd += group_len[grp];
    }
}

/// `FDKaacEnc_groupShortData()`: for short blocks: regroup spectrum and group energies and
/// thresholds according to grouping; calculates `maxSfbPerGroup`, the grouped sfb offsets and
/// the grouped minimum SNRs. See the [module documentation](self) for the memory layout.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_group_short_data(
    mdct_spectrum: &mut [FixpDbl],
    sfb_threshold: &mut SfbThreshold,
    sfb_energy: &mut SfbEnergy,
    sfb_energy_ms: &mut SfbEnergy,
    sfb_spread_energy: &mut SfbEnergy,
    sfb_cnt: i32,
    sfb_active: i32,
    sfb_offset: &[i32],
    sfb_min_snr_ld_data: &[FixpDbl],
    grouped_sfb_offset: &mut [i32],
    max_sfb_per_group: &mut i32,
    grouped_sfb_min_snr_ld_data: &mut [FixpDbl],
    no_of_groups: i32,
    group_len: &[i32],
    granule_length: i32,
) {
    let granule_length_short: i32 = granule_length / TRANS_FAC;
    let ngroups = no_of_groups.max(0) as usize;
    let so = |k: i32| sfb_offset[k as usize];

    // `C_ALLOC_SCRATCH_START(tmpSpectrum, FIXP_DBL, (1024))`: zeroed here (see the module
    // documentation).
    let mut tmp_spectrum = [0 as FixpDbl; 1024];

    // for short blocks: regroup spectrum and group energies and thresholds according to
    // grouping

    // calculate maxSfbPerGroup
    let mut highest_sfb: i32 = 0;
    for wnd in 0..TRANS_FAC {
        let mut sfb = sfb_active - 1;
        while sfb >= highest_sfb {
            let mut line = so(sfb + 1) - 1;
            while line >= so(sfb) {
                if mdct_spectrum[(wnd * granule_length_short + line) as usize] != 0 {
                    break; // this band is not completely zero
                }
                line -= 1;
            }
            if line >= so(sfb) {
                break; // this band was not completely zero
            }
            sfb -= 1;
        }
        highest_sfb = fix_max(highest_sfb, sfb);
    }
    highest_sfb = if highest_sfb > 0 { highest_sfb } else { 0 };
    *max_sfb_per_group = highest_sfb + 1;

    // calculate groupedSfbOffset
    let mut i: i32 = 0;
    let mut offset: i32 = 0;
    for grp in 0..ngroups {
        let mut sfb: i32 = 0;
        while sfb < sfb_active + 1 {
            grouped_sfb_offset[i as usize] = offset + so(sfb) * group_len[grp];
            i += 1;
            sfb += 1;
        }
        i += sfb_cnt - sfb;
        offset += group_len[grp] * granule_length_short;
    }
    grouped_sfb_offset[i as usize] = granule_length;

    // calculate groupedSfbMinSnr
    i = 0;
    for _grp in 0..ngroups {
        let mut sfb: i32 = 0;
        while sfb < sfb_active {
            grouped_sfb_min_snr_ld_data[i as usize] = sfb_min_snr_ld_data[sfb as usize];
            i += 1;
            sfb += 1;
        }
        i += sfb_cnt - sfb;
    }

    // sum up sfbThresholds
    sum_up(sfb_threshold, sfb_cnt, sfb_active, no_of_groups, group_len);

    // sum up sfbEnergies left/right
    sum_up(sfb_energy, sfb_cnt, sfb_active, no_of_groups, group_len);

    // sum up sfbEnergies mid/side
    sum_up(sfb_energy_ms, sfb_cnt, sfb_active, no_of_groups, group_len);

    // sum up sfbSpreadEnergies
    sum_up(sfb_spread_energy, sfb_cnt, sfb_active, no_of_groups, group_len);

    // re-group spectrum
    let mut wnd: i32 = 0;
    i = 0;
    for grp in 0..ngroups {
        let mut sfb: i32 = 0;
        while sfb < sfb_active {
            let width = so(sfb + 1) - so(sfb);
            let mut p_mdct_spectrum = so(sfb) + wnd * granule_length_short;
            for _j in 0..group_len[grp] {
                let mut p_tmp = p_mdct_spectrum;
                for _line in 0..width.max(0) {
                    tmp_spectrum[i as usize] = mdct_spectrum[p_tmp as usize];
                    i += 1;
                    p_tmp += 1;
                }
                p_mdct_spectrum += granule_length_short;
            }
            sfb += 1;
        }
        i += group_len[grp] * (so(sfb_cnt) - so(sfb));
        wnd += group_len[grp];
    }

    let n = granule_length.max(0) as usize;
    mdct_spectrum[..n].copy_from_slice(&tmp_spectrum[..n]);
}

const _: () = assert!(sfb_short(TRANS_FAC as usize) == SFB_UNION_LEN);

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/sf_estim.h, libAACenc/src/sf_estim.cpp.

//! Port of `sf_estim.h`: scalefactor estimation.
//!
//! # Memory
//!
//! In C, the functions reach `mdctSpectrum`, `sfbEnergyLdData` and `sfbThresholdLdData` through
//! both `PSY_OUT_CHANNEL` and `QC_OUT_CHANNEL`; they are the arrays of the `QC_OUT_CHANNEL` (see
//! [`interface`](crate::interface)). `FDKaacEnc_EstimateScaleFactorsChannel()` also gets
//! `scf`, `globalGain`, `sfbFormFactorLdData` and `quantSpec` of the same `QC_OUT_CHANNEL` as
//! separate pointers. The port works on the `QcOutChannel` fields.
//!
//! Reads of the following members of the structure (`globalGain` after `scf`, `quantSpec` after
//! `mdctSpectrum`) are reproduced from the C layout; the value read after `scf` is not used.
//!
//! Compiled behavior: the maximum search of `FDKaacEnc_EstimateScaleFactorsChannel()` is
//! vectorized in both builds with an unsigned inner and a signed outer maximum, so a group of 4
//! lines containing `MINVAL_DBL` is ignored (see the comment at the loop).
//!
//! Scratch and stack memory: values that `quantSpecTmp` (`C_ALLOC_SCRATCH`) and the local
//! `sfbDistNew[]` arrays hold from earlier passes of the same call are reproduced (the buffers
//! live as long as in C); for entries the reference does not define, the port uses sentinels and
//! counts an `fdk_assert!` violation.

use crate::bit_cnt::fdk_aac_enc_bit_count_scalefactor_delta;
use crate::interface::PsyOutChannel;
use crate::psy_const::MAX_GROUPED_SFB;
use crate::qc_data::QcOutChannel;
use crate::quantize::{fdk_aac_enc_calc_sfb_dist, fdk_aac_enc_calc_sfb_quant_energy_and_dist};
use fdk_aac_rs_fdk::common_fix::{
    f_abs, f_max, f_mult, fix_max, fix_min, fixp_abs, fl2fxconst_dbl, FixpDbl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::{
    calc_inv_ld_data, calc_ld_data, ld_data_vector, sqrt_fixp, LD_DATA_SHIFT,
};
use fdk_aac_rs_fdk::generic_stds::{FDK_INT_MAX, FDK_INT_MIN};

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

/// `FORM_FAC_SHIFT`
pub const FORM_FAC_SHIFT: i32 = 6;

/// `UPCOUNT_LIMIT`
const UPCOUNT_LIMIT: i32 = 1;
/// `AS_PE_FAC_SHIFT`
const AS_PE_FAC_SHIFT: i32 = 7;
/// `DIST_FAC_SHIFT`
const DIST_FAC_SHIFT: i32 = 3;
/// `AS_PE_FAC_FLOAT`
const AS_PE_FAC_FLOAT: f32 = (1 << AS_PE_FAC_SHIFT) as f32;
/// `MAX_SCF_DELTA`
const MAX_SCF_DELTA: i32 = 60;

/// `FL2FXCONST_DBL(x)` of a `float` constant.
const fn fl(x: f32) -> FixpDbl {
    fl2fxconst_dbl(x as f64)
}

/// `PE_C1`: (log(8.0)/log(2)) >> AS_PE_FAC_SHIFT
const PE_C1: FixpDbl = fl(3.0f32 / AS_PE_FAC_FLOAT);
/// `PE_C2`: (log(2.5)/log(2)) >> AS_PE_FAC_SHIFT
const PE_C2: FixpDbl = fl(1.3219281f32 / AS_PE_FAC_FLOAT);
/// `PE_C3`: 1-C2/C1
const PE_C3: FixpDbl = fl(0.5593573f32);

/// `FDKaacEnc_FDKaacEnc_CalcFormFactorChannel`: Calculates the formfactor
///
/// sf: scale factor of the mdct spectrum
/// sfbFormFactorLdData is scaled with the factor 1/(((2^sf)^0.5) * (2^FORM_FAC_SHIFT))
fn fdk_aac_enc_fdk_aac_enc_calc_form_factor_channel(
    sfb_form_factor_ld_data: &mut [FixpDbl],
    psy_out_chan: &PsyOutChannel,
    mdct_spectrum: &[FixpDbl],
) {
    let tmp0 = psy_out_chan.sfb_cnt;
    let tmp1 = psy_out_chan.max_sfb_per_group;
    let step = psy_out_chan.sfb_per_group;
    let mut sfb_grp = 0;
    // the sums of a group, then their ld values with the vector CalcLdData (each ld value
    // depends only on its sum)
    let mut form_factor = [0 as FixpDbl; MAX_GROUPED_SFB];
    while sfb_grp < tmp0 {
        let mut sfb = 0;
        while sfb < tmp1 {
            let i = (sfb_grp + sfb) as usize;
            // calc sum of sqrt(spec)
            let (lo, hi) = (psy_out_chan.sfb_offsets[i], psy_out_chan.sfb_offsets[i + 1]);
            form_factor[sfb as usize] = if lo < hi {
                form_factor_sum(&mdct_spectrum[lo as usize..hi as usize])
            } else {
                fl(0.0)
            };
            sfb += 1;
        }
        if sfb > 0 {
            let g = sfb_grp as usize;
            ld_data_vector(&form_factor[..sfb as usize], &mut sfb_form_factor_ld_data[g..]);
        }
        // set sfbFormFactor for sfbs with zero spec to zero. Just for debugging.
        while sfb < psy_out_chan.sfb_per_group {
            sfb_form_factor_ld_data[(sfb_grp + sfb) as usize] = fl(-1.0);
            sfb += 1;
        }
        sfb_grp += step;
    }
}

/// The sum of `sqrtFixp(fixp_abs(spec[j])) >> FORM_FAC_SHIFT` (wrapping) over a band
/// (straight port of the loop of `FDKaacEnc_FDKaacEnc_CalcFormFactorChannel()`).
#[doc(hidden)]
pub fn form_factor_sum_straight(spec: &[FixpDbl]) -> FixpDbl {
    let mut form_factor: FixpDbl = fl(0.0);
    for &x in spec {
        form_factor = form_factor.wrapping_add(sqrt_fixp(fixp_abs(x)) >> FORM_FAC_SHIFT);
    }
    form_factor
}

/// [`form_factor_sum_straight`], with the NEON kernel on AArch64 for the variants whose
/// `sqrtFixp()` is the integer code, and the AVX2 kernels on x86 for the `x86` (single precision)
/// and `x86_32` (x87) variants.
#[doc(hidden)]
#[inline]
pub fn form_factor_sum(spec: &[FixpDbl]) -> FixpDbl {
    #[cfg(target_arch = "aarch64")]
    if matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::Aarch64 | fdk_aac_rs_fdk::Arch::Generic)
    {
        return neon::form_factor_sum(spec);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if crate::simd::avx2() {
        match fdk_aac_rs_fdk::ARCH {
            // SAFETY: the CPU supports AVX2.
            fdk_aac_rs_fdk::Arch::X86 => return unsafe { x86::form_factor_sum_x86(spec) },
            // SAFETY: the CPU supports AVX2.
            fdk_aac_rs_fdk::Arch::X86_32 => return unsafe { x86::form_factor_sum_x86_32(spec) },
            _ => {}
        }
    }
    form_factor_sum_straight(spec)
}

/// `FDKaacEnc_CalcFormFactor`: Calls `FDKaacEnc_FDKaacEnc_CalcFormFactorChannel()` for each
/// channel.
///
/// `qc_out_channel[j]` provides `mdctSpectrum` (read) and `sfbFormFactorLdData` (written) of
/// channel `j`.
pub fn fdk_aac_enc_calc_form_factor(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    n_channels: i32,
) {
    for j in 0..n_channels.max(0) as usize {
        let qc = &mut qc_out_channel[j];
        fdk_aac_enc_fdk_aac_enc_calc_form_factor_channel(
            &mut qc.sfb_form_factor_ld_data,
            &psy_out_channel[j],
            &qc.mdct_spectrum,
        );
    }
}

/// `FDKaacEnc_calcSfbRelevantLines`: Calculates sfbNRelevantLines
///
/// sfbNRelevantLines is scaled with the factor 1/((2^FORM_FAC_SHIFT) * 2.0)
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_calc_sfb_relevant_lines(
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_energy_ld_data: &[FixpDbl],
    sfb_threshold_ld_data: &[FixpDbl],
    sfb_offsets: &[i32],
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
    sfb_n_relevant_lines: &mut [FixpDbl],
) {
    let as_pe_fac_ld_data: FixpDbl = fl2fxconst_dbl(0.109375); // AS_PE_FAC_SHIFT*ld64(2)

    // sfbNRelevantLines[i] = 2^( (sfbFormFactorLdData[i] - 0.25 * (sfbEnergyLdData[i] -
    // ld64(sfbWidth[i]/(2^7)) - AS_PE_FAC_SHIFT*ld64(2)) * 64);

    for x in sfb_n_relevant_lines[..sfb_cnt.max(0) as usize].iter_mut() {
        *x = 0;
    }

    // CalcLdData of every band width of a group with the vector CalcLdData: the values of
    // the bands that use them are the same, the others are not used
    let mut width_ld = [0 as FixpDbl; MAX_GROUPED_SFB];
    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        let n = max_sfb_per_group.max(0) as usize;
        for (sfb, w) in width_ld[..n].iter_mut().enumerate() {
            let i = sfb_offs as usize + sfb;
            let sfb_width = sfb_offsets[i + 1] - sfb_offsets[i];
            *w = sfb_width.wrapping_shl((DFRACT_BITS - 1 - AS_PE_FAC_SHIFT) as u32);
        }
        let widths = width_ld;
        ld_data_vector(&widths[..n], &mut width_ld);
        for sfb in 0..max_sfb_per_group {
            let i = (sfb_offs + sfb) as usize;
            // calc sum of sqrt(spec)
            if sfb_energy_ld_data[i] > sfb_threshold_ld_data[i] {
                // avgFormFactorLdData = sqrtFixp(sqrtFixp(sfbEnergyLdData[sfbOffs+sfb]/sfbWidth));
                // sfbNRelevantLines[sfbOffs+sfb] = sfbFormFactor[sfbOffs+sfb] / avgFormFactorLdData;
                // CalcLdData(sfbWidth << (DFRACT_BITS - 1 - AS_PE_FAC_SHIFT))
                let sfb_width_ld_data: FixpDbl = width_ld[sfb as usize];

                let mut accu = sfb_energy_ld_data[i]
                    .wrapping_sub(sfb_width_ld_data)
                    .wrapping_sub(as_pe_fac_ld_data);
                accu = sfb_form_factor_ld_data[i].wrapping_sub(accu >> 2);

                sfb_n_relevant_lines[i] = calc_inv_ld_data(accu) >> 1;
            }
        }
        sfb_offs += sfb_per_group;
    }
}

/// `FDKaacEnc_countSingleScfBits`
///
/// scfBitsFract is scaled by 1/(2^(2*AS_PE_FAC_SHIFT))
fn fdk_aac_enc_count_single_scf_bits(scf: i32, scf_left: i32, scf_right: i32) -> FixpDbl {
    let scf_bits_fract: FixpDbl =
        fdk_aac_enc_bit_count_scalefactor_delta(scf_left.wrapping_sub(scf))
            .wrapping_add(fdk_aac_enc_bit_count_scalefactor_delta(scf.wrapping_sub(scf_right)));

    // output scaled by 1/(2^(2*AS_PE_FAC))
    scf_bits_fract.wrapping_shl((DFRACT_BITS - 1 - (2 * AS_PE_FAC_SHIFT)) as u32)
}

/// `FDKaacEnc_calcSingleSpecPe`
///
/// specPe is scaled by 1/(2^(2*AS_PE_FAC_SHIFT))
fn fdk_aac_enc_calc_single_spec_pe(
    scf: i32,
    sfb_const_pe_part: FixpDbl,
    n_lines: FixpDbl,
) -> FixpDbl {
    let scf_fract: FixpDbl = scf.wrapping_shl((DFRACT_BITS - 1 - AS_PE_FAC_SHIFT) as u32);

    let ld_ratio = sfb_const_pe_part.wrapping_sub(f_mult(fl(0.375f32), scf_fract));

    // output scaled by 1/(2^(2*AS_PE_FAC))
    if ld_ratio >= PE_C1 {
        f_mult(fl(0.7f32), f_mult(n_lines, ld_ratio))
    } else {
        f_mult(fl(0.7f32), f_mult(n_lines, PE_C2.wrapping_add(f_mult(PE_C3, ld_ratio))))
    }
}

/// `FDKaacEnc_countScfBitsDiff`
///
/// scfBitsDiff is scaled by 1/(2^(2*AS_PE_FAC_SHIFT))
fn fdk_aac_enc_count_scf_bits_diff(
    scf_old: &[i32],
    scf_new: &[i32],
    sfb_cnt: i32,
    start_sfb: i32,
    stop_sfb: i32,
) -> FixpDbl {
    let mut scf_bits_diff: i32 = 0;
    let bits = fdk_aac_enc_bit_count_scalefactor_delta;
    let o = |i: i32| scf_old[i as usize];
    let n = |i: i32| scf_new[i as usize];

    // search for first relevant sfb
    let mut sfb_last = start_sfb;
    while (sfb_last < stop_sfb) && (o(sfb_last) == FDK_INT_MIN) {
        sfb_last += 1;
    }
    // search for previous relevant sfb and count diff
    let mut sfb_prev = start_sfb - 1;
    while (sfb_prev >= 0) && (o(sfb_prev) == FDK_INT_MIN) {
        sfb_prev -= 1;
    }
    if sfb_prev >= 0 {
        scf_bits_diff = scf_bits_diff
            .wrapping_add(bits(n(sfb_prev).wrapping_sub(n(sfb_last))))
            .wrapping_sub(bits(o(sfb_prev).wrapping_sub(o(sfb_last))));
    }
    // now loop through all sfbs and count diffs of relevant sfbs
    for sfb in sfb_last + 1..stop_sfb {
        if o(sfb) != FDK_INT_MIN {
            scf_bits_diff = scf_bits_diff
                .wrapping_add(bits(n(sfb_last).wrapping_sub(n(sfb))))
                .wrapping_sub(bits(o(sfb_last).wrapping_sub(o(sfb))));
            sfb_last = sfb;
        }
    }
    // search for next relevant sfb and count diff
    let mut sfb_next = stop_sfb;
    while (sfb_next < sfb_cnt) && (o(sfb_next) == FDK_INT_MIN) {
        sfb_next += 1;
    }
    if sfb_next < sfb_cnt {
        scf_bits_diff = scf_bits_diff
            .wrapping_add(bits(n(sfb_last).wrapping_sub(n(sfb_next))))
            .wrapping_sub(bits(o(sfb_last).wrapping_sub(o(sfb_next))));
    }

    scf_bits_diff.wrapping_shl((DFRACT_BITS - 1 - (2 * AS_PE_FAC_SHIFT)) as u32)
}

/// `sfbConstPePart[sfb]` if it is not computed yet (`FDK_INT_MIN`):
/// `sfbConstPePart[sfb] = (float)log(sfbEnergy[sfb] * 6.75f / sfbFormFactor[sfb]) * LOG2_1;`
///
/// 0.02152255861f = log(6.75)/log(2)/AS_PE_FAC_FLOAT; LOG2_1 is 1.0 for log2;
/// 0.09375f = log(64.0)/log(2.0)/64.0 = scale of sfbFormFactorLdData
#[inline]
fn const_pe_part(sfb_const_pe_part: &mut FixpDbl, en_ld_data: FixpDbl, form_factor_ld: FixpDbl) {
    if *sfb_const_pe_part == FDK_INT_MIN {
        *sfb_const_pe_part = (en_ld_data.wrapping_sub(form_factor_ld).wrapping_sub(fl(0.09375f32))
            >> 1)
            .wrapping_add(fl(0.02152255861f32));
    }
}

/// `FDKaacEnc_calcSpecPeDiff`
///
/// specPeDiff is scaled by 1/(2^(2*AS_PE_FAC_SHIFT))
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_calc_spec_pe_diff(
    sfb_energy_ld_data: &[FixpDbl],
    scf_old: &[i32],
    scf_new: &[i32],
    sfb_const_pe_part: &mut [FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_n_relevant_lines: &[FixpDbl],
    start_sfb: i32,
    stop_sfb: i32,
) -> FixpDbl {
    let mut spec_pe_diff: FixpDbl = fl(0.0);

    // loop through all sfbs and count pe difference
    for sfb in start_sfb as usize..stop_sfb.max(start_sfb) as usize {
        if scf_old[sfb] != FDK_INT_MIN {
            const_pe_part(
                &mut sfb_const_pe_part[sfb],
                sfb_energy_ld_data[sfb],
                sfb_form_factor_ld_data[sfb],
            );

            let scf_fract = scf_old[sfb].wrapping_shl((DFRACT_BITS - 1 - AS_PE_FAC_SHIFT) as u32);
            let ld_ratio_old = sfb_const_pe_part[sfb].wrapping_sub(f_mult(fl(0.375f32), scf_fract));

            let scf_fract = scf_new[sfb].wrapping_shl((DFRACT_BITS - 1 - AS_PE_FAC_SHIFT) as u32);
            let ld_ratio_new = sfb_const_pe_part[sfb].wrapping_sub(f_mult(fl(0.375f32), scf_fract));

            let p_old = if ld_ratio_old >= PE_C1 {
                ld_ratio_old
            } else {
                PE_C2.wrapping_add(f_mult(PE_C3, ld_ratio_old))
            };

            let p_new = if ld_ratio_new >= PE_C1 {
                ld_ratio_new
            } else {
                PE_C2.wrapping_add(f_mult(PE_C3, ld_ratio_new))
            };

            spec_pe_diff = spec_pe_diff.wrapping_add(f_mult(
                fl(0.7f32),
                f_mult(sfb_n_relevant_lines[sfb], p_new.wrapping_sub(p_old)),
            ));
        }
    }

    spec_pe_diff
}

/// The bands with a relevant scalefactor (`scf[sfb] != FDK_INT_MIN`) in ascending order, and for
/// every band `0..=sfbCnt` the position (in that list) of the first relevant band at or after
/// it.
///
/// The assimilation functions only replace relevant scalefactors by other values, so the list
/// built at their start stays valid while they run; [`fdk_aac_enc_count_scf_bits_diff`] and
/// [`fdk_aac_enc_calc_spec_pe_diff`] then visit the relevant bands of a region from the list
/// instead of testing every band (`count_scf_bits_diff_rel`, `calc_spec_pe_diff_rel`).
struct RelBands {
    idx: [u8; MAX_GROUPED_SFB],
    n: usize,
    first_at: [u8; MAX_GROUPED_SFB + 1],
    sfb_cnt: i32,
}

impl RelBands {
    /// The relevant bands of `scf[..sfb_cnt]`, or `None` for `sfb_cnt` outside
    /// `0..=MAX_GROUPED_SFB` (the callers then use the straight functions).
    fn new(scf: &[i32], sfb_cnt: i32) -> Option<Self> {
        if !(0..=MAX_GROUPED_SFB as i32).contains(&sfb_cnt) {
            return None;
        }
        let mut r = RelBands {
            idx: [0; MAX_GROUPED_SFB],
            n: 0,
            first_at: [0; MAX_GROUPED_SFB + 1],
            sfb_cnt,
        };
        for (sfb, &s) in scf[..sfb_cnt as usize].iter().enumerate() {
            r.first_at[sfb] = r.n as u8;
            if s != FDK_INT_MIN {
                r.idx[r.n] = sfb as u8;
                r.n += 1;
            }
        }
        r.first_at[sfb_cnt as usize] = r.n as u8;
        // first_at[sfb] counted the relevant bands before sfb, which is the position of the
        // first relevant band at or after sfb
        Some(r)
    }

    /// The positions `p0..p1` of the relevant bands in `start..stop`, or `None` if the region
    /// is outside `0..=sfbCnt` or holds no relevant band (the straight functions then run).
    #[inline]
    fn region(&self, start: i32, stop: i32) -> Option<(usize, usize)> {
        if start < 0 || stop > self.sfb_cnt || start >= stop {
            return None;
        }
        let (p0, p1) =
            (self.first_at[start as usize] as usize, self.first_at[stop as usize] as usize);
        (p0 < p1).then_some((p0, p1))
    }
}

/// [`fdk_aac_enc_count_scf_bits_diff`] for the relevant bands `rel` of `scf_old`.
///
/// The straight function sums `bits(new[a] - new[b]) - bits(old[a] - old[b])` over the pairs of
/// consecutive relevant bands from the last relevant band before `start_sfb` (if any) over the
/// relevant bands of the region to the first relevant band at or after `stop_sfb` (if any); a
/// region with relevant bands starts its chain at its first one. The list gives these bands
/// directly; the wrapping sum does not depend on the order.
#[inline]
fn count_scf_bits_diff_rel(
    scf_old: &[i32],
    scf_new: &[i32],
    rel: Option<&RelBands>,
    sfb_cnt: i32,
    start_sfb: i32,
    stop_sfb: i32,
) -> FixpDbl {
    let Some((rel, (p0, p1))) = rel.and_then(|r| Some((r, r.region(start_sfb, stop_sfb)?))) else {
        return fdk_aac_enc_count_scf_bits_diff(scf_old, scf_new, sfb_cnt, start_sfb, stop_sfb);
    };
    debug_assert_eq!(rel.sfb_cnt, sfb_cnt);
    let bits = fdk_aac_enc_bit_count_scalefactor_delta;
    let pair = |a: u8, b: u8| {
        let (a, b) = (a as usize, b as usize);
        bits(scf_new[a].wrapping_sub(scf_new[b]))
            .wrapping_sub(bits(scf_old[a].wrapping_sub(scf_old[b])))
    };
    let lo = if p0 > 0 { p0 - 1 } else { p0 };
    let hi = if p1 < rel.n { p1 + 1 } else { p1 };
    let mut d: i32 = 0;
    for w in rel.idx[lo..hi].windows(2) {
        d = d.wrapping_add(pair(w[0], w[1]));
    }
    d.wrapping_shl((DFRACT_BITS - 1 - (2 * AS_PE_FAC_SHIFT)) as u32)
}

/// [`fdk_aac_enc_calc_spec_pe_diff`] for the relevant bands `rel` of `scf_old`: the same terms
/// of the relevant bands of the region (in the same order).
#[allow(clippy::too_many_arguments)]
#[inline]
fn calc_spec_pe_diff_rel(
    sfb_energy_ld_data: &[FixpDbl],
    scf_old: &[i32],
    scf_new: &[i32],
    rel: Option<&RelBands>,
    sfb_const_pe_part: &mut [FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_n_relevant_lines: &[FixpDbl],
    start_sfb: i32,
    stop_sfb: i32,
) -> FixpDbl {
    let Some((rel, (p0, p1))) = rel.and_then(|r| Some((r, r.region(start_sfb, stop_sfb)?))) else {
        return fdk_aac_enc_calc_spec_pe_diff(
            sfb_energy_ld_data,
            scf_old,
            scf_new,
            sfb_const_pe_part,
            sfb_form_factor_ld_data,
            sfb_n_relevant_lines,
            start_sfb,
            stop_sfb,
        );
    };
    let mut spec_pe_diff: FixpDbl = fl(0.0);
    let p = |ld_ratio: FixpDbl| {
        if ld_ratio >= PE_C1 {
            ld_ratio
        } else {
            PE_C2.wrapping_add(f_mult(PE_C3, ld_ratio))
        }
    };
    for &sfb in &rel.idx[p0..p1] {
        let sfb = sfb as usize;
        debug_assert!(scf_old[sfb] != FDK_INT_MIN);
        const_pe_part(
            &mut sfb_const_pe_part[sfb],
            sfb_energy_ld_data[sfb],
            sfb_form_factor_ld_data[sfb],
        );
        let cpp = sfb_const_pe_part[sfb];
        let shift = (DFRACT_BITS - 1 - AS_PE_FAC_SHIFT) as u32;
        let ld_ratio_old = cpp.wrapping_sub(f_mult(fl(0.375f32), scf_old[sfb].wrapping_shl(shift)));
        let ld_ratio_new = cpp.wrapping_sub(f_mult(fl(0.375f32), scf_new[sfb].wrapping_shl(shift)));
        let d = p(ld_ratio_new).wrapping_sub(p(ld_ratio_old));
        spec_pe_diff =
            spec_pe_diff.wrapping_add(f_mult(fl(0.7f32), f_mult(sfb_n_relevant_lines[sfb], d)));
    }
    spec_pe_diff
}

/// Copies `n` quantized lines from `quantSpecTmp` to `quantSpec` (`for (k = 0; k < sfbWidth;
/// k++) quantSpec[k] = quantSpecTmp[k];`). Lines of `quantSpecTmp` that no call wrote hold the
/// sentinel -32768 (which the quantizer never produces): the reference does not define them.
#[inline]
fn copy_quant(quant_spec: &mut [i16], quant_spec_tmp: &[i16], offs: usize, n: i32) {
    for k in offs..offs + n.max(0) as usize {
        fdk_assert!(quant_spec_tmp[k] != QUANT_SPEC_TMP_UNWRITTEN);
        quant_spec[k] = quant_spec_tmp[k];
    }
}

/// Initial contents of `quantSpecTmp` (not defined by the reference).
const QUANT_SPEC_TMP_UNWRITTEN: i16 = i16::MIN;
/// Initial contents of the local `sfbDistNew[]` arrays (not defined by the reference; a
/// distortion in ld data format is never positive).
const SFB_DIST_NEW_UNWRITTEN: FixpDbl = FDK_INT_MAX;

/// `FDKaacEnc_improveScf`: Calculate the distortion by quantization and inverse quantization of
/// the spectrum with various scalefactors. The scalefactor which provides the best results will
/// be used.
///
/// `spec`, `quant_spec` and `quant_spec_tmp` start at the band.
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_improve_scf(
    spec: &[FixpDbl],
    quant_spec: &mut [i16],
    quant_spec_tmp: &mut [i16],
    sfb_width: i32,
    thresh_ld_data: FixpDbl,
    mut scf: i32,
    min_scf: i32,
    dist_ld_data: &mut FixpDbl,
    min_scf_calculated: &mut i32,
    d_zone_quant_enable: i32,
) -> i32 {
    let mut scf_best = scf;
    let dist_factor_ld_data: FixpDbl = fl2fxconst_dbl(-0.0050301265); // ld64(1/1.25)

    // calc real distortion
    let mut sfb_dist_ld_data =
        fdk_aac_enc_calc_sfb_dist(spec, quant_spec, sfb_width, scf, d_zone_quant_enable);
    *min_scf_calculated = scf;
    // nmr > 1.25 -> try to improve nmr
    if sfb_dist_ld_data > thresh_ld_data.wrapping_sub(dist_factor_ld_data) {
        let scf_estimated = scf;
        let mut sfb_dist_best_ld_data = sfb_dist_ld_data;
        // improve by bigger scf ?
        let mut cnt = 0;

        while sfb_dist_ld_data > thresh_ld_data.wrapping_sub(dist_factor_ld_data) && {
            let c = cnt;
            cnt += 1;
            c < UPCOUNT_LIMIT
        } {
            scf = scf.wrapping_add(1);
            sfb_dist_ld_data = fdk_aac_enc_calc_sfb_dist(
                spec,
                quant_spec_tmp,
                sfb_width,
                scf,
                d_zone_quant_enable,
            );

            if sfb_dist_ld_data < sfb_dist_best_ld_data {
                scf_best = scf;
                sfb_dist_best_ld_data = sfb_dist_ld_data;
                copy_quant(quant_spec, quant_spec_tmp, 0, sfb_width);
            }
        }
        // improve by smaller scf ?
        cnt = 0;
        scf = scf_estimated;
        sfb_dist_ld_data = sfb_dist_best_ld_data;
        while sfb_dist_ld_data > thresh_ld_data.wrapping_sub(dist_factor_ld_data)
            && {
                let c = cnt;
                cnt += 1;
                c < 1
            }
            && (scf > min_scf)
        {
            scf -= 1;
            sfb_dist_ld_data = fdk_aac_enc_calc_sfb_dist(
                spec,
                quant_spec_tmp,
                sfb_width,
                scf,
                d_zone_quant_enable,
            );

            if sfb_dist_ld_data < sfb_dist_best_ld_data {
                scf_best = scf;
                sfb_dist_best_ld_data = sfb_dist_ld_data;
                copy_quant(quant_spec, quant_spec_tmp, 0, sfb_width);
            }
            *min_scf_calculated = scf;
        }
        *dist_ld_data = sfb_dist_best_ld_data;
    } else {
        // nmr <= 1.25 -> try to find bigger scf to use less bits
        let mut sfb_dist_best_ld_data = sfb_dist_ld_data;
        let sfb_dist_allowed_ld_data =
            fix_min(sfb_dist_ld_data.wrapping_sub(dist_factor_ld_data), thresh_ld_data);
        for _cnt in 0..UPCOUNT_LIMIT {
            scf = scf.wrapping_add(1);
            sfb_dist_ld_data = fdk_aac_enc_calc_sfb_dist(
                spec,
                quant_spec_tmp,
                sfb_width,
                scf,
                d_zone_quant_enable,
            );

            if sfb_dist_ld_data < sfb_dist_allowed_ld_data {
                *min_scf_calculated = scf_best + 1;
                scf_best = scf;
                sfb_dist_best_ld_data = sfb_dist_ld_data;
                copy_quant(quant_spec, quant_spec_tmp, 0, sfb_width);
            }
        }
        *dist_ld_data = sfb_dist_best_ld_data;
    }

    // return best scalefactor
    scf_best
}

/// The read-only per-channel inputs of the assimilation functions.
struct ChannelIn<'a> {
    psy: &'a PsyOutChannel,
    mdct_spectrum: &'a [FixpDbl],
    sfb_energy_ld_data: &'a [FixpDbl],
    /// `psyOutChan->sfbEnergyLdData` of `FDKaacEnc_calcSpecPeDiff()`: `sfb_energy_ld_data`,
    /// unless the `PSY_OUT_CHANNEL` points to another element's arrays (see
    /// [`PsyArrayViews`](crate::adj_thr::PsyArrayViews))
    psy_sfb_energy_ld_data: &'a [FixpDbl],
    sfb_threshold_ld_data: &'a [FixpDbl],
    d_zone_quant_enable: i32,
}

impl ChannelIn<'_> {
    /// `qcOutChannel->mdctSpectrum + sfbOffs`
    fn spec(&self, sfb: usize) -> &[FixpDbl] {
        &self.mdct_spectrum[self.psy.sfb_offsets[sfb] as usize..]
    }
    /// `psyOutChan->sfbOffsets[sfb + 1] - psyOutChan->sfbOffsets[sfb]`
    fn width(&self, sfb: usize) -> i32 {
        self.psy.sfb_offsets[sfb + 1] - self.psy.sfb_offsets[sfb]
    }
    fn offs(&self, sfb: usize) -> usize {
        self.psy.sfb_offsets[sfb] as usize
    }
}

/// The targets of the `INT *scfLast` / `INT *scfNext` pointers of
/// `FDKaacEnc_assimilateSingleScf()`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScfPtr {
    /// `0` (never dereferenced)
    Null,
    /// `scf + i`
    Scf(usize),
    /// `&scfAct` (follows the changes of `scfAct`)
    Act,
}

/// Bound of the scalefactor values for which the fast assimilation functions run: with all
/// values involved within `±SCF_FAST_LIMIT`, every sum and difference of the straight functions
/// (`± 1`, `± MAX_SCF_DELTA`) is exact, so their wrapping arithmetic equals plain integer
/// arithmetic. The encoder's scalefactors lie within `-256..=256`.
const SCF_FAST_LIMIT: i32 = 1 << 20;

/// Whether `scf`, `min_scf` and (if given) `min_scf_calculated` of the relevant bands `rel` lie
/// within `±SCF_FAST_LIMIT`.
fn scf_values_fast(rel: &RelBands, scf: &[i32], min_scf: &[i32], min_calc: Option<&[i32]>) -> bool {
    let ok = |v: i32| (-SCF_FAST_LIMIT..=SCF_FAST_LIMIT).contains(&v);
    rel.idx[..rel.n].iter().all(|&b| {
        let b = b as usize;
        ok(scf[b]) && ok(min_scf[b]) && min_calc.is_none_or(|m| ok(m[b]))
    })
}

/// `FDKaacEnc_assimilateSingleScf`, on the list of relevant bands (the straight port is
/// [`fdk_aac_enc_assimilate_single_scf_straight`]).
///
/// Exactness:
/// - The function only replaces relevant scalefactors by values `scfAct >= scfMin`, and `scfMin`
///   is the maximum of relevant scalefactors and `minScf`, so the set of relevant bands (`scf !=
///   FDK_INT_MIN`) is the same during the whole call. The search `sfbNext++; while (scf[sfbNext]
///   == FDK_INT_MIN) sfbNext++;` therefore visits the bands of the list in order: the visit of
///   position `p` has `sfbAct = idx[p]`, `sfbLast = idx[p - 1]` (or -1) and `sfbNext = idx[p +
///   1]` (or `sfbCnt`). A visit with `sfbAct == -1` (the first one and the first one after a
///   restart) does nothing; a restart therefore continues at position 0.
/// - With at least two relevant bands every visit takes one of the three cases (first, middle,
///   last relevant band), which set `scfAct`, `scfLast`, `scfNext`, `scfMin` and `scfMax`; with
///   fewer the condition is false for every visit (`sfbLast >= 0 || sfbNext < sfbCnt` fails or
///   there is no visit), so nothing changes.
/// - `prevScfLast`, `prevScfNext` and `deltaPeLast` are read and written only at relevant
///   bands; they are kept per position, as are copies of `scf` (updated where the function
///   changes it) and `minScf`.
/// - A visit whose condition is false changes nothing (`success` is then 0 or, without
///   `restartOnSuccess`, has no effect), so the function evaluates the conditions of the next
///   positions without side effects (8 middle positions at a time, without branches) and
///   continues at the first one that holds; if none does, the loop ends as the straight one does
///   after its last visit.
/// - The inner loop runs `scfAct` from `scfAct - 1` down to `scfMin` (it ends with `scfAct ==
///   scfMin`). An iteration does something only if `scfAct < minScfCalculated[sfbAct]` and
///   `scfAct >= scfMax - MAX_SCF_DELTA`; `minScfCalculated[sfbAct]` is only lowered to the
///   current `scfAct`, so these are exactly the values from `min(scfAct, minScfCalculated) - 1`
///   down to `max(scfMin, scfMax - MAX_SCF_DELTA)`, which the loop visits directly. `*scfLast`
///   / `*scfNext` follow `scfAct` where they point to it (first / last relevant band), and are
///   `scfMin` after the loop.
/// - All values are within `±SCF_FAST_LIMIT` (checked, else the straight port runs), so the
///   arithmetic does not wrap; the PE and distortion computations are the same calls in the same
///   order.
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_assimilate_single_scf(
    ch: &ChannelIn,
    quant_spec: &mut [i16],
    quant_spec_tmp: &mut [i16],
    scf: &mut [i32],
    min_scf: &[i32],
    sfb_dist: &mut [FixpDbl],
    sfb_const_pe_part: &mut [FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_n_relevant_lines: &[FixpDbl],
    min_scf_calculated: &mut [i32],
    restart_on_success: i32,
) {
    let sfb_cnt = ch.psy.sfb_cnt;
    let rel = match RelBands::new(scf, sfb_cnt) {
        Some(r) if scf_values_fast(&r, scf, min_scf, Some(min_scf_calculated)) => r,
        _ => {
            return fdk_aac_enc_assimilate_single_scf_straight(
                ch,
                quant_spec,
                quant_spec_tmp,
                scf,
                min_scf,
                sfb_dist,
                sfb_const_pe_part,
                sfb_form_factor_ld_data,
                sfb_n_relevant_lines,
                min_scf_calculated,
                restart_on_success,
            )
        }
    };
    let n = rel.n;
    if n < 2 {
        return;
    }
    let idx = &rel.idx[..n];
    let mut delta_pe: FixpDbl = fl(0.0);
    // per position: prevScfLast, prevScfNext, deltaPeLast, scf, minScf
    let mut prev_scf_last = [FDK_INT_MAX; MAX_GROUPED_SFB];
    let mut prev_scf_next = [FDK_INT_MAX; MAX_GROUPED_SFB];
    let mut delta_pe_last = [FDK_INT_MAX as FixpDbl; MAX_GROUPED_SFB];
    let mut sv = [0i32; MAX_GROUPED_SFB];
    let mut mv = [0i32; MAX_GROUPED_SFB];
    for (p, &b) in idx.iter().enumerate() {
        sv[p] = scf[b as usize];
        mv[p] = min_scf[b as usize];
    }

    // (*scfLast, *scfNext, scfLast == &scfAct, scfNext == &scfAct, scfMin, scfMax) of a visit of
    // position p, with scfMin = fixMax(scfMin, minScf[sfbAct])
    let state = |sv: &[i32; MAX_GROUPED_SFB], p: usize| {
        let s0 = sv[p];
        let (lv, nv, last_act, next_act, scf_min, scf_max) = if p == 0 {
            // first relevant scf
            (s0, sv[1], true, false, sv[1], sv[1])
        } else if p + 1 == n {
            // last relevant scf
            (sv[p - 1], s0, false, true, sv[p - 1], sv[p - 1])
        } else {
            // relevant scfs to the left and to the right
            let (lv, nv) = (sv[p - 1], sv[p + 1]);
            (lv, nv, false, false, fix_min(lv, nv), fix_max(lv, nv))
        };
        (lv, nv, last_act, next_act, fix_max(scf_min, mv[p]), scf_max)
    };
    // the condition of the visit of position p (evaluated without branches)
    let cond = |sv: &[i32; MAX_GROUPED_SFB],
                pl: &[i32; MAX_GROUPED_SFB],
                pn: &[i32; MAX_GROUPED_SFB],
                dpl: &[FixpDbl; MAX_GROUPED_SFB],
                delta_pe: FixpDbl,
                p: usize| {
        let s0 = sv[p];
        let (lv, nv, _, _, scf_min, scf_max) = state(sv, p);
        (s0 > scf_min)
            & (s0 <= scf_min + MAX_SCF_DELTA)
            & (s0 >= scf_max - MAX_SCF_DELTA)
            & (s0 <= fix_min(scf_min, fix_min(lv, nv)) + MAX_SCF_DELTA)
            & ((lv != pl[p]) | (nv != pn[p]) | (delta_pe < dpl[p]))
    };

    let mut p = 0usize;
    loop {
        // Visits whose condition is false change nothing: go to the next visit whose condition
        // holds (blocks of 8 middle positions at a time), or end.
        {
            let (sv, pl, pn, dpl) = (&sv, &prev_scf_last, &prev_scf_next, &delta_pe_last);
            let c = |p: usize| cond(sv, pl, pn, dpl, delta_pe, p);
            let mut q = p;
            let found = 'f: {
                if q == 0 {
                    if c(0) {
                        break 'f true;
                    }
                    q = 1;
                }
                // middle positions (0 < q, q + 8 < n): both neighbours are relevant bands
                while q + 8 < n {
                    let mut bits = 0u32;
                    for k in 0..8 {
                        let r = q + k;
                        let (lv, s0, nv) = (sv[r - 1], sv[r], sv[r + 1]);
                        let scf_min = fix_max(fix_min(lv, nv), mv[r]);
                        let scf_max = fix_max(lv, nv);
                        let t = (s0 > scf_min)
                            & (s0 <= scf_min + MAX_SCF_DELTA)
                            & (s0 >= scf_max - MAX_SCF_DELTA)
                            & (s0 <= fix_min(scf_min, fix_min(lv, nv)) + MAX_SCF_DELTA)
                            & ((lv != pl[r]) | (nv != pn[r]) | (delta_pe < dpl[r]));
                        bits |= (t as u32) << k;
                    }
                    if bits != 0 {
                        q += bits.trailing_zeros() as usize;
                        break 'f true;
                    }
                    q += 8;
                }
                while q < n {
                    if c(q) {
                        break 'f true;
                    }
                    q += 1;
                }
                false
            };
            if !found {
                break;
            }
            p = q;
        }

        let a = idx[p] as usize;
        let scf_act0 = sv[p];
        let (lv, nv, last_act, next_act, scf_min, scf_max) = state(&sv, p);
        // bigger than neighbouring scf found, try to use smaller scf
        let mut success = false;
        {
            let sfb_width = ch.width(a);
            let sfb_offs = ch.offs(a);
            let en_ld_data = ch.sfb_energy_ld_data[a];
            const_pe_part(&mut sfb_const_pe_part[a], en_ld_data, sfb_form_factor_ld_data[a]);
            let cpp = sfb_const_pe_part[a];
            let n_lines = sfb_n_relevant_lines[a];

            let sfb_pe_old = fdk_aac_enc_calc_single_spec_pe(scf_act0, cpp, n_lines)
                .wrapping_add(fdk_aac_enc_count_single_scf_bits(scf_act0, lv, nv));

            let mut delta_pe_new = delta_pe;
            let mut update_min_scf_calculated = true;

            // the values of scfAct for which the inner loop does something (see above)
            let lo = fix_max(scf_min, scf_max - MAX_SCF_DELTA);
            let mut scf_act = fix_min(scf_act0, min_scf_calculated[a]) - 1;
            while scf_act >= lo {
                let l = if last_act { scf_act } else { lv };
                let r = if next_act { scf_act } else { nv };
                // estimate required bits for new scf
                let sfb_pe_new = fdk_aac_enc_calc_single_spec_pe(scf_act, cpp, n_lines)
                    .wrapping_add(fdk_aac_enc_count_single_scf_bits(scf_act, l, r));
                // use new scf if no increase in pe and quantization error is smaller
                let delta_pe_tmp = delta_pe.wrapping_add(sfb_pe_new).wrapping_sub(sfb_pe_old);
                // 0.0006103515625f = 10.0f/(2^(2*AS_PE_FAC_SHIFT))
                if delta_pe_tmp < fl(0.0006103515625f32) {
                    // distortion of new scf
                    let sfb_dist_new = fdk_aac_enc_calc_sfb_dist(
                        ch.spec(a),
                        &mut quant_spec_tmp[sfb_offs..],
                        sfb_width,
                        scf_act,
                        ch.d_zone_quant_enable,
                    );
                    if sfb_dist_new < sfb_dist[a] {
                        // success, replace scf by new one
                        scf[a] = scf_act;
                        sfb_dist[a] = sfb_dist_new;
                        copy_quant(quant_spec, quant_spec_tmp, sfb_offs, sfb_width);
                        delta_pe_new = delta_pe_tmp;
                        success = true;
                    }
                    // mark as already checked
                    if update_min_scf_calculated {
                        min_scf_calculated[a] = scf_act;
                    }
                } else {
                    // from this scf value on not all new values have been checked
                    update_min_scf_calculated = false;
                }
                scf_act -= 1;
            }

            delta_pe = delta_pe_new;
            sv[p] = scf[a];

            // save parameters to avoid multiple computations of the same sfb (scfAct ends at
            // scfMin)
            prev_scf_last[p] = if last_act { scf_min } else { lv };
            prev_scf_next[p] = if next_act { scf_min } else { nv };
            delta_pe_last[p] = delta_pe;
        }

        if success && restart_on_success != 0 {
            // start again at first sfb
            p = 0;
        } else {
            // shift sfbs for next band
            p += 1;
            if p == n {
                break;
            }
        }
    }
}

/// `FDKaacEnc_assimilateSingleScf` (straight port, kept as the reference of
/// [`fdk_aac_enc_assimilate_single_scf`]).
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_assimilate_single_scf_straight(
    ch: &ChannelIn,
    quant_spec: &mut [i16],
    quant_spec_tmp: &mut [i16],
    scf: &mut [i32],
    min_scf: &[i32],
    sfb_dist: &mut [FixpDbl],
    sfb_const_pe_part: &mut [FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_n_relevant_lines: &[FixpDbl],
    min_scf_calculated: &mut [i32],
    restart_on_success: i32,
) {
    let sfb_cnt = ch.psy.sfb_cnt;
    let mut success = 0;
    let mut delta_pe: FixpDbl = fl(0.0);
    let mut prev_scf_last = [0i32; MAX_GROUPED_SFB];
    let mut prev_scf_next = [0i32; MAX_GROUPED_SFB];
    let mut delta_pe_last = [0 as FixpDbl; MAX_GROUPED_SFB];

    for i in 0..sfb_cnt.max(0) as usize {
        prev_scf_last[i] = FDK_INT_MAX;
        prev_scf_next[i] = FDK_INT_MAX;
        delta_pe_last[i] = FDK_INT_MAX;
    }

    let mut sfb_last: i32 = -1;
    let mut sfb_act: i32 = -1;
    let mut sfb_next: i32 = -1;
    let mut scf_act: i32 = 0;
    let mut scf_last = ScfPtr::Null;
    let mut scf_next = ScfPtr::Null;
    let mut scf_min = FDK_INT_MAX;
    let mut scf_max = FDK_INT_MAX;

    // *ptr
    let deref = |p: ScfPtr, scf: &[i32], scf_act: i32| -> i32 {
        match p {
            ScfPtr::Scf(i) => scf[i],
            ScfPtr::Act => scf_act,
            ScfPtr::Null => {
                fdk_assert!(false);
                0
            }
        }
    };

    loop {
        // search for new relevant sfb
        sfb_next += 1;
        while (sfb_next < sfb_cnt) && (scf[sfb_next as usize] == FDK_INT_MIN) {
            sfb_next += 1;
        }
        if (sfb_last >= 0) && (sfb_act >= 0) && (sfb_next < sfb_cnt) {
            // relevant scfs to the left and to the right
            scf_act = scf[sfb_act as usize];
            scf_last = ScfPtr::Scf(sfb_last as usize);
            scf_next = ScfPtr::Scf(sfb_next as usize);
            scf_min = fix_min(scf[sfb_last as usize], scf[sfb_next as usize]);
            scf_max = fix_max(scf[sfb_last as usize], scf[sfb_next as usize]);
        } else if (sfb_last == -1) && (sfb_act >= 0) && (sfb_next < sfb_cnt) {
            // first relevant scf
            scf_act = scf[sfb_act as usize];
            scf_last = ScfPtr::Act;
            scf_next = ScfPtr::Scf(sfb_next as usize);
            scf_min = scf[sfb_next as usize];
            scf_max = scf[sfb_next as usize];
        } else if (sfb_last >= 0) && (sfb_act >= 0) && (sfb_next == sfb_cnt) {
            // last relevant scf
            scf_act = scf[sfb_act as usize];
            scf_last = ScfPtr::Scf(sfb_last as usize);
            scf_next = ScfPtr::Act;
            scf_min = scf[sfb_last as usize];
            scf_max = scf[sfb_last as usize];
        }
        if sfb_act >= 0 {
            scf_min = fix_max(scf_min, min_scf[sfb_act as usize]);
        }

        if (sfb_act >= 0)
            && (sfb_last >= 0 || sfb_next < sfb_cnt)
            && (scf_act > scf_min)
            && (scf_act <= scf_min.wrapping_add(MAX_SCF_DELTA))
            && (scf_act >= scf_max.wrapping_sub(MAX_SCF_DELTA))
            && (scf_act
                <= fix_min(
                    scf_min,
                    fix_min(deref(scf_last, scf, scf_act), deref(scf_next, scf, scf_act)),
                )
                .wrapping_add(MAX_SCF_DELTA))
            && (deref(scf_last, scf, scf_act) != prev_scf_last[sfb_act as usize]
                || deref(scf_next, scf, scf_act) != prev_scf_next[sfb_act as usize]
                || delta_pe < delta_pe_last[sfb_act as usize])
        {
            let a = sfb_act as usize;
            // bigger than neighbouring scf found, try to use smaller scf
            success = 0;

            let sfb_width = ch.width(a);
            let sfb_offs = ch.offs(a);

            // estimate required bits for actual scf
            let en_ld_data = ch.sfb_energy_ld_data[a];

            // sfbConstPePart[sfbAct] = (float)log(6.75f*en/sfbFormFactor[sfbAct]) * LOG2_1;
            const_pe_part(&mut sfb_const_pe_part[a], en_ld_data, sfb_form_factor_ld_data[a]);

            let sfb_pe_old = fdk_aac_enc_calc_single_spec_pe(
                scf_act,
                sfb_const_pe_part[a],
                sfb_n_relevant_lines[a],
            )
            .wrapping_add(fdk_aac_enc_count_single_scf_bits(
                scf_act,
                deref(scf_last, scf, scf_act),
                deref(scf_next, scf, scf_act),
            ));

            let mut delta_pe_new = delta_pe;
            let mut update_min_scf_calculated = 1;

            loop {
                // estimate required bits for smaller scf
                scf_act = scf_act.wrapping_sub(1);
                // check only if the same check was not done before
                if scf_act < min_scf_calculated[a] && scf_act >= scf_max.wrapping_sub(MAX_SCF_DELTA)
                {
                    // estimate required bits for new scf
                    let sfb_pe_new = fdk_aac_enc_calc_single_spec_pe(
                        scf_act,
                        sfb_const_pe_part[a],
                        sfb_n_relevant_lines[a],
                    )
                    .wrapping_add(fdk_aac_enc_count_single_scf_bits(
                        scf_act,
                        deref(scf_last, scf, scf_act),
                        deref(scf_next, scf, scf_act),
                    ));

                    // use new scf if no increase in pe and quantization error is smaller
                    let delta_pe_tmp = delta_pe.wrapping_add(sfb_pe_new).wrapping_sub(sfb_pe_old);
                    // 0.0006103515625f = 10.0f/(2^(2*AS_PE_FAC_SHIFT))
                    if delta_pe_tmp < fl(0.0006103515625f32) {
                        // distortion of new scf
                        let sfb_dist_new = fdk_aac_enc_calc_sfb_dist(
                            ch.spec(a),
                            &mut quant_spec_tmp[sfb_offs..],
                            sfb_width,
                            scf_act,
                            ch.d_zone_quant_enable,
                        );

                        if sfb_dist_new < sfb_dist[a] {
                            // success, replace scf by new one
                            scf[a] = scf_act;
                            sfb_dist[a] = sfb_dist_new;

                            copy_quant(quant_spec, quant_spec_tmp, sfb_offs, sfb_width);

                            delta_pe_new = delta_pe_tmp;
                            success = 1;
                        }
                        // mark as already checked
                        if update_min_scf_calculated != 0 {
                            min_scf_calculated[a] = scf_act;
                        }
                    } else {
                        // from this scf value on not all new values have been checked
                        update_min_scf_calculated = 0;
                    }
                }
                if scf_act <= scf_min {
                    break;
                }
            }

            delta_pe = delta_pe_new;

            // save parameters to avoid multiple computations of the same sfb
            prev_scf_last[a] = deref(scf_last, scf, scf_act);
            prev_scf_next[a] = deref(scf_next, scf, scf_act);
            delta_pe_last[a] = delta_pe;
        }

        if success != 0 && restart_on_success != 0 {
            // start again at first sfb
            sfb_last = -1;
            sfb_act = -1;
            sfb_next = -1;
            scf_last = ScfPtr::Null;
            scf_next = ScfPtr::Null;
            scf_min = FDK_INT_MAX;
            scf_max = FDK_INT_MAX;
            success = 0;
        } else {
            // shift sfbs for next band
            sfb_last = sfb_act;
            sfb_act = sfb_next;
        }
        if sfb_next >= sfb_cnt {
            break;
        }
    }
}

/// `FDKaacEnc_assimilateMultipleScf`, on the list of relevant bands (the straight port is
/// [`fdk_aac_enc_assimilate_multiple_scf_straight`]).
///
/// Exactness: the function only replaces relevant scalefactors by `scfAct` (`scfMin <= scfAct <
/// scfMax`), so the relevant bands stay the same. Every test in the region searches and loops is
/// `scf[sfb] != FDK_INT_MIN && ...` (or skips `FDK_INT_MIN` bands), so they depend only on the
/// relevant bands: a region is a run `q0..q1` of positions in the list whose scalefactors are
/// above `scfAct` (read when the search reaches them, as in C), `startSfb = idx[q0]`, `stopSfb =
/// idx[q1]` or `sfbCnt`, and the next search starts at position `q1`. Without a further region
/// the straight loop ends (`stopSfb > sfbCnt`). The region work (the same calls in the same
/// order, including the quirks described at the straight port) visits the positions of the run.
/// With all values within `±SCF_FAST_LIMIT` (checked, else the straight port runs) nothing
/// wraps.
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_assimilate_multiple_scf(
    ch: &ChannelIn,
    quant_spec: &mut [i16],
    quant_spec_tmp: &mut [i16],
    scf: &mut [i32],
    min_scf: &[i32],
    sfb_dist: &mut [FixpDbl],
    sfb_const_pe_part: &mut [FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_n_relevant_lines: &[FixpDbl],
) {
    let sfb_cnt = ch.psy.sfb_cnt;
    let rel = match RelBands::new(scf, sfb_cnt) {
        Some(r) if r.n > 0 && scf_values_fast(&r, scf, min_scf, None) => r,
        _ => {
            return fdk_aac_enc_assimilate_multiple_scf_straight(
                ch,
                quant_spec,
                quant_spec_tmp,
                scf,
                min_scf,
                sfb_dist,
                sfb_const_pe_part,
                sfb_form_factor_ld_data,
                sfb_n_relevant_lines,
            )
        }
    };
    let n = rel.n;
    let idx = &rel.idx[..n];
    let mut scf_tmp = [0i32; MAX_GROUPED_SFB];
    let mut sfb_dist_new = [SFB_DIST_NEW_UNWRITTEN; MAX_GROUPED_SFB];
    let mut delta_pe: FixpDbl = fl(0.0);
    // scf and minScf per position
    let mut sv = [0i32; MAX_GROUPED_SFB];
    let mut mv = [0i32; MAX_GROUPED_SFB];
    for (p, &b) in idx.iter().enumerate() {
        sv[p] = scf[b as usize];
        mv[p] = min_scf[b as usize];
    }

    // calc min and max scalfactors
    let scf_min = sv[..n].iter().copied().fold(FDK_INT_MAX, fix_min);
    let scf_max = sv[..n].iter().copied().fold(FDK_INT_MIN, fix_max);

    if scf_max <= scf_min + MAX_SCF_DELTA {
        let mut scf_act = scf_max;
        loop {
            // try smaller scf
            scf_act -= 1;
            scf_tmp.copy_from_slice(&scf[..MAX_GROUPED_SFB]);
            let mut q = 0usize;
            loop {
                // search for region where all scfs are bigger than scfAct
                while q < n && sv[q] <= scf_act {
                    q += 1;
                }
                if q == n {
                    // startSfb == sfbCnt: no region, the search ends
                    break;
                }
                let q0 = q;
                q += 1;
                while q < n && sv[q] > scf_act {
                    q += 1;
                }
                let q1 = q;
                let start_sfb = idx[q0] as i32;
                let stop_sfb = if q1 < n { idx[q1] as i32 } else { sfb_cnt };

                // check if in all sfb of a valid region scfAct >= minScf[sfb]
                if mv[q0..q1].iter().any(|&m| scf_act < m) {
                    continue;
                }
                // region found: replace scfs in region by scfAct
                for &b in &idx[q0..q1] {
                    scf_tmp[b as usize] = scf_act;
                }

                // estimate change in bit demand for new scfs
                let delta_scf_bits = count_scf_bits_diff_rel(
                    scf,
                    &scf_tmp,
                    Some(&rel),
                    sfb_cnt,
                    start_sfb,
                    stop_sfb,
                );
                let delta_spec_pe = calc_spec_pe_diff_rel(
                    ch.psy_sfb_energy_ld_data,
                    scf,
                    &scf_tmp,
                    Some(&rel),
                    sfb_const_pe_part,
                    sfb_form_factor_ld_data,
                    sfb_n_relevant_lines,
                    start_sfb,
                    stop_sfb,
                );
                let delta_pe_new =
                    delta_pe.wrapping_add(delta_scf_bits).wrapping_add(delta_spec_pe);

                // new bit demand small enough ?
                // 0.0006103515625f = 10.0f/(2^(2*AS_PE_FAC_SHIFT))
                if delta_pe_new < fl(0.0006103515625f32) {
                    // quantize and calc sum of new distortion
                    let mut dist_old_sum: FixpDbl = fl(0.0);
                    let mut dist_new_sum: FixpDbl = fl(0.0);
                    for &b in &idx[q0..q1] {
                        let sfb = b as usize;
                        dist_old_sum = dist_old_sum
                            .wrapping_add(calc_inv_ld_data(sfb_dist[sfb]) >> DIST_FAC_SHIFT);
                        let sfb_offs = ch.offs(sfb);
                        sfb_dist_new[sfb] = fdk_aac_enc_calc_sfb_dist(
                            ch.spec(sfb),
                            &mut quant_spec_tmp[sfb_offs..],
                            ch.width(sfb),
                            scf_act,
                            ch.d_zone_quant_enable,
                        );
                        if sfb_dist_new[sfb] > ch.sfb_threshold_ld_data[sfb] {
                            // no improvement, skip further dist. calculations
                            dist_new_sum = dist_old_sum.wrapping_shl(1);
                            break;
                        }
                        dist_new_sum = dist_new_sum
                            .wrapping_add(calc_inv_ld_data(sfb_dist_new[sfb]) >> DIST_FAC_SHIFT);
                    }
                    // distortion smaller ? -> use new scalefactors
                    if dist_new_sum < dist_old_sum {
                        delta_pe = delta_pe_new;
                        for (p, &b) in idx.iter().enumerate().take(q1).skip(q0) {
                            let sfb = b as usize;
                            scf[sfb] = scf_act;
                            sv[p] = scf_act;
                            fdk_assert!(sfb_dist_new[sfb] != SFB_DIST_NEW_UNWRITTEN);
                            sfb_dist[sfb] = sfb_dist_new[sfb];
                            copy_quant(quant_spec, quant_spec_tmp, ch.offs(sfb), ch.width(sfb));
                        }
                    }
                }
            }
            if scf_act <= scf_min {
                break;
            }
        }
    }
}

/// `FDKaacEnc_assimilateMultipleScf` (straight port, kept as the reference of
/// [`fdk_aac_enc_assimilate_multiple_scf`])
///
/// `distNewSum = distOldSum << 1` (no improvement) wraps as in C; values of earlier passes in
/// `sfbDistNew` and `quantSpecTmp` are reproduced (see the module documentation).
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_assimilate_multiple_scf_straight(
    ch: &ChannelIn,
    quant_spec: &mut [i16],
    quant_spec_tmp: &mut [i16],
    scf: &mut [i32],
    min_scf: &[i32],
    sfb_dist: &mut [FixpDbl],
    sfb_const_pe_part: &mut [FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_n_relevant_lines: &[FixpDbl],
) {
    let mut scf_tmp = [0i32; MAX_GROUPED_SFB];
    let mut sfb_dist_new = [SFB_DIST_NEW_UNWRITTEN; MAX_GROUPED_SFB];
    let mut delta_pe: FixpDbl = fl(0.0);
    let sfb_cnt = ch.psy.sfb_cnt;
    let rel = RelBands::new(scf, sfb_cnt);
    let rel = rel.as_ref();

    // calc min and max scalfactors
    let mut scf_min = FDK_INT_MAX;
    let mut scf_max = FDK_INT_MIN;
    for sfb in 0..sfb_cnt.max(0) as usize {
        if scf[sfb] != FDK_INT_MIN {
            scf_min = fix_min(scf_min, scf[sfb]);
            scf_max = fix_max(scf_max, scf[sfb]);
        }
    }

    if scf_max != FDK_INT_MIN && scf_max <= scf_min.wrapping_add(MAX_SCF_DELTA) {
        let mut scf_act = scf_max;

        loop {
            // try smaller scf
            scf_act -= 1;
            scf_tmp.copy_from_slice(&scf[..MAX_GROUPED_SFB]);
            let mut stop_sfb = 0;
            loop {
                // search for region where all scfs are bigger than scfAct
                let mut sfb = stop_sfb;
                while sfb < sfb_cnt
                    && (scf[sfb as usize] == FDK_INT_MIN || scf[sfb as usize] <= scf_act)
                {
                    sfb += 1;
                }
                let start_sfb = sfb;
                sfb += 1;
                while sfb < sfb_cnt
                    && (scf[sfb as usize] == FDK_INT_MIN || scf[sfb as usize] > scf_act)
                {
                    sfb += 1;
                }
                stop_sfb = sfb;

                // check if in all sfb of a valid region scfAct >= minScf[sfb]
                let mut possible_region_found = 0;
                if start_sfb < sfb_cnt {
                    possible_region_found = 1;
                    for sfb in start_sfb..stop_sfb {
                        if scf[sfb as usize] != FDK_INT_MIN && scf_act < min_scf[sfb as usize] {
                            possible_region_found = 0;
                            break;
                        }
                    }
                }

                if possible_region_found != 0 {
                    // region found
                    let region = start_sfb as usize..stop_sfb as usize;

                    // replace scfs in region by scfAct
                    for sfb in region.clone() {
                        if scf_tmp[sfb] != FDK_INT_MIN {
                            scf_tmp[sfb] = scf_act;
                        }
                    }

                    // estimate change in bit demand for new scfs
                    let delta_scf_bits =
                        count_scf_bits_diff_rel(scf, &scf_tmp, rel, sfb_cnt, start_sfb, stop_sfb);

                    let delta_spec_pe = calc_spec_pe_diff_rel(
                        ch.psy_sfb_energy_ld_data,
                        scf,
                        &scf_tmp,
                        rel,
                        sfb_const_pe_part,
                        sfb_form_factor_ld_data,
                        sfb_n_relevant_lines,
                        start_sfb,
                        stop_sfb,
                    );

                    let delta_pe_new =
                        delta_pe.wrapping_add(delta_scf_bits).wrapping_add(delta_spec_pe);

                    // new bit demand small enough ?
                    // 0.0006103515625f = 10.0f/(2^(2*AS_PE_FAC_SHIFT))
                    if delta_pe_new < fl(0.0006103515625f32) {
                        // quantize and calc sum of new distortion
                        let mut dist_old_sum: FixpDbl = fl(0.0);
                        let mut dist_new_sum: FixpDbl = fl(0.0);
                        for sfb in region.clone() {
                            if scf_tmp[sfb] != FDK_INT_MIN {
                                dist_old_sum = dist_old_sum.wrapping_add(
                                    calc_inv_ld_data(sfb_dist[sfb]) >> DIST_FAC_SHIFT,
                                );

                                let sfb_width = ch.width(sfb);
                                let sfb_offs = ch.offs(sfb);

                                sfb_dist_new[sfb] = fdk_aac_enc_calc_sfb_dist(
                                    ch.spec(sfb),
                                    &mut quant_spec_tmp[sfb_offs..],
                                    sfb_width,
                                    scf_act,
                                    ch.d_zone_quant_enable,
                                );

                                if sfb_dist_new[sfb] > ch.sfb_threshold_ld_data[sfb] {
                                    // no improvement, skip further dist. calculations
                                    dist_new_sum = dist_old_sum.wrapping_shl(1);
                                    break;
                                }
                                dist_new_sum = dist_new_sum.wrapping_add(
                                    calc_inv_ld_data(sfb_dist_new[sfb]) >> DIST_FAC_SHIFT,
                                );
                            }
                        }
                        // distortion smaller ? -> use new scalefactors
                        if dist_new_sum < dist_old_sum {
                            delta_pe = delta_pe_new;
                            for sfb in region.clone() {
                                if scf[sfb] != FDK_INT_MIN {
                                    let sfb_width = ch.width(sfb);
                                    let sfb_offs = ch.offs(sfb);
                                    scf[sfb] = scf_act;
                                    fdk_assert!(sfb_dist_new[sfb] != SFB_DIST_NEW_UNWRITTEN);
                                    sfb_dist[sfb] = sfb_dist_new[sfb];

                                    copy_quant(quant_spec, quant_spec_tmp, sfb_offs, sfb_width);
                                }
                            }
                        }
                    }
                }

                if stop_sfb > sfb_cnt {
                    break;
                }
            }

            if scf_act <= scf_min {
                break;
            }
        }
    }
}

/// `FDKaacEnc_FDKaacEnc_assimilateMultipleScf2`
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_fdk_aac_enc_assimilate_multiple_scf2(
    ch: &ChannelIn,
    quant_spec: &mut [i16],
    quant_spec_tmp: &mut [i16],
    scf: &mut [i32],
    min_scf: &[i32],
    sfb_dist: &mut [FixpDbl],
    sfb_const_pe_part: &mut [FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_n_relevant_lines: &[FixpDbl],
) {
    let mut scf_tmp = [0i32; MAX_GROUPED_SFB];
    let sfb_offs = &ch.psy.sfb_offsets;
    let mut sfb_dist_new = [SFB_DIST_NEW_UNWRITTEN; MAX_GROUPED_SFB];
    let mut sfb_dist_max = [0 as FixpDbl; MAX_GROUPED_SFB];
    let mut delta_pe: FixpDbl = fl(0.0);
    let mut delta_pe_new: FixpDbl = fl(0.0);
    let sfb_cnt = ch.psy.sfb_cnt;
    let rel = RelBands::new(scf, sfb_cnt);
    let rel = rel.as_ref();
    let offs = |sfb: usize| sfb_offs[sfb] as usize;
    let width = |sfb: usize| sfb_offs[sfb + 1] - sfb_offs[sfb];

    // calc min and max scalfactors
    let mut scf_min = FDK_INT_MAX;
    let mut scf_max = FDK_INT_MIN;
    for sfb in 0..sfb_cnt.max(0) as usize {
        if scf[sfb] != FDK_INT_MIN {
            scf_min = fix_min(scf_min, scf[sfb]);
            scf_max = fix_max(scf_max, scf[sfb]);
        }
    }

    let mut stop_sfb: i32 = 0;
    let mut scf_act: i32 = FDK_INT_MIN;
    loop {
        // search for region with same scf values scfAct
        let mut scf_prev = scf_act;

        let mut sfb = stop_sfb;
        while sfb < sfb_cnt && (scf[sfb as usize] == FDK_INT_MIN) {
            sfb += 1;
        }
        let start_sfb = sfb;
        // C: scfAct = scf[startSfb]; past the bands the value is not used (the loop ends
        // without a region).
        scf_act = scf.get(start_sfb as usize).copied().unwrap_or(0);
        sfb += 1;
        while sfb < sfb_cnt
            && ((scf[sfb as usize] == FDK_INT_MIN)
                || (scf[sfb as usize] == scf[start_sfb as usize]))
        {
            sfb += 1;
        }
        stop_sfb = sfb;

        let scf_next = if stop_sfb < sfb_cnt { scf[stop_sfb as usize] } else { scf_act };

        if scf_prev == FDK_INT_MIN {
            scf_prev = scf_act;
        }

        let scf_prev_next_max = fix_max(scf_prev, scf_next);
        let scf_prev_next_min = fix_min(scf_prev, scf_next);

        // try to reduce bits by checking scf values in the range scf[startSfb]...scfHi
        let scf_hi = fix_max(scf_prev_next_max, scf_act);
        // try to find a better solution by reducing the scf difference to the nearest possible
        // lower scf
        let scf_lo = if scf_prev_next_max >= scf_act {
            fix_min(scf_act, scf_prev_next_min)
        } else {
            scf_prev_next_max
        };

        if start_sfb < sfb_cnt && scf_hi.wrapping_sub(scf_lo) <= MAX_SCF_DELTA {
            // region found
            let region = start_sfb as usize..stop_sfb as usize;
            let s0 = start_sfb as usize;

            // 1. try to save bits by coarser quantization
            if scf_hi > scf[s0] {
                // calculate the allowed distortion
                for sfb in region.clone() {
                    if scf[sfb] != FDK_INT_MIN {
                        // sfbDistMax[sfb] =
                        // (float)pow(qcOutChannel->sfbThreshold[sfb]*sfbDist[sfb]*sfbDist[sfb],1.0f/3.0f);
                        // sfbDistMax[sfb] =
                        // fixMax(sfbDistMax[sfb],qcOutChannel->sfbEnergy[sfb]*FL2FXCONST_DBL(1.e-3f));
                        // -0.15571537944 = ld64(1.e-3f)
                        sfb_dist_max[sfb] =
                            f_mult(fl(1.0f32 / 3.0f32), ch.sfb_threshold_ld_data[sfb])
                                .wrapping_add(f_mult(fl(1.0f32 / 3.0f32), sfb_dist[sfb]))
                                .wrapping_add(f_mult(fl(1.0f32 / 3.0f32), sfb_dist[sfb]));
                        sfb_dist_max[sfb] = fix_max(
                            sfb_dist_max[sfb],
                            ch.sfb_energy_ld_data[sfb].wrapping_sub(fl2fxconst_dbl(0.15571537944)),
                        );
                        sfb_dist_max[sfb] =
                            fix_min(sfb_dist_max[sfb], ch.sfb_threshold_ld_data[sfb]);
                    }
                }

                // loop over all possible scf values for this region
                let mut b_check_scf = 1;
                let mut scf_new = scf[s0].wrapping_add(1);
                while scf_new <= scf_hi {
                    scf_tmp.copy_from_slice(&scf[..MAX_GROUPED_SFB]);

                    // replace scfs in region by scfNew
                    for sfb in region.clone() {
                        if scf_tmp[sfb] != FDK_INT_MIN {
                            scf_tmp[sfb] = scf_new;
                        }
                    }

                    // estimate change in bit demand for new scfs
                    let delta_scf_bits =
                        count_scf_bits_diff_rel(scf, &scf_tmp, rel, sfb_cnt, start_sfb, stop_sfb);

                    let delta_spec_pe = calc_spec_pe_diff_rel(
                        ch.psy_sfb_energy_ld_data,
                        scf,
                        &scf_tmp,
                        rel,
                        sfb_const_pe_part,
                        sfb_form_factor_ld_data,
                        sfb_n_relevant_lines,
                        start_sfb,
                        stop_sfb,
                    );

                    delta_pe_new =
                        delta_pe.wrapping_add(delta_scf_bits).wrapping_add(delta_spec_pe);

                    // new bit demand small enough ?
                    if delta_pe_new < fl(0.0) {
                        let mut b_success = 1;

                        // quantize and calc sum of new distortion
                        for sfb in region.clone() {
                            if scf_tmp[sfb] != FDK_INT_MIN {
                                sfb_dist_new[sfb] = fdk_aac_enc_calc_sfb_dist(
                                    &ch.mdct_spectrum[offs(sfb)..],
                                    &mut quant_spec_tmp[offs(sfb)..],
                                    width(sfb),
                                    scf_new,
                                    ch.d_zone_quant_enable,
                                );

                                if sfb_dist_new[sfb] > sfb_dist_max[sfb] {
                                    // no improvement, skip further dist. calculations
                                    b_success = 0;
                                    if sfb_dist_new[sfb] == ch.sfb_energy_ld_data[sfb] {
                                        // if whole sfb is already quantized to 0, further checks
                                        // with even coarser quant. are useless
                                        b_check_scf = 0;
                                    }
                                    break;
                                }
                            }
                        }
                        if b_check_scf == 0 {
                            // further calculations useless ?
                            break;
                        }
                        // distortion small enough ? -> use new scalefactors
                        if b_success != 0 {
                            delta_pe = delta_pe_new;
                            for sfb in region.clone() {
                                if scf[sfb] != FDK_INT_MIN {
                                    scf[sfb] = scf_new;
                                    sfb_dist[sfb] = sfb_dist_new[sfb];

                                    copy_quant(quant_spec, quant_spec_tmp, offs(sfb), width(sfb));
                                }
                            }
                        }
                    }
                    scf_new += 1;
                }
            }

            // 2. only if coarser quantization was not successful, try to find a better solution
            // by finer quantization and reducing bits for scalefactor coding
            if scf_act == scf[s0]
                && scf_lo < scf_act
                && scf_max.wrapping_sub(scf_min) <= MAX_SCF_DELTA
            {
                let mut bmin_scf_violation = 0;

                scf_tmp.copy_from_slice(&scf[..MAX_GROUPED_SFB]);

                let scf_new = scf_lo;

                // replace scfs in region by scfNew and check if in all sfb scfNew >= minScf[sfb]
                for sfb in region.clone() {
                    if scf_tmp[sfb] != FDK_INT_MIN {
                        scf_tmp[sfb] = scf_new;
                        if scf_new < min_scf[sfb] {
                            bmin_scf_violation = 1;
                        }
                    }
                }

                if bmin_scf_violation == 0 {
                    // estimate change in bit demand for new scfs
                    let delta_scf_bits =
                        count_scf_bits_diff_rel(scf, &scf_tmp, rel, sfb_cnt, start_sfb, stop_sfb);

                    let delta_spec_pe = calc_spec_pe_diff_rel(
                        ch.psy_sfb_energy_ld_data,
                        scf,
                        &scf_tmp,
                        rel,
                        sfb_const_pe_part,
                        sfb_form_factor_ld_data,
                        sfb_n_relevant_lines,
                        start_sfb,
                        stop_sfb,
                    );

                    delta_pe_new =
                        delta_pe.wrapping_add(delta_scf_bits).wrapping_add(delta_spec_pe);
                }

                // new bit demand small enough ?
                if bmin_scf_violation == 0 && delta_pe_new < fl(0.0) {
                    // quantize and calc sum of new distortion
                    let mut dist_old_sum: FixpDbl = fl(0.0);
                    let mut dist_new_sum: FixpDbl = fl(0.0);
                    for sfb in region.clone() {
                        if scf_tmp[sfb] != FDK_INT_MIN {
                            dist_old_sum = dist_old_sum
                                .wrapping_add(calc_inv_ld_data(sfb_dist[sfb]) >> DIST_FAC_SHIFT);

                            sfb_dist_new[sfb] = fdk_aac_enc_calc_sfb_dist(
                                &ch.mdct_spectrum[offs(sfb)..],
                                &mut quant_spec_tmp[offs(sfb)..],
                                width(sfb),
                                scf_new,
                                ch.d_zone_quant_enable,
                            );

                            if sfb_dist_new[sfb] > ch.sfb_threshold_ld_data[sfb] {
                                // no improvement, skip further dist. calculations
                                dist_new_sum = dist_old_sum.wrapping_shl(1);
                                break;
                            }
                            dist_new_sum = dist_new_sum.wrapping_add(
                                calc_inv_ld_data(sfb_dist_new[sfb]) >> DIST_FAC_SHIFT,
                            );
                        }
                    }
                    // distortion smaller ? -> use new scalefactors
                    if dist_new_sum < f_mult(fl(0.8f32), dist_old_sum) {
                        delta_pe = delta_pe_new;
                        for sfb in region.clone() {
                            if scf[sfb] != FDK_INT_MIN {
                                scf[sfb] = scf_new;
                                fdk_assert!(sfb_dist_new[sfb] != SFB_DIST_NEW_UNWRITTEN);
                                sfb_dist[sfb] = sfb_dist_new[sfb];

                                copy_quant(quant_spec, quant_spec_tmp, offs(sfb), width(sfb));
                            }
                        }
                    }
                }
            }

            // 3. try to find a better solution (save bits) by only reducing the scalefactor
            // without new quantization
            if scf_max.wrapping_sub(scf_min) <= MAX_SCF_DELTA - 3 {
                // 3 bec. scf is reduced 3 times, see for loop below

                scf_tmp[..sfb_cnt.max(0) as usize].copy_from_slice(&scf[..sfb_cnt.max(0) as usize]);

                for _i in 0..3 {
                    let scf_new = scf_tmp[s0].wrapping_sub(1);
                    // replace scfs in region by scfNew
                    for sfb in region.clone() {
                        if scf_tmp[sfb] != FDK_INT_MIN {
                            scf_tmp[sfb] = scf_new;
                        }
                    }
                    // estimate change in bit demand for new scfs
                    let delta_scf_bits =
                        count_scf_bits_diff_rel(scf, &scf_tmp, rel, sfb_cnt, start_sfb, stop_sfb);
                    delta_pe_new = delta_pe.wrapping_add(delta_scf_bits);
                    // new bit demand small enough ?
                    if delta_pe_new <= fl(0.0) {
                        let mut b_success = 1;
                        let mut dist_old_sum: FixpDbl = fl(0.0);
                        let mut dist_new_sum: FixpDbl = fl(0.0);
                        for sfb in region.clone() {
                            if scf_tmp[sfb] != FDK_INT_MIN {
                                let mut sfb_en_q: FixpDbl = 0;
                                // calc the energy and distortion of the quantized spectrum for a
                                // smaller scf
                                fdk_aac_enc_calc_sfb_quant_energy_and_dist(
                                    &ch.mdct_spectrum[offs(sfb)..],
                                    &quant_spec[offs(sfb)..],
                                    width(sfb),
                                    scf_new,
                                    &mut sfb_en_q,
                                    &mut sfb_dist_new[sfb],
                                );

                                dist_old_sum = dist_old_sum.wrapping_add(
                                    calc_inv_ld_data(sfb_dist[sfb]) >> DIST_FAC_SHIFT,
                                );
                                dist_new_sum = dist_new_sum.wrapping_add(
                                    calc_inv_ld_data(sfb_dist_new[sfb]) >> DIST_FAC_SHIFT,
                                );

                                //  0.00259488556167 = ld64(1.122f)
                                // -0.00778722686652 = ld64(0.7079f)
                                if (sfb_dist_new[sfb]
                                    > sfb_dist[sfb].wrapping_add(fl(0.00259488556167f32)))
                                    || (sfb_en_q
                                        < ch.sfb_energy_ld_data[sfb]
                                            .wrapping_sub(fl(0.00778722686652f32)))
                                {
                                    b_success = 0;
                                    break;
                                }
                            }
                        }
                        // distortion smaller ? -> use new scalefactors
                        if dist_new_sum < dist_old_sum && b_success != 0 {
                            delta_pe = delta_pe_new;
                            for sfb in region.clone() {
                                if scf[sfb] != FDK_INT_MIN {
                                    scf[sfb] = scf_new;
                                    sfb_dist[sfb] = sfb_dist_new[sfb];
                                }
                            }
                        }
                    }
                }
            }
        }

        if stop_sfb > sfb_cnt {
            break;
        }
    }
}

/// `mdctSpectrum[j]` as the maximum search reads it: past the 1024 lines, the following
/// `quantSpec` of `QC_OUT_CHANNEL` (little endian), reproduced from the C layout.
#[inline]
fn spec_at(mdct_spectrum: &[FixpDbl; 1024], quant_spec: &[i16; 1024], j: usize) -> FixpDbl {
    if j < 1024 {
        mdct_spectrum[j]
    } else {
        let k = 2 * (j - 1024);
        let lo = quant_spec.get(k).copied().unwrap_or(0) as u16 as u32;
        let hi = quant_spec.get(k + 1).copied().unwrap_or(0) as u16 as u32;
        (lo | (hi << 16)) as i32
    }
}

/// `FDKaacEnc_EstimateScaleFactorsChannel`
///
/// C passes `qcOutChannel->scf`, `&qcOutChannel->globalGain`,
/// `qcOutChannel->sfbFormFactorLdData` and `qcOutChannel->quantSpec` as separate pointers; the
/// port uses the fields of `qc_out_channel`.
pub(crate) fn fdk_aac_enc_estimate_scale_factors_channel(
    qc_out_channel: &mut QcOutChannel,
    psy_out_channel: &PsyOutChannel,
    inv_quant: i32,
    d_zone_quant_enable: i32,
) {
    fdk_aac_enc_estimate_scale_factors_channel_view(
        qc_out_channel,
        psy_out_channel,
        None,
        inv_quant,
        d_zone_quant_enable,
    )
}

/// [`fdk_aac_enc_estimate_scale_factors_channel`] with the arrays C reads through the
/// `PSY_OUT_CHANNEL` (`sfbEnergyLdData` in `FDKaacEnc_calcSpecPeDiff()`) from `psy_view` if set
/// (see [`PsyArrayViews`](crate::adj_thr::PsyArrayViews)).
pub(crate) fn fdk_aac_enc_estimate_scale_factors_channel_view(
    qc_out_channel: &mut QcOutChannel,
    psy_out_channel: &PsyOutChannel,
    psy_view: Option<&QcOutChannel>,
    inv_quant: i32,
    d_zone_quant_enable: i32,
) {
    estimate_scale_factors_channel_with(
        qc_out_channel,
        psy_out_channel,
        psy_view,
        inv_quant,
        d_zone_quant_enable,
        false,
    )
}

/// [`fdk_aac_enc_estimate_scale_factors_channel_view`] with the optimized assimilation functions
/// or (`straight`, for tests) their straight ports.
fn estimate_scale_factors_channel_with(
    qc_out_channel: &mut QcOutChannel,
    psy_out_channel: &PsyOutChannel,
    psy_view: Option<&QcOutChannel>,
    inv_quant: i32,
    d_zone_quant_enable: i32,
    straight: bool,
) {
    let QcOutChannel {
        mdct_spectrum,
        quant_spec,
        scf,
        global_gain,
        sfb_form_factor_ld_data,
        sfb_threshold_ld_data,
        sfb_energy_ld_data,
        ..
    } = qc_out_channel;
    let psy = psy_out_channel;
    let sfb_cnt = psy.sfb_cnt;
    let sfb_per_group = psy.sfb_per_group;
    let max_sfb_per_group = psy.max_sfb_per_group;

    // uninitialized in C; read only where written (see the module documentation)
    let mut min_scf_calculated = [0i32; MAX_GROUPED_SFB];
    let mut sfb_dist_ld_data = [0 as FixpDbl; MAX_GROUPED_SFB];
    // C_ALLOC_SCRATCH_START(quantSpecTmp, SHORT, (1024))
    let mut quant_spec_tmp = [QUANT_SPEC_TMP_UNWRITTEN; 1024];
    let mut min_sf_max_quant = [0i32; MAX_GROUPED_SFB];

    let thresh_const_ld_data: FixpDbl = fl(0.04304511722f32); // log10(6.75)/log10(2.0)/64.0
    let conv_const: FixpDbl = fl(0.30102999566f32); // log10(2.0)
    let c1_const: FixpDbl = fl(-0.27083183594f32); // C1 = -69.33295 => C1/2^8

    if inv_quant > 0 {
        quant_spec.fill(0);
    }

    // scfs without energy or with thresh>energy are marked with FDK_INT_MIN
    for x in scf[..sfb_cnt.max(0) as usize].iter_mut() {
        *x = FDK_INT_MIN;
    }

    min_sf_max_quant.fill(FDK_INT_MIN);

    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let i = (sfb_offs + sfb) as usize;
            let thresh_ld_data = sfb_threshold_ld_data[i];
            let energy_ld_data = sfb_energy_ld_data[i];

            sfb_dist_ld_data[i] = energy_ld_data;

            if energy_ld_data > thresh_ld_data {
                // energyPart = (float)log10(sfbFormFactor[sfbOffs+sfb]);
                // 0.09375f = log(64.0)/log(2.0)/64.0 = scale of sfbFormFactorLdData
                let energy_part_ld_data = sfb_form_factor_ld_data[i].wrapping_add(fl(0.09375f32));

                // influence of allowed distortion
                // thresholdPart = (float)log10(6.75*thresh+FLT_MIN);
                let threshold_part_ld_data = thresh_const_ld_data.wrapping_add(thresh_ld_data);

                // scf calc
                // scfFloat = 8.8585f * (thresholdPart - energyPart);
                let mut scf_fract = threshold_part_ld_data.wrapping_sub(energy_part_ld_data);
                // conversion from log2 to log10
                scf_fract = f_mult(conv_const, scf_fract);
                // (8.8585f * scfFract)/8 = 8/8 * scfFract + 0.8585 * scfFract/8
                scf_fract = scf_fract.wrapping_add(f_mult(fl(0.8585f32), scf_fract >> 3));

                // integer scalefactor
                // scfInt = (int)floor(scfFloat);
                // 3 bits => scfFract/8.0; 6 bits => ld64
                let mut scf_int = scf_fract >> ((DFRACT_BITS - 1) - 3 - LD_DATA_SHIFT);

                // maximum of spectrum
                let mut max_spec: FixpDbl = fl(0.0);

                // Unroll by 4, allow dual memory access
                // C: maxSpec = fMax(maxSpec, fMax(fMax(fAbs(s0), fAbs(s1)),
                //                               fMax(fAbs(s2), fAbs(s3))));
                // Both builds assume fAbs() >= 0 and take the inner maximum unsigned (umax /
                // pmaxud) and the outer one signed (smax / pmaxsd): a group of 4 lines containing
                // MINVAL_DBL (whose fAbs() is MINVAL_DBL) has the maximum 0x80000000, which the
                // outer signed maximum ignores.
                let mut j = psy.sfb_offsets[i];
                while j < psy.sfb_offsets[i + 1] {
                    let s = |k: i32| f_abs(spec_at(mdct_spectrum, quant_spec, k as usize)) as u32;
                    let group = s(j).max(s(j + 1)).max(s(j + 2).max(s(j + 3)));
                    max_spec = f_max(max_spec, group as FixpDbl);
                    j += 4;
                }
                // lower scf limit to avoid quantized values bigger than MAX_QUANT
                // C1 = -69.33295f, C2 = 5.77078f = 4/log(2)
                // minSfMaxQuant[sfbOffs+sfb] = (int)ceil(C1 + C2*log(maxSpec));
                // C1/2^8 + 4/log(2.0)*log(maxSpec)/2^8  => C1/2^8 + log(maxSpec)/log(2.0)*4/2^8
                // => C1/2^8 + log(maxSpec)/log(2.0)/64.0

                // minSfMaxQuant[sfbOffs+sfb] = ((INT) ((c1Const + CalcLdData(maxSpec)) >>
                // ((DFRACT_BITS-1)-8))) + 1;
                let tmp = calc_ld_data(max_spec);
                if c1_const > fl(-1.0f32).wrapping_sub(tmp) {
                    min_sf_max_quant[i] =
                        (c1_const.wrapping_add(tmp) >> ((DFRACT_BITS - 1) - 8)) + 1;
                } else {
                    min_sf_max_quant[i] = (fl(-1.0f32) >> ((DFRACT_BITS - 1) - 8)) + 1;
                }

                scf_int = fix_max(scf_int, min_sf_max_quant[i]);

                // find better scalefactor with analysis by synthesis
                if inv_quant > 0 {
                    let o = psy.sfb_offsets[i] as usize;
                    scf_int = fdk_aac_enc_improve_scf(
                        &mdct_spectrum[o..],
                        &mut quant_spec[o..],
                        &mut quant_spec_tmp[o..],
                        psy.sfb_offsets[i + 1] - psy.sfb_offsets[i],
                        thresh_ld_data,
                        scf_int,
                        min_sf_max_quant[i],
                        &mut sfb_dist_ld_data[i],
                        &mut min_scf_calculated[i],
                        d_zone_quant_enable,
                    );
                }
                scf[i] = scf_int;
            }
        }
        sfb_offs += sfb_per_group;
    }

    if inv_quant > 0 {
        // try to decrease scf differences
        let mut sfb_const_pe_part = [0 as FixpDbl; MAX_GROUPED_SFB];
        let mut sfb_n_relevant_lines = [0 as FixpDbl; MAX_GROUPED_SFB];

        for x in sfb_const_pe_part[..sfb_cnt.max(0) as usize].iter_mut() {
            *x = FDK_INT_MIN;
        }

        fdk_aac_enc_calc_sfb_relevant_lines(
            sfb_form_factor_ld_data,
            sfb_energy_ld_data,
            sfb_threshold_ld_data,
            &psy.sfb_offsets,
            sfb_cnt,
            sfb_per_group,
            max_sfb_per_group,
            &mut sfb_n_relevant_lines,
        );

        let ch = ChannelIn {
            psy,
            mdct_spectrum: &mdct_spectrum[..],
            sfb_energy_ld_data: &sfb_energy_ld_data[..],
            psy_sfb_energy_ld_data: psy_view
                .map_or(&sfb_energy_ld_data[..], |v| &v.sfb_energy_ld_data[..]),
            sfb_threshold_ld_data: &sfb_threshold_ld_data[..],
            d_zone_quant_enable,
        };

        let assimilate_single_scf = if straight {
            fdk_aac_enc_assimilate_single_scf_straight
        } else {
            fdk_aac_enc_assimilate_single_scf
        };
        assimilate_single_scf(
            &ch,
            quant_spec,
            &mut quant_spec_tmp,
            scf,
            &min_sf_max_quant,
            &mut sfb_dist_ld_data,
            &mut sfb_const_pe_part,
            sfb_form_factor_ld_data,
            &sfb_n_relevant_lines,
            &mut min_scf_calculated,
            1,
        );

        if inv_quant > 1 {
            let assimilate_multiple_scf = if straight {
                fdk_aac_enc_assimilate_multiple_scf_straight
            } else {
                fdk_aac_enc_assimilate_multiple_scf
            };
            assimilate_multiple_scf(
                &ch,
                quant_spec,
                &mut quant_spec_tmp,
                scf,
                &min_sf_max_quant,
                &mut sfb_dist_ld_data,
                &mut sfb_const_pe_part,
                sfb_form_factor_ld_data,
                &sfb_n_relevant_lines,
            );

            fdk_aac_enc_fdk_aac_enc_assimilate_multiple_scf2(
                &ch,
                quant_spec,
                &mut quant_spec_tmp,
                scf,
                &min_sf_max_quant,
                &mut sfb_dist_ld_data,
                &mut sfb_const_pe_part,
                sfb_form_factor_ld_data,
                &sfb_n_relevant_lines,
            );
        }
    }

    // get min scalefac
    let mut min_sf = FDK_INT_MAX;
    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let i = (sfb_offs + sfb) as usize;
            if scf[i] != FDK_INT_MIN {
                min_sf = fix_min(min_sf, scf[i]);
            }
        }
        sfb_offs += sfb_per_group;
    }

    // limit scf delta
    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let i = (sfb_offs + sfb) as usize;
            if (scf[i] != FDK_INT_MIN) && min_sf.wrapping_add(MAX_SCF_DELTA) < scf[i] {
                scf[i] = min_sf + MAX_SCF_DELTA;
                if inv_quant > 0 {
                    // changed bands need to be quantized again
                    let o = psy.sfb_offsets[i] as usize;
                    sfb_dist_ld_data[i] = fdk_aac_enc_calc_sfb_dist(
                        &mdct_spectrum[o..],
                        &mut quant_spec[o..],
                        psy.sfb_offsets[i + 1] - psy.sfb_offsets[i],
                        scf[i],
                        d_zone_quant_enable,
                    );
                }
            }
        }
        sfb_offs += sfb_per_group;
    }

    // get max scalefac for global gain
    let mut max_sf = FDK_INT_MIN;
    let mut sfb_offs = 0;
    while sfb_offs < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            max_sf = fix_max(max_sf, scf[(sfb_offs + sfb) as usize]);
        }
        sfb_offs += sfb_per_group;
    }

    // calc loop scalefactors, if spec is not all zero (i.e. maxSf == -99)
    if max_sf > FDK_INT_MIN {
        *global_gain = max_sf;
        let mut sfb_offs = 0;
        while sfb_offs < sfb_cnt {
            for sfb in 0..max_sfb_per_group {
                let i = (sfb_offs + sfb) as usize;
                if scf[i] == FDK_INT_MIN {
                    scf[i] = 0;
                    // set band explicitely to zero
                    for j in psy.sfb_offsets[i]..psy.sfb_offsets[i + 1] {
                        mdct_spectrum[j as usize] = fl(0.0);
                    }
                } else {
                    scf[i] = max_sf.wrapping_sub(scf[i]);
                }
            }
            sfb_offs += sfb_per_group;
        }
    } else {
        *global_gain = 0;
        // set spectrum explicitely to zero
        let mut sfb_offs = 0;
        while sfb_offs < sfb_cnt {
            for sfb in 0..max_sfb_per_group {
                let i = (sfb_offs + sfb) as usize;
                scf[i] = 0;
                // set band explicitely to zero
                for j in psy.sfb_offsets[i]..psy.sfb_offsets[i + 1] {
                    mdct_spectrum[j as usize] = fl(0.0);
                }
            }
            sfb_offs += sfb_per_group;
        }
    }

    // free quantSpecTmp from scratch
}

/// `FDKaacEnc_EstimateScaleFactors`: estimates the scalefactors, the global gain and (for
/// `invQuant > 0`) the quantized spectrum of `n_channels` channels.
///
/// `psy_out_channel[ch]` and `qc_out_channel[ch]` are the channels of the element (the
/// `psyOutChannel[]`/`qcOutChannel[]` pointer arrays of C).
pub fn fdk_aac_enc_estimate_scale_factors(
    psy_out_channel: &[PsyOutChannel],
    qc_out_channel: &mut [QcOutChannel],
    inv_quant: i32,
    d_zone_quant_enable: i32,
    n_channels: i32,
) {
    fdk_aac_enc_estimate_scale_factors_views(
        psy_out_channel,
        qc_out_channel,
        &crate::adj_thr::NO_PSY_ARRAY_VIEWS,
        inv_quant,
        d_zone_quant_enable,
        n_channels,
    )
}

/// [`fdk_aac_enc_estimate_scale_factors`] with the arrays C reads through the `PSY_OUT_CHANNEL`
/// from `views` where set (see [`PsyArrayViews`](crate::adj_thr::PsyArrayViews)).
pub fn fdk_aac_enc_estimate_scale_factors_views(
    psy_out_channel: &[PsyOutChannel],
    qc_out_channel: &mut [QcOutChannel],
    views: &crate::adj_thr::PsyArrayViews,
    inv_quant: i32,
    d_zone_quant_enable: i32,
    n_channels: i32,
) {
    for ch in 0..n_channels.max(0) as usize {
        fdk_aac_enc_estimate_scale_factors_channel_view(
            &mut qc_out_channel[ch],
            &psy_out_channel[ch],
            views.get(ch).and_then(|v| v.as_deref()),
            inv_quant,
            d_zone_quant_enable,
        );
    }
}

// The functions on the list of relevant bands against the straight ones (no C reference
// needed: they are compared with the straight port, which the cref tests compare with C).
#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 32) as u32
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() as u64 % (hi as i64 - lo as i64 + 1) as u64) as i32
        }
    }

    /// A random channel: band layout (1-4 groups), spectrum with a random envelope (zero bands,
    /// full-scale and `MINVAL_DBL` lines now and then), energies from the encoder's band energy
    /// function, thresholds around them, form factors from `FDKaacEnc_CalcFormFactor`.
    fn random_channel(r: &mut Rng) -> (QcOutChannel, PsyOutChannel) {
        use crate::band_nrg::{
            fdk_aac_enc_calc_band_energy_optim_long, fdk_aac_enc_calc_sfb_max_scale_spec,
        };
        const WIDTHS: [i32; 10] = [4, 4, 4, 8, 8, 12, 16, 24, 32, 48];
        let groups = if r.range(0, 3) == 0 { r.range(2, 4) } else { 1 };
        let per_group = r.range(1, MAX_GROUPED_SFB as i32 / groups);
        let mut psy = PsyOutChannel {
            sfb_cnt: groups * per_group,
            sfb_per_group: per_group,
            max_sfb_per_group: if r.range(0, 4) == 0 { r.range(0, per_group) } else { per_group },
            ..Default::default()
        };
        let mut off = 0;
        for sfb in 0..psy.sfb_cnt as usize {
            psy.sfb_offsets[sfb] = off;
            let w = WIDTHS[r.range(0, 9) as usize];
            off = (off + w).min(1024);
        }
        psy.sfb_offsets[psy.sfb_cnt as usize] = off;
        let mut qc = QcOutChannel::default();
        let top = r.range(4, 30);
        for sfb in 0..psy.sfb_cnt as usize {
            let e = (top - (sfb as i32 * r.range(0, 3)) / 4).clamp(0, 30);
            let zero = r.range(0, 9) == 0;
            for j in psy.sfb_offsets[sfb]..psy.sfb_offsets[sfb + 1] {
                let v = &mut qc.mdct_spectrum[j as usize];
                *v = if zero {
                    0
                } else {
                    match r.range(0, 400) {
                        0 => i32::MIN,
                        1 => i32::MAX,
                        _ => (r.next() as i32) >> (31 - e),
                    }
                };
            }
        }
        let mut max_scale = [0i32; MAX_GROUPED_SFB];
        let mut nrg = [0 as FixpDbl; MAX_GROUPED_SFB];
        let cnt = psy.sfb_cnt;
        fdk_aac_enc_calc_sfb_max_scale_spec(
            &qc.mdct_spectrum,
            &psy.sfb_offsets,
            &mut max_scale,
            cnt,
        );
        fdk_aac_enc_calc_band_energy_optim_long(
            &qc.mdct_spectrum,
            &max_scale,
            &psy.sfb_offsets,
            cnt,
            &mut nrg,
            &mut qc.sfb_energy_ld_data,
        );
        for sfb in 0..cnt as usize {
            // thresholds 0..0.25 (ld64) below the energy, or above it
            let d = if r.range(0, 7) == 0 { -r.range(0, 1 << 22) } else { r.range(0, 1 << 25) };
            qc.sfb_threshold_ld_data[sfb] = qc.sfb_energy_ld_data[sfb].wrapping_sub(d);
        }
        fdk_aac_enc_calc_form_factor(
            core::slice::from_mut(&mut qc),
            core::slice::from_ref(&psy),
            1,
        );
        (qc, psy)
    }

    /// The scalefactor estimation of random channels with the optimized assimilation functions
    /// and with their straight ports.
    #[test]
    fn estimation_with_fast_assimilation_equals_straight() {
        let mut r = Rng(0x0005_eed0_fa55);
        let n = if cfg!(debug_assertions) { 600 } else { 4000 };
        let (mut changed, mut relevant) = (0, 0);
        for k in 0..n {
            let (qc, psy) = random_channel(&mut r);
            let inv_quant = if k % 5 == 0 { 1 } else { 2 };
            let dz = (k % 3 == 0) as i32;
            let (mut a, mut b) = (qc, qc);
            estimate_scale_factors_channel_with(&mut a, &psy, None, inv_quant, dz, true);
            estimate_scale_factors_channel_with(&mut b, &psy, None, inv_quant, dz, false);
            assert!(a == b, "channel {k}");
            let mut c = qc;
            estimate_scale_factors_channel_with(&mut c, &psy, None, 0, dz, true);
            changed += (c.scf != a.scf) as u32;
            relevant += (a.global_gain != 0) as u32;
        }
        // the assimilation functions changed scalefactors in most channels
        assert!(changed > n / 2 && relevant > n / 2, "{changed} {relevant} of {n}");
    }

    /// The optimized assimilation functions against the straight ports on random states,
    /// including values outside `±SCF_FAST_LIMIT` (the fallback to the straight ports).
    #[test]
    fn fast_assimilation_functions_equal_straight() {
        let mut r = Rng(0xa55_1e5);
        let n = if cfg!(debug_assertions) { 1500 } else { 10000 };
        for k in 0..n {
            let (qc, psy) = random_channel(&mut r);
            let cnt = psy.sfb_cnt as usize;
            let wide = k % 16 == 0;
            let base = r.range(-40, 80);
            let mut scf = [0i32; MAX_GROUPED_SFB];
            let mut min_scf = [0i32; MAX_GROUPED_SFB];
            let mut min_calc = [0i32; MAX_GROUPED_SFB];
            let mut dist = [0 as FixpDbl; MAX_GROUPED_SFB];
            let mut cpp = [0 as FixpDbl; MAX_GROUPED_SFB];
            let mut nrl = [0 as FixpDbl; MAX_GROUPED_SFB];
            let p_min = r.range(0, 4);
            for sfb in 0..MAX_GROUPED_SFB {
                scf[sfb] = if sfb >= cnt || r.range(0, 7) < p_min {
                    FDK_INT_MIN
                } else if wide && r.range(0, 9) == 0 {
                    r.next() as i32
                } else {
                    base + r.range(-12, 12)
                };
                min_scf[sfb] = if wide && r.range(0, 9) == 0 {
                    r.next() as i32
                } else {
                    base + r.range(-30, 6)
                };
                min_calc[sfb] = scf[sfb].wrapping_add(r.range(-3, 3));
                dist[sfb] = -(r.range(0, 1 << 28));
                cpp[sfb] = if r.range(0, 1) == 0 { FDK_INT_MIN } else { r.next() as i32 >> 3 };
                nrl[sfb] = r.range(0, 1 << 26);
            }
            let ch = ChannelIn {
                psy: &psy,
                mdct_spectrum: &qc.mdct_spectrum,
                sfb_energy_ld_data: &qc.sfb_energy_ld_data,
                psy_sfb_energy_ld_data: &qc.sfb_energy_ld_data,
                sfb_threshold_ld_data: &qc.sfb_threshold_ld_data,
                d_zone_quant_enable: (k % 2) as i32,
            };
            let mut qs = [[0i16; 1024]; 2];
            let mut qt = [[QUANT_SPEC_TMP_UNWRITTEN; 1024]; 2];
            let mut s = [scf; 2];
            let mut d = [dist; 2];
            let mut c = [cpp; 2];
            let mut m = [min_calc; 2];
            for (i, straight) in [true, false].into_iter().enumerate() {
                let (qs, qt, s, d, c, m) =
                    (&mut qs[i], &mut qt[i], &mut s[i], &mut d[i], &mut c[i], &mut m[i]);
                if k % 2 == 0 {
                    let f = if straight {
                        fdk_aac_enc_assimilate_single_scf_straight
                    } else {
                        fdk_aac_enc_assimilate_single_scf
                    };
                    f(&ch, qs, qt, s, &min_scf, d, c, &qc.sfb_form_factor_ld_data, &nrl, m, 1);
                } else {
                    let f = if straight {
                        fdk_aac_enc_assimilate_multiple_scf_straight
                    } else {
                        fdk_aac_enc_assimilate_multiple_scf
                    };
                    f(&ch, qs, qt, s, &min_scf, d, c, &qc.sfb_form_factor_ld_data, &nrl);
                }
            }
            assert!(
                qs[0] == qs[1] && qt[0] == qt[1] && s[0] == s[1],
                "case {k}: quantized lines or scalefactors"
            );
            assert!(d[0] == d[1] && c[0] == c[1] && m[0] == m[1], "case {k}: distortions");
        }
    }

    #[test]
    fn rel_bands_functions_equal_straight_port() {
        let mut r = Rng(0x0dd_b1a5_e5e5);
        for k in 0..300_000 {
            let sfb_cnt = r.range(0, MAX_GROUPED_SFB as i32);
            let p_min = r.range(0, 8);
            let wide = k % 8 == 0;
            let mut scf_old = [0i32; MAX_GROUPED_SFB + 1];
            for x in scf_old.iter_mut() {
                *x = if r.range(0, 7) < p_min {
                    FDK_INT_MIN
                } else if wide {
                    r.next() as i32
                } else {
                    r.range(-20, 100)
                };
            }
            let rel = RelBands::new(&scf_old, sfb_cnt);
            assert!(rel.is_some());
            // new values: as the callers make them (the region set to one value, the rest equal,
            // or after an earlier region of the same pass), or arbitrary
            let start = r.range(0, sfb_cnt.max(1)) - (k % 50 == 0) as i32;
            let stop = r.range(start, (sfb_cnt + (k % 40 == 0) as i32).max(start));
            let mut scf_new = scf_old;
            let v = r.range(-20, 100);
            for (sfb, x) in scf_new.iter_mut().enumerate() {
                let in_region = (start.max(0) as usize..stop.max(0) as usize).contains(&sfb);
                if *x != FDK_INT_MIN && (in_region || r.range(0, 9) == 0) {
                    *x = if wide { r.next() as i32 } else { v };
                }
            }
            if start < 0 || stop > sfb_cnt {
                // outside the regions of the callers (the rel functions defer to the straight
                // ones there)
                continue;
            }
            let a = fdk_aac_enc_count_scf_bits_diff(&scf_old, &scf_new, sfb_cnt, start, stop);
            let b = count_scf_bits_diff_rel(&scf_old, &scf_new, rel.as_ref(), sfb_cnt, start, stop);
            assert_eq!(a, b, "countScfBitsDiff {start}..{stop} of {sfb_cnt}");

            let en: Vec<FixpDbl> =
                (0..MAX_GROUPED_SFB + 1).map(|_| r.next() as i32 >> r.range(0, 31)).collect();
            let ff: Vec<FixpDbl> =
                (0..MAX_GROUPED_SFB + 1).map(|_| r.next() as i32 >> r.range(0, 31)).collect();
            let nl: Vec<FixpDbl> =
                (0..MAX_GROUPED_SFB + 1).map(|_| r.next() as i32 >> r.range(0, 31)).collect();
            let mut cpp_a: Vec<FixpDbl> = (0..MAX_GROUPED_SFB + 1)
                .map(|_| if r.range(0, 1) == 0 { FDK_INT_MIN } else { r.next() as i32 })
                .collect();
            let mut cpp_b = cpp_a.clone();
            let a = fdk_aac_enc_calc_spec_pe_diff(
                &en, &scf_old, &scf_new, &mut cpp_a, &ff, &nl, start, stop,
            );
            let b = calc_spec_pe_diff_rel(
                &en,
                &scf_old,
                &scf_new,
                rel.as_ref(),
                &mut cpp_b,
                &ff,
                &nl,
                start,
                stop,
            );
            assert_eq!((a, &cpp_a), (b, &cpp_b), "calcSpecPeDiff {start}..{stop} of {sfb_cnt}");
        }
    }
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/line_pe.h, libAACenc/src/line_pe.cpp.

//! Port of `line_pe.h`: perceptual entropy per scalefactor band.

use crate::psy_const::MAX_GROUPED_SFB;
use crate::sf_estim::FORM_FAC_SHIFT;
use fdk_aac_rs_fdk::common_fix::{f_min, f_mult, f_mult_div2, fl2fxconst_dbl, FixpDbl, FRACT_BITS};
use fdk_aac_rs_fdk::fixpoint_math::{
    calc_inv_ld_data, calc_ld_int, f_mult_i, LD_DATA_SCALING, LD_DATA_SHIFT,
};

/// `PE_CONSTPART_SHIFT`
pub const PE_CONSTPART_SHIFT: i32 = FRACT_BITS;

/// `PE_CHANNEL_DATA` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeChannelData {
    /// number of relevant lines in sfb (calculated by FDKaacEnc_prepareSfbPe)
    pub sfb_n_lines: [i32; MAX_GROUPED_SFB],
    /// pe for each sfb
    pub sfb_pe: [i32; MAX_GROUPED_SFB],
    /// constant part for each sfb
    pub sfb_const_part: [i32; MAX_GROUPED_SFB],
    /// number of active lines in sfb
    pub sfb_n_active_lines: [i32; MAX_GROUPED_SFB],
    /// sum of sfbPe
    pub pe: i32,
    /// sum of sfbConstPart
    pub const_part: i32,
    /// sum of sfbNActiveLines
    pub n_active_lines: i32,
}

impl Default for PeChannelData {
    fn default() -> Self {
        PeChannelData {
            sfb_n_lines: [0; MAX_GROUPED_SFB],
            sfb_pe: [0; MAX_GROUPED_SFB],
            sfb_const_part: [0; MAX_GROUPED_SFB],
            sfb_n_active_lines: [0; MAX_GROUPED_SFB],
            pe: 0,
            const_part: 0,
            n_active_lines: 0,
        }
    }
}

/// `PE_DATA` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeData {
    pub pe_channel_data: [PeChannelData; 2],
    pub pe: i32,
    pub const_part: i32,
    pub n_active_lines: i32,
    pub offset: i32,
}

/// `C1LdData`: C1 = 3.0 = log(8.0)/log(2)
const C1_LD_DATA: FixpDbl = fl2fxconst_dbl(3.0 / LD_DATA_SCALING as f64);
/// `C2LdData`: C2 = 1.3219281 = log(2.5)/log(2)
const C2_LD_DATA: FixpDbl = fl2fxconst_dbl(1.3219281 / LD_DATA_SCALING as f64);
/// `C3LdData`: 1-C2/C1
const C3_LD_DATA: FixpDbl = fl2fxconst_dbl(0.5593573);

/// `FDKaacEnc_prepareSfbPe`: constants that do not change during successive pe calculations.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_prepare_sfb_pe(
    pe_chan_data: &mut PeChannelData,
    sfb_energy_ld_data: &[FixpDbl],
    sfb_threshold_ld_data: &[FixpDbl],
    sfb_form_factor_ld_data: &[FixpDbl],
    sfb_offset: &[i32],
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
) {
    let form_fac_scaling: FixpDbl =
        fl2fxconst_dbl((FORM_FAC_SHIFT as f32 / LD_DATA_SCALING) as f64);

    let mut sfb_grp = 0;
    while sfb_grp < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let i = (sfb_grp + sfb) as usize;
            if sfb_energy_ld_data[i] > sfb_threshold_ld_data[i] {
                let sfb_width = sfb_offset[i + 1].wrapping_sub(sfb_offset[i]);
                // estimate number of active lines
                // C: ((-sfbEnergyLdData >> 1) + (CalcLdInt(sfbWidth) >> 1)) >> 1; the negation
                // wraps for MINVAL_DBL.
                let avg_form_factor_ld_data = ((sfb_energy_ld_data[i].wrapping_neg() >> 1)
                    + (calc_ld_int(sfb_width) >> 1))
                    >> 1;
                pe_chan_data.sfb_n_lines[i] = calc_inv_ld_data(
                    sfb_form_factor_ld_data[i]
                        .wrapping_add(form_fac_scaling)
                        .wrapping_add(avg_form_factor_ld_data),
                );
                // Make sure sfbNLines is never greater than sfbWidth due to unaccuracies (e.g.
                // sfbEnergyLdData[sfbGrp+sfb] = 0x80000000)
                pe_chan_data.sfb_n_lines[i] = f_min(sfb_width, pe_chan_data.sfb_n_lines[i]);
            } else {
                pe_chan_data.sfb_n_lines[i] = 0;
            }
        }
        sfb_grp += sfb_per_group;
    }
}

/// `FDKaacEnc_calcSfbPe`
///
/// formula for one sfb:
/// ```text
///   pe = n * ld(en/thr),                if ld(en/thr) >= C1
///   pe = n * (C2 + C3 * ld(en/thr)),    if ld(en/thr) <  C1
///   n: estimated number of lines in sfb,
///   ld(x) = log(x)/log(2)
/// ```
/// constPart is sfbPe without the threshold part n*ld(thr) or n*C3*ld(thr)
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_calc_sfb_pe(
    pe_chan_data: &mut PeChannelData,
    sfb_energy_ld_data: &[FixpDbl],
    sfb_threshold_ld_data: &[FixpDbl],
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
    is_book: &[i32],
    is_scale: &[i32],
) {
    let scale_ld: FixpDbl = 0;
    let mut last_val_is: i32 = 0;

    let mut pe: FixpDbl = 0;
    let mut const_part: FixpDbl = 0;
    let mut n_active_lines: FixpDbl = 0;

    let mut sfb_grp = 0;
    while sfb_grp < sfb_cnt {
        for sfb in 0..max_sfb_per_group {
            let mut tmp_pe: FixpDbl = 0;
            let mut tmp_const_part: FixpDbl = 0;
            let mut tmp_n_active_lines: FixpDbl = 0;

            let this_sfb = (sfb_grp + sfb) as usize;

            if sfb_energy_ld_data[this_sfb] > sfb_threshold_ld_data[this_sfb] {
                let log_data_ratio =
                    sfb_energy_ld_data[this_sfb].wrapping_sub(sfb_threshold_ld_data[this_sfb]);
                let mut n_lines = pe_chan_data.sfb_n_lines[this_sfb];

                // C: nLines << (LD_DATA_SHIFT + PE_CONSTPART_SHIFT + 1) (lsl, wraps for
                // nLines >= 256)
                let factor: FixpDbl =
                    n_lines.wrapping_shl((LD_DATA_SHIFT + PE_CONSTPART_SHIFT + 1) as u32);
                if log_data_ratio >= C1_LD_DATA {
                    // scale sfbPe and sfbConstPart with PE_CONSTPART_SHIFT
                    tmp_pe = f_mult_div2(log_data_ratio, factor);
                    tmp_const_part =
                        f_mult_div2(sfb_energy_ld_data[this_sfb].wrapping_add(scale_ld), factor);
                } else {
                    // scale sfbPe and sfbConstPart with PE_CONSTPART_SHIFT
                    tmp_pe = f_mult_div2(
                        C2_LD_DATA.wrapping_add(f_mult(C3_LD_DATA, log_data_ratio)),
                        factor,
                    );
                    tmp_const_part = f_mult_div2(
                        C2_LD_DATA.wrapping_add(f_mult(
                            C3_LD_DATA,
                            sfb_energy_ld_data[this_sfb].wrapping_add(scale_ld),
                        )),
                        factor,
                    );

                    n_lines = f_mult_i(C3_LD_DATA, n_lines);
                }
                tmp_n_active_lines = n_lines;
            } else if is_book[this_sfb] != 0 {
                // provide for cost of scale factor for Intensity
                let _delta = is_scale[this_sfb].wrapping_sub(last_val_is);
                last_val_is = is_scale[this_sfb];
                // C: sfbPe = FDKaacEnc_bitCountScalefactorDelta(delta) << PE_CONSTPART_SHIFT;
                // sfbConstPart = 0; sfbNActiveLines = 0. All three are overwritten below, and the
                // compiled reference (arm64 and x86) does not read FDKaacEnc_huff_ltabscf here
                // at all, so the port does not evaluate the (possibly out-of-range) delta.
            }
            pe_chan_data.sfb_pe[this_sfb] = tmp_pe;
            pe_chan_data.sfb_const_part[this_sfb] = tmp_const_part;
            pe_chan_data.sfb_n_active_lines[this_sfb] = tmp_n_active_lines;

            // sum up peChanData values
            pe = pe.wrapping_add(tmp_pe);
            const_part = const_part.wrapping_add(tmp_const_part);
            n_active_lines = n_active_lines.wrapping_add(tmp_n_active_lines);
        }
        sfb_grp += sfb_per_group;
    }

    // correct scaled pe and constPart values
    pe_chan_data.pe = pe >> PE_CONSTPART_SHIFT;
    pe_chan_data.const_part = const_part >> PE_CONSTPART_SHIFT;

    pe_chan_data.n_active_lines = n_active_lines;
}

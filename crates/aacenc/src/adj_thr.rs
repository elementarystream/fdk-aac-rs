// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/adj_thr.h, libAACenc/src/adj_thr.cpp.

//! Port of `adj_thr.h`: threshold adjustment (bit reservoir control, PE correction, avoid-hole
//! logic, VBR).
//!
//! # Memory model
//!
//! The C functions get arrays of pointers `qcOutChannel[2]` / `psyOutChannel[2]` of one element;
//! here they get the slices of the element's channels (`QcOutChannel`s of `QcOut`,
//! `PsyOutChannel`s of `PsyOut`, see [`qc_data`](crate::qc_data)). The six arrays that
//! `PSY_OUT_CHANNEL` aliases (`sfbEnergy`, `sfbEnergyLdData`, `sfbThresholdLdData`, ...) are read
//! from the `QcOutChannel` of the same channel.
//!
//! The scratch buffers of C:
//!
//! * `dynMem_Ah_Flag`, `dynMem_Thr_Exp` (`ahFlag[8][2][60]`, `thrExp[8][2][60]` of
//!   `FDKaacEnc_adaptThresholdsToPe()`) and `dynMem_SfbNActiveLinesLdData`
//!   (`FDKaacEnc_correctThresh()`) live in the shared dynamic RAM `P_BUF_1` in C, which also holds
//!   `PSY_DYNAMIC` and the bit counter look-up tables. Where the reference defines the result, it
//!   does not depend on the earlier contents (checked by the tests with poisoned buffers; see
//!   [`adj_thr_stale_ah_flag_reads`]). The buffers are available as [`AdjThrDynMem`] with the C
//!   byte layout ([`fdk_aac_enc_adjust_thresholds_dyn_mem`]); [`fdk_aac_enc_adjust_thresholds`]
//!   uses a local copy.
//! * `FDKaacEnc_correctThresh()` keeps `sfbPeFactorsLdData` in the `quantSpec` arrays of the
//!   `QC_OUT_CHANNEL`s ("scratch is empty; use temporal memory from quantSpec"). These writes are
//!   visible state of `QC_OUT_CHANNEL` and are reproduced: entry `i` occupies
//!   `quantSpec[2 * i]` (low half) and `quantSpec[2 * i + 1]` (high half), the little-endian layout
//!   of both reference targets.
//! * `FDKaacEnc_AdaptThresholdsVBR()` uses stack scratch buffers (`C_ALLOC_SCRATCH_START`); all
//!   entries that are read are written first.
//!
//! # Compiled form of overflowing expressions
//!
//! Where the reference builds evaluate an expression with signed overflow differently from the
//! 32-bit wrapping evaluation of the source (clang rewrites some comparisons and sums under the
//! assumption that signed arithmetic does not overflow), the port follows the compiled code; each
//! such site is commented with the C++ expression and the instructions it was compiled to.

use crate::aacenc::MIN_BUFSIZE_PER_EFF_CHAN;
use crate::adj_thr_data::*;
use crate::interface::{PsyOut, PsyOutChannel, ToolsInfo};
use crate::line_pe::{
    fdk_aac_enc_calc_sfb_pe, fdk_aac_enc_prepare_sfb_pe, PeChannelData, PeData, PE_CONSTPART_SHIFT,
};
use crate::psy_const::*;
use crate::qc_data::{ChannelMapping, QcOut, QcOutChannel, QcOutElement};
use crate::sf_estim::FORM_FAC_SHIFT;
use fdk_aac_rs_fdk::audio::{ID_CPE, ID_DSE, ID_LFE, ID_SCE};
use fdk_aac_rs_fdk::clz::{fixnorm_d, fixnormz_d};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::scale::scale_value;
use fdk_aac_rs_fdk::{Arch, ARCH};

pub mod tables;
use tables::*;

/// `NUM_NRG_LEVS`
const NUM_NRG_LEVS: usize = 8;
/// `INV_INT_TAB_SIZE`
const INV_INT_TAB_SIZE: i32 = 8;
/// `INV_SQRT4_TAB_SIZE`
const INV_SQRT4_TAB_SIZE: i32 = 8;

/// `AFTERBURNER_STATI`
const AFTERBURNER_STATI: usize = 2;
/// `MAX_ALLOWED_EL_CHANNELS`
const MAX_ALLOWED_EL_CHANNELS: usize = 2;

/// `BIT_PE_SFAC`
pub struct BitPeSfac {
    pub bitrate: i32,
    pub bits2_pe_factor: [[FixpDbl; MAX_ALLOWED_EL_CHANNELS]; AFTERBURNER_STATI],
}

/// `BITS2PE_CFG_TAB` (`nEntries` is `p_pe_tab.len()`)
pub struct Bits2PeCfgTab {
    pub sample_rate: i32,
    pub p_pe_tab: &'static [BitPeSfac],
}

/// `_avoid_hole_state`: values for avoid hole flag
pub const NO_AH: u8 = 0;
/// `AH_INACTIVE`
pub const AH_INACTIVE: u8 = 1;
/// `AH_ACTIVE`
pub const AH_ACTIVE: u8 = 2;

/// `Q_BITFAC`: Q scaling used in FDKaacEnc_bitresCalcBitFac() calculation
#[allow(dead_code)]
const Q_BITFAC: i32 = 24;
/// `Q_AVGBITS`: scale bit values
const Q_AVGBITS: i32 = 17;

/// `CONSTPART_HEADROOM`
const CONSTPART_HEADROOM: i32 = 4;

/// `ahFlag[(2)][MAX_GROUPED_SFB]` of one element
pub type AhFlag = [[u8; MAX_GROUPED_SFB]; 2];
/// `thrExp[(2)][MAX_GROUPED_SFB]` of one element
pub type ThrExp = [[FixpDbl; MAX_GROUPED_SFB]; 2];

/// The adjust-thresholds part of the shared dynamic RAM `P_BUF_1` (`dynMem_Ah_Flag`,
/// `dynMem_Thr_Exp`, `dynMem_SfbNActiveLinesLdData` of `QC_OUT_ELEMENT`, which all elements
/// share), with the byte layout of C (`ADJ_THR_AH_FLAG_SIZE`, `ADJ_THR_THR_EXP_SIZE`,
/// `ADJ_THR_ACT_LIN_LD_DATA_SIZE` from `P_BUF_1 + 0`).
///
/// Where the reference defines the result of the threshold adjustment, it does not depend on the
/// contents (see [`adj_thr_stale_ah_flag_reads`]). In C, these bytes overlap `PSY_DYNAMIC` and
/// the bit counter look-up tables; [`fdk_aac_enc_adjust_thresholds_dyn_mem`] keeps them in a
/// buffer that the caller owns (entries that a call does not write keep their contents, as in
/// C).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdjThrDynMem {
    /// `ahFlag[8][2][MAX_GROUPED_SFB]` (FDKaacEnc_adaptThresholdsToPe)
    pub ah_flag: [AhFlag; 8],
    /// `thrExp[8][2][MAX_GROUPED_SFB]` (FDKaacEnc_adaptThresholdsToPe)
    pub thr_exp: [ThrExp; 8],
    /// `sfbNActiveLinesLdData[8][2][MAX_GROUPED_SFB]` (FDKaacEnc_correctThresh)
    pub sfb_n_active_lines_ld_data: [[[FixpDbl; MAX_GROUPED_SFB]; 2]; 8],
}

impl Default for AdjThrDynMem {
    fn default() -> Self {
        AdjThrDynMem {
            ah_flag: [[[0; MAX_GROUPED_SFB]; 2]; 8],
            thr_exp: [[[0; MAX_GROUPED_SFB]; 2]; 8],
            sfb_n_active_lines_ld_data: [[[0; MAX_GROUPED_SFB]; 2]; 8],
        }
    }
}

/// `for (sfbGrp = 0; sfbGrp < sfbCnt; sfbGrp += sfbPerGroup) for (sfb = 0; sfb < maxSfbPerGroup;
/// sfb++)`: calls `f(sfbGrp + sfb)`.
#[inline(always)]
fn for_each_sfb(p: &PsyOutChannel, mut f: impl FnMut(usize)) {
    let mut sfb_grp = 0;
    while sfb_grp < p.sfb_cnt {
        for sfb in 0..p.max_sfb_per_group {
            f((sfb_grp + sfb) as usize);
        }
        sfb_grp += p.sfb_per_group;
    }
}

/// `CalcLdData(x)` (`fLog2(x, 0)`) without the test `x <= 0`, as the reference builds compile
/// it where clang concludes (assuming no signed overflow) that the argument is positive: the
/// normalization shift is `clz(x) - 1` (-1, taken modulo 32, for negative `x`), and
/// `x2_m - 1.0` is computed as `-(x2_m | MINVAL_DBL)` (arm64 `orr`/`neg`, x86 `orl`/`negl`). Equal
/// to `CalcLdData(x)` for `x > 0`. (For `x == 0`, x86 `bsr` leaves its destination undefined; the
/// port computes the arm64 result.)
fn calc_ld_data_unguarded(x: FixpDbl) -> FixpDbl {
    let b_norm = fixnormz_d(x) - 1;
    let x2_m = (x.wrapping_shl(b_norm as u32) | MINVAL_DBL).wrapping_neg();
    let x_e: i32 = 0i32.wrapping_sub(b_norm);
    // the polynomial of fLog2() (the optimized form for x > 0, see `f_log2_poly`)
    let mut result_m = f_log2_poly(x2_m);
    let result_e = if x_e != 0 {
        let enorm = DFRACT_BITS - fixnorm_d(x_e);
        result_m = (result_m >> (enorm - 1))
            .wrapping_add(x_e.wrapping_shl((DFRACT_BITS - 1 - enorm) as u32));
        enorm
    } else {
        1
    };
    scale_value(result_m, result_e - LD_DATA_SHIFT)
}

/// `CalcLdData(fAbs(x))` as the reference builds compile it: clang assumes that `fAbs()` is not
/// negative and tests only for 0; for `x == MINVAL_DBL` (`fAbs()` overflows) the logarithm is
/// the unguarded one of `MINVAL_DBL` (see [`calc_ld_data_unguarded`]); `fLog2()` would return
/// -1.0.
fn calc_ld_data_abs(x: FixpDbl) -> FixpDbl {
    if x == 0 {
        fl2fxconst_dbl(-1.0)
    } else {
        calc_ld_data_unguarded(f_abs(x))
    }
}

/// `CalcInvLdData(y - c)` as both reference builds compile it where the argument is a sum minus
/// the constant `c`: clang folds `c` into the comparisons of `CalcInvLdData()` (assuming that
/// the subtraction does not overflow), so the range checks (`x < -31/64`, `x >= 31/64`,
/// `x > 0`) use the mathematical value `y - c`, while the table indices, the fraction and the
/// shift amount use the wrapped 32-bit difference (for `x > 0` the shift amount
/// `31 - (x >> 25)` is computed with a logical shift). Identical to `CalcInvLdData(y - c)`
/// when `y - c` does not overflow.
fn calc_inv_ld_data_sub(y: FixpDbl, c: FixpDbl) -> FixpDbl {
    let xm = y as i64 - c as i64;
    let x = y.wrapping_sub(c);
    let set_zero: u32 = if xm < fl2fxconst_dbl(-31.0 / 64.0) as i64 { 0 } else { 1 };
    let set_max = x == 0 || xm >= fl2fxconst_dbl(31.0 / 64.0) as i64;

    let frac = (x & 0x3FF) as FixpSgl;
    let index3 = (x >> 10) as u32 & 0x1F;
    let index2 = (x >> 15) as u32 & 0x1F;
    let index1 = (x >> 20) as u32 & 0x1F;
    let exp = f_min(31, if xm > 0 { 31 - ((x as u32) >> 25) as i32 } else { -(x >> 25) });

    let lookup1 = EXP2_TAB_LONG[index1 as usize].wrapping_mul(set_zero);
    let lookup2 = EXP2W_TAB_LONG[index2 as usize];
    let lookup3 = EXP2X_TAB_LONG[index3 as usize];
    let lookup3f = lookup3.wrapping_add(f_mult_div2(0x0016302F_i32, frac) as u32);

    let lookup12 = f_mult(lookup1 as i32, lookup2 as i32) as u32;
    let lookup = f_mult(lookup12 as i32, lookup3f as i32) as u32;

    let ret_val = (lookup << 3).wrapping_shr(exp as u32) as FixpDbl;
    if set_max {
        MAXVAL_DBL
    } else {
        ret_val
    }
}

/// `FDKaacEnc_InitBits2PeFactor`: retrieve bits2PeFactor from table (static in C; public for the
/// differential tests)
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_init_bits2_pe_factor(
    bits2_pe_factor_m: &mut FixpDbl,
    bits2_pe_factor_e: &mut i32,
    bit_rate: i32,
    n_channels: i32,
    sample_rate: i32,
    advanced_bits_to_pe: i32,
    d_zone_quant_enable: i32,
    inv_quant: i32,
) {
    // 1) Set default bits2pe factor
    let mut bit2_pe_m: FixpDbl = fl2fxconst_dbl((1.18f32 / 2.0f32) as f64);
    let mut bit2_pe_e: i32 = 1;

    // 2) For AAC-(E)LD, make use of advanced bits to pe factor table
    if advanced_bits_to_pe != 0 && n_channels <= 2 {
        let mut pe_tab: Option<&'static [BitPeSfac]> = None;
        let mut size: usize = 0;

        // 2.1) Get correct table entry
        for t in BITS2_PE_CONFIG_TAB.iter() {
            if sample_rate >= t.sample_rate {
                pe_tab = Some(t.p_pe_tab);
                size = t.p_pe_tab.len();
            }
        }

        if let (Some(pe_tab), true) = (pe_tab, size != 0) {
            // bitrate entry in table that is the next-lower to actual bitrate
            let mut start_b: i32 = -1;
            // bitrate entry in table that is the next-higher to actual bitrate
            let mut stop_b: i32 = -1;
            // bits2PE factor entry in table that is the next-lower to actual bits2PE factor
            let mut start_pf: FixpDbl = 0;
            // bits2PE factor entry in table that is the next-higher to actual bits2PE factor
            let mut stop_pf: FixpDbl = 0;
            let quality_idx = if inv_quant == 0 { 0 } else { 1 };
            // nChannels is 1 or 2 here (element channels).
            let ch_idx = (n_channels - 1) as usize;

            if bit_rate >= pe_tab[size - 1].bitrate {
                // Chosen bitrate is higher than the highest bitrate in table.
                // The slope for extrapolating the bits2PE factor must be zero.
                // Values are set accordingly.
                start_b = pe_tab[size - 1].bitrate;
                // Can be an arbitrary value greater than startB and bitrate.
                stop_b = bit_rate.wrapping_add(1);
                start_pf = pe_tab[size - 1].bits2_pe_factor[quality_idx][ch_idx];
                stop_pf = pe_tab[size - 1].bits2_pe_factor[quality_idx][ch_idx];
            } else {
                for i in 0..size - 1 {
                    if pe_tab[i].bitrate <= bit_rate && pe_tab[i + 1].bitrate > bit_rate {
                        start_b = pe_tab[i].bitrate;
                        stop_b = pe_tab[i + 1].bitrate;
                        start_pf = pe_tab[i].bits2_pe_factor[quality_idx][ch_idx];
                        stop_pf = pe_tab[i + 1].bits2_pe_factor[quality_idx][ch_idx];
                        break;
                    }
                }
            }

            // 2.2) Configuration available?
            if start_b != -1 {
                // 2.2.1) linear interpolate to actual PEfactor
                let max_bit2_pe: FixpDbl = fl2fxconst_dbl((3.0f32 / 4.0f32) as f64);

                // bit2PE = ((stopPF-startPF)/(stopB-startB))*(bitRate-startB)+startPF;
                let slope = f_div_norm_noexp(bit_rate - start_b, stop_b - start_b);
                let mut bit2_pe = f_mult(slope, stop_pf - start_pf).wrapping_add(start_pf);

                bit2_pe = f_min(max_bit2_pe, bit2_pe);

                // 2.2.2) sanity check if bits2pe value is high enough
                if bit2_pe >= (fl2fxconst_dbl(0.35f32 as f64) >> 2) {
                    bit2_pe_m = bit2_pe;
                    bit2_pe_e = 2; // table is fixed scaled
                }
            }
        }
    }

    if d_zone_quant_enable != 0 {
        if bit2_pe_m >= (fl2fxconst_dbl(0.6f32 as f64) >> bit2_pe_e) {
            // Additional headroom for addition
            bit2_pe_m >>= 1;
            bit2_pe_e += 1;
        }

        // the quantTendencyCompensator compensates a lower bit consumption due to increasing
        // the tendency to quantize low spectral values to the lower quantizer border for
        // bitrates below a certain bitrate threshold --> see also function calcSfbDistLD in
        // quantize.c
        let br = bit_rate / n_channels;
        if br > 32000 && br <= 40000 {
            bit2_pe_m += fl2fxconst_dbl(0.4f32 as f64) >> bit2_pe_e;
        } else if br > 20000 {
            bit2_pe_m += fl2fxconst_dbl(0.3f32 as f64) >> bit2_pe_e;
        } else if br >= 16000 {
            bit2_pe_m += fl2fxconst_dbl(0.3f32 as f64) >> bit2_pe_e;
        } else {
            bit2_pe_m += fl2fxconst_dbl(0.0f32 as f64) >> bit2_pe_e;
        }
    }

    // 3.) Return bits2pe factor
    *bits2_pe_factor_m = bit2_pe_m;
    *bits2_pe_factor_e = bit2_pe_e;
}

/// `FDKaacEnc_bits2pe2`: convert from bits to pe
///
/// `(INT)(fMult(factor_m, (FIXP_DBL)(bits << Q_AVGBITS)) >> (Q_AVGBITS - factor_e))`: the left
/// shift wraps, the right shift amount is taken modulo 32 (`asr` with a register amount).
#[inline]
pub fn fdk_aac_enc_bits2pe2(bits: i32, factor_m: FixpDbl, factor_e: i32) -> i32 {
    f_mult(factor_m, bits.wrapping_shl(Q_AVGBITS as u32))
        .wrapping_shr(Q_AVGBITS.wrapping_sub(factor_e) as u32)
}

/// `FDKaacEnc_calcThreshExp`: loudness calculation (threshold to the power of redExp) (static in
/// C; public for the differential tests)
pub fn fdk_aac_enc_calc_thresh_exp(
    thr_exp: &mut ThrExp,
    qc_out_channel: &[QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    n_channels: i32,
) {
    fdk_aac_enc_calc_thresh_exp_views(
        thr_exp,
        qc_out_channel,
        psy_out_channel,
        &NO_PSY_ARRAY_VIEWS,
        n_channels,
    )
}

/// [`fdk_aac_enc_calc_thresh_exp`] with the arrays C reads through the `PSY_OUT_CHANNEL`
/// (`sfbThresholdLdData`) from `views` where set.
pub fn fdk_aac_enc_calc_thresh_exp_views(
    thr_exp: &mut ThrExp,
    qc_out_channel: &[QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    views: &PsyArrayViews,
    n_channels: i32,
) {
    for ch in 0..n_channels as usize {
        let qc = views.get(ch).and_then(|v| v.as_deref()).unwrap_or(&qc_out_channel[ch]);
        for_each_sfb(&psy_out_channel[ch], |i| {
            let thr_exp_ld_data = qc.sfb_threshold_ld_data[i] >> 2;
            thr_exp[ch][i] = calc_inv_ld_data(thr_exp_ld_data);
        });
    }
}

/// `FDKaacEnc_adaptMinSnr`: reduce minSnr requirements for bands with relative low energies
/// (static in C; public for the differential tests)
pub fn fdk_aac_enc_adapt_min_snr(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    msa_param: &MinsnrAdaptParam,
    n_channels: i32,
) {
    fdk_aac_enc_adapt_min_snr_views(
        qc_out_channel,
        psy_out_channel,
        &NO_PSY_ARRAY_VIEWS,
        msa_param,
        n_channels,
    )
}

/// [`fdk_aac_enc_adapt_min_snr`] with the arrays C reads through the `PSY_OUT_CHANNEL`
/// (`sfbEnergy` of the average) from `views` where set.
pub fn fdk_aac_enc_adapt_min_snr_views(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    views: &PsyArrayViews,
    msa_param: &MinsnrAdaptParam,
    n_channels: i32,
) {
    // ld64(0.8f)
    let min_snr_limit_ld64: FixpDbl = fl2fxconst_dbl(-0.00503012648262f32 as f64);

    let msa_param_max_red = msa_param.max_red;
    let msa_param_start_ratio = msa_param.start_ratio;
    let msa_param_red_ratio_fac =
        f_mult(msa_param.red_ratio_fac, fl2fxconst_dbl(0.3010299956f32 as f64));
    let msa_param_red_offs = msa_param.red_offs;

    for ch in 0..n_channels as usize {
        let psy = &psy_out_channel[ch];
        let view = views.get(ch).and_then(|v| v.as_deref());
        let qc = &mut qc_out_channel[ch];
        // psyOutChannel[ch]->sfbEnergy
        let psy_sfb_energy = view.map_or(&qc.sfb_energy, |v| &v.sfb_energy);

        // calc average energy per scalefactor band
        let mut n_sfb: i32 = 0;
        let mut accu: FixpDbl = 0;

        let mut sfb_grp = 0;
        while sfb_grp < psy.sfb_cnt {
            let max_sfb_per_group = psy.max_sfb_per_group;
            n_sfb += max_sfb_per_group;
            for sfb in 0..max_sfb_per_group {
                accu = accu.wrapping_add(psy_sfb_energy[(sfb_grp + sfb) as usize] >> 6);
            }
            sfb_grp += psy.sfb_per_group;
        }

        let avg_en_ld64 = if accu == 0 || n_sfb == 0 {
            fl2fxconst_dbl(-1.0)
        } else {
            let n_sfb_ld64 = calc_ld_int(n_sfb);
            let avg_en_ld64 = calc_ld_data(accu);
            // 0.09375f: compensate shift with 6
            avg_en_ld64.wrapping_add(fl2fxconst_dbl(0.09375f32 as f64)).wrapping_sub(n_sfb_ld64)
        };

        // reduce minSnr requirement by minSnr^minSnrRed dependent on avgEn/sfbEn
        for_each_sfb(psy, |i| {
            let sfb_energy_ld_data = qc.sfb_energy_ld_data[i];
            let sfb_min_snr_ld_data = qc.sfb_min_snr_ld_data[i];
            let db_ratio = avg_en_ld64.wrapping_sub(sfb_energy_ld_data);
            let update = msa_param_start_ratio < db_ratio;
            // scaled by 1.0f/64.0f
            let mut min_snr_red =
                msa_param_red_offs.wrapping_add(f_mult(msa_param_red_ratio_fac, db_ratio));
            min_snr_red = fix_max(min_snr_red, msa_param_max_red); // scaled by 1.0f/64.0f
            min_snr_red = f_mult(sfb_min_snr_ld_data, min_snr_red).wrapping_shl(6);
            min_snr_red = fix_min(min_snr_limit_ld64, min_snr_red);
            qc.sfb_min_snr_ld_data[i] = if update { min_snr_red } else { sfb_min_snr_ld_data };
        });
    }
}

/// `FDKaacEnc_initAvoidHoleFlag`: determine bands where avoid hole is not necessary resp.
/// possible (static in C; public for the differential tests)
pub fn fdk_aac_enc_init_avoid_hole_flag(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    ah_flag: &mut AhFlag,
    tools_info: &ToolsInfo,
    n_channels: i32,
    ah_param: &AhParam,
) {
    let n = n_channels as usize;

    // decrease spread energy by 3dB for long blocks, resp. 2dB for shorts (avoid more holes in
    // long blocks)
    for ch in 0..n {
        let psy = &psy_out_channel[ch];
        let qc = &mut qc_out_channel[ch];
        if psy.last_window_sequence != SHORT_WINDOW {
            for_each_sfb(psy, |i| qc.sfb_spread_energy[i] >>= 1);
        } else {
            for_each_sfb(psy, |i| {
                qc.sfb_spread_energy[i] =
                    f_mult(fl2fxconst_dbl(0.63f32 as f64), qc.sfb_spread_energy[i]);
            });
        }
    }

    // increase minSnr for local peaks, decrease it for valleys
    if ah_param.modify_min_snr != 0 {
        for ch in 0..n {
            let psy = &psy_out_channel[ch];
            let qc = &mut qc_out_channel[ch];
            let mut sfb_grp = 0;
            // the neighbour averages of a group and their ld values with the vector
            // CalcLdData: the loop does not write sfbEnergy
            let mut avg = [0 as FixpDbl; MAX_GROUPED_SFB];
            let mut avg_ld = [0 as FixpDbl; MAX_GROUPED_SFB];
            while sfb_grp < psy.sfb_cnt {
                let n_grp = psy.max_sfb_per_group.max(0) as usize;
                for (sfb, a) in avg[..n_grp].iter_mut().enumerate() {
                    let i = sfb_grp as usize + sfb;
                    let sfb_enm1 = if sfb > 0 { qc.sfb_energy[i - 1] } else { qc.sfb_energy[i] };
                    let sfb_enp1 =
                        if sfb + 1 < n_grp { qc.sfb_energy[i + 1] } else { qc.sfb_energy[i] };
                    *a = (sfb_enm1 >> 1) + (sfb_enp1 >> 1);
                }
                ld_data_vector(&avg[..n_grp], &mut avg_ld);
                for sfb in 0..psy.max_sfb_per_group {
                    let i = (sfb_grp + sfb) as usize;
                    let avg_en = avg[sfb as usize];
                    let avg_en_ld_data = avg_ld[sfb as usize];
                    let sfb_en = qc.sfb_energy[i];
                    let sfb_en_ld_data = qc.sfb_energy_ld_data[i];
                    // peak ?
                    if sfb_en > avg_en {
                        let tmp_min_snr_ld_data = if psy.last_window_sequence == LONG_WINDOW {
                            SNR_LD_FAC.wrapping_add(fix_max(
                                avg_en_ld_data.wrapping_sub(sfb_en_ld_data),
                                SNR_LD_MIN_1.wrapping_sub(SNR_LD_FAC),
                            ))
                        } else {
                            SNR_LD_FAC.wrapping_add(fix_max(
                                avg_en_ld_data.wrapping_sub(sfb_en_ld_data),
                                SNR_LD_MIN_3.wrapping_sub(SNR_LD_FAC),
                            ))
                        };

                        qc.sfb_min_snr_ld_data[i] =
                            fix_min(qc.sfb_min_snr_ld_data[i], tmp_min_snr_ld_data);
                    }
                    // valley ?
                    if sfb_en_ld_data.wrapping_add(SNR_LD_MIN_4) < avg_en_ld_data
                        && sfb_en > fl2fxconst_dbl(0.0)
                    {
                        // C++:
                        //   tmp = avgEnLdData - sfbEnLdData - SnrLdMin4 + minSnr;
                        //   tmp = fixMin(SnrLdFac, tmp);
                        //   minSnr = fixMin(tmp, minSnr + SnrLdMin2);
                        // Apple clang (ref-arm64) compiles this (assuming no overflow) as
                        //   fixMin(fixMin(avgEnLdData - sfbEnLdData, SnrLdMin2 + SnrLdMin4)
                        //          + (minSnr - SnrLdMin4), SnrLdFac)
                        // (sub, cmp/csel with 0x0551e1a2, add, add, cmp/csel), which differs
                        // where the sums overflow. Debian clang 19 (ref-x86_64 at 0x5cfd-0x5d28,
                        // ref-i686 at 0x5ed6 and 0x6108) keeps the source order with wrapping
                        // additions (lea, add $0xfe000000, add $0x351e1a2, cmp/cmovl, cmp/cmovge
                        // with 0xff5b2c3e). Apple clang for x86-64 (ref-x86_64-mac) uses the
                        // arm64 form; the port does not reproduce that build.
                        let min_snr = qc.sfb_min_snr_ld_data[i];
                        if matches!(ARCH, Arch::X86 | Arch::X86_32) {
                            let t = avg_en_ld_data
                                .wrapping_sub(sfb_en_ld_data)
                                .wrapping_sub(SNR_LD_MIN_4)
                                .wrapping_add(min_snr);
                            let t = fix_min(SNR_LD_FAC, t);
                            qc.sfb_min_snr_ld_data[i] =
                                fix_min(t, min_snr.wrapping_add(SNR_LD_MIN_2));
                        } else {
                            let t = fix_min(
                                avg_en_ld_data.wrapping_sub(sfb_en_ld_data),
                                SNR_LD_MIN_2.wrapping_add(SNR_LD_MIN_4),
                            );
                            let t = t.wrapping_add(min_snr.wrapping_sub(SNR_LD_MIN_4));
                            qc.sfb_min_snr_ld_data[i] = fix_min(t, SNR_LD_FAC);
                        }
                    }
                }
                sfb_grp += psy.sfb_per_group;
            }
        }
    }

    // stereo: adapt the minimum requirements sfbMinSnr of mid and side channels to avoid
    // spending unnoticable bits
    if n_channels == 2 {
        let psy_out_chan_m = &psy_out_channel[0];
        let (qm, qs) = qc_out_channel.split_at_mut(1);
        let qc_out_chan_m = &mut qm[0];
        let qc_out_chan_s = &mut qs[0];
        for_each_sfb(psy_out_chan_m, |i| {
            if tools_info.ms_mask[i] != 0 {
                let max_sfb_en_ld = fix_max(
                    qc_out_chan_m.sfb_energy_ld_data[i],
                    qc_out_chan_s.sfb_energy_ld_data[i],
                );

                let max_thr_ld = if (SNR_LD_MIN_5 >> 1)
                    .wrapping_add(max_sfb_en_ld >> 1)
                    .wrapping_add(qc_out_chan_m.sfb_min_snr_ld_data[i] >> 1)
                    <= fl2fxconst_dbl(-0.5f32 as f64)
                {
                    fl2fxconst_dbl(-1.0)
                } else {
                    SNR_LD_MIN_5
                        .wrapping_add(max_sfb_en_ld)
                        .wrapping_add(qc_out_chan_m.sfb_min_snr_ld_data[i])
                };

                let sfb_min_snr_tmp_ld = if qc_out_chan_m.sfb_energy[i] > 0 {
                    max_thr_ld.wrapping_sub(qc_out_chan_m.sfb_energy_ld_data[i])
                } else {
                    0
                };

                qc_out_chan_m.sfb_min_snr_ld_data[i] =
                    fix_max(qc_out_chan_m.sfb_min_snr_ld_data[i], sfb_min_snr_tmp_ld);

                if qc_out_chan_m.sfb_min_snr_ld_data[i] <= 0 {
                    qc_out_chan_m.sfb_min_snr_ld_data[i] =
                        fix_min(qc_out_chan_m.sfb_min_snr_ld_data[i], SNR_LD_FAC);
                }

                let sfb_min_snr_tmp_ld = if qc_out_chan_s.sfb_energy[i] > 0 {
                    max_thr_ld.wrapping_sub(qc_out_chan_s.sfb_energy_ld_data[i])
                } else {
                    0
                };

                qc_out_chan_s.sfb_min_snr_ld_data[i] =
                    fix_max(qc_out_chan_s.sfb_min_snr_ld_data[i], sfb_min_snr_tmp_ld);

                if qc_out_chan_s.sfb_min_snr_ld_data[i] <= 0 {
                    qc_out_chan_s.sfb_min_snr_ld_data[i] =
                        fix_min(qc_out_chan_s.sfb_min_snr_ld_data[i], SNR_LD_FAC);
                }

                if qc_out_chan_m.sfb_energy[i] > qc_out_chan_m.sfb_spread_energy[i] {
                    qc_out_chan_s.sfb_spread_energy[i] =
                        f_mult(qc_out_chan_s.sfb_energy[i], fl2fxconst_dbl(0.9f32 as f64));
                }

                if qc_out_chan_s.sfb_energy[i] > qc_out_chan_s.sfb_spread_energy[i] {
                    qc_out_chan_m.sfb_spread_energy[i] =
                        f_mult(qc_out_chan_m.sfb_energy[i], fl2fxconst_dbl(0.9f32 as f64));
                }
            }
        });
    }

    // init ahFlag (0: no ah necessary, 1: ah possible, 2: ah active
    for ch in 0..n {
        let qc = &qc_out_channel[ch];
        for_each_sfb(&psy_out_channel[ch], |i| {
            if qc.sfb_spread_energy[i] > qc.sfb_energy[i] || qc.sfb_min_snr_ld_data[i] > 0 {
                ah_flag[ch][i] = NO_AH;
            } else {
                ah_flag[ch][i] = AH_INACTIVE;
            }
        });
    }
}

/// `FDKaacEnc_preparePe`: calculate constants that do not change during successive pe
/// calculations (static in C; public for the differential tests)
pub fn fdk_aac_enc_prepare_pe(
    pe_data: &mut PeData,
    psy_out_channel: &[PsyOutChannel],
    qc_out_channel: &[QcOutChannel],
    n_channels: i32,
    pe_offset: i32,
) {
    for ch in 0..n_channels as usize {
        let psy_out_chan = &psy_out_channel[ch];
        let qc = &qc_out_channel[ch];
        fdk_aac_enc_prepare_sfb_pe(
            &mut pe_data.pe_channel_data[ch],
            &qc.sfb_energy_ld_data,
            &qc.sfb_threshold_ld_data,
            &qc.sfb_form_factor_ld_data,
            &psy_out_chan.sfb_offsets,
            psy_out_chan.sfb_cnt,
            psy_out_chan.sfb_per_group,
            psy_out_chan.max_sfb_per_group,
        );
    }
    pe_data.offset = pe_offset;
}

/// `FDKaacEnc_calcWeighting`: calculate weighting factor for threshold adjustment (static in C;
/// public for the differential tests)
///
/// Calculate weighting factor to be applied at energies and thresholds in ld64 format.
pub fn fdk_aac_enc_calc_weighting(
    pe_data: &PeData,
    psy_out_channel: &[PsyOutChannel],
    qc_out_channel: &mut [QcOutChannel],
    tools_info: &ToolsInfo,
    adj_thr_state_element: &mut AtsElement,
    n_channels: i32,
    use_patch_tool: i32,
) {
    let n = n_channels as usize;
    let mut no_short_window_in_frame = true;
    let mut exe_patch_m: i32 = 0;

    for ch in 0..n {
        if psy_out_channel[ch].last_window_sequence == SHORT_WINDOW {
            no_short_window_in_frame = false;
        }
        qc_out_channel[ch].sfb_en_fac_ld = [0; MAX_GROUPED_SFB];
    }

    if use_patch_tool == 0 {
        return; // tool is disabled
    }

    for ch in 0..n {
        let psy_out_chan = &psy_out_channel[ch];
        let qc = &mut qc_out_channel[ch];

        if no_short_window_in_frame {
            // retain energy ratio between blocks of different length

            let mut n_lines_sum: i32 = 0;
            let mut nrg_sum14: FixpDbl = 0;
            let mut nrg_sum12: FixpDbl = 0;
            let mut nrg_sum34: FixpDbl = 0;
            let mut nrg_total: FixpDbl = 0;

            // calculate flatness of audible spectrum, i.e. spectrum above masking threshold.
            for_each_sfb(psy_out_chan, |i| {
                let nrg_fac12 = calc_inv_ld_data(qc.sfb_energy_ld_data[i] >> 1); // nrg^(1/2)
                let nrg_fac14 = calc_inv_ld_data(qc.sfb_energy_ld_data[i] >> 2); // nrg^(1/4)

                // maximal number of bands is 64, results scaling factor 6
                n_lines_sum = n_lines_sum.wrapping_add(pe_data.pe_channel_data[ch].sfb_n_lines[i]); // relevant lines
                nrg_total = nrg_total.wrapping_add(qc.sfb_energy[i] >> 6); // sum up nrg
                nrg_sum12 = nrg_sum12.wrapping_add(nrg_fac12 >> 6); // sum up nrg^(2/4)
                nrg_sum14 = nrg_sum14.wrapping_add(nrg_fac14 >> 6); // sum up nrg^(1/4)
                nrg_sum34 = nrg_sum34.wrapping_add(f_mult(nrg_fac14, nrg_fac12) >> 6);
                // sum up nrg^(3/4)
            });

            nrg_total = calc_ld_data(nrg_total); // get ld64 of total nrg

            let nrg_fac_ld_14 = calc_ld_data(nrg_sum14).wrapping_sub(nrg_total); // ld64(nrgSum14/nrgTotal)
            let nrg_fac_ld_12 = calc_ld_data(nrg_sum12).wrapping_sub(nrg_total); // ld64(nrgSum12/nrgTotal)
            let nrg_fac_ld_34 = calc_ld_data(nrg_sum34).wrapping_sub(nrg_total); // ld64(nrgSum34/nrgTotal)

            // Note: nLinesSum cannot be larger than the number of total lines, thats taken care
            // of in line_pe.cpp FDKaacEnc_prepareSfbPe()
            adj_thr_state_element.chaos_measure_en_fac[ch] = f_max(
                fl2fxconst_dbl(0.1875f32 as f64),
                f_div_norm_noexp(
                    n_lines_sum,
                    psy_out_chan.sfb_offsets[psy_out_chan.sfb_cnt as usize],
                ),
            );

            let use_patch = (adj_thr_state_element.chaos_measure_en_fac[ch]
                > fl2fxconst_dbl(0.78125f32 as f64)) as i32;
            let exe_patch =
                (use_patch != 0 && adj_thr_state_element.last_en_fac_patch[ch] != 0) as i32;

            let chaos = adj_thr_state_element.chaos_measure_en_fac[ch];
            for_each_sfb(psy_out_chan, |i| {
                // for MS coupled SFBs, also execute patch in side channel if done in mid
                // channel
                let sfb_exe_patch =
                    if ch == 1 && tools_info.ms_mask[i] != 0 { exe_patch_m } else { exe_patch };

                if sfb_exe_patch != 0 && qc.sfb_energy[i] > 0 {
                    let e = qc.sfb_energy_ld_data[i];
                    // execute patch based on spectral flatness calculated above
                    if chaos > fl2fxconst_dbl(0.8125f32 as f64) {
                        // sfbEnergy^(3/4)
                        qc.sfb_en_fac_ld[i] =
                            nrg_fac_ld_14.wrapping_add(e.wrapping_add(e >> 1)) >> 1;
                    } else if chaos > fl2fxconst_dbl(0.796875f32 as f64) {
                        // sfbEnergy^(2/4)
                        qc.sfb_en_fac_ld[i] = nrg_fac_ld_12.wrapping_add(e) >> 1;
                    } else {
                        // sfbEnergy^(1/4)
                        qc.sfb_en_fac_ld[i] = nrg_fac_ld_34.wrapping_add(e >> 1) >> 1;
                    }
                    qc.sfb_en_fac_ld[i] = fix_min(qc.sfb_en_fac_ld[i], 0);
                }
            });

            adj_thr_state_element.last_en_fac_patch[ch] = use_patch;
            exe_patch_m = exe_patch;
        } else {
            // !noShortWindowInFrame
            adj_thr_state_element.chaos_measure_en_fac[ch] = fl2fxconst_dbl(0.75f32 as f64);
            // allow use of sfbEnFac patch in upcoming frame
            adj_thr_state_element.last_en_fac_patch[ch] = TRUE;
        }
    }
}

/// `FDKaacEnc_calcPe`: calculate pe for both channels (static in C; public for the differential
/// tests)
pub fn fdk_aac_enc_calc_pe(
    psy_out_channel: &[PsyOutChannel],
    qc_out_channel: &[QcOutChannel],
    pe_data: &mut PeData,
    n_channels: i32,
) {
    pe_data.pe = pe_data.offset;
    pe_data.const_part = 0;
    pe_data.n_active_lines = 0;
    for ch in 0..n_channels as usize {
        let pe_chan_data = &mut pe_data.pe_channel_data[ch];
        let psy = &psy_out_channel[ch];
        let qc = &qc_out_channel[ch];

        fdk_aac_enc_calc_sfb_pe(
            pe_chan_data,
            &qc.sfb_weighted_energy_ld_data,
            &qc.sfb_threshold_ld_data,
            psy.sfb_cnt,
            psy.sfb_per_group,
            psy.max_sfb_per_group,
            &psy.is_book,
            &psy.is_scale,
        );

        let (pe, const_part, n_active_lines) =
            (pe_chan_data.pe, pe_chan_data.const_part, pe_chan_data.n_active_lines);
        pe_data.pe = pe_data.pe.wrapping_add(pe);
        pe_data.const_part = pe_data.const_part.wrapping_add(const_part);
        pe_data.n_active_lines = pe_data.n_active_lines.wrapping_add(n_active_lines);
    }
}

/// `FDKaacEnc_peCalculation`
pub fn fdk_aac_enc_pe_calculation(
    pe_data: &mut PeData,
    psy_out_channel: &[PsyOutChannel],
    qc_out_channel: &mut [QcOutChannel],
    tools_info: &ToolsInfo,
    adj_thr_state_element: &mut AtsElement,
    n_channels: i32,
) {
    // constants that will not change during successive pe calculations
    fdk_aac_enc_prepare_pe(
        pe_data,
        psy_out_channel,
        qc_out_channel,
        n_channels,
        adj_thr_state_element.pe_offset,
    );

    // calculate weighting factor for threshold adjustment
    fdk_aac_enc_calc_weighting(
        pe_data,
        psy_out_channel,
        qc_out_channel,
        tools_info,
        adj_thr_state_element,
        n_channels,
        1,
    );
    {
        // no weighting of threholds and energies for mlout
        // weight energies and thresholds
        for ch in 0..n_channels as usize {
            let p_qc_out_ch = &mut qc_out_channel[ch];
            for_each_sfb(&psy_out_channel[ch], |i| {
                p_qc_out_ch.sfb_weighted_energy_ld_data[i] =
                    p_qc_out_ch.sfb_energy_ld_data[i].wrapping_sub(p_qc_out_ch.sfb_en_fac_ld[i]);
                p_qc_out_ch.sfb_threshold_ld_data[i] =
                    p_qc_out_ch.sfb_threshold_ld_data[i].wrapping_sub(p_qc_out_ch.sfb_en_fac_ld[i]);
            });
        }
    }

    // pe without reduction
    fdk_aac_enc_calc_pe(psy_out_channel, qc_out_channel, pe_data, n_channels);
}

/// `FDKaacEnc_FDKaacEnc_calcPeNoAH`: sum the pe data only for bands where avoid hole is
/// inactive (static in C; public for the differential tests)
pub fn fdk_aac_enc_calc_pe_no_ah(
    pe: &mut i32,
    const_part: &mut i32,
    n_active_lines: &mut i32,
    pe_data: &PeData,
    ah_flag: &AhFlag,
    psy_out_channel: &[PsyOutChannel],
    n_channels: i32,
) {
    let mut pe_tmp = pe_data.offset;
    let mut const_part_tmp: i32 = 0;
    let mut n_active_lines_tmp: i32 = 0;
    for ch in 0..n_channels as usize {
        let pe_chan_data = &pe_data.pe_channel_data[ch];
        for_each_sfb(&psy_out_channel[ch], |i| {
            if ah_flag[ch][i] < AH_ACTIVE {
                pe_tmp = pe_tmp.wrapping_add(pe_chan_data.sfb_pe[i]);
                const_part_tmp = const_part_tmp
                    .wrapping_add(pe_chan_data.sfb_const_part[i] >> CONSTPART_HEADROOM);
                n_active_lines_tmp =
                    n_active_lines_tmp.wrapping_add(pe_chan_data.sfb_n_active_lines[i]);
            }
        });
    }
    // correct scaled pe and constPart values
    *pe = pe_tmp >> PE_CONSTPART_SHIFT;
    *const_part = const_part_tmp >> (PE_CONSTPART_SHIFT - CONSTPART_HEADROOM);

    *n_active_lines = n_active_lines_tmp;
}

/// [`calc_ld_data_abs`] of every value of `x`, in place, with the vector `CalcLdData()` of
/// `fdk_aac_rs_fdk`: for `x != MINVAL_DBL` the argument of the unguarded logarithm is
/// `|x| >= 0`, where it equals `CalcLdData(|x|)` (0 gives -1.0 in both); `MINVAL_DBL` takes the
/// scalar code.
fn calc_ld_data_abs_vec(x: &mut [FixpDbl]) {
    let mut a = [0 as FixpDbl; MAX_GROUPED_SFB];
    let a = &mut a[..x.len()];
    for (a, &v) in a.iter_mut().zip(x.iter()) {
        *a = v.wrapping_abs();
    }
    let src: &[FixpDbl] = a;
    let mut out = [0 as FixpDbl; MAX_GROUPED_SFB];
    ld_data_vector(src, &mut out);
    for (v, &l) in x.iter_mut().zip(out.iter()) {
        *v = if *v == MINVAL_DBL { calc_ld_data_abs(*v) } else { l };
    }
}

/// `FDKaacEnc_reduceThresholdsCBR`: apply reduction formula (static in C; public for the
/// differential tests)
///
/// The logarithms of a channel are computed together with the vector `CalcLdData()`: the
/// bands are visited in the same order; a band's condition reads only its own `ahFlag` and
/// thresholds before this function writes them, and every value depends only on the band
/// (the groups do not overlap: `maxSfbPerGroup <= sfbPerGroup`, otherwise the straight port
/// [`fdk_aac_enc_reduce_thresholds_cbr_straight`] runs).
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_reduce_thresholds_cbr(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    ah_flag: &mut AhFlag,
    thr_exp: &ThrExp,
    n_channels: i32,
    red_val_m: FixpDbl,
    red_val_e: i8,
) {
    if psy_out_channel[..n_channels.max(0) as usize]
        .iter()
        .any(|p| p.max_sfb_per_group > p.sfb_per_group || p.sfb_cnt > MAX_GROUPED_SFB as i32)
    {
        return fdk_aac_enc_reduce_thresholds_cbr_straight(
            qc_out_channel,
            psy_out_channel,
            ah_flag,
            thr_exp,
            n_channels,
            red_val_m,
            red_val_e,
        );
    }
    let red_val_e = red_val_e as i32;
    for ch in 0..n_channels as usize {
        let qc_out_chan = &mut qc_out_channel[ch];
        // the bands that change and the arguments of their logarithms
        let mut idx = [0u8; MAX_GROUPED_SFB];
        let mut ld = [0 as FixpDbl; MAX_GROUPED_SFB];
        let mut scale = [0i32; MAX_GROUPED_SFB];
        let mut n = 0;
        for_each_sfb(&psy_out_channel[ch], |i| {
            let sfb_en_ld_data = qc_out_chan.sfb_weighted_energy_ld_data[i];
            let sfb_thr_ld_data = qc_out_chan.sfb_threshold_ld_data[i];
            let sfb_thr_exp = thr_exp[ch][i];
            if sfb_en_ld_data > sfb_thr_ld_data && ah_flag[ch][i] != AH_ACTIVE {
                let min_scale = fix_min(
                    count_leading_bits(sfb_thr_exp),
                    count_leading_bits(red_val_m) - red_val_e,
                ) - 1;
                idx[n] = i as u8;
                scale[n] = min_scale;
                ld[n] = scale_value(sfb_thr_exp, min_scale)
                    .wrapping_add(scale_value(red_val_m, red_val_e.wrapping_add(min_scale)));
                n += 1;
            }
        });
        calc_ld_data_abs_vec(&mut ld[..n]);
        for k in 0..n {
            let i = idx[k] as usize;
            let sfb_en_ld_data = qc_out_chan.sfb_weighted_energy_ld_data[i];
            let sfb_thr_ld_data = qc_out_chan.sfb_threshold_ld_data[i];
            let min_scale = scale[k];
            // 4*log( sfbThrExp + redVal )
            let mut sfb_thr_reduced_ld_data = ld[k]
                .wrapping_sub(min_scale.wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT) as u32));
            sfb_thr_reduced_ld_data = sfb_thr_reduced_ld_data.wrapping_shl(2);

            // avoid holes
            if sfb_thr_reduced_ld_data
                > qc_out_chan.sfb_min_snr_ld_data[i].wrapping_add(sfb_en_ld_data)
                && ah_flag[ch][i] != NO_AH
            {
                if qc_out_chan.sfb_min_snr_ld_data[i]
                    > fl2fxconst_dbl(-1.0).wrapping_sub(sfb_en_ld_data)
                {
                    sfb_thr_reduced_ld_data = fix_max(
                        qc_out_chan.sfb_min_snr_ld_data[i].wrapping_add(sfb_en_ld_data),
                        sfb_thr_ld_data,
                    );
                } else {
                    sfb_thr_reduced_ld_data = sfb_thr_ld_data;
                }
                ah_flag[ch][i] = AH_ACTIVE;
            }

            // minimum of 29 dB Ratio for Thresholds (see the straight port)
            let c29 = fl2fxconst_dbl(9.6336206 / LD_DATA_SCALING as f64);
            if sfb_en_ld_data as i64 > c29 as i64 - MAXVAL_DBL as i64 {
                sfb_thr_reduced_ld_data =
                    fix_max(sfb_thr_reduced_ld_data, sfb_en_ld_data.wrapping_sub(c29));
            }

            qc_out_chan.sfb_threshold_ld_data[i] = sfb_thr_reduced_ld_data;
        }
    }
}

/// `FDKaacEnc_reduceThresholdsCBR`: straight port, kept as the reference of
/// [`fdk_aac_enc_reduce_thresholds_cbr`].
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_reduce_thresholds_cbr_straight(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    ah_flag: &mut AhFlag,
    thr_exp: &ThrExp,
    n_channels: i32,
    red_val_m: FixpDbl,
    red_val_e: i8,
) {
    let red_val_e = red_val_e as i32;
    for ch in 0..n_channels as usize {
        let qc_out_chan = &mut qc_out_channel[ch];
        for_each_sfb(&psy_out_channel[ch], |i| {
            let sfb_en_ld_data = qc_out_chan.sfb_weighted_energy_ld_data[i];
            let sfb_thr_ld_data = qc_out_chan.sfb_threshold_ld_data[i];
            let sfb_thr_exp = thr_exp[ch][i];
            if sfb_en_ld_data > sfb_thr_ld_data && ah_flag[ch][i] != AH_ACTIVE {
                // threshold reduction formula:
                //  float tmp = thrExp[ch][sfb]+redVal;
                //  tmp *= tmp;
                //  sfbThrReduced = tmp*tmp;
                let min_scale = fix_min(
                    count_leading_bits(sfb_thr_exp),
                    count_leading_bits(red_val_m) - red_val_e,
                ) - 1;

                // 4*log( sfbThrExp + redVal )
                let mut sfb_thr_reduced_ld_data = calc_ld_data_abs(
                    scale_value(sfb_thr_exp, min_scale)
                        .wrapping_add(scale_value(red_val_m, red_val_e.wrapping_add(min_scale))),
                )
                .wrapping_sub(min_scale.wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT) as u32));
                sfb_thr_reduced_ld_data = sfb_thr_reduced_ld_data.wrapping_shl(2);

                // avoid holes
                if sfb_thr_reduced_ld_data
                    > qc_out_chan.sfb_min_snr_ld_data[i].wrapping_add(sfb_en_ld_data)
                    && ah_flag[ch][i] != NO_AH
                {
                    if qc_out_chan.sfb_min_snr_ld_data[i]
                        > fl2fxconst_dbl(-1.0).wrapping_sub(sfb_en_ld_data)
                    {
                        sfb_thr_reduced_ld_data = fix_max(
                            qc_out_chan.sfb_min_snr_ld_data[i].wrapping_add(sfb_en_ld_data),
                            sfb_thr_ld_data,
                        );
                    } else {
                        sfb_thr_reduced_ld_data = sfb_thr_ld_data;
                    }
                    ah_flag[ch][i] = AH_ACTIVE;
                }

                // minimum of 29 dB Ratio for Thresholds
                // C++: (sfbEnLdData + (FIXP_DBL)MAXVAL_DBL) > FL2FXCONST_DBL(9.6336206 /
                // LD_DATA_SCALING). Both reference builds compile this as the comparison
                // sfbEnLdData > FL2FXCONST_DBL(9.6336206 / 64) - MAXVAL_DBL (arm64: cmp with
                // 0x934469ec), i.e. without the wrap of the sum.
                let c29 = fl2fxconst_dbl(9.6336206 / LD_DATA_SCALING as f64);
                if sfb_en_ld_data as i64 > c29 as i64 - MAXVAL_DBL as i64 {
                    sfb_thr_reduced_ld_data =
                        fix_max(sfb_thr_reduced_ld_data, sfb_en_ld_data.wrapping_sub(c29));
                }

                qc_out_chan.sfb_threshold_ld_data[i] = sfb_thr_reduced_ld_data;
            }
        });
    }
}

/// `SCALE_FORM_FAC`: (SCALE_FORM_FAC+FORM_FAC_SHIFT) >= ld(FRAME_LENGTH)
const SCALE_FORM_FAC: i32 = 4;
/// `SCALE_NRGS`
const SCALE_NRGS: i32 = 8;
/// `SCALE_NLINES`
const SCALE_NLINES: i32 = 16;
/// `SCALE_NRGS_SQRT4`: 0.25 * SCALE_NRGS
const SCALE_NRGS_SQRT4: i32 = 2;
/// `SCALE_NLINES_P34`: 0.75 * SCALE_NLINES
const SCALE_NLINES_P34: i32 = 12;

/// `FDKaacEnc_calcChaosMeasure`: similar to prepareSfbPe1() (static in C; public for the
/// differential tests)
///
/// `sfbEnergyLdData`, `sfbThresholdLdData` and `sfbEnergy` of the `PSY_OUT_CHANNEL` are read from
/// `qc_out_channel`, `sfbFormFactorLdData` is `qc_out_channel.sfb_form_factor_ld_data`.
pub fn fdk_aac_enc_calc_chaos_measure(
    psy_out_channel: &PsyOutChannel,
    qc_out_channel: &QcOutChannel,
) -> FixpDbl {
    fdk_aac_enc_calc_chaos_measure_view(psy_out_channel, qc_out_channel, qc_out_channel)
}

/// [`fdk_aac_enc_calc_chaos_measure`] with the arrays C reads through the `PSY_OUT_CHANNEL`
/// (`sfbEnergyLdData`, `sfbThresholdLdData`, `sfbEnergy`) from `psy_view` and
/// `sfbFormFactorLdData` from `qc_out_channel`.
pub fn fdk_aac_enc_calc_chaos_measure_view(
    psy_out_channel: &PsyOutChannel,
    qc_out_channel: &QcOutChannel,
    psy_view: &QcOutChannel,
) -> FixpDbl {
    let mut frame_n_lines: i32 = 0;
    let mut frame_form_factor: FixpDbl = 0;
    let mut frame_energy: FixpDbl = 0;
    let qc = qc_out_channel;
    let pv = psy_view;

    for_each_sfb(psy_out_channel, |i| {
        if pv.sfb_energy_ld_data[i] > pv.sfb_threshold_ld_data[i] {
            frame_form_factor = frame_form_factor
                .wrapping_add(calc_inv_ld_data(qc.sfb_form_factor_ld_data[i]) >> SCALE_FORM_FAC);
            frame_n_lines = frame_n_lines.wrapping_add(
                psy_out_channel.sfb_offsets[i + 1].wrapping_sub(psy_out_channel.sfb_offsets[i]),
            );
            frame_energy = frame_energy.wrapping_add(pv.sfb_energy[i] >> SCALE_NRGS);
        }
    });

    if frame_n_lines > 0 {
        // frameNActiveLines = frameFormFactor*2^FORM_FAC_SHIFT * ((frameEnergy
        // *2^SCALE_NRGS)/frameNLines)^-0.25 chaosMeasure      = frameNActiveLines /
        // frameNLines
        let c: FixpDbl = -((-SCALE_FORM_FAC + SCALE_NRGS_SQRT4 - FORM_FAC_SHIFT
            + SCALE_NLINES_P34)
            << (DFRACT_BITS - 1 - LD_DATA_SHIFT));
        calc_inv_ld_data(
            ((calc_ld_data(frame_form_factor) >> 1)
                .wrapping_sub(calc_ld_data(frame_energy) >> (2 + 1))
                .wrapping_sub(
                    f_mult_div2(
                        fl2fxconst_dbl(0.75f32 as f64),
                        calc_ld_data(
                            frame_n_lines.wrapping_shl((DFRACT_BITS - 1 - SCALE_NLINES) as u32),
                        ),
                    )
                    .wrapping_sub(c >> 1),
                ))
            .wrapping_shl(1),
        )
    } else {
        // assuming total chaos, if no sfb is above thresholds
        fl2fxconst_dbl(1.0)
    }
}

/// `SCALE_GROUP_ENERGY`
const SCALE_GROUP_ENERGY: i32 = 8;
/// `WIN_TYPE_SCALE`
const WIN_TYPE_SCALE: i32 = 3;
/// `MIN_LDTHRESH`
const MIN_LDTHRESH: FixpDbl = fl2fxconst_dbl(-0.515625f32 as f64);

/// `FDKaacEnc_reduceThresholdsVBR`: apply reduction formula for VBR-mode (static in C; public
/// for the differential tests)
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_reduce_thresholds_vbr(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    ah_flag: &mut AhFlag,
    thr_exp: &ThrExp,
    n_channels: i32,
    vbr_qual_factor: FixpDbl,
    chaos_measure_old: &mut FixpDbl,
) {
    fdk_aac_enc_reduce_thresholds_vbr_views(
        qc_out_channel,
        psy_out_channel,
        &NO_PSY_ARRAY_VIEWS,
        ah_flag,
        thr_exp,
        n_channels,
        vbr_qual_factor,
        chaos_measure_old,
    )
}

/// [`fdk_aac_enc_reduce_thresholds_vbr`] with the arrays C reads through the `PSY_OUT_CHANNEL`
/// (`sfbEnergy`, and those of the chaos measure) from `views` where set.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_reduce_thresholds_vbr_views(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    views: &PsyArrayViews,
    ah_flag: &mut AhFlag,
    thr_exp: &ThrExp,
    n_channels: i32,
    vbr_qual_factor: FixpDbl,
    chaos_measure_old: &mut FixpDbl,
) {
    let n = n_channels as usize;
    // energy for each group and channel
    let mut ch_group_energy: [[FixpDbl; 2]; TRANS_FAC as usize] = [[0; 2]; TRANS_FAC as usize];
    let mut ch_chaos_measure: [FixpDbl; 2] = [0; 2];
    let mut frame_energy: FixpDbl = fl2fxconst_dbl(1e-10f32 as f64);
    let mut chaos_measure: FixpDbl = 0;
    // reduction values; in short-block case one redVal for each group
    let mut red_val: [FixpDbl; TRANS_FAC as usize] = [0; TRANS_FAC as usize];

    const CONST_CHAOS_MEAS_AVG_FAC_0: FixpDbl = fl2fxconst_dbl(0.25f32 as f64);
    const CONST_CHAOS_MEAS_AVG_FAC_1: FixpDbl = fl2fxconst_dbl((1.0f32 - 0.25f32) as f64);

    for ch in 0..n {
        let psy_out_chan = &psy_out_channel[ch];
        let qc = &qc_out_channel[ch];
        // the arrays of psyOutChannel[ch]
        let pv = views.get(ch).and_then(|v| v.as_deref()).unwrap_or(qc);

        // adding up energy for each channel and each group separately
        let mut ch_energy: FixpDbl = 0;
        let mut group_cnt = 0usize;

        let mut sfb_grp = 0;
        while sfb_grp < psy_out_chan.sfb_cnt {
            ch_group_energy[group_cnt][ch] = 0;
            for sfb in 0..psy_out_chan.max_sfb_per_group {
                ch_group_energy[group_cnt][ch] = ch_group_energy[group_cnt][ch]
                    .wrapping_add(pv.sfb_energy[(sfb_grp + sfb) as usize] >> SCALE_GROUP_ENERGY);
            }
            ch_energy = ch_energy.wrapping_add(ch_group_energy[group_cnt][ch]);
            sfb_grp += psy_out_chan.sfb_per_group;
            group_cnt += 1;
        }
        frame_energy = frame_energy.wrapping_add(ch_energy);

        // chaosMeasure
        if psy_out_channel[0].last_window_sequence == SHORT_WINDOW {
            // assume a constant chaos measure of 0.5f for short blocks
            ch_chaos_measure[ch] = fl2fxconst_dbl(0.5f32 as f64);
        } else {
            ch_chaos_measure[ch] = fdk_aac_enc_calc_chaos_measure_view(psy_out_chan, qc, pv);
        }
        chaos_measure = chaos_measure.wrapping_add(f_mult(ch_chaos_measure[ch], ch_energy));
    }

    if frame_energy > chaos_measure {
        let scale = cnt_leading_zeros(frame_energy) - 1;
        let num = chaos_measure.wrapping_shl(scale as u32);
        let denum = frame_energy.wrapping_shl(scale as u32);
        chaos_measure = schur_div(num, denum, 16);
    } else {
        chaos_measure = fl2fxconst_dbl(1.0);
    }

    // averaging chaos measure
    let chaos_measure_avg = f_mult(CONST_CHAOS_MEAS_AVG_FAC_0, chaos_measure)
        .wrapping_add(f_mult(CONST_CHAOS_MEAS_AVG_FAC_1, *chaos_measure_old));
    // use min-value, safe for next frame
    chaos_measure = fix_min(chaos_measure, chaos_measure_avg);
    *chaos_measure_old = chaos_measure;

    // characteristic curve
    //   chaosMeasure = 0.2f + 0.7f/0.3f * (chaosMeasure - 0.2f);
    //   chaosMeasure = fixMin(1.0f, fixMax(0.1f, chaosMeasure));
    //   constants scaled by 4.f
    chaos_measure = (fl2fxconst_dbl(0.2f32 as f64) >> 2).wrapping_add(f_mult(
        fl2fxconst_dbl((0.7f32 / (4.0f32 * 0.3f32)) as f64),
        chaos_measure.wrapping_sub(fl2fxconst_dbl(0.2f32 as f64)),
    ));
    chaos_measure = fix_min(
        fl2fxconst_dbl(1.0f32 as f64) >> 2,
        fix_max(fl2fxconst_dbl(0.1f32 as f64) >> 2, chaos_measure),
    ) << 2;

    // calculation of reduction value
    if psy_out_channel[0].last_window_sequence == SHORT_WINDOW {
        // short-blocks
        fdk_assert!(TRANS_FAC == 8);

        let p0 = &psy_out_channel[0];
        let mut group_cnt = 0usize;
        let mut sfb_grp = 0;
        while sfb_grp < p0.sfb_cnt {
            let mut group_energy: FixpDbl = 0;

            for ch in 0..n {
                // adding up the channels groupEnergy
                group_energy = group_energy.wrapping_add(ch_group_energy[group_cnt][ch]);
            }

            fdk_assert!(p0.group_len[group_cnt] <= INV_INT_TAB_SIZE);
            // correction of group energy
            // Bounded: indices outside invInt[] read 0.
            group_energy = f_mult(
                group_energy,
                INV_INT.get(p0.group_len[group_cnt] as usize).copied().unwrap_or(0),
            );
            // do not allow an higher redVal as calculated framewise
            group_energy = fix_min(group_energy, frame_energy >> WIN_TYPE_SCALE);

            group_energy >>= 2; // 2*WIN_TYPE_SCALE = 6 => 6+2 = 8 ==> 8/4 = int number

            red_val[group_cnt] = f_mult(
                f_mult(vbr_qual_factor, chaos_measure),
                calc_inv_ld_data(calc_ld_data(group_energy) >> 2),
            )
            .wrapping_shl(((2 + (2 * WIN_TYPE_SCALE) + SCALE_GROUP_ENERGY) >> 2) as u32);

            sfb_grp += p0.sfb_per_group;
            group_cnt += 1;
        }
    } else {
        // long-block
        red_val[0] = f_mult(
            f_mult(vbr_qual_factor, chaos_measure),
            calc_inv_ld_data(calc_ld_data(frame_energy) >> 2),
        )
        .wrapping_shl((SCALE_GROUP_ENERGY >> 2) as u32);
    }

    for ch in 0..n {
        let qc_out_chan = &mut qc_out_channel[ch];
        let psy_out_chan = &psy_out_channel[ch];

        let mut sfb_grp = 0;
        while sfb_grp < psy_out_chan.sfb_cnt {
            for sfb in 0..psy_out_chan.max_sfb_per_group {
                let i = (sfb_grp + sfb) as usize;
                let sfb_en_ld_data = qc_out_chan.sfb_weighted_energy_ld_data[i];
                let sfb_thr_ld_data = qc_out_chan.sfb_threshold_ld_data[i];
                let mut sfb_thr_exp = thr_exp[ch][i];
                let mut sfb_thr_reduced_ld_data: FixpDbl;

                if sfb_thr_ld_data >= MIN_LDTHRESH
                    && sfb_en_ld_data > sfb_thr_ld_data
                    && ah_flag[ch][i] != AH_ACTIVE
                {
                    // Short-Window
                    if psy_out_chan.last_window_sequence == SHORT_WINDOW {
                        // Quirk: sfb < maxSfbPerGroup <= sfbPerGroup, so this is always group 0.
                        let group_number = (sfb / psy_out_chan.sfb_per_group) as usize;

                        fdk_assert!(INV_SQRT4_TAB_SIZE > psy_out_chan.group_len[group_number]);

                        sfb_thr_exp = f_mult(
                            sfb_thr_exp,
                            f_mult(
                                fl2fxconst_dbl((2.82f32 / 4.0f32) as f64),
                                // Bounded: indices outside invSqrt4[] read 0.
                                INV_SQRT_4
                                    .get(psy_out_chan.group_len[group_number] as usize)
                                    .copied()
                                    .unwrap_or(0),
                            ),
                        )
                        .wrapping_shl(2);

                        if sfb_thr_exp
                            <= LIMIT_THR_REDUCED_LD_DATA.wrapping_sub(red_val[group_number])
                        {
                            sfb_thr_reduced_ld_data = fl2fxconst_dbl(-1.0);
                        } else if red_val[group_number]
                            >= fl2fxconst_dbl(1.0).wrapping_sub(sfb_thr_exp)
                        {
                            sfb_thr_reduced_ld_data = 0;
                        } else {
                            // threshold reduction formula
                            // C++: CalcLdData(sfbThrExp + redVal[groupNumber]). After the test
                            // sfbThrExp > limitThrReducedLdData - redVal, both reference builds
                            // drop the test `x <= 0` of fLog2() (the sum is positive without
                            // overflow) and take the logarithm of the wrapped sum.
                            sfb_thr_reduced_ld_data = calc_ld_data_unguarded(
                                sfb_thr_exp.wrapping_add(red_val[group_number]),
                            );
                            sfb_thr_reduced_ld_data = sfb_thr_reduced_ld_data.wrapping_shl(2);
                        }
                        sfb_thr_reduced_ld_data = sfb_thr_reduced_ld_data.wrapping_add(
                            calc_ld_int(psy_out_chan.group_len[group_number])
                                .wrapping_sub(6 << (DFRACT_BITS - 1 - LD_DATA_SHIFT)),
                        );
                    }
                    // Long-Window
                    else if red_val[0] >= fl2fxconst_dbl(1.0).wrapping_sub(sfb_thr_exp) {
                        sfb_thr_reduced_ld_data = 0;
                    } else {
                        // threshold reduction formula
                        // C++: CalcLdData(sfbThrExp + redVal[0]). arm64 tests the wrapped sum
                        // (add, cmp #1, b.lt); x86 tests the sum without overflow (addl, jle:
                        // SF != OF) and then takes the logarithm of the wrapped sum.
                        let sum = sfb_thr_exp.wrapping_add(red_val[0]);
                        sfb_thr_reduced_ld_data = if matches!(ARCH, Arch::X86 | Arch::X86_32) {
                            if sfb_thr_exp as i64 + red_val[0] as i64 <= 0 {
                                fl2fxconst_dbl(-1.0)
                            } else {
                                calc_ld_data_unguarded(sum)
                            }
                        } else {
                            calc_ld_data(sum)
                        };
                        sfb_thr_reduced_ld_data = sfb_thr_reduced_ld_data.wrapping_shl(2);
                    }

                    // avoid holes
                    if sfb_thr_reduced_ld_data.wrapping_sub(sfb_en_ld_data)
                        > qc_out_chan.sfb_min_snr_ld_data[i]
                        && ah_flag[ch][i] != NO_AH
                    {
                        if qc_out_chan.sfb_min_snr_ld_data[i]
                            > fl2fxconst_dbl(-1.0).wrapping_sub(sfb_en_ld_data)
                        {
                            sfb_thr_reduced_ld_data = fix_max(
                                qc_out_chan.sfb_min_snr_ld_data[i].wrapping_add(sfb_en_ld_data),
                                sfb_thr_ld_data,
                            );
                        } else {
                            sfb_thr_reduced_ld_data = sfb_thr_ld_data;
                        }
                        ah_flag[ch][i] = AH_ACTIVE;
                    }

                    if sfb_thr_reduced_ld_data < fl2fxconst_dbl(-0.5f32 as f64) {
                        sfb_thr_reduced_ld_data = fl2fxconst_dbl(-1.0f32 as f64);
                    }

                    sfb_thr_reduced_ld_data = fix_max(MIN_LDTHRESH, sfb_thr_reduced_ld_data);

                    qc_out_chan.sfb_threshold_ld_data[i] = sfb_thr_reduced_ld_data;
                }
            }
            sfb_grp += psy_out_chan.sfb_per_group;
        }
    }
}

/// Range of the channels of element `el` in `QcOut::p_qc_out_channels`.
#[inline]
fn qc_range(
    qc_element: &[QcOutElement],
    cm: &ChannelMapping,
    el: usize,
) -> core::ops::Range<usize> {
    let s = qc_element[el].qc_out_channel[0];
    s..s + cm.el_info[el].n_channels_in_el as usize
}

/// Range of the channels of element `el` in `PsyOut::p_psy_out_channels`.
#[inline]
fn psy_range(psy_out: &PsyOut, cm: &ChannelMapping, el: usize) -> core::ops::Range<usize> {
    let s = psy_out.psy_out_element[el].psy_out_channel[0];
    s..s + cm.el_info[el].n_channels_in_el as usize
}

/// The arrays that C reads through the six array pointers of the `PSY_OUT_CHANNEL`s of an
/// element (`psyOutChannel[ch]->sfbEnergy` etc.), per channel: `None` for the element's own
/// `QC_OUT_CHANNEL` (the normal case), otherwise a copy of the other `QC_OUT_CHANNEL` they point
/// to (see the [`interface`](crate::interface) module documentation). A copy is exact: the
/// readers do not write through these pointers, and the other element's channel does not change
/// during the call.
pub type PsyArrayViews = [Option<Box<QcOutChannel>>; 2];

/// No views: every channel reads its own `QC_OUT_CHANNEL`.
pub const NO_PSY_ARRAY_VIEWS: PsyArrayViews = [None, None];

/// The [`PsyArrayViews`] of the channels `psy` (in `PsyOut::p_psy_out_channels`) whose own
/// `QC_OUT_CHANNEL`s are `qc` (in `QcOut::p_qc_out_channels`).
pub fn psy_array_views(
    psy_out: &PsyOut,
    qc_out: &QcOut,
    psy: core::ops::Range<usize>,
    qc: core::ops::Range<usize>,
) -> PsyArrayViews {
    let mut v = NO_PSY_ARRAY_VIEWS;
    for (k, (p, q)) in psy.zip(qc).take(2).enumerate() {
        if let Some(&a) = psy_out.p_array_qc.get(p) {
            if a != q {
                v[k] = qc_out.p_qc_out_channels.get(a).map(|c| Box::new(*c));
            }
        }
    }
    v
}

/// Stores `sfbPeFactorsLdData[elementId][ch][i]`, which C keeps in the `quantSpec` array of the
/// channel (`FIXP_DBL` entry `i` = `SHORT`s `2 * i`, `2 * i + 1`, little-endian).
#[inline]
fn set_pe_factor(qc: &mut QcOutChannel, i: usize, v: FixpDbl) {
    qc.quant_spec[2 * i] = v as i16;
    qc.quant_spec[2 * i + 1] = (v >> 16) as i16;
}

/// Reads `sfbPeFactorsLdData[elementId][ch][i]` from `quantSpec` (see [`set_pe_factor`]).
#[inline]
fn get_pe_factor(qc: &QcOutChannel, i: usize) -> FixpDbl {
    (qc.quant_spec[2 * i] as u16 as i32) | ((qc.quant_spec[2 * i + 1] as i32) << 16)
}

/// `FDKaacEnc_correctThresh`: if pe difference deltaPe between desired pe and real pe is small
/// enough, the difference can be distributed among the scale factor bands. New thresholds can
/// be derived from this pe-difference (static in C; public for the differential tests)
///
/// `ahFlag`/`thrExp` are indexed by element (`[8]` in C). `sfbPeFactorsLdData` is written into
/// the `quantSpec` arrays of the channels, as in C; `sfb_n_active_lines_ld_data` is
/// `dynMem_SfbNActiveLinesLdData` (see [`AdjThrDynMem`]).
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_correct_thresh(
    cm: &ChannelMapping,
    qc_out: &mut QcOut,
    psy_out: &PsyOut,
    ah_flag: &mut [AhFlag],
    thr_exp: &[ThrExp],
    red_val_m: FixpDbl,
    red_val_e: i8,
    delta_pe: i32,
    process_elements: i32,
    element_offset: i32,
    sfb_n_active_lines_ld_data: &mut [[[FixpDbl; MAX_GROUPED_SFB]; 2]; 8],
) {
    let red_val_e = red_val_e as i32;
    let n_elements = element_offset + process_elements;
    let QcOut { qc_element, p_qc_out_channels, .. } = qc_out;

    // sfbNActiveLinesLdData lives in the shared dynamic RAM (dynMem_SfbNActiveLinesLdData) in
    // C; every entry that the second loop reads is written by the first loop (same element,
    // channel and band ranges, and the second loop reads only bands with sfbNActiveLines > 0).

    // for each sfb calc relative factors for pe changes
    let mut norm_factor_int: i32 = 0;

    for element_id in element_offset as usize..n_elements as usize {
        if cm.el_info[element_id].el_type != ID_DSE {
            let qr = qc_range(qc_element, cm, element_id);
            let pr = psy_range(psy_out, cm, element_id);
            for ch in 0..cm.el_info[element_id].n_channels_in_el as usize {
                let psy_out_chan = &psy_out.p_psy_out_channels[pr.start + ch];
                let pe_chan_data: &PeChannelData =
                    &qc_element[element_id].pe_data.pe_channel_data[ch];
                let qc = &mut p_qc_out_channels[qr.start + ch];
                let nal = &mut sfb_n_active_lines_ld_data[element_id][ch];
                let ahf = &ah_flag[element_id][ch];
                let te = &thr_exp[element_id][ch];

                for_each_sfb(psy_out_chan, |i| {
                    if pe_chan_data.sfb_n_active_lines[i] == 0 {
                        nal[i] = fl2fxconst_dbl(-1.0);
                    } else {
                        // Both CalcLdInt and CalcLdData can be used!
                        // No offset has to be subtracted, because sfbNActiveLinesLdData is
                        // shorted while thrFactor calculation
                        nal[i] = calc_ld_int(pe_chan_data.sfb_n_active_lines[i]);
                    }
                    if (ah_flag_lt_active(ahf[i]) || delta_pe > 0)
                        && pe_chan_data.sfb_n_active_lines[i] != 0
                    {
                        if te[i] > red_val_m.wrapping_neg() {
                            // sfbPeFactors[ch][sfbGrp+sfb] =
                            //    peChanData->sfbNActiveLines[sfbGrp+sfb] /
                            //                   (thrExp[elementId][ch][sfbGrp+sfb] +
                            //    redVal[elementId]);

                            let min_scale = fix_min(
                                count_leading_bits(te[i]),
                                count_leading_bits(red_val_m) - red_val_e,
                            ) - 1;

                            // sumld = ld64( sfbThrExp + redVal )
                            let sum_ld = calc_ld_data(scale_value(te[i], min_scale).wrapping_add(
                                scale_value(red_val_m, red_val_e.wrapping_add(min_scale)),
                            ))
                            .wrapping_sub(
                                min_scale.wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT) as u32),
                            );

                            let f = if sum_ld < 0 {
                                nal[i].wrapping_sub(sum_ld)
                            } else if nal[i] > fl2fxconst_dbl(-1.0).wrapping_add(sum_ld) {
                                nal[i].wrapping_sub(sum_ld)
                            } else {
                                nal[i]
                            };
                            set_pe_factor(qc, i, f);

                            norm_factor_int = norm_factor_int.wrapping_add(calc_inv_ld_data(f));
                        } else {
                            set_pe_factor(qc, i, fl2fxconst_dbl(1.0));
                        }
                    } else {
                        set_pe_factor(qc, i, fl2fxconst_dbl(-1.0));
                    }
                });
            }
        }
    }

    // normFactorLdData = ld64(deltaPe/normFactorInt)
    let norm_factor_ld_data =
        calc_ld_data(if delta_pe < 0 { delta_pe.wrapping_neg() } else { delta_pe })
            .wrapping_sub(calc_ld_data(norm_factor_int));

    // distribute the pe difference to the scalefactors and calculate the according thresholds
    for element_id in element_offset as usize..n_elements as usize {
        if cm.el_info[element_id].el_type != ID_DSE {
            let qr = qc_range(qc_element, cm, element_id);
            let pr = psy_range(psy_out, cm, element_id);
            for ch in 0..cm.el_info[element_id].n_channels_in_el as usize {
                let qc_out_chan = &mut p_qc_out_channels[qr.start + ch];
                let psy_out_chan = &psy_out.p_psy_out_channels[pr.start + ch];
                let pe_chan_data = &qc_element[element_id].pe_data.pe_channel_data[ch];
                let nal = &sfb_n_active_lines_ld_data[element_id][ch];
                let ahf = &mut ah_flag[element_id][ch];

                for_each_sfb(psy_out_chan, |i| {
                    if pe_chan_data.sfb_n_active_lines[i] > 0 {
                        // pe difference for this sfb
                        let thr_factor_ld_data = if get_pe_factor(qc_out_chan, i)
                            == fl2fxconst_dbl(-1.0)
                            || delta_pe == 0
                        {
                            0
                        } else {
                            // new threshold
                            // C++: CalcInvLdData(sfbPeFactorsLdData + normFactorLdData -
                            // sfbNActiveLinesLdData - FL2FXCONST_DBL((float)LD_DATA_SHIFT /
                            // LD_DATA_SCALING)); the constant is folded into the range checks
                            // (arm64: cmp with 0x0c000000, 0xcdffffff, 0x49ffffff).
                            let mut tmp = calc_inv_ld_data_sub(
                                get_pe_factor(qc_out_chan, i)
                                    .wrapping_add(norm_factor_ld_data)
                                    .wrapping_sub(nal[i]),
                                fl2fxconst_dbl((LD_DATA_SHIFT as f32 / LD_DATA_SCALING) as f64),
                            );

                            // limit thrFactor to 60dB
                            tmp = if delta_pe < 0 { tmp } else { tmp.wrapping_neg() };
                            f_min(tmp, fl2fxconst_dbl((20.0f32 / LD_DATA_SCALING) as f64))
                        };

                        // new threshold
                        let sfb_thr_ld_data = qc_out_chan.sfb_threshold_ld_data[i];
                        let sfb_en_ld_data = qc_out_chan.sfb_weighted_energy_ld_data[i];

                        let mut sfb_thr_reduced_ld_data = if thr_factor_ld_data < 0 {
                            if sfb_thr_ld_data
                                > fl2fxconst_dbl(-1.0).wrapping_sub(thr_factor_ld_data)
                            {
                                sfb_thr_ld_data.wrapping_add(thr_factor_ld_data)
                            } else {
                                fl2fxconst_dbl(-1.0)
                            }
                        } else {
                            sfb_thr_ld_data.wrapping_add(thr_factor_ld_data)
                        };

                        // avoid hole
                        if sfb_thr_reduced_ld_data.wrapping_sub(sfb_en_ld_data)
                            > qc_out_chan.sfb_min_snr_ld_data[i]
                            && ahf[i] == AH_INACTIVE
                        {
                            // sfbThrReduced = max(psyOutChan[ch]->sfbMinSnr[i] * sfbEn, sfbThr);
                            if sfb_en_ld_data
                                > sfb_thr_ld_data.wrapping_sub(qc_out_chan.sfb_min_snr_ld_data[i])
                            {
                                sfb_thr_reduced_ld_data =
                                    qc_out_chan.sfb_min_snr_ld_data[i].wrapping_add(sfb_en_ld_data);
                            } else {
                                sfb_thr_reduced_ld_data = sfb_thr_ld_data;
                            }
                            ahf[i] = AH_ACTIVE;
                        }

                        qc_out_chan.sfb_threshold_ld_data[i] = sfb_thr_reduced_ld_data;
                    }
                });
            }
        }
    }
}

#[inline(always)]
fn ah_flag_lt_active(f: u8) -> bool {
    f < AH_ACTIVE
}

/// `FDKaacEnc_reduceMinSnr`: if the desired pe can not be reached, reduce pe by reducing minSnr
/// (static in C; public for the differential tests)
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_reduce_min_snr(
    cm: &ChannelMapping,
    qc_out: &mut QcOut,
    psy_out: &PsyOut,
    ah_flag: &[AhFlag],
    desired_pe: i32,
    red_pe_global: &mut i32,
    process_elements: i32,
    element_offset: i32,
) {
    let n_elements = element_offset + process_elements;
    let mut new_global_pe = *red_pe_global;
    let mut global_max_sfb: i32 = 0;
    let QcOut { qc_element, p_qc_out_channels, .. } = qc_out;

    'bail: {
        if new_global_pe <= desired_pe {
            break 'bail;
        }

        // global maximum of maxSfbPerGroup
        for element_id in element_offset as usize..n_elements as usize {
            if cm.el_info[element_id].el_type != ID_DSE {
                let pr = psy_range(psy_out, cm, element_id);
                for ch in 0..cm.el_info[element_id].n_channels_in_el as usize {
                    global_max_sfb = f_max(
                        global_max_sfb,
                        psy_out.p_psy_out_channels[pr.start + ch].max_sfb_per_group,
                    );
                }
            }
        }

        // as long as globalPE is above desirePE reduce SNR to 1.0 dB, starting at highest SFB
        while new_global_pe > desired_pe && {
            global_max_sfb -= 1;
            global_max_sfb >= 0
        } {
            for element_id in element_offset as usize..n_elements as usize {
                if cm.el_info[element_id].el_type != ID_DSE {
                    let qr = qc_range(qc_element, cm, element_id);
                    let pr = psy_range(psy_out, cm, element_id);
                    let pe_data = &mut qc_element[element_id].pe_data;

                    for ch in 0..cm.el_info[element_id].n_channels_in_el as usize {
                        let qc_out_chan = &mut p_qc_out_channels[qr.start + ch];
                        let psy_out_chan = &psy_out.p_psy_out_channels[pr.start + ch];

                        // try to reduce SNR of channel's uppermost SFB(s)
                        if global_max_sfb < psy_out_chan.max_sfb_per_group {
                            let mut delta_pe: i32 = 0;

                            let mut sfb = global_max_sfb;
                            while sfb < psy_out_chan.sfb_cnt {
                                let s = sfb as usize;
                                if ah_flag[element_id][ch][s] != NO_AH
                                    && qc_out_chan.sfb_min_snr_ld_data[s] < SNR_LD_FAC
                                    && qc_out_chan.sfb_weighted_energy_ld_data[s]
                                        > qc_out_chan.sfb_threshold_ld_data[s]
                                            .wrapping_sub(SNR_LD_FAC)
                                {
                                    // increase threshold to new minSnr of 1dB
                                    qc_out_chan.sfb_min_snr_ld_data[s] = SNR_LD_FAC;
                                    qc_out_chan.sfb_threshold_ld_data[s] = qc_out_chan
                                        .sfb_weighted_energy_ld_data[s]
                                        .wrapping_add(SNR_LD_FAC);

                                    // calc new pe
                                    // C2 + C3*ld(1/0.8) = 1.5
                                    let pcd = &mut pe_data.pe_channel_data[ch];
                                    delta_pe = delta_pe.wrapping_sub(pcd.sfb_pe[s]);

                                    // sfbPe = 1.5 * sfbNLines
                                    pcd.sfb_pe[s] = (3i32.wrapping_mul(pcd.sfb_n_lines[s]))
                                        .wrapping_shl((PE_CONSTPART_SHIFT - 1) as u32);
                                    delta_pe = delta_pe.wrapping_add(pcd.sfb_pe[s]);
                                }
                                sfb += psy_out_chan.sfb_per_group;
                            }

                            delta_pe >>= PE_CONSTPART_SHIFT;
                            pe_data.pe = pe_data.pe.wrapping_add(delta_pe);
                            pe_data.pe_channel_data[ch].pe =
                                pe_data.pe_channel_data[ch].pe.wrapping_add(delta_pe);
                            new_global_pe = new_global_pe.wrapping_add(delta_pe);
                        }

                        // stop if enough has been saved
                        if new_global_pe <= desired_pe {
                            break 'bail;
                        }
                    }
                }
            }
        }
    }

    // update global PE
    *red_pe_global = new_global_pe;
}

thread_local! {
    static STALE_AH_FLAG_READS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn adj_thr_stale_ah_flag_reads() -> u64 {
    STALE_AH_FLAG_READS.with(|c| c.get())
}

/// Adds to the counter of [`adj_thr_stale_ah_flag_reads`] (the carry of the thread pool).
pub(crate) fn add_adj_thr_stale_ah_flag_reads(n: u64) {
    STALE_AH_FLAG_READS.with(|c| c.set(c.get().wrapping_add(n)));
}

/// Whether `FDKaacEnc_initAvoidHoleFlag()` writes the avoid-hole flag of band `i` (grouped
/// index) of a channel `p`: `i` lies in a group (`sfbGrp` from 0 below `sfbCnt` in steps of
/// `sfbPerGroup`) below `maxSfbPerGroup` of it.
fn ah_flag_written(p: &PsyOutChannel, i: i32) -> bool {
    if p.sfb_per_group <= 0 {
        // (not defined by the reference)
        return false;
    }
    let grp = i.div_euclid(p.sfb_per_group) * p.sfb_per_group;
    i >= 0 && grp < p.sfb_cnt && i - grp < p.max_sfb_per_group
}

/// `FDKaacEnc_allowMoreHoles`: if the desired pe can not be reached, some more scalefactor
/// bands have to be quantized to zero (static in C; public for the differential tests)
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_allow_more_holes(
    cm: &ChannelMapping,
    qc_out: &mut QcOut,
    psy_out: &PsyOut,
    adj_thr_state_element: &[AtsElement],
    ah_flag: &mut [AhFlag],
    desired_pe: i32,
    current_pe: i32,
    process_elements: i32,
    element_offset: i32,
) {
    let n_elements = element_offset + process_elements;
    let mut act_pe = current_pe;
    let QcOut { qc_element, p_qc_out_channels, .. } = qc_out;

    if act_pe <= desired_pe {
        return; // nothing to do
    }

    for element_id in element_offset as usize..n_elements as usize {
        if cm.el_info[element_id].el_type != ID_DSE {
            let n_channels = cm.el_info[element_id].n_channels_in_el;
            let qr = qc_range(qc_element, cm, element_id);
            let pe_data = &mut qc_element[element_id].pe_data;
            let pr = psy_range(psy_out, cm, element_id);
            let psy_out_channel = &psy_out.p_psy_out_channels[pr];
            let qc_out_channel = &mut p_qc_out_channels[qr];

            for ch in 0..n_channels as usize {
                let p = &psy_out_channel[ch];
                let mut sfb_grp = 0;
                while sfb_grp < p.sfb_cnt {
                    for sfb in p.max_sfb_per_group..p.sfb_per_group {
                        pe_data.pe_channel_data[ch].sfb_pe[(sfb_grp + sfb) as usize] = 0;
                    }
                    sfb_grp += p.sfb_per_group;
                }
            }

            // for MS allow hole in the channel with less energy
            if n_channels == 2
                && psy_out_channel[0].last_window_sequence
                    == psy_out_channel[1].last_window_sequence
            {
                let p0 = &psy_out_channel[0];
                let mut sfb = p0.max_sfb_per_group - 1;
                while sfb >= 0 {
                    let mut sfb_grp = 0;
                    while sfb_grp < p0.sfb_cnt {
                        let i = (sfb_grp + sfb) as usize;
                        if psy_out.psy_out_element[element_id].tools_info.ms_mask[i] != 0 {
                            let energy_ld_l = qc_out_channel[0].sfb_weighted_energy_ld_data[i];
                            let energy_ld_r = qc_out_channel[1].sfb_weighted_energy_ld_data[i];

                            // Counted where the reference has no defined result (see
                            // adj_thr_stale_ah_flag_reads; the port's flags there are 0).
                            if !ah_flag_written(&psy_out_channel[1], i as i32) {
                                add_adj_thr_stale_ah_flag_reads(1);
                            }

                            // allow hole in side channel ?
                            if ah_flag[element_id][1][i] != NO_AH
                                && (fl2fxconst_dbl(-0.02065512648f32 as f64) >> 1)
                                    .wrapping_add(qc_out_channel[0].sfb_min_snr_ld_data[i] >> 1)
                                    > (energy_ld_r >> 1).wrapping_sub(energy_ld_l >> 1)
                            {
                                ah_flag[element_id][1][i] = NO_AH;
                                qc_out_channel[1].sfb_threshold_ld_data[i] =
                                    fl2fxconst_dbl(0.015625f32 as f64).wrapping_add(energy_ld_r);
                                act_pe = act_pe.wrapping_sub(
                                    pe_data.pe_channel_data[1].sfb_pe[i] >> PE_CONSTPART_SHIFT,
                                );
                            }
                            // allow hole in mid channel ?
                            else if ah_flag[element_id][0][i] != NO_AH
                                && (fl2fxconst_dbl(-0.02065512648f32 as f64) >> 1)
                                    .wrapping_add(qc_out_channel[1].sfb_min_snr_ld_data[i] >> 1)
                                    > (energy_ld_l >> 1).wrapping_sub(energy_ld_r >> 1)
                            {
                                ah_flag[element_id][0][i] = NO_AH;
                                qc_out_channel[0].sfb_threshold_ld_data[i] =
                                    fl2fxconst_dbl(0.015625f32 as f64).wrapping_add(energy_ld_l);
                                act_pe = act_pe.wrapping_sub(
                                    pe_data.pe_channel_data[0].sfb_pe[i] >> PE_CONSTPART_SHIFT,
                                );
                            }
                        }
                        sfb_grp += p0.sfb_per_group;
                    }
                    if act_pe <= desired_pe {
                        return; // stop if enough has been saved
                    }
                    sfb -= 1;
                }
            }
        }
    }

    if act_pe > desired_pe {
        // more holes necessary? subsequently erase bands starting with low energies
        let mut start_sfb = [0i32; 8];
        let mut sfb_cnt = [0i32; 8];
        let mut sfb_per_group = [0i32; 8];
        let mut max_sfb_per_group = [0i32; 8];
        let mut en_ld64 = [0 as FixpDbl; NUM_NRG_LEVS];

        // get the scaling factor over all audio elements and channels
        let mut max_sfb: i32 = 0;
        for element_id in element_offset as usize..n_elements as usize {
            if cm.el_info[element_id].el_type != ID_DSE {
                let pr = psy_range(psy_out, cm, element_id);
                for ch in 0..cm.el_info[element_id].n_channels_in_el as usize {
                    let p = &psy_out.p_psy_out_channels[pr.start + ch];
                    let mut sfb_grp = 0;
                    while sfb_grp < p.sfb_cnt {
                        max_sfb += p.max_sfb_per_group;
                        sfb_grp += p.sfb_per_group;
                    }
                }
            }
        }
        // ilog2()
        let avg_en_e = DFRACT_BITS - fixnormz_d(f_max(0, max_sfb - 1));

        let mut ah_cnt: i32 = 0;
        max_sfb = 0;
        let mut min_sfb: i32 = MAX_SFB as i32;
        let mut avg_en: FixpDbl = 0;
        let mut min_en_ld64: FixpDbl = 0;

        for element_id in element_offset as usize..n_elements as usize {
            if cm.el_info[element_id].el_type != ID_DSE {
                let qr = qc_range(qc_element, cm, element_id);
                let pr = psy_range(psy_out, cm, element_id);
                for ch in 0..cm.el_info[element_id].n_channels_in_el as usize {
                    let ch_idx = cm.el_info[element_id].channel_index[ch] as usize;
                    let qc_out_channel = &p_qc_out_channels[qr.start + ch];
                    let psy_out_channel = &psy_out.p_psy_out_channels[pr.start + ch];

                    max_sfb_per_group[ch_idx] = psy_out_channel.max_sfb_per_group;
                    sfb_cnt[ch_idx] = psy_out_channel.sfb_cnt;
                    sfb_per_group[ch_idx] = psy_out_channel.sfb_per_group;

                    max_sfb = f_max(max_sfb, psy_out_channel.max_sfb_per_group);

                    if psy_out_channel.last_window_sequence != SHORT_WINDOW {
                        start_sfb[ch_idx] = adj_thr_state_element[element_id].ah_param.start_sfb_l;
                    } else {
                        start_sfb[ch_idx] = adj_thr_state_element[element_id].ah_param.start_sfb_s;
                    }

                    min_sfb = f_min(min_sfb, start_sfb[ch_idx]);

                    let mut sfb_grp = 0;
                    let mut sfb = start_sfb[ch_idx];

                    loop {
                        while sfb < psy_out_channel.max_sfb_per_group {
                            let i = (sfb_grp + sfb) as usize;
                            if ah_flag[element_id][ch][i] != NO_AH
                                && qc_out_channel.sfb_weighted_energy_ld_data[i]
                                    > qc_out_channel.sfb_threshold_ld_data[i]
                            {
                                min_en_ld64 =
                                    fix_min(min_en_ld64, qc_out_channel.sfb_energy_ld_data[i]);
                                avg_en = avg_en.wrapping_add(
                                    qc_out_channel.sfb_energy[i].wrapping_shr(avg_en_e as u32),
                                );
                                ah_cnt += 1;
                            }
                            sfb += 1;
                        }

                        sfb_grp += psy_out_channel.sfb_per_group;
                        sfb = start_sfb[ch_idx];

                        if sfb_grp >= psy_out_channel.sfb_cnt {
                            break;
                        }
                    }
                }
            }
        }

        let avg_en_ld64 = if avg_en == 0 || ah_cnt == 0 {
            0
        } else {
            calc_ld_data(avg_en)
                .wrapping_add(avg_en_e.wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT) as u32))
                .wrapping_sub(calc_ld_int(ah_cnt))
        };

        // calc some energy borders between minEn and avgEn

        // for (enIdx = 0; enIdx < NUM_NRG_LEVS; enIdx++) {
        //   en[enIdx] = (2.0f*enIdx+1.0f)/(2.0f*NUM_NRG_LEVS-1.0f);
        // }
        let d = avg_en_ld64.wrapping_sub(min_en_ld64);
        en_ld64[0] = min_en_ld64.wrapping_add(f_mult(d, fl2fxconst_dbl(0.06666667f32 as f64)));
        en_ld64[1] = min_en_ld64.wrapping_add(f_mult(d, fl2fxconst_dbl(0.20000000f32 as f64)));
        en_ld64[2] = min_en_ld64.wrapping_add(f_mult(d, fl2fxconst_dbl(0.33333334f32 as f64)));
        en_ld64[3] = min_en_ld64.wrapping_add(f_mult(d, fl2fxconst_dbl(0.46666667f32 as f64)));
        en_ld64[4] = min_en_ld64.wrapping_add(f_mult(d, fl2fxconst_dbl(0.60000002f32 as f64)));
        en_ld64[5] = min_en_ld64.wrapping_add(f_mult(d, fl2fxconst_dbl(0.73333335f32 as f64)));
        en_ld64[6] = min_en_ld64.wrapping_add(f_mult(d, fl2fxconst_dbl(0.86666667f32 as f64)));
        en_ld64[7] = min_en_ld64.wrapping_add(d);

        let mut done = false;
        let mut en_idx = 0usize;
        let mut sfb = max_sfb - 1;

        while !done {
            for element_id in element_offset as usize..n_elements as usize {
                if cm.el_info[element_id].el_type != ID_DSE {
                    let qr = qc_range(qc_element, cm, element_id);
                    let pe_data = &qc_element[element_id].pe_data;
                    for ch in 0..cm.el_info[element_id].n_channels_in_el as usize {
                        let ch_idx = cm.el_info[element_id].channel_index[ch] as usize;
                        let qc_out_channel = &mut p_qc_out_channels[qr.start + ch];
                        if sfb >= start_sfb[ch_idx] && sfb < max_sfb_per_group[ch_idx] {
                            let mut sfb_grp = 0;
                            while sfb_grp < sfb_cnt[ch_idx] {
                                let i = (sfb_grp + sfb) as usize;
                                // sfb energy below border ?
                                if ah_flag[element_id][ch][i] != NO_AH
                                    && qc_out_channel.sfb_energy_ld_data[i] < en_ld64[en_idx]
                                {
                                    // allow hole
                                    ah_flag[element_id][ch][i] = NO_AH;
                                    qc_out_channel.sfb_threshold_ld_data[i] = fl2fxconst_dbl(
                                        0.015625f32 as f64,
                                    )
                                    .wrapping_add(qc_out_channel.sfb_weighted_energy_ld_data[i]);
                                    act_pe = act_pe.wrapping_sub(
                                        pe_data.pe_channel_data[ch].sfb_pe[i] >> PE_CONSTPART_SHIFT,
                                    );
                                }
                                if act_pe <= desired_pe {
                                    return; // stop if enough has been saved
                                }
                                sfb_grp += sfb_per_group[ch_idx];
                            }
                        }
                    }
                }
            }

            sfb -= 1;
            if sfb < min_sfb {
                // restart with next energy border
                sfb = max_sfb;
                en_idx += 1;
                if en_idx >= NUM_NRG_LEVS {
                    done = true;
                }
            }
        }
    }
}

/// `FDKaacEnc_resetAHFlags`: reset avoid hole flags from AH_ACTIVE to AH_INACTIVE (static in C;
/// public for the differential tests)
pub fn fdk_aac_enc_reset_ah_flags(
    ah_flag: &mut AhFlag,
    n_channels: i32,
    psy_out_channel: &[PsyOutChannel],
) {
    for ch in 0..n_channels as usize {
        for_each_sfb(&psy_out_channel[ch], |i| {
            if ah_flag[ch][i] == AH_ACTIVE {
                ah_flag[ch][i] = AH_INACTIVE;
            }
        });
    }
}

/// `CalcRedValPower` (static in C; public for the differential tests)
pub fn calc_red_val_power(num: FixpDbl, denum: FixpDbl, scaling: &mut i32) -> FixpDbl {
    let mut value = if num >= 0 {
        f_div_norm(num, denum, scaling)
    } else {
        f_div_norm(num.wrapping_neg(), denum, scaling).wrapping_neg()
    };
    value = f2_pow(value, *scaling, scaling);

    value
}

/// `FDKaacEnc_adaptThresholdsToPe`: two guesses for the reduction value and one final
/// correction of the thresholds (static in C; public for the differential tests)
///
/// `dyn_mem` is the shared dynamic RAM of C (see [`AdjThrDynMem`]).
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_adapt_thresholds_to_pe(
    cm: &ChannelMapping,
    adj_thr_state_element: &[AtsElement],
    qc_out: &mut QcOut,
    psy_out: &PsyOut,
    desired_pe: i32,
    max_iter_2nd_guess: i32,
    process_elements: i32,
    element_offset: i32,
    dyn_mem: &mut AdjThrDynMem,
) {
    let mut const_part_global: i32 = 0;
    let mut no_red_pe_global: i32 = 0;
    let mut n_active_lines_global: i32 = 0;
    let mut red_pe_global: i32 = 0;

    let mut n_elements = element_offset + process_elements;
    if n_elements > cm.n_elements {
        n_elements = cm.n_elements;
    }
    let elements = element_offset as usize..f_max(element_offset, n_elements) as usize;

    // dynMem_Ah_Flag / dynMem_Thr_Exp of C (shared dynamic RAM): every entry that is read is
    // written in Part I of this call.
    let AdjThrDynMem { ah_flag: p_ah_flag, thr_exp: p_thr_exp, sfb_n_active_lines_ld_data } =
        dyn_mem;

    // -------------------------------------------------------
    // Part I: Initialize data structures and variables...
    // -------------------------------------------------------
    for element_id in elements.clone() {
        if cm.el_info[element_id].el_type != ID_DSE {
            let n_channels = cm.el_info[element_id].n_channels_in_el;
            let qr = qc_range(&qc_out.qc_element, cm, element_id);
            let pr = psy_range(psy_out, cm, element_id);
            let views = psy_array_views(psy_out, qc_out, pr.clone(), qr.clone());
            let qc_ch = &mut qc_out.p_qc_out_channels[qr];
            let psy_ch = &psy_out.p_psy_out_channels[pr];

            // thresholds to the power of redExp
            fdk_aac_enc_calc_thresh_exp_views(
                &mut p_thr_exp[element_id],
                qc_ch,
                psy_ch,
                &views,
                n_channels,
            );

            // lower the minSnr requirements for low energies compared to the average energy in
            // this frame
            fdk_aac_enc_adapt_min_snr_views(
                qc_ch,
                psy_ch,
                &views,
                &adj_thr_state_element[element_id].min_snr_adapt_param,
                n_channels,
            );

            // init ahFlag (0: no ah necessary, 1: ah possible, 2: ah active
            fdk_aac_enc_init_avoid_hole_flag(
                qc_ch,
                psy_ch,
                &mut p_ah_flag[element_id],
                &psy_out.psy_out_element[element_id].tools_info,
                n_channels,
                &adj_thr_state_element[element_id].ah_param,
            );

            // sum up
            let pe_data = &qc_out.qc_element[element_id].pe_data;
            const_part_global = const_part_global.wrapping_add(pe_data.const_part);
            no_red_pe_global = no_red_pe_global.wrapping_add(pe_data.pe);
            n_active_lines_global =
                n_active_lines_global.wrapping_add(fix_max(pe_data.n_active_lines, 1));
        }
    }

    // First guess of reduction value:
    // avgThrExp = (float)pow(2.0f, (constPartGlobal - noRedPeGlobal)/(4.0f *
    // nActiveLinesGlobal)); redVal    = (float)pow(2.0f, (constPartGlobal -
    // desiredPe)/(4.0f * nActiveLinesGlobal)) - avgThrExp; redVal    = max(0.f,
    // redVal);
    let mut red_val_e: i32 = 0;
    let mut avg_thr_exp_e: i32 = 0;

    let mut red_val_m = calc_red_val_power(
        const_part_global.wrapping_sub(desired_pe),
        n_active_lines_global.wrapping_mul(4),
        &mut red_val_e,
    );
    let mut avg_thr_exp_m = calc_red_val_power(
        const_part_global.wrapping_sub(no_red_pe_global),
        n_active_lines_global.wrapping_mul(4),
        &mut avg_thr_exp_e,
    );
    let mut result_e = f_max(red_val_e, avg_thr_exp_e) + 1;

    let mut reduction_value_m: FixpDbl = f_max(
        0,
        scale_value(red_val_m, red_val_e - result_e)
            .wrapping_sub(scale_value(avg_thr_exp_m, avg_thr_exp_e - result_e)),
    );
    // SCHAR reductionValue_e
    let mut reduction_value_e: i8 = result_e as i8;

    // -----------------------------------------------------------------------
    // Part II: Calculate bit consumption of initial bit constraints setup
    // -----------------------------------------------------------------------
    for element_id in elements.clone() {
        if cm.el_info[element_id].el_type != ID_DSE {
            let n_channels = cm.el_info[element_id].n_channels_in_el;
            let qr = qc_range(&qc_out.qc_element, cm, element_id);
            let pr = psy_range(psy_out, cm, element_id);
            let qc_ch = &mut qc_out.p_qc_out_channels[qr];
            let psy_ch = &psy_out.p_psy_out_channels[pr];

            // reduce thresholds
            fdk_aac_enc_reduce_thresholds_cbr(
                qc_ch,
                psy_ch,
                &mut p_ah_flag[element_id],
                &p_thr_exp[element_id],
                n_channels,
                reduction_value_m,
                reduction_value_e,
            );

            // pe after first guess
            let pe_data = &mut qc_out.qc_element[element_id].pe_data;
            fdk_aac_enc_calc_pe(psy_ch, qc_ch, pe_data, n_channels);

            red_pe_global = red_pe_global.wrapping_add(pe_data.pe);
        }
    }

    // --------------------------------------------------
    // Part III: Iterate until bit constraints are met
    // --------------------------------------------------
    let mut iter = 0;
    while fixp_abs(red_pe_global.wrapping_sub(desired_pe))
        > f_mult_i(fl2fxconst_dbl(0.05f32 as f64), desired_pe)
        && iter < max_iter_2nd_guess
    {
        let mut red_pe_no_ah_global: i32 = 0;
        let mut const_part_no_ah_global: i32 = 0;
        let mut n_active_lines_no_ah_global: i32 = 0;

        for element_id in elements.clone() {
            if cm.el_info[element_id].el_type != ID_DSE {
                let mut red_pe_no_ah = 0;
                let mut const_part_no_ah = 0;
                let mut n_active_lines_no_ah = 0;
                let n_channels = cm.el_info[element_id].n_channels_in_el;
                let pr = psy_range(psy_out, cm, element_id);

                // pe for bands where avoid hole is inactive
                fdk_aac_enc_calc_pe_no_ah(
                    &mut red_pe_no_ah,
                    &mut const_part_no_ah,
                    &mut n_active_lines_no_ah,
                    &qc_out.qc_element[element_id].pe_data,
                    &p_ah_flag[element_id],
                    &psy_out.p_psy_out_channels[pr],
                    n_channels,
                );

                red_pe_no_ah_global = red_pe_no_ah_global.wrapping_add(red_pe_no_ah);
                const_part_no_ah_global = const_part_no_ah_global.wrapping_add(const_part_no_ah);
                n_active_lines_no_ah_global =
                    n_active_lines_no_ah_global.wrapping_add(n_active_lines_no_ah);
            }
        }

        // Calculate new redVal ...
        if desired_pe < red_pe_global {
            // new desired pe without bands where avoid hole is active
            let mut desired_pe_no_ah_global =
                desired_pe.wrapping_sub(red_pe_global.wrapping_sub(red_pe_no_ah_global));

            // limit desiredPeNoAH to positive values, as the PE can not become negative
            desired_pe_no_ah_global = f_max(0, desired_pe_no_ah_global);

            // second guess (only if there are bands left where avoid hole is inactive)
            if n_active_lines_no_ah_global > 0 {
                // avgThrExp = (float)pow(2.0f, (constPartNoAHGlobal - redPeNoAHGlobal) /
                // (4.0f * nActiveLinesNoAHGlobal)); redVal   += (float)pow(2.0f,
                // (constPartNoAHGlobal - desiredPeNoAHGlobal) / (4.0f *
                // nActiveLinesNoAHGlobal)) - avgThrExp; redVal    = max(0.0f, redVal);

                red_val_m = calc_red_val_power(
                    const_part_no_ah_global.wrapping_sub(desired_pe_no_ah_global),
                    n_active_lines_no_ah_global.wrapping_mul(4),
                    &mut red_val_e,
                );
                avg_thr_exp_m = calc_red_val_power(
                    const_part_no_ah_global.wrapping_sub(red_pe_no_ah_global),
                    n_active_lines_no_ah_global.wrapping_mul(4),
                    &mut avg_thr_exp_e,
                );
                result_e = f_max(reduction_value_e as i32, f_max(red_val_e, avg_thr_exp_e) + 1) + 1;

                reduction_value_m = f_max(
                    0,
                    scale_value(reduction_value_m, reduction_value_e as i32 - result_e)
                        .wrapping_add(scale_value(red_val_m, red_val_e - result_e))
                        .wrapping_sub(scale_value(avg_thr_exp_m, avg_thr_exp_e - result_e)),
                );
                reduction_value_e = result_e as i8;
            }
        } else {
            // redVal *= redPeGlobal/desiredPe;
            let mut sc0 = 0;
            let mut sc1 = 0;
            reduction_value_m = f_mult_norm(
                reduction_value_m,
                f_div_norm(red_pe_global, desired_pe, &mut sc0),
                &mut sc1,
            );
            // SCHAR += INT: computed in int, truncated to SCHAR
            reduction_value_e = (reduction_value_e as i32).wrapping_add(sc0 + sc1) as i8;

            for element_id in elements.clone() {
                if cm.el_info[element_id].el_type != ID_DSE {
                    let pr = psy_range(psy_out, cm, element_id);
                    fdk_aac_enc_reset_ah_flags(
                        &mut p_ah_flag[element_id],
                        cm.el_info[element_id].n_channels_in_el,
                        &psy_out.p_psy_out_channels[pr],
                    );
                }
            }
        }

        red_pe_global = 0;
        // Calculate new redVal's PE...
        for element_id in elements.clone() {
            if cm.el_info[element_id].el_type != ID_DSE {
                let n_channels = cm.el_info[element_id].n_channels_in_el;
                let qr = qc_range(&qc_out.qc_element, cm, element_id);
                let pr = psy_range(psy_out, cm, element_id);
                let qc_ch = &mut qc_out.p_qc_out_channels[qr];
                let psy_ch = &psy_out.p_psy_out_channels[pr];

                // reduce thresholds
                fdk_aac_enc_reduce_thresholds_cbr(
                    qc_ch,
                    psy_ch,
                    &mut p_ah_flag[element_id],
                    &p_thr_exp[element_id],
                    n_channels,
                    reduction_value_m,
                    reduction_value_e,
                );

                // pe after second guess
                let pe_data = &mut qc_out.qc_element[element_id].pe_data;
                fdk_aac_enc_calc_pe(psy_ch, qc_ch, pe_data, n_channels);
                red_pe_global = red_pe_global.wrapping_add(pe_data.pe);
            }
        }

        iter += 1;
    }

    // -------------------------------------------------------
    // Part IV: if still required, further reduce constraints
    // -------------------------------------------------------
    //                  1.0*        1.15*       1.20*
    //               desiredPe   desiredPe   desiredPe
    //                   |           |           |
    // ...XXXXXXXXXXXXXXXXXXXXXXXXXXX|           |
    //                   |           |           |XXXXXXXXXXX...
    //                   |           |XXXXXXXXXXX|
    //            --- A ---          | --- B --- | --- C ---
    //
    // (X): redPeGlobal
    // (A): FDKaacEnc_correctThresh()
    // (B): FDKaacEnc_allowMoreHoles()
    // (C): FDKaacEnc_reduceMinSnr()

    // correct thresholds to get closer to the desired pe
    if red_pe_global > desired_pe {
        fdk_aac_enc_correct_thresh(
            cm,
            qc_out,
            psy_out,
            p_ah_flag,
            p_thr_exp,
            reduction_value_m,
            reduction_value_e,
            desired_pe.wrapping_sub(red_pe_global),
            process_elements,
            element_offset,
            sfb_n_active_lines_ld_data,
        );

        // update PE
        red_pe_global = 0;
        for element_id in elements.clone() {
            if cm.el_info[element_id].el_type != ID_DSE {
                let n_channels = cm.el_info[element_id].n_channels_in_el;
                let qr = qc_range(&qc_out.qc_element, cm, element_id);
                let pr = psy_range(psy_out, cm, element_id);
                let pe_data = &mut qc_out.qc_element[element_id].pe_data;

                // pe after correctThresh
                fdk_aac_enc_calc_pe(
                    &psy_out.p_psy_out_channels[pr],
                    &qc_out.p_qc_out_channels[qr],
                    pe_data,
                    n_channels,
                );
                red_pe_global = red_pe_global.wrapping_add(pe_data.pe);
            }
        }
    }

    if red_pe_global > desired_pe {
        // reduce pe by reducing minSnr requirements
        fdk_aac_enc_reduce_min_snr(
            cm,
            qc_out,
            psy_out,
            p_ah_flag,
            f_mult_i(fl2fxconst_dbl(0.15f32 as f64), desired_pe).wrapping_add(desired_pe),
            &mut red_pe_global,
            process_elements,
            element_offset,
        );

        // reduce pe by allowing additional spectral holes
        fdk_aac_enc_allow_more_holes(
            cm,
            qc_out,
            psy_out,
            adj_thr_state_element,
            p_ah_flag,
            desired_pe,
            red_pe_global,
            process_elements,
            element_offset,
        );
    }
}

/// `FDKaacEnc_AdaptThresholdsVBR`: similar to FDKaacEnc_adaptThresholdsToPe(), for VBR-mode
/// (static in C; public for the differential tests)
pub fn fdk_aac_enc_adapt_thresholds_vbr(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    adj_thr_state_element: &mut AtsElement,
    tools_info: &ToolsInfo,
    n_channels: i32,
) {
    fdk_aac_enc_adapt_thresholds_vbr_views(
        qc_out_channel,
        psy_out_channel,
        &NO_PSY_ARRAY_VIEWS,
        adj_thr_state_element,
        tools_info,
        n_channels,
    )
}

/// [`fdk_aac_enc_adapt_thresholds_vbr`] with the arrays C reads through the `PSY_OUT_CHANNEL`
/// from `views` where set (see [`PsyArrayViews`]).
pub fn fdk_aac_enc_adapt_thresholds_vbr_views(
    qc_out_channel: &mut [QcOutChannel],
    psy_out_channel: &[PsyOutChannel],
    views: &PsyArrayViews,
    adj_thr_state_element: &mut AtsElement,
    tools_info: &ToolsInfo,
    n_channels: i32,
) {
    // scratch memory (all entries that are read are written first)
    let mut p_ah_flag: AhFlag = [[0; MAX_GROUPED_SFB]; 2];
    let mut p_thr_exp: ThrExp = [[0; MAX_GROUPED_SFB]; 2];

    // thresholds to the power of redExp
    fdk_aac_enc_calc_thresh_exp_views(
        &mut p_thr_exp,
        qc_out_channel,
        psy_out_channel,
        views,
        n_channels,
    );

    // lower the minSnr requirements for low energies compared to the average energy in this
    // frame
    fdk_aac_enc_adapt_min_snr_views(
        qc_out_channel,
        psy_out_channel,
        views,
        &adj_thr_state_element.min_snr_adapt_param,
        n_channels,
    );

    // init ahFlag (0: no ah necessary, 1: ah possible, 2: ah active
    fdk_aac_enc_init_avoid_hole_flag(
        qc_out_channel,
        psy_out_channel,
        &mut p_ah_flag,
        tools_info,
        n_channels,
        &adj_thr_state_element.ah_param,
    );

    // reduce thresholds
    let vbr_qual_factor = adj_thr_state_element.vbr_qual_factor;
    fdk_aac_enc_reduce_thresholds_vbr_views(
        qc_out_channel,
        psy_out_channel,
        views,
        &mut p_ah_flag,
        &p_thr_exp,
        n_channels,
        vbr_qual_factor,
        &mut adj_thr_state_element.chaos_measure_old,
    );
}

/// `FDKaacEnc_calcBitSave`: calculates percentage of bit save (static in C; public for the
/// differential tests)
///
/// ```text
///         bitsave
///                     maxBitSave(%)|   clipLow
///                                  |---\
///                                  |    \
///                                  |     \
///                                  |      \
///                                  |       \
///                                  |--------\--------------> bitres
///                                  |         \
///                     minBitSave(%)|          \------------
///                                           clipHigh      maxBitres
/// ```
pub fn fdk_aac_enc_calc_bit_save(
    mut fill_level: FixpDbl,
    clip_low: FixpDbl,
    clip_high: FixpDbl,
    _min_bit_save: FixpDbl,
    max_bit_save: FixpDbl,
    bitsave_slope: FixpDbl,
) -> FixpDbl {
    fill_level = fix_max(fill_level, clip_low);
    fill_level = fix_min(fill_level, clip_high);

    max_bit_save.wrapping_sub(f_mult(fill_level.wrapping_sub(clip_low), bitsave_slope))
}

/// `FDKaacEnc_calcBitSpend`: calculates percentage of bit spend (static in C; public for the
/// differential tests)
///
/// ```text
///                               bitspend      clipHigh
///                    maxBitSpend(%)|          /-----------maxBitres
///                                  |         /
///                                  |        /
///                                  |       /
///                                  |      /
///                                  |     /
///                                  |----/-----------------> bitres
///                                  |   /
///                    minBitSpend(%)|--/
///                                    clipLow
/// ```
pub fn fdk_aac_enc_calc_bit_spend(
    mut fill_level: FixpDbl,
    clip_low: FixpDbl,
    clip_high: FixpDbl,
    min_bit_spend: FixpDbl,
    _max_bit_spend: FixpDbl,
    bitspend_slope: FixpDbl,
) -> FixpDbl {
    fill_level = fix_max(fill_level, clip_low);
    fill_level = fix_min(fill_level, clip_high);

    min_bit_spend.wrapping_add(f_mult(fill_level.wrapping_sub(clip_low), bitspend_slope))
}

/// `FDKaacEnc_adjustPeMinMax()`: adjusts peMin and peMax parameters over time (static in C;
/// public for the differential tests)
pub fn fdk_aac_enc_adjust_pe_min_max(curr_pe: i32, pe_min: &mut i32, pe_max: &mut i32) {
    let min_fac_hi = fl2fxconst_dbl(0.3f32 as f64);
    let max_fac_hi = MAXVAL_DBL;
    let min_fac_lo = fl2fxconst_dbl(0.14f32 as f64);
    let max_fac_lo = fl2fxconst_dbl(0.07f32 as f64);

    let min_diff_fix = f_mult_i(fl2fxconst_dbl(0.1666666667f32 as f64), curr_pe);

    if curr_pe > *pe_max {
        let diff = curr_pe.wrapping_sub(*pe_max);
        *pe_min = pe_min.wrapping_add(f_mult_i(min_fac_hi, diff));
        *pe_max = pe_max.wrapping_add(f_mult_i(max_fac_hi, diff));
    } else if curr_pe < *pe_min {
        let diff = pe_min.wrapping_sub(curr_pe);
        *pe_min = pe_min.wrapping_sub(f_mult_i(min_fac_lo, diff));
        *pe_max = pe_max.wrapping_sub(f_mult_i(max_fac_lo, diff));
    } else {
        *pe_min = pe_min.wrapping_add(f_mult_i(min_fac_hi, curr_pe.wrapping_sub(*pe_min)));
        *pe_max = pe_max.wrapping_sub(f_mult_i(max_fac_lo, pe_max.wrapping_sub(curr_pe)));
    }

    if pe_max.wrapping_sub(*pe_min) < min_diff_fix {
        let pe_max_fix = *pe_max;
        let pe_min_fix = *pe_min;

        let part_lo_fix: FixpDbl = fix_max(0, curr_pe.wrapping_sub(pe_min_fix));
        let part_hi_fix: FixpDbl = fix_max(0, pe_max_fix.wrapping_sub(curr_pe));

        let pe_max_fix = curr_pe.wrapping_add(f_mult_i(
            f_div_norm_noexp(part_hi_fix, part_lo_fix.wrapping_add(part_hi_fix)),
            min_diff_fix,
        ));
        let mut pe_min_fix = curr_pe.wrapping_sub(f_mult_i(
            f_div_norm_noexp(part_lo_fix, part_lo_fix.wrapping_add(part_hi_fix)),
            min_diff_fix,
        ));
        pe_min_fix = fix_max(0, pe_min_fix);

        *pe_max = pe_max_fix;
        *pe_min = pe_min_fix;
    }
}

/// `FDKaacEnc_bitresCalcBitFac`: calculates factor of spending bits for one frame
///
/// * 1.0 : take all frame dynpart bits
/// * \>1.0 : take all frame dynpart bits + bitres
/// * <1.0 : put bits in bitreservoir
///
/// ```text
///                      bitfac(%)            pemax
///                    bitspend(%)   |          /-----------maxBitres
///                                  |         /
///                                  |        /
///                                  |       /
///                                  |      /
///                                  |     /
///                                  |----/-----------------> pe
///                                  |   /
///                    bitsave(%)    |--/
///                                     pemin
/// ```
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_bitres_calc_bit_fac(
    bitres_bits: i32,
    max_bitres_bits: i32,
    pe: i32,
    last_window_sequence: i32,
    avg_bits: i32,
    max_bit_fac: FixpDbl,
    adj_thr: &AdjThrState,
    adj_thr_chan: &mut AtsElement,
    p_bitres_fac: &mut FixpDbl,
    p_bitres_fac_e: &mut i32,
) {
    let bres_param: &BresParam;
    let bitsave_slope: FixpDbl;
    let bitspend_slope: FixpDbl;
    let mut fill_level_fix: FixpDbl = MAXVAL_DBL;
    let mut fill_level_e: i32 = 0;

    if last_window_sequence != SHORT_WINDOW {
        bres_param = &adj_thr.bres_param_long;
        bitsave_slope = fl2fxconst_dbl(0.466666666);
        bitspend_slope = fl2fxconst_dbl(0.666666666);
    } else {
        bres_param = &adj_thr.bres_param_short;
        bitsave_slope = 0x2E8BA2E9;
        bitspend_slope = 0x7fffffff;
    }

    // fillLevel = (float)(bitresBits+avgBits) / (float)(maxBitresBits + avgBits);
    if bitres_bits < max_bitres_bits {
        fill_level_fix = f_div_norm_noexp(bitres_bits, max_bitres_bits);
    }

    let mut pex = f_max(pe, adj_thr_chan.pe_min);
    pex = f_min(pex, adj_thr_chan.pe_max);

    let bit_save = fdk_aac_enc_calc_bit_save(
        fill_level_fix,
        bres_param.clip_save_low,
        bres_param.clip_save_high,
        bres_param.min_bit_save,
        bres_param.max_bit_save,
        bitsave_slope,
    );

    let bit_spend = fdk_aac_enc_calc_bit_spend(
        fill_level_fix,
        bres_param.clip_spend_low,
        bres_param.clip_spend_high,
        bres_param.min_bit_spend,
        bres_param.max_bit_spend,
        bitspend_slope,
    );

    let slope = schur_div(
        pex.wrapping_sub(adj_thr_chan.pe_min),
        adj_thr_chan.pe_max.wrapping_sub(adj_thr_chan.pe_min),
        31,
    );

    // scale down by 1 bit because the result of the following addition can be bigger than 1
    // (though smaller than 2)
    let mut bitres_fac: FixpDbl = (MAXVAL_DBL >> 1).wrapping_sub(bit_save >> 1);
    let mut bitres_fac_e: i32 = 1; // exp=1
    bitres_fac = f_mult_add_div2(bitres_fac, slope, bit_spend.wrapping_add(bit_save)); // exp=1

    // limit bitresFac for small bitreservoir
    let mut fill_level = f_div_norm(bitres_bits, avg_bits, &mut fill_level_e);
    if fill_level_e < 0 {
        fill_level = scale_value(fill_level, fill_level_e);
        fill_level_e = 0;
    }
    // shift down value by 1 because of summation, ...
    fill_level >>= 1;
    fill_level_e += 1;
    // ..., this summation:
    fill_level = fill_level.wrapping_add(scale_value(fl2fxconst_dbl(0.7f32 as f64), -fill_level_e));
    // set bitresfactor to same exponent as fillLevel
    if scale_value(bitres_fac, -fill_level_e + 1) > fill_level {
        bitres_fac = fill_level;
        bitres_fac_e = fill_level_e;
    }

    // limit bitresFac for high bitrates
    if scale_value(bitres_fac, bitres_fac_e - (DFRACT_BITS - 1 - 24)) > max_bit_fac {
        bitres_fac = max_bit_fac;
        bitres_fac_e = DFRACT_BITS - 1 - 24;
    }

    fdk_aac_enc_adjust_pe_min_max(pe, &mut adj_thr_chan.pe_min, &mut adj_thr_chan.pe_max);

    // output values
    *p_bitres_fac = bitres_fac;
    *p_bitres_fac_e = bitres_fac_e;
}

/// `FDKaacEnc_AdjThrNew`: allocate ADJ_THR_STATE
///
/// C takes the state from static RAM (cleared) and allocates `nElements` `ATS_ELEMENT`s
/// (cleared); here the state is reset and owns `n_elements` cleared elements. Always returns 0.
pub fn fdk_aac_enc_adj_thr_new(h_adj_thr: &mut AdjThrState, n_elements: i32) -> i32 {
    *h_adj_thr = AdjThrState::default();
    h_adj_thr.adj_thr_state_elem = vec![AtsElement::default(); f_max(n_elements, 0) as usize];
    0
}

/// `FDKaacEnc_AdjThrInit`: initialize ADJ_THR_STATE
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_adj_thr_init(
    h_adj_thr: &mut AdjThrState,
    mean_pe: i32,
    inv_quant: i32,
    channel_mapping: &ChannelMapping,
    sample_rate: i32,
    total_bitrate: i32,
    is_low_delay: i32,
    _bit_res_mode: AacencBitresMode,
    d_zone_quant_enable: i32,
    bit_distribution_mode: i32,
    vbr_qual_factor: FixpDbl,
) {
    let point8 = fl2fxconst_dbl(0.8f32 as f64);
    let point6 = fl2fxconst_dbl(0.6f32 as f64);

    if bit_distribution_mode == 1 {
        h_adj_thr.bit_distribution_mode = AACENC_BD_MODE_INTRA_ELEMENT;
    } else {
        h_adj_thr.bit_distribution_mode = AACENC_BD_MODE_INTER_ELEMENT;
    }

    // Max number of iterations in second guess is 3 for lowdelay aot and for configurations
    // with multiple audio elements in general, otherwise iteration value is always 1.
    h_adj_thr.max_iter_2nd_guess =
        if is_low_delay != 0 || channel_mapping.n_elements > 1 { 3 } else { 1 };

    // common for all elements:
    // parameters for bitres control
    h_adj_thr.bres_param_long.clip_save_low = 0x1999999a; // FL2FXCONST_DBL(0.2f);
    h_adj_thr.bres_param_long.clip_save_high = 0x7999999a; // FL2FXCONST_DBL(0.95f);
    h_adj_thr.bres_param_long.min_bit_save = 0xf999999au32 as i32; // FL2FXCONST_DBL(-0.05f);
    h_adj_thr.bres_param_long.max_bit_save = 0x26666666; // FL2FXCONST_DBL(0.3f);
    h_adj_thr.bres_param_long.clip_spend_low = 0x1999999a; // FL2FXCONST_DBL(0.2f);
    h_adj_thr.bres_param_long.clip_spend_high = 0x7999999a; // FL2FXCONST_DBL(0.95f);
    h_adj_thr.bres_param_long.min_bit_spend = 0xf3333333u32 as i32; // FL2FXCONST_DBL(-0.10f);
    h_adj_thr.bres_param_long.max_bit_spend = 0x33333333; // FL2FXCONST_DBL(0.4f);

    h_adj_thr.bres_param_short.clip_save_low = 0x199999a0; // FL2FXCONST_DBL(0.2f);
    h_adj_thr.bres_param_short.clip_save_high = 0x5fffffff; // FL2FXCONST_DBL(0.75f);
    h_adj_thr.bres_param_short.min_bit_save = 0x00000000; // FL2FXCONST_DBL(0.0f);
    h_adj_thr.bres_param_short.max_bit_save = 0x199999a0; // FL2FXCONST_DBL(0.2f);
    h_adj_thr.bres_param_short.clip_spend_low = 0x199999a0; // FL2FXCONST_DBL(0.2f);
    h_adj_thr.bres_param_short.clip_spend_high = 0x5fffffff; // FL2FXCONST_DBL(0.75f);
    h_adj_thr.bres_param_short.min_bit_spend = 0xf9999998u32 as i32; // FL2FXCONST_DBL(-0.05f);
    h_adj_thr.bres_param_short.max_bit_spend = 0x40000000; // FL2FXCONST_DBL(0.5f);

    // specific for each element:
    for i in 0..f_max(channel_mapping.n_elements, 0) as usize {
        let relative_bits = channel_mapping.el_info[i].relative_bits;
        let n_channels_in_element = channel_mapping.el_info[i].n_channels_in_el;
        let bitrate_in_element = if relative_bits != MAXVAL_DBL {
            f_mult_norm_noexp(relative_bits, total_bitrate)
        } else {
            total_bitrate
        };
        let ch_bitrate = bitrate_in_element >> (if n_channels_in_element == 1 { 0 } else { 1 });

        let ats_elem = &mut h_adj_thr.adj_thr_state_elem[i];

        // parameters for bitres control
        if is_low_delay != 0 {
            ats_elem.pe_min = f_mult_i(point8, mean_pe);
            ats_elem.pe_max = f_mult_i(point6, mean_pe).wrapping_shl(1);
        } else {
            ats_elem.pe_min = f_mult_i(point8, mean_pe) >> 1;
            ats_elem.pe_max = f_mult_i(point6, mean_pe);
        }

        // for use in FDKaacEnc_reduceThresholdsVBR
        ats_elem.chaos_measure_old = fl2fxconst_dbl(0.3f32 as f64);

        // additional pe offset to correct pe2bits for low bitrates
        // ---- no longer necessary, set by table -----
        ats_elem.pe_offset = 0;

        // vbr initialisation
        ats_elem.vbr_qual_factor = vbr_qual_factor;
        if ch_bitrate < 32000 {
            ats_elem.pe_offset = fix_max(50, 100 - f_mult_i(0x666667, ch_bitrate));
        }

        // avoid hole parameters
        if ch_bitrate >= 20000 {
            ats_elem.ah_param.modify_min_snr = TRUE;
            ats_elem.ah_param.start_sfb_l = 15;
            ats_elem.ah_param.start_sfb_s = 3;
        } else {
            ats_elem.ah_param.modify_min_snr = FALSE;
            ats_elem.ah_param.start_sfb_l = 0;
            ats_elem.ah_param.start_sfb_s = 0;
        }

        // minSnr adaptation
        let msa_param = &mut ats_elem.min_snr_adapt_param;
        msa_param.max_red = fl2fxconst_dbl(0.00390625f32 as f64); // 0.25f/64.0f
                                                                  // start adaptation of minSnr for avgEn/sfbEn > startRatio
        msa_param.start_ratio = fl2fxconst_dbl(0.05190512648f32 as f64); // ld64(10.0f)
                                                                         // maximum minSnr reduction to minSnr^maxRed is reached for avgEn/sfbEn >= maxRatio
                                                                         // msaParam->maxRatio = 1000.0f;
                                                                         // msaParam->redRatioFac = ((float)1.0f - msaParam->maxRed) /
                                                                         // ((float)10.0f*log10(msaParam->startRatio/msaParam->maxRatio)/log10(2.0f)*(float)0.3010299956f);
        msa_param.red_ratio_fac = fl2fxconst_dbl(-0.375f32 as f64); // -0.0375f * 10.0f
                                                                    // msaParam->redOffs = (float)1.0f - msaParam->redRatioFac * (float)10.0f *
                                                                    // log10(msaParam->startRatio)/log10(2.0f) * (float)0.3010299956f;
        msa_param.red_offs = fl2fxconst_dbl(0.021484375); // 1.375f/64.0f

        // init pe correction
        ats_elem.pe_correction_factor_m = fl2fxconst_dbl(0.5f32 as f64); // 1.0
        ats_elem.pe_correction_factor_e = 1;

        ats_elem.dyn_bits_last = -1;
        ats_elem.pe_last = 0;

        // init bits to pe factor

        // init bits2PeFactor
        fdk_aac_enc_init_bits2_pe_factor(
            &mut ats_elem.bits2_pe_factor_m,
            &mut ats_elem.bits2_pe_factor_e,
            bitrate_in_element,
            n_channels_in_element,
            sample_rate,
            is_low_delay,
            d_zone_quant_enable,
            inv_quant,
        );
    }
}

/// `peAct < 1.5f * peLast && peAct > 0.7f * peLast` on the x87 unit (`fildl`, `fmuls`,
/// `fucomip`): the conversions and products are exact (at most 55 significant bits), so the
/// comparisons are the exact integer comparisons. Public for the differential tests against
/// [`fdk_aac_rs_fdk::x87`].
pub fn pe_correction_in_range_x87(pe_act: i32, pe_last: i32) -> bool {
    // Exact: 1.5 * peLast = 3 * peLast / 2, 0.7f = 11744051 / 2^24.
    let (a, l) = (pe_act as i64, pe_last as i64);
    debug_assert_eq!(0.7f32.to_bits(), 0x3f33_3333);
    2 * a < 3 * l && (a << 24) > 11_744_051 * l
}

/// `peAct < 1.5f * peLast && peAct > 0.7f * peLast` of `FDKaacEnc_FDKaacEnc_calcPeCorrection`.
fn pe_correction_in_range(pe_act: i32, pe_last: i32) -> bool {
    if ARCH == Arch::X86_32 {
        pe_correction_in_range_x87(pe_act, pe_last)
    } else {
        (pe_act as f32) < 1.5f32 * (pe_last as f32) && (pe_act as f32) > 0.7f32 * (pe_last as f32)
    }
}

/// `FDKaacEnc_FDKaacEnc_calcPeCorrection`: calc desired pe (static in C; public for the
/// differential tests)
///
/// `peAct < 1.5f * peLast` and `peAct > 0.7f * peLast` are evaluated in `float` (both reference
/// builds: `scvtf`/`fmul`/`fcmp` resp. `cvtsi2ss`/`mulss`/`ucomiss`). The `x86_32` build
/// evaluates them on the x87 unit (`fildl`, `fmuls`, `fucomip`): the conversions and the products
/// are exact there (at most 55 significant bits), so the comparisons are exact.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_calc_pe_correction(
    correction_fac_m: &mut FixpDbl,
    correction_fac_e: &mut i32,
    pe_act: i32,
    pe_last: i32,
    bits_last: i32,
    bits2_pe_factor_m: FixpDbl,
    bits2_pe_factor_e: i32,
) {
    if bits_last > 0
        && pe_correction_in_range(pe_act, pe_last)
        && fdk_aac_enc_bits2pe2(
            bits_last,
            f_mult(fl2fxconst_dbl((1.2f32 / 2.0f32) as f64), bits2_pe_factor_m),
            bits2_pe_factor_e + 1,
        ) > pe_last
        && fdk_aac_enc_bits2pe2(
            bits_last,
            f_mult(fl2fxconst_dbl(0.65f32 as f64), bits2_pe_factor_m),
            bits2_pe_factor_e,
        ) < pe_last
    {
        let mut corr_fac = *correction_fac_m;

        let mut scaling = 0;
        let denum: FixpDbl = fdk_aac_enc_bits2pe2(bits_last, bits2_pe_factor_m, bits2_pe_factor_e);
        let mut new_fac = f_div_norm(pe_last, denum, &mut scaling);

        // dead zone, newFac and corrFac are scaled by 0.5
        if pe_last <= denum {
            // ratio <= 1.f
            new_fac = fix_max(
                scale_value(
                    fix_min(
                        f_mult(fl2fxconst_dbl((1.1f32 / 2.0f32) as f64), new_fac),
                        scale_value(fl2fxconst_dbl((1.0f32 / 2.0f32) as f64), -scaling),
                    ),
                    scaling,
                ),
                fl2fxconst_dbl((0.85f32 / 2.0f32) as f64),
            );
        } else {
            // ratio < 1.f
            new_fac = fix_max(
                fix_min(
                    scale_value(f_mult(fl2fxconst_dbl((0.9f32 / 2.0f32) as f64), new_fac), scaling),
                    fl2fxconst_dbl((1.15f32 / 2.0f32) as f64),
                ),
                fl2fxconst_dbl((1.0f32 / 2.0f32) as f64),
            );
        }

        let half = fl2fxconst_dbl((1.0f32 / 2.0f32) as f64);
        if (new_fac > half && corr_fac < half) || (new_fac < half && corr_fac > half) {
            corr_fac = half;
        }

        // faster adaptation towards 1.0, slower in the other direction
        if (corr_fac < half && new_fac < corr_fac) || (corr_fac > half && new_fac > corr_fac) {
            corr_fac = f_mult(fl2fxconst_dbl(0.85f32 as f64), corr_fac)
                .wrapping_add(f_mult(fl2fxconst_dbl(0.15f32 as f64), new_fac));
        } else {
            corr_fac = f_mult(fl2fxconst_dbl(0.7f32 as f64), corr_fac)
                .wrapping_add(f_mult(fl2fxconst_dbl(0.3f32 as f64), new_fac));
        }

        corr_fac = fix_max(
            fix_min(corr_fac, fl2fxconst_dbl((1.15f32 / 2.0f32) as f64)),
            fl2fxconst_dbl(0.85 / 2.0f32 as f64),
        );

        *correction_fac_m = corr_fac;
        *correction_fac_e = 1;
    } else {
        *correction_fac_m = fl2fxconst_dbl((1.0f32 / 2.0f32) as f64);
        *correction_fac_e = 1;
    }
}

/// `FDKaacEnc_calcPeCorrectionLowBitRes` (static in C; public for the differential tests)
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_calc_pe_correction_low_bit_res(
    correction_fac_m: &mut FixpDbl,
    correction_fac_e: &mut i32,
    pe_last: i32,
    bits_last: i32,
    bitres_level: i32,
    n_channels: i32,
    bits2_pe_factor_m: FixpDbl,
    bits2_pe_factor_e: i32,
) {
    // tuning params
    let amp = fl2fxconst_dbl(0.005);
    let max_diff = fl2fxconst_dbl(0.25f32 as f64);

    if bits_last > 0 {
        // Estimate deviation of granted and used dynamic bits in previous frame, in PE units
        let bits_bal_last = pe_last.wrapping_sub(fdk_aac_enc_bits2pe2(
            bits_last,
            bits2_pe_factor_m,
            bits2_pe_factor_e,
        ));

        // reserve n bits per channel
        let mut headroom = if bitres_level >= 50 * n_channels { 0 } else { 100 * n_channels };

        // in PE units
        headroom = fdk_aac_enc_bits2pe2(headroom, bits2_pe_factor_m, bits2_pe_factor_e);

        // diff = amp * ((bitsBalLast - headroom) / (bitresLevel + headroom)
        // diff = max ( min ( diff, maxDiff, -maxDiff)) / 2
        let denominator: FixpDbl =
            fdk_aac_enc_bits2pe2(bitres_level, bits2_pe_factor_m, bits2_pe_factor_e)
                .wrapping_add(headroom);

        let mut scaling = 0;
        let mut diff = if bits_bal_last >= headroom {
            f_mult(amp, f_div_norm(bits_bal_last.wrapping_sub(headroom), denominator, &mut scaling))
        } else {
            f_mult(
                amp,
                f_div_norm(
                    bits_bal_last.wrapping_sub(headroom).wrapping_neg(),
                    denominator,
                    &mut scaling,
                ),
            )
            .wrapping_neg()
        };

        scaling -= 1; // divide by 2

        diff = if scaling <= 0 {
            f_max(
                f_min(diff.wrapping_shr(scaling.wrapping_neg() as u32), max_diff >> 1),
                -max_diff >> 1,
            )
        } else {
            f_max(
                f_min(diff, max_diff.wrapping_shr((1 + scaling) as u32)),
                (-max_diff).wrapping_shr((1 + scaling) as u32),
            )
            .wrapping_shl(scaling as u32)
        };

        // corrFac += diff
        // corrFac = max ( min ( corrFac/2.f, 1.f/2.f, 0.75f/2.f ) )
        *correction_fac_m = f_max(
            f_min(correction_fac_m.wrapping_add(diff), fl2fxconst_dbl((1.0f32 / 2.0f32) as f64)),
            fl2fxconst_dbl((0.75f32 / 2.0f32) as f64),
        );
        *correction_fac_e = 1;
    } else {
        *correction_fac_m = fl2fxconst_dbl(0.75 / 2.0f32 as f64);
        *correction_fac_e = 1;
    }
}

/// `FDKaacEnc_DistributeBits`
///
/// `el` is the index of `AdjThrStateElement` in `adj_thr_state.adj_thr_state_elem`;
/// `psy_out_channel` holds the element's channels; `avg_bits` is `grantedDynBits` of the C
/// definition.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_distribute_bits(
    adj_thr_state: &mut AdjThrState,
    el: usize,
    psy_out_channel: &[PsyOutChannel],
    pe_data: &mut PeData,
    granted_pe: &mut i32,
    granted_pe_corr: &mut i32,
    n_channels: i32,
    _common_window: i32,
    avg_bits: i32,
    bitres_bits: i32,
    max_bitres_bits: i32,
    max_bit_fac: FixpDbl,
    bit_res_mode: AacencBitresMode,
) {
    let granted_dyn_bits = avg_bits;
    let mut bit_factor: FixpDbl = 0;
    let mut bit_factor_e: i32 = 0;
    let no_red_pe = pe_data.pe;

    // prefer short windows for calculation of bitFactor
    let mut cur_window_sequence = LONG_WINDOW;
    if n_channels == 2 {
        if psy_out_channel[0].last_window_sequence == SHORT_WINDOW
            || psy_out_channel[1].last_window_sequence == SHORT_WINDOW
        {
            cur_window_sequence = SHORT_WINDOW;
        }
    } else {
        cur_window_sequence = psy_out_channel[0].last_window_sequence;
    }

    if granted_dyn_bits >= 1 {
        if bit_res_mode != AACENC_BR_MODE_FULL {
            // small or disabled bitreservoir
            let e = &adj_thr_state.adj_thr_state_elem[el];
            *granted_pe =
                fdk_aac_enc_bits2pe2(granted_dyn_bits, e.bits2_pe_factor_m, e.bits2_pe_factor_e);
        } else {
            // factor dependend on current fill level and pe
            let mut e = adj_thr_state.adj_thr_state_elem[el];
            fdk_aac_enc_bitres_calc_bit_fac(
                bitres_bits,
                max_bitres_bits,
                no_red_pe,
                cur_window_sequence,
                granted_dyn_bits,
                max_bit_fac,
                adj_thr_state,
                &mut e,
                &mut bit_factor,
                &mut bit_factor_e,
            );
            adj_thr_state.adj_thr_state_elem[el] = e;

            // desired pe for actual frame
            // Worst case max of grantedDynBits is = 1024 * 5.27 * 2
            *granted_pe = fdk_aac_enc_bits2pe2(
                granted_dyn_bits,
                f_mult(bit_factor, e.bits2_pe_factor_m),
                e.bits2_pe_factor_e.wrapping_add(bit_factor_e),
            );
        }
    } else {
        *granted_pe = 0; // prevent divsion by 0
    }

    let e = &mut adj_thr_state.adj_thr_state_elem[el];

    // correction of pe value
    match bit_res_mode {
        AACENC_BR_MODE_DISABLED | AACENC_BR_MODE_REDUCED => {
            // correction of pe value for low bitres
            fdk_aac_enc_calc_pe_correction_low_bit_res(
                &mut e.pe_correction_factor_m,
                &mut e.pe_correction_factor_e,
                e.pe_last,
                e.dyn_bits_last,
                bitres_bits,
                n_channels,
                e.bits2_pe_factor_m,
                e.bits2_pe_factor_e,
            );
        }
        _ => {
            // correction of pe value for high bitres
            fdk_aac_enc_calc_pe_correction(
                &mut e.pe_correction_factor_m,
                &mut e.pe_correction_factor_e,
                fix_min(*granted_pe, no_red_pe),
                e.pe_last,
                e.dyn_bits_last,
                e.bits2_pe_factor_m,
                e.bits2_pe_factor_e,
            );
        }
    }

    *granted_pe_corr = f_mult(granted_pe.wrapping_shl(Q_AVGBITS as u32), e.pe_correction_factor_m)
        .wrapping_shr(Q_AVGBITS.wrapping_sub(e.pe_correction_factor_e) as u32);

    // update last pe
    e.pe_last = *granted_pe;
    e.dyn_bits_last = -1;
}

/// `FDKaacEnc_AdjustThresholds`: adjust thresholds
///
/// `qcElement[]` / `psyOutElement[]` of C are `qc_out.qc_element` / `psy_out.psy_out_element`.
/// `psy_out` is not changed (the arrays that C changes through `PSY_OUT_CHANNEL` live in
/// `qc_out`).
pub fn fdk_aac_enc_adjust_thresholds(
    h_adj_thr: &mut AdjThrState,
    qc_out: &mut QcOut,
    psy_out: &mut PsyOut,
    cbr_bitrate_mode: i32,
    cm: &ChannelMapping,
) {
    let mut dyn_mem = AdjThrDynMem::default();
    fdk_aac_enc_adjust_thresholds_dyn_mem(
        h_adj_thr,
        qc_out,
        psy_out,
        cbr_bitrate_mode,
        cm,
        &mut dyn_mem,
    );
}

/// `FDKaacEnc_AdjustThresholds` with the caller's copy of the adjust-thresholds part of the
/// shared dynamic RAM (`dynMem_*`, see [`AdjThrDynMem`]): for callers that model the contents
/// of `P_BUF_1` that later stages observe. The results do not depend on `dyn_mem`.
pub fn fdk_aac_enc_adjust_thresholds_dyn_mem(
    h_adj_thr: &mut AdjThrState,
    qc_out: &mut QcOut,
    psy_out: &mut PsyOut,
    cbr_bitrate_mode: i32,
    cm: &ChannelMapping,
    dyn_mem: &mut AdjThrDynMem,
) {
    let psy_out: &PsyOut = psy_out;
    let n_el = f_max(cm.n_elements, 0) as usize;
    let is_audio = |t| t == ID_SCE || t == ID_CPE || t == ID_LFE;

    if cbr_bitrate_mode != 0 {
        // In case, no bits must be shifted between different elements, an element-wise
        // execution of the pe-dependent threshold- adaption becomes necessary...
        if h_adj_thr.bit_distribution_mode == AACENC_BD_MODE_INTRA_ELEMENT {
            for i in 0..n_el {
                let el_info = cm.el_info[i];

                if is_audio(el_info.el_type) {
                    // qcElement[i]->grantedPe = 2000;  Use this only for debugging
                    // if (totalGrantedPeCorr < totalNoRedPe) {
                    if qc_out.qc_element[i].granted_pe_corr < qc_out.qc_element[i].pe_data.pe {
                        // calc threshold necessary for desired pe
                        let granted_pe_corr = qc_out.qc_element[i].granted_pe_corr;
                        fdk_aac_enc_adapt_thresholds_to_pe(
                            cm,
                            &h_adj_thr.adj_thr_state_elem,
                            qc_out,
                            psy_out,
                            granted_pe_corr,
                            h_adj_thr.max_iter_2nd_guess,
                            1,        // Process only 1 element
                            i as i32, // Process exactly THIS element
                            dyn_mem,
                        );
                    }
                }
            }
        } else if h_adj_thr.bit_distribution_mode == AACENC_BD_MODE_INTER_ELEMENT {
            // Use global Pe to obtain the thresholds?
            if qc_out.total_granted_pe_corr < qc_out.total_no_red_pe {
                // add equal loadness quantization noise to match the desired pe calc threshold
                // necessary for desired pe. Now carried out globally to cover all(!) channels.
                let total_granted_pe_corr = qc_out.total_granted_pe_corr;
                fdk_aac_enc_adapt_thresholds_to_pe(
                    cm,
                    &h_adj_thr.adj_thr_state_elem,
                    qc_out,
                    psy_out,
                    total_granted_pe_corr,
                    h_adj_thr.max_iter_2nd_guess,
                    cm.n_elements, // Process all elements
                    0,             // Process exactly THIS element
                    dyn_mem,
                );
            } else {
                // In case global pe doesn't need to be reduced check each element to hold
                // estimated bitrate below maximum element bitrate.
                for i in 0..n_el {
                    if is_audio(cm.el_info[i].el_type) {
                        // Element pe applies to dynamic bits of maximum element bitrate.
                        let max_element_pe = fdk_aac_enc_bits2pe2(
                            (cm.el_info[i].n_channels_in_el * MIN_BUFSIZE_PER_EFF_CHAN)
                                .wrapping_sub(qc_out.qc_element[i].static_bits_used)
                                .wrapping_sub(qc_out.qc_element[i].ext_bits_used),
                            h_adj_thr.adj_thr_state_elem[i].bits2_pe_factor_m,
                            h_adj_thr.adj_thr_state_elem[i].bits2_pe_factor_e,
                        );

                        if max_element_pe < qc_out.qc_element[i].pe_data.pe {
                            fdk_aac_enc_adapt_thresholds_to_pe(
                                cm,
                                &h_adj_thr.adj_thr_state_elem,
                                qc_out,
                                psy_out,
                                max_element_pe,
                                h_adj_thr.max_iter_2nd_guess,
                                1,
                                i as i32,
                                dyn_mem,
                            );
                        }
                    }
                }
            }
        }
    } else {
        for i in 0..n_el {
            let el_info = cm.el_info[i];

            if is_audio(el_info.el_type) {
                // for VBR-mode
                let qr = qc_range(&qc_out.qc_element, cm, i);
                let pr = psy_range(psy_out, cm, i);
                let views = psy_array_views(psy_out, qc_out, pr.clone(), qr.clone());
                fdk_aac_enc_adapt_thresholds_vbr_views(
                    &mut qc_out.p_qc_out_channels[qr],
                    &psy_out.p_psy_out_channels[pr],
                    &views,
                    &mut h_adj_thr.adj_thr_state_elem[i],
                    &psy_out.psy_out_element[i].tools_info,
                    cm.el_info[i].n_channels_in_el,
                );
            }
        }
    }
    for i in 0..n_el {
        // no weighting of threholds and energies for mlout
        // weight energies and thresholds
        let qr = qc_range(&qc_out.qc_element, cm, i);
        let pr = psy_range(psy_out, cm, i);
        for ch in 0..cm.el_info[i].n_channels_in_el as usize {
            let p_qc_out_ch = &mut qc_out.p_qc_out_channels[qr.start + ch];
            for_each_sfb(&psy_out.p_psy_out_channels[pr.start + ch], |j| {
                p_qc_out_ch.sfb_threshold_ld_data[j] =
                    p_qc_out_ch.sfb_threshold_ld_data[j].wrapping_add(p_qc_out_ch.sfb_en_fac_ld[j]);
            });
        }
    }
}

/// `FDKaacEnc_AdjThrClose`: the owned elements are released with the state; this clears them.
pub fn fdk_aac_enc_adj_thr_close(h_adj_thr: &mut AdjThrState) {
    h_adj_thr.adj_thr_state_elem = Vec::new();
}

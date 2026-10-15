// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/pvc_dec.h, libSBRdec/src/pvc_dec.cpp.

//! Port of `pvc_dec.h` and `pvc_dec.cpp`: decoding of the Predictive Vector Coding data of USAC
//! (the SBR envelope is predicted from the grouped energies of the low band).
//!
//! `PVC_DYNAMIC_DATA` is a local variable of `sbr_dec()` that `pvcInitFrame()` fills only partly
//! (nothing for legacy SBR); the port starts from [`PvcDynamicData::default`] (all zero). Its
//! table pointers are references to the static tables; `pPvcID` points to the frame data, which
//! does not change while the dynamic data is used, so the port keeps a copy of the 16 IDs.

pub mod tables;
use tables::*;

use crate::sbr_ram::pod_struct;
use fdk_aac_rs_fdk::common_fix::*;

/// `PVC_DIVMODE_BITS`
pub const PVC_DIVMODE_BITS: u32 = 3;
/// `PVC_REUSEPVCID_BITS`
pub const PVC_REUSEPVCID_BITS: u32 = 1;
/// `PVC_PVCID_BITS`
pub const PVC_PVCID_BITS: u32 = 7;
/// `PVC_GRIDINFO_BITS`
pub const PVC_GRIDINFO_BITS: u32 = 1;
/// `MAX_PVC_ENVELOPES`
pub const MAX_PVC_ENVELOPES: usize = 2;
/// `PVC_NTIMESLOT`
pub const PVC_NTIMESLOT: usize = 16;
/// `PVC_NBLOW`: max. number of grouped QMF subbands below SBR range
pub const PVC_NBLOW: usize = 3;
/// `PVC_NBHIGH_MODE1`
pub const PVC_NBHIGH_MODE1: usize = 8;
/// `PVC_NBHIGH_MODE2`
pub const PVC_NBHIGH_MODE2: usize = 6;
/// `PVC_NBHIGH_MAX`
pub const PVC_NBHIGH_MAX: usize = PVC_NBHIGH_MODE1;
/// `PVC_NS_MAX`
pub const PVC_NS_MAX: usize = 16;

/// `PVC_ESG_EXP`: exponent of `pPvcStaticData->Esg` and `predictedEsg` in dB domain.
const PVC_ESG_EXP: i32 = 7;
/// `LOG10FAC` (`== 10/log2(10) * 2^-2`)
const LOG10FAC: f32 = 0.752574989159953f32;
/// `LOG10FAC_INV` (`== log2(10)/10 * 2^1`)
const LOG10FAC_INV: f32 = 0.664385618977472f32;

pod_struct! {
    /// `PVC_STATIC_DATA`: data for each PVC instance which needs to be persistent accross SBR
    /// frames
    pub struct PvcStaticData {
        /// `kx_last`: Xover frequency of last frame
        pub kx_last: u8,
        /// `pvc_mode_last`: PVC mode of last frame
        pub pvc_mode_last: u8,
        /// `Esg_slot_index`: ring buffer index to current Esg time slot
        pub esg_slot_index: u8,
        /// `pvcBorder0`: start SBR time slot of PVC frame
        pub pvc_border0: u8,
        /// `Esg`: Esg(ksg,t) of current and 15 previous time slots (ring buffer) in
        /// logarithmical domain
        pub esg: [[FixpDbl; PVC_NBLOW]; PVC_NS_MAX],
    }
}

/// `PVC_DYNAMIC_DATA`: data for each PVC instance which is valid during one SBR frame.
#[derive(Clone, Copy, Debug)]
pub struct PvcDynamicData {
    /// `pvc_mode`: PVC mode 1 or 2, 0 means legacy SBR
    pub pvc_mode: u8,
    /// `pvcBorder0`: start SBR time slot of PVC frame
    pub pvc_border0: u8,
    /// `kx`: index of the first QMF subband in the SBR range
    pub kx: u8,
    /// `RATE`: number of QMF subband samples per time slot (2 or 4)
    pub rate: u8,
    /// `ns`: number of time slots for time-domain smoothing of Esg(ksg,t)
    pub ns: u8,
    /// `pPvcID`: the prediction coefficient matrix index table (a copy, see the module
    /// documentation)
    pub p_pvc_id: [u8; PVC_NTIMESLOT],
    /// `pastEsgSlotsAvail`: number of past Esg(ksg,t) which are available for smoothing filter
    pub past_esg_slots_avail: u8,
    /// `pSCcoeffs`: smoothing window table
    pub p_sc_coeffs: &'static [FixpSgl],
    /// `sg_offset_low`: offset table for PVC grouping of SBR subbands below SBR range
    pub sg_offset_low: [i8; PVC_NBLOW + 1],
    /// `sg_offset_high_kx`: offset table for PVC grouping of SBR subbands in SBR range
    /// (relative to kx)
    pub sg_offset_high_kx: [i8; PVC_NBHIGH_MAX + 1],
    /// `nbHigh`: number of grouped QMF subbands in the SBR range
    pub nb_high: u8,
    /// `pScalingCoef`: scaling coeff table
    pub p_scaling_coef: &'static [i8],
    /// `pPVCTab1`: PVC mode 1 table (flattened)
    pub p_pvc_tab1: &'static [u8],
    /// `pPVCTab2`: PVC mode 2 table (flattened)
    pub p_pvc_tab2: &'static [u8],
    /// `pPVCTab1_dp`: mapping of pvcID to PVC mode 1 table
    pub p_pvc_tab1_dp: &'static [u8],
    /// `predEsg`: predicted energy in linear domain
    pub pred_esg: [[FixpDbl; PVC_NBHIGH_MAX]; PVC_NTIMESLOT],
    /// `predEsg_exp`: exponent of predicted energy in linear domain
    pub pred_esg_exp: [i32; PVC_NTIMESLOT],
    /// `predEsg_expMax`: maximum of `predEsg_exp[]`
    pub pred_esg_exp_max: i32,
}

impl Default for PvcDynamicData {
    fn default() -> Self {
        PvcDynamicData {
            pvc_mode: 0,
            pvc_border0: 0,
            kx: 0,
            rate: 0,
            ns: 0,
            p_pvc_id: [0; PVC_NTIMESLOT],
            past_esg_slots_avail: 0,
            p_sc_coeffs: &[],
            sg_offset_low: [0; PVC_NBLOW + 1],
            sg_offset_high_kx: [0; PVC_NBHIGH_MAX + 1],
            nb_high: 0,
            p_scaling_coef: &[],
            p_pvc_tab1: &[],
            p_pvc_tab2: &[],
            p_pvc_tab1_dp: &[],
            pred_esg: [[0; PVC_NBHIGH_MAX]; PVC_NTIMESLOT],
            pred_esg_exp: [0; PVC_NTIMESLOT],
            pred_esg_exp_max: 0,
        }
    }
}

/// `pvcInitFrame()`: initializes the PVC data of the current frame (call for pvcMode 0, 1, 2).
/// Returns 1 for an invalid PVC mode or `ns`, 0 otherwise.
#[allow(clippy::too_many_arguments)]
pub fn pvc_init_frame(
    p_pvc_static_data: &mut PvcStaticData,
    p_pvc_dynamic_data: &mut PvcDynamicData,
    pvc_mode: u8,
    ns: u8,
    rate: i32,
    kx: i32,
    pvc_border0: i32,
    p_pvc_id: &[u8; PVC_NTIMESLOT],
) -> i32 {
    p_pvc_dynamic_data.pvc_mode = pvc_mode;
    p_pvc_dynamic_data.kx = kx as u8;
    p_pvc_dynamic_data.rate = rate as u8;

    // RATE is the time step (1, 2 or 4)
    let rate_d = if rate != 0 { rate } else { 1 };
    let hbw = match pvc_mode {
        0 => {
            // legacy SBR, nothing to do
            return 0;
        }
        1 => {
            p_pvc_dynamic_data.nb_high = 8;
            p_pvc_dynamic_data.p_pvc_tab1 = G_3A_PVC_TAB1_MODE1.as_flattened();
            p_pvc_dynamic_data.p_pvc_tab2 = G_2A_PVC_TAB2_MODE1.as_flattened();
            p_pvc_dynamic_data.p_pvc_tab1_dp = &G_A_PVC_TAB1_DP_MODE1;
            p_pvc_dynamic_data.p_scaling_coef = &G_A_SCALING_COEF_MODE1;
            8 / rate_d
        }
        2 => {
            p_pvc_dynamic_data.nb_high = 6;
            p_pvc_dynamic_data.p_pvc_tab1 = G_3A_PVC_TAB1_MODE2.as_flattened();
            p_pvc_dynamic_data.p_pvc_tab2 = G_2A_PVC_TAB2_MODE2.as_flattened();
            p_pvc_dynamic_data.p_pvc_tab1_dp = &G_A_PVC_TAB1_DP_MODE2;
            p_pvc_dynamic_data.p_scaling_coef = &G_A_SCALING_COEF_MODE2;
            12 / rate_d
        }
        _ => {
            // invalid pvcMode
            return 1;
        }
    };

    p_pvc_dynamic_data.pvc_border0 = pvc_border0 as u8;
    let pvc_border0_last = p_pvc_static_data.pvc_border0;
    p_pvc_static_data.pvc_border0 = pvc_border0 as u8;
    p_pvc_dynamic_data.p_pvc_id = *p_pvc_id;

    p_pvc_dynamic_data.ns = ns;
    p_pvc_dynamic_data.p_sc_coeffs = match ns {
        16 => &PVC_SC_16,
        12 => &PVC_SC_12,
        4 => &PVC_SC_4,
        3 => &PVC_SC_3,
        _ => return 1,
    };

    // in the lower part of Esg-array there are previous values of Esg (from last call to this
    // function In case of an previous legay-SBR frame, or if there was a change in cross-over
    // FQ the value of first PVC SBR timeslot is propagated to prev-values in order to have
    // reasonable values for smooth-filtering
    if p_pvc_static_data.pvc_mode_last == 0 || p_pvc_static_data.kx_last as i32 != kx {
        p_pvc_dynamic_data.past_esg_slots_avail = 0;
    } else {
        p_pvc_dynamic_data.past_esg_slots_avail =
            (PVC_NS_MAX as i32 - pvc_border0_last as i32) as u8;
    }

    let lbw = 8 / rate_d;

    let mut temp = kx;
    for i in (0..=PVC_NBLOW).rev() {
        p_pvc_dynamic_data.sg_offset_low[i] = temp as i8;
        temp -= lbw;
    }

    temp = 0;
    for i in 0..=p_pvc_dynamic_data.nb_high as usize {
        p_pvc_dynamic_data.sg_offset_high_kx[i] = temp as i8;
        temp += hbw;
    }

    0
}

/// `pvcDecodeFrame()`: decodes the PVC data of one frame (call for pvcMode 1, 2). The slots of
/// time slot `t` are `qmf_buffer_real[t * RATE..]`; the first `overlap` slots have the exponent
/// `qmf_exponent_overlap`.
pub fn pvc_decode_frame(
    p_pvc_static_data: &mut PvcStaticData,
    p_pvc_dynamic_data: &mut PvcDynamicData,
    qmf_buffer_real: &[&mut [FixpDbl]],
    qmf_buffer_imag: &[&mut [FixpDbl]],
    overlap: i32,
    qmf_exponent_overlap: i32,
    qmf_exponent_current: i32,
) {
    let rate = p_pvc_dynamic_data.rate as usize;
    let pvc_border0 = p_pvc_dynamic_data.pvc_border0 as usize;

    for t in pvc_border0..PVC_NTIMESLOT {
        // predictedEsgSlot points into predEsg[t]: only the first nbHigh values are written.
        let mut pred_esg_exp = p_pvc_dynamic_data.pred_esg_exp[t];
        let mut predicted_esg_slot = p_pvc_dynamic_data.pred_esg[t];

        pvc_decode_time_slot(
            p_pvc_static_data,
            p_pvc_dynamic_data,
            &qmf_buffer_real[(t * rate).min(qmf_buffer_real.len())..],
            &qmf_buffer_imag[(t * rate).min(qmf_buffer_imag.len())..],
            if ((t * rate) as i32) < overlap { qmf_exponent_overlap } else { qmf_exponent_current },
            pvc_border0 as i32,
            t as i32,
            &mut predicted_esg_slot,
            &mut pred_esg_exp,
        );
        p_pvc_dynamic_data.pred_esg[t] = predicted_esg_slot;
        p_pvc_dynamic_data.pred_esg_exp[t] = pred_esg_exp;
    }
}

/// `pvcDecodeTimeSlot()`: decodes the PVC data of one SBR time slot (call for pvcMode 1, 2).
#[allow(clippy::too_many_arguments)]
pub fn pvc_decode_time_slot(
    p_pvc_static_data: &mut PvcStaticData,
    p_pvc_dynamic_data: &mut PvcDynamicData,
    qmf_slot_real: &[&mut [FixpDbl]],
    qmf_slot_imag: &[&mut [FixpDbl]],
    qmf_exponent: i32,
    _pvc_border0: i32,
    time_slot_number: i32,
    predicted_esg_slot: &mut [FixpDbl; PVC_NBHIGH_MAX],
    predicted_esg_exp: &mut i32,
) {
    let rate = p_pvc_dynamic_data.rate as usize;
    let esg_index = p_pvc_static_data.esg_slot_index as usize & (PVC_NS_MAX - 1);
    let sg_borders = p_pvc_dynamic_data.sg_offset_low;
    let mut e = [0 as FixpDbl; PVC_NBLOW];
    let mut ksg_start = 0usize;
    let min_esg = fl2fxconst_dbl(-10.0 / (1 << PVC_ESG_EXP) as f64); // 10*log10(0.1)

    // Subband grouping in QMF subbands below SBR range: within one timeslot ( i = [0...(RATE-1)]
    // QMF subsamples) calculate energy E(ib,t) and group them to Esg(ksg,t). Then transfer
    // values to logarithmical domain and store them for time domain smoothing. (7.5.6.3 Subband
    // grouping in QMF subbands below SBR range)
    let mut ksg = 0usize;
    while ksg < PVC_NBLOW + 1 && sg_borders[ksg] < 0 {
        // sg_offset_low[PVC_NBLOW] = kx >= 0 ends the loop
        if ksg < PVC_NBLOW {
            p_pvc_static_data.esg[esg_index][ksg] = min_esg;
        }
        ksg_start += 1;
        ksg += 1;
    }

    for i in 0..rate {
        let (Some(qmf_r), Some(qmf_i)) = (qmf_slot_real.get(i), qmf_slot_imag.get(i)) else {
            continue;
        };
        for ksg in ksg_start..PVC_NBLOW {
            for band in sg_borders[ksg] as i32..sg_borders[ksg + 1] as i32 {
                let r = qmf_r.get(band as usize).copied().unwrap_or(0);
                let im = qmf_i.get(band as usize).copied().unwrap_or(0);
                // The division by 8 == (RATE*lbw) is required algorithmically
                e[ksg] = e[ksg]
                    .wrapping_add(((f_pow2_div2(r) >> 1).wrapping_add(f_pow2_div2(im) >> 1)) >> 3);
            }
        }
    }
    for ksg in ksg_start..PVC_NBLOW {
        if e[ksg] > 0 {
            // 10/log2(10) = 0.752574989159953 * 2^2
            let mut exp_log = 0;
            let mut nrg = calc_log2(e[ksg], 2 * qmf_exponent + 2, &mut exp_log);
            nrg = f_mult(nrg, fl2fxconst_sgl(LOG10FAC as f64));
            nrg = scale_value(nrg, exp_log - PVC_ESG_EXP + 2);
            p_pvc_static_data.esg[esg_index][ksg] = f_max(nrg, min_esg);
        } else {
            p_pvc_static_data.esg[esg_index][ksg] = min_esg;
        }
    }

    // Time domain smoothing of subband-grouped energy
    {
        let mut idx = p_pvc_static_data.esg_slot_index as usize & (PVC_NS_MAX - 1);

        e = [0; PVC_NBLOW];
        for i in 0..p_pvc_dynamic_data.ns as usize {
            let sc_coeff = p_pvc_dynamic_data.p_sc_coeffs.get(i).copied().unwrap_or(0);
            let p_esg_filt = p_pvc_static_data.esg[idx];
            // Div2 is compensated by scaling of coeff table
            e[0] = f_mult_add_div2(e[0], p_esg_filt[0], sc_coeff);
            e[1] = f_mult_add_div2(e[1], p_esg_filt[1], sc_coeff);
            e[2] = f_mult_add_div2(e[2], p_esg_filt[2], sc_coeff);
            if i >= p_pvc_dynamic_data.past_esg_slots_avail as usize {
                // if past Esg values are not available use the ones from the last valid slot
                continue;
            }
            if idx > 0 {
                idx -= 1;
            } else {
                idx += PVC_NS_MAX - 1;
            }
        }
    }

    // SBR envelope scalefactor prediction
    {
        let mut e_high_exp = [0i32; PVC_NBHIGH_MAX];
        let mut e_high_exp_max = 0;
        let nb_high = (p_pvc_dynamic_data.nb_high as usize).min(PVC_NBHIGH_MAX);
        // the PVC IDs are 7 bit values (0..=127)
        let pvc_tab2_id = p_pvc_dynamic_data.p_pvc_id
            [(time_slot_number as usize).min(PVC_NTIMESLOT - 1)] as usize;
        let dp = p_pvc_dynamic_data.p_pvc_tab1_dp;
        let pvc_tab1_id = if pvc_tab2_id < dp[0] as usize {
            0
        } else if pvc_tab2_id < dp[1] as usize {
            1
        } else {
            2
        };
        let p_tab1 = &p_pvc_dynamic_data.p_pvc_tab1[pvc_tab1_id * PVC_NBLOW * nb_high..];
        let p_tab2 = &p_pvc_dynamic_data.p_pvc_tab2[pvc_tab2_id * nb_high..];
        let sc = p_pvc_dynamic_data.p_scaling_coef;
        for ksg in 0..nb_high {
            e_high_exp[ksg] = 0;

            // residual part
            let mut accu: FixpDbl = ((p_tab2[ksg] as i8 as i32)
                << (DFRACT_BITS - 8 - PVC_ESG_EXP - 2 + sc[3] as i32))
                as FixpDbl;

            // linear combination of lower grouped energies part
            for kb in 0..PVC_NBLOW {
                let pred_coeff: FixpSgl =
                    (((p_tab1[kb * nb_high + ksg] as i8) as i16 as i32) << 8) as FixpSgl;
                // +1 to compensate for Div2; -2 for accu
                let pred_coeff_exp = -(sc[kb] as i32 + 1 - 2);
                accu = accu.wrapping_add(f_mult_div2(e[kb], pred_coeff) >> pred_coeff_exp);
            }
            // convert back to linear domain
            accu = f_mult(accu, fl2fxconst_sgl(LOG10FAC_INV as f64));
            let mut pred_coeff_exp = 0;
            // -1 compensates for exponent of LOG10FAC_INV; +2 for accu
            accu = f2_pow(accu, PVC_ESG_EXP - 1 + 2, &mut pred_coeff_exp);
            predicted_esg_slot[ksg] = accu;
            e_high_exp[ksg] = pred_coeff_exp;
            if pred_coeff_exp > e_high_exp_max {
                e_high_exp_max = pred_coeff_exp;
            }
        }

        // rescale output vector according to largest exponent
        for ksg in 0..nb_high {
            let scale = f_min(e_high_exp_max - e_high_exp[ksg], DFRACT_BITS - 1);
            predicted_esg_slot[ksg] >>= scale;
        }
        *predicted_esg_exp = e_high_exp_max;
    }

    p_pvc_static_data.esg_slot_index =
        ((p_pvc_static_data.esg_slot_index as usize + 1) & (PVC_NS_MAX - 1)) as u8;
    p_pvc_dynamic_data.past_esg_slots_avail =
        f_min(p_pvc_dynamic_data.past_esg_slots_avail as i32 + 1, PVC_NS_MAX as i32 - 1) as u8;
}

/// `pvcEndFrame()`: finishes the current PVC frame (call for pvcMode 0, 1, 2).
pub fn pvc_end_frame(
    p_pvc_static_data: &mut PvcStaticData,
    p_pvc_dynamic_data: &mut PvcDynamicData,
) {
    p_pvc_static_data.pvc_mode_last = p_pvc_dynamic_data.pvc_mode;
    p_pvc_static_data.kx_last = p_pvc_dynamic_data.kx;

    if p_pvc_dynamic_data.pvc_mode == 0 {
        return;
    }

    let mut max = -100;
    for t in p_pvc_dynamic_data.pvc_border0 as usize..PVC_NTIMESLOT {
        if p_pvc_dynamic_data.pred_esg_exp[t] > max {
            max = p_pvc_dynamic_data.pred_esg_exp[t];
        }
    }
    p_pvc_dynamic_data.pred_esg_exp_max = max;
}

/// `expandPredEsg()`: expands the predicted grouped energies of a time slot to the full QMF
/// subband resolution (`length_output_vector` values).
pub fn expand_pred_esg(
    p_pvc_dynamic_data: &PvcDynamicData,
    time_slot: i32,
    length_output_vector: i32,
    p_output: &mut [FixpDbl],
    p_output_exp: &mut [i8],
) {
    let ts = (time_slot.max(0) as usize).min(PVC_NTIMESLOT - 1);
    let pred_esg = &p_pvc_dynamic_data.pred_esg[ts];
    let e = p_pvc_dynamic_data.pred_esg_exp[ts] as i8;
    let nb_high = (p_pvc_dynamic_data.nb_high as usize).min(PVC_NBHIGH_MAX);
    let len = (length_output_vector.max(0) as usize).min(p_output.len());

    let mut k = 0usize;
    let mut ksg = 0usize;
    while ksg < nb_high {
        while (k as i32) < p_pvc_dynamic_data.sg_offset_high_kx[ksg + 1] as i32 && k < len {
            p_output[k] = pred_esg[ksg];
            p_output_exp[k] = e;
            k += 1;
        }
        ksg += 1;
    }
    // ksg--: the last group (nbHigh is 6 or 8)
    let last = nb_high.saturating_sub(1);
    while k < len {
        p_output[k] = pred_esg[last];
        p_output_exp[k] = e;
        k += 1;
    }
}

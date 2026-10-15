// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcGainDec_init.h, libDRCdec/src/drcGainDec_init.cpp.

//! Port of `drcGainDec_init.h`: initialization of the DRC gain decoder and of the active DRC
//! sets.

use fdk_aac_rs_fdk::common_fix::fl2fxconst_dbl;

use crate::drc_dec_gain_decoder::*;
use crate::drc_dec_tools::*;
use crate::drc_dec_types::*;
use crate::drc_decoder::*;

/// `_generateDrcInstructionsDerivedData(hGainDec, hUniDrcConfig, pInst, pCoef, pActiveDrc)`
fn generate_drc_instructions_derived_data(
    h_uni_drc_config: &UniDrcConfig,
    p_inst: &DrcInstructionsUniDrc,
    p_coef: &DrcCoefficientsUniDrc,
    p_active_drc: &mut ActiveDrc,
) -> DrcError {
    let mut gain_element_count = 0i32;
    let mut n_drc_channel_groups: u8 = 0;
    let mut gain_set_index_for_channel_group = [0i8; 8];

    let ducking = p_inst.drc_set_effect as i32 & (EB_DUCK_OTHER | EB_DUCK_SELF) != 0;
    let ducking_in = *p_inst.ducking_modification_for_channel();
    let err = derive_drc_channel_groups(
        p_inst.drc_set_effect as i32,
        p_inst.drc_channel_count as i32,
        &p_inst.gain_set_index,
        if ducking { Some(&ducking_in) } else { None },
        &mut n_drc_channel_groups,
        &mut gain_set_index_for_channel_group,
        &mut p_active_drc.channel_group_for_channel,
        if ducking { Some(&mut p_active_drc.ducking_modification_for_channel_group) } else { None },
    );
    if err != 0 {
        return err;
    }

    /* sanity check */
    if n_drc_channel_groups != p_inst.n_drc_channel_groups {
        return DE_NOT_OK;
    }
    // nDrcChannelGroups is at most 8 (deriveDrcChannelGroups).
    let n_groups = (p_inst.n_drc_channel_groups as usize).min(8);
    for g in 0..n_groups {
        if gain_set_index_for_channel_group[g] != p_inst.gain_set_index_for_channel_group[g] {
            return DE_NOT_OK;
        }
    }

    for g in 0..n_groups {
        let seq = p_inst.gain_set_index_for_channel_group[g] as i32;
        if seq != -1
            && (h_uni_drc_config.drc_coefficients_uni_drc_count == 0
                || seq >= p_coef.gain_set_count as i32)
        {
            p_active_drc.channel_group_is_parametric_drc[g] = 1;
        } else {
            p_active_drc.channel_group_is_parametric_drc[g] = 0;
            if seq >= p_coef.gain_set_count as i32 {
                return DE_NOT_OK;
            }
        }
    }

    /* gainElementCount */
    if ducking {
        for g in 0..n_groups {
            p_active_drc.band_count_for_channel_group[g] = 1;
        }
        p_active_drc.gain_element_count = p_inst.n_drc_channel_groups; /* one gain element per channel group */
    } else {
        for g in 0..n_groups {
            if p_active_drc.channel_group_is_parametric_drc[g] != 0 {
                gain_element_count += 1;
                p_active_drc.band_count_for_channel_group[g] = 1;
            } else {
                let seq = p_inst.gain_set_index_for_channel_group[g] as i32;
                // seq is in 0..gainSetCount here (checked above; the channel groups of
                // non-ducking sets have gain set indices >= 0).
                let band_count = usize::try_from(seq)
                    .ok()
                    .and_then(|s| p_coef.gain_set.get(s))
                    .map_or(0, |gs| gs.band_count);
                p_active_drc.band_count_for_channel_group[g] = band_count;
                gain_element_count += band_count as i32;
            }
        }
        p_active_drc.gain_element_count = gain_element_count as u8;
    }

    /* prepare gainElementForGroup (cumulated sum of bandCountForChannelGroup) */
    p_active_drc.gain_element_for_group[0] = 0;
    for g in 1..n_groups {
        /* index of first gain sequence in channel group */
        p_active_drc.gain_element_for_group[g] = p_active_drc.gain_element_for_group[g - 1]
            .wrapping_add(p_active_drc.band_count_for_channel_group[g - 1]);
    }

    DE_OK
}

/// `initGainDec(hGainDec)`
pub fn init_gain_dec(h_gain_dec: &mut DrcGainDecoder) -> DrcError {
    /* sanity check */
    if h_gain_dec.delta_tmin_default > h_gain_dec.frame_size {
        return DE_NOT_OK;
    }

    for i in 0..MAX_ACTIVE_DRCS {
        for j in 0..8 {
            /* use startup node at the beginning */
            h_gain_dec.active_drc[i].lnb_index_for_channel[j][0] = 0;
            for k in 1..NUM_LNB_FRAMES {
                h_gain_dec.active_drc[i].lnb_index_for_channel[j][k] = -1;
            }
        }
    }

    for j in 0..8 {
        h_gain_dec.channel_gain[j] = fl2fxconst_dbl(1.0 / 256.0);
    }

    for i in 0..4 * 1024 / 256 {
        h_gain_dec.dummy_subband_gains[i] = fl2fxconst_dbl(1.0 / 128.0);
    }

    h_gain_dec.status = 0; /* startup */

    DE_OK
}

/// `initDrcGainBuffers(frameSize, drcGainBuffers)`
pub fn init_drc_gain_buffers(frame_size: i32, drc_gain_buffers: &mut DrcGainBuffers) {
    /* prepare 12 instances of node buffers */
    for i in 0..12 {
        for j in 0..NUM_LNB_FRAMES {
            let lnb = &mut drc_gain_buffers.linear_node_buffer[i];
            lnb.n_nodes[j] = 1;
            lnb.linear_node[j][0].gain_lin = fl2fxconst_dbl(1.0 / 128.0);
            if j == 0 {
                lnb.linear_node[j][0].time = 0; /* initialize last node with startup node */
            } else {
                lnb.linear_node[j][0].time = frame_size.wrapping_sub(1) as i16;
            }
        }
    }

    /* prepare dummyLnb, a linearNodeBuffer containing a constant gain of 0 dB, for the "no DRC
     * processing" case */
    drc_gain_buffers.dummy_lnb.gain_interpolation_type = GIT_LINEAR;
    for i in 0..NUM_LNB_FRAMES {
        drc_gain_buffers.dummy_lnb.n_nodes[i] = 1;
        drc_gain_buffers.dummy_lnb.linear_node[i][0].gain_lin = fl2fxconst_dbl(1.0 / 128.0);
        drc_gain_buffers.dummy_lnb.linear_node[i][0].time = frame_size.wrapping_sub(1) as i16;
    }

    /* prepare channelGain delay line */
    for c in 0..8 {
        for i in 0..NUM_LNB_FRAMES {
            drc_gain_buffers.channel_gain[c][i] = fl2fxconst_dbl(1.0 / 256.0);
        }
    }

    drc_gain_buffers.lnb_pointer = 0;
}

/// `initActiveDrc(hGainDec, hUniDrcConfig, drcSetIdSelected, downmixIdSelected)`
pub fn init_active_drc(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    drc_set_id_selected: i32,
    downmix_id_selected: i32,
) -> DrcError {
    let mut is_multiband = 0;
    let mut p_coef_idx: Option<usize> = None;

    let Some(p_inst_idx) = select_drc_instructions(h_uni_drc_config, drc_set_id_selected) else {
        return DE_NOT_OK;
    };
    let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[p_inst_idx];

    // nActiveDrcs is at most 2 here: the selection process selects at most 3 DRC sets and the
    // count is reset for every configuration. Other counts have no defined reference result;
    // the port returns DE_NOT_OK.
    let n = h_gain_dec.n_active_drcs;
    if !(0..MAX_ACTIVE_DRCS as i32).contains(&n) {
        return DE_NOT_OK;
    }
    let n = n as usize;

    if p_inst.drc_set_id >= 0 {
        let Some(c) = select_drc_coefficients(h_uni_drc_config, p_inst.drc_location as i32) else {
            return DE_NOT_OK;
        };
        p_coef_idx = Some(c);
        let p_coef = &h_uni_drc_config.drc_coefficients_uni_drc[c];

        if p_coef.drc_frame_size_present != 0 {
            if p_coef.drc_frame_size as i32 != h_gain_dec.frame_size {
                return DE_NOT_OK;
            }
        }

        let err = generate_drc_instructions_derived_data(
            h_uni_drc_config,
            p_inst,
            p_coef,
            &mut h_gain_dec.active_drc[n],
        );
        if err != 0 {
            return err;
        }
    }

    h_gain_dec.active_drc[n].p_inst = p_inst_idx as isize + 1;
    h_gain_dec.active_drc[n].p_coef = p_coef_idx.map_or(0, |c| c as isize + 1);

    for g in 0..(p_inst.n_drc_channel_groups as usize).min(8) {
        if h_gain_dec.active_drc[n].band_count_for_channel_group[g] > 1 {
            if h_gain_dec.multi_band_active_drc_index != -1 {
                return DE_NOT_OK;
            }
            is_multiband = 1;
        }
    }

    if is_multiband != 0 {
        /* Keep activeDrc index of multiband DRC set */
        h_gain_dec.multi_band_active_drc_index = h_gain_dec.n_active_drcs;
    }

    if (h_gain_dec.channel_gain_active_drc_index == -1)
        && (downmix_id_selected == DOWNMIX_ID_BASE_LAYOUT)
        && (h_uni_drc_config.drc_instructions_uni_drc_count > 0)
    {
        /* use this activeDrc to apply channelGains */
        h_gain_dec.channel_gain_active_drc_index = h_gain_dec.n_active_drcs;
    }

    h_gain_dec.n_active_drcs += 1;
    if h_gain_dec.n_active_drcs > MAX_ACTIVE_DRCS as i32 {
        return DE_NOT_OK;
    }

    DE_OK
}

/// `initActiveDrcOffset(hGainDec)`
pub fn init_active_drc_offset(h_gain_dec: &mut DrcGainDecoder) -> DrcError {
    let mut acc_gain_element_count = 0i32;
    for a in 0..h_gain_dec.n_active_drcs.clamp(0, MAX_ACTIVE_DRCS as i32) as usize {
        h_gain_dec.active_drc[a].active_drc_offset = acc_gain_element_count;
        acc_gain_element_count += h_gain_dec.active_drc[a].gain_element_count as i32;
        if acc_gain_element_count > 12 {
            h_gain_dec.n_active_drcs = a as i32;
            return DE_NOT_OK;
        }
    }

    DE_OK
}

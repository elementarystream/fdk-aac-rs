// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcDec_reader.h, libDRCdec/src/drcDec_reader.cpp.

//! Port of `drcDec_reader.h`: the bitstream parser of uniDrc(), uniDrcConfig(),
//! loudnessInfoSet() and uniDrcGain() (ISO/IEC 23003-4 including AMD 1).
//!
//! The C functions check their structure handles for `NULL`; the references of the port cannot
//! be null, so these checks are gone.
//!
//! The `isBox` variants of the parsers (MP4 boxes) are ported, but every caller passes 0, as in
//! the reference (the box readers of `FDK_drcDecLib.cpp` return before parsing).

use fdk_aac_rs_fdk::bitstream::FdkBitStream;
use fdk_aac_rs_fdk::common_fix::{fl2fxconst_dbl, fl2fxconst_sgl, FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::common_fix::{DFRACT_BITS, FRACT_BITS, MAXVAL_SGL};

use crate::drc_dec_rom::*;
use crate::drc_dec_tools::*;
use crate::drc_dec_types::*;
use crate::drc_decoder::*;

/* MPEG-D DRC AMD 1 */

const UNIDRCCONFEXT_PARAM_DRC: u8 = 0x1;
const UNIDRCCONFEXT_V1: u8 = 0x2;
const UNIDRCLOUDEXT_EQ: u8 = 0x1;

const UNIDRCGAINEXT_TERM: u8 = 0x0;
const UNIDRCLOUDEXT_TERM: u8 = 0x0;
const UNIDRCCONFEXT_TERM: u8 = 0x0;

/// C integer division `a / b` as the arm64 reference computes it (`sdiv`): 0 for `b == 0`,
/// `a` for `INT_MIN / -1`, on every variant.
#[inline]
fn c_div(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `_getZ(nNodesMax)`
///
/// Z is the minimum codeword length that is needed to encode all possible timeDelta values,
/// `Z = ceil(log2(2*nNodesMax))`. Both reference builds compute `2 * nNodesMax` in 32 bits and
/// `1 << Z` with the shift amount modulo 32 (`lsl`/`shl` with a register amount). For
/// `2 * nNodesMax > 2^30` the port stops at Z = 31 (PORTING.md, "Deviations from the
/// reference").
fn get_z(n_nodes_max: i32) -> i32 {
    let two_n = n_nodes_max.wrapping_mul(2);
    if two_n > (1 << 30) {
        return 31;
    }
    let mut z = 1;
    while 1i32.wrapping_shl(z as u32) < two_n {
        z += 1;
    }
    z
}

/// `_getTimeDeltaMin(pGset, deltaTminDefault)`
fn get_time_delta_min(p_gset: &GainSet, delta_tmin_default: i32) -> i32 {
    if p_gset.time_delta_min_present != 0 {
        p_gset.time_delta_min as i32
    } else {
        delta_tmin_default
    }
}

/// `_compAssign(dest, src)`: compare and assign
#[inline]
pub(crate) fn comp_assign<T: PartialEq + Copy>(dest: &mut T, src: T) -> i32 {
    let mut diff = 0;
    if *dest != src {
        diff = 1;
    }
    *dest = src;
    diff
}

/// `Huffman`: `const SCHAR (*)[2]`
type Huffman = [[i8; 2]];

/// `_decodeHuffmanCW(h, hBs)`
pub fn decode_huffman_cw(h: &Huffman, h_bs: &mut FdkBitStream) -> i32 {
    let mut index: i8 = 0;

    while index >= 0 {
        let bit = h_bs.read_bits(1) as usize;
        // The codebooks are trees with indices inside the table.
        index = h[index as usize][bit];
    }

    index as i32 + 64 /* Add offset */
}

/**********/
/* uniDrc */
/**********/

/// `drcDec_readUniDrc(hBs, hUniDrcConfig, hLoudnessInfoSet, frameSize, deltaTminDefault,
/// hUniDrcGain)`
pub fn drc_dec_read_uni_drc(
    h_bs: &mut FdkBitStream,
    h_uni_drc_config: &mut UniDrcConfig,
    h_loudness_info_set: &mut LoudnessInfoSet,
    frame_size: i32,
    delta_tmin_default: i32,
    h_uni_drc_gain: &mut UniDrcGain,
) -> DrcError {
    let mut err;

    let loudness_info_set_present = h_bs.read_bits(1);
    if loudness_info_set_present != 0 {
        let uni_drc_config_present = h_bs.read_bits(1);
        if uni_drc_config_present != 0 {
            err = drc_dec_read_uni_drc_config(h_bs, h_uni_drc_config);
            if err != 0 {
                /* clear config, if parsing error occured */
                *h_uni_drc_config = UniDrcConfig::ZERO;
                h_uni_drc_config.diff = 1;
            }
        }
        err = drc_dec_read_loudness_info_set(h_bs, h_loudness_info_set);
        if err != 0 {
            /* clear config, if parsing error occured */
            *h_loudness_info_set = LoudnessInfoSet::ZERO;
            h_loudness_info_set.diff = 1;
        }
    }

    err = drc_dec_read_uni_drc_gain(
        h_bs,
        h_uni_drc_config,
        frame_size,
        delta_tmin_default,
        h_uni_drc_gain,
    );

    err
}

/**************/
/* uniDrcGain */
/**************/

/// `_decodeGainInitial(hBs, gainCodingProfile)`
fn decode_gain_initial(h_bs: &mut FdkBitStream, gain_coding_profile: i32) -> FixpSgl {
    let mut gain_initial: FixpSgl = 0;
    match gain_coding_profile {
        GCP_REGULAR => {
            let sign = h_bs.read_bits(1);
            let magn = h_bs.read_bits(8) as i32;

            gain_initial = (magn << (FRACT_BITS - 1 - 3 - 7)) as FixpSgl; /* magn * 0.125; */
            if sign != 0 {
                gain_initial = gain_initial.wrapping_neg();
            }
        }
        GCP_FADING => {
            let sign = h_bs.read_bits(1);
            if sign == 0 {
                gain_initial = 0;
            } else {
                let magn = h_bs.read_bits(10) as i32;
                /* - (magn + 1) * 0.125; */
                gain_initial = (((magn + 1) << (FRACT_BITS - 1 - 3 - 7)) as FixpSgl).wrapping_neg();
            }
        }
        GCP_CLIPPING_DUCKING => {
            let sign = h_bs.read_bits(1);
            if sign == 0 {
                gain_initial = 0;
            } else {
                let magn = h_bs.read_bits(8) as i32;
                /* - (magn + 1) * 0.125; */
                gain_initial = (((magn + 1) << (FRACT_BITS - 1 - 3 - 7)) as FixpSgl).wrapping_neg();
            }
        }
        GCP_CONSTANT => {}
        _ => {}
    }
    gain_initial
}

/// `_decodeNNodes(hBs)`
fn decode_n_nodes(h_bs: &mut FdkBitStream) -> i32 {
    let mut n_nodes = 0;
    let mut end_marker = 0;

    /* decode number of nodes */
    while end_marker != 1 {
        n_nodes += 1;
        if n_nodes >= 128 {
            break;
        }
        end_marker = h_bs.read_bits(1);
    }
    n_nodes
}

/// `_decodeGains(hBs, gainCodingProfile, nNodes, pNodes)`
fn decode_gains(
    h_bs: &mut FdkBitStream,
    gain_coding_profile: i32,
    n_nodes: i32,
    p_nodes: &mut [GainNode; 16],
) {
    p_nodes[0].gain_db = decode_gain_initial(h_bs, gain_coding_profile);

    let delta_gain_codebook: &Huffman = if gain_coding_profile == GCP_CLIPPING_DUCKING {
        &DELTA_GAIN_CODING_PROFILE_2_HUFFMAN
    } else {
        &DELTA_GAIN_CODING_PROFILE_0_1_HUFFMAN
    };

    for k in 1..n_nodes.max(1) as usize {
        let delta_gain = decode_huffman_cw(delta_gain_codebook, h_bs);
        if k >= 16 {
            continue;
        }
        /* gain_dB_e = 7 */
        /* pNodes[k-1].gainDb + 0.125*deltaGain */
        p_nodes[k].gain_db = p_nodes[k - 1]
            .gain_db
            .wrapping_add((delta_gain << (FRACT_BITS - 1 - 7 - 3)) as FixpSgl);
    }
}

/// `_decodeSlopes(hBs, gainInterpolationType, nNodes, pNodes)`
fn decode_slopes(h_bs: &mut FdkBitStream, gain_interpolation_type: i32, n_nodes: i32) {
    if gain_interpolation_type == GIT_SPLINE {
        /* decode slope steepness */
        for _ in 0..n_nodes {
            decode_huffman_cw(&SLOPE_STEEPNESS_HUFFMAN, h_bs);
        }
    }
}

/// `_decodeTimeDelta(hBs, Z)`
fn decode_time_delta(h_bs: &mut FdkBitStream, z: i32) -> i32 {
    let prefix = h_bs.read_bits(2);
    match prefix {
        0x0 => 1,
        0x1 => {
            let mu = h_bs.read_bits(2) as i32;
            mu + 2
        }
        0x2 => {
            let mu = h_bs.read_bits(3) as i32;
            mu + 6
        }
        0x3 => {
            let mu = h_bs.read_bits(z as u32) as i32;
            mu.wrapping_add(14)
        }
        _ => 0,
    }
}

/// `_decodeTimes(hBs, deltaTmin, frameSize, fullFrame, timeOffset, Z, nNodes, pNodes)`
fn decode_times(
    h_bs: &mut FdkBitStream,
    delta_tmin: i32,
    frame_size: i32,
    full_frame: i32,
    time_offset: i32,
    z: i32,
    n_nodes: i32,
    p_nodes: &mut [GainNode; 16],
) {
    let mut time_offs = time_offset;

    let frame_end_flag = if full_frame == 0 { h_bs.read_bits(1) as i32 } else { 1 };

    if frame_end_flag == 1 {
        /* frameEndFlag == 1 signals that the last node is at the end of the DRC frame */
        let mut node_res_flag = 0;
        let mut k: i32 = 0;
        while k < n_nodes - 1 {
            /* decode a delta time value */
            let time_delta = decode_time_delta(h_bs, z);
            if k >= (16 - 1) {
                k += 1;
                continue;
            }
            /* frameEndFlag == 1 needs special handling for last node with node reservoir */
            let node_time_tmp = time_offs.wrapping_add(time_delta.wrapping_mul(delta_tmin));
            if node_time_tmp > frame_size.wrapping_add(time_offset) {
                if node_res_flag == 0 {
                    p_nodes[k as usize].time = frame_size.wrapping_add(time_offset) as i16;
                    node_res_flag = 1;
                }
                p_nodes[k as usize + 1].time = node_time_tmp as i16;
            } else {
                p_nodes[k as usize].time = node_time_tmp as i16;
            }
            time_offs = node_time_tmp;
            k += 1;
        }
        if node_res_flag == 0 {
            k = k.min(16 - 1);
            p_nodes[k as usize].time = frame_size.wrapping_add(time_offset) as i16;
        }
    } else {
        for k in 0..n_nodes.max(0) as usize {
            /* decode a delta time value */
            let time_delta = decode_time_delta(h_bs, z);
            if k >= 16 {
                continue;
            }
            p_nodes[k].time = time_offs.wrapping_add(time_delta.wrapping_mul(delta_tmin)) as i16;
            time_offs = p_nodes[k].time as i32;
        }
    }
}

/// `_readNodes(hBs, gainSet, frameSize, timeDeltaMin, pNNodes, pNodes)`
fn read_nodes(
    h_bs: &mut FdkBitStream,
    gain_set: &GainSet,
    frame_size: i32,
    time_delta_min: i32,
    p_n_nodes: &mut u8,
    p_nodes: &mut [GainNode; 16],
) {
    let n_nodes;
    let z = get_z(c_div(frame_size, time_delta_min));
    let time_offset = if gain_set.time_alignment == 0 {
        -1
    } else {
        /* timeOffset = - deltaTmin + floor((deltaTmin-1)/2); */
        time_delta_min.wrapping_neg().wrapping_add(time_delta_min.wrapping_sub(1) / 2)
    };

    let drc_gain_coding_mode = h_bs.read_bits(1);
    if drc_gain_coding_mode == 0 {
        /* "simple" mode: only one node at the end of the frame with slope = 0 */
        n_nodes = 1;
        p_nodes[0].gain_db = decode_gain_initial(h_bs, gain_set.gain_coding_profile as i32);
        p_nodes[0].time = frame_size.wrapping_add(time_offset) as i16;
    } else {
        n_nodes = decode_n_nodes(h_bs);

        decode_slopes(h_bs, gain_set.gain_interpolation_type as i32, n_nodes);

        decode_times(
            h_bs,
            time_delta_min,
            frame_size,
            gain_set.full_frame as i32,
            time_offset,
            z,
            n_nodes,
            p_nodes,
        );

        decode_gains(h_bs, gain_set.gain_coding_profile as i32, n_nodes, p_nodes);
    }
    *p_n_nodes = n_nodes as u8;
}

/// `_readDrcGainSequence(hBs, gainSet, frameSize, timeDeltaMin, pNNodes, pNodes)`
fn read_drc_gain_sequence(
    h_bs: &mut FdkBitStream,
    gain_set: &GainSet,
    frame_size: i32,
    time_delta_min: i32,
    p_n_nodes: &mut u8,
    p_nodes: &mut [GainNode; 16],
) {
    let mut time_buf_prev_frame = [0i16; 16];
    let mut time_buf_cur_frame = [0i16; 16];

    if gain_set.gain_coding_profile as i32 == GCP_CONSTANT {
        *p_n_nodes = 1;
        p_nodes[0].time = frame_size.wrapping_sub(1) as i16;
        p_nodes[0].gain_db = 0;
    } else {
        read_nodes(h_bs, gain_set, frame_size, time_delta_min, p_n_nodes, p_nodes);

        /* count number of nodes in node reservoir */
        let mut n_nodes_node_res = 0usize;
        let mut n_nodes_cur = 0usize;
        /* count and buffer nodes from node reservoir */
        for k in 0..*p_n_nodes as usize {
            if k >= 16 {
                continue;
            }
            if p_nodes[k].time as i32 >= frame_size {
                /* write node reservoir times into buffer */
                time_buf_prev_frame[n_nodes_node_res] = p_nodes[k].time;
                n_nodes_node_res += 1;
            } else {
                /* times from current frame */
                time_buf_cur_frame[n_nodes_cur] = p_nodes[k].time;
                n_nodes_cur += 1;
            }
        }
        /* compose right time order (bit reservoir first) */
        let mut k = 0;
        while k < n_nodes_node_res {
            /* subtract two time frameSize: one to remove node reservoir offset and one to get
             * the negative index relative to the current frame */
            p_nodes[k].time =
                (time_buf_prev_frame[k] as i32).wrapping_sub(frame_size.wrapping_mul(2)) as i16;
            k += 1;
        }
        /* ...and times from current frame */
        for m in 0..n_nodes_cur {
            p_nodes[k].time = time_buf_cur_frame[m];
            k += 1;
        }
    }
}

/// `_readUniDrcGainExtension(hBs, pExt)`
fn read_uni_drc_gain_extension(
    h_bs: &mut FdkBitStream,
    p_ext: &mut UniDrcGainExtension,
) -> DrcError {
    let err = DE_OK;

    let mut k = 0usize;
    p_ext.uni_drc_gain_ext_type[k] = h_bs.read_bits(4) as u8;
    while p_ext.uni_drc_gain_ext_type[k] != UNIDRCGAINEXT_TERM {
        if k >= (8 - 1) {
            return DE_MEMORY_ERROR;
        }
        let bit_size_len = h_bs.read_bits(3) as i32;
        let ext_size_bits = bit_size_len + 4;

        let bit_size = h_bs.read_bits(ext_size_bits as u32) as i32;
        p_ext.ext_bit_size[k] = (bit_size + 1) as u32;

        /* add future extensions here */
        h_bs.push_for(p_ext.ext_bit_size[k]);
        k += 1;
        p_ext.uni_drc_gain_ext_type[k] = h_bs.read_bits(4) as u8;
    }

    err
}

/// `drcDec_readUniDrcGain(hBs, hUniDrcConfig, frameSize, deltaTminDefault, hUniDrcGain)`
pub fn drc_dec_read_uni_drc_gain(
    h_bs: &mut FdkBitStream,
    h_uni_drc_config: &UniDrcConfig,
    frame_size: i32,
    delta_tmin_default: i32,
    h_uni_drc_gain: &mut UniDrcGain,
) -> DrcError {
    let mut err = DE_OK;
    let p_coef = select_drc_coefficients(h_uni_drc_config, LOCATION_SELECTED)
        .map(|c| &h_uni_drc_config.drc_coefficients_uni_drc[c]);
    h_uni_drc_gain.status = 0;
    let gain_sequence_count = match p_coef {
        Some(c) => c.gain_sequence_count.min(12) as usize,
        None => 0,
    };

    for seq in 0..gain_sequence_count {
        // p_coef is Some because gain_sequence_count > 0.
        let Some(coef) = p_coef else { break };
        let index = coef.gain_set_index_for_gain_sequence[seq];
        let mut tmp_n_nodes: u8 = 0;
        let mut tmp_nodes = [GainNode::ZERO; 16];

        if (index >= coef.gain_set_count) || (index >= 12) {
            return DE_NOT_OK;
        }
        let gain_set = &coef.gain_set[index as usize];

        let time_delta_min = get_time_delta_min(gain_set, delta_tmin_default);

        read_drc_gain_sequence(
            h_bs,
            gain_set,
            frame_size,
            time_delta_min,
            &mut tmp_n_nodes,
            &mut tmp_nodes,
        );

        h_uni_drc_gain.n_nodes[seq] = tmp_n_nodes;
        let n = tmp_n_nodes.min(16) as usize;
        h_uni_drc_gain.gain_node[seq][..n].copy_from_slice(&tmp_nodes[..n]);
    }

    if let Some(coef) = p_coef {
        if gain_sequence_count == coef.gain_sequence_count as usize {
            /* all sequences have been read */
            h_uni_drc_gain.uni_drc_gain_ext_present = h_bs.read_bits(1) as u8;
            if h_uni_drc_gain.uni_drc_gain_ext_present == 1 {
                err = read_uni_drc_gain_extension(h_bs, &mut h_uni_drc_gain.uni_drc_gain_extension);
                if err != 0 {
                    return err;
                }
            }
        }
    }

    if err == DE_OK && gain_sequence_count > 0 {
        h_uni_drc_gain.status = 1;
    }
    err
}

/****************/
/* uniDrcConfig */
/****************/

/// `_decodeDuckingModification(hBs, pDMod, isBox)`
fn decode_ducking_modification(
    h_bs: &mut FdkBitStream,
    p_dmod: &mut DuckingModification,
    is_box: i32,
) {
    if is_box != 0 {
        h_bs.push_for(7); /* reserved */
    }
    p_dmod.ducking_scaling_present = h_bs.read_bits(1) as u8;

    if p_dmod.ducking_scaling_present != 0 {
        if is_box != 0 {
            h_bs.push_for(4); /* reserved */
        }
        let bs_ducking_scaling = h_bs.read_bits(4) as i32;
        let sigma = bs_ducking_scaling >> 3;
        let mu = bs_ducking_scaling & 0x7;

        if sigma != 0 {
            /* 1.0 - 0.125 * (1 + mu); */
            p_dmod.ducking_scaling = ((7 - mu) << (FRACT_BITS - 1 - 3 - 2)) as FixpSgl;
        } else {
            /* 1.0 + 0.125 * (1 + mu); */
            p_dmod.ducking_scaling = ((9 + mu) << (FRACT_BITS - 1 - 3 - 2)) as FixpSgl;
        }
    } else {
        p_dmod.ducking_scaling = (1 << (FRACT_BITS - 1 - 2)) as FixpSgl; /* 1.0 */
    }
}

/// `_decodeGainModification(hBs, version, bandCount, pGMod, isBox)`
///
/// `pGMod` points to the 4 gain modifications of a channel group. Bands beyond 4 have no
/// defined reference result; the port ignores them.
fn decode_gain_modification(
    h_bs: &mut FdkBitStream,
    version: i32,
    mut band_count: i32,
    p_gmod: &mut [GainModification; 4],
    is_box: i32,
) {
    if version > 0 {
        if is_box != 0 {
            h_bs.push_for(4); /* reserved */
            band_count = h_bs.read_bits(4) as i32;
        }

        for b in 0..band_count.max(0) as usize {
            let mut dummy = GainModification::ZERO;
            let gm = p_gmod.get_mut(b).unwrap_or(&mut dummy);
            if is_box != 0 {
                h_bs.push_for(4); /* reserved */
                gm.target_characteristic_left_present = h_bs.read_bits(1) as u8;
                gm.target_characteristic_right_present = h_bs.read_bits(1) as u8;
                gm.gain_scaling_present = h_bs.read_bits(1) as u8;
                gm.gain_offset_present = h_bs.read_bits(1) as u8;
            }

            if is_box == 0 {
                gm.target_characteristic_left_present = h_bs.read_bits(1) as u8;
            }
            if gm.target_characteristic_left_present != 0 {
                if is_box != 0 {
                    h_bs.push_for(4); /* reserved */
                }
                gm.target_characteristic_left_index = h_bs.read_bits(4) as u8;
            }
            if is_box == 0 {
                gm.target_characteristic_right_present = h_bs.read_bits(1) as u8;
            }
            if gm.target_characteristic_right_present != 0 {
                if is_box != 0 {
                    h_bs.push_for(4); /* reserved */
                }
                gm.target_characteristic_right_index = h_bs.read_bits(4) as u8;
            }
            if is_box == 0 {
                gm.gain_scaling_present = h_bs.read_bits(1) as u8;
            }
            if gm.gain_scaling_present != 0 {
                let bs_attenuation_scaling = h_bs.read_bits(4) as i32;
                /* bsAttenuationScaling * 0.125; */
                gm.attenuation_scaling =
                    (bs_attenuation_scaling << (FRACT_BITS - 1 - 3 - 2)) as FixpSgl;
                let bs_amplification_scaling = h_bs.read_bits(4) as i32;
                /* bsAmplificationScaling * 0.125; */
                gm.amplification_scaling =
                    (bs_amplification_scaling << (FRACT_BITS - 1 - 3 - 2)) as FixpSgl;
            }
            if is_box == 0 {
                gm.gain_offset_present = h_bs.read_bits(1) as u8;
            }
            if gm.gain_offset_present != 0 {
                if is_box != 0 {
                    h_bs.push_for(2); /* reserved */
                }
                let sign = h_bs.read_bits(1);
                let bs_gain_offset = h_bs.read_bits(5) as i32;
                /* (1+bsGainOffset) * 0.25; */
                gm.gain_offset = ((1 + bs_gain_offset) << (FRACT_BITS - 1 - 2 - 4)) as FixpSgl;
                if sign != 0 {
                    gm.gain_offset = gm.gain_offset.wrapping_neg();
                }
            }
        }
        if band_count == 1 {
            let shape_filter_present = h_bs.read_bits(1);
            if shape_filter_present != 0 {
                if is_box != 0 {
                    h_bs.push_for(3); /* reserved */
                }
                h_bs.push_for(4); /* pGMod->shapeFilterIndex */
            } else {
                if is_box != 0 {
                    h_bs.push_for(7); /* reserved */
                }
            }
        }
    } else {
        let mut attenuation_scaling: FixpSgl = fl2fxconst_sgl((1.0f32 / 4.0f32) as f64);
        let mut amplification_scaling: FixpSgl = fl2fxconst_sgl((1.0f32 / 4.0f32) as f64);
        let mut gain_offset: FixpSgl = 0;
        if is_box != 0 {
            h_bs.push_for(7); /* reserved */
        }
        let gain_scaling_present = h_bs.read_bits(1) as u8;
        if gain_scaling_present != 0 {
            let bs_attenuation_scaling = h_bs.read_bits(4) as i32;
            /* bsAttenuationScaling * 0.125; */
            attenuation_scaling = (bs_attenuation_scaling << (FRACT_BITS - 1 - 3 - 2)) as FixpSgl;
            let bs_amplification_scaling = h_bs.read_bits(4) as i32;
            /* bsAmplificationScaling * 0.125; */
            amplification_scaling =
                (bs_amplification_scaling << (FRACT_BITS - 1 - 3 - 2)) as FixpSgl;
        }
        if is_box != 0 {
            h_bs.push_for(7); /* reserved */
        }
        let gain_offset_present = h_bs.read_bits(1) as u8;
        if gain_offset_present != 0 {
            if is_box != 0 {
                h_bs.push_for(2); /* reserved */
            }
            let sign = h_bs.read_bits(1);
            let bs_gain_offset = h_bs.read_bits(5) as i32;
            /* (1+bsGainOffset) * 0.25; */
            gain_offset = ((1 + bs_gain_offset) << (FRACT_BITS - 1 - 2 - 4)) as FixpSgl;
            if sign != 0 {
                gain_offset = gain_offset.wrapping_neg();
            }
        }
        for gm in p_gmod.iter_mut() {
            gm.target_characteristic_left_present = 0;
            gm.target_characteristic_right_present = 0;
            gm.gain_scaling_present = gain_scaling_present;
            gm.attenuation_scaling = attenuation_scaling;
            gm.amplification_scaling = amplification_scaling;
            gm.gain_offset_present = gain_offset_present;
            gm.gain_offset = gain_offset;
        }
    }
}

/// `_readDrcCharacteristic(hBs, version, pDChar, isBox)`
fn read_drc_characteristic(
    h_bs: &mut FdkBitStream,
    version: i32,
    p_dchar: &mut DrcCharacteristic,
    is_box: i32,
) {
    if version == 0 {
        if is_box != 0 {
            h_bs.push_for(1); /* reserved */
        }
        p_dchar.cicp_index = h_bs.read_bits(7) as u8;
        if p_dchar.cicp_index > 0 {
            p_dchar.present = 1;
            p_dchar.is_cicp = 1;
        } else {
            p_dchar.present = 0;
        }
    } else {
        p_dchar.present = h_bs.read_bits(1) as u8;
        if is_box != 0 {
            p_dchar.is_cicp = h_bs.read_bits(1) as u8;
        }
        if p_dchar.present != 0 {
            if is_box == 0 {
                p_dchar.is_cicp = h_bs.read_bits(1) as u8;
            }
            if p_dchar.is_cicp != 0 {
                if is_box != 0 {
                    h_bs.push_for(1); /* reserved */
                }
                p_dchar.cicp_index = h_bs.read_bits(7) as u8;
            } else {
                p_dchar.set_custom_left(h_bs.read_bits(4) as u8);
                p_dchar.custom_right = h_bs.read_bits(4) as u8;
            }
        }
    }
}

/// `_readBandBorder(hBs, pBBord, drcBandType, isBox)`
fn read_band_border(
    h_bs: &mut FdkBitStream,
    p_bbord: &mut BandBorder,
    drc_band_type: i32,
    is_box: i32,
) {
    if drc_band_type != 0 {
        if is_box != 0 {
            h_bs.push_for(4); /* reserved */
        }
        p_bbord.set_crossover_freq_index(h_bs.read_bits(4) as u8);
    } else {
        if is_box != 0 {
            h_bs.push_for(6); /* reserved */
        }
        p_bbord.start_sub_band_index = h_bs.read_bits(10) as u16;
    }
}

/// `_readGainSet(hBs, version, gainSequenceIndex, pGSet, isBox)`
fn read_gain_set(
    h_bs: &mut FdkBitStream,
    version: i32,
    gain_sequence_index: &mut i32,
    p_gset: &mut GainSet,
    is_box: i32,
) -> DrcError {
    if is_box != 0 {
        h_bs.push_for(2); /* reserved */
    }
    p_gset.gain_coding_profile = h_bs.read_bits(2) as u8;
    p_gset.gain_interpolation_type = h_bs.read_bits(1) as u8;
    p_gset.full_frame = h_bs.read_bits(1) as u8;
    p_gset.time_alignment = h_bs.read_bits(1) as u8;
    p_gset.time_delta_min_present = h_bs.read_bits(1) as u8;

    if p_gset.time_delta_min_present != 0 {
        if is_box != 0 {
            h_bs.push_for(5); /* reserved */
        }
        let bs_time_delta_min = h_bs.read_bits(11) as i32;
        p_gset.time_delta_min = (bs_time_delta_min + 1) as u16;
    }

    if p_gset.gain_coding_profile as i32 != GCP_CONSTANT {
        if is_box != 0 {
            h_bs.push_for(3); /* reserved */
        }
        p_gset.band_count = h_bs.read_bits(4) as u8;
        if p_gset.band_count > 4 {
            return DE_MEMORY_ERROR;
        }

        if (p_gset.band_count > 1) || is_box != 0 {
            p_gset.drc_band_type = h_bs.read_bits(1) as u8;
        }

        for i in 0..p_gset.band_count as usize {
            if version == 0 {
                *gain_sequence_index += 1;
            } else {
                let index_present = if is_box != 0 { 1 } else { h_bs.read_bits(1) };
                if index_present != 0 {
                    let bs_index = h_bs.read_bits(6) as i32;
                    *gain_sequence_index = bs_index;
                } else {
                    *gain_sequence_index += 1;
                }
            }
            p_gset.gain_sequence_index[i] = *gain_sequence_index as u8;
            read_drc_characteristic(h_bs, version, &mut p_gset.drc_characteristic[i], is_box);
        }
        for i in 1..p_gset.band_count as usize {
            let drc_band_type = p_gset.drc_band_type as i32;
            read_band_border(h_bs, &mut p_gset.band_border[i], drc_band_type, is_box);
        }
    } else {
        p_gset.band_count = 1;
        *gain_sequence_index += 1;
        p_gset.gain_sequence_index[0] = *gain_sequence_index as u8;
    }

    DE_OK
}

/// `FL2FXCONST_SGL(0.05f / (float)(1 << 2))`
const IO_RATIO_OFFSET: FixpSgl = fl2fxconst_sgl((0.05f32 / 4.0f32) as f64);

/// `_readCustomDrcCharacteristic(hBs, side, pCharacteristicFormat, pCChar, isBox)`
fn read_custom_drc_characteristic(
    h_bs: &mut FdkBitStream,
    side: i32,
    p_characteristic_format: &mut u8,
    p_cchar: &mut CustomDrcChar,
    is_box: i32,
) -> DrcError {
    if is_box != 0 {
        h_bs.push_for(7); /* reserved */
    }
    *p_characteristic_format = h_bs.read_bits(1) as u8;
    if *p_characteristic_format as i32 == CF_SIGMOID {
        if is_box != 0 {
            h_bs.push_for(1); /* reserved */
        }
        let sigmoid = p_cchar.sigmoid_mut();
        let bs_gain = h_bs.read_bits(6) as i32;
        if side == CS_LEFT {
            sigmoid.gain = (bs_gain << (FRACT_BITS - 1 - 6)) as FixpSgl;
        } else {
            sigmoid.gain = ((-bs_gain) << (FRACT_BITS - 1 - 6)) as FixpSgl;
        }
        let bs_io_ratio = h_bs.read_bits(4) as i32;
        /* pCChar->sigmoid.ioRatio = 0.05 + 0.15 * bsIoRatio; */
        sigmoid.io_ratio = (IO_RATIO_OFFSET as i32
            + ((((3 * bs_io_ratio) << (FRACT_BITS - 1)) / 5) >> 4) as FixpSgl as i32)
            as FixpSgl;
        let bs_exp = h_bs.read_bits(4) as i32;
        if bs_exp < 15 {
            sigmoid.exp = ((1 + 2 * bs_exp) << (FRACT_BITS - 1 - 5)) as FixpSgl;
        } else {
            sigmoid.exp = MAXVAL_SGL as FixpSgl; /* represents infinity */
        }
        sigmoid.flip_sign = h_bs.read_bits(1) as u8;
    } else {
        /* CF_NODES */
        if is_box != 0 {
            h_bs.push_for(6); /* reserved */
        }
        let nodes = p_cchar.nodes_mut();
        let bs_characteristic_node_count = h_bs.read_bits(2) as i32;
        nodes.characteristic_node_count = (bs_characteristic_node_count + 1) as u8;
        if nodes.characteristic_node_count > 4 {
            return DE_MEMORY_ERROR;
        }
        nodes.node_level[0] = DRC_INPUT_LOUDNESS_TARGET_SGL;
        nodes.node_gain[0] = 0;
        for i in 0..nodes.characteristic_node_count as usize {
            if is_box != 0 {
                h_bs.push_for(3); /* reserved */
            }
            let bs_node_level_delta = h_bs.read_bits(5) as i32;
            let delta = ((1 + bs_node_level_delta) << (FRACT_BITS - 1 - 7)) as FixpSgl;
            if side == CS_LEFT {
                nodes.node_level[i + 1] = nodes.node_level[i].wrapping_sub(delta);
            } else {
                nodes.node_level[i + 1] = nodes.node_level[i].wrapping_add(delta);
            }
            let bs_node_gain = h_bs.read_bits(8) as i32;
            /* 0.5f * bsNodeGain - 64.0f; */
            nodes.node_gain[i + 1] = ((bs_node_gain - 128) << (FRACT_BITS - 1 - 1 - 7)) as FixpSgl;
        }
    }
    DE_OK
}

/// `_skipLoudEqInstructions(hBs)`
fn skip_loud_eq_instructions(h_bs: &mut FdkBitStream) {
    h_bs.push_for(4); /* loudEqSetId */
    h_bs.push_for(4); /* drcLocation */
    let downmix_id_present = h_bs.read_bits(1);
    if downmix_id_present != 0 {
        h_bs.push_for(7); /* downmixId */
        let additional_downmix_id_present = h_bs.read_bits(1);
        if additional_downmix_id_present != 0 {
            let additional_downmix_id_count = h_bs.read_bits(7);
            for _ in 0..additional_downmix_id_count {
                h_bs.push_for(7); /* additionalDownmixId */
            }
        }
    }

    let drc_set_id_present = h_bs.read_bits(1);
    if drc_set_id_present != 0 {
        h_bs.push_for(6); /* drcSetId */
        let additional_drc_set_id_present = h_bs.read_bits(1);
        if additional_drc_set_id_present != 0 {
            let additional_drc_set_id_count = h_bs.read_bits(6);
            for _ in 0..additional_drc_set_id_count {
                h_bs.push_for(6); /* additionalDrcSetId; */
            }
        }
    }

    let eq_set_id_present = h_bs.read_bits(1);
    if eq_set_id_present != 0 {
        h_bs.push_for(6); /* eqSetId */
        let additional_eq_set_id_present = h_bs.read_bits(1);
        if additional_eq_set_id_present != 0 {
            let additional_eq_set_id_count = h_bs.read_bits(6);
            for _ in 0..additional_eq_set_id_count {
                h_bs.push_for(6); /* additionalEqSetId; */
            }
        }
    }

    h_bs.push_for(1); /* loudnessAfterDrc */
    h_bs.push_for(1); /* loudnessAfterEq */
    let loud_eq_gain_sequence_count = h_bs.read_bits(6);
    for _ in 0..loud_eq_gain_sequence_count {
        h_bs.push_for(6); /* gainSequenceIndex */
        let drc_characteristic_format_is_cicp = h_bs.read_bits(1);
        if drc_characteristic_format_is_cicp != 0 {
            h_bs.push_for(7); /* drcCharacteristic */
        } else {
            h_bs.push_for(4); /* drcCharacteristicLeftIndex */
            h_bs.push_for(4); /* drcCharacteristicRightIndex */
        }
        h_bs.push_for(6); /* frequencyRangeIndex */
        h_bs.push_for(3); /* bsLoudEqScaling */
        h_bs.push_for(5); /* bsLoudEqOffset */
    }
}

/// `_skipEqSubbandGainSpline(hBs)`
fn skip_eq_subband_gain_spline(h_bs: &mut FdkBitStream) {
    let mut n_eq_nodes = h_bs.read_bits(5) as i32;
    n_eq_nodes += 2;
    for _ in 0..n_eq_nodes {
        let bits = h_bs.read_bits(1);
        if bits == 0 {
            h_bs.push_for(4);
        }
    }
    h_bs.push_for((4 * (n_eq_nodes - 1)) as u32);
    let bits = h_bs.read_bits(2);
    match bits {
        0 => h_bs.push_for(5),
        1 | 2 => h_bs.push_for(4),
        3 => h_bs.push_for(3),
        _ => {}
    }
    h_bs.push_for((5 * (n_eq_nodes - 1)) as u32);
}

/// `_skipEqCoefficients(hBs)`
fn skip_eq_coefficients(h_bs: &mut FdkBitStream) {
    let eq_delay_max_present = h_bs.read_bits(1);
    if eq_delay_max_present != 0 {
        h_bs.push_for(8); /* bsEqDelayMax */
    }

    let unique_filter_block_count = h_bs.read_bits(6);
    for _ in 0..unique_filter_block_count {
        let filter_element_count = h_bs.read_bits(6);
        for _ in 0..filter_element_count {
            h_bs.push_for(6); /* filterElementIndex */
            let filter_element_gain_present = h_bs.read_bits(1);
            if filter_element_gain_present != 0 {
                h_bs.push_for(10); /* bsFilterElementGain */
            }
        }
    }
    let unique_td_filter_element_count = h_bs.read_bits(6);
    for _ in 0..unique_td_filter_element_count {
        let eq_filter_format = h_bs.read_bits(1);
        if eq_filter_format == 0 {
            /* pole/zero */
            let bs_real_zero_radius_one_count = h_bs.read_bits(3);
            let real_zero_count = h_bs.read_bits(6);
            let generic_zero_count = h_bs.read_bits(6);
            let real_pole_count = h_bs.read_bits(4);
            let complex_pole_count = h_bs.read_bits(4);
            h_bs.push_for(2 * bs_real_zero_radius_one_count * 1);
            h_bs.push_for(real_zero_count * 8);
            h_bs.push_for(generic_zero_count * 14);
            h_bs.push_for(real_pole_count * 8);
            h_bs.push_for(complex_pole_count * 14);
        } else {
            /* FIR coefficients */
            let fir_filter_order = h_bs.read_bits(7);
            h_bs.push_for(1);
            h_bs.push_for((fir_filter_order / 2 + 1) * 11);
        }
    }
    let unique_eq_subband_gains_count = h_bs.read_bits(6);
    if unique_eq_subband_gains_count > 0 {
        let eq_subband_gain_representation = h_bs.read_bits(1);
        let eq_subband_gain_format = h_bs.read_bits(4) as i32;
        let eq_subband_gain_count: u32 = match eq_subband_gain_format {
            GF_QMF32 => 32,
            GF_QMFHYBRID39 => 39,
            GF_QMF64 => 64,
            GF_QMFHYBRID71 => 71,
            GF_QMF128 => 128,
            GF_QMFHYBRID135 => 135,
            _ => {
                /* GF_UNIFORM, default */
                h_bs.read_bits(8) + 1
            }
        };
        for _ in 0..unique_eq_subband_gains_count {
            if eq_subband_gain_representation == 1 {
                skip_eq_subband_gain_spline(h_bs);
            } else {
                h_bs.push_for(eq_subband_gain_count * 9);
            }
        }
    }
}

/// `_skipTdFilterCascade(hBs, eqChannelGroupCount)`
fn skip_td_filter_cascade(h_bs: &mut FdkBitStream, eq_channel_group_count: i32) {
    for _ in 0..eq_channel_group_count {
        let eq_cascade_gain_present = h_bs.read_bits(1);
        if eq_cascade_gain_present != 0 {
            h_bs.push_for(10); /* bsEqCascadeGain */
        }
        let filter_block_count = h_bs.read_bits(4);
        h_bs.push_for(filter_block_count * 7); /* filterBlockIndex */
    }
    let eq_phase_alignment_present = h_bs.read_bits(1);
    if eq_phase_alignment_present != 0 {
        for i in 0..eq_channel_group_count {
            h_bs.push_for(((eq_channel_group_count - i - 1) * 1) as u32);
        }
    }
}

/// `_skipEqInstructions(hBs, hUniDrcConfig)`
fn skip_eq_instructions(h_bs: &mut FdkBitStream, h_uni_drc_config: &UniDrcConfig) -> DrcError {
    let err = DE_OK;
    let mut additional_downmix_id_count = 0i32;
    let mut eq_channel_group_for_channel = [0u8; 8];

    h_bs.push_for(6); /* eqSetId */
    h_bs.push_for(4); /* eqSetComplexityLevel */
    let downmix_id_present = h_bs.read_bits(1) as i32;
    let downmix_id;
    let eq_apply_to_downmix;
    if downmix_id_present != 0 {
        downmix_id = h_bs.read_bits(7) as i32;
        eq_apply_to_downmix = h_bs.read_bits(1) as i32;
        let additional_downmix_id_present = h_bs.read_bits(1);
        if additional_downmix_id_present != 0 {
            additional_downmix_id_count = h_bs.read_bits(7) as i32;
            h_bs.push_for((additional_downmix_id_count * 7) as u32); /* additionalDownmixId */
        }
    } else {
        downmix_id = 0;
        eq_apply_to_downmix = 0;
    }
    h_bs.push_for(6); /* drcSetId */
    let additional_drc_set_id_present = h_bs.read_bits(1);
    if additional_drc_set_id_present != 0 {
        let additional_drc_set_id_count = h_bs.read_bits(6);
        for _ in 0..additional_drc_set_id_count {
            h_bs.push_for(6); /* additionalDrcSetId */
        }
    }
    h_bs.push_for(16); /* eqSetPurpose */
    let depends_on_eq_set_present = h_bs.read_bits(1);
    if depends_on_eq_set_present != 0 {
        h_bs.push_for(6); /* dependsOnEqSet */
    } else {
        h_bs.push_for(1); /* noIndependentEqUse */
    }

    let mut channel_count = h_uni_drc_config.channel_layout.base_channel_count as i32;
    if (downmix_id_present == 1)
        && (eq_apply_to_downmix == 1)
        && (downmix_id != 0)
        && (downmix_id != DOWNMIX_ID_ANY_DOWNMIX)
        && (additional_downmix_id_count == 0)
    {
        let Some(p_down) = select_downmix_instructions(h_uni_drc_config, downmix_id) else {
            return DE_NOT_OK;
        };

        /* targetChannelCountFromDownmixId*/
        channel_count = h_uni_drc_config.downmix_instructions[p_down].target_channel_count as i32;
    } else if (downmix_id == DOWNMIX_ID_ANY_DOWNMIX) || (additional_downmix_id_count > 1) {
        channel_count = 1;
    }

    let mut eq_channel_group_count = 0;
    for c in 0..channel_count.max(0) as usize {
        let mut new_group = 1;
        if c >= 8 {
            return DE_MEMORY_ERROR;
        }
        eq_channel_group_for_channel[c] = h_bs.read_bits(7) as u8;
        for k in 0..c {
            if eq_channel_group_for_channel[c] == eq_channel_group_for_channel[k] {
                new_group = 0;
            }
        }
        if new_group == 1 {
            eq_channel_group_count += 1;
        }
    }
    let td_filter_cascade_present = h_bs.read_bits(1);
    if td_filter_cascade_present != 0 {
        skip_td_filter_cascade(h_bs, eq_channel_group_count);
    }
    let subband_gains_present = h_bs.read_bits(1);
    if subband_gains_present != 0 {
        h_bs.push_for((eq_channel_group_count * 6) as u32); /* subbandGainsIndex */
    }
    let eq_transition_duration_present = h_bs.read_bits(1);
    if eq_transition_duration_present != 0 {
        h_bs.push_for(5); /* bsEqTransitionDuration */
    }
    err
}

/// `_skipDrcCoefficientsBasic(hBs)`
fn skip_drc_coefficients_basic(h_bs: &mut FdkBitStream) {
    h_bs.push_for(4); /* drcLocation */
    h_bs.push_for(7); /* drcCharacteristic */
}

/// `_readDrcCoefficientsUniDrc(hBs, version, pCoef)`
fn read_drc_coefficients_uni_drc(
    h_bs: &mut FdkBitStream,
    version: i32,
    p_coef: &mut DrcCoefficientsUniDrc,
) -> DrcError {
    let mut err = DE_OK;
    let mut gain_sequence_index: i32 = -1;

    p_coef.drc_location = h_bs.read_bits(4) as u8;
    p_coef.drc_frame_size_present = h_bs.read_bits(1) as u8;

    if p_coef.drc_frame_size_present == 1 {
        let bs_drc_frame_size = h_bs.read_bits(15) as i32;
        p_coef.drc_frame_size = (bs_drc_frame_size + 1) as u16;
    }
    if version == 0 {
        let mut gain_sequence_count = 0i32;
        p_coef.characteristic_left_count = 0;
        p_coef.characteristic_right_count = 0;
        let gain_set_count = h_bs.read_bits(6) as i32;
        p_coef.gain_set_count = gain_set_count.min(12) as u8;
        for i in 0..gain_set_count as usize {
            let mut tmp_gset = GainSet::ZERO;
            err = read_gain_set(h_bs, version, &mut gain_sequence_index, &mut tmp_gset, 0);
            if err != 0 {
                return err;
            }
            gain_sequence_count += tmp_gset.band_count as i32;

            if i >= 12 {
                continue;
            }
            p_coef.gain_set[i] = tmp_gset;
        }
        p_coef.gain_sequence_count = gain_sequence_count as u8;
    } else {
        /* (version == 1) */
        let drc_characteristic_left_present = h_bs.read_bits(1);
        if drc_characteristic_left_present != 0 {
            p_coef.characteristic_left_count = h_bs.read_bits(4) as u8;
            if (p_coef.characteristic_left_count as i32 + 1) > 16 {
                return DE_MEMORY_ERROR;
            }
            for i in 0..p_coef.characteristic_left_count as usize {
                err = read_custom_drc_characteristic(
                    h_bs,
                    CS_LEFT,
                    &mut p_coef.characteristic_left_format[i + 1],
                    &mut p_coef.custom_characteristic_left[i + 1],
                    0,
                );
                if err != 0 {
                    return err;
                }
            }
        }
        let drc_characteristic_right_present = h_bs.read_bits(1);
        if drc_characteristic_right_present != 0 {
            p_coef.characteristic_right_count = h_bs.read_bits(4) as u8;
            if (p_coef.characteristic_right_count as i32 + 1) > 16 {
                return DE_MEMORY_ERROR;
            }
            for i in 0..p_coef.characteristic_right_count as usize {
                err = read_custom_drc_characteristic(
                    h_bs,
                    CS_RIGHT,
                    &mut p_coef.characteristic_right_format[i + 1],
                    &mut p_coef.custom_characteristic_right[i + 1],
                    0,
                );
                if err != 0 {
                    return err;
                }
            }
        }
        let shape_filters_present = h_bs.read_bits(1);
        if shape_filters_present != 0 {
            let shape_filter_count = h_bs.read_bits(4) as u8;
            for _ in 0..shape_filter_count {
                let mut tmp_present = h_bs.read_bits(1);
                if tmp_present != 0 {
                    /* lfCutParams */
                    h_bs.push_for(5);
                }

                tmp_present = h_bs.read_bits(1);
                if tmp_present != 0 {
                    /* lfBoostParams */
                    h_bs.push_for(5);
                }

                tmp_present = h_bs.read_bits(1);
                if tmp_present != 0 {
                    /* hfCutParams */
                    h_bs.push_for(5);
                }

                tmp_present = h_bs.read_bits(1);
                if tmp_present != 0 {
                    /* hfBoostParams */
                    h_bs.push_for(5);
                }
            }
        }
        p_coef.gain_sequence_count = h_bs.read_bits(6) as u8;
        let gain_set_count = h_bs.read_bits(6) as i32;
        p_coef.gain_set_count = gain_set_count.min(12) as u8;
        for i in 0..gain_set_count as usize {
            let mut tmp_gset = GainSet::ZERO;
            err = read_gain_set(h_bs, version, &mut gain_sequence_index, &mut tmp_gset, 0);
            if err != 0 {
                return err;
            }

            if i >= 12 {
                continue;
            }
            p_coef.gain_set[i] = tmp_gset;
        }
    }
    for i in 0..12 {
        p_coef.gain_set_index_for_gain_sequence[i] = 255;
    }
    for i in 0..p_coef.gain_set_count as usize {
        for b in 0..p_coef.gain_set[i].band_count as usize {
            let gsi = p_coef.gain_set[i].gain_sequence_index[b];
            if gsi >= 12 {
                continue;
            }
            p_coef.gain_set_index_for_gain_sequence[gsi as usize] = i as u8;
        }
    }

    err
}

/// `_skipDrcInstructionsBasic(hBs)`
fn skip_drc_instructions_basic(h_bs: &mut FdkBitStream) {
    h_bs.push_for(6); /* drcSetId */
    h_bs.push_for(4); /* drcLocation */
    h_bs.push_for(7); /* downmixId */
    let additional_downmix_id_present = h_bs.read_bits(1);
    if additional_downmix_id_present != 0 {
        let additional_downmix_id_count = h_bs.read_bits(3);
        h_bs.push_for(7 * additional_downmix_id_count); /* additionalDownmixId */
    }

    let drc_set_effect = h_bs.read_bits(16) as i32;
    if drc_set_effect & (EB_DUCK_OTHER | EB_DUCK_SELF) == 0 {
        let limiter_peak_target_present = h_bs.read_bits(1);
        if limiter_peak_target_present != 0 {
            h_bs.push_for(8); /* bsLimiterPeakTarget */
        }
    }

    let drc_set_target_loudness_present = h_bs.read_bits(1);
    if drc_set_target_loudness_present != 0 {
        h_bs.push_for(6); /* bsDrcSetTargetLoudnessValueUpper */
        let drc_set_target_loudness_value_lower_present = h_bs.read_bits(1);
        if drc_set_target_loudness_value_lower_present != 0 {
            h_bs.push_for(6); /* bsDrcSetTargetLoudnessValueLower */
        }
    }
}

/// `_readDrcInstructionsUniDrc(hBs, version, hUniDrcConfig, pInst)`
fn read_drc_instructions_uni_drc(
    h_bs: &mut FdkBitStream,
    version: i32,
    h_uni_drc_config: &UniDrcConfig,
    p_inst: &mut DrcInstructionsUniDrc,
) -> DrcError {
    let err;
    let mut channel_group_for_channel = [0i8; 8];
    let mut ducking_modification_for_channel_group = [DuckingModification::ZERO; 8];

    p_inst.drc_set_id = h_bs.read_bits(6) as i8;
    if version == 0 {
        /* Assume all v0 DRC sets to be manageable in terms of complexity */
        p_inst.drc_set_complexity_level = 2;
    } else {
        p_inst.drc_set_complexity_level = h_bs.read_bits(4) as u8;
    }
    p_inst.drc_location = h_bs.read_bits(4) as u8;
    let downmix_id_present = if version == 0 { 1 } else { h_bs.read_bits(1) };
    if downmix_id_present != 0 {
        p_inst.downmix_id[0] = h_bs.read_bits(7) as u8;
        if version == 0 {
            if p_inst.downmix_id[0] == 0 {
                p_inst.drc_apply_to_downmix = 0;
            } else {
                p_inst.drc_apply_to_downmix = 1;
            }
        } else {
            p_inst.drc_apply_to_downmix = h_bs.read_bits(1) as u8;
        }

        let additional_downmix_id_present = h_bs.read_bits(1);
        if additional_downmix_id_present != 0 {
            let additional_downmix_id_count = h_bs.read_bits(3) as usize;
            if (1 + additional_downmix_id_count) > 8 {
                return DE_MEMORY_ERROR;
            }
            for i in 0..additional_downmix_id_count {
                p_inst.downmix_id[i + 1] = h_bs.read_bits(7) as u8;
            }
            p_inst.downmix_id_count = (1 + additional_downmix_id_count) as u8;
        } else {
            p_inst.downmix_id_count = 1;
        }
    } else {
        p_inst.downmix_id[0] = 0;
        p_inst.downmix_id_count = 1;
    }

    p_inst.drc_set_effect = h_bs.read_bits(16) as u16;

    if (p_inst.drc_set_effect as i32 & (EB_DUCK_OTHER | EB_DUCK_SELF)) == 0 {
        p_inst.limiter_peak_target_present = h_bs.read_bits(1) as u8;
        if p_inst.limiter_peak_target_present != 0 {
            let bs_limiter_peak_target = h_bs.read_bits(8) as i32;
            /* - bsLimiterPeakTarget * 0.125; */
            p_inst.limiter_peak_target =
                ((bs_limiter_peak_target << (FRACT_BITS - 1 - 3 - 5)) as FixpSgl).wrapping_neg();
        }
    }

    p_inst.drc_set_target_loudness_present = h_bs.read_bits(1) as u8;

    /* set default values */
    p_inst.drc_set_target_loudness_value_upper = 0;
    p_inst.drc_set_target_loudness_value_lower = -63;

    if p_inst.drc_set_target_loudness_present == 1 {
        let bs_drc_set_target_loudness_value_upper = h_bs.read_bits(6) as i32;
        p_inst.drc_set_target_loudness_value_upper =
            (bs_drc_set_target_loudness_value_upper - 63) as i8;
        let drc_set_target_loudness_value_lower_present = h_bs.read_bits(1);
        if drc_set_target_loudness_value_lower_present == 1 {
            let bs_drc_set_target_loudness_value_lower = h_bs.read_bits(6) as i32;
            p_inst.drc_set_target_loudness_value_lower =
                (bs_drc_set_target_loudness_value_lower - 63) as i8;
        }
    }

    p_inst.depends_on_drc_set_present = h_bs.read_bits(1) as u8;

    p_inst.set_no_independent_use(0);
    if p_inst.depends_on_drc_set_present != 0 {
        p_inst.depends_on_drc_set = h_bs.read_bits(6) as i8;
    } else {
        p_inst.set_no_independent_use(h_bs.read_bits(1) as u8);
    }

    if version == 0 {
        p_inst.requires_eq = 0;
    } else {
        p_inst.requires_eq = h_bs.read_bits(1) as u8;
    }

    let p_coef = select_drc_coefficients(h_uni_drc_config, p_inst.drc_location as i32)
        .map(|c| &h_uni_drc_config.drc_coefficients_uni_drc[c]);

    let mut channel_count = h_uni_drc_config.channel_layout.base_channel_count as i32;
    p_inst.drc_channel_count = channel_count as u8;

    if p_inst.drc_set_effect as i32 & (EB_DUCK_OTHER | EB_DUCK_SELF) != 0 {
        let mut c: i32 = 0;
        while c < channel_count {
            let bs_gain_set_index = h_bs.read_bits(6) as i32;
            if c >= 8 {
                return DE_MEMORY_ERROR;
            }
            p_inst.gain_set_index[c as usize] = (bs_gain_set_index - 1) as i8;
            decode_ducking_modification(
                h_bs,
                &mut p_inst.ducking_modification_for_channel_mut()[c as usize],
                0,
            );

            c += 1;
            let repeat_parameters = h_bs.read_bits(1);
            if repeat_parameters == 1 {
                let mut bs_repeat_parameters_count = h_bs.read_bits(5) as i32;
                bs_repeat_parameters_count += 1;
                for _ in 0..bs_repeat_parameters_count {
                    if c >= 8 {
                        return DE_MEMORY_ERROR;
                    }
                    let cu = c as usize;
                    p_inst.gain_set_index[cu] = p_inst.gain_set_index[cu - 1];
                    let dm = p_inst.ducking_modification_for_channel_mut();
                    dm[cu] = dm[cu - 1];
                    c += 1;
                }
            }
        }
        if c > channel_count {
            return DE_NOT_OK;
        }

        let gain_set_index = p_inst.gain_set_index;
        let ducking = *p_inst.ducking_modification_for_channel();
        err = derive_drc_channel_groups(
            p_inst.drc_set_effect as i32,
            p_inst.drc_channel_count as i32,
            &gain_set_index,
            Some(&ducking),
            &mut p_inst.n_drc_channel_groups,
            &mut p_inst.gain_set_index_for_channel_group,
            &mut channel_group_for_channel,
            Some(&mut ducking_modification_for_channel_group),
        );
        if err != 0 {
            return err;
        }
    } else {
        let mut derive_channel_count = 0;
        if ((version == 0) || (p_inst.drc_apply_to_downmix != 0))
            && (p_inst.downmix_id[0] as i32 != DOWNMIX_ID_BASE_LAYOUT)
            && (p_inst.downmix_id[0] as i32 != DOWNMIX_ID_ANY_DOWNMIX)
            && (p_inst.downmix_id_count == 1)
        {
            if h_uni_drc_config.downmix_instructions_count != 0 {
                let Some(p_down) =
                    select_downmix_instructions(h_uni_drc_config, p_inst.downmix_id[0] as i32)
                else {
                    return DE_NOT_OK;
                };
                /* targetChannelCountFromDownmixId*/
                channel_count =
                    h_uni_drc_config.downmix_instructions[p_down].target_channel_count as i32;
                p_inst.drc_channel_count = channel_count as u8;
            } else {
                derive_channel_count = 1;
                channel_count = 1;
            }
        } else if ((version == 0) || (p_inst.drc_apply_to_downmix != 0))
            && ((p_inst.downmix_id[0] as i32 == DOWNMIX_ID_ANY_DOWNMIX)
                || (p_inst.downmix_id_count > 1))
        {
            /* Set maximum channel count as upper border. The effective channel count is set at
             * the process function. */
            p_inst.drc_channel_count = 8;
            channel_count = 1;
        }

        let mut c: i32 = 0;
        while c < channel_count {
            let bs_gain_set_index = h_bs.read_bits(6) as i32;
            if c >= 8 {
                return DE_MEMORY_ERROR;
            }
            p_inst.gain_set_index[c as usize] = (bs_gain_set_index - 1) as i8;
            c += 1;
            let repeat_sequence_index = h_bs.read_bits(1);

            if repeat_sequence_index == 1 {
                let mut bs_repeat_sequence_count = h_bs.read_bits(5) as i32;
                bs_repeat_sequence_count += 1;
                if derive_channel_count != 0 {
                    channel_count = 1 + bs_repeat_sequence_count;
                }
                for _ in 0..bs_repeat_sequence_count {
                    if c >= 8 {
                        return DE_MEMORY_ERROR;
                    }
                    p_inst.gain_set_index[c as usize] = (bs_gain_set_index - 1) as i8;
                    c += 1;
                }
            }
        }
        if c > channel_count {
            return DE_NOT_OK;
        }
        if derive_channel_count != 0 {
            p_inst.drc_channel_count = channel_count as u8;
        }

        /* DOWNMIX_ID_ANY_DOWNMIX: channelCount is 1. Distribute gainSetIndex to all
         * channels. */
        if (p_inst.downmix_id[0] as i32 == DOWNMIX_ID_ANY_DOWNMIX) || (p_inst.downmix_id_count > 1)
        {
            // drcChannelCount is at most 8 here (8, or baseChannelCount <= 8).
            for c in 1..(p_inst.drc_channel_count as usize).min(8) {
                p_inst.gain_set_index[c] = p_inst.gain_set_index[0];
            }
        }

        let gain_set_index = p_inst.gain_set_index;
        err = derive_drc_channel_groups(
            p_inst.drc_set_effect as i32,
            p_inst.drc_channel_count as i32,
            &gain_set_index,
            None,
            &mut p_inst.n_drc_channel_groups,
            &mut p_inst.gain_set_index_for_channel_group,
            &mut channel_group_for_channel,
            None,
        );
        if err != 0 {
            return err;
        }

        for g in 0..p_inst.n_drc_channel_groups as usize {
            let set = p_inst.gain_set_index_for_channel_group[g] as i32;

            /* get bandCount */
            let band_count = match p_coef {
                Some(coef) if set < coef.gain_set_count as i32 => {
                    // set >= 0: gainSetIndexForChannelGroup holds indices >= 0 only.
                    coef.gain_set.get(set as usize).map_or(1, |gs| gs.band_count as i32)
                }
                _ => 1,
            };

            decode_gain_modification(
                h_bs,
                version,
                band_count,
                &mut p_inst.gain_modification_for_channel_group_mut()[g],
                0,
            );
        }
    }

    err
}

/// `_readChannelLayout(hBs, pChan)`
fn read_channel_layout(h_bs: &mut FdkBitStream, p_chan: &mut ChannelLayout) -> DrcError {
    let err = DE_OK;

    p_chan.base_channel_count = h_bs.read_bits(7) as u8;

    if p_chan.base_channel_count > 8 {
        return DE_NOT_OK;
    }

    p_chan.layout_signaling_present = h_bs.read_bits(1) as u8;

    if p_chan.layout_signaling_present != 0 {
        p_chan.defined_layout = h_bs.read_bits(8) as u8;

        if p_chan.defined_layout == 0 {
            for i in 0..p_chan.base_channel_count as usize {
                if i < 8 {
                    p_chan.speaker_position[i] = h_bs.read_bits(7) as u8;
                } else {
                    h_bs.push_for(7);
                }
            }
        }
    }
    err
}

/// `_readDownmixInstructions(hBs, version, pChan, pDown)`
fn read_downmix_instructions(
    h_bs: &mut FdkBitStream,
    version: i32,
    p_chan: &ChannelLayout,
    p_down: &mut DownmixInstructions,
) -> DrcError {
    let err = DE_OK;

    p_down.downmix_id = h_bs.read_bits(7) as u8;
    p_down.target_channel_count = h_bs.read_bits(7) as u8;
    p_down.target_layout = h_bs.read_bits(8) as u8;
    p_down.downmix_coefficients_present = h_bs.read_bits(1) as u8;

    if p_down.downmix_coefficients_present != 0 {
        let n_downmix_coeffs =
            p_down.target_channel_count as i32 * p_chan.base_channel_count as i32;
        if n_downmix_coeffs > 8 * 8 {
            return DE_NOT_OK;
        }
        if version == 0 {
            p_down.bs_downmix_offset = 0;
            for i in 0..n_downmix_coeffs as usize {
                /* LFE downmix coefficients are not supported. */
                p_down.downmix_coefficient[i] = DOWNMIX_COEFF[h_bs.read_bits(4) as usize];
            }
        } else {
            p_down.bs_downmix_offset = h_bs.read_bits(4) as u8;
            for i in 0..n_downmix_coeffs as usize {
                p_down.downmix_coefficient[i] = DOWNMIX_COEFF_V1[h_bs.read_bits(5) as usize];
            }
        }
    }
    err
}

/// `_readDrcExtensionV1(hBs, hUniDrcConfig)`
///
/// Note: the loop over the new `drcInstructionsUniDrc` runs over the saturated total count
/// `drcInstructionsUniDrcCount`, not over `drcInstructionsUniDrcCountV1`, as in the reference.
fn read_drc_extension_v1(h_bs: &mut FdkBitStream, h_uni_drc_config: &mut UniDrcConfig) -> DrcError {
    let mut err = DE_OK;
    let mut diff = h_uni_drc_config.diff as i32;

    let downmix_instructions_v1_present = h_bs.read_bits(1);
    if downmix_instructions_v1_present == 1 {
        diff |= comp_assign(
            &mut h_uni_drc_config.downmix_instructions_count_v1,
            h_bs.read_bits(7) as u8,
        );
        let offset = h_uni_drc_config.downmix_instructions_count_v0 as usize;
        h_uni_drc_config.downmix_instructions_count =
            ((offset + h_uni_drc_config.downmix_instructions_count_v1 as usize) as u8).min(6);
        for i in 0..h_uni_drc_config.downmix_instructions_count_v1 as usize {
            let mut tmp_down = DownmixInstructions::ZERO;
            err =
                read_downmix_instructions(h_bs, 1, &h_uni_drc_config.channel_layout, &mut tmp_down);
            if err != 0 {
                return err;
            }
            if (offset + i) >= 6 {
                continue;
            }
            if diff == 0 {
                diff |= (tmp_down != h_uni_drc_config.downmix_instructions[offset + i]) as i32;
            }
            h_uni_drc_config.downmix_instructions[offset + i] = tmp_down;
        }
    } else {
        diff |= comp_assign(&mut h_uni_drc_config.downmix_instructions_count_v1, 0);
    }

    let drc_coeffs_and_instructions_uni_drc_v1_present = h_bs.read_bits(1);
    if drc_coeffs_and_instructions_uni_drc_v1_present == 1 {
        diff |= comp_assign(
            &mut h_uni_drc_config.drc_coefficients_uni_drc_count_v1,
            h_bs.read_bits(3) as u8,
        );
        let offset = h_uni_drc_config.drc_coefficients_uni_drc_count_v0 as usize;
        h_uni_drc_config.drc_coefficients_uni_drc_count =
            ((offset + h_uni_drc_config.drc_coefficients_uni_drc_count_v1 as usize) as u8).min(2);
        for i in 0..h_uni_drc_config.drc_coefficients_uni_drc_count_v1 as usize {
            let mut tmp_coef = DrcCoefficientsUniDrc::ZERO;
            err = read_drc_coefficients_uni_drc(h_bs, 1, &mut tmp_coef);
            if err != 0 {
                return err;
            }
            if (offset + i) >= 2 {
                continue;
            }
            if diff == 0 {
                diff |= (tmp_coef != h_uni_drc_config.drc_coefficients_uni_drc[offset + i]) as i32;
            }
            h_uni_drc_config.drc_coefficients_uni_drc[offset + i] = tmp_coef;
        }

        diff |= comp_assign(
            &mut h_uni_drc_config.drc_instructions_uni_drc_count_v1,
            h_bs.read_bits(6) as u8,
        );
        let offset = h_uni_drc_config.drc_instructions_uni_drc_count as usize;
        h_uni_drc_config.drc_instructions_uni_drc_count =
            ((offset + h_uni_drc_config.drc_instructions_uni_drc_count_v1 as usize) as u8).min(12);
        for i in 0..h_uni_drc_config.drc_instructions_uni_drc_count as usize {
            let mut tmp_inst = DrcInstructionsUniDrc::ZERO;
            err = read_drc_instructions_uni_drc(h_bs, 1, h_uni_drc_config, &mut tmp_inst);
            if err != 0 {
                return err;
            }
            if (offset + i) >= 12 {
                continue;
            }
            if diff == 0 {
                diff |= (tmp_inst != h_uni_drc_config.drc_instructions_uni_drc[offset + i]) as i32;
            }
            h_uni_drc_config.drc_instructions_uni_drc[offset + i] = tmp_inst;
        }
    } else {
        diff |= comp_assign(&mut h_uni_drc_config.drc_coefficients_uni_drc_count_v1, 0);
        diff |= comp_assign(&mut h_uni_drc_config.drc_instructions_uni_drc_count_v1, 0);
    }

    let loud_eq_instructions_present = h_bs.read_bits(1);
    if loud_eq_instructions_present == 1 {
        let loud_eq_instructions_count = h_bs.read_bits(4);
        for _ in 0..loud_eq_instructions_count {
            skip_loud_eq_instructions(h_bs);
        }
    }

    let eq_present = h_bs.read_bits(1);
    if eq_present == 1 {
        skip_eq_coefficients(h_bs);
        let eq_instructions_count = h_bs.read_bits(4);
        for _ in 0..eq_instructions_count {
            skip_eq_instructions(h_bs, h_uni_drc_config);
        }
    }

    h_uni_drc_config.diff = diff as u8;

    err
}

/// `_readUniDrcConfigExtension(hBs, hUniDrcConfig)`
fn read_uni_drc_config_extension(
    h_bs: &mut FdkBitStream,
    h_uni_drc_config: &mut UniDrcConfig,
) -> DrcError {
    let mut err = DE_OK;

    let mut k = 0usize;
    h_uni_drc_config.uni_drc_config_ext.uni_drc_config_ext_type[k] = h_bs.read_bits(4) as u8;
    while h_uni_drc_config.uni_drc_config_ext.uni_drc_config_ext_type[k] != UNIDRCCONFEXT_TERM {
        if k >= (8 - 1) {
            return DE_MEMORY_ERROR;
        }
        let bit_size_len = h_bs.read_bits(4) as i32;
        let ext_size_bits = bit_size_len + 4;

        let bit_size = h_bs.read_bits(ext_size_bits as u32) as i32;
        h_uni_drc_config.uni_drc_config_ext.ext_bit_size[k] = (bit_size + 1) as u32;
        let n_bits_remaining = h_bs.get_valid_bits() as i32;

        match h_uni_drc_config.uni_drc_config_ext.uni_drc_config_ext_type[k] {
            UNIDRCCONFEXT_V1 => {
                err = read_drc_extension_v1(h_bs, h_uni_drc_config);
                if err != 0 {
                    return err;
                }
                if n_bits_remaining
                    != (h_uni_drc_config.uni_drc_config_ext.ext_bit_size[k] as i32)
                        .wrapping_add(h_bs.get_valid_bits() as i32)
                {
                    return DE_NOT_OK;
                }
            }
            /* UNIDRCCONFEXT_PARAM_DRC: add future extensions here */
            _ => {
                let _ = UNIDRCCONFEXT_PARAM_DRC;
                h_bs.push_for(h_uni_drc_config.uni_drc_config_ext.ext_bit_size[k]);
            }
        }
        k += 1;
        h_uni_drc_config.uni_drc_config_ext.uni_drc_config_ext_type[k] = h_bs.read_bits(4) as u8;
    }

    err
}

/// `drcDec_readUniDrcConfig(hBs, hUniDrcConfig)`
pub fn drc_dec_read_uni_drc_config(
    h_bs: &mut FdkBitStream,
    h_uni_drc_config: &mut UniDrcConfig,
) -> DrcError {
    let mut err;
    let mut diff = 0i32;
    let mut tmp_chan = ChannelLayout::ZERO;

    diff |= comp_assign(&mut h_uni_drc_config.sample_rate_present, h_bs.read_bits(1) as u8);

    if h_uni_drc_config.sample_rate_present == 1 {
        diff |= comp_assign(&mut h_uni_drc_config.sample_rate, h_bs.read_bits(18) + 1000);
    }

    diff |=
        comp_assign(&mut h_uni_drc_config.downmix_instructions_count_v0, h_bs.read_bits(7) as u8);

    let drc_description_basic_present = h_bs.read_bits(1);
    let drc_coefficients_basic_count;
    let drc_instructions_basic_count;
    if drc_description_basic_present == 1 {
        drc_coefficients_basic_count = h_bs.read_bits(3);
        drc_instructions_basic_count = h_bs.read_bits(4);
    } else {
        drc_coefficients_basic_count = 0;
        drc_instructions_basic_count = 0;
    }

    diff |= comp_assign(
        &mut h_uni_drc_config.drc_coefficients_uni_drc_count_v0,
        h_bs.read_bits(3) as u8,
    );
    diff |= comp_assign(
        &mut h_uni_drc_config.drc_instructions_uni_drc_count_v0,
        h_bs.read_bits(6) as u8,
    );

    err = read_channel_layout(h_bs, &mut tmp_chan);
    if err != 0 {
        return err;
    }

    if diff == 0 {
        diff |= (tmp_chan != h_uni_drc_config.channel_layout) as i32;
    }
    h_uni_drc_config.channel_layout = tmp_chan;

    h_uni_drc_config.downmix_instructions_count =
        h_uni_drc_config.downmix_instructions_count_v0.min(6);
    for i in 0..h_uni_drc_config.downmix_instructions_count_v0 as usize {
        let mut tmp_down = DownmixInstructions::ZERO;
        err = read_downmix_instructions(h_bs, 0, &h_uni_drc_config.channel_layout, &mut tmp_down);
        if err != 0 {
            return err;
        }
        if i >= 6 {
            continue;
        }
        if diff == 0 {
            diff |= (tmp_down != h_uni_drc_config.downmix_instructions[i]) as i32;
        }
        h_uni_drc_config.downmix_instructions[i] = tmp_down;
    }

    for _ in 0..drc_coefficients_basic_count {
        skip_drc_coefficients_basic(h_bs);
    }
    for _ in 0..drc_instructions_basic_count {
        skip_drc_instructions_basic(h_bs);
    }

    h_uni_drc_config.drc_coefficients_uni_drc_count =
        h_uni_drc_config.drc_coefficients_uni_drc_count_v0.min(2);
    for i in 0..h_uni_drc_config.drc_coefficients_uni_drc_count_v0 as usize {
        let mut tmp_coef = DrcCoefficientsUniDrc::ZERO;
        err = read_drc_coefficients_uni_drc(h_bs, 0, &mut tmp_coef);
        if err != 0 {
            return err;
        }
        if i >= 2 {
            continue;
        }
        if diff == 0 {
            diff |= (tmp_coef != h_uni_drc_config.drc_coefficients_uni_drc[i]) as i32;
        }
        h_uni_drc_config.drc_coefficients_uni_drc[i] = tmp_coef;
    }

    h_uni_drc_config.drc_instructions_uni_drc_count =
        h_uni_drc_config.drc_instructions_uni_drc_count_v0.min(12);
    for i in 0..h_uni_drc_config.drc_instructions_uni_drc_count_v0 as usize {
        let mut tmp_inst = DrcInstructionsUniDrc::ZERO;
        err = read_drc_instructions_uni_drc(h_bs, 0, h_uni_drc_config, &mut tmp_inst);
        if err != 0 {
            return err;
        }
        if i >= 12 {
            continue;
        }
        if diff == 0 {
            diff |= (tmp_inst != h_uni_drc_config.drc_instructions_uni_drc[i]) as i32;
        }
        h_uni_drc_config.drc_instructions_uni_drc[i] = tmp_inst;
    }

    diff |= comp_assign(&mut h_uni_drc_config.uni_drc_config_ext_present, h_bs.read_bits(1) as u8);
    h_uni_drc_config.diff = diff as u8;

    if h_uni_drc_config.uni_drc_config_ext_present == 1 {
        err = read_uni_drc_config_extension(h_bs, h_uni_drc_config);
        if err != 0 {
            return err;
        }
    }

    err
}

/*******************/
/* loudnessInfoSet */
/*******************/

/// `_decodeMethodValue(hBs, methodDefinition, methodValue, isBox)`
fn decode_method_value(
    h_bs: &mut FdkBitStream,
    method_definition: u8,
    method_value: &mut FixpDbl,
    is_box: i32,
) -> DrcError {
    let val: FixpDbl;
    match method_definition as i32 {
        MD_UNKNOWN_OTHER
        | MD_PROGRAM_LOUDNESS
        | MD_ANCHOR_LOUDNESS
        | MD_MAX_OF_LOUDNESS_RANGE
        | MD_MOMENTARY_LOUDNESS_MAX
        | MD_SHORT_TERM_LOUDNESS_MAX => {
            let tmp = h_bs.read_bits(8) as i32;
            /* -57.75 + tmp * 0.25; */
            val = fl2fxconst_dbl((-57.75f32 / 128.0f32) as f64)
                .wrapping_add(tmp << (DFRACT_BITS - 1 - 2 - 7));
        }
        MD_LOUDNESS_RANGE => {
            let tmp = h_bs.read_bits(8) as i32;
            if tmp == 0 {
                val = 0;
            } else if tmp <= 128 {
                val = tmp << (DFRACT_BITS - 1 - 2 - 7); /* tmp * 0.25; */
            } else if tmp <= 204 {
                /* 0.5 * tmp - 32.0f; */
                val = (tmp << (DFRACT_BITS - 1 - 1 - 7))
                    .wrapping_sub(fl2fxconst_dbl((32.0f32 / 128.0f32) as f64));
            } else {
                /* downscale by 1 more bit to prevent overflow at intermediate result */
                /* tmp - 134.0; */
                let v = (tmp << (DFRACT_BITS - 1 - 8))
                    .wrapping_sub(fl2fxconst_dbl((134.0f32 / 256.0f32) as f64));
                val = v.wrapping_shl(1);
            }
        }
        MD_MIXING_LEVEL => {
            let tmp = h_bs.read_bits(if is_box != 0 { 8 } else { 5 }) as i32;
            /* tmp + 80.0; */
            val = (tmp << (DFRACT_BITS - 1 - 7))
                .wrapping_add(fl2fxconst_dbl((80.0f32 / 128.0f32) as f64));
        }
        MD_ROOM_TYPE => {
            let tmp = h_bs.read_bits(if is_box != 0 { 8 } else { 2 }) as i32;
            val = tmp << (DFRACT_BITS - 1 - 7); /* tmp; */
        }
        MD_SHORT_TERM_LOUDNESS => {
            let tmp = h_bs.read_bits(8) as i32;
            /* -116.0 + tmp * 0.5; */
            val = fl2fxconst_dbl((-116.0f32 / 128.0f32) as f64)
                .wrapping_add(tmp << (DFRACT_BITS - 1 - 1 - 7));
        }
        _ => return DE_NOT_OK, /* invalid methodDefinition value */
    }
    *method_value = val;
    DE_OK
}

/// `_readLoudnessMeasurement(hBs, pMeas)`
fn read_loudness_measurement(
    h_bs: &mut FdkBitStream,
    p_meas: &mut LoudnessMeasurement,
) -> DrcError {
    p_meas.method_definition = h_bs.read_bits(4) as u8;
    let err = decode_method_value(h_bs, p_meas.method_definition, &mut p_meas.method_value, 0);
    if err != 0 {
        return err;
    }
    p_meas.measurement_system = h_bs.read_bits(4) as u8;
    p_meas.reliability = h_bs.read_bits(2) as u8;

    err
}

/// `FL2FXCONST_DBL(20.0f / (float)(1 << 7))`
const PEAK_LEVEL_OFFSET: FixpDbl = fl2fxconst_dbl((20.0f32 / 128.0f32) as f64);

/// `_readLoudnessInfo(hBs, version, loudnessInfo)`
fn read_loudness_info(
    h_bs: &mut FdkBitStream,
    version: i32,
    loudness_info: &mut LoudnessInfo,
) -> DrcError {
    let mut err = DE_OK;

    loudness_info.drc_set_id = h_bs.read_bits(6) as i8;
    if version >= 1 {
        loudness_info.eq_set_id = h_bs.read_bits(6) as u8;
    } else {
        loudness_info.eq_set_id = 0;
    }
    loudness_info.downmix_id = h_bs.read_bits(7) as u8;

    loudness_info.sample_peak_level_present = h_bs.read_bits(1) as u8;
    if loudness_info.sample_peak_level_present != 0 {
        let bs_sample_peak_level = h_bs.read_bits(12) as i32;
        if bs_sample_peak_level == 0 {
            loudness_info.sample_peak_level_present = 0;
            loudness_info.sample_peak_level = 0;
        } else {
            /* 20.0 - bsSamplePeakLevel * 0.03125; */
            loudness_info.sample_peak_level =
                PEAK_LEVEL_OFFSET.wrapping_sub(bs_sample_peak_level << (DFRACT_BITS - 1 - 5 - 7));
        }
    }

    loudness_info.true_peak_level_present = h_bs.read_bits(1) as u8;
    if loudness_info.true_peak_level_present != 0 {
        let bs_true_peak_level = h_bs.read_bits(12) as i32;
        if bs_true_peak_level == 0 {
            loudness_info.true_peak_level_present = 0;
            loudness_info.true_peak_level = 0;
        } else {
            loudness_info.true_peak_level =
                PEAK_LEVEL_OFFSET.wrapping_sub(bs_true_peak_level << (DFRACT_BITS - 1 - 5 - 7));
        }
        loudness_info.true_peak_level_measurement_system = h_bs.read_bits(4) as u8;
        loudness_info.true_peak_level_reliability = h_bs.read_bits(2) as u8;
    }

    let measurement_count = h_bs.read_bits(4) as usize;
    loudness_info.measurement_count = measurement_count.min(8) as u8;
    for i in 0..measurement_count {
        let mut tmp_meas = LoudnessMeasurement::ZERO;
        err = read_loudness_measurement(h_bs, &mut tmp_meas);
        if err != 0 {
            return err;
        }
        if i >= 8 {
            continue;
        }
        loudness_info.loudness_measurement[i] = tmp_meas;
    }

    err
}

/// `_readLoudnessInfoSetExtEq(hBs, hLoudnessInfoSet)`
fn read_loudness_info_set_ext_eq(
    h_bs: &mut FdkBitStream,
    h_loudness_info_set: &mut LoudnessInfoSet,
) -> DrcError {
    let mut err = DE_OK;
    let mut diff = h_loudness_info_set.diff as i32;

    diff |=
        comp_assign(&mut h_loudness_info_set.loudness_info_album_count_v1, h_bs.read_bits(6) as u8);
    diff |= comp_assign(&mut h_loudness_info_set.loudness_info_count_v1, h_bs.read_bits(6) as u8);

    let offset = h_loudness_info_set.loudness_info_album_count_v0 as usize;
    h_loudness_info_set.loudness_info_album_count =
        ((offset + h_loudness_info_set.loudness_info_album_count_v1 as usize) as u8).min(12);
    for i in 0..h_loudness_info_set.loudness_info_album_count_v1 as usize {
        let mut tmp_loud = LoudnessInfo::ZERO;
        err = read_loudness_info(h_bs, 1, &mut tmp_loud);
        if err != 0 {
            return err;
        }
        if (offset + i) >= 12 {
            continue;
        }
        if diff == 0 {
            diff |= (tmp_loud != h_loudness_info_set.loudness_info_album[offset + i]) as i32;
        }
        h_loudness_info_set.loudness_info_album[offset + i] = tmp_loud;
    }

    let offset = h_loudness_info_set.loudness_info_count_v0 as usize;
    h_loudness_info_set.loudness_info_count =
        ((offset + h_loudness_info_set.loudness_info_count_v1 as usize) as u8).min(12);
    for i in 0..h_loudness_info_set.loudness_info_count_v1 as usize {
        let mut tmp_loud = LoudnessInfo::ZERO;
        err = read_loudness_info(h_bs, 1, &mut tmp_loud);
        if err != 0 {
            return err;
        }
        if (offset + i) >= 12 {
            continue;
        }
        if diff == 0 {
            diff |= (tmp_loud != h_loudness_info_set.loudness_info[offset + i]) as i32;
        }
        h_loudness_info_set.loudness_info[offset + i] = tmp_loud;
    }
    h_loudness_info_set.diff = diff as u8;
    err
}

/// `_readLoudnessInfoSetExtension(hBs, hLoudnessInfoSet)`
fn read_loudness_info_set_extension(
    h_bs: &mut FdkBitStream,
    h_loudness_info_set: &mut LoudnessInfoSet,
) -> DrcError {
    let mut err = DE_OK;

    let mut k = 0usize;
    h_loudness_info_set.loudness_info_set_ext.loudness_info_set_ext_type[k] =
        h_bs.read_bits(4) as u8;
    while h_loudness_info_set.loudness_info_set_ext.loudness_info_set_ext_type[k]
        != UNIDRCLOUDEXT_TERM
    {
        if k >= (8 - 1) {
            return DE_MEMORY_ERROR;
        }
        let bit_size_len = h_bs.read_bits(4) as i32;
        let ext_size_bits = bit_size_len + 4;

        let bit_size = h_bs.read_bits(ext_size_bits as u32) as i32;
        h_loudness_info_set.loudness_info_set_ext.ext_bit_size[k] = (bit_size + 1) as u32;
        let n_bits_remaining = h_bs.get_valid_bits() as i32;

        match h_loudness_info_set.loudness_info_set_ext.loudness_info_set_ext_type[k] {
            UNIDRCLOUDEXT_EQ => {
                err = read_loudness_info_set_ext_eq(h_bs, h_loudness_info_set);
                if err != 0 {
                    return err;
                }
                if n_bits_remaining
                    != (h_loudness_info_set.loudness_info_set_ext.ext_bit_size[k] as i32)
                        .wrapping_add(h_bs.get_valid_bits() as i32)
                {
                    return DE_NOT_OK;
                }
            }
            /* add future extensions here */
            _ => {
                h_bs.push_for(h_loudness_info_set.loudness_info_set_ext.ext_bit_size[k]);
            }
        }
        k += 1;
        h_loudness_info_set.loudness_info_set_ext.loudness_info_set_ext_type[k] =
            h_bs.read_bits(4) as u8;
    }

    err
}

/// `drcDec_readLoudnessInfoSet(hBs, hLoudnessInfoSet)`: parser for loudnessInfoSet()
pub fn drc_dec_read_loudness_info_set(
    h_bs: &mut FdkBitStream,
    h_loudness_info_set: &mut LoudnessInfoSet,
) -> DrcError {
    let mut err;
    let mut diff = 0i32;

    diff |=
        comp_assign(&mut h_loudness_info_set.loudness_info_album_count_v0, h_bs.read_bits(6) as u8);
    diff |= comp_assign(&mut h_loudness_info_set.loudness_info_count_v0, h_bs.read_bits(6) as u8);

    h_loudness_info_set.loudness_info_album_count =
        h_loudness_info_set.loudness_info_album_count_v0.min(12);
    for i in 0..h_loudness_info_set.loudness_info_album_count_v0 as usize {
        let mut tmp_loud = LoudnessInfo::ZERO;
        err = read_loudness_info(h_bs, 0, &mut tmp_loud);
        if err != 0 {
            return err;
        }
        if i >= 12 {
            continue;
        }
        if diff == 0 {
            diff |= (tmp_loud != h_loudness_info_set.loudness_info_album[i]) as i32;
        }
        h_loudness_info_set.loudness_info_album[i] = tmp_loud;
    }

    h_loudness_info_set.loudness_info_count = h_loudness_info_set.loudness_info_count_v0.min(12);
    for i in 0..h_loudness_info_set.loudness_info_count_v0 as usize {
        let mut tmp_loud = LoudnessInfo::ZERO;
        err = read_loudness_info(h_bs, 0, &mut tmp_loud);
        if err != 0 {
            return err;
        }
        if i >= 12 {
            continue;
        }
        if diff == 0 {
            diff |= (tmp_loud != h_loudness_info_set.loudness_info[i]) as i32;
        }
        h_loudness_info_set.loudness_info[i] = tmp_loud;
    }

    diff |= comp_assign(
        &mut h_loudness_info_set.loudness_info_set_ext_present,
        h_bs.read_bits(1) as u8,
    );
    h_loudness_info_set.diff = diff as u8;

    err = DE_OK;
    if h_loudness_info_set.loudness_info_set_ext_present != 0 {
        err = read_loudness_info_set_extension(h_bs, h_loudness_info_set);
        if err != 0 {
            return err;
        }
    }

    err
}

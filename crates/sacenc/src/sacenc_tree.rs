// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_tree.h, libSACenc/src/sacenc_tree.cpp.

//! Port of `sacenc_tree.h` and `sacenc_tree.cpp`: the tree structure of the encoder (one TTO
//! box for the 212 tree).
//!
//! The encoder passes the same hybrid buffers as the input (`pppHybrid__FDK`) and the output
//! (`pppHybridIn__FDK`) of the analysis windowing; [`fdk_sacenc_space_tree_apply`] takes the one
//! buffer and windows it in place. The parameter band offsets that the TTO boxes point to are
//! passed along (see [`crate::sacenc_paramextract`]).
//!
//! The code is identical in the `aarch64` and `x86` builds.

use fdk_aac_rs_fdk::common_fix::FixpDpk;

use crate::sacenc_bitstream::*;
use crate::sacenc_const::SACENC_MAX_NUM_BOXES;
use crate::sacenc_framewindowing::*;
use crate::sacenc_lib::*;
use crate::sacenc_paramextract::*;

mod tables;
pub use tables::*;

/// `BOX_0`
const BOX_0: usize = 0;
/// `CH_L`, `CH_R`
const CH_L: u8 = 0;
const CH_R: u8 = 1;
/// `TTO_CH_0`, `TTO_CH_1`
const TTO_CH_0: u8 = 0;
const TTO_CH_1: u8 = 1;
/// `WIN_INACTIV`, `WIN_ACTIV`
const WIN_ACTIV: u8 = 1;
/// `MAX_KEEP_FRAMECOUNT`
const MAX_KEEP_FRAMECOUNT: i32 = 100;

/// `SPACETREE_MODE`
pub type SpaceTreeMode = i32;
pub const SPACETREE_INVALID_MODE: SpaceTreeMode = 0;
pub const SPACETREE_212: SpaceTreeMode = 8;

/// `SPACE_TREE_SETUP`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpaceTreeSetup {
    pub n_param_bands: u8,
    pub b_use_coarse_quant_tto_cld: u8,
    pub b_use_coarse_quant_tto_icc: u8,
    pub quant_mode: QuantMode,
    pub mode: SpaceTreeMode,

    pub n_channels_in_max: u8,
    pub n_hybrid_bands_max: u8,
}

/// `SPACE_TREE_DESCRIPTION`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpaceTreeDescription {
    pub n_ott_boxes: u8,
    pub n_in_channels: u8,
    pub n_out_channels: u8,
}

/// `struct SPACE_TREE`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpaceTree {
    pub mode: SpaceTreeMode,
    pub descr: SpaceTreeDescription,
    pub tto_box: [HandleTtoBox; SACENC_MAX_NUM_BOXES],
    pub n_param_bands: u8,
    pub b_use_coarse_quant_tto_icc: u8,
    pub b_use_coarse_quant_tto_cld: u8,
    pub quant_mode: QuantMode,
    pub frame_count: i32,
    pub b_frame_keep: u8,

    // Intermediate buffers
    pub p_cld_prev: [[u8; MAX_NUM_PARAM_BANDS]; SACENC_MAX_NUM_BOXES],
    pub p_icc_prev: [[u8; MAX_NUM_PARAM_BANDS]; SACENC_MAX_NUM_BOXES],

    pub n_channels_in_max: u8,
    pub n_hybrid_bands_max: u8,
}

/// `HANDLE_SPACE_TREE`
pub type HandleSpaceTree = Option<Box<SpaceTree>>;

/// `TTO_DESCRIPTOR`
#[derive(Clone, Copy, Debug)]
struct TtoDescriptor {
    box_id: u8,
    in_ch1: u8,
    in_ch2: u8,
    in_ch3: u8,
    in_ch4: u8,
    w_ch1: u8,
    w_ch2: u8,
}

/// `TREE_CONFIG`
struct TreeConfigEntry {
    mode: SpaceTreeMode,
    tree_description: SpaceTreeDescription,
}

/// `TREE_SETUP`
struct TreeSetup {
    mode: SpaceTreeMode,
    n_channels_in: u8,
    #[allow(unused)]
    n_channels_out: u8,
    n_tto_boxes: u8,
    tto_descriptor: [TtoDescriptor; 1],
}

/// `treeConfigTable`, built from its dump [`TREE_CONFIG_TABLE_RAW`].
static TREE_CONFIG_TABLE: [TreeConfigEntry; 2] = tree_config_table();

const fn tree_config_table() -> [TreeConfigEntry; 2] {
    [tree_config_entry(0), tree_config_entry(1)]
}

const fn tree_config_entry(i: usize) -> TreeConfigEntry {
    let r = &TREE_CONFIG_TABLE_RAW;
    TreeConfigEntry {
        mode: r[4 * i],
        tree_description: SpaceTreeDescription {
            n_ott_boxes: r[4 * i + 1] as u8,
            n_in_channels: r[4 * i + 2] as u8,
            n_out_channels: r[4 * i + 3] as u8,
        },
    }
}

/// `treeSetupTable`, built from its dump [`TREE_SETUP_TABLE_RAW`].
static TREE_SETUP_TABLE: [TreeSetup; 2] = tree_setup_table();

const fn tree_setup_table() -> [TreeSetup; 2] {
    [tree_setup_entry(0), tree_setup_entry(1)]
}

const fn tree_setup_entry(i: usize) -> TreeSetup {
    let r = &TREE_SETUP_TABLE_RAW;
    let k = 11 * i;
    {
        TreeSetup {
            mode: r[k],
            n_channels_in: r[k + 1] as u8,
            n_channels_out: r[k + 2] as u8,
            n_tto_boxes: r[k + 3] as u8,
            tto_descriptor: [TtoDescriptor {
                box_id: r[k + 4] as u8,
                in_ch1: r[k + 5] as u8,
                in_ch2: r[k + 6] as u8,
                in_ch3: r[k + 7] as u8,
                in_ch4: r[k + 8] as u8,
                w_ch1: r[k + 9] as u8,
                w_ch2: r[k + 10] as u8,
            }],
        }
    }
}

/// `getTreeConfig(mode, pTreeDescription)` (static in C)
fn get_tree_config(
    mode: SpaceTreeMode,
    p_tree_description: &mut SpaceTreeDescription,
) -> FdkSacencError {
    for c in &TREE_CONFIG_TABLE {
        if c.mode == mode {
            *p_tree_description = c.tree_description;
            return SACENC_OK;
        }
    }
    SACENC_INIT_ERROR
}

/// `getTreeSetup(mode)` (static in C)
fn get_tree_setup(mode: SpaceTreeMode) -> Option<&'static TreeSetup> {
    TREE_SETUP_TABLE.iter().find(|s| s.mode == mode)
}

/// `fdk_sacenc_spaceTree_Open(phSpaceTree)`
pub fn fdk_sacenc_space_tree_open(ph_space_tree: Option<&mut HandleSpaceTree>) -> FdkSacencError {
    match ph_space_tree {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            let mut st = Box::<SpaceTree>::default();
            for b in st.tto_box.iter_mut() {
                fdk_sacenc_create_tto_box(Some(b));
            }
            *h = Some(st);
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_spaceTree_Init(hST, hSetup, pParameterBand2HybridBandOffset, bFrameKeep)`:
/// `has_offsets` tells whether the offset pointer is non-`NULL`.
pub fn fdk_sacenc_space_tree_init(
    h_st: &mut SpaceTree,
    h_setup: &SpaceTreeSetup,
    has_offsets: bool,
    b_frame_keep: i32,
) -> FdkSacencError {
    let b_tto_box_front_back_combin = [0u8; SACENC_MAX_NUM_BOXES];

    h_st.frame_count = 0;
    h_st.b_frame_keep = b_frame_keep as u8;

    // Init
    h_st.mode = h_setup.mode;
    h_st.n_param_bands = h_setup.n_param_bands;
    h_st.b_use_coarse_quant_tto_icc = h_setup.b_use_coarse_quant_tto_icc;
    h_st.b_use_coarse_quant_tto_cld = h_setup.b_use_coarse_quant_tto_cld;
    h_st.quant_mode = h_setup.quant_mode;
    h_st.n_channels_in_max = h_setup.n_channels_in_max;
    h_st.n_hybrid_bands_max = h_setup.n_hybrid_bands_max;

    let error = get_tree_config(h_st.mode, &mut h_st.descr);
    if error != SACENC_OK {
        return error;
    }

    match h_st.mode {
        SPACETREE_212 => {}
        _ => return SACENC_INIT_ERROR,
    }

    if h_st.descr.n_ott_boxes as usize > SACENC_MAX_NUM_BOXES {
        return SACENC_INIT_ERROR;
    }

    for b in 0..h_st.descr.n_ott_boxes as usize {
        let box_config = TtoBoxConfig {
            subband_config: h_st.n_param_bands as BoxSubbandConfig,
            b_use_coarse_quant_cld: h_st.b_use_coarse_quant_tto_cld,
            b_use_coarse_quant_icc: h_st.b_use_coarse_quant_tto_icc,
            b_use_coherence_icc_only: b_tto_box_front_back_combin[b],
            box_quant_mode: h_st.quant_mode as BoxQuantMode,
            n_hybrid_bands_max: h_st.n_hybrid_bands_max,
            b_frame_keep: h_st.b_frame_keep,
        };

        let error = match h_st.tto_box[b].as_deref_mut() {
            Some(t) => fdk_sacenc_init_tto_box(t, &box_config, has_offsets),
            None => SACENC_INVALID_HANDLE,
        };
        if error != SACENC_OK {
            return error;
        }
    }

    SACENC_OK
}

/// `SpaceTree_FrameKeep212(hST, hSTOut, avoid_keep)` (static in C)
fn space_tree_frame_keep212(h_st: &mut SpaceTree, h_st_out: &mut SpatialFrame, avoid_keep: i32) {
    let n = h_st.n_param_bands as usize;
    // The parameter bands are at most 23 (MAX_NUM_BINS) in the frame.
    let n = n.min(MAX_NUM_BINS);
    if avoid_keep == 0 {
        if h_st.frame_count % 2 == 0 {
            for pb in 0..n {
                h_st.p_icc_prev[BOX_0][pb] = h_st_out.ott_data.icc[BOX_0][0][pb] as u8;
                h_st_out.ott_data.cld[BOX_0][0][pb] = h_st.p_cld_prev[BOX_0][pb] as i8;
            }
        } else {
            for pb in 0..n {
                h_st_out.ott_data.icc[BOX_0][0][pb] = h_st.p_icc_prev[BOX_0][pb] as i8;
                h_st.p_cld_prev[BOX_0][pb] = h_st_out.ott_data.cld[BOX_0][0][pb] as u8;
            }
        }
    } else {
        for pb in 0..n {
            h_st.p_icc_prev[BOX_0][pb] = h_st_out.ott_data.icc[BOX_0][0][pb] as u8;
            h_st.p_cld_prev[BOX_0][pb] = h_st_out.ott_data.cld[BOX_0][0][pb] as u8;
        }
    }
    h_st.frame_count += 1;
    if h_st.frame_count == MAX_KEEP_FRAMECOUNT {
        h_st.frame_count = 0;
    }
}

/// `SpaceTree_FrameKeep(hST, hSTOut, avoid_keep)` (static in C)
fn space_tree_frame_keep(
    h_st: &mut SpaceTree,
    h_st_out: &mut SpatialFrame,
    avoid_keep: i32,
) -> FdkSacencError {
    match h_st.mode {
        SPACETREE_212 => {
            space_tree_frame_keep212(h_st, h_st_out, avoid_keep);
            SACENC_OK
        }
        _ => SACENC_INVALID_CONFIG,
    }
}

/// `fdk_sacenc_spaceTree_Apply(hST, paramSet, nChannelsIn, nTimeSlots, startTimeSlot,
/// nHybridBands, pFrameWindowAna__FDK, pppHybrid__FDK, pppHybridIn__FDK, hSTOut, avoid_keep,
/// pEncoderInputChScale)` with `pppHybrid__FDK == pppHybridIn__FDK` (`ppp_hybrid`, indexed
/// `[channel][time slot][band]`) and the parameter band offsets of the TTO boxes.
pub fn fdk_sacenc_space_tree_apply(
    h_st: &mut SpaceTree,
    p_parameter_band2_hybrid_band_offset: &[u8],
    param_set: i32,
    n_channels_in: i32,
    n_time_slots: i32,
    start_time_slot: i32,
    n_hybrid_bands: i32,
    p_frame_window_ana_fdk: &[FixpWin],
    ppp_hybrid: &mut [Vec<Vec<FixpDpk>>],
    h_st_out: &mut SpatialFrame,
    avoid_keep: i32,
    p_encoder_input_ch_scale: &[i32],
) -> FdkSacencError {
    //  TREE_212:
    //                         _______
    //        L -- TTO_CH_0 --|       |
    //                        | TTO_0 |-- TTO_CH_0
    //        R -- TTO_CH_1 --|_______|

    let Some(tree_setup) = get_tree_setup(h_st.mode) else {
        return SACENC_INVALID_CONFIG;
    };

    // Sanity Checks
    if (n_channels_in != tree_setup.n_channels_in as i32)
        || (n_channels_in > h_st.n_channels_in_max as i32)
        || (n_hybrid_bands > h_st.n_hybrid_bands_max as i32)
    {
        return SACENC_INVALID_CONFIG;
    }

    // Apply all TTO boxes.
    for k in 0..tree_setup.n_tto_boxes as usize {
        let p_tto = &tree_setup.tto_descriptor[k];

        let in_ch = [p_tto.in_ch1 as usize, p_tto.in_ch2 as usize];
        let out_ch = [p_tto.in_ch3 as usize, p_tto.in_ch4 as usize];
        let win = [p_tto.w_ch1, p_tto.w_ch2];

        for i in 0..2 {
            if win[i] == WIN_ACTIV {
                // pppHybrid__FDK[inCh[i]] -> pppHybridIn__FDK[outCh[i]]: the same buffer for
                // the 212 tree (inCh == outCh).
                debug_assert_eq!(in_ch[i], out_ch[i]);
                fdk_sacenc_analysis_windowing_in_place(
                    n_time_slots,
                    start_time_slot,
                    p_frame_window_ana_fdk,
                    &mut ppp_hybrid[out_ch[i]],
                    n_hybrid_bands,
                );
            }
        }

        // Calculate output downmix within last TTO box, if no TTT box is applied.
        let b = p_tto.box_id as usize;
        let ps = param_set as usize;
        let Some(tto) = h_st.tto_box[b].as_deref_mut() else {
            return SACENC_INVALID_HANDLE;
        };
        let use_bb_cues = h_st_out.b_use_bb_cues as i32;
        let SpatialFrame { ott_data, icc_lossless_data, cld_lossless_data, .. } = h_st_out;
        let error = fdk_sacenc_apply_tto_box(
            tto,
            p_parameter_band2_hybrid_band_offset,
            n_time_slots,
            start_time_slot,
            n_hybrid_bands,
            &ppp_hybrid[p_tto.in_ch3 as usize],
            &ppp_hybrid[p_tto.in_ch4 as usize],
            &mut ott_data.icc[b][ps],
            &mut icc_lossless_data.bs_quant_coarse_xxx[b][ps],
            &mut ott_data.cld[b][ps],
            &mut cld_lossless_data.bs_quant_coarse_xxx[b][ps],
            use_bb_cues,
            p_encoder_input_ch_scale[in_ch[0]],
            p_encoder_input_ch_scale[in_ch[1]],
        );
        if error != SACENC_OK {
            return error;
        }
    }

    if h_st.b_frame_keep == 1 {
        let error = space_tree_frame_keep(h_st, h_st_out, avoid_keep);
        if error != SACENC_OK {
            return error;
        }
    }

    SACENC_OK
}

/// `fdk_sacenc_spaceTree_Close(phSpaceTree)`
pub fn fdk_sacenc_space_tree_close(ph_space_tree: Option<&mut HandleSpaceTree>) -> FdkSacencError {
    match ph_space_tree {
        Some(h) if h.is_some() => {
            *h = None;
            SACENC_OK
        }
        _ => SACENC_INVALID_HANDLE,
    }
}

/// `fdk_sacenc_spaceTree_GetDescription(hSpaceTree, pSpaceTreeDescription)`
pub fn fdk_sacenc_space_tree_get_description(
    h_space_tree: &SpaceTree,
    p_space_tree_description: &mut SpaceTreeDescription,
) -> FdkSacencError {
    *p_space_tree_description = h_space_tree.descr;
    SACENC_OK
}

/// `fdk_sacenc_spaceTree_Hybrid2ParamBand(nParamBands, nHybridBand)`
pub fn fdk_sacenc_space_tree_hybrid2_param_band(n_param_bands: i32, n_hybrid_band: i32) -> i32 {
    fdk_sacenc_subband2_param_band(n_param_bands as BoxSubbandConfig, n_hybrid_band)
}

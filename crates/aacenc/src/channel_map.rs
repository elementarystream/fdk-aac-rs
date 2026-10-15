// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/channel_map.h, libAACenc/src/channel_map.cpp.

//! Port of `channel_map.h` and `channel_map.cpp`: channel mapping functionality.
//!
//! channel_assignment treats the relationship of Input file channels to the encoder channels.
//! This is necessary because the usual order in RIFF files (.wav) is different from the elements
//! order in the coder given by Table 8.1 (implicit speaker mapping) of the AAC standard. In mono
//! and stereo case, this is trivial. In mc case, it looks like this:
//!
//! ```text
//! Channel         Input file       coder chan
//! 5ch:
//!    front center    2                0 (SCE channel)
//!    left center     0                1 (1st of 1st CPE)
//!    right center    1                2 (2nd of 1st CPE)
//!    left surround   3                3 (1st of 2nd CPE)
//!    right surround  4                4 (2nd of 2nd CPE)
//! 5.1ch:
//!    front center    2                0 (SCE channel)
//!    left center     0                1 (1st of 1st CPE)
//!    right center    1                2 (2nd of 1st CPE)
//!    left surround   4                3 (1st of 2nd CPE)
//!    right surround  5                4 (2nd of 2nd CPE)
//!    LFE             3                5 (LFE)
//! ```

use crate::aacenc::{
    AacEncoderError, ChannelOrder, AAC_ENC_OK, AAC_ENC_UNSUPPORTED_CHANNELCONFIG, CH_ORDER_MPEG,
};
use crate::qc_data::{ChannelMapping, ElementBits, ElementInfo, QcState};
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::{
    count_leading_bits, f_max, f_mult, fix_max, fl2fxconst_dbl, FixpDbl, MAXVAL_DBL,
};
use fdk_aac_rs_fdk::fixpoint_math::get_inv_int;
use fdk_aac_rs_fdk::syslib_channel_map_descr::{
    fdk_ch_map_descr_get_map_value, fdk_ch_map_descr_init, FdkChannelMapDescr,
};

/// `CHANNEL_MODE_CONFIG_TAB`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelModeConfigTab {
    pub enc_mode: ChannelMode,
    pub n_channels: i32,
    pub n_channels_eff: i32,
    pub n_elements: i32,
}

/// `ELEMENT_MODE`
pub type ElementMode = i32;
/// `EL_MODE_INVALID`
pub const EL_MODE_INVALID: ElementMode = 0;
/// `EL_MODE_MONO`
pub const EL_MODE_MONO: ElementMode = 1;
/// `EL_MODE_STEREO`
pub const EL_MODE_STEREO: ElementMode = 2;

const fn cmc(
    enc_mode: ChannelMode,
    n_channels: i32,
    n_channels_eff: i32,
    n_elements: i32,
) -> ChannelModeConfigTab {
    ChannelModeConfigTab { enc_mode, n_channels, n_channels_eff, n_elements }
}

/// `channelModeConfig`: channel mode configuration tab provides, corresponding number of
/// channels and elements.
///
/// Dumped with `scripts/dump_table.py libAACenc/src/channel_map.cpp channelModeConfig INT
/// --per-line 4` (one row per entry: `encMode`, `nChannels`, `nChannelsEff`, `nElements`).
pub static CHANNEL_MODE_CONFIG: [ChannelModeConfigTab; 12] = [
    cmc(1, 1, 1, 1),  // chCfg  1, SCE
    cmc(2, 2, 2, 1),  // chCfg  2, CPE
    cmc(3, 3, 3, 2),  // chCfg  3, SCE,CPE
    cmc(4, 4, 4, 3),  // chCfg  4, SCE,CPE,SCE
    cmc(5, 5, 5, 3),  // chCfg  5, SCE,CPE,CPE
    cmc(6, 6, 5, 4),  // chCfg  6, SCE,CPE,CPE,LFE
    cmc(7, 8, 7, 5),  // chCfg  7, SCE,CPE,CPE,CPE,LFE
    cmc(11, 7, 6, 5), // chCfg 11, SCE,CPE,CPE,SCE,LFE
    cmc(12, 8, 7, 5), // chCfg 12, SCE,CPE,CPE,CPE,LFE
    cmc(14, 8, 7, 5), // chCfg 14, SCE,CPE,CPE,LFE,CPE
    cmc(33, 8, 7, 5), // same as MODE_7_1_BACK,  SCE,CPE,CPE,CPE,LFE
    cmc(34, 8, 7, 5), // same as MODE_1_2_2_2_1, SCE,CPE,CPE,CPE,LFE
];

/// `FDKaacEnc_DetermineEncoderMode(CHANNEL_MODE* mode, INT nChannels)`
///
/// Deviation: for a mode other than `MODE_UNKNOWN` that is not in [`CHANNEL_MODE_CONFIG`], the
/// reference has no defined result; the port returns `AAC_ENC_UNSUPPORTED_CHANNELCONFIG`.
pub fn fdk_aac_enc_determine_encoder_mode(
    mode: &mut ChannelMode,
    n_channels: i32,
) -> AacEncoderError {
    let mut enc_mode = MODE_INVALID;

    if *mode == MODE_UNKNOWN {
        for cfg in CHANNEL_MODE_CONFIG.iter() {
            if cfg.n_channels == n_channels {
                enc_mode = cfg.enc_mode;
                break;
            }
        }
        *mode = enc_mode;
    } else {
        // check if valid channel configuration
        if let Some(cfg) = fdk_aac_enc_get_channel_mode_configuration(*mode) {
            if cfg.n_channels == n_channels {
                enc_mode = *mode;
            }
        }
    }

    if enc_mode == MODE_INVALID {
        return AAC_ENC_UNSUPPORTED_CHANNELCONFIG;
    }

    AAC_ENC_OK
}

/// `FDKaacEnc_initElement(ELEMENT_INFO* elInfo, MP4_ELEMENT_ID elType, INT* cnt,
/// FDK_channelMapDescr* mapDescr, UINT mapIdx, INT* it_cnt, const FIXP_DBL relBits)`
/// (file-static)
fn fdk_aac_enc_init_element(
    el_info: &mut ElementInfo,
    el_type: Mp4ElementId,
    cnt: &mut i32,
    map_descr: &FdkChannelMapDescr,
    map_idx: u32,
    it_cnt: &mut [i32; ID_END as usize + 1],
    rel_bits: FixpDbl,
) -> i32 {
    let mut error = 0;
    let mut counter = *cnt;

    el_info.el_type = el_type;
    el_info.relative_bits = rel_bits;

    // The channel index argument of FDK_chMapDescr_getMapValue() is a UCHAR.
    let next = |counter: &mut i32| {
        let v = fdk_ch_map_descr_get_map_value(map_descr, *counter as u8, map_idx) as i32;
        *counter += 1;
        v
    };

    match el_info.el_type {
        ID_SCE | ID_LFE | ID_CCE => {
            el_info.n_channels_in_el = 1;
            el_info.channel_index[0] = next(&mut counter);
            el_info.instance_tag = it_cnt[el_type as usize];
            it_cnt[el_type as usize] += 1;
        }
        ID_CPE => {
            el_info.n_channels_in_el = 2;
            el_info.channel_index[0] = next(&mut counter);
            el_info.channel_index[1] = next(&mut counter);
            el_info.instance_tag = it_cnt[el_type as usize];
            it_cnt[el_type as usize] += 1;
        }
        ID_DSE => {
            el_info.n_channels_in_el = 0;
            el_info.channel_index[0] = 0;
            el_info.channel_index[1] = 0;
            el_info.instance_tag = it_cnt[el_type as usize];
            it_cnt[el_type as usize] += 1;
        }
        _ => error = 1,
    }
    *cnt = counter;

    error
}

/// `FDKaacEnc_InitChannelMapping(CHANNEL_MODE mode, CHANNEL_ORDER co, CHANNEL_MAPPING* cm)`
pub fn fdk_aac_enc_init_channel_mapping(
    mode: ChannelMode,
    co: ChannelOrder,
    cm: &mut ChannelMapping,
) -> AacEncoderError {
    let mut count: i32 = 0; // count through coder channels
                            // it_cnt[ID_END] is not initialized in C; it is never used.
    let mut it_cnt = [0i32; ID_END as usize + 1];
    let mut map_descr = FdkChannelMapDescr::default();

    *cm = ChannelMapping::default();

    // init channel mapping
    for cfg in CHANNEL_MODE_CONFIG.iter() {
        if cfg.enc_mode == mode {
            cm.enc_mode = cfg.enc_mode;
            cm.n_channels = cfg.n_channels;
            cm.n_channels_eff = cfg.n_channels_eff;
            cm.n_elements = cfg.n_elements;
            break;
        }
    }

    // init map descriptor
    fdk_ch_map_descr_init(&mut map_descr, None, 0, if co == CH_ORDER_MPEG { 1 } else { 0 });
    let map_idx: u32 = match mode {
        // MODE_7_1_REAR_SURROUND is equivalent to MODE_7_1_BACK
        MODE_7_1_REAR_SURROUND => MODE_7_1_BACK as u32,
        // MODE_7_1_FRONT_CENTER is equivalent to MODE_1_2_2_2_1
        MODE_7_1_FRONT_CENTER => MODE_1_2_2_2_1 as u32,
        // if channel config > 14 MPEG mapping will be used
        _ => {
            if mode > 14 {
                0
            } else {
                mode as u32
            }
        }
    };

    let md = &map_descr;
    let it = &mut it_cnt;
    let c = &mut count;
    let el = &mut cm.el_info;

    // init element info struct
    match mode {
        MODE_1 => {
            // (mono) sce
            fdk_aac_enc_init_element(&mut el[0], ID_SCE, c, md, map_idx, it, MAXVAL_DBL);
        }
        MODE_2 => {
            // (stereo) cpe
            fdk_aac_enc_init_element(&mut el[0], ID_CPE, c, md, map_idx, it, MAXVAL_DBL);
        }
        MODE_1_2 => {
            // sce + cpe
            fdk_aac_enc_init_element(&mut el[0], ID_SCE, c, md, map_idx, it, fl(0.4));
            fdk_aac_enc_init_element(&mut el[1], ID_CPE, c, md, map_idx, it, fl(0.6));
        }
        MODE_1_2_1 => {
            // sce + cpe + sce
            fdk_aac_enc_init_element(&mut el[0], ID_SCE, c, md, map_idx, it, fl(0.3));
            fdk_aac_enc_init_element(&mut el[1], ID_CPE, c, md, map_idx, it, fl(0.4));
            fdk_aac_enc_init_element(&mut el[2], ID_SCE, c, md, map_idx, it, fl(0.3));
        }
        MODE_1_2_2 => {
            // sce + cpe + cpe
            fdk_aac_enc_init_element(&mut el[0], ID_SCE, c, md, map_idx, it, fl(0.26));
            fdk_aac_enc_init_element(&mut el[1], ID_CPE, c, md, map_idx, it, fl(0.37));
            fdk_aac_enc_init_element(&mut el[2], ID_CPE, c, md, map_idx, it, fl(0.37));
        }
        MODE_1_2_2_1 => {
            // (5.1) sce + cpe + cpe + lfe
            fdk_aac_enc_init_element(&mut el[0], ID_SCE, c, md, map_idx, it, fl(0.24));
            fdk_aac_enc_init_element(&mut el[1], ID_CPE, c, md, map_idx, it, fl(0.35));
            fdk_aac_enc_init_element(&mut el[2], ID_CPE, c, md, map_idx, it, fl(0.35));
            fdk_aac_enc_init_element(&mut el[3], ID_LFE, c, md, map_idx, it, fl(0.06));
        }
        MODE_6_1 => {
            // (6.1) sce + cpe + cpe + sce + lfe
            fdk_aac_enc_init_element(&mut el[0], ID_SCE, c, md, map_idx, it, fl(0.2));
            fdk_aac_enc_init_element(&mut el[1], ID_CPE, c, md, map_idx, it, fl(0.275));
            fdk_aac_enc_init_element(&mut el[2], ID_CPE, c, md, map_idx, it, fl(0.275));
            fdk_aac_enc_init_element(&mut el[3], ID_SCE, c, md, map_idx, it, fl(0.2));
            fdk_aac_enc_init_element(&mut el[4], ID_LFE, c, md, map_idx, it, fl(0.05));
        }
        MODE_1_2_2_2_1
        | MODE_7_1_BACK
        | MODE_7_1_TOP_FRONT
        | MODE_7_1_REAR_SURROUND
        | MODE_7_1_FRONT_CENTER => {
            // (7.1) sce + cpe + cpe + cpe + lfe
            // (7.1 top) sce + cpe + cpe + lfe + cpe
            fdk_aac_enc_init_element(&mut el[0], ID_SCE, c, md, map_idx, it, fl(0.18));
            fdk_aac_enc_init_element(&mut el[1], ID_CPE, c, md, map_idx, it, fl(0.26));
            fdk_aac_enc_init_element(&mut el[2], ID_CPE, c, md, map_idx, it, fl(0.26));
            if mode != MODE_7_1_TOP_FRONT {
                fdk_aac_enc_init_element(&mut el[3], ID_CPE, c, md, map_idx, it, fl(0.26));
                fdk_aac_enc_init_element(&mut el[4], ID_LFE, c, md, map_idx, it, fl(0.04));
            } else {
                fdk_aac_enc_init_element(&mut el[3], ID_LFE, c, md, map_idx, it, fl(0.04));
                fdk_aac_enc_init_element(&mut el[4], ID_CPE, c, md, map_idx, it, fl(0.26));
            }
        }
        _ => {
            //*chMap=0;
            return AAC_ENC_UNSUPPORTED_CHANNELCONFIG;
        }
    }

    fdk_aac_rs_fdk::fdk_assert!(cm.n_elements <= 8);

    AAC_ENC_OK
}

/// `FL2FXCONST_DBL(x)` of a `float` literal.
#[inline(always)]
fn fl(x: f32) -> FixpDbl {
    fl2fxconst_dbl(x as f64)
}

/// `(fMult(rate, (FIXP_DBL)(bitrateTot << sc_brTot)) >> shift)`.
///
/// `sc_brTot = CountLeadingBits(bitrateTot)` is 31 only for `bitrateTot == -1`, which makes the
/// shift by `sc_brTot + 1` a shift by 32 (undefined); both builds use a register shift (`asr`,
/// `sar`) that takes the amount modulo 32.
#[inline(always)]
fn ch_bitrate(rate: FixpDbl, bitrate_tot: i32, sc_br_tot: i32, shift: i32) -> i32 {
    f_mult(rate, bitrate_tot << sc_br_tot).wrapping_shr(shift as u32)
}

/// `maxLfeBits` of `FDKaacEnc_InitElementBits()`:
/// `fMax((INT)((fMult(lfeRate, (FIXP_DBL)(maxChannelBits << sc)) >> sc) << 1),
/// (INT)((fMult(FL2FXCONST_DBL(1.1f / 2.f), fMult(lfeRate, (FIXP_DBL)(averageBitsTot << sc))) << 1) >> sc))`
#[inline(always)]
fn max_lfe_bits(lfe_rate: FixpDbl, max_channel_bits: i32, average_bits_tot: i32, sc: i32) -> i32 {
    f_max(
        (f_mult(lfe_rate, max_channel_bits << sc) >> sc) << 1,
        (f_mult(
            fl2fxconst_dbl((1.1f32 / 2.0f32) as f64),
            f_mult(lfe_rate, average_bits_tot << sc),
        ) << 1)
            >> sc,
    )
}

/// `FDKaacEnc_InitElementBits(QC_STATE* hQC, CHANNEL_MAPPING* cm, INT bitrateTot,
/// INT averageBitsTot, INT maxChannelBits)`
///
/// `hQC->elementBits[i]` are the entries of [`QcState::element_bits`]; writes to entries that
/// the vector does not have (C writes through the pointer array, which the encoder always fills
/// for `nElements` entries) are dropped.
pub fn fdk_aac_enc_init_element_bits(
    h_qc: &mut QcState,
    cm: &ChannelMapping,
    bitrate_tot: i32,
    average_bits_tot: i32,
    mut max_channel_bits: i32,
) -> AacEncoderError {
    let sc_br_tot = count_leading_bits(bitrate_tot);

    // Work on a copy of the first 8 entries (unwritten fields keep their values).
    let mut eb = [ElementBits::default(); 8];
    for (d, s) in eb.iter_mut().zip(h_qc.element_bits.iter()) {
        *d = *s;
    }
    let el = &cm.el_info;
    let br = |rate: FixpDbl, shift: i32| ch_bitrate(rate, bitrate_tot, sc_br_tot, shift);

    match cm.enc_mode {
        MODE_1 => {
            eb[0].ch_bitrate_el = bitrate_tot;
            eb[0].max_bits_el = max_channel_bits;
            eb[0].relative_bits_el = el[0].relative_bits;
        }
        MODE_2 => {
            eb[0].ch_bitrate_el = bitrate_tot >> 1;
            eb[0].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[0].relative_bits_el = el[0].relative_bits;
        }
        MODE_1_2 => {
            eb[0].relative_bits_el = el[0].relative_bits;
            eb[1].relative_bits_el = el[1].relative_bits;
            let sce_rate = el[0].relative_bits;
            let cpe_rate = el[1].relative_bits;
            eb[0].ch_bitrate_el = br(sce_rate, sc_br_tot);
            eb[1].ch_bitrate_el = br(cpe_rate, sc_br_tot + 1);
            eb[0].max_bits_el = max_channel_bits;
            eb[1].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
        }
        MODE_1_2_1 => {
            // sce + cpe + sce
            eb[0].relative_bits_el = el[0].relative_bits;
            eb[1].relative_bits_el = el[1].relative_bits;
            eb[2].relative_bits_el = el[2].relative_bits;
            let sce1_rate = el[0].relative_bits;
            let cpe_rate = el[1].relative_bits;
            let sce2_rate = el[2].relative_bits;
            eb[0].ch_bitrate_el = br(sce1_rate, sc_br_tot);
            eb[1].ch_bitrate_el = br(cpe_rate, sc_br_tot + 1);
            eb[2].ch_bitrate_el = br(sce2_rate, sc_br_tot);
            eb[0].max_bits_el = max_channel_bits;
            eb[1].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[2].max_bits_el = max_channel_bits;
        }
        MODE_1_2_2 => {
            // sce + cpe + cpe
            eb[0].relative_bits_el = el[0].relative_bits;
            eb[1].relative_bits_el = el[1].relative_bits;
            eb[2].relative_bits_el = el[2].relative_bits;
            let sce_rate = el[0].relative_bits;
            let cpe1_rate = el[1].relative_bits;
            let cpe2_rate = el[2].relative_bits;
            eb[0].ch_bitrate_el = br(sce_rate, sc_br_tot);
            eb[1].ch_bitrate_el = br(cpe1_rate, sc_br_tot + 1);
            eb[2].ch_bitrate_el = br(cpe2_rate, sc_br_tot + 1);
            eb[0].max_bits_el = max_channel_bits;
            eb[1].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[2].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
        }
        MODE_1_2_2_1 => {
            // (5.1) sce + cpe + cpe + lfe
            eb[0].relative_bits_el = el[0].relative_bits;
            eb[1].relative_bits_el = el[1].relative_bits;
            eb[2].relative_bits_el = el[2].relative_bits;
            eb[3].relative_bits_el = el[3].relative_bits;
            let sce_rate = el[0].relative_bits;
            let cpe1_rate = el[1].relative_bits;
            let cpe2_rate = el[2].relative_bits;
            let lfe_rate = el[3].relative_bits;

            let max_bits_tot = max_channel_bits.wrapping_mul(5); // LFE does not add to bit reservoir
            let mut sc = count_leading_bits(fix_max(max_channel_bits, average_bits_tot));
            let max_lfe_bits = max_lfe_bits(lfe_rate, max_channel_bits, average_bits_tot, sc);

            max_channel_bits = max_bits_tot.wrapping_sub(max_lfe_bits);
            sc = count_leading_bits(max_channel_bits);
            max_channel_bits = f_mult(max_channel_bits << sc, get_inv_int(5)) >> sc;

            eb[0].ch_bitrate_el = br(sce_rate, sc_br_tot);
            eb[1].ch_bitrate_el = br(cpe1_rate, sc_br_tot + 1);
            eb[2].ch_bitrate_el = br(cpe2_rate, sc_br_tot + 1);
            eb[3].ch_bitrate_el = br(lfe_rate, sc_br_tot);
            eb[0].max_bits_el = max_channel_bits;
            eb[1].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[2].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[3].max_bits_el = max_lfe_bits;
        }
        MODE_6_1 => {
            // (6.1) sce + cpe + cpe + sce + lfe
            eb[0].relative_bits_el = el[0].relative_bits;
            let sce_rate = el[0].relative_bits;
            eb[1].relative_bits_el = el[1].relative_bits;
            let cpe1_rate = el[1].relative_bits;
            eb[2].relative_bits_el = el[2].relative_bits;
            let cpe2_rate = el[2].relative_bits;
            eb[3].relative_bits_el = el[3].relative_bits;
            let sce2_rate = el[3].relative_bits;
            eb[4].relative_bits_el = el[4].relative_bits;
            let lfe_rate = el[4].relative_bits;

            let max_bits_tot = max_channel_bits.wrapping_mul(6); // LFE does not add to bit reservoir
            let sc = count_leading_bits(fix_max(max_channel_bits, average_bits_tot));
            let max_lfe_bits = max_lfe_bits(lfe_rate, max_channel_bits, average_bits_tot, sc);

            max_channel_bits = max_bits_tot.wrapping_sub(max_lfe_bits) / 6;

            eb[0].ch_bitrate_el = br(sce_rate, sc_br_tot);
            eb[1].ch_bitrate_el = br(cpe1_rate, sc_br_tot + 1);
            eb[2].ch_bitrate_el = br(cpe2_rate, sc_br_tot + 1);
            eb[3].ch_bitrate_el = br(sce2_rate, sc_br_tot + 1);
            eb[4].ch_bitrate_el = br(lfe_rate, sc_br_tot);
            eb[0].max_bits_el = max_channel_bits;
            eb[1].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[2].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[3].max_bits_el = max_channel_bits;
            eb[4].max_bits_el = max_lfe_bits;
        }
        MODE_7_1_TOP_FRONT
        | MODE_7_1_BACK
        | MODE_7_1_REAR_SURROUND
        | MODE_7_1_FRONT_CENTER
        | MODE_1_2_2_2_1 => {
            let cpe3_idx = if cm.enc_mode != MODE_7_1_TOP_FRONT { 3 } else { 4 };
            let lfe_idx = if cm.enc_mode != MODE_7_1_TOP_FRONT { 4 } else { 3 };

            // (7.1) sce + cpe + cpe + cpe + lfe
            eb[0].relative_bits_el = el[0].relative_bits;
            let sce_rate = el[0].relative_bits;
            eb[1].relative_bits_el = el[1].relative_bits;
            let cpe1_rate = el[1].relative_bits;
            eb[2].relative_bits_el = el[2].relative_bits;
            let cpe2_rate = el[2].relative_bits;
            eb[cpe3_idx].relative_bits_el = el[cpe3_idx].relative_bits;
            let cpe3_rate = el[cpe3_idx].relative_bits;
            eb[lfe_idx].relative_bits_el = el[lfe_idx].relative_bits;
            let lfe_rate = el[lfe_idx].relative_bits;

            let max_bits_tot = max_channel_bits.wrapping_mul(7); // LFE does not add to bit reservoir
            let sc = count_leading_bits(fix_max(max_channel_bits, average_bits_tot));
            let max_lfe_bits = max_lfe_bits(lfe_rate, max_channel_bits, average_bits_tot, sc);

            max_channel_bits = max_bits_tot.wrapping_sub(max_lfe_bits) / 7;

            eb[0].ch_bitrate_el = br(sce_rate, sc_br_tot);
            eb[1].ch_bitrate_el = br(cpe1_rate, sc_br_tot + 1);
            eb[2].ch_bitrate_el = br(cpe2_rate, sc_br_tot + 1);
            eb[cpe3_idx].ch_bitrate_el = br(cpe3_rate, sc_br_tot + 1);
            eb[lfe_idx].ch_bitrate_el = br(lfe_rate, sc_br_tot);
            eb[0].max_bits_el = max_channel_bits;
            eb[1].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[2].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[cpe3_idx].max_bits_el = 2i32.wrapping_mul(max_channel_bits);
            eb[lfe_idx].max_bits_el = max_lfe_bits;
        }
        _ => return AAC_ENC_UNSUPPORTED_CHANNELCONFIG,
    }

    for (d, s) in h_qc.element_bits.iter_mut().zip(eb.iter()) {
        *d = *s;
    }

    AAC_ENC_OK
}

/// `FDKaacEnc_GetMonoStereoMode(const CHANNEL_MODE mode)`: determines encoder setting from
/// channel mode. Multichannel modes are mapped to mono or stereo modes; returns
/// `EL_MODE_MONO` in case of mono, `EL_MODE_STEREO` in case of stereo, `EL_MODE_INVALID` in case
/// of error.
pub fn fdk_aac_enc_get_mono_stereo_mode(mode: ChannelMode) -> ElementMode {
    match mode {
        MODE_1 => EL_MODE_MONO, // mono setups
        MODE_2
        | MODE_1_2
        | MODE_1_2_1
        | MODE_1_2_2
        | MODE_1_2_2_1
        | MODE_6_1
        | MODE_1_2_2_2_1
        | MODE_7_1_REAR_SURROUND
        | MODE_7_1_FRONT_CENTER
        | MODE_7_1_BACK
        | MODE_7_1_TOP_FRONT => {
            EL_MODE_STEREO // stereo setups
        }
        _ => EL_MODE_INVALID, // error
    }
}

/// `FDKaacEnc_GetChannelModeConfiguration(const CHANNEL_MODE mode)`: the entry of
/// [`CHANNEL_MODE_CONFIG`] for `mode`, `None` (`NULL`) if there is none.
pub fn fdk_aac_enc_get_channel_mode_configuration(
    mode: ChannelMode,
) -> Option<&'static ChannelModeConfigTab> {
    CHANNEL_MODE_CONFIG.iter().find(|cfg| cfg.enc_mode == mode)
}

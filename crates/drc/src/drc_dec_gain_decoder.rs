// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcDec_gainDecoder.h, libDRCdec/src/drcDec_gainDecoder.cpp.

//! Port of `drcDec_gainDecoder.h`: the DRC gain decoder (gain interpolation and application).
//!
//! The structures have the memory layout of the C structures (see
//! [`crate::drc_dec_types`]), so that the differential tests can compare the complete state. The
//! pointers `pInst` and `pCoef` of `ACTIVE_DRC` point into the `UNI_DRC_CONFIG` of the DRC
//! decoder; they are stored as the index of the element plus one (0 for `NULL`, so that the
//! cleared structure holds `NULL` as in C) in a 64 bit field at the place of the pointer, and the functions that dereference them take the
//! `UNI_DRC_CONFIG` as an additional parameter.
//!
//! `FUNCTION_drcDec_GainDecoder_SetChannelGains_func1` is not defined in either reference build
//! (checked in the preprocessed sources), so the generic loop is ported.

use fdk_aac_rs_fdk::common_fix::{cnt_leading_zeros, f_mult, f_mult_div2, fl2fxconst_dbl};
use fdk_aac_rs_fdk::common_fix::{fl2fxconst_sgl, fx_dbl2fx_sgl, FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::common_fix::{DFRACT_BITS, MINVAL_DBL};
use fdk_aac_rs_fdk::scale::{saturate_left_shift, scale_value};

use crate::drc_dec_tools::*;
use crate::drc_dec_types::*;
use crate::drc_decoder::*;
use crate::drc_gain_dec_init::*;
use crate::drc_gain_dec_preprocess::*;
use crate::drc_gain_dec_process::*;

/* Definitions common to gainDecoder submodule */

/// `NUM_LNB_FRAMES`: previous frame + this frame + one frame for DM_REGULAR_DELAY + (maximum
/// delaySamples)/frameSize
pub const NUM_LNB_FRAMES: usize = 5;

/* QMF64 */
/// `SUBBAND_NUM_BANDS_QMF64`
pub const SUBBAND_NUM_BANDS_QMF64: i32 = 64;
/// `SUBBAND_DOWNSAMPLING_FACTOR_QMF64`
pub const SUBBAND_DOWNSAMPLING_FACTOR_QMF64: i32 = 64;
/// `SUBBAND_ANALYSIS_DELAY_QMF64`
pub const SUBBAND_ANALYSIS_DELAY_QMF64: i32 = 320;

/* QMF71 (according to ISO/IEC 23003-1:2007) */
/// `SUBBAND_NUM_BANDS_QMF71`
pub const SUBBAND_NUM_BANDS_QMF71: i32 = 71;
/// `SUBBAND_DOWNSAMPLING_FACTOR_QMF71`
pub const SUBBAND_DOWNSAMPLING_FACTOR_QMF71: i32 = 64;
/// `SUBBAND_ANALYSIS_DELAY_QMF71`
pub const SUBBAND_ANALYSIS_DELAY_QMF71: i32 = 320 + 384;

/* STFT256 (according to ISO/IEC 23008-3:2015/AMD3) */
/// `SUBBAND_NUM_BANDS_STFT256`
pub const SUBBAND_NUM_BANDS_STFT256: i32 = 256;
/// `SUBBAND_DOWNSAMPLING_FACTOR_STFT256`
pub const SUBBAND_DOWNSAMPLING_FACTOR_STFT256: i32 = 256;
/// `SUBBAND_ANALYSIS_DELAY_STFT256`
pub const SUBBAND_ANALYSIS_DELAY_STFT256: i32 = 256;

/// `GAIN_DEC_LOCATION`
pub type GainDecLocation = i32;
/// `GAIN_DEC_DRC1`
pub const GAIN_DEC_DRC1: GainDecLocation = 0;
/// `GAIN_DEC_DRC1_DRC2`
pub const GAIN_DEC_DRC1_DRC2: GainDecLocation = 1;
/// `GAIN_DEC_DRC2`
pub const GAIN_DEC_DRC2: GainDecLocation = 2;
/// `GAIN_DEC_DRC3`
pub const GAIN_DEC_DRC3: GainDecLocation = 3;
/// `GAIN_DEC_DRC2_DRC3`
pub const GAIN_DEC_DRC2_DRC3: GainDecLocation = 4;

/// `GAIN_DEC_PARAM`
pub type GainDecParam = i32;
/// `GAIN_DEC_FRAME_SIZE`
pub const GAIN_DEC_FRAME_SIZE: GainDecParam = 0;
/// `GAIN_DEC_SAMPLE_RATE`
pub const GAIN_DEC_SAMPLE_RATE: GainDecParam = 1;

pod_struct! {
    /// `NODE_LIN`
    pub struct NodeLin {
        /// e = 7
        pub gain_lin: FixpDbl,
        pub time: i16,
        pub _pad0: i16,
    }
}

pod_struct! {
    /// `LINEAR_NODE_BUFFER`
    pub struct LinearNodeBuffer {
        pub gain_interpolation_type: GainInterpolationType,
        /// number of nodes, saturated to 16
        pub n_nodes: [i32; NUM_LNB_FRAMES],
        pub linear_node: [[NodeLin; 16]; NUM_LNB_FRAMES],
    }
}

impl LinearNodeBuffer {
    /// `linearNode[row][idx]` with the flat addressing of C: `idx` may be -1 (the last node of a
    /// node buffer without nodes, `linearNode[lnbIx][nNodes - 1]`), which is the last node of the
    /// previous row, or for row 0 the 8 bytes before `linearNode`: `nNodes[3]` and the low half
    /// of `nNodes[4]` (little endian).
    pub fn node(&self, row: usize, idx: i32) -> NodeLin {
        let flat = row as i32 * 16 + idx;
        if (0..(NUM_LNB_FRAMES * 16) as i32).contains(&flat) {
            return self.linear_node[flat as usize / 16][flat as usize % 16];
        }
        // flat == -1 (idx >= -1, row 0): the bytes 16..24 of the structure.
        let off = core::mem::offset_of!(LinearNodeBuffer, linear_node) as i32 + flat * 8;
        if off < 0 {
            return NodeLin::ZERO;
        }
        let b = pod_bytes(self);
        let off = off as usize;
        let rd = |o: usize| b.get(o).copied().unwrap_or(0);
        NodeLin {
            gain_lin: i32::from_le_bytes([rd(off), rd(off + 1), rd(off + 2), rd(off + 3)]),
            time: i16::from_le_bytes([rd(off + 4), rd(off + 5)]),
            _pad0: i16::from_le_bytes([rd(off + 6), rd(off + 7)]),
        }
    }
}

pod_struct! {
    /// `DRC_GAIN_BUFFERS`
    pub struct DrcGainBuffers {
        pub lnb_pointer: i32,
        pub linear_node_buffer: [LinearNodeBuffer; 12],
        pub dummy_lnb: LinearNodeBuffer,
        /// e = 8
        pub channel_gain: [[FixpDbl; NUM_LNB_FRAMES]; 8],
    }
}

impl DrcGainBuffers {
    /// `&pLinearNodeBuffer[i]` (`linearNodeBuffer[i]`); index 12 is `dummyLnb`, which follows
    /// the array in memory (reproduced from the C layout). Other indices outside the array (no
    /// defined reference result) give `dummyLnb` as well.
    pub fn lnb(&self, i: i32) -> &LinearNodeBuffer {
        match self.linear_node_buffer.get(i as usize) {
            Some(l) if i >= 0 => l,
            _ => &self.dummy_lnb,
        }
    }

    /// Mutable [`DrcGainBuffers::lnb`].
    pub fn lnb_mut(&mut self, i: i32) -> &mut LinearNodeBuffer {
        if i >= 0 && (i as usize) < 12 {
            &mut self.linear_node_buffer[i as usize]
        } else {
            &mut self.dummy_lnb
        }
    }
}

pod_struct! {
    /// `ACTIVE_DRC`
    pub struct ActiveDrc {
        pub active_drc_offset: i32,
        /// Up to the alignment of the pointers (none with 4-byte pointers).
        pub _pad0: [u8; if cfg!(target_pointer_width = "32") { 0 } else { 4 }],
        /// `pInst`: index into `drcInstructionsUniDrc` of the `UNI_DRC_CONFIG` plus one, 0 for
        /// `NULL` (pointer-sized, as the C pointer)
        pub p_inst: isize,
        /// `pCoef`: index into `drcCoefficientsUniDrc` of the `UNI_DRC_CONFIG` plus one, 0 for
        /// `NULL` (pointer-sized, as the C pointer)
        pub p_coef: isize,
        pub ducking_modification_for_channel_group: [DuckingModification; 8],
        pub channel_group_for_channel: [i8; 8],
        pub band_count_for_channel_group: [u8; 8],
        pub gain_element_for_group: [u8; 8],
        pub channel_group_is_parametric_drc: [u8; 8],
        /// number of different DRC gains inluding all DRC bands
        pub gain_element_count: u8,
        pub _pad1: [u8; 3],
        pub lnb_index_for_channel: [[i32; NUM_LNB_FRAMES]; 8],
        pub subband_gains_ready: i32,
    }
}

impl ActiveDrc {
    /// `pInst` as an index into `drcInstructionsUniDrc`.
    pub fn inst(&self) -> Option<usize> {
        if self.p_inst > 0 {
            Some(self.p_inst as usize - 1)
        } else {
            None
        }
    }

    /// `pCoef` as an index into `drcCoefficientsUniDrc`.
    pub fn coef(&self) -> Option<usize> {
        if self.p_coef > 0 {
            Some(self.p_coef as usize - 1)
        } else {
            None
        }
    }
}

pod_struct! {
    /// `DRC_GAIN_DECODER`
    pub struct DrcGainDecoder {
        pub delta_tmin_default: i32,
        pub frame_size: i32,
        pub loudness_normalisation_gain_db: FixpDbl,
        pub delay_mode: DelayMode,
        pub n_active_drcs: i32,
        /// Up to the alignment of the pointers in `active_drc` (none with 4-byte pointers).
        pub _pad0: [u8; if cfg!(target_pointer_width = "32") { 0 } else { 4 }],
        pub active_drc: [ActiveDrc; MAX_ACTIVE_DRCS],
        pub multi_band_active_drc_index: i32,
        pub channel_gain_active_drc_index: i32,
        /// e = 8
        pub channel_gain: [FixpDbl; 8],
        pub drc_gain_buffers: DrcGainBuffers,
        pub subband_gains: [[FixpDbl; 4 * 1024 / 256]; 12],
        pub dummy_subband_gains: [FixpDbl; 4 * 1024 / 256],
        pub status: i32,
        pub time_domain_supported: i32,
        pub subband_domain_supported: SubbandDomainMode,
    }
}

/// `HANDLE_DRC_GAIN_DECODER`
pub type HandleDrcGainDecoder = Option<Box<DrcGainDecoder>>;

/*******************************************/
/* static functions                        */
/*******************************************/

/// `_fitsLocation(pInst, drcLocation)`
fn fits_location(p_inst: &DrcInstructionsUniDrc, drc_location: GainDecLocation) -> i32 {
    let downmix_id = if p_inst.drc_apply_to_downmix != 0 { p_inst.downmix_id[0] as i32 } else { 0 };
    match drc_location {
        GAIN_DEC_DRC1 => (downmix_id == 0) as i32,
        GAIN_DEC_DRC1_DRC2 => ((downmix_id == 0) || (downmix_id == DOWNMIX_ID_ANY_DOWNMIX)) as i32,
        GAIN_DEC_DRC2 => (downmix_id == DOWNMIX_ID_ANY_DOWNMIX) as i32,
        GAIN_DEC_DRC3 => ((downmix_id != 0) && (downmix_id != DOWNMIX_ID_ANY_DOWNMIX)) as i32,
        GAIN_DEC_DRC2_DRC3 => (downmix_id != 0) as i32,
        _ => 0,
    }
}

/// `_setChannelGains(hGainDec, numChannelGains, channelGainDb)`
///
/// `numChannelGains` must not exceed 8 (`FDK_ASSERT`); the port ignores further channels.
fn set_channel_gains_internal(
    h_gain_dec: &mut DrcGainDecoder,
    num_channel_gains: i32,
    channel_gain_db: &[FixpDbl],
) {
    let mut channel_gain_e = 0;
    fdk_aac_rs_fdk::fdk_assert!(num_channel_gains <= 8);
    for i in 0..num_channel_gains.clamp(0, 8) as usize {
        let db = channel_gain_db.get(i).copied().unwrap_or(0);
        if db == MINVAL_DBL {
            h_gain_dec.channel_gain[i] = 0;
        } else {
            /* add loudness normalisation gain (dB) to channel gain (dB) */
            let mut tmp_channel_gain_db =
                (db >> 1).wrapping_add(h_gain_dec.loudness_normalisation_gain_db >> 2);
            tmp_channel_gain_db = saturate_left_shift(tmp_channel_gain_db, 1, DFRACT_BITS);
            let channel_gain = db2lin(tmp_channel_gain_db, 8, &mut channel_gain_e);
            h_gain_dec.channel_gain[i] = scale_value(channel_gain, channel_gain_e - 8);
        }
    }
}

/*******************************************/
/* public functions                        */
/*******************************************/

/// `drcDec_GainDecoder_Open(phGainDec)`
pub fn drc_dec_gain_decoder_open(ph_gain_dec: &mut HandleDrcGainDecoder) -> DrcError {
    let mut h_gain_dec = Box::new(DrcGainDecoder::ZERO);

    h_gain_dec.multi_band_active_drc_index = -1;
    h_gain_dec.channel_gain_active_drc_index = -1;

    *ph_gain_dec = Some(h_gain_dec);

    DE_OK
}

/// `drcDec_GainDecoder_Init(hGainDec)`
pub fn drc_dec_gain_decoder_init(h_gain_dec: &mut DrcGainDecoder) -> DrcError {
    let err = init_gain_dec(h_gain_dec);
    if err != 0 {
        return err;
    }

    let frame_size = h_gain_dec.frame_size;
    init_drc_gain_buffers(frame_size, &mut h_gain_dec.drc_gain_buffers);

    err
}

/// `drcDec_GainDecoder_SetParam(hGainDec, paramType, paramValue)`
pub fn drc_dec_gain_decoder_set_param(
    h_gain_dec: &mut DrcGainDecoder,
    param_type: GainDecParam,
    param_value: i32,
) -> DrcError {
    match param_type {
        GAIN_DEC_FRAME_SIZE => {
            if param_value < 0 {
                return DE_PARAM_OUT_OF_RANGE;
            }
            h_gain_dec.frame_size = param_value;
        }
        GAIN_DEC_SAMPLE_RATE => {
            if param_value < 0 {
                return DE_PARAM_OUT_OF_RANGE;
            }
            h_gain_dec.delta_tmin_default = get_delta_tmin(param_value);
        }
        _ => return DE_PARAM_INVALID,
    }
    DE_OK
}

/// `drcDec_GainDecoder_SetCodecDependentParameters(hGainDec, delayMode, timeDomainSupported,
/// subbandDomainSupported)`
pub fn drc_dec_gain_decoder_set_codec_dependent_parameters(
    h_gain_dec: &mut DrcGainDecoder,
    delay_mode: DelayMode,
    time_domain_supported: i32,
    subband_domain_supported: SubbandDomainMode,
) -> DrcError {
    if (delay_mode != DM_REGULAR_DELAY) && (delay_mode != DM_LOW_DELAY) {
        return DE_NOT_OK;
    }
    h_gain_dec.delay_mode = delay_mode;
    h_gain_dec.time_domain_supported = time_domain_supported;
    h_gain_dec.subband_domain_supported = subband_domain_supported;

    DE_OK
}

/// `drcDec_GainDecoder_Config(hGainDec, hUniDrcConfig, numSelectedDrcSets, selectedDrcSetIds,
/// selectedDownmixIds)`
pub fn drc_dec_gain_decoder_config(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    num_selected_drc_sets: u8,
    selected_drc_set_ids: &[i8],
    selected_downmix_ids: &[u8],
) -> DrcError {
    let mut err;

    h_gain_dec.n_active_drcs = 0;
    h_gain_dec.multi_band_active_drc_index = -1;
    h_gain_dec.channel_gain_active_drc_index = -1;
    for a in 0..num_selected_drc_sets as usize {
        // numSelectedDrcSets is at most 3 (MAX_ACTIVE_DRCS) after the selection process.
        let (Some(&id), Some(&dmx)) = (selected_drc_set_ids.get(a), selected_downmix_ids.get(a))
        else {
            break;
        };
        err = init_active_drc(h_gain_dec, h_uni_drc_config, id as i32, dmx as i32);
        if err != 0 {
            return err;
        }
    }

    err = init_active_drc_offset(h_gain_dec);
    if err != 0 {
        return err;
    }

    err
}

/// `drcDec_GainDecoder_Close(phGainDec)`
pub fn drc_dec_gain_decoder_close(ph_gain_dec: &mut HandleDrcGainDecoder) -> DrcError {
    *ph_gain_dec = None;

    DE_OK
}

/// `drcDec_GainDecoder_Preprocess(hGainDec, hUniDrcGain, loudnessNormalizationGainDb, boost,
/// compress)`
pub fn drc_dec_gain_decoder_preprocess(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    h_uni_drc_gain: &UniDrcGain,
    loudness_normalization_gain_db: FixpDbl,
    boost: FixpSgl,
    compress: FixpSgl,
) -> DrcError {
    let mut err = DE_OK;

    /* lnbPointer is the index on the most recent node buffer */
    h_gain_dec.drc_gain_buffers.lnb_pointer += 1;
    if h_gain_dec.drc_gain_buffers.lnb_pointer >= NUM_LNB_FRAMES as i32 {
        h_gain_dec.drc_gain_buffers.lnb_pointer = 0;
    }

    for a in 0..h_gain_dec.n_active_drcs.max(0) {
        /* prepare gain interpolation of sequences used by copying and modifying nodes in node
         * buffers */
        err = prepare_drc_gain(
            h_gain_dec,
            h_uni_drc_config,
            h_uni_drc_gain,
            compress,
            boost,
            loudness_normalization_gain_db,
            a as usize,
        );
        if err != 0 {
            return err;
        }
    }

    let lnbp = h_gain_dec.drc_gain_buffers.lnb_pointer as usize;
    for a in 0..MAX_ACTIVE_DRCS {
        for c in 0..8 {
            h_gain_dec.active_drc[a].lnb_index_for_channel[c][lnbp] = -1; /* "no DRC processing" */
        }
        h_gain_dec.active_drc[a].subband_gains_ready = 0;
    }

    for c in 0..8 {
        h_gain_dec.drc_gain_buffers.channel_gain[c][lnbp] = fl2fxconst_dbl(1.0 / 256.0);
    }

    err
}

/// `FL2FXCONST_SGL(0.9f)`
const CONCEAL_FACTOR_POS: FixpSgl = fl2fxconst_sgl(0.9f32 as f64);
/// `FL2FXCONST_SGL(0.98f)`
const CONCEAL_FACTOR_NEG: FixpSgl = fl2fxconst_sgl(0.98f32 as f64);

/// `drcDec_GainDecoder_Conceal(hGainDec, hUniDrcConfig, hUniDrcGain)`: create gain sequence out
/// of gain sequences of last frame for concealment and flushing
pub fn drc_dec_gain_decoder_conceal(
    h_gain_dec: &DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    h_uni_drc_gain: &mut UniDrcGain,
) -> DrcError {
    let p_coef = select_drc_coefficients(h_uni_drc_config, LOCATION_SELECTED)
        .map(|c| &h_uni_drc_config.drc_coefficients_uni_drc[c]);
    let gain_sequence_count = match p_coef {
        Some(c) if c.gain_sequence_count != 0 => c.gain_sequence_count.min(12) as usize,
        _ => 1,
    };

    for seq in 0..gain_sequence_count {
        let mut last_gain_db: FixpSgl = 0;

        let last_node_index = h_uni_drc_gain.n_nodes[seq] as i32 - 1;
        if (last_node_index >= 0) && (last_node_index < 16) {
            last_gain_db = h_uni_drc_gain.gain_node[seq][last_node_index as usize].gain_db;
        }

        h_uni_drc_gain.n_nodes[seq] = 1;
        if last_gain_db > 0 {
            h_uni_drc_gain.gain_node[seq][0].gain_db =
                fx_dbl2fx_sgl(f_mult(CONCEAL_FACTOR_POS, last_gain_db));
        } else {
            h_uni_drc_gain.gain_node[seq][0].gain_db =
                fx_dbl2fx_sgl(f_mult(CONCEAL_FACTOR_NEG, last_gain_db));
        }
        h_uni_drc_gain.gain_node[seq][0].time = h_gain_dec.frame_size.wrapping_sub(1) as i16;
    }
    DE_OK
}

/// `drcDec_GainDecoder_SetChannelGains(hGainDec, numChannels, frameSize, channelGainDb,
/// audioBufferChannelOffset, audioBuffer)`
///
/// `audioBuffer` holds `numChannels` channels of `frameSize` samples at a distance of
/// `audioBufferChannelOffset`. Samples outside of the slice are skipped. x86 division by zero:
/// the arm64 result (0) on every variant.
pub fn drc_dec_gain_decoder_set_channel_gains(
    h_gain_dec: &mut DrcGainDecoder,
    num_channels: i32,
    frame_size: i32,
    channel_gain_db: &[FixpDbl],
    audio_buffer_channel_offset: i32,
    audio_buffer: &mut [FixpDbl],
) {
    if h_gain_dec.channel_gain_active_drc_index >= 0 {
        /* channel gains will be applied in drcDec_GainDecoder_ProcessTimeDomain or
         * drcDec_GainDecoder_ProcessSubbandDomain, respectively. */
        set_channel_gains_internal(h_gain_dec, num_channels, channel_gain_db);

        if h_gain_dec.status == 0 {
            /* overwrite previous channel gains at startup */
            for c in 0..num_channels.clamp(0, 8) as usize {
                for i in 0..NUM_LNB_FRAMES {
                    h_gain_dec.drc_gain_buffers.channel_gain[c][i] = h_gain_dec.channel_gain[c];
                }
            }
            h_gain_dec.status = 1;
        }
    } else {
        /* smooth and apply channel gains */
        let mut prev_channel_gain = [0 as FixpDbl; 8];
        let nch = num_channels.clamp(0, 8) as usize;
        prev_channel_gain[..nch].copy_from_slice(&h_gain_dec.channel_gain[..nch]);

        set_channel_gains_internal(h_gain_dec, num_channels, channel_gain_db);

        if h_gain_dec.status == 0 {
            /* overwrite previous channel gains at startup */
            prev_channel_gain[..nch].copy_from_slice(&h_gain_dec.channel_gain[..nch]);
            h_gain_dec.status = 1;
        }

        let mut base: i64 = 0;
        for c in 0..nch {
            let mut n_min = cnt_leading_zeros(prev_channel_gain[c])
                .min(cnt_leading_zeros(h_gain_dec.channel_gain[c]))
                .wrapping_sub(1)
                .min(9);
            let mut gain = prev_channel_gain[c].wrapping_shl(n_min as u32);
            let mut stepsize =
                h_gain_dec.channel_gain[c].wrapping_shl(n_min as u32).wrapping_sub(gain);
            if stepsize != 0 {
                if frame_size == 1024 {
                    stepsize >>= 10;
                } else if frame_size != 0 {
                    stepsize = stepsize.wrapping_div(frame_size);
                } else {
                    stepsize = 0;
                }
            }
            n_min = 9 - n_min;
            for i in 0..frame_size.max(0) as i64 {
                if let Some(s) =
                    usize::try_from(base + i).ok().and_then(|k| audio_buffer.get_mut(k))
                {
                    *s = f_mult_div2(*s, gain).wrapping_shl(n_min as u32);
                }
                gain = gain.wrapping_add(stepsize);
            }
            base += audio_buffer_channel_offset as i64;
        }
    }
}

/// `drcDec_GainDecoder_ProcessTimeDomain(hGainDec, delaySamples, drcLocation, channelOffset,
/// drcChannelOffset, numChannelsProcessed, timeDataChannelOffset, audioIOBuffer)`
pub fn drc_dec_gain_decoder_process_time_domain(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    delay_samples: i32,
    drc_location: GainDecLocation,
    channel_offset: i32,
    drc_channel_offset: i32,
    num_channels_processed: i32,
    time_data_channel_offset: i32,
    audio_io_buffer: &mut [FixpDbl],
) -> DrcError {
    let mut err = DE_OK;

    if h_gain_dec.time_domain_supported == 0 {
        return DE_NOT_OK;
    }

    for a in 0..h_gain_dec.n_active_drcs.max(0) as usize {
        let Some(inst) = h_gain_dec.active_drc.get(a).and_then(|d| d.inst()) else {
            // pInst is never NULL for an active DRC (initActiveDrc).
            continue;
        };
        if fits_location(&h_uni_drc_config.drc_instructions_uni_drc[inst], drc_location) == 0 {
            continue;
        }

        /* Apply DRC */
        err = process_drc_time(
            h_gain_dec,
            h_uni_drc_config,
            a,
            delay_samples,
            channel_offset,
            drc_channel_offset,
            num_channels_processed,
            time_data_channel_offset,
            audio_io_buffer,
        );
        if err != 0 {
            return err;
        }
    }

    err
}

/// `drcDec_GainDecoder_ProcessSubbandDomain(hGainDec, delaySamples, drcLocation, channelOffset,
/// drcChannelOffset, numChannelsProcessed, processSingleTimeslot, audioIOBufferReal,
/// audioIOBufferImag)`
pub fn drc_dec_gain_decoder_process_subband_domain(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    delay_samples: i32,
    drc_location: GainDecLocation,
    channel_offset: i32,
    drc_channel_offset: i32,
    num_channels_processed: i32,
    process_single_timeslot: i32,
    audio_io_buffer_real: &mut [&mut [FixpDbl]],
    audio_io_buffer_imag: &mut [&mut [FixpDbl]],
) -> DrcError {
    let mut err = DE_OK;

    if h_gain_dec.subband_domain_supported == SDM_OFF {
        return DE_NOT_OK;
    }

    for a in 0..h_gain_dec.n_active_drcs.max(0) as usize {
        let Some(inst) = h_gain_dec.active_drc.get(a).and_then(|d| d.inst()) else {
            continue;
        };
        if fits_location(&h_uni_drc_config.drc_instructions_uni_drc[inst], drc_location) == 0 {
            continue;
        }

        /* Apply DRC */
        err = process_drc_subband(
            h_gain_dec,
            h_uni_drc_config,
            a,
            delay_samples,
            channel_offset,
            drc_channel_offset,
            num_channels_processed,
            process_single_timeslot,
            audio_io_buffer_real,
            audio_io_buffer_imag,
        );
        if err != 0 {
            return err;
        }
    }

    err
}

/// `drcDec_GainDecoder_SetLoudnessNormalizationGainDb(hGainDec, loudnessNormalizationGainDb)`
pub fn drc_dec_gain_decoder_set_loudness_normalization_gain_db(
    h_gain_dec: &mut DrcGainDecoder,
    loudness_normalization_gain_db: FixpDbl,
) -> DrcError {
    h_gain_dec.loudness_normalisation_gain_db = loudness_normalization_gain_db;

    DE_OK
}

/// `drcDec_GainDecoder_GetFrameSize(hGainDec)`
pub fn drc_dec_gain_decoder_get_frame_size(h_gain_dec: Option<&DrcGainDecoder>) -> i32 {
    match h_gain_dec {
        None => -1,
        Some(h) => h.frame_size,
    }
}

/// `drcDec_GainDecoder_GetDeltaTminDefault(hGainDec)`
pub fn drc_dec_gain_decoder_get_delta_tmin_default(h_gain_dec: Option<&DrcGainDecoder>) -> i32 {
    match h_gain_dec {
        None => -1,
        Some(h) => h.delta_tmin_default,
    }
}

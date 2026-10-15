// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_bitstream.h, libSACenc/src/sacenc_bitstream.cpp.

//! Port of `sacenc_bitstream.h` and `sacenc_bitstream.cpp`: the SpatialSpecificConfig and
//! SpatialFrame writer.
//!
//! # Output buffers
//!
//! The C writers bind the caller's buffer to a bit buffer of `outputBufferSize` bytes (a power
//! of two: 64 for the SpatialSpecificConfig, `MAX_MPEGS_BYTES` = 16384 for the frames). The
//! AAC encoder passes a smaller buffer for the frames (1023 bytes); the payload never comes
//! close to its end. The port writes through a bit buffer of `outputBufferSize` bytes: directly
//! into the caller's slice if it is that long, otherwise into a copy of the slice, of which the
//! slice part is copied back. Bytes that the bit buffer only rewrites with their old values
//! (`FDK_put` always stores four bytes) keep the caller's content as in C. Bytes beyond the
//! caller's slice are dropped.
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_get_valid_bits, fdk_init_bit_stream, fdk_write_bits, FdkBitStream,
    BS_WRITER,
};

use crate::sacenc_const::*;
use crate::sacenc_lib::*;
use crate::sacenc_nlc_enc::*;

mod tables;
pub use tables::*;

/// `MAX_NUM_BINS`
pub const MAX_NUM_BINS: usize = 23;
/// `MAX_NUM_PARAMS`
pub const MAX_NUM_PARAMS: usize = 2;
/// `MAX_NUM_OUTPUTCHANNELS`
pub const MAX_NUM_OUTPUTCHANNELS: usize = SACENC_MAX_OUTPUT_CHANNELS;
/// `MAX_TIME_SLOTS`
pub const MAX_TIME_SLOTS: usize = 32;

/// `MAX_FREQ_RES_INDEX`
const MAX_FREQ_RES_INDEX: usize = 8;
/// `MAX_SAMPLING_FREQUENCY_INDEX`
const MAX_SAMPLING_FREQUENCY_INDEX: usize = 13;
/// `SAMPLING_FREQUENCY_INDEX_ESCAPE`
const SAMPLING_FREQUENCY_INDEX_ESCAPE: i32 = 15;

/// `TREECONFIG`
pub type TreeConfig = i32;
pub const TREE_212: TreeConfig = 7;
pub const TREE_ESCAPE: TreeConfig = 15;

/// `FREQ`
pub type Freq = i32;
pub const FREQ_RES_40: Freq = 0;
pub const FREQ_RES_20: Freq = 1;
pub const FREQ_RES_10: Freq = 2;
pub const FREQ_RES_5: Freq = 3;

/// `QUANTMODE`
pub type QuantMode = i32;
pub const QUANTMODE_INVALID: QuantMode = -1;
pub const QUANTMODE_FINE: QuantMode = 0;
pub const QUANTMODE_EBQ1: QuantMode = 1;
pub const QUANTMODE_EBQ2: QuantMode = 2;

/// `TEMPSHAPECONFIG`
pub type TempShapeConfig = i32;
pub const TEMPSHAPE_OFF: TempShapeConfig = 0;

/// `FIXEDGAINDMXCONFIG`
pub type FixedGainDmxConfig = i32;
pub const FIXEDGAINDMX_INVALID: FixedGainDmxConfig = -1;
pub const FIXEDGAINDMX_0: FixedGainDmxConfig = 0;
pub const FIXEDGAINDMX_1: FixedGainDmxConfig = 1;
pub const FIXEDGAINDMX_2: FixedGainDmxConfig = 2;
pub const FIXEDGAINDMX_3: FixedGainDmxConfig = 3;
pub const FIXEDGAINDMX_4: FixedGainDmxConfig = 4;
pub const FIXEDGAINDMX_5: FixedGainDmxConfig = 5;
pub const FIXEDGAINDMX_6: FixedGainDmxConfig = 6;
pub const FIXEDGAINDMX_7: FixedGainDmxConfig = 7;

/// `DECORRCONFIG`
pub type DecorrConfig = i32;
pub const DECORR_INVALID: DecorrConfig = -1;
/// QMF splitfreq: 3, 15, 24, 65
pub const DECORR_QMFSPLIT0: DecorrConfig = 0;
/// QMF splitfreq: 3, 50, 65, 65
pub const DECORR_QMFSPLIT1: DecorrConfig = 1;
/// QMF splitfreq: 0, 15, 65, 65
pub const DECORR_QMFSPLIT2: DecorrConfig = 2;

/// `DATA_MODE`
pub type DataMode = i32;
pub const DEFAULT: DataMode = 0;
pub const KEEP: DataMode = 1;
pub const INTERPOLATE: DataMode = 2;
pub const FINECOARSE: DataMode = 3;

/// `SPATIALFRAME_TYPE`
pub type SpatialFrameType = i32;
pub const READ_SPATIALFRAME: SpatialFrameType = 0;
pub const WRITE_SPATIALFRAME: SpatialFrameType = 1;

/// `TREEDESCRIPTION`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeDescription {
    pub num_ott_boxes: i32,
    pub num_in_chan: i32,
    pub num_out_chan: i32,
}

/// `OTTCONFIG`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OttConfig {
    pub bs_ott_bands: i32,
}

/// `SPATIALSPECIFICCONFIG`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialSpecificConfig {
    /// for bsSamplingFrequencyIndex
    pub bs_sampling_frequency: i32,
    pub bs_frame_length: i32,
    /// for bsFreqRes
    pub num_bands: i32,
    pub bs_tree_config: TreeConfig,
    pub bs_quant_mode: QuantMode,
    pub bs_fixed_gain_dmx: FixedGainDmxConfig,
    pub bs_env_quant_mode: i32,
    pub bs_decorr_config: DecorrConfig,
    pub tree_description: TreeDescription,
    pub ott_config: [OttConfig; SACENC_MAX_NUM_BOXES],
}

/// `FRAMINGINFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramingInfo {
    pub bs_framing_type: u8,
    pub num_param_sets: u8,
    pub bs_param_slots: [u8; MAX_NUM_PARAMS],
}

/// `OTTDATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OttData {
    pub cld: [[[i8; MAX_NUM_BINS]; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
    pub icc: [[[i8; MAX_NUM_BINS]; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
}

/// `SMGDATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SmgData {
    pub bs_smooth_mode: [u8; MAX_NUM_PARAMS],
    pub bs_smooth_time: [u8; MAX_NUM_PARAMS],
    pub bs_freq_res_stride: [u8; MAX_NUM_PARAMS],
    pub bs_smg_data: [[u8; MAX_NUM_BINS]; MAX_NUM_PARAMS],
}

/// `TEMPSHAPEDATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TempShapeData {
    pub bs_env_shape_channel: [u8; MAX_NUM_OUTPUTCHANNELS],
    pub bs_env_shape_data: [[u8; MAX_TIME_SLOTS]; MAX_NUM_OUTPUTCHANNELS],
}

/// `LOSSLESSDATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LosslessData {
    pub bs_xxx_data_mode: [[u8; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
    pub bs_data_pair: [[u8; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
    pub bs_quant_coarse_xxx: [[u8; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
    pub bs_freq_res_stride_xxx: [[u8; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
}

/// `SPATIALFRAME`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialFrame {
    pub framing_info: FramingInfo,
    pub bs_independency_flag: u8,
    pub ott_data: OttData,
    pub smg_data: SmgData,
    pub temp_shape_data: TempShapeData,
    pub cld_lossless_data: LosslessData,
    pub icc_lossless_data: LosslessData,
    pub b_use_bb_cues: u8,
}

/// `PREV_OTTDATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrevOttData {
    pub cld_old: [[i8; MAX_NUM_BINS]; SACENC_MAX_NUM_BOXES],
    pub icc_old: [[i8; MAX_NUM_BINS]; SACENC_MAX_NUM_BOXES],
    pub quant_coarse_cld_prev: [[u8; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
    pub quant_coarse_icc_prev: [[u8; MAX_NUM_PARAMS]; SACENC_MAX_NUM_BOXES],
}

/// `STATIC_SPATIALFRAME`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticSpatialFrame {
    pub prev_ott_data: PrevOttData,
}

/// `BSF_INSTANCE`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BsfInstance {
    pub spatial_specific_config: SpatialSpecificConfig,
    pub frame: SpatialFrame,
    pub prev_frame_data: StaticSpatialFrame,
}

/// `HANDLE_BSF_INSTANCE`
pub type HandleBsfInstance = Option<Box<BsfInstance>>;

/// `DuplicateLosslessData(startBox, stopBox, hLosslessDataFrom, setFrom, hLosslessDataTo,
/// setTo)` (static in C)
pub fn duplicate_lossless_data(
    start_box: i32,
    stop_box: i32,
    h_lossless_data_from: &LosslessData,
    set_from: i32,
    h_lossless_data_to: &mut LosslessData,
    set_to: i32,
) -> FdkSacencError {
    let (f, t) = (set_from as usize, set_to as usize);
    for i in start_box.max(0) as usize..stop_box.max(0) as usize {
        h_lossless_data_to.bs_xxx_data_mode[i][t] = h_lossless_data_from.bs_xxx_data_mode[i][f];
        h_lossless_data_to.bs_data_pair[i][t] = h_lossless_data_from.bs_data_pair[i][f];
        h_lossless_data_to.bs_quant_coarse_xxx[i][t] =
            h_lossless_data_from.bs_quant_coarse_xxx[i][f];
        h_lossless_data_to.bs_freq_res_stride_xxx[i][t] =
            h_lossless_data_from.bs_freq_res_stride_xxx[i][f];
    }
    SACENC_OK
}

/// `fdk_sacenc_duplicateParameterSet(hFrom, setFrom, hTo, setTo)`: copies the parameter set
/// `setFrom` of `hFrom` to the parameter set `setTo` of `hTo`.
pub fn fdk_sacenc_duplicate_parameter_set(
    h_from: &SpatialFrame,
    set_from: i32,
    h_to: &mut SpatialFrame,
    set_to: i32,
) -> FdkSacencError {
    let (f, t) = (set_from as usize, set_to as usize);
    // Only Copy Parameter Set selective stuff

    // OTT-Data
    for b in 0..SACENC_MAX_NUM_BOXES {
        h_to.ott_data.cld[b][t] = h_from.ott_data.cld[b][f];
        h_to.ott_data.icc[b][t] = h_from.ott_data.icc[b][f];
    }

    // LOSSLESSDATA
    duplicate_lossless_data(
        0,
        SACENC_MAX_NUM_BOXES as i32,
        &h_from.cld_lossless_data,
        set_from,
        &mut h_to.cld_lossless_data,
        set_to,
    );
    duplicate_lossless_data(
        0,
        SACENC_MAX_NUM_BOXES as i32,
        &h_from.icc_lossless_data,
        set_from,
        &mut h_to.icc_lossless_data,
        set_to,
    );

    SACENC_OK
}

/// `clearFrame(pFrame)` (static in C): set frame defaults.
pub fn clear_frame(p_frame: &mut SpatialFrame) {
    *p_frame = SpatialFrame::default();

    p_frame.bs_independency_flag = 1;
    p_frame.framing_info.num_param_sets = 1;
}

/// `fine2coarse(data, dataType, startBand, numBands)` (static in C)
pub fn fine2coarse(data: &mut [i8], data_type: DataType, start_band: i32, num_bands: i32) {
    let r = start_band.max(0) as usize..(start_band + num_bands).max(0) as usize;
    if data_type == t_CLD {
        for v in &mut data[r] {
            *v /= 2;
        }
    } else {
        for v in &mut data[r] {
            *v >>= 1;
        }
    }
}

/// `coarse2fine(data, dataType, startBand, numBands)` (static in C)
pub fn coarse2fine(data: &mut [i8], data_type: DataType, start_band: i32, num_bands: i32) {
    let r = start_band.max(0) as usize..(start_band + num_bands).max(0) as usize;
    for v in &mut data[r.clone()] {
        // `data[i] <<= 1`: computed in int, truncated to SCHAR.
        *v = ((*v as i32) << 1) as i8;
    }

    if data_type == t_CLD {
        for v in &mut data[r] {
            if *v == -14 {
                *v = -15;
            } else if *v == 14 {
                *v = 15;
            }
        }
    }
}

/// `getBsFreqResStride(index)` (static in C)
pub fn get_bs_freq_res_stride(index: i32) -> u8 {
    let table = &FREQ_RES_STRIDE_TABLE_LD;
    if index >= 0 && (index as usize) < table.len() {
        table[index as usize]
    } else {
        1
    }
}

/// `ecData(bitstream, data, oldData, quantCoarseXXXprev, losslessData, dataType, paramIdx,
/// numParamSets, independencyFlag, startBand, stopBand, defaultValue)` (static in C): writes
/// the CLD or ICC data of one box.
pub fn ec_data(
    bitstream: &mut FdkBitStream,
    data: &[[i8; MAX_NUM_BINS]; MAX_NUM_PARAMS],
    old_data: &mut [i8; MAX_NUM_BINS],
    quant_coarse_xxx_prev: &mut [u8; MAX_NUM_PARAMS],
    lossless_data: &mut LosslessData,
    data_type: DataType,
    param_idx: i32,
    num_param_sets: i32,
    independency_flag: i32,
    start_band: i32,
    stop_band: i32,
    default_value: i32,
) {
    let p = param_idx as usize;
    let sb = start_band.max(0) as usize;
    let eb = stop_band.max(0) as usize;
    let mut a_strides = [0i32; MAX_NUM_BINS + 1];
    let mut cmp_idx_data = [[0i16; MAX_NUM_BINS]; 2];
    let mut cmp_old_data = [0i16; MAX_NUM_BINS];

    // bsXXXDataMode
    if independency_flag != 0
        || (lossless_data.bs_quant_coarse_xxx[p][0] != quant_coarse_xxx_prev[p])
    {
        lossless_data.bs_xxx_data_mode[p][0] = FINECOARSE as u8;
    } else {
        lossless_data.bs_xxx_data_mode[p][0] = KEEP as u8;
        for i in sb..eb {
            if data[0][i] != old_data[i] {
                lossless_data.bs_xxx_data_mode[p][0] = FINECOARSE as u8;
                break;
            }
        }
    }

    fdk_write_bits(bitstream, lossless_data.bs_xxx_data_mode[p][0] as u32, 2);

    for ps in 1..num_param_sets.max(0) as usize {
        if lossless_data.bs_quant_coarse_xxx[p][ps] != lossless_data.bs_quant_coarse_xxx[p][ps - 1]
        {
            lossless_data.bs_xxx_data_mode[p][ps] = FINECOARSE as u8;
        } else {
            lossless_data.bs_xxx_data_mode[p][ps] = KEEP as u8;
            for i in sb..eb {
                if data[ps][i] != data[ps - 1][i] {
                    lossless_data.bs_xxx_data_mode[p][ps] = FINECOARSE as u8;
                    break;
                }
            }
        }

        fdk_write_bits(bitstream, lossless_data.bs_xxx_data_mode[p][ps] as u32, 2);
    }

    // Create data pairs if possible
    let mut ps = 0usize;
    while (ps as i32) < num_param_sets - 1 {
        if lossless_data.bs_xxx_data_mode[p][ps] == FINECOARSE as u8 {
            // Check if next parameter set is FINCOARSE
            if lossless_data.bs_xxx_data_mode[p][ps + 1] == FINECOARSE as u8 {
                // We have to check if ps and ps+1 use the same bsXXXQuantMode and also have the
                // same stride
                if (lossless_data.bs_quant_coarse_xxx[p][ps + 1]
                    == lossless_data.bs_quant_coarse_xxx[p][ps])
                    && (lossless_data.bs_freq_res_stride_xxx[p][ps + 1]
                        == lossless_data.bs_freq_res_stride_xxx[p][ps])
                {
                    lossless_data.bs_data_pair[p][ps] = 1;
                    lossless_data.bs_data_pair[p][ps + 1] = 1;

                    // We have a data pair -> Jump to the ps after next ps
                    ps += 2;
                    continue;
                }
            }
            // dataMode of next ps is not FINECOARSE or does not use the same
            // bsXXXQuantMode/stride -> no dataPair possible
            lossless_data.bs_data_pair[p][ps] = 0;

            // Initialize ps after next ps to Zero (only important for the last parameter set)
            lossless_data.bs_data_pair[p][ps + 1] = 0;
        } else {
            // No FINECOARSE -> no data pair possible
            lossless_data.bs_data_pair[p][ps] = 0;

            // Initialize ps after next ps to Zero (only important for the last parameter set)
            lossless_data.bs_data_pair[p][ps + 1] = 0;
        }
        ps += 1;
    }

    let mut ps = 0usize;
    while (ps as i32) < num_param_sets {
        if lossless_data.bs_xxx_data_mode[p][ps] == DEFAULT as u8 {
            // Prepare old data
            for v in &mut old_data[sb..eb] {
                *v = default_value as i8;
            }
            quant_coarse_xxx_prev[p] = 0; // Default data are always fine
        }

        if lossless_data.bs_xxx_data_mode[p][ps] == FINECOARSE as u8 {
            fdk_write_bits(bitstream, lossless_data.bs_data_pair[p][ps] as u32, 1);
            fdk_write_bits(bitstream, lossless_data.bs_quant_coarse_xxx[p][ps] as u32, 1);
            fdk_write_bits(bitstream, lossless_data.bs_freq_res_stride_xxx[p][ps] as u32, 2);

            if lossless_data.bs_quant_coarse_xxx[p][ps] != quant_coarse_xxx_prev[p] {
                if quant_coarse_xxx_prev[p] != 0 {
                    coarse2fine(old_data, data_type, start_band, stop_band - start_band);
                } else {
                    fine2coarse(old_data, data_type, start_band, stop_band - start_band);
                }
            }

            // Handle strides
            let pb_stride =
                get_bs_freq_res_stride(lossless_data.bs_freq_res_stride_xxx[p][ps] as i32) as i32;
            let data_bands = (stop_band - start_band - 1) / pb_stride + 1;
            let db = data_bands.max(0) as usize;

            a_strides[0] = start_band;
            for pb in 1..=db {
                a_strides[pb] = a_strides[pb - 1] + pb_stride;
            }

            let mut str_offset = 0usize;
            while a_strides[db] > stop_band {
                if str_offset < db {
                    str_offset += 1;
                }
                for v in &mut a_strides[str_offset..=db] {
                    *v -= 1;
                }
            }

            let pair = lossless_data.bs_data_pair[p][ps] != 0;
            for pb in 0..db {
                let s = a_strides[pb] as usize;
                cmp_old_data[sb + pb] = old_data[s] as i16;
                cmp_idx_data[0][sb + pb] = data[ps][s] as i16;

                if pair {
                    cmp_idx_data[1][sb + pb] = data[ps + 1][s] as i16;
                }
            }

            // Finally encode
            let indep = (independency_flag != 0 && ps == 0) as i32;
            if pair {
                fdk_sacenc_ec_data_pair_enc(
                    bitstream,
                    &cmp_idx_data,
                    &cmp_old_data,
                    data_type,
                    0,
                    start_band,
                    data_bands,
                    lossless_data.bs_quant_coarse_xxx[p][ps] as i32,
                    indep,
                );
            } else {
                fdk_sacenc_ec_data_single_enc(
                    bitstream,
                    &cmp_idx_data,
                    &cmp_old_data,
                    data_type,
                    0,
                    start_band,
                    data_bands,
                    lossless_data.bs_quant_coarse_xxx[p][ps] as i32,
                    indep,
                );
            }

            // Overwrite old data
            for i in sb..eb {
                old_data[i] = if pair { data[ps + 1][i] } else { data[ps][i] };
            }

            quant_coarse_xxx_prev[p] = lossless_data.bs_quant_coarse_xxx[p][ps];

            // Jump forward if we have encoded a data pair
            if pair {
                ps += 1;
            }
        }
        ps += 1;
    }
}

/// `getBsFreqResIndex(numBands, pbsFreqResIndex)` (static in C)
pub fn get_bs_freq_res_index(num_bands: i32, pbs_freq_res_index: &mut i32) -> FdkSacencError {
    let mut error = SACENC_OK;
    *pbs_freq_res_index = -1;

    for (i, &b) in FREQ_RES_BIN_TABLE_LD.iter().enumerate() {
        if num_bands == b as i32 {
            *pbs_freq_res_index = i as i32;
            break;
        }
    }
    if *pbs_freq_res_index < 0 || *pbs_freq_res_index >= MAX_FREQ_RES_INDEX as i32 {
        error = SACENC_INVALID_CONFIG;
    }
    error
}

/// `getSamplingFrequencyIndex(bsSamplingFrequency, pbsSamplingFrequencyIndex)` (static in C)
pub fn get_sampling_frequency_index(
    bs_sampling_frequency: i32,
    pbs_sampling_frequency_index: &mut i32,
) -> FdkSacencError {
    *pbs_sampling_frequency_index = SAMPLING_FREQUENCY_INDEX_ESCAPE;

    for (i, &f) in SAMPLE_RATE_TABLE.iter().enumerate() {
        if bs_sampling_frequency == f {
            // spatial sampling rate
            *pbs_sampling_frequency_index = i as i32;
            break;
        }
    }
    SACENC_OK
}

/// `fdk_sacenc_destroySpatialBitstreamEncoder(selfPtr)`
pub fn fdk_sacenc_destroy_spatial_bitstream_encoder(
    self_ptr: Option<&mut HandleBsfInstance>,
) -> FdkSacencError {
    match self_ptr {
        Some(p) if p.is_some() => {
            *p = None;
            SACENC_OK
        }
        _ => SACENC_INVALID_HANDLE,
    }
}

/// `fdk_sacenc_createSpatialBitstreamEncoder(selfPtr)`
pub fn fdk_sacenc_create_spatial_bitstream_encoder(
    self_ptr: Option<&mut HandleBsfInstance>,
) -> FdkSacencError {
    match self_ptr {
        None => SACENC_INVALID_HANDLE,
        Some(p) => {
            // allocate encoder struct (zeroed)
            *p = Some(Box::default());
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_initSpatialBitstreamEncoder(selfPtr)`
pub fn fdk_sacenc_init_spatial_bitstream_encoder(
    self_ptr: Option<&mut BsfInstance>,
) -> FdkSacencError {
    match self_ptr {
        None => SACENC_INVALID_HANDLE,
        Some(s) => {
            // init/clear
            clear_frame(&mut s.frame);
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_getSpatialSpecificConfig(selfPtr)`
pub fn fdk_sacenc_get_spatial_specific_config(
    self_ptr: &mut BsfInstance,
) -> &mut SpatialSpecificConfig {
    &mut self_ptr.spatial_specific_config
}

/// Runs `f` on a bitstream writer bound to `buffer` with a bit buffer of `buffer_size` bytes
/// (see the module documentation).
fn with_writer<R>(
    buffer: &mut [u8],
    buffer_size: i32,
    f: impl FnOnce(&mut FdkBitStream) -> R,
) -> R {
    let size = buffer_size.max(0) as usize;
    let mut bs = FdkBitStream::default();
    // The writer works on a copy of the buffer (at least `size` bytes), copied back afterwards.
    let mem = BitMem::new(size.max(buffer.len()));
    mem.write(0, buffer);
    fdk_init_bit_stream(&mut bs, &mem, size as u32, 0, BS_WRITER);
    let r = f(&mut bs);
    mem.read(0, buffer);
    r
}

/// `fdk_sacenc_writeSpatialSpecificConfig(spatialSpecificConfig, pOutputBuffer,
/// outputBufferSize, pnOutputBits)`: writes the SpatialSpecificConfig.
pub fn fdk_sacenc_write_spatial_specific_config(
    spatial_specific_config: &SpatialSpecificConfig,
    p_output_buffer: &mut [u8],
    output_buffer_size: i32,
    pn_output_bits: &mut i32,
) -> FdkSacencError {
    let mut bs_sampling_frequency_index: i32 = 0;
    let mut bs_freq_res: i32 = 0;

    // Find FreqRes
    let error = get_bs_freq_res_index(spatial_specific_config.num_bands, &mut bs_freq_res);
    if error != SACENC_OK {
        return error;
    }

    // Find SamplingFrequencyIndex
    let error = get_sampling_frequency_index(
        spatial_specific_config.bs_sampling_frequency,
        &mut bs_sampling_frequency_index,
    );
    if error != SACENC_OK {
        return error;
    }

    let ssc = spatial_specific_config;
    with_writer(p_output_buffer, output_buffer_size, |bitstream| {
        // write to bitstream
        fdk_write_bits(bitstream, bs_sampling_frequency_index as u32, 4);

        if bs_sampling_frequency_index == 15 {
            fdk_write_bits(bitstream, ssc.bs_sampling_frequency as u32, 24);
        }

        fdk_write_bits(bitstream, ssc.bs_frame_length as u32, 5);

        fdk_write_bits(bitstream, bs_freq_res as u32, 3);
        fdk_write_bits(bitstream, ssc.bs_tree_config as u32, 4);
        fdk_write_bits(bitstream, ssc.bs_quant_mode as u32, 2);

        fdk_write_bits(bitstream, 0, 1); // bsArbitraryDownmix

        fdk_write_bits(bitstream, ssc.bs_fixed_gain_dmx as u32, 3);

        fdk_write_bits(bitstream, TEMPSHAPE_OFF as u32, 2);
        fdk_write_bits(bitstream, ssc.bs_decorr_config as u32, 2);

        fdk_byte_align(bitstream, 0); // byte alignment

        // return number of valid bits in bitstream
        *pn_output_bits = fdk_get_valid_bits(bitstream) as i32;
        if *pn_output_bits > output_buffer_size.wrapping_mul(8) {
            return SACENC_INVALID_CONFIG;
        }

        // terminate buffer with alignment
        fdk_byte_align(bitstream, 0);
        SACENC_OK
    })
}

/// `fdk_sacenc_getSpatialFrame(selfPtr, frameType)`: both frame types give the one frame.
pub fn fdk_sacenc_get_spatial_frame(
    self_ptr: Option<&mut BsfInstance>,
    frame_type: SpatialFrameType,
) -> Option<&mut SpatialFrame> {
    let idx = match frame_type {
        READ_SPATIALFRAME | WRITE_SPATIALFRAME => 0,
        _ => -1, // invalid configuration
    };
    match self_ptr {
        Some(s) if idx != -1 => Some(&mut s.frame),
        _ => None,
    }
}

/// `writeFramingInfo(hBitstream, pFramingInfo, frameLength)` (static in C)
pub fn write_framing_info(
    h_bitstream: &mut FdkBitStream,
    p_framing_info: &FramingInfo,
    frame_length: i32,
) -> FdkSacencError {
    fdk_write_bits(h_bitstream, p_framing_info.bs_framing_type as u32, 1);
    fdk_write_bits(h_bitstream, (p_framing_info.num_param_sets as i32 - 1) as u32, 1);

    if p_framing_info.bs_framing_type != 0 {
        let num_param_sets = p_framing_info.num_param_sets as usize;
        for ps in 0..num_param_sets {
            let mut bits_param_slot = 0u32;
            // `(1 << bitsParamSlot) < (frameLength + 1)`: bitsParamSlot stays below 32 for
            // every int frameLength.
            while (1i64 << bits_param_slot) < (frame_length as i64 + 1) {
                bits_param_slot += 1;
            }
            if bits_param_slot > 0 {
                let slot = p_framing_info.bs_param_slots.get(ps).copied().unwrap_or(0);
                fdk_write_bits(h_bitstream, slot as u32, bits_param_slot);
            }
        }
    }

    SACENC_OK
}

/// `writeSmgData(hBitstream, pSmgData, numParamSets, dataBands)` (static in C)
pub fn write_smg_data(
    h_bitstream: &mut FdkBitStream,
    p_smg_data: &SmgData,
    num_param_sets: i32,
    data_bands: i32,
) -> FdkSacencError {
    for i in 0..num_param_sets.max(0) as usize {
        fdk_write_bits(h_bitstream, p_smg_data.bs_smooth_mode[i] as u32, 2);

        if p_smg_data.bs_smooth_mode[i] >= 2 {
            fdk_write_bits(h_bitstream, p_smg_data.bs_smooth_time[i] as u32, 2);
        }
        if p_smg_data.bs_smooth_mode[i] == 3 {
            let stride = get_bs_freq_res_stride(p_smg_data.bs_freq_res_stride[i] as i32) as usize;
            fdk_write_bits(h_bitstream, p_smg_data.bs_freq_res_stride[i] as u32, 2);
            let mut j = 0usize;
            while (j as i32) < data_bands {
                fdk_write_bits(h_bitstream, p_smg_data.bs_smg_data[i][j] as u32, 1);
                j += stride;
            }
        }
    }

    SACENC_OK
}

/// `writeOttData(hBitstream, pPrevOttData, pOttData, ottConfig, pCLDLosslessData,
/// pICCLosslessData, numOttBoxes, numBands, numParamSets, bsIndependencyFlag)` (static in C)
pub fn write_ott_data(
    h_bitstream: &mut FdkBitStream,
    p_prev_ott_data: &mut PrevOttData,
    p_ott_data: &OttData,
    ott_config: &[OttConfig; SACENC_MAX_NUM_BOXES],
    p_cld_lossless_data: &mut LosslessData,
    p_icc_lossless_data: &mut LosslessData,
    num_ott_boxes: i32,
    num_bands: i32,
    num_param_sets: i32,
    bs_independency_flag: i32,
) -> FdkSacencError {
    // The tree of the library has one box; bounded to the array size.
    let n = (num_ott_boxes.max(0) as usize).min(SACENC_MAX_NUM_BOXES);
    for i in 0..n {
        ec_data(
            h_bitstream,
            &p_ott_data.cld[i],
            &mut p_prev_ott_data.cld_old[i],
            &mut p_prev_ott_data.quant_coarse_cld_prev[i],
            p_cld_lossless_data,
            t_CLD,
            i as i32,
            num_param_sets,
            bs_independency_flag,
            0,
            ott_config[i].bs_ott_bands,
            15,
        );
    }
    for i in 0..n {
        ec_data(
            h_bitstream,
            &p_ott_data.icc[i],
            &mut p_prev_ott_data.icc_old[i],
            &mut p_prev_ott_data.quant_coarse_icc_prev[i],
            p_icc_lossless_data,
            t_ICC,
            i as i32,
            num_param_sets,
            bs_independency_flag,
            0,
            num_bands,
            0,
        );
    }

    SACENC_OK
}

/// `WriteSpatialExtensionFrame(bitstream, self)` (static in C)
pub fn write_spatial_extension_frame(bitstream: &mut FdkBitStream) -> FdkSacencError {
    fdk_byte_align(bitstream, 0);
    SACENC_OK
}

/// `fdk_sacenc_writeSpatialFrame(pOutputBuffer, outputBufferSize, pnOutputBits, selfPtr)`:
/// writes the frame data and clears the frame.
pub fn fdk_sacenc_write_spatial_frame(
    p_output_buffer: &mut [u8],
    output_buffer_size: i32,
    pn_output_bits: &mut i32,
    self_ptr: &mut BsfInstance,
) -> FdkSacencError {
    let num_ott_boxes = self_ptr.spatial_specific_config.tree_description.num_ott_boxes;

    let frame = &mut self_ptr.frame;
    let num_param_sets = frame.framing_info.num_param_sets as i32;

    if frame.b_use_bb_cues != 0 {
        for i in 0..SACENC_MAX_NUM_BOXES {
            // If a transient was detected, force only the second ps broad band
            if num_param_sets == 1 {
                frame.cld_lossless_data.bs_freq_res_stride_xxx[i][0] = 3;
                frame.icc_lossless_data.bs_freq_res_stride_xxx[i][0] = 3;
            } else {
                for j in 1..MAX_NUM_PARAMS {
                    frame.cld_lossless_data.bs_freq_res_stride_xxx[i][j] = 3;
                    frame.icc_lossless_data.bs_freq_res_stride_xxx[i][j] = 3;
                }
            }
        }
    }

    let BsfInstance { spatial_specific_config: config, frame, prev_frame_data } = self_ptr;

    with_writer(p_output_buffer, output_buffer_size, |bitstream| {
        let error = write_framing_info(bitstream, &frame.framing_info, config.bs_frame_length);
        if error != SACENC_OK {
            return error;
        }

        // write bsIndependencyFlag
        fdk_write_bits(bitstream, frame.bs_independency_flag as u32, 1);

        // write spatial data to bitstream
        let ott_data = frame.ott_data;
        let error = write_ott_data(
            bitstream,
            &mut prev_frame_data.prev_ott_data,
            &ott_data,
            &config.ott_config,
            &mut frame.cld_lossless_data,
            &mut frame.icc_lossless_data,
            num_ott_boxes,
            config.num_bands,
            num_param_sets,
            frame.bs_independency_flag as i32,
        );
        if error != SACENC_OK {
            return error;
        }
        let error = write_smg_data(bitstream, &frame.smg_data, num_param_sets, config.num_bands);
        if error != SACENC_OK {
            return error;
        }

        // byte alignment
        fdk_byte_align(bitstream, 0);

        // Write SpatialExtensionFrame
        let error = write_spatial_extension_frame(bitstream);
        if error != SACENC_OK {
            return error;
        }

        clear_frame(frame);

        // return number of valid bits in bitstream
        *pn_output_bits = fdk_get_valid_bits(bitstream) as i32;
        if *pn_output_bits > output_buffer_size.wrapping_mul(8) {
            return SACENC_INVALID_CONFIG;
        }

        // terminate buffer with alignment
        fdk_byte_align(bitstream, 0);
        SACENC_OK
    })
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/metadata_main.h, libAACenc/src/metadata_main.cpp.

//! Port of `metadata_main.h`: the metadata encoder (MPEG `dynamic_range_info()`, the ETSI
//! TS 101 154 (DVB) ancillary data, the matrix mixdown index of the PCE, and the delay
//! compensation of audio and metadata).
//!
//! # Memory model
//!
//! `FDK_METADATA_ENCODER` holds `UCHAR drcInfoPayload[12]`, `UCHAR drcDsePayload[8]` and
//! `INT matrix_mixdown_idx` consecutively, and the payload writers initialize their bit writers
//! with a buffer size of 16 bytes. The port keeps the three members as one byte image
//! ([`FdkMetadataEncoder::payload_mem`]), reproduced from the C layout, including the bytes of
//! `matrix_mixdown_idx` that `FDK_MetadataEnc_Process()` returns to the encoder; four more bytes
//! cover the rest of the 16-byte writer of `drcDsePayload` (in C the first bytes of
//! `exPayload[0]`, which the writer only reads and writes back unchanged).
//!
//! `exPayload[].pData` point into this image (the encoder is boxed, so the pointers stay valid
//! while it lives).

use crate::aacenc::*;
use crate::aacenc_lib::AacencMetaData;
use crate::metadata_compressor::*;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_init_bit_stream, fdk_write_bits, FdkBitStream, BS_WRITER,
};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::scale::scale_value;
use fdk_aac_rs_fdk::syslib_channel_map_descr::{
    fdk_ch_map_descr_get_map_value, fdk_ch_map_descr_init, FdkChannelMapDescr,
    FDK_MAP_INFO_TAB_LEN_WG4, FDK_MAP_INFO_TAB_WG4,
};

mod tables;
use tables::*;

/// The file-static tables of `metadata_main.cpp` (for the table tests): `dmxTable`,
/// `dmxLfeTable`, `surmix2matrix_mixdown_idx`, `defaultMetaDataSetup`.
#[doc(hidden)]
pub fn tables(
) -> (&'static [FixpDbl; 8], &'static [FixpDbl; 16], &'static [u8; 8], &'static AacencMetaData) {
    (&DMX_TABLE, &DMX_LFE_TABLE, &SURMIX2MATRIX_MIXDOWN_IDX, &DEFAULT_META_DATA_SETUP)
}

/// `FDK_METADATA_ERROR`
pub type FdkMetadataError = i32;
/// No error happened. All fine.
pub const METADATA_OK: FdkMetadataError = 0x0000;
/// Handle passed to function call was invalid.
pub const METADATA_INVALID_HANDLE: FdkMetadataError = 0x0020;
/// Memory allocation failed.
pub const METADATA_MEMORY_ERROR: FdkMetadataError = 0x0021;
/// General initialization error.
pub const METADATA_INIT_ERROR: FdkMetadataError = 0x0040;
/// The encoding process was interrupted by an unexpected error.
pub const METADATA_ENCODE_ERROR: FdkMetadataError = 0x0060;

/// `MAX_DRC_BANDS`
const MAX_DRC_BANDS: usize = 1 << 4;
/// `MAX_DRC_FRAMELEN`
const MAX_DRC_FRAMELEN: i32 = 2 * 1024;
/// `MAX_DELAY_FRAMES`
const MAX_DELAY_FRAMES: usize = 3;

/// `AAC_METADATA::mpegDrc`: MPEG Dynamic Range Control
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MpegDrc {
    pub prog_ref_level_present: u8,
    pub prog_ref_level: i8,
    pub dyn_rng_sgn: [u8; MAX_DRC_BANDS],
    pub dyn_rng_ctl: [u8; MAX_DRC_BANDS],
    pub drc_bands_present: u8,
    pub drc_band_incr: u8,
    pub drc_band_top: [u8; MAX_DRC_BANDS],
    pub drc_interpolation_scheme: u8,
    pub drc_profile: i32,
    /// used for Limiter
    pub drc_target_ref_level: i32,
    // excluded channels
    pub excluded_chns_present: u8,
    /// MAX_NUMBER_CHANNELS/8
    pub exclude_mask: [u8; 2],
}

/// `AAC_METADATA::etsiAncData.extAncData`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtAncData {
    pub ext_downmix_lvl_status: u8,
    pub ext_downmix_gain_status: u8,
    pub ext_lfe_downmix_status: u8,
    /// extended downmix level (0..7, according to table)
    pub ext_dmix_a_idx: u8,
    /// extended downmix level (0..7, according to table)
    pub ext_dmix_b_idx: u8,
    pub dmx_gain_5_sgn: u8,
    pub dmx_gain_5_idx: u8,
    pub dmx_gain_2_sgn: u8,
    pub dmx_gain_2_idx: u8,
    /// extended downmix level for lfe (0..15, according to table)
    pub ext_dmix_lfe_idx: u8,
}

/// `AAC_METADATA::etsiAncData`: ETSI addtl ancillary data
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EtsiAncData {
    // Heavy Compression
    /// flag, if compression value should be written
    pub compression_on: u8,
    /// compression value
    pub compression_value: u8,
    pub comp_profile: i32,
    /// used for Limiter
    pub comp_target_ref_level: i32,
    pub timecode_coarse_status: i32,
    pub timecode_fine_status: i32,
    pub ext_anc_data_status: u8,
    pub ext_anc_data: ExtAncData,
}

/// `AAC_METADATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacMetadata {
    pub mpeg_drc: MpegDrc,
    pub etsi_anc_data: EtsiAncData,
    /// center downmix level (0...7, according to table)
    pub center_mix_level: i8,
    /// surround downmix level (0...7, according to table)
    pub surround_mix_level: i8,
    /// flag
    pub write_pce_mix_dwn_idx: u8,
    /// flag
    pub dmx_lvl_on: u8,
    pub dolby_surround_mode: u8,
    pub drc_presentation_mode: u8,
    /// indicate meta data mode in current frame (delay line)
    pub metadata_mode: u8,
}

/// Offset of `drcInfoPayload` in [`FdkMetadataEncoder::payload_mem`].
const DRC_INFO_PAYLOAD: usize = 0;
/// Offset of `drcDsePayload` in [`FdkMetadataEncoder::payload_mem`].
const DRC_DSE_PAYLOAD: usize = 12;
/// Offset of `matrix_mixdown_idx` in [`FdkMetadataEncoder::payload_mem`].
const MATRIX_MIXDOWN_IDX: usize = 20;
/// Size of the byte image: the three members and the 4 bytes after them that the 16-byte
/// writer of `drcDsePayload` covers.
pub const PAYLOAD_MEM_SIZE: usize = DRC_DSE_PAYLOAD + 16;

/// `struct FDK_METADATA_ENCODER` (`HANDLE_FDK_METADATA_ENCODER` is `&mut FdkMetadataEncoder`).
#[derive(Clone, Debug)]
pub struct FdkMetadataEncoder {
    pub metadata_mode: i32,
    pub h_drc_comp: Option<Box<DrcComp>>,
    pub submitted_meta_data: AacencMetaData,

    /// Additional delay to round up to next frame border (in samples)
    pub n_audio_data_delay: i32,
    /// Meta data delay (in frames)
    pub n_meta_data_delay: i32,
    pub n_channels: i32,
    pub channel_mode: ChannelMode,

    /// `pAudioDelayBuffer` (`maxChannels * MAX_DRC_FRAMELEN` samples)
    pub p_audio_delay_buffer: Vec<IntPcm>,

    pub meta_data_buffer: [AacMetadata; MAX_DELAY_FRAMES],
    pub meta_data_delay_idx: i32,

    /// `drcInfoPayload[12]`, `drcDsePayload[8]` and `matrix_mixdown_idx` as their byte image
    /// (see the module documentation).
    pub payload_mem: [u8; PAYLOAD_MEM_SIZE],

    pub ex_payload: [AacencExtPayload; 2],
    pub n_extensions: i32,

    /// Maximum number of audio channels to be supported.
    pub max_channels: u32,

    /// Delay switch off by one frame and write default configuration to finalize the metadata
    /// setup.
    pub finalize_meta_data: i32,
    /// Fill up delay line with first meta data info. This is required to have meta data
    /// already in first frame.
    pub initialize_meta_data: i32,
}

// SAFETY: the raw pointers of `ex_payload` point into the structure itself.
unsafe impl Send for FdkMetadataEncoder {}

impl FdkMetadataEncoder {
    /// `matrix_mixdown_idx`
    pub fn matrix_mixdown_idx(&self) -> i32 {
        let b = &self.payload_mem[MATRIX_MIXDOWN_IDX..MATRIX_MIXDOWN_IDX + 4];
        i32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    fn set_matrix_mixdown_idx(&mut self, v: i32) {
        self.payload_mem[MATRIX_MIXDOWN_IDX..MATRIX_MIXDOWN_IDX + 4]
            .copy_from_slice(&v.to_le_bytes());
    }

    /// `drcInfoPayload`
    pub fn drc_info_payload(&self) -> &[u8] {
        &self.payload_mem[DRC_INFO_PAYLOAD..DRC_DSE_PAYLOAD]
    }

    /// `drcDsePayload` (with the bytes that follow it in the C layout: the DSE is up to 11 bytes
    /// long)
    pub fn drc_dse_payload(&self) -> &[u8] {
        &self.payload_mem[DRC_DSE_PAYLOAD..]
    }
}

/// `convertProfile(aacProfile)`
pub fn convert_profile(aac_profile: i32) -> DrcProfile {
    match aac_profile {
        0 => DRC_NONE,         // AACENC_METADATA_DRC_NONE
        1 => DRC_FILMSTANDARD, // AACENC_METADATA_DRC_FILMSTANDARD
        2 => DRC_FILMLIGHT,    // AACENC_METADATA_DRC_FILMLIGHT
        3 => DRC_MUSICSTANDARD,
        4 => DRC_MUSICLIGHT,
        5 => DRC_SPEECH,
        256 => DRC_NOT_PRESENT, // AACENC_METADATA_DRC_NOT_PRESENT
        _ => DRC_NONE,
    }
}

/// `dialnorm2progreflvl(d)`: convert dialog normalization to program reference level
pub fn dialnorm2progreflvl(d: i32) -> u8 {
    f_max(0, f_min((d.wrapping_neg().wrapping_add(1 << 13)) >> 14, 127)) as u8
}

/// `progreflvl2dialnorm(p)`: convert program reference level to dialog normalization
pub fn progreflvl2dialnorm(p: u8) -> i32 {
    -((p as i32) << (16 - 2))
}

/// `encodeDmxLvls(cmixlev, surmixlev)`: encode downmix levels to Downmixing_levels_MPEG4
pub fn encode_dmx_lvls(cmixlev: i8, surmixlev: i8) -> i8 {
    let mut dmx_lvls: i32 = 0;
    dmx_lvls |= 0x80 | ((cmixlev as i32) << 4); // center_mix_level_on
    dmx_lvls |= 0x08 | surmixlev as i32; // surround_mix_level_on

    dmx_lvls as i8
}

/// `encodeDynrng(gain, dyn_rng_ctl, dyn_rng_sgn)`: encode AAC DRC gain (ISO/IEC 14496-3:2005
/// 4.5.2.7)
///
/// `gain = -gain` overflows for `INT_MIN`; both reference builds compile the negation and
/// `fMin(gain, 127 << 14)` as an absolute value with an unsigned minimum (arm64 `cneg` +
/// `csel lo`, x86 `neg`/`cmovs` + `cmovae`), which gives 127 for `INT_MIN`.
pub fn encode_dynrng(gain: i32, dyn_rng_ctl: &mut u8, dyn_rng_sgn: &mut u8) {
    *dyn_rng_sgn = (gain < 0) as u8;
    let g = gain.unsigned_abs().min(127 << 14);

    *dyn_rng_ctl = ((g + (1 << 13)) >> 14) as u8;
}

/// `decodeDynrng(dyn_rng_ctl, dyn_rng_sgn)`: decode AAC DRC gain (ISO/IEC 14496-3:2005 4.5.2.7)
pub fn decode_dynrng(dyn_rng_ctl: u8, dyn_rng_sgn: u8) -> i32 {
    let mut tmp = (dyn_rng_ctl as i32) << (16 - 2);
    if dyn_rng_sgn != 0 {
        tmp = -tmp;
    }
    tmp
}

/// `encodeCompr(gain)`: encode AAC compression value (ETSI TS 101 154 page 99)
///
/// `(3156476 - gain) * 15` overflows for large gains (undefined behavior); both reference
/// builds decide the two range checks on `gain` itself (`gain < -3143427`: 255,
/// `gain > 3195932`: 0), as the exact arithmetic does, and compute the value only in between.
pub fn encode_compr(gain: i32) -> u8 {
    if gain < -3143427 {
        return 0xFF;
    } else if gain > 3195932 {
        return 0;
    }
    // tmp = (int)((48.164f - gain) / 6.0206f * 15 + 0.5f);
    let tmp = ((3156476 - gain) * 15 + 197283) / 394566;

    if tmp >= 240 {
        return 0xFF;
    } else if tmp < 0 {
        return 0;
    }
    let x = (tmp / 15) as u8;
    let y = (tmp % 15) as u8;

    (x << 4) | y
}

/// `FL2FXCONST_DBL(6.0206f / 128.f)`
const DECODE_COMPR_X_STEP: FixpDbl = fl2fxconst_dbl((6.0206f32 / 128.0f32) as f64);
/// `FL2FXCONST_DBL(0.4014f / 128.f)`
const DECODE_COMPR_Y_STEP: FixpDbl = fl2fxconst_dbl((0.4014f32 / 128.0f32) as f64);

/// `decodeCompr(compr)`: decode AAC compression value (ETSI TS 101 154 page 99)
pub fn decode_compr(compr: u8) -> i32 {
    let x = (compr >> 4) as i8 as i32; // 4 MSB of compr
    let y = (compr & 0x0F) as i32; // 4 LSB of compr

    // gain = (INT)((48.164f - 6.0206f * x - 0.4014f * y) );
    scale_value(
        DECODE_COMPR_X_STEP.wrapping_mul(8 - x).wrapping_sub(DECODE_COMPR_Y_STEP.wrapping_mul(y)),
        -(DFRACT_BITS - 1 - 7 - 16),
    )
}

/// `FDK_MetadataEnc_Open(phMetaData, maxChannels)`: open a Meta Data instance.
pub fn fdk_metadata_enc_open(
    ph_meta_data: Option<&mut Option<Box<FdkMetadataEncoder>>>,
    max_channels: u32,
) -> FdkMetadataError {
    let Some(ph) = ph_meta_data else {
        return METADATA_INVALID_HANDLE;
    };

    let mut h_drc_comp = None;
    // Allocate DRC Compressor.
    if fdk_drc_generator_open(&mut h_drc_comp) != 0 {
        return METADATA_MEMORY_ERROR;
    }

    let h = FdkMetadataEncoder {
        metadata_mode: 0,
        h_drc_comp,
        submitted_meta_data: AacencMetaData::default(),
        n_audio_data_delay: 0,
        n_meta_data_delay: 0,
        n_channels: 0,
        channel_mode: MODE_UNKNOWN,
        p_audio_delay_buffer: vec![0; max_channels as usize * MAX_DRC_FRAMELEN as usize],
        meta_data_buffer: [AacMetadata::default(); MAX_DELAY_FRAMES],
        meta_data_delay_idx: 0,
        payload_mem: [0; PAYLOAD_MEM_SIZE],
        ex_payload: [AacencExtPayload::default(); 2],
        n_extensions: 0,
        max_channels,
        finalize_meta_data: 0,
        initialize_meta_data: 0,
    };

    // Return metadata instance
    *ph = Some(Box::new(h));

    METADATA_OK
}

/// `FDK_MetadataEnc_Close(phMetaData)`: close the Meta Data instance.
pub fn fdk_metadata_enc_close(
    ph_meta_data: Option<&mut Option<Box<FdkMetadataEncoder>>>,
) -> FdkMetadataError {
    let Some(ph) = ph_meta_data else {
        return METADATA_INVALID_HANDLE;
    };
    if let Some(h) = ph.as_deref_mut() {
        fdk_drc_generator_close(&mut h.h_drc_comp);
    }
    *ph = None;
    METADATA_OK
}

/// `FDK_MetadataEnc_Init(hMetaData, resetStates, metadataMode, audioDelay, frameLength,
/// sampleRate, nChannels, channelMode, channelOrder)`: initialize a Meta Data instance.
#[allow(clippy::too_many_arguments)]
pub fn fdk_metadata_enc_init(
    h_meta_data: Option<&mut FdkMetadataEncoder>,
    reset_states: i32,
    metadata_mode: i32,
    audio_delay: i32,
    frame_length: u32,
    sample_rate: u32,
    n_channels: u32,
    channel_mode: ChannelMode,
    channel_order: ChannelOrder,
) -> FdkMetadataError {
    let mut err = METADATA_OK;

    let Some(h_meta_data) = h_meta_data else {
        return METADATA_INVALID_HANDLE;
    };

    // Determine values for delay compensation.
    let mut n_frames: i32 = 0;
    let mut delay = audio_delay.wrapping_sub(frame_length as i32);
    while delay > 0 {
        delay = delay.wrapping_sub(frame_length as i32);
        n_frames += 1;
        if frame_length == 0 {
            // (not defined by the reference)
            break;
        }
    }

    if (n_channels > 8)
        || (n_channels > h_meta_data.max_channels)
        || (delay.wrapping_neg() > MAX_DRC_FRAMELEN)
        || n_frames >= MAX_DELAY_FRAMES as i32
    {
        return METADATA_INIT_ERROR;
    }

    // Initialize with default setup.
    h_meta_data.submitted_meta_data = DEFAULT_META_DATA_SETUP;

    // finalize meta data only while on/off switching, else disabled
    h_meta_data.finalize_meta_data = 0;
    // fill up meta data delay line only at a reset otherwise disabled
    h_meta_data.initialize_meta_data = 0;

    // Reset delay lines.
    if reset_states != 0
        || (h_meta_data.n_audio_data_delay != delay.wrapping_neg())
        || (h_meta_data.channel_mode != channel_mode)
    {
        if reset_states != 0 || (h_meta_data.channel_mode == MODE_UNKNOWN) {
            // clear delay buffer
            h_meta_data.p_audio_delay_buffer.fill(0);
        } else {
            // if possible, keep static audio channels for seamless channel reconfiguration
            let mut map_descr_prev = FdkChannelMapDescr::default();
            let mut map_descr = FdkChannelMapDescr::default();
            let mut src: [i32; 2] = [-1, -1];
            let mut dst: [i32; 2] = [-1, -1];

            if channel_order == CH_ORDER_WG4 {
                fdk_ch_map_descr_init(
                    &mut map_descr_prev,
                    Some(&FDK_MAP_INFO_TAB_WG4),
                    FDK_MAP_INFO_TAB_LEN_WG4,
                    0,
                );
                fdk_ch_map_descr_init(
                    &mut map_descr,
                    Some(&FDK_MAP_INFO_TAB_WG4),
                    FDK_MAP_INFO_TAB_LEN_WG4,
                    0,
                );
            } else {
                let pass = if channel_order == CH_ORDER_MPEG { 1 } else { 0 };
                fdk_ch_map_descr_init(&mut map_descr_prev, None, 0, pass);
                fdk_ch_map_descr_init(&mut map_descr, None, 0, pass);
            }

            let prev_mode = h_meta_data.channel_mode;
            match channel_mode {
                MODE_1 => {
                    if n_channels as i32 != 2 {
                        // preserve center channel
                        src[0] =
                            fdk_ch_map_descr_get_map_value(&map_descr_prev, 0, prev_mode as u32)
                                as i32;
                        dst[0] = fdk_ch_map_descr_get_map_value(&map_descr, 0, channel_mode as u32)
                            as i32;
                    }
                }
                MODE_2 | MODE_1_2 | MODE_1_2_1 | MODE_1_2_2 | MODE_1_2_2_1 => {
                    if h_meta_data.n_channels >= 2 {
                        // preserve left/right channel
                        let pl = if prev_mode == 2 { 0 } else { 1 };
                        let pr = if prev_mode == 2 { 1 } else { 2 };
                        let l = if channel_mode == 2 { 0 } else { 1 };
                        let r = if channel_mode == 2 { 1 } else { 2 };
                        src[0] =
                            fdk_ch_map_descr_get_map_value(&map_descr_prev, pl, prev_mode as u32)
                                as i32;
                        src[1] =
                            fdk_ch_map_descr_get_map_value(&map_descr_prev, pr, prev_mode as u32)
                                as i32;
                        dst[0] = fdk_ch_map_descr_get_map_value(&map_descr, l, channel_mode as u32)
                            as i32;
                        dst[1] = fdk_ch_map_descr_get_map_value(&map_descr, r, channel_mode as u32)
                            as i32;
                    }
                }
                _ => {}
            }
            let mut scratch_audio_delay_buffer = [0 as IntPcm; 8];

            let prev_n_channels = h_meta_data.n_channels;
            let decreasing = prev_n_channels > n_channels as i32;
            let mut i: i32 = if decreasing { 0 } else { h_meta_data.n_audio_data_delay - 1 };
            let buf = &mut h_meta_data.p_audio_delay_buffer;
            loop {
                for c in 0..2 {
                    if src[c] != -1 && dst[c] != -1 {
                        // Where the reference has no defined result, the port reads 0.
                        let k = i.wrapping_mul(prev_n_channels).wrapping_add(src[c]);
                        let v =
                            usize::try_from(k).ok().and_then(|k| buf.get(k)).copied().unwrap_or(0);
                        if let Some(d) = scratch_audio_delay_buffer.get_mut(dst[c] as usize) {
                            *d = v;
                        }
                    }
                }
                // FDKmemcpy(&pAudioDelayBuffer[i * nChannels], scratch, nChannels)
                let base = i.wrapping_mul(n_channels as i32);
                for ch in 0..(n_channels as usize).min(8) {
                    if let Some(d) = usize::try_from(base.wrapping_add(ch as i32))
                        .ok()
                        .and_then(|k| buf.get_mut(k))
                    {
                        *d = scratch_audio_delay_buffer[ch];
                    }
                }
                i += if decreasing { 1 } else { -1 };
                if !((i < h_meta_data.n_audio_data_delay) && (i >= 0)) {
                    break;
                }
            }
        }
        h_meta_data.meta_data_buffer = [AacMetadata::default(); MAX_DELAY_FRAMES];
        h_meta_data.meta_data_delay_idx = 0;
        // fill up delay line with first meta data info
        h_meta_data.initialize_meta_data = 1;
    } else {
        // Enable meta data.
        if (h_meta_data.metadata_mode == 0) && (metadata_mode != 0) {
            // disable meta data in all delay lines
            for i in 0..MAX_DELAY_FRAMES {
                load_submitted_metadata(
                    Some(&h_meta_data.submitted_meta_data),
                    n_channels as i32,
                    0,
                    &mut h_meta_data.meta_data_buffer[i],
                );
            }
        }

        // Disable meta data.
        if (h_meta_data.metadata_mode != 0) && (metadata_mode == 0) {
            h_meta_data.finalize_meta_data = h_meta_data.metadata_mode;
        }
    }

    // Initialize delay.
    h_meta_data.n_audio_data_delay = delay.wrapping_neg();
    h_meta_data.n_meta_data_delay = n_frames;
    h_meta_data.n_channels = n_channels as i32;
    h_meta_data.channel_mode = channel_mode;
    h_meta_data.metadata_mode = metadata_mode;

    // Initialize compressor.
    if (metadata_mode == 1) || (metadata_mode == 2) {
        if let Some(drc) = h_meta_data.h_drc_comp.as_deref_mut() {
            if fdk_drc_generator_initialize(
                drc,
                DRC_NONE,
                DRC_NONE,
                frame_length as i32,
                sample_rate,
                channel_mode,
                channel_order,
                1,
            ) != 0
            {
                err = METADATA_INIT_ERROR;
            }
        }
    }

    err
}

/// `ProcessCompressor(pMetadata, hDrcComp, pSamples, samplesBufSize, nSamples)`
pub fn process_compressor(
    p_metadata: &mut AacMetadata,
    h_drc_comp: Option<&mut DrcComp>,
    p_samples: &[IntPcm],
    samples_buf_size: u32,
    _n_samples: i32,
) -> FdkMetadataError {
    let Some(h_drc_comp) = h_drc_comp else {
        return METADATA_INVALID_HANDLE;
    };

    let profile_drc = convert_profile(p_metadata.mpeg_drc.drc_profile);
    let profile_comp = convert_profile(p_metadata.etsi_anc_data.comp_profile);

    // first, check if profile is same as last frame otherwise, update setup
    if (profile_drc != fdk_drc_generator_get_drc_profile(h_drc_comp))
        || (profile_comp != fdk_drc_generator_get_comp_profile(h_drc_comp))
    {
        fdk_drc_generator_set_drc_profile(h_drc_comp, profile_drc, profile_comp);
    }

    // Sanity check
    if profile_comp == DRC_NONE {
        // to ensure no external values will be written if not configured
        p_metadata.etsi_anc_data.compression_value = 0x80;
    }

    // in case of embedding external values, copy this now (limiter may overwrite them)
    let mut dynrng =
        decode_dynrng(p_metadata.mpeg_drc.dyn_rng_ctl[0], p_metadata.mpeg_drc.dyn_rng_sgn[0]);
    let mut compr = decode_compr(p_metadata.etsi_anc_data.compression_value);

    let ext = p_metadata.etsi_anc_data.ext_anc_data;
    let dmx_gain5 = decode_dynrng(ext.dmx_gain_5_idx, ext.dmx_gain_5_sgn);
    let dmx_gain2 = decode_dynrng(ext.dmx_gain_2_idx, ext.dmx_gain_2_sgn);

    // Where the reference has no defined result, the port reads 0 (deviation).
    let dmx = |i: i32| usize::try_from(i).ok().and_then(|i| DMX_TABLE.get(i)).copied().unwrap_or(0);

    // Call compressor
    if fdk_drc_generator_calc(
        h_drc_comp,
        p_samples,
        samples_buf_size,
        progreflvl2dialnorm(p_metadata.mpeg_drc.prog_ref_level as u8),
        p_metadata.mpeg_drc.drc_target_ref_level,
        p_metadata.etsi_anc_data.comp_target_ref_level,
        dmx(p_metadata.center_mix_level as i32),
        dmx(p_metadata.surround_mix_level as i32),
        dmx(ext.ext_dmix_a_idx as i32),
        dmx(ext.ext_dmix_b_idx as i32),
        if ext.ext_lfe_downmix_status != 0 {
            DMX_LFE_TABLE.get(ext.ext_dmix_lfe_idx as usize).copied().unwrap_or(0)
        } else {
            0
        },
        dmx_gain5,
        dmx_gain2,
        &mut dynrng,
        &mut compr,
    ) != 0
    {
        return METADATA_ENCODE_ERROR;
    }

    // Write DRC values
    p_metadata.mpeg_drc.drc_band_incr = 0;
    let (mut ctl, mut sgn) =
        (p_metadata.mpeg_drc.dyn_rng_ctl[0], p_metadata.mpeg_drc.dyn_rng_sgn[0]);
    encode_dynrng(dynrng, &mut ctl, &mut sgn);
    p_metadata.mpeg_drc.dyn_rng_ctl[0] = ctl;
    p_metadata.mpeg_drc.dyn_rng_sgn[0] = sgn;
    p_metadata.etsi_anc_data.compression_value = encode_compr(compr);

    METADATA_OK
}

/// `FDK_MetadataEnc_Process(hMetaDataEnc, pAudioSamples, audioSamplesBufSize, nAudioSamples,
/// pMetadata, ppMetaDataExtPayload, nMetaDataExtensions, matrix_mixdown_idx)`: calculate Meta
/// Data processing (new metadata, DRC compressor, payloads, delay compensation of audio and
/// metadata).
///
/// `p_audio_samples` starts at `pAudioSamples` (channel stride `audio_samples_buf_size`); the
/// extension payloads are `&h.ex_payload[..*n_meta_data_extensions]` on return
/// (`*ppMetaDataExtPayload`, set only on success, as in C).
#[allow(clippy::too_many_arguments)]
pub fn fdk_metadata_enc_process(
    h_meta_data_enc: &mut FdkMetadataEncoder,
    p_audio_samples: &mut [IntPcm],
    audio_samples_buf_size: u32,
    n_audio_samples: i32,
    p_metadata: Option<&AacencMetaData>,
    pp_meta_data_ext_payload: &mut bool,
    n_meta_data_extensions: &mut u32,
    matrix_mixdown_idx: &mut i32,
) -> FdkMetadataError {
    let mut err: FdkMetadataError;

    'bail: {
        // Where to write new meta data info
        let meta_data_delay_write_idx = h_meta_data_enc.meta_data_delay_idx;

        // How to write the data
        let mut metadata_mode = h_meta_data_enc.metadata_mode;

        // Compensate meta data delay.
        h_meta_data_enc.meta_data_delay_idx += 1;
        if h_meta_data_enc.meta_data_delay_idx > h_meta_data_enc.n_meta_data_delay {
            h_meta_data_enc.meta_data_delay_idx = 0;
        }

        // Where to read pending meta data info from.
        let meta_data_delay_read_idx = h_meta_data_enc.meta_data_delay_idx;

        // Submit new data if available.
        if let Some(m) = p_metadata {
            h_meta_data_enc.submitted_meta_data = *m;
        }

        // Write one additional frame with default configuration of meta data. Ensure defined
        // behaviour on decoder side.
        if (h_meta_data_enc.finalize_meta_data != 0) && (h_meta_data_enc.metadata_mode == 0) {
            h_meta_data_enc.submitted_meta_data = DEFAULT_META_DATA_SETUP;
            metadata_mode = h_meta_data_enc.finalize_meta_data;
            h_meta_data_enc.finalize_meta_data = 0;
        }

        // The delay line indexes stay below MAX_DELAY_FRAMES (nMetaDataDelay < 3).
        let wi = (meta_data_delay_write_idx as usize).min(MAX_DELAY_FRAMES - 1);

        // Get last submitted data.
        err = load_submitted_metadata(
            Some(&h_meta_data_enc.submitted_meta_data),
            h_meta_data_enc.n_channels,
            metadata_mode,
            &mut h_meta_data_enc.meta_data_buffer[wi],
        );
        if err != METADATA_OK {
            break 'bail;
        }

        // Calculate compressor if necessary and updata meta data info
        let mm = h_meta_data_enc.meta_data_buffer[wi].metadata_mode;
        if (mm == 1) || (mm == 2) {
            err = process_compressor(
                &mut h_meta_data_enc.meta_data_buffer[wi],
                h_meta_data_enc.h_drc_comp.as_deref_mut(),
                p_audio_samples,
                audio_samples_buf_size,
                n_audio_samples,
            );
            if err != METADATA_OK {
                // Get last submitted data again.
                load_submitted_metadata(
                    Some(&h_meta_data_enc.submitted_meta_data),
                    h_meta_data_enc.n_channels,
                    metadata_mode,
                    &mut h_meta_data_enc.meta_data_buffer[wi],
                );
            }
        }

        // Fill up delay line with initial meta data info.
        if (h_meta_data_enc.initialize_meta_data != 0) && (h_meta_data_enc.metadata_mode != 0) {
            for i in 0..MAX_DELAY_FRAMES {
                if i != wi {
                    h_meta_data_enc.meta_data_buffer[i] = h_meta_data_enc.meta_data_buffer[wi];
                }
            }
            h_meta_data_enc.initialize_meta_data = 0;
        }

        // Convert Meta Data side info to bitstream data.
        fdk_assert!((meta_data_delay_read_idx as usize) < MAX_DELAY_FRAMES);
        let ri = (meta_data_delay_read_idx as usize).min(MAX_DELAY_FRAMES - 1);
        let md = h_meta_data_enc.meta_data_buffer[ri];
        err = write_metadata_payload(h_meta_data_enc, &md);
        if err != METADATA_OK {
            break 'bail;
        }

        // Assign meta data to output
        *pp_meta_data_ext_payload = true;
        *n_meta_data_extensions = h_meta_data_enc.n_extensions as u32;
        *matrix_mixdown_idx = h_meta_data_enc.matrix_mixdown_idx();
    }

    // bail: Compensate audio delay, reset err status.
    let n = if h_meta_data_enc.n_channels == 0 {
        0 // sdiv (arm64); never 0 after a successful initialization
    } else {
        n_audio_samples.wrapping_div(h_meta_data_enc.n_channels)
    };
    err = compensate_audio_delay(h_meta_data_enc, p_audio_samples, audio_samples_buf_size, n);
    let _ = METADATA_ENCODE_ERROR;

    err
}

thread_local! {
    static INPUT_OVERRUNS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn metadata_input_overruns() -> u64 {
    INPUT_OVERRUNS.with(|c| c.get())
}

/// Adds to the counter of [`metadata_input_overruns`] (the carry of the thread pool).
pub(crate) fn add_metadata_input_overruns(n: u64) {
    INPUT_OVERRUNS.with(|c| c.set(c.get().wrapping_add(n)));
}

#[cold]
#[inline(never)]
pub(crate) fn input_overrun() {
    INPUT_OVERRUNS.with(|c| c.set(c.get() + 1));
}

/// `CompensateAudioDelay(hMetaDataEnc, pAudioSamples, audioSamplesBufSize, nAudioSamples)`
pub fn compensate_audio_delay(
    h_meta_data_enc: &mut FdkMetadataEncoder,
    p_audio_samples: &mut [IntPcm],
    audio_samples_buf_size: u32,
    n_audio_samples: i32,
) -> FdkMetadataError {
    let err = METADATA_OK;

    if h_meta_data_enc.n_audio_data_delay != 0 {
        let mut scratch_audio_delay_buffer = [0 as IntPcm; 1024];
        let delay = h_meta_data_enc.n_audio_data_delay;

        for c in 0..h_meta_data_enc.n_channels.max(0) {
            let mut m: i32 = 1024;
            let base = (c as usize).wrapping_mul(audio_samples_buf_size as usize);
            let mut delay_idx = delay;

            loop {
                m = f_min(m, delay_idx);
                delay_idx -= m;

                // The input frame (nAudioSamples) is always longer than the delay (the delay is
                // below the frame length); the port stops otherwise.
                let (Ok(mu), Ok(nu)) = (usize::try_from(m), usize::try_from(n_audio_samples))
                else {
                    break;
                };
                if mu > nu {
                    break;
                }
                if base + nu > p_audio_samples.len() {
                    input_overrun();
                    break;
                }
                let s2 = &mut p_audio_samples[base..base + nu];
                let db = (delay_idx + c * delay) as usize;
                let dbuf = &mut h_meta_data_enc.p_audio_delay_buffer;
                if db + mu > dbuf.len() {
                    break;
                }

                scratch_audio_delay_buffer[..mu].copy_from_slice(&s2[nu - mu..nu]);
                s2.copy_within(0..nu - mu, mu);
                s2[..mu].copy_from_slice(&dbuf[db..db + mu]);
                dbuf[db..db + mu].copy_from_slice(&scratch_audio_delay_buffer[..mu]);

                if delay_idx <= 0 {
                    break;
                }
            }
        }
    }

    err
}

/// `WriteMetadataPayload(hMetaData, pMetadata)`: fills anc data and extension payload
pub fn write_metadata_payload(
    h_meta_data: &mut FdkMetadataEncoder,
    p_metadata: &AacMetadata,
) -> FdkMetadataError {
    h_meta_data.n_extensions = 0;
    h_meta_data.set_matrix_mixdown_idx(-1);

    if p_metadata.metadata_mode != 0 {
        // AAC-DRC
        if (p_metadata.metadata_mode == 1) || (p_metadata.metadata_mode == 2) {
            let n = h_meta_data.n_extensions as usize;
            h_meta_data.ex_payload[n].p_data = h_meta_data.payload_mem[DRC_INFO_PAYLOAD..].as_ptr();
            h_meta_data.ex_payload[n].data_type = EXT_DYNAMIC_RANGE;
            h_meta_data.ex_payload[n].associated_ch_element = -1;

            h_meta_data.ex_payload[n].data_size = write_dynamic_range_info_payload(
                p_metadata,
                &mut h_meta_data.payload_mem,
                DRC_INFO_PAYLOAD,
            ) as u32;

            h_meta_data.n_extensions += 1;
        } // pMetadata->metadataMode==1 || pMetadata->metadataMode==2

        // Matrix Mixdown Coefficient in PCE
        if p_metadata.write_pce_mix_dwn_idx != 0 {
            // Where the reference has no defined result, the port uses 0 (deviation).
            let v = usize::try_from(p_metadata.surround_mix_level as i32)
                .ok()
                .and_then(|i| SURMIX2MATRIX_MIXDOWN_IDX.get(i))
                .copied()
                .unwrap_or(0);
            h_meta_data.set_matrix_mixdown_idx(v as i32);
        }

        // ETSI TS 101 154 (DVB) - MPEG4 ancillary_data()
        if (p_metadata.metadata_mode == 2) || (p_metadata.metadata_mode == 3) {
            // MP4_METADATA_MPEG_ETSI
            let n = h_meta_data.n_extensions as usize;
            h_meta_data.ex_payload[n].p_data = h_meta_data.payload_mem[DRC_DSE_PAYLOAD..].as_ptr();
            h_meta_data.ex_payload[n].data_type = EXT_DATA_ELEMENT;
            h_meta_data.ex_payload[n].associated_ch_element = -1;

            h_meta_data.ex_payload[n].data_size = write_etsi_ancillary_data_payload(
                p_metadata,
                &mut h_meta_data.payload_mem,
                DRC_DSE_PAYLOAD,
            ) as u32;

            h_meta_data.n_extensions += 1;
        } // metadataMode==2 || metadataMode==3
    } // metadataMode != 0

    METADATA_OK
}

/// Initializes a bit writer on `mem[offset..offset + 16]` (`FDKinitBitStream(&bsWriter,
/// pExtensionPayload, 16, 0, BS_WRITER)`): the writer works on a copy of the 16 bytes, which
/// [`payload_done`] writes back.
fn payload_writer(mem: &[u8; PAYLOAD_MEM_SIZE], offset: usize) -> (FdkBitStream, BitMem) {
    let mut bs = FdkBitStream::default();
    fdk_assert!(offset + 16 <= PAYLOAD_MEM_SIZE);
    let window =
        BitMem::from_slice(&mem[offset.min(PAYLOAD_MEM_SIZE)..(offset + 16).min(PAYLOAD_MEM_SIZE)]);
    fdk_init_bit_stream(&mut bs, &window, 16, 0, BS_WRITER);
    (bs, window)
}

/// Writes the bytes of a [`payload_writer`] back to `mem[offset..offset + 16]`.
fn payload_done(mem: &mut [u8; PAYLOAD_MEM_SIZE], offset: usize, window: &BitMem) {
    let end = (offset + 16).min(PAYLOAD_MEM_SIZE);
    window.read(0, &mut mem[offset.min(end)..end]);
}

/// `WriteDynamicRangeInfoPayload(pMetadata, pExtensionPayload)`: returns the number of valid
/// bits of the payload written at `mem[offset..]`.
pub fn write_dynamic_range_info_payload(
    p_metadata: &AacMetadata,
    mem: &mut [u8; PAYLOAD_MEM_SIZE],
    offset: usize,
) -> i32 {
    let pce_tag_present = 0; // yet fixed setting!
    let prog_ref_lev_res_bits = 0;
    let mut drc_num_bands: usize = 1;

    let (mut bs_writer, window) = payload_writer(mem, offset);
    let w = &mut bs_writer;
    let d = &p_metadata.mpeg_drc;

    // dynamic_range_info()
    fdk_write_bits(w, pce_tag_present, 1); // pce_tag_present
    if pce_tag_present != 0 {
        fdk_write_bits(w, 0x0, 4); // pce_instance_tag
        fdk_write_bits(w, 0x0, 4); // drc_tag_reserved_bits
    }

    // Exclude channels
    fdk_write_bits(w, (d.excluded_chns_present != 0) as u32, 1); // excluded_chns_present

    // Multiband DRC
    fdk_write_bits(w, (d.drc_bands_present != 0) as u32, 1); // drc_bands_present
    if d.drc_bands_present != 0 {
        fdk_write_bits(w, d.drc_band_incr as u32, 4); // drc_band_incr
        fdk_write_bits(w, d.drc_interpolation_scheme as u32, 4); // drc_interpolation_scheme
        drc_num_bands += d.drc_band_incr as usize;
        for i in 0..drc_num_bands.min(MAX_DRC_BANDS) {
            fdk_write_bits(w, d.drc_band_top[i] as u32, 8); // drc_band_top
        }
    }

    // Program Reference Level
    fdk_write_bits(w, d.prog_ref_level_present as u32, 1); // prog_ref_level_present
    if d.prog_ref_level_present != 0 {
        fdk_write_bits(w, d.prog_ref_level as u8 as u32, 7); // prog_ref_level
        fdk_write_bits(w, prog_ref_lev_res_bits, 1); // prog_ref_level_reserved_bits
    }

    // DRC Values
    for i in 0..drc_num_bands.min(MAX_DRC_BANDS) {
        fdk_write_bits(w, (d.dyn_rng_sgn[i] != 0) as u32, 1); // dyn_rng_sgn[
        fdk_write_bits(w, d.dyn_rng_ctl[i] as u32, 7); // dyn_rng_ctl
    }

    // return number of valid bits in extension payload.
    let n = fdk_get_valid_bits(w) as i32;
    payload_done(mem, offset, &window);
    n
}

/// `WriteEtsiAncillaryDataPayload(pMetadata, pExtensionPayload)`: returns the number of valid
/// bits of the payload written at `mem[offset..]`.
pub fn write_etsi_ancillary_data_payload(
    p_metadata: &AacMetadata,
    mem: &mut [u8; PAYLOAD_MEM_SIZE],
    offset: usize,
) -> i32 {
    let (mut bs_writer, window) = payload_writer(mem, offset);
    let w = &mut bs_writer;
    let e = &p_metadata.etsi_anc_data;
    let x = &e.ext_anc_data;

    // ancillary_data_sync
    fdk_write_bits(w, 0xBC, 8);

    // bs_info
    fdk_write_bits(w, 0x3, 2); // mpeg_audio_type
    fdk_write_bits(w, p_metadata.dolby_surround_mode as u32, 2); // dolby_surround_mode
    fdk_write_bits(w, p_metadata.drc_presentation_mode as u32, 2); // DRC presentation mode
    fdk_write_bits(w, 0x0, 1); // stereo_downmix_mode
    fdk_write_bits(w, 0x0, 1); // reserved

    // ancillary_data_status
    fdk_write_bits(w, 0, 3); // 3 bit Reserved, set to "0"
    fdk_write_bits(w, (p_metadata.dmx_lvl_on != 0) as u32, 1); // downmixing_levels_MPEG4_status
    fdk_write_bits(w, e.ext_anc_data_status as u32, 1); // ext_anc_data_status
                                                        // audio_coding_mode_and_compression status
    fdk_write_bits(w, (e.compression_on != 0) as u32, 1);
    fdk_write_bits(w, (e.timecode_coarse_status != 0) as u32, 1); // coarse_grain_timecode_status
    fdk_write_bits(w, (e.timecode_fine_status != 0) as u32, 1); // fine_grain_timecode_status

    // downmixing_levels_MPEG4_status
    if p_metadata.dmx_lvl_on != 0 {
        fdk_write_bits(
            w,
            encode_dmx_lvls(p_metadata.center_mix_level, p_metadata.surround_mix_level) as u8
                as u32,
            8,
        );
    }

    // audio_coding_mode_and_compression_status
    if e.compression_on != 0 {
        fdk_write_bits(w, 0x01, 8); // audio coding mode
        fdk_write_bits(w, e.compression_value as u32, 8); // compression value
    }

    // grain-timecode coarse/fine
    if e.timecode_coarse_status != 0 {
        fdk_write_bits(w, 0x0, 16); // not yet supported
    }

    if e.timecode_fine_status != 0 {
        fdk_write_bits(w, 0x0, 16); // not yet supported
    }

    // extended ancillary data structure
    if e.ext_anc_data_status != 0 {
        // ext_ancillary_data_status
        fdk_write_bits(w, 0, 1); // Reserved, set to "0"
        fdk_write_bits(w, x.ext_downmix_lvl_status as u32, 1); // ext_downmixing_levels_status
        fdk_write_bits(w, x.ext_downmix_gain_status as u32, 1); // ext_downmixing_global_gains_status
        fdk_write_bits(w, x.ext_lfe_downmix_status as u32, 1); // ext_downmixing_lfe_level_status"
        fdk_write_bits(w, 0, 4); // Reserved, set to "0"

        // ext_downmixing_levels
        if x.ext_downmix_lvl_status != 0 {
            fdk_write_bits(w, x.ext_dmix_a_idx as u32, 3); // dmix_a_idx
            fdk_write_bits(w, x.ext_dmix_b_idx as u32, 3); // dmix_b_idx
            fdk_write_bits(w, 0, 2); // Reserved, set to "0"
        }

        // ext_downmixing_gains
        if x.ext_downmix_gain_status != 0 {
            fdk_write_bits(w, x.dmx_gain_5_sgn as u32, 1); // dmx_gain_5_sign
            fdk_write_bits(w, x.dmx_gain_5_idx as u32, 6); // dmx_gain_5_idx
            fdk_write_bits(w, 0, 1); // Reserved, set to "0"
            fdk_write_bits(w, x.dmx_gain_2_sgn as u32, 1); // dmx_gain_2_sign
            fdk_write_bits(w, x.dmx_gain_2_idx as u32, 6); // dmx_gain_2_idx
            fdk_write_bits(w, 0, 1); // Reserved, set to "0"
        }

        if x.ext_lfe_downmix_status != 0 {
            fdk_write_bits(w, x.ext_dmix_lfe_idx as u32, 4); // dmix_lfe_idx
            fdk_write_bits(w, 0, 4); // Reserved, set to "0"
        }
    }

    let n = fdk_get_valid_bits(w) as i32;
    payload_done(mem, offset, &window);
    n
}

/// `LoadSubmittedMetadata(hMetadata, nChannels, metadataMode, pAacMetaData)`
pub fn load_submitted_metadata(
    h_metadata: Option<&AacencMetaData>,
    n_channels: i32,
    metadata_mode: i32,
    p_aac_meta_data: &mut AacMetadata,
) -> FdkMetadataError {
    // init struct
    *p_aac_meta_data = AacMetadata::default();

    let Some(h) = h_metadata else {
        p_aac_meta_data.metadata_mode = 0; // there is no configuration available
        return METADATA_OK;
    };
    let p = p_aac_meta_data;

    // convert data
    p.mpeg_drc.drc_profile = h.drc_profile;
    p.etsi_anc_data.comp_profile = h.comp_profile;
    p.mpeg_drc.drc_target_ref_level = h.drc_target_ref_level;
    p.etsi_anc_data.comp_target_ref_level = h.comp_target_ref_level;
    p.mpeg_drc.prog_ref_level_present = h.prog_ref_level_present as u8;
    p.mpeg_drc.prog_ref_level = dialnorm2progreflvl(h.prog_ref_level) as i8;

    p.center_mix_level = h.center_mix_level;
    p.surround_mix_level = h.surround_mix_level;
    p.write_pce_mix_dwn_idx = h.pce_mixdown_idx_present;
    p.dmx_lvl_on = h.etsi_dmx_lvl_present;

    p.etsi_anc_data.compression_on = if h.comp_profile == 256 { 0 } else { 1 };

    if p.mpeg_drc.drc_profile == 256 {
        // AACENC_METADATA_DRC_NOT_PRESENT: MPEG DRC gains are always present in BS syntax
        p.mpeg_drc.drc_profile = 0; // AACENC_METADATA_DRC_NONE
                                    // we should give a warning, but ErrorHandler does not support this
    }

    if n_channels == 2 {
        p.dolby_surround_mode = h.dolby_surround_mode; // dolby_surround_mode
    } else {
        p.dolby_surround_mode = 0;
    }

    p.drc_presentation_mode = h.drc_presentation_mode;
    // override external values if DVB DRC presentation mode is given
    if p.drc_presentation_mode == 1 {
        p.mpeg_drc.drc_target_ref_level = f_max(-(31 << 16), p.mpeg_drc.drc_target_ref_level);
        // implies -23dB
        p.etsi_anc_data.comp_target_ref_level =
            f_max(-(20 << 16), p.etsi_anc_data.comp_target_ref_level);
    }
    if p.drc_presentation_mode == 2 {
        p.mpeg_drc.drc_target_ref_level = f_max(-(23 << 16), p.mpeg_drc.drc_target_ref_level);
        p.etsi_anc_data.comp_target_ref_level =
            f_max(-(23 << 16), p.etsi_anc_data.comp_target_ref_level);
    }
    if p.etsi_anc_data.comp_profile == 256 {
        // DVB defines to revert to Light DRC if heavy is not present
        if p.drc_presentation_mode != 0 {
            // we exclude the "not indicated" mode as this requires the user to define desired
            // levels anyway
            p.mpeg_drc.drc_target_ref_level =
                f_max(p.etsi_anc_data.comp_target_ref_level, p.mpeg_drc.drc_target_ref_level);
        }
    }

    // not yet supported - attention: Update GetEstMetadataBytesPerFrame() if enable this!
    p.etsi_anc_data.timecode_coarse_status = 0;
    p.etsi_anc_data.timecode_fine_status = 0;

    // extended ancillary data
    p.etsi_anc_data.ext_anc_data_status = (h.ext_meta_data.ext_anc_data_enable == 1) as u8;

    let x = &mut p.etsi_anc_data.ext_anc_data;
    let hx = &h.ext_meta_data;
    if p.etsi_anc_data.ext_anc_data_status != 0 {
        x.ext_downmix_lvl_status = (hx.ext_downmix_level_enable != 0) as u8;
        x.ext_downmix_gain_status = (hx.dmx_gain_enable != 0) as u8;
        x.ext_lfe_downmix_status = (hx.lfe_dmx_enable != 0) as u8;

        x.ext_dmix_a_idx = hx.ext_downmix_level_a;
        x.ext_dmix_b_idx = hx.ext_downmix_level_b;

        if x.ext_downmix_gain_status != 0 {
            encode_dynrng(hx.dmx_gain5, &mut x.dmx_gain_5_idx, &mut x.dmx_gain_5_sgn);
            encode_dynrng(hx.dmx_gain2, &mut x.dmx_gain_2_idx, &mut x.dmx_gain_2_sgn);
        } else {
            encode_dynrng(1 << 16, &mut x.dmx_gain_5_idx, &mut x.dmx_gain_5_sgn);
            encode_dynrng(1 << 16, &mut x.dmx_gain_2_idx, &mut x.dmx_gain_2_sgn);
        }

        if x.ext_lfe_downmix_status != 0 {
            x.ext_dmix_lfe_idx = hx.lfe_dmx_level;
        } else {
            x.ext_dmix_lfe_idx = 15; // -inf dB
        }
    } else {
        x.ext_downmix_lvl_status = 0;
        x.ext_downmix_gain_status = 0;
        x.ext_lfe_downmix_status = 0;

        x.ext_dmix_a_idx = 7; // -inf dB
        x.ext_dmix_b_idx = 7; // -inf dB

        encode_dynrng(1 << 16, &mut x.dmx_gain_5_idx, &mut x.dmx_gain_5_sgn);
        encode_dynrng(1 << 16, &mut x.dmx_gain_2_idx, &mut x.dmx_gain_2_sgn);

        x.ext_dmix_lfe_idx = 15; // -inf dB
    }

    p.metadata_mode = metadata_mode as u8;

    METADATA_OK
}

/// `FDK_MetadataEnc_GetDelay(hMetadataEnc)`: delay caused by the Meta Data module.
pub fn fdk_metadata_enc_get_delay(h_metadata_enc: Option<&FdkMetadataEncoder>) -> i32 {
    h_metadata_enc.map_or(0, |h| h.n_audio_data_delay)
}

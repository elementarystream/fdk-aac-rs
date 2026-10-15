// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPEnc/src/tpenc_latm.h, libMpegTPEnc/src/tpenc_latm.cpp.

//! Port of `tpenc_latm.h`: the LATM/LOAS writer (AudioMuxElement, StreamMuxConfig with
//! AudioMuxVersion 0 and 1, PayloadLengthInfo, other data bits, the LOAS sync layer).
//!
//! The code is identical in the `aarch64` and `x86` reference builds (the preprocessed
//! translation units differ only in `FDK_ASSERT`). The division of
//! `allStreamsSameTimeFraming()` (`samplesPerFrame * highestSamplingRate / samplingRate`) is
//! removed by both compilers, because its result is never used (one program with one layer);
//! the port does not compute it either.
//!
//! # Configurations
//!
//! `LATM_STREAM::config[prog][layer]` is a pointer to the `CODER_CONFIG` of a layer, set by
//! `transportEnc_Latm_Init()` to its `layerConfig` argument (in the transport encoder always
//! `&hTpEnc->config`), and written through by `transportEnc_writeASC()` (which may replace a
//! channel mode `MODE_INVALID`). The port keeps only the pointer value as an integer
//! ([`LatmStream::config`], 0 = `NULL`, [`LATM_CONFIG_SET`] otherwise), so that the byte image
//! of the transport encoder's writer union stays exact; the functions that dereference it take
//! the configuration as an argument (`layer_config`).
//!
//! # Programs and layers
//!
//! `LATM_MAX_PROGRAMS` and `LATM_MAX_LAYERS` are 1, and `transportEnc_Latm_Init()` always sets
//! `noProgram = 1`, `noLayer[0] = 1` and `allStreamsSameTimeFraming = 1`. Bounded: loops over
//! more programs stop at `LATM_MAX_PROGRAMS` (PORTING.md, "Deviations from the reference").

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_get_valid_bits, fdk_init_bit_stream_at, fdk_push_back, fdk_push_for,
    fdk_reset_bitbuffer, fdk_sync_cache, fdk_write_bits, FdkBitStream, BS_WRITER,
};
use fdk_aac_rs_fdk::fdk_assert;

use crate::tpenc_asc::transport_enc_write_asc;
use crate::tpenc_lib::*;

/// `DEFAULT_LATM_NR_OF_SUBFRAMES`
pub const DEFAULT_LATM_NR_OF_SUBFRAMES: u8 = 1;
/// `DEFAULT_LATM_SMC_REPEAT`
pub const DEFAULT_LATM_SMC_REPEAT: i32 = 8;
/// `MAX_AAC_LAYERS`
pub const MAX_AAC_LAYERS: usize = 9;
/// `LATM_MAX_PROGRAMS`
pub const LATM_MAX_PROGRAMS: usize = 1;
/// `LATM_MAX_STREAM_ID`
pub const LATM_MAX_STREAM_ID: i32 = 16;
/// `LATM_MAX_LAYERS`
pub const LATM_MAX_LAYERS: usize = 1;
/// `MAX_NR_OF_SUBFRAMES`: set this carefully to avoid buffer overflows
pub const MAX_NR_OF_SUBFRAMES: i32 = 2;

/// `LATM_VAR_MODE`
pub type LatmVarMode = i32;
/// `LATMVAR_SIMPLE_SEQUENCE`
pub const LATMVAR_SIMPLE_SEQUENCE: LatmVarMode = 0;

/// The value the port stores in [`LatmStream::config`] for a configuration that is set (a
/// non-`NULL` pointer in C).
pub const LATM_CONFIG_SET: u64 = 1;

/// `celpFrameLengthTable[]`, dumped with
///   scripts/dump_table.py libMpegTPEnc/src/tpenc_latm.cpp celpFrameLengthTable SHORT
pub static CELP_FRAME_LENGTH_TABLE: [i16; 64] = [
    154, 170, 186, 147, 156, 165, 114, 120, 186, 126, 132, 138, 142, //
    146, 154, 166, 174, 182, 190, 198, 206, 210, 214, 110, 114, 118, //
    120, 122, 218, 230, 242, 254, 266, 278, 286, 294, 318, 342, 358, //
    374, 390, 406, 422, 136, 142, 148, 154, 160, 166, 170, 174, 186, //
    198, 206, 214, 222, 230, 238, 216, 160, 280, 338, 0, 0, //
];

/// `LATM_LAYER_INFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatmLayerInfo {
    pub frame_length_type: i32,
    pub frame_length_bits: i32,
    pub var_frame_length_table: [i32; 4],
    pub stream_id: i32,
}

/// `LATM_STREAM`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatmStream {
    pub m_linfo: [[LatmLayerInfo; LATM_MAX_LAYERS]; LATM_MAX_PROGRAMS],
    /// `CODER_CONFIG *config[][]`: the pointer value (see the module documentation).
    pub config: [[u64; LATM_MAX_LAYERS]; LATM_MAX_PROGRAMS],

    pub var_mode: LatmVarMode,
    pub tt: TransportType,

    pub audio_mux_length_bytes: i32,

    pub audio_mux_length_bytes_pos: i32,
    /// state of the bit reservoir
    pub tara_buffer_fullness: i32,
    pub var_stream_cnt: i32,

    /// Current frame number. Counts modulo muxConfigPeriod
    pub latm_frame_counter: u8,
    /// Distance in frames between MuxConfig
    pub mux_config_period: u8,

    /// AMV1 supports transmission of taraBufferFullness and ASC lengths
    pub audio_mux_version: u8,
    /// for future extensions
    pub audio_mux_version_a: u8,

    pub no_program: u8,
    pub no_layer: [u8; LATM_MAX_PROGRAMS],
    pub fract_delay_present: u8,

    pub all_streams_same_time_framing: u8,
    /// Current Subframe frame
    pub sub_frame_cnt: u8,
    /// Number of subframes
    pub no_subframes: u8,
    /// Position of current subframe start
    pub latm_subframe_start: u32,
    pub no_subframes_next: u8,

    /// AudioMuxElement other data bits
    pub other_data_len_bits: u8,
    /// AudioMuxElement fill bits
    pub fill_bits: u8,
    pub stream_mux_config_bits: u32,
}

/// `HANDLE_LATM_STREAM`
pub type HandleLatmStream<'a> = &'a mut LatmStream;

/// Size of `LATM_STREAM` in the reference builds (88 bytes on the LP64 builds, 80 on
/// ref-i686).
pub const LATM_STREAM_SIZE: usize = if ILP32 { 80 } else { 88 };

/// Offset of the pointer `config[0][0]` (after `m_linfo`, 28 bytes, aligned to the pointer).
const OFF_CONFIG: usize = if ILP32 { 28 } else { 32 };
/// Offset of `varMode`, the field after `config` (40 on the LP64 builds, 32 on ref-i686); the
/// fields after it follow without further padding.
const A: usize = OFF_CONFIG + PTR_SIZE;

impl LatmStream {
    /// Reads the structure from its C layout (the writer union of the transport encoder).
    pub fn load(b: &[u8]) -> Self {
        let li = &mut LatmLayerInfo::default();
        li.frame_length_type = rd_i32(b, 0);
        li.frame_length_bits = rd_i32(b, 4);
        for k in 0..4 {
            li.var_frame_length_table[k] = rd_i32(b, 8 + 4 * k);
        }
        li.stream_id = rd_i32(b, 24);
        LatmStream {
            m_linfo: [[*li]],
            config: [[rd_ptr(b, OFF_CONFIG)]],
            var_mode: rd_i32(b, A),
            tt: rd_i32(b, A + 4),
            audio_mux_length_bytes: rd_i32(b, A + 8),
            audio_mux_length_bytes_pos: rd_i32(b, A + 12),
            tara_buffer_fullness: rd_i32(b, A + 16),
            var_stream_cnt: rd_i32(b, A + 20),
            latm_frame_counter: rd_u8(b, A + 24),
            mux_config_period: rd_u8(b, A + 25),
            audio_mux_version: rd_u8(b, A + 26),
            audio_mux_version_a: rd_u8(b, A + 27),
            no_program: rd_u8(b, A + 28),
            no_layer: [rd_u8(b, A + 29)],
            fract_delay_present: rd_u8(b, A + 30),
            all_streams_same_time_framing: rd_u8(b, A + 31),
            sub_frame_cnt: rd_u8(b, A + 32),
            no_subframes: rd_u8(b, A + 33),
            latm_subframe_start: rd_u32(b, A + 36),
            no_subframes_next: rd_u8(b, A + 40),
            other_data_len_bits: rd_u8(b, A + 41),
            fill_bits: rd_u8(b, A + 42),
            stream_mux_config_bits: rd_u32(b, A + 44),
        }
    }

    /// Writes the structure in its C layout (the padding bytes are left unchanged).
    pub fn store(&self, b: &mut [u8]) {
        let li = &self.m_linfo[0][0];
        wr_i32(b, 0, li.frame_length_type);
        wr_i32(b, 4, li.frame_length_bits);
        for k in 0..4 {
            wr_i32(b, 8 + 4 * k, li.var_frame_length_table[k]);
        }
        wr_i32(b, 24, li.stream_id);
        wr_ptr(b, OFF_CONFIG, self.config[0][0]);
        wr_i32(b, A, self.var_mode);
        wr_i32(b, A + 4, self.tt);
        wr_i32(b, A + 8, self.audio_mux_length_bytes);
        wr_i32(b, A + 12, self.audio_mux_length_bytes_pos);
        wr_i32(b, A + 16, self.tara_buffer_fullness);
        wr_i32(b, A + 20, self.var_stream_cnt);
        wr_u8(b, A + 24, self.latm_frame_counter);
        wr_u8(b, A + 25, self.mux_config_period);
        wr_u8(b, A + 26, self.audio_mux_version);
        wr_u8(b, A + 27, self.audio_mux_version_a);
        wr_u8(b, A + 28, self.no_program);
        wr_u8(b, A + 29, self.no_layer[0]);
        wr_u8(b, A + 30, self.fract_delay_present);
        wr_u8(b, A + 31, self.all_streams_same_time_framing);
        wr_u8(b, A + 32, self.sub_frame_cnt);
        wr_u8(b, A + 33, self.no_subframes);
        wr_u32(b, A + 36, self.latm_subframe_start);
        wr_u8(b, A + 40, self.no_subframes_next);
        wr_u8(b, A + 41, self.other_data_len_bits);
        wr_u8(b, A + 42, self.fill_bits);
        wr_u32(b, A + 44, self.stream_mux_config_bits);
    }
}

/// Number of programs the loops visit: `noProgram`, limited to the arrays (see the module
/// documentation).
#[inline]
fn programs(n: u8) -> usize {
    (n as usize).min(LATM_MAX_PROGRAMS)
}

/// `transportEnc_LatmWriteValue(hBs, value)`: writes a LATM value (2 bits byte count, then 1..4
/// bytes, most significant first). Returns the number of bits written.
pub fn transport_enc_latm_write_value(h_bs: &mut FdkBitStream, value: i32) -> u32 {
    let value_bytes: u8 = if value < (1 << 8) {
        1
    } else if value < (1 << 16) {
        2
    } else if value < (1 << 24) {
        3
    } else {
        4
    };

    fdk_write_bits(h_bs, (value_bytes - 1) as u32, 2); // size of value in Bytes
    for i in 0..value_bytes as i32 {
        // write most significant Byte first
        let b = value.wrapping_shr(((value_bytes as i32 - 1 - i) << 3) as u32) as u8;
        fdk_write_bits(h_bs, b as u32, 8);
    }

    ((value_bytes as u32) << 3) + 2
}

/// `transportEnc_LatmCountFixBitDemandHeader(hAss)`: the bits of the LOAS header, the
/// AudioMuxElement flags, the StreamMuxConfig (if due), the other data and the byte alignment
/// (also stores `fill_bits`). 0 for subframes after the first.
pub fn transport_enc_latm_count_fix_bit_demand_header(h_ass: &mut LatmStream) -> u32 {
    let mut bit_demand: i32 = 0;
    let insert_setup_data: i32;

    // only if start of new latm frame
    if h_ass.sub_frame_cnt == 0 {
        // AudioSyncStream

        if h_ass.tt == TT_MP4_LOAS {
            bit_demand += 11; // syncword
            bit_demand += 13; // audioMuxLengthBytes
        }

        // AudioMuxElement

        // AudioMuxElement::Stream Mux Config
        if h_ass.mux_config_period > 0 {
            insert_setup_data = (h_ass.latm_frame_counter == 0) as i32;
        } else {
            insert_setup_data = 0;
        }

        if h_ass.tt != TT_MP4_LATM_MCP0 {
            // AudioMuxElement::useSameStreamMux Flag
            bit_demand += 1;

            if insert_setup_data != 0 {
                // int += UINT
                bit_demand = (bit_demand as u32).wrapping_add(h_ass.stream_mux_config_bits) as i32;
            }
        }

        // AudioMuxElement::otherDataBits
        bit_demand = bit_demand.wrapping_add(h_ass.other_data_len_bits as i32);

        // AudioMuxElement::ByteAlign
        if bit_demand % 8 != 0 {
            h_ass.fill_bits = (8 - (bit_demand % 8)) as u8;
            bit_demand = bit_demand.wrapping_add(h_ass.fill_bits as i32);
        } else {
            h_ass.fill_bits = 0;
        }
    }

    bit_demand as u32
}

/// `while (streamDataLength >= (255 << 3)) { bitDemand += 8; streamDataLength -= (255 << 3); }`
/// in closed form (the same result; `streamDataLength` can be close to 2^32 after a wrap).
#[inline]
fn payload_length_bytes(bit_demand: &mut i32, stream_data_length: &mut u32) {
    let n = *stream_data_length / (255 << 3);
    *bit_demand = bit_demand.wrapping_add((n as i32).wrapping_mul(8));
    *stream_data_length -= n * (255 << 3);
}

/// `transportEnc_LatmCountVarBitDemandHeader(hAss, streamDataLength)`: the bits of the
/// PayloadLengthInfo for a payload of `stream_data_length` bits.
pub fn transport_enc_latm_count_var_bit_demand_header(
    h_ass: &mut LatmStream,
    mut stream_data_length: u32,
) -> u32 {
    let mut bit_demand: i32 = 0;

    // Payload Length Info
    if h_ass.all_streams_same_time_framing != 0 {
        for prog in 0..programs(h_ass.no_program) {
            for layer in 0..LATM_MAX_LAYERS {
                let p_linfo = &h_ass.m_linfo[prog][layer];

                if p_linfo.stream_id >= 0 {
                    match p_linfo.frame_length_type {
                        0 => {
                            if stream_data_length > 0 {
                                stream_data_length =
                                    stream_data_length.wrapping_sub(bit_demand as u32);
                                payload_length_bytes(&mut bit_demand, &mut stream_data_length);
                                bit_demand = bit_demand.wrapping_add(8);
                            }
                        }
                        1 | 4 | 6 => {
                            bit_demand = bit_demand.wrapping_add(2);
                        }
                        _ => return 0,
                    }
                }
            }
        }
    } else {
        // there are many possibilities to use this mechanism.
        match h_ass.var_mode {
            LATMVAR_SIMPLE_SEQUENCE => {
                // Use the sequence generated by the encoder
                bit_demand += 4;

                h_ass.var_stream_cnt = 0;
                for prog in 0..programs(h_ass.no_program) {
                    for layer in 0..LATM_MAX_LAYERS {
                        let p_linfo = h_ass.m_linfo[prog][layer];

                        if p_linfo.stream_id >= 0 {
                            bit_demand = bit_demand.wrapping_add(4); // streamID
                            match p_linfo.frame_length_type {
                                0 => {
                                    stream_data_length =
                                        stream_data_length.wrapping_sub(bit_demand as u32);
                                    payload_length_bytes(&mut bit_demand, &mut stream_data_length);

                                    bit_demand = bit_demand.wrapping_add(8);
                                }
                                1 | 4 | 6 => {}
                                _ => return 0,
                            }
                            h_ass.var_stream_cnt = h_ass.var_stream_cnt.wrapping_add(1);
                        }
                    }
                }
                bit_demand = bit_demand.wrapping_add(4);
            }
            _ => return 0,
        }
    }

    bit_demand as u32
}

/// `CreateStreamMuxConfig(hAss, hBs, bufferFullness, cb)`: writes a StreamMuxConfig and counts
/// its bits in `stream_mux_config_bits`. `layer_config` is the configuration
/// `hAss->config[0][0]` points to.
pub fn create_stream_mux_config(
    h_ass: &mut LatmStream,
    h_bs: &mut FdkBitStream,
    buffer_fullness: i32,
    layer_config: &mut CoderConfig,
    cb: &mut dyn TpEncCallbacks,
) -> TransportEncError {
    let mut stream_idcnt: i32;
    let core_frame_offset: u16 = 0;

    h_ass.tara_buffer_fullness = 0xFF;
    h_ass.audio_mux_version_a = 0; // for future extensions
    h_ass.stream_mux_config_bits = 0;

    fdk_write_bits(h_bs, h_ass.audio_mux_version as u32, 1); // audioMuxVersion
    h_ass.stream_mux_config_bits += 1;

    if h_ass.audio_mux_version == 1 {
        fdk_write_bits(h_bs, h_ass.audio_mux_version_a as u32, 1); // audioMuxVersionA
        h_ass.stream_mux_config_bits += 1;
    }

    if h_ass.audio_mux_version_a == 0 {
        if h_ass.audio_mux_version == 1 {
            // taraBufferFullness
            h_ass.stream_mux_config_bits = h_ass
                .stream_mux_config_bits
                .wrapping_add(transport_enc_latm_write_value(h_bs, h_ass.tara_buffer_fullness));
        }
        // allStreamsSameTimeFraming
        fdk_write_bits(h_bs, (h_ass.all_streams_same_time_framing != 0) as u32, 1);
        // Number of Subframes
        fdk_write_bits(h_bs, (h_ass.no_subframes as i32 - 1) as u32, 6);
        // Number of Programs
        fdk_write_bits(h_bs, (h_ass.no_program as i32 - 1) as u32, 4);

        h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(11);

        stream_idcnt = 0;
        for prog in 0..programs(h_ass.no_program) {
            let mut trans_layer: i32 = 0;

            fdk_write_bits(h_bs, (h_ass.no_layer[prog] as i32 - 1) as u32, 3);
            h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(3);

            for layer in 0..LATM_MAX_LAYERS {
                h_ass.m_linfo[prog][layer].stream_id = -1;

                if h_ass.config[prog][layer] != 0 {
                    let use_same_config: i32 = 0;

                    if trans_layer > 0 {
                        fdk_write_bits(h_bs, (use_same_config != 0) as u32, 1);
                        h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(1);
                    }
                    if use_same_config == 0 || trans_layer == 0 {
                        let align_anchor: u32 = fdk_get_valid_bits(h_bs);

                        if 0 != transport_enc_write_asc(h_bs, layer_config, cb) {
                            return TRANSPORTENC_UNKOWN_ERROR;
                        }

                        if h_ass.audio_mux_version == 1 {
                            let mut asc_len: u32 = transport_enc_latm_write_value(h_bs, 0);
                            fdk_byte_align(h_bs, align_anchor);
                            asc_len = fdk_get_valid_bits(h_bs)
                                .wrapping_sub(align_anchor)
                                .wrapping_sub(asc_len);
                            let back = fdk_get_valid_bits(h_bs).wrapping_sub(align_anchor);
                            fdk_push_back(h_bs, back);

                            transport_enc_latm_write_value(h_bs, asc_len as i32);

                            if 0 != transport_enc_write_asc(h_bs, layer_config, cb) {
                                return TRANSPORTENC_UNKOWN_ERROR;
                            }

                            fdk_byte_align(h_bs, align_anchor); // asc length fillbits
                        }

                        // add asc length to smc summary
                        let v = fdk_get_valid_bits(h_bs).wrapping_sub(align_anchor);
                        h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(v);
                    }
                    trans_layer += 1;

                    if h_ass.all_streams_same_time_framing == 0
                        && stream_idcnt >= LATM_MAX_STREAM_ID
                    {
                        return TRANSPORTENC_INVALID_CONFIG;
                    }
                    h_ass.m_linfo[prog][layer].stream_id = stream_idcnt;
                    stream_idcnt += 1;

                    let p_lci = &*layer_config;
                    let p_linfo = &mut h_ass.m_linfo[prog][layer];
                    match p_lci.aot {
                        AOT_AAC_MAIN | AOT_AAC_LC | AOT_AAC_SSR | AOT_AAC_LTP | AOT_AAC_SCAL
                        | AOT_ER_AAC_LD | AOT_ER_AAC_ELD | AOT_USAC => {
                            p_linfo.frame_length_type = 0;

                            fdk_write_bits(h_bs, p_linfo.frame_length_type as u32, 3); // frameLengthType
                            fdk_write_bits(h_bs, buffer_fullness as u32, 8); // bufferFullness
                            h_ass.stream_mux_config_bits =
                                h_ass.stream_mux_config_bits.wrapping_add(11);

                            if h_ass.all_streams_same_time_framing == 0 {
                                // `hAss->config[prog][layer - 1]` has no defined result in the
                                // reference here; the port treats it as a configuration that is
                                // not CELP.
                                let prev_is_celp = false;
                                if (p_lci.aot == AOT_AAC_SCAL || p_lci.aot == AOT_ER_AAC_SCAL)
                                    && prev_is_celp
                                {
                                    fdk_write_bits(h_bs, core_frame_offset as u32, 6); // coreFrameOffset
                                    h_ass.stream_mux_config_bits =
                                        h_ass.stream_mux_config_bits.wrapping_add(6);
                                }
                            }
                        }

                        AOT_TWIN_VQ => {
                            p_linfo.frame_length_type = 1;
                            // transmission frame length in bytes
                            let tmp = (p_lci.bits_frame.wrapping_add(7) >> 3).wrapping_sub(20);
                            if tmp < 0 {
                                return TRANSPORTENC_INVALID_TRANSMISSION_FRAME_LENGTH;
                            }
                            fdk_write_bits(h_bs, p_linfo.frame_length_type as u32, 3); // frameLengthType
                            fdk_write_bits(h_bs, tmp as u32, 9);
                            h_ass.stream_mux_config_bits =
                                h_ass.stream_mux_config_bits.wrapping_add(12);

                            p_linfo.frame_length_bits = tmp.wrapping_add(20).wrapping_shl(3);
                        }

                        AOT_CELP => {
                            p_linfo.frame_length_type = 4;
                            fdk_write_bits(h_bs, p_linfo.frame_length_type as u32, 3); // frameLengthType
                            h_ass.stream_mux_config_bits =
                                h_ass.stream_mux_config_bits.wrapping_add(3);
                            {
                                let mut i = 0usize;
                                while i < 62 {
                                    if CELP_FRAME_LENGTH_TABLE[i] as i32 == p_lci.bits_frame {
                                        break;
                                    }
                                    i += 1;
                                }
                                if i >= 62 {
                                    return TRANSPORTENC_INVALID_CELP_FRAME_LENGTH;
                                }

                                fdk_write_bits(h_bs, i as u32, 6); // CELPframeLengthTabelIndex
                                h_ass.stream_mux_config_bits =
                                    h_ass.stream_mux_config_bits.wrapping_add(6);
                            }
                            p_linfo.frame_length_bits = p_lci.bits_frame;
                        }

                        AOT_HVXC => {
                            p_linfo.frame_length_type = 6;
                            fdk_write_bits(h_bs, p_linfo.frame_length_type as u32, 3); // frameLengthType
                            h_ass.stream_mux_config_bits =
                                h_ass.stream_mux_config_bits.wrapping_add(3);
                            {
                                let i: u32 = if p_lci.bits_frame == 40 {
                                    0
                                } else if p_lci.bits_frame == 80 {
                                    1
                                } else {
                                    return TRANSPORTENC_INVALID_FRAME_BITS;
                                };
                                fdk_write_bits(h_bs, i, 1); // HVXCframeLengthTableIndex
                                h_ass.stream_mux_config_bits =
                                    h_ass.stream_mux_config_bits.wrapping_add(1);
                            }
                            p_linfo.frame_length_bits = p_lci.bits_frame;
                        }

                        _ => return TRANSPORTENC_INVALID_AOT, // AOT_NULL_OBJECT and others
                    }
                }
            }
        }

        // otherDataPresent
        fdk_write_bits(h_bs, (h_ass.other_data_len_bits > 0) as u32, 1);
        h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(1);

        if h_ass.other_data_len_bits > 0 {
            fdk_write_bits(h_bs, 0, 1);
            fdk_write_bits(h_bs, h_ass.other_data_len_bits as u32, 8);
            h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(9);
        }

        fdk_write_bits(h_bs, 0, 1); // crcCheckPresent=0
        h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(1);
    } else {
        // if ( audioMuxVersionA == 0 )

        // for future extensions
    }

    TRANSPORTENC_OK
}

/// `WriteAuPayloadLengthInfo(hBitStream, AuLengthBits)`: the PayloadLengthInfo of an access
/// unit of `au_length_bits` bits (a multiple of 8).
pub fn write_au_payload_length_info(
    h_bit_stream: &mut FdkBitStream,
    mut au_length_bits: i32,
) -> TransportEncError {
    if au_length_bits % 8 != 0 {
        return TRANSPORTENC_INVALID_AU_LENGTH;
    }

    while au_length_bits >= 255 * 8 {
        fdk_write_bits(h_bit_stream, 255, 8); // 255 shows incomplete AU
        au_length_bits -= 255 * 8;
    }

    let rest_bytes = au_length_bits >> 3;
    fdk_write_bits(h_bit_stream, rest_bytes as u32, 8);

    TRANSPORTENC_OK
}

/// `transportEnc_LatmSetNrOfSubframes(hAss, noSubframes_next)`: the number of access units per
/// LATM frame (1 or 2); taken over at once only at the start of the stream.
pub fn transport_enc_latm_set_nr_of_subframes(
    h_ass: &mut LatmStream,
    no_subframes_next: i32,
) -> TransportEncError {
    // sanity chk
    if no_subframes_next < 1 || no_subframes_next > MAX_NR_OF_SUBFRAMES {
        return TRANSPORTENC_LATM_INVALID_NR_OF_SUBFRAMES;
    }

    h_ass.no_subframes_next = no_subframes_next as u8;

    // if at start then we can take over the value immediately, otherwise we have to wait for
    // the next SMC
    if h_ass.sub_frame_cnt == 0 && h_ass.latm_frame_counter == 0 {
        h_ass.no_subframes = no_subframes_next as u8;
    }

    TRANSPORTENC_OK
}

/// `allStreamsSameTimeFraming(hAss, noProgram, noLayer)`: counts the layers per program into
/// `h_ass.no_layer` and returns whether all layers have the same frame duration (always 1 with
/// one layer; see the module documentation for the unused division).
pub fn all_streams_same_time_framing(h_ass: &mut LatmStream, no_program: u8) -> i32 {
    for prog in 0..programs(no_program) {
        h_ass.no_layer[prog] = 0;

        for layer in 0..LATM_MAX_LAYERS {
            if h_ass.config[prog][layer] != 0 {
                h_ass.no_layer[prog] = h_ass.no_layer[prog].wrapping_add(1);

                // hsfSamplesFrame = samplesPerFrame * highestSamplingRate / samplingRate is only
                // compared with the value of an earlier layer (and the unused minimum and
                // maximum); with one program of one layer there is none.
            }
        }
    }

    1
}

/// `transportEnc_InitLatmStream(hAss, fractDelayPresent, muxConfigPeriod, audioMuxVersion,
/// tt)`: initializes the LATM/LOAS stream (program 0, layer 0).
pub fn transport_enc_init_latm_stream(
    h_ass: &mut LatmStream,
    fract_delay_present: i32,
    mux_config_period: i32, // insert setup data every muxConfigPeriod frames
    audio_mux_version: u32,
    tt: TransportType,
) -> TransportEncError {
    let error_status = TRANSPORTENC_OK;

    h_ass.tt = tt;

    h_ass.no_program = 1;

    h_ass.audio_mux_version = audio_mux_version as u8;

    // Fill noLayer array using hAss->config
    h_ass.all_streams_same_time_framing =
        all_streams_same_time_framing(h_ass, h_ass.no_program) as u8;
    // Only allStreamsSameTimeFraming==1 is supported
    fdk_assert!(h_ass.all_streams_same_time_framing != 0);

    h_ass.fract_delay_present = fract_delay_present as u8;
    h_ass.other_data_len_bits = 0;

    h_ass.var_mode = LATMVAR_SIMPLE_SEQUENCE;

    // initialize counters
    h_ass.sub_frame_cnt = 0;
    h_ass.no_subframes = DEFAULT_LATM_NR_OF_SUBFRAMES;
    h_ass.no_subframes_next = DEFAULT_LATM_NR_OF_SUBFRAMES;

    // sync layer related
    h_ass.audio_mux_length_bytes = 0;

    h_ass.latm_frame_counter = 0;
    h_ass.mux_config_period = mux_config_period as u8;

    error_status
}

/// `transportEnc_LatmCountTotalBitDemandHeader(hAss, streamDataLength)`: the bits of the next
/// LATM/LOAS header for a payload of `stream_data_length` bits.
pub fn transport_enc_latm_count_total_bit_demand_header(
    h_ass: &mut LatmStream,
    stream_data_length: u32,
) -> u32 {
    let mut bit_demand: u32 = 0;

    match h_ass.tt {
        TT_MP4_LOAS | TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 => {
            if h_ass.sub_frame_cnt == 0 {
                bit_demand = transport_enc_latm_count_fix_bit_demand_header(h_ass);
            }
            bit_demand = bit_demand.wrapping_add(transport_enc_latm_count_var_bit_demand_header(
                h_ass,
                stream_data_length, // - bitDemand
            ));
        }
        _ => {}
    }

    bit_demand
}

/// `AdvanceAudioMuxElement(hAss, hBs, auBits, bufferFullness, cb)`: writes the start of an
/// AudioMuxElement (useSameStreamMux and the StreamMuxConfig if due) and the
/// PayloadLengthInfo.
pub fn advance_audio_mux_element(
    h_ass: &mut LatmStream,
    h_bs: &mut FdkBitStream,
    au_bits: i32,
    buffer_fullness: i32,
    layer_config: &mut CoderConfig,
    cb: &mut dyn TpEncCallbacks,
) -> TransportEncError {
    // Insert setup data to assemble Buffer
    if h_ass.sub_frame_cnt == 0 {
        let insert_mux_setup =
            if h_ass.mux_config_period > 0 { (h_ass.latm_frame_counter == 0) as i32 } else { 0 };

        if h_ass.tt != TT_MP4_LATM_MCP0 {
            if insert_mux_setup != 0 {
                fdk_write_bits(h_bs, 0, 1); // useSameStreamMux useNewStreamMux
                let error_status =
                    create_stream_mux_config(h_ass, h_bs, buffer_fullness, layer_config, cb);
                if TRANSPORTENC_OK != error_status {
                    return error_status;
                }
            } else {
                fdk_write_bits(h_bs, 1, 1); // useSameStreamMux
            }
        }
    }

    // PayloadLengthInfo
    for prog in 0..programs(h_ass.no_program) {
        for _layer in 0..h_ass.no_layer[prog] {
            let error_status = write_au_payload_length_info(h_bs, au_bits);
            if error_status != TRANSPORTENC_OK {
                return error_status;
            }
        }
    }
    // At this point comes the access unit.

    TRANSPORTENC_OK
}

/// `transportEnc_LatmWrite(hAss, hBitstream, auBits, bufferFullness, cb)`: writes the LATM/LOAS
/// header of an access unit (a new frame for the first subframe).
pub fn transport_enc_latm_write(
    h_ass: &mut LatmStream,
    h_bs: &mut FdkBitStream,
    au_bits: i32,
    buffer_fullness: i32,
    layer_config: &mut CoderConfig,
    cb: &mut dyn TpEncCallbacks,
) -> TransportEncError {
    if h_ass.sub_frame_cnt == 0 {
        // Start new frame
        fdk_reset_bitbuffer(h_bs, BS_WRITER);
    }

    h_ass.latm_subframe_start = fdk_get_valid_bits(h_bs);

    // Insert syncword and syncword distance
    // - only if loas
    // - we must update the syncword distance (=audiomuxlengthbytes) later
    if h_ass.tt == TT_MP4_LOAS && h_ass.sub_frame_cnt == 0 {
        // Start new LOAS frame
        fdk_write_bits(h_bs, 0x2B7, 11);
        h_ass.audio_mux_length_bytes = 0;
        h_ass.audio_mux_length_bytes_pos = fdk_get_valid_bits(h_bs) as i32; // store read pointer position
        fdk_write_bits(h_bs, h_ass.audio_mux_length_bytes as u32, 13);
    }

    advance_audio_mux_element(h_ass, h_bs, au_bits, buffer_fullness, layer_config, cb)
}

/// `transportEnc_LatmAdjustSubframeBits(hAss, pBits)`: makes the frame bit count `*bits`
/// relative to the current subframe (plus the other data and fill bits of the first one).
pub fn transport_enc_latm_adjust_subframe_bits(h_ass: &LatmStream, bits: &mut i32) {
    // Substract bits from possible previous subframe (int -= UINT)
    *bits = (*bits as u32).wrapping_sub(h_ass.latm_subframe_start) as i32;
    // Add fill bits
    if h_ass.sub_frame_cnt == 0 {
        *bits = bits.wrapping_add(h_ass.other_data_len_bits as i32);
        *bits = bits.wrapping_add(h_ass.fill_bits as i32);
    }
}

/// `FDKgetValidBits()` of a writer without synchronizing the cache (for assertions, which must
/// not change the bitstream).
#[inline]
fn valid_bits_no_sync(h_bs: &FdkBitStream) -> u32 {
    h_bs.h_bit_buf.valid_bits.wrapping_add(h_bs.bits_in_cache)
}

/// `transportEnc_LatmGetFrame(hAss, hBs, pBytes)`: completes the LATM/LOAS frame after its last
/// subframe (LOAS length, other data, alignment) and returns its size in `*p_bytes` (0 while
/// subframes are missing; unchanged on an error).
pub fn transport_enc_latm_get_frame(
    h_ass: &mut LatmStream,
    h_bs: &mut FdkBitStream,
    p_bytes: &mut i32,
) -> TransportEncError {
    h_ass.sub_frame_cnt = h_ass.sub_frame_cnt.wrapping_add(1);
    if h_ass.sub_frame_cnt >= h_ass.no_subframes {
        // Add LOAS frame length if required.
        if h_ass.tt == TT_MP4_LOAS {
            let mut tmp_buf = FdkBitStream::default();

            // Determine frame length info (UINT arithmetic)
            h_ass.audio_mux_length_bytes = (fdk_get_valid_bits(h_bs)
                .wrapping_add(h_ass.other_data_len_bits as u32)
                .wrapping_add(7)
                >> 3)
                .wrapping_sub(3) as i32; // 3=Syncword + length

            // Check frame length info
            if h_ass.audio_mux_length_bytes >= (1 << 13) {
                return TRANSPORTENC_INVALID_AU_LENGTH;
            }

            // Write length info into assembler buffer
            fdk_init_bit_stream_at(
                &mut tmp_buf,
                h_bs.h_bit_buf.mem(),
                h_bs.h_bit_buf.base(),
                h_bs.h_bit_buf.buf_size(),
                0,
                BS_WRITER,
            );
            fdk_push_for(&mut tmp_buf, h_ass.audio_mux_length_bytes_pos as u32);
            fdk_write_bits(&mut tmp_buf, h_ass.audio_mux_length_bytes as u32, 13);
            fdk_sync_cache(&mut tmp_buf);
        }

        // Write AudioMuxElement other data bits
        fdk_write_bits(h_bs, 0, h_ass.other_data_len_bits as u32);

        // Write AudioMuxElement byte alignment fill bits
        fdk_write_bits(h_bs, 0, h_ass.fill_bits as u32);

        fdk_assert!(valid_bits_no_sync(h_bs) % 8 == 0);

        h_ass.sub_frame_cnt = 0;

        fdk_sync_cache(h_bs);
        *p_bytes = (fdk_get_valid_bits(h_bs).wrapping_add(7) >> 3) as i32;

        if h_ass.mux_config_period > 0 {
            h_ass.latm_frame_counter = h_ass.latm_frame_counter.wrapping_add(1);

            if h_ass.latm_frame_counter >= h_ass.mux_config_period {
                h_ass.latm_frame_counter = 0;
                h_ass.no_subframes = h_ass.no_subframes_next;
            }
        }
    } else {
        // No data this time
        *p_bytes = 0;
    }

    TRANSPORTENC_OK
}

/// `transportEnc_Latm_Init(hAss, hBs, layerConfig, audioMuxVersion, tt, cb)`: initializes the
/// LATM/LOAS writer with one program of one layer (`layer_config`) and writes the first
/// StreamMuxConfig into `h_bs` to determine its size.
pub fn transport_enc_latm_init(
    h_ass: &mut LatmStream,
    h_bs: &mut FdkBitStream,
    layer_config: &mut CoderConfig,
    audio_mux_version: u32,
    tt: TransportType,
    cb: &mut dyn TpEncCallbacks,
) -> TransportEncError {
    let fract_delay_present: i32 = 0;

    let setup_data_distance_frames: i32 = layer_config.header_period as i32;

    fdk_assert!(setup_data_distance_frames >= 0);

    for prog in 0..LATM_MAX_PROGRAMS {
        for layer in 0..LATM_MAX_LAYERS {
            h_ass.config[prog][layer] = 0;
            h_ass.m_linfo[prog][layer].stream_id = -1;
        }
    }

    h_ass.config[0][0] = LATM_CONFIG_SET;
    h_ass.m_linfo[0][0].stream_id = 0;

    let mut error_status = transport_enc_init_latm_stream(
        h_ass,
        fract_delay_present,
        setup_data_distance_frames,
        (audio_mux_version != 0) as u32,
        tt,
    );
    if error_status != TRANSPORTENC_OK {
        return error_status;
    }

    error_status = transport_enc_latm_set_nr_of_subframes(h_ass, layer_config.n_sub_frames);
    if error_status != TRANSPORTENC_OK {
        return error_status;
    }

    // Get the size of the StreamMuxConfig somehow
    advance_audio_mux_element(h_ass, h_bs, 0, 0, layer_config, cb)
}

/// `transportEnc_LatmAddOtherDataBits(hAss, otherDataBits)`: adds other data bits to every
/// AudioMuxElement (once, whole bytes only).
///
/// For MCP1/LOAS the C code stores `otherDataBits - 9` in a `UCHAR`, so 0 and 8 give 247 and
/// 255 bits. Where `transportEnc_LatmGetFrame()` has no defined result, the port deviates.
pub fn transport_enc_latm_add_other_data_bits(
    h_ass: &mut LatmStream,
    other_data_bits: i32,
) -> TransportEncError {
    let mut error_status = TRANSPORTENC_OK;

    if h_ass.other_data_len_bits != 0 || other_data_bits % 8 != 0 {
        // This implementation allows to add other data bits only once.
        // To keep existing alignment only whole bytes are allowed.
        error_status = TRANSPORTENC_UNKOWN_ERROR;
    } else {
        // Ensure correct addional bits in payload.
        if h_ass.tt == TT_MP4_LATM_MCP0 {
            h_ass.other_data_len_bits = other_data_bits as u8;
        } else {
            h_ass.other_data_len_bits = other_data_bits.wrapping_sub(9) as u8;
            h_ass.stream_mux_config_bits = h_ass.stream_mux_config_bits.wrapping_add(9);
        }
    }

    error_status
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/bit_sbr.h, libSBRenc/src/bit_sbr.cpp.

//! Port of `bit_sbr.h` and `bit_sbr.cpp`: writing of the SBR header and the SBR data of single
//! channel and channel pair elements.
//!
//! `SBR_ENV_DATA::hSbrBSGrid` points to the bitstream grid of the channel's envelope frame,
//! or (with coupling) to the grid of the left channel. In the port it is the index of the
//! channel within the element ([`SbrEnvData::h_sbr_bs_grid`]), and the writers take the
//! envelope data together with the resolved grid ([`EnvRef`]).

use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_push_back, fdk_write_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

use crate::cmondata::CommonData;
use crate::fram_gen::*;
use crate::mh_det::ENV_DATA_TAIL_BYTES;
use crate::ps_main::{fdk_sbr_enc_ps_enc_write_ps_data, ParametricStereo};
use crate::sbr_def::*;
use crate::sbr_encoder::{FreqRes, SBR_SYNTAX_SCALABLE};

/// `struct SBR_BITSTREAM_DATA` (`HANDLE_SBR_BITSTREAM_DATA` is `&mut SbrBitstreamData`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrBitstreamData {
    /// `TotalBits`
    pub total_bits: i32,
    /// `PayloadBits`
    pub payload_bits: i32,
    /// `FillBits`
    pub fill_bits: i32,
    /// `HeaderActive`
    pub header_active: i32,
    /// `HeaderActiveDelay`: sbr payload and its header is delayed depending on encoder
    /// configuration
    pub header_active_delay: i32,
    /// `NrSendHeaderData`: input from commandline
    pub nr_send_header_data: i32,
    /// `CountSendHeaderData`: modulo count. If < 0 then no counting is done (no SBR headers)
    pub count_send_header_data: i32,
    /// `rightBorderFIX`: force VARFIX or FIXFIX frames
    pub right_border_fix: i32,
}

/// `struct SBR_HEADER_DATA` (`HANDLE_SBR_HEADER_DATA` is `&mut SbrHeaderData`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrHeaderData {
    /// `sbr_amp_res`
    pub sbr_amp_res: AmpRes,
    /// `sbr_start_frequency`
    pub sbr_start_frequency: i32,
    /// `sbr_stop_frequency`
    pub sbr_stop_frequency: i32,
    /// `sbr_xover_band`
    pub sbr_xover_band: i32,
    /// `sbr_noise_bands`
    pub sbr_noise_bands: i32,
    /// `sbr_data_extra`
    pub sbr_data_extra: i32,
    /// `header_extra_1`
    pub header_extra_1: i32,
    /// `header_extra_2`
    pub header_extra_2: i32,
    /// `sbr_lc_stereo_mode`
    pub sbr_lc_stereo_mode: i32,
    /// `sbr_limiter_bands`
    pub sbr_limiter_bands: i32,
    /// `sbr_limiter_gains`
    pub sbr_limiter_gains: i32,
    /// `sbr_interpol_freq`
    pub sbr_interpol_freq: i32,
    /// `sbr_smoothing_length`
    pub sbr_smoothing_length: i32,
    /// `alterScale`
    pub alter_scale: i32,
    /// `freqScale`
    pub freq_scale: i32,

    // element of channelpairelement
    /// `coupling`
    pub coupling: i32,
    /// `prev_coupling`
    pub prev_coupling: i32,
}

/// `struct SBR_ENV_DATA` (`HANDLE_SBR_ENV_DATA` is `&mut SbrEnvData`). The Huffman tables are
/// references to the tables of [`crate::sbrenc_rom`] (`None` for `NULL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrEnvData {
    /// `sbr_xpos_ctrl`
    pub sbr_xpos_ctrl: i32,
    /// `freq_res_fixfix`
    pub freq_res_fixfix: [FreqRes; 2],
    /// `fResTransIsLow`
    pub f_res_trans_is_low: u8,

    /// `sbr_invf_mode`
    pub sbr_invf_mode: InvfMode,
    /// `sbr_invf_mode_vec`
    pub sbr_invf_mode_vec: [InvfMode; MAX_NUM_NOISE_VALUES],

    /// `sbr_xpos_mode`
    pub sbr_xpos_mode: XposMode,

    /// `ienvelope`
    pub ienvelope: [[i32; MAX_FREQ_COEFFS]; MAX_ENVELOPES],

    /// `codeBookScfLavBalance`
    pub code_book_scf_lav_balance: i32,
    /// `codeBookScfLav`
    pub code_book_scf_lav: i32,
    /// `hufftableTimeC`
    pub hufftable_time_c: Option<&'static [i32]>,
    /// `hufftableFreqC`
    pub hufftable_freq_c: Option<&'static [i32]>,
    /// `hufftableTimeL`
    pub hufftable_time_l: Option<&'static [u8]>,
    /// `hufftableFreqL`
    pub hufftable_freq_l: Option<&'static [u8]>,

    /// `hufftableLevelTimeC`
    pub hufftable_level_time_c: Option<&'static [i32]>,
    /// `hufftableBalanceTimeC`
    pub hufftable_balance_time_c: Option<&'static [i32]>,
    /// `hufftableLevelFreqC`
    pub hufftable_level_freq_c: Option<&'static [i32]>,
    /// `hufftableBalanceFreqC`
    pub hufftable_balance_freq_c: Option<&'static [i32]>,
    /// `hufftableLevelTimeL`
    pub hufftable_level_time_l: Option<&'static [u8]>,
    /// `hufftableBalanceTimeL`
    pub hufftable_balance_time_l: Option<&'static [u8]>,
    /// `hufftableLevelFreqL`
    pub hufftable_level_freq_l: Option<&'static [u8]>,
    /// `hufftableBalanceFreqL`
    pub hufftable_balance_freq_l: Option<&'static [u8]>,

    /// `hufftableNoiseTimeL`
    pub hufftable_noise_time_l: Option<&'static [u8]>,
    /// `hufftableNoiseTimeC`
    pub hufftable_noise_time_c: Option<&'static [i32]>,
    /// `hufftableNoiseFreqL`
    pub hufftable_noise_freq_l: Option<&'static [u8]>,
    /// `hufftableNoiseFreqC`
    pub hufftable_noise_freq_c: Option<&'static [i32]>,

    /// `hufftableNoiseLevelTimeL`
    pub hufftable_noise_level_time_l: Option<&'static [u8]>,
    /// `hufftableNoiseLevelTimeC`
    pub hufftable_noise_level_time_c: Option<&'static [i32]>,
    /// `hufftableNoiseBalanceTimeL`
    pub hufftable_noise_balance_time_l: Option<&'static [u8]>,
    /// `hufftableNoiseBalanceTimeC`
    pub hufftable_noise_balance_time_c: Option<&'static [i32]>,
    /// `hufftableNoiseLevelFreqL`
    pub hufftable_noise_level_freq_l: Option<&'static [u8]>,
    /// `hufftableNoiseLevelFreqC`
    pub hufftable_noise_level_freq_c: Option<&'static [i32]>,
    /// `hufftableNoiseBalanceFreqL`
    pub hufftable_noise_balance_freq_l: Option<&'static [u8]>,
    /// `hufftableNoiseBalanceFreqC`
    pub hufftable_noise_balance_freq_c: Option<&'static [i32]>,

    /// `hSbrBSGrid`: the index (within the element) of the channel whose envelope frame holds
    /// the bitstream grid (`None` for `NULL`)
    pub h_sbr_bs_grid: Option<usize>,

    /// `noHarmonics`
    pub no_harmonics: i32,
    /// `addHarmonicFlag`
    pub add_harmonic_flag: i32,
    /// `addHarmonic`
    pub add_harmonic: [u8; MAX_FREQ_COEFFS],

    // calculated helper vars
    /// `si_sbr_start_env_bits_balance`
    pub si_sbr_start_env_bits_balance: i32,
    /// `si_sbr_start_env_bits`
    pub si_sbr_start_env_bits: i32,
    /// `si_sbr_start_noise_bits_balance`
    pub si_sbr_start_noise_bits_balance: i32,
    /// `si_sbr_start_noise_bits`
    pub si_sbr_start_noise_bits: i32,

    /// `noOfEnvelopes`
    pub no_of_envelopes: i32,
    /// `noScfBands`
    pub no_scf_bands: [i32; MAX_ENVELOPES],
    /// `domain_vec`
    pub domain_vec: [i32; MAX_ENVELOPES],
    /// `domain_vec_noise`
    pub domain_vec_noise: [i32; MAX_ENVELOPES],
    /// `sbr_noise_levels`
    pub sbr_noise_levels: [i8; MAX_FREQ_COEFFS],
    /// `noOfnoisebands`
    pub no_ofnoisebands: i32,

    /// `balance`
    pub balance: i32,
    /// `init_sbr_amp_res`
    pub init_sbr_amp_res: AmpRes,
    /// `currentAmpResFF`
    pub current_amp_res_ff: AmpRes,
    /// `ton_HF`: tonality is scaled by 2^19/0.524288f (fract part of RELAXATION)
    pub ton_hf: [FixpDbl; SBR_GLOBAL_TONALITY_VALUES],
    /// `global_tonality`
    pub global_tonality: FixpDbl,

    // extended data
    /// `extended_data`
    pub extended_data: i32,
    /// `extension_size`
    pub extension_size: i32,
    /// `extension_id`
    pub extension_id: i32,
    /// `extended_data_buffer`
    pub extended_data_buffer: [u8; SBR_EXTENDED_DATA_MAX_CNT],

    /// `ldGrid`
    pub ld_grid: u8,
}

impl Default for SbrEnvData {
    fn default() -> Self {
        Self {
            sbr_xpos_ctrl: 0,
            freq_res_fixfix: [0; 2],
            f_res_trans_is_low: 0,
            sbr_invf_mode: 0,
            sbr_invf_mode_vec: [0; MAX_NUM_NOISE_VALUES],
            sbr_xpos_mode: 0,
            ienvelope: [[0; MAX_FREQ_COEFFS]; MAX_ENVELOPES],
            code_book_scf_lav_balance: 0,
            code_book_scf_lav: 0,
            hufftable_time_c: None,
            hufftable_freq_c: None,
            hufftable_time_l: None,
            hufftable_freq_l: None,
            hufftable_level_time_c: None,
            hufftable_balance_time_c: None,
            hufftable_level_freq_c: None,
            hufftable_balance_freq_c: None,
            hufftable_level_time_l: None,
            hufftable_balance_time_l: None,
            hufftable_level_freq_l: None,
            hufftable_balance_freq_l: None,
            hufftable_noise_time_l: None,
            hufftable_noise_time_c: None,
            hufftable_noise_freq_l: None,
            hufftable_noise_freq_c: None,
            hufftable_noise_level_time_l: None,
            hufftable_noise_level_time_c: None,
            hufftable_noise_balance_time_l: None,
            hufftable_noise_balance_time_c: None,
            hufftable_noise_level_freq_l: None,
            hufftable_noise_level_freq_c: None,
            hufftable_noise_balance_freq_l: None,
            hufftable_noise_balance_freq_c: None,
            h_sbr_bs_grid: None,
            no_harmonics: 0,
            add_harmonic_flag: 0,
            add_harmonic: [0; MAX_FREQ_COEFFS],
            si_sbr_start_env_bits_balance: 0,
            si_sbr_start_env_bits: 0,
            si_sbr_start_noise_bits_balance: 0,
            si_sbr_start_noise_bits: 0,
            no_of_envelopes: 0,
            no_scf_bands: [0; MAX_ENVELOPES],
            domain_vec: [0; MAX_ENVELOPES],
            domain_vec_noise: [0; MAX_ENVELOPES],
            sbr_noise_levels: [0; MAX_FREQ_COEFFS],
            no_ofnoisebands: 0,
            balance: 0,
            init_sbr_amp_res: 0,
            current_amp_res_ff: 0,
            ton_hf: [0; SBR_GLOBAL_TONALITY_VALUES],
            global_tonality: 0,
            extended_data: 0,
            extension_size: 0,
            extension_id: 0,
            extended_data_buffer: [0; SBR_EXTENDED_DATA_MAX_CNT],
            ld_grid: 0,
        }
    }
}

impl SbrEnvData {
    /// The byte image of the members that follow `addHarmonic` in `struct SBR_ENV_DATA`
    /// (`si_sbr_start_env_bits_balance` through `currentAmpResFF`, 144 bytes, in the layout of
    /// both reference targets: no padding, little endian). The missing harmonics detector
    /// copies `nSfb * sizeof(INT)` bytes starting at `addHarmonic` (see
    /// [`crate::mh_det`]).
    pub fn add_harmonic_tail(&self) -> [u8; ENV_DATA_TAIL_BYTES] {
        let mut t = [0u8; ENV_DATA_TAIL_BYTES];
        let mut pos = 0usize;
        let mut put = |t: &mut [u8; ENV_DATA_TAIL_BYTES], bytes: &[u8]| {
            t[pos..pos + bytes.len()].copy_from_slice(bytes);
            pos += bytes.len();
        };
        put(&mut t, &self.si_sbr_start_env_bits_balance.to_le_bytes());
        put(&mut t, &self.si_sbr_start_env_bits.to_le_bytes());
        put(&mut t, &self.si_sbr_start_noise_bits_balance.to_le_bytes());
        put(&mut t, &self.si_sbr_start_noise_bits.to_le_bytes());
        put(&mut t, &self.no_of_envelopes.to_le_bytes());
        for v in self.no_scf_bands {
            put(&mut t, &v.to_le_bytes());
        }
        for v in self.domain_vec {
            put(&mut t, &v.to_le_bytes());
        }
        for v in self.domain_vec_noise {
            put(&mut t, &v.to_le_bytes());
        }
        for v in self.sbr_noise_levels {
            put(&mut t, &[v as u8]);
        }
        put(&mut t, &self.no_ofnoisebands.to_le_bytes());
        put(&mut t, &self.balance.to_le_bytes());
        put(&mut t, &self.init_sbr_amp_res.to_le_bytes());
        put(&mut t, &self.current_amp_res_ff.to_le_bytes());
        t
    }
}

/// The envelope data of a channel with its bitstream grid (`sbrEnvData` and
/// `sbrEnvData->hSbrBSGrid`).
#[derive(Clone, Copy)]
pub struct EnvRef<'a> {
    /// `sbrEnvData`
    pub data: &'a SbrEnvData,
    /// `sbrEnvData->hSbrBSGrid`
    pub grid: &'a SbrGrid,
}

/// `SBR_ELEMENT_TYPE`
type SbrElementType = i32;
const SBR_ID_SCE: SbrElementType = 1;
const SBR_ID_CPE: SbrElementType = 2;

/// `FDKwriteBits(hBitStream, value, numberOfBits)` for the `INT` values of C (converted to
/// `UINT`).
#[inline]
fn wb(bs: &mut FdkBitStream, value: i32, n: i32) -> i32 {
    fdk_write_bits(bs, value as u32, n as u32) as i32
}

/// `FDKsbrEnc_WriteEnvSingleChannelElement(sbrHeaderData, hParametricStereo,
/// sbrBitstreamData, sbrEnvData, cmonData, sbrSyntaxFlags)`: writes pure SBR single channel
/// data element. Returns the number of bits written.
pub fn fdk_sbr_enc_write_env_single_channel_element(
    sbr_header_data: &SbrHeaderData,
    h_parametric_stereo: Option<&ParametricStereo>,
    sbr_bitstream_data: &SbrBitstreamData,
    sbr_env_data: Option<EnvRef>,
    cmon_data: &mut CommonData,
    sbr_syntax_flags: u32,
) -> i32 {
    let mut payload_bits = 0;

    cmon_data.sbr_hdr_bits = 0;
    cmon_data.sbr_data_bits = 0;

    // write pure sbr data
    if let Some(env) = sbr_env_data {
        // write header
        payload_bits += encode_sbr_header(sbr_header_data, sbr_bitstream_data, cmon_data);

        // write data
        payload_bits += encode_sbr_data(
            env,
            None,
            h_parametric_stereo,
            cmon_data,
            SBR_ID_SCE,
            0,
            sbr_syntax_flags,
        );
    }
    payload_bits
}

/// `FDKsbrEnc_WriteEnvChannelPairElement(sbrHeaderData, hParametricStereo, sbrBitstreamData,
/// sbrEnvDataLeft, sbrEnvDataRight, cmonData, sbrSyntaxFlags)`: writes pure SBR channel pair
/// data element. Returns the number of bits written.
pub fn fdk_sbr_enc_write_env_channel_pair_element(
    sbr_header_data: &SbrHeaderData,
    h_parametric_stereo: Option<&ParametricStereo>,
    sbr_bitstream_data: &SbrBitstreamData,
    sbr_env_data_left: Option<EnvRef>,
    sbr_env_data_right: Option<EnvRef>,
    cmon_data: &mut CommonData,
    sbr_syntax_flags: u32,
) -> i32 {
    let mut payload_bits = 0;
    cmon_data.sbr_hdr_bits = 0;
    cmon_data.sbr_data_bits = 0;

    // write pure sbr data
    if let (Some(left), Some(right)) = (sbr_env_data_left, sbr_env_data_right) {
        // write header
        payload_bits += encode_sbr_header(sbr_header_data, sbr_bitstream_data, cmon_data);

        // write data
        payload_bits += encode_sbr_data(
            left,
            Some(right),
            h_parametric_stereo,
            cmon_data,
            SBR_ID_CPE,
            sbr_header_data.coupling,
            sbr_syntax_flags,
        );
    }
    payload_bits
}

/// `FDKsbrEnc_CountSbrChannelPairElement(...)`: writes the channel pair element, pushes the
/// bitstream back and returns the number of bits.
pub fn fdk_sbr_enc_count_sbr_channel_pair_element(
    sbr_header_data: &SbrHeaderData,
    h_parametric_stereo: Option<&ParametricStereo>,
    sbr_bitstream_data: &SbrBitstreamData,
    sbr_env_data_left: Option<EnvRef>,
    sbr_env_data_right: Option<EnvRef>,
    cmon_data: &mut CommonData,
    sbr_syntax_flags: u32,
) -> i32 {
    let bit_pos = fdk_get_valid_bits(&mut cmon_data.sbr_bitbuf) as i32;

    let payload_bits = fdk_sbr_enc_write_env_channel_pair_element(
        sbr_header_data,
        h_parametric_stereo,
        sbr_bitstream_data,
        sbr_env_data_left,
        sbr_env_data_right,
        cmon_data,
        sbr_syntax_flags,
    );

    let n = (fdk_get_valid_bits(&mut cmon_data.sbr_bitbuf) as i32).wrapping_sub(bit_pos);
    fdk_push_back(&mut cmon_data.sbr_bitbuf, n as u32);

    payload_bits
}

/// `encodeSbrHeader(sbrHeaderData, sbrBitstreamData, cmonData)`: encodes SBR Header
/// information. Returns the number of bits written.
fn encode_sbr_header(
    sbr_header_data: &SbrHeaderData,
    sbr_bitstream_data: &SbrBitstreamData,
    cmon_data: &mut CommonData,
) -> i32 {
    let mut payload_bits = 0;

    if sbr_bitstream_data.header_active != 0 {
        payload_bits += wb(&mut cmon_data.sbr_bitbuf, 1, 1);
        payload_bits += encode_sbr_header_data(Some(sbr_header_data), &mut cmon_data.sbr_bitbuf);
    } else {
        payload_bits += wb(&mut cmon_data.sbr_bitbuf, 0, 1);
    }

    cmon_data.sbr_hdr_bits = payload_bits;

    payload_bits
}

/// `encodeSbrHeaderData(sbrHeaderData, hBitStream)`: writes sbr_header() bs_protocol_version
/// through bs_header_extra_2. Returns the number of bits written.
pub fn encode_sbr_header_data(
    sbr_header_data: Option<&SbrHeaderData>,
    h_bit_stream: &mut FdkBitStream,
) -> i32 {
    let mut payload_bits = 0;
    if let Some(h) = sbr_header_data {
        let bs = h_bit_stream;
        payload_bits += wb(bs, h.sbr_amp_res, SI_SBR_AMP_RES_BITS as i32);
        payload_bits += wb(bs, h.sbr_start_frequency, SI_SBR_START_FREQ_BITS as i32);
        payload_bits += wb(bs, h.sbr_stop_frequency, SI_SBR_STOP_FREQ_BITS as i32);
        payload_bits += wb(bs, h.sbr_xover_band, SI_SBR_XOVER_BAND_BITS as i32);

        payload_bits += wb(bs, 0, SI_SBR_RESERVED_BITS as i32);

        payload_bits += wb(bs, h.header_extra_1, SI_SBR_HEADER_EXTRA_1_BITS as i32);
        payload_bits += wb(bs, h.header_extra_2, SI_SBR_HEADER_EXTRA_2_BITS as i32);

        if h.header_extra_1 != 0 {
            payload_bits += wb(bs, h.freq_scale, SI_SBR_FREQ_SCALE_BITS as i32);
            payload_bits += wb(bs, h.alter_scale, SI_SBR_ALTER_SCALE_BITS as i32);
            payload_bits += wb(bs, h.sbr_noise_bands, SI_SBR_NOISE_BANDS_BITS as i32);
        } // sbrHeaderData->header_extra_1

        if h.header_extra_2 != 0 {
            payload_bits += wb(bs, h.sbr_limiter_bands, SI_SBR_LIMITER_BANDS_BITS as i32);
            payload_bits += wb(bs, h.sbr_limiter_gains, SI_SBR_LIMITER_GAINS_BITS as i32);
            payload_bits += wb(bs, h.sbr_interpol_freq, SI_SBR_INTERPOL_FREQ_BITS as i32);
            payload_bits += wb(bs, h.sbr_smoothing_length, SI_SBR_SMOOTHING_LENGTH_BITS as i32);
        } // sbrHeaderData->header_extra_2
    } // sbrHeaderData != NULL

    payload_bits
}

/// `encodeSbrData(sbrEnvDataLeft, sbrEnvDataRight, hParametricStereo, cmonData, sbrElem,
/// coupling, sbrSyntaxFlags)`: encodes sbr Data information. Returns the number of bits
/// written.
fn encode_sbr_data(
    sbr_env_data_left: EnvRef,
    sbr_env_data_right: Option<EnvRef>,
    h_parametric_stereo: Option<&ParametricStereo>,
    cmon_data: &mut CommonData,
    sbr_elem: SbrElementType,
    coupling: i32,
    sbr_syntax_flags: u32,
) -> i32 {
    let mut payload_bits = 0;

    match (sbr_elem, sbr_env_data_right) {
        (SBR_ID_SCE, _) => {
            payload_bits += encode_sbr_single_channel_element(
                sbr_env_data_left,
                &mut cmon_data.sbr_bitbuf,
                h_parametric_stereo,
                sbr_syntax_flags,
            );
        }
        (SBR_ID_CPE, Some(right)) => {
            payload_bits += encode_sbr_channel_pair_element(
                sbr_env_data_left,
                right,
                h_parametric_stereo,
                &mut cmon_data.sbr_bitbuf,
                coupling,
                sbr_syntax_flags,
            );
        }
        _ => {
            // we never should apply SBR to any other element type
            fdk_assert!(false);
        }
    }

    cmon_data.sbr_data_bits = payload_bits;

    payload_bits
}

/// `MODE_FREQ_TANS`
const MODE_FREQ_TANS: i32 = 1;
/// `LD_TRANSMISSION`
const LD_TRANSMISSION: i32 = MODE_FREQ_TANS;
/// `encodeFreqs(mode)`
fn encode_freqs(mode: i32) -> i32 {
    if mode & MODE_FREQ_TANS != 0 {
        1
    } else {
        0
    }
}

/// `encodeSbrSingleChannelElement(sbrEnvData, hBitStream, hParametricStereo,
/// sbrSyntaxFlags)`: encodes sbr SCE information. Returns the number of bits written.
fn encode_sbr_single_channel_element(
    sbr_env_data: EnvRef,
    h_bit_stream: &mut FdkBitStream,
    h_parametric_stereo: Option<&ParametricStereo>,
    sbr_syntax_flags: u32,
) -> i32 {
    let mut payload_bits = 0;
    let e = sbr_env_data.data;

    payload_bits += wb(h_bit_stream, 0, SI_SBR_DATA_EXTRA_BITS as i32); // no reserved bits

    if e.ld_grid != 0 {
        if sbr_env_data.grid.frame_class != FIXFIX_ONLY {
            // encode normal SbrGrid
            payload_bits += encode_sbr_grid(sbr_env_data, h_bit_stream);
        } else {
            // use FIXFIXonly frame Grid
            payload_bits += encode_low_delay_sbr_grid(
                sbr_env_data,
                h_bit_stream,
                encode_freqs(LD_TRANSMISSION),
                sbr_syntax_flags,
            );
        }
    } else {
        if sbr_syntax_flags & SBR_SYNTAX_SCALABLE != 0 {
            payload_bits += wb(h_bit_stream, 1, SI_SBR_COUPLING_BITS as i32);
        }
        payload_bits += encode_sbr_grid(sbr_env_data, h_bit_stream);
    }

    payload_bits += encode_sbr_dtdf(e, h_bit_stream);

    for i in 0..e.no_ofnoisebands.max(0) as usize {
        payload_bits += wb(h_bit_stream, invf_mode_at(e, i), SI_SBR_INVF_MODE_BITS as i32);
    }

    payload_bits += write_envelope_data(e, h_bit_stream, 0);
    payload_bits += write_noise_level_data(e, h_bit_stream, 0);

    payload_bits += write_synthetic_coding_data(e, h_bit_stream);

    payload_bits += encode_extended_data(h_parametric_stereo, h_bit_stream);

    payload_bits
}

/// `sbrEnvData->sbr_invf_mode_vec[i]`
///
/// Bounded: indices outside the array read 0.
fn invf_mode_at(e: &SbrEnvData, i: usize) -> i32 {
    e.sbr_invf_mode_vec.get(i).copied().unwrap_or(0)
}

/// `encodeSbrChannelPairElement(sbrEnvDataLeft, sbrEnvDataRight, hParametricStereo,
/// hBitStream, coupling, sbrSyntaxFlags)`: encodes sbr CPE information.
fn encode_sbr_channel_pair_element(
    left: EnvRef,
    right: EnvRef,
    h_parametric_stereo: Option<&ParametricStereo>,
    h_bit_stream: &mut FdkBitStream,
    coupling: i32,
    sbr_syntax_flags: u32,
) -> i32 {
    let mut payload_bits = 0;
    let (l, r) = (left.data, right.data);

    payload_bits += wb(h_bit_stream, 0, SI_SBR_DATA_EXTRA_BITS as i32); // no reserved bits

    payload_bits += wb(h_bit_stream, coupling, SI_SBR_COUPLING_BITS as i32);

    if coupling != 0 {
        if l.ld_grid != 0 {
            if left.grid.frame_class != FIXFIX_ONLY {
                // normal SbrGrid
                payload_bits += encode_sbr_grid(left, h_bit_stream);
            } else {
                // FIXFIXonly frame Grid
                payload_bits += encode_low_delay_sbr_grid(
                    left,
                    h_bit_stream,
                    encode_freqs(LD_TRANSMISSION),
                    sbr_syntax_flags,
                );
            }
        } else {
            payload_bits += encode_sbr_grid(left, h_bit_stream);
        }

        payload_bits += encode_sbr_dtdf(l, h_bit_stream);
        payload_bits += encode_sbr_dtdf(r, h_bit_stream);

        for i in 0..l.no_ofnoisebands.max(0) as usize {
            payload_bits += wb(h_bit_stream, invf_mode_at(l, i), SI_SBR_INVF_MODE_BITS as i32);
        }

        payload_bits += write_envelope_data(l, h_bit_stream, 1);
        payload_bits += write_noise_level_data(l, h_bit_stream, 1);
        payload_bits += write_envelope_data(r, h_bit_stream, 1);
        payload_bits += write_noise_level_data(r, h_bit_stream, 1);

        payload_bits += write_synthetic_coding_data(l, h_bit_stream);
        payload_bits += write_synthetic_coding_data(r, h_bit_stream);
    } else {
        // no coupling
        fdk_assert!(l.ld_grid == r.ld_grid);

        if l.ld_grid != 0 || r.ld_grid != 0 {
            // sbrEnvDataLeft (left channel)
            if left.grid.frame_class != FIXFIX_ONLY {
                // no FIXFIXonly Frame so we dont need encodeLowDelaySbrGrid
                // normal SbrGrid
                payload_bits += encode_sbr_grid(left, h_bit_stream);
            } else {
                // FIXFIXonly frame Grid
                payload_bits += encode_low_delay_sbr_grid(
                    left,
                    h_bit_stream,
                    encode_freqs(LD_TRANSMISSION),
                    sbr_syntax_flags,
                );
            }

            // sbrEnvDataRight (right channel)
            if right.grid.frame_class != FIXFIX_ONLY {
                // no FIXFIXonly Frame so we dont need encodeLowDelaySbrGrid
                // normal SbrGrid
                payload_bits += encode_sbr_grid(right, h_bit_stream);
            } else {
                // FIXFIXonly frame Grid
                payload_bits += encode_low_delay_sbr_grid(
                    right,
                    h_bit_stream,
                    encode_freqs(LD_TRANSMISSION),
                    sbr_syntax_flags,
                );
            }
        } else {
            payload_bits += encode_sbr_grid(left, h_bit_stream);
            payload_bits += encode_sbr_grid(right, h_bit_stream);
        }
        payload_bits += encode_sbr_dtdf(l, h_bit_stream);
        payload_bits += encode_sbr_dtdf(r, h_bit_stream);

        for i in 0..l.no_ofnoisebands.max(0) as usize {
            payload_bits += wb(h_bit_stream, invf_mode_at(l, i), SI_SBR_INVF_MODE_BITS as i32);
        }
        for i in 0..r.no_ofnoisebands.max(0) as usize {
            payload_bits += wb(h_bit_stream, invf_mode_at(r, i), SI_SBR_INVF_MODE_BITS as i32);
        }

        payload_bits += write_envelope_data(l, h_bit_stream, 0);
        payload_bits += write_envelope_data(r, h_bit_stream, 0);
        payload_bits += write_noise_level_data(l, h_bit_stream, 0);
        payload_bits += write_noise_level_data(r, h_bit_stream, 0);

        payload_bits += write_synthetic_coding_data(l, h_bit_stream);
        payload_bits += write_synthetic_coding_data(r, h_bit_stream);
    } // coupling

    payload_bits += encode_extended_data(h_parametric_stereo, h_bit_stream);

    payload_bits
}

/// `ceil_ln2(x)`
fn ceil_ln2(x: i32) -> i32 {
    let mut tmp: i32 = -1;
    loop {
        tmp += 1;
        // C: (1 << ++tmp) < x; x is at most 2 + 2 * MAX_NUM_REL here, so the loop ends long
        // before the shift could overflow.
        if 1i32.wrapping_shl(tmp as u32) >= x || tmp >= 31 {
            break;
        }
    }
    tmp
}

/// `encodeSbrGrid(sbrEnvData, hBitStream)`: writes the bits that describe the time/frequency
/// grouping of a frame. Returns the number of bits written.
fn encode_sbr_grid(sbr_env_data: EnvRef, h_bit_stream: &mut FdkBitStream) -> i32 {
    let mut payload_bits = 0;
    let e = sbr_env_data.data;
    let g = sbr_env_data.grid;
    let buffer_frame_start = g.buffer_frame_start;
    let number_time_slots = g.number_time_slots;

    if e.ld_grid != 0 {
        payload_bits += wb(h_bit_stream, g.frame_class, SBR_CLA_BITS_LD as i32);
    } else {
        payload_bits += wb(h_bit_stream, g.frame_class, SBR_CLA_BITS as i32);
    }

    match g.frame_class {
        FIXFIX_ONLY => {
            fdk_assert!(false /* Fatal error in encodeSbrGrid! */);
        }
        FIXFIX => {
            let temp = ceil_ln2(g.bs_num_env);
            payload_bits += wb(h_bit_stream, temp, SBR_ENV_BITS as i32);
            if e.ld_grid != 0 && g.bs_num_env == 1 {
                payload_bits += wb(h_bit_stream, e.current_amp_res_ff, SI_SBR_AMP_RES_BITS as i32);
            }
            payload_bits += wb(h_bit_stream, g.v_f[0], SBR_RES_BITS as i32);
        }
        FIXVAR | VARFIX => {
            let temp = if g.frame_class == FIXVAR {
                g.bs_abs_bord - (buffer_frame_start + number_time_slots)
            } else {
                g.bs_abs_bord - buffer_frame_start
            };

            payload_bits += wb(h_bit_stream, temp, SBR_ABS_BITS as i32);
            payload_bits += wb(h_bit_stream, g.n, SBR_NUM_BITS as i32);

            for i in 0..g.n.max(0) as usize {
                let temp = (rel_bord(&g.bs_rel_bord, i) - 2) >> 1;
                payload_bits += wb(h_bit_stream, temp, SBR_REL_BITS as i32);
            }

            let temp = ceil_ln2(g.n + 2);
            payload_bits += wb(h_bit_stream, g.p, temp);

            for i in 0..(g.n + 1).max(0) as usize {
                payload_bits += wb(h_bit_stream, rel_bord(&g.v_f, i), SBR_RES_BITS as i32);
            }
        }
        VARVAR => {
            let temp = g.bs_abs_bord_0 - buffer_frame_start;
            payload_bits += wb(h_bit_stream, temp, SBR_ABS_BITS as i32);
            let temp = g.bs_abs_bord_1 - (buffer_frame_start + number_time_slots);
            payload_bits += wb(h_bit_stream, temp, SBR_ABS_BITS as i32);

            payload_bits += wb(h_bit_stream, g.bs_num_rel_0, SBR_NUM_BITS as i32);
            payload_bits += wb(h_bit_stream, g.bs_num_rel_1, SBR_NUM_BITS as i32);

            for i in 0..g.bs_num_rel_0.max(0) as usize {
                let temp = (rel_bord(&g.bs_rel_bord_0, i) - 2) >> 1;
                payload_bits += wb(h_bit_stream, temp, SBR_REL_BITS as i32);
            }

            for i in 0..g.bs_num_rel_1.max(0) as usize {
                let temp = (rel_bord(&g.bs_rel_bord_1, i) - 2) >> 1;
                payload_bits += wb(h_bit_stream, temp, SBR_REL_BITS as i32);
            }

            let temp = ceil_ln2(g.bs_num_rel_0 + g.bs_num_rel_1 + 2);
            payload_bits += wb(h_bit_stream, g.p, temp);

            let temp = g.bs_num_rel_0 + g.bs_num_rel_1 + 1;

            for i in 0..temp.max(0) as usize {
                payload_bits += wb(h_bit_stream, rel_bord(&g.v_f_lr, i), SBR_RES_BITS as i32);
            }
        }
        _ => {}
    }

    payload_bits
}

/// An element of a grid array.
///
/// Bounded: indices outside the array read 0.
fn rel_bord(a: &[i32], i: usize) -> i32 {
    a.get(i).copied().unwrap_or(0)
}

/// `encodeLowDelaySbrGrid(sbrEnvData, hBitStream, transmitFreqs, sbrSyntaxFlags)`: writes the
/// FIXFIXonly grid. Returns the number of bits written.
fn encode_low_delay_sbr_grid(
    sbr_env_data: EnvRef,
    h_bit_stream: &mut FdkBitStream,
    transmit_freqs: i32,
    _sbr_syntax_flags: u32,
) -> i32 {
    let mut payload_bits = 0;
    let g = sbr_env_data.grid;

    // write FIXFIXonly Grid
    // write frameClass [1 bit] for FIXFIXonly Grid
    payload_bits += wb(h_bit_stream, 1, SBR_CLA_BITS_LD as i32);

    // absolute Borders are fix: 0,X,X,X,nTimeSlots; so we dont have to transmit them
    // only transmit the transient position!
    // with this info (b1) we can reconstruct the Frame on Decoder side :
    // border[0] = 0; border[1] = b1; border[2]=b1+2; border[3] = nrTimeSlots

    // use 3 or 4bits for transient border (border)
    if g.number_time_slots == 8 {
        payload_bits += wb(h_bit_stream, g.bs_abs_bord, 3);
    } else {
        payload_bits += wb(h_bit_stream, g.bs_abs_bord, 4);
    }

    if transmit_freqs != 0 {
        // write FreqRes grid
        for i in 0..g.bs_num_env.max(0) as usize {
            payload_bits += wb(h_bit_stream, rel_bord(&g.v_f, i), SBR_RES_BITS as i32);
        }
    }

    payload_bits
}

/// `encodeSbrDtdf(sbrEnvData, hBitStream)`: writes the bits that describe the direction of
/// the envelopes of a frame. Returns the number of bits written.
fn encode_sbr_dtdf(e: &SbrEnvData, h_bit_stream: &mut FdkBitStream) -> i32 {
    let mut payload_bits = 0;

    let no_of_noise_envelopes = if e.no_of_envelopes > 1 { 2 } else { 1 };

    for i in 0..e.no_of_envelopes.max(0) as usize {
        payload_bits += wb(h_bit_stream, rel_bord(&e.domain_vec, i), SBR_DIR_BITS as i32);
    }
    for i in 0..no_of_noise_envelopes {
        payload_bits += wb(h_bit_stream, e.domain_vec_noise[i], SBR_DIR_BITS as i32);
    }

    payload_bits
}

/// The Huffman code and length of `index` in a code table and a length table.
///
/// Deviation: the envelope coder limits the values to the codebook; where the reference has no
/// defined result, the port writes 0 bits.
fn huff(c: Option<&'static [i32]>, l: Option<&'static [u8]>, index: i32) -> (i32, i32) {
    let i = usize::try_from(index).ok();
    let code = i.and_then(|i| c.and_then(|c| c.get(i))).copied();
    let len = i.and_then(|i| l.and_then(|l| l.get(i))).copied();
    match (code, len) {
        (Some(code), Some(len)) => (code, len as i32),
        _ => (0, 0),
    }
}

/// `sbrEnvData->sbr_noise_levels[j]`
fn noise_level_at(e: &SbrEnvData, j: i32) -> i32 {
    usize::try_from(j).ok().and_then(|j| e.sbr_noise_levels.get(j)).copied().unwrap_or(0) as i32
}

/// `writeNoiseLevelData(sbrEnvData, hBitStream, coupling)`: writes the bits corresponding to
/// the noise-floor-level. Returns the number of bits written.
fn write_noise_level_data(e: &SbrEnvData, h_bit_stream: &mut FdkBitStream, coupling: i32) -> i32 {
    let mut payload_bits = 0;
    let n_noise_envelopes = if e.no_of_envelopes > 1 { 2 } else { 1 };
    let nb = e.no_ofnoisebands;

    for i in 0..n_noise_envelopes {
        match e.domain_vec_noise[i as usize] {
            FREQ => {
                if coupling != 0 && e.balance != 0 {
                    payload_bits += wb(
                        h_bit_stream,
                        noise_level_at(e, i * nb),
                        e.si_sbr_start_noise_bits_balance,
                    );
                } else {
                    payload_bits +=
                        wb(h_bit_stream, noise_level_at(e, i * nb), e.si_sbr_start_noise_bits);
                }

                for j in (1 + i * nb)..(nb * (1 + i)) {
                    let v = noise_level_at(e, j);
                    let (code, len) = if coupling != 0 {
                        if e.balance != 0 {
                            // coupling && balance
                            huff(
                                e.hufftable_noise_balance_freq_c,
                                e.hufftable_noise_balance_freq_l,
                                v + CODE_BOOK_SCF_LAV_BALANCE11,
                            )
                        } else {
                            // coupling && !balance
                            huff(
                                e.hufftable_noise_level_freq_c,
                                e.hufftable_noise_level_freq_l,
                                v + CODE_BOOK_SCF_LAV11,
                            )
                        }
                    } else {
                        // !coupling
                        huff(
                            e.hufftable_noise_freq_c,
                            e.hufftable_noise_freq_l,
                            v + CODE_BOOK_SCF_LAV11,
                        )
                    };
                    payload_bits += wb(h_bit_stream, code, len);
                }
            }
            TIME => {
                for j in (i * nb)..(nb * (1 + i)) {
                    let v = noise_level_at(e, j);
                    let (code, len) = if coupling != 0 {
                        if e.balance != 0 {
                            // coupling && balance
                            huff(
                                e.hufftable_noise_balance_time_c,
                                e.hufftable_noise_balance_time_l,
                                v + CODE_BOOK_SCF_LAV_BALANCE11,
                            )
                        } else {
                            // coupling && !balance
                            huff(
                                e.hufftable_noise_level_time_c,
                                e.hufftable_noise_level_time_l,
                                v + CODE_BOOK_SCF_LAV11,
                            )
                        }
                    } else {
                        // !coupling
                        huff(
                            e.hufftable_noise_level_time_c,
                            e.hufftable_noise_level_time_l,
                            v + CODE_BOOK_SCF_LAV11,
                        )
                    };
                    payload_bits += wb(h_bit_stream, code, len);
                }
            }
            _ => {}
        }
    }
    payload_bits
}

/// `writeEnvelopeData(sbrEnvData, hBitStream, coupling)`: writes the bits corresponding to
/// the envelope. Returns the number of bits written.
fn write_envelope_data(e: &SbrEnvData, h_bit_stream: &mut FdkBitStream, coupling: i32) -> i32 {
    let mut payload_bits = 0;

    for j in 0..e.no_of_envelopes.max(0) as usize {
        // loop over all envelopes
        let Some(env) = e.ienvelope.get(j) else {
            break;
        };
        let domain = rel_bord(&e.domain_vec, j);
        if domain == FREQ {
            if coupling != 0 && e.balance != 0 {
                payload_bits += wb(h_bit_stream, env[0], e.si_sbr_start_env_bits_balance);
            } else {
                payload_bits += wb(h_bit_stream, env[0], e.si_sbr_start_env_bits);
            }
        }

        let mut i = 1 - domain;
        while i < rel_bord(&e.no_scf_bands, j) {
            let delta = rel_bord(env, i.max(0) as usize);
            if coupling != 0 && e.balance != 0 {
                fdk_assert!(fixp_abs(delta) <= e.code_book_scf_lav_balance);
            } else {
                fdk_assert!(fixp_abs(delta) <= e.code_book_scf_lav);
            }
            let (code, len) = if coupling != 0 {
                if e.balance != 0 {
                    if domain != 0 {
                        // coupling && balance && TIME
                        huff(
                            e.hufftable_balance_time_c,
                            e.hufftable_balance_time_l,
                            delta + e.code_book_scf_lav_balance,
                        )
                    } else {
                        // coupling && balance && FREQ
                        huff(
                            e.hufftable_balance_freq_c,
                            e.hufftable_balance_freq_l,
                            delta + e.code_book_scf_lav_balance,
                        )
                    }
                } else if domain != 0 {
                    // coupling && !balance && TIME
                    huff(
                        e.hufftable_level_time_c,
                        e.hufftable_level_time_l,
                        delta + e.code_book_scf_lav,
                    )
                } else {
                    // coupling && !balance && FREQ
                    huff(
                        e.hufftable_level_freq_c,
                        e.hufftable_level_freq_l,
                        delta + e.code_book_scf_lav,
                    )
                }
            } else if domain != 0 {
                // !coupling && TIME
                huff(e.hufftable_time_c, e.hufftable_time_l, delta + e.code_book_scf_lav)
            } else {
                // !coupling && FREQ
                huff(e.hufftable_freq_c, e.hufftable_freq_l, delta + e.code_book_scf_lav)
            };
            payload_bits += wb(h_bit_stream, code, len);
            i += 1;
        }
    }
    payload_bits
}

/// `encodeExtendedData(hParametricStereo, hBitStream)`: writes the bits corresponding to the
/// extended data. Returns the number of bits written.
fn encode_extended_data(
    h_parametric_stereo: Option<&ParametricStereo>,
    h_bit_stream: &mut FdkBitStream,
) -> i32 {
    let mut payload_bits = 0;

    let ext_data_size = get_sbr_extended_data_size(h_parametric_stereo);

    if ext_data_size != 0 {
        let max_ext_size = (1 << SI_SBR_EXTENSION_SIZE_BITS) - 1;
        let mut written_no_bits = 0; // needed to byte align the extended data

        payload_bits += wb(h_bit_stream, 1, SI_SBR_EXTENDED_DATA_BITS as i32);
        fdk_assert!(ext_data_size <= SBR_EXTENDED_DATA_MAX_CNT as i32);

        if ext_data_size < max_ext_size {
            payload_bits += wb(h_bit_stream, ext_data_size, SI_SBR_EXTENSION_SIZE_BITS as i32);
        } else {
            payload_bits += wb(h_bit_stream, max_ext_size, SI_SBR_EXTENSION_SIZE_BITS as i32);
            payload_bits += wb(
                h_bit_stream,
                ext_data_size - max_ext_size,
                SI_SBR_EXTENSION_ESC_COUNT_BITS as i32,
            );
        }

        // parametric coding signalled here?
        if h_parametric_stereo.is_some() {
            written_no_bits +=
                wb(h_bit_stream, EXTENSION_ID_PS_CODING as i32, SI_SBR_EXTENSION_ID_BITS as i32);
            written_no_bits +=
                fdk_sbr_enc_ps_enc_write_ps_data(h_parametric_stereo, Some(h_bit_stream));
        }

        payload_bits += written_no_bits;

        // byte alignment
        written_no_bits %= 8;
        if written_no_bits != 0 {
            payload_bits += wb(h_bit_stream, 0, 8 - written_no_bits);
        }
    } else {
        payload_bits += wb(h_bit_stream, 0, SI_SBR_EXTENDED_DATA_BITS as i32);
    }

    payload_bits
}

/// `writeSyntheticCodingData(sbrEnvData, hBitStream)`: writes the bits corresponding to the
/// "synthetic-coding"-extension. Returns the number of bits written.
fn write_synthetic_coding_data(e: &SbrEnvData, h_bit_stream: &mut FdkBitStream) -> i32 {
    let mut payload_bits = 0;

    payload_bits += wb(h_bit_stream, e.add_harmonic_flag, 1);

    if e.add_harmonic_flag != 0 {
        for i in 0..e.no_harmonics.max(0) as usize {
            let v = e.add_harmonic.get(i).copied().unwrap_or(0) as i32;
            payload_bits += wb(h_bit_stream, v, 1);
        }
    }

    payload_bits
}

/// `getSbrExtendedDataSize(hParametricStereo)`: the number of bytes needed for encoding the
/// extended data (including extension id).
fn get_sbr_extended_data_size(h_parametric_stereo: Option<&ParametricStereo>) -> i32 {
    let mut ext_data_bits = 0;

    // add your new extended data counting methods here

    // no extended data

    if h_parametric_stereo.is_some() {
        // PS extended data
        ext_data_bits += SI_SBR_EXTENSION_ID_BITS as i32;
        ext_data_bits += fdk_sbr_enc_ps_enc_write_ps_data(h_parametric_stereo, None);
    }

    (ext_data_bits + 7) >> 3
}

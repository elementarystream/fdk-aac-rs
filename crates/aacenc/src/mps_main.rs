// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/mps_main.h, libAACenc/src/mps_main.cpp.

//! Port of `mps_main.h`: the MPEG Surround (ELDv2, 212) integration of the AAC encoder.
//!
//! `MPS_ENCODER` keeps buffer descriptors (`inBufDesc`, `outBufDesc`) whose arrays are members
//! of the structure itself; `FDK_MpegsEnc_Init()` sets them up and `FDK_MpegsEnc_Process()`
//! fills in the buffer addresses. The port keeps the arrays and the number of buffers of each
//! descriptor, and builds the descriptors from them when it calls the MPEG Surround encoder
//! (the same addresses and values as in C; the encoder is boxed).

use crate::aacenc::AacencExtPayload;
use core::ffi::c_void;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_get_valid_bits, fdk_init_bit_stream, fdk_write_bits, FdkBitStream,
    BS_WRITER,
};
use fdk_aac_rs_fdk::common_fix::{f_max, f_min};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_sacenc::sacenc_lib::*;

mod tables;
use tables::MPS_CONFIG_TAB;

/// `mpsConfigTab` as rows of (`audio_object_type`, `channel_mode`, `sbr_ratio`,
/// `sampling_rate`, `bitrate_min`, `bitrate_max`) (for the table tests).
#[doc(hidden)]
pub fn mps_config_tab() -> Vec<[u32; 6]> {
    MPS_CONFIG_TAB
        .iter()
        .map(|t| {
            [
                t.audio_object_type as u32,
                t.channel_mode as u32,
                t.sbr_ratio,
                t.sampling_rate,
                t.bitrate_min,
                t.bitrate_max,
            ]
        })
        .collect()
}

/// `MPS_ENCODER_ERROR`
pub type MpsEncoderError = i32;
/// No error happened. All fine.
pub const MPS_ENCODER_OK: MpsEncoderError = 0x0000;
/// Handle passed to function call was invalid.
pub const MPS_ENCODER_INVALID_HANDLE: MpsEncoderError = 0x0020;
/// Memory allocation failed.
pub const MPS_ENCODER_MEMORY_ERROR: MpsEncoderError = 0x0021;
/// General initialization error.
pub const MPS_ENCODER_INIT_ERROR: MpsEncoderError = 0x0040;
/// The encoding process was interrupted by an unexpected error.
pub const MPS_ENCODER_ENCODE_ERROR: MpsEncoderError = 0x0060;

/// Size of `MPS_ENCODER::sacOutBuffer`: worst case memory consumption for ELDv2: 768 bytes =>
/// 6144 bits (Core + SBR + MPS)
pub const SAC_OUT_BUFFER_SIZE: usize = 1024;

/// `struct MPS_ENCODER` (`HANDLE_MPS_ENCODER` is `&mut MpsEncoder`).
#[derive(Debug)]
pub struct MpsEncoder {
    pub h_sac_encoder: HandleMp4SpaceEncoder,

    pub audio_object_type: AudioObjectType,

    /// `inBufDesc.numBufs` (the other members point to the `p_in_buffer*` arrays)
    pub in_buf_desc_num_bufs: u32,
    /// `outBufDesc.numBufs` (the other members point to the `p_out_buffer*` arrays)
    pub out_buf_desc_num_bufs: u32,
    pub inargs: SacencInArgs,
    pub outargs: SacencOutArgs,

    pub p_in_buffer: [*mut c_void; 1],
    pub p_in_buffer_size: [u32; 1],
    pub p_in_buffer_el_size: [u32; 1],
    pub p_in_buffer_type: [u32; 1],

    pub p_out_buffer: [*mut c_void; 2],
    pub p_out_buffer_size: [u32; 2],
    pub p_out_buffer_el_size: [u32; 2],
    pub p_out_buffer_type: [u32; 2],

    pub sac_out_buffer: [u8; SAC_OUT_BUFFER_SIZE],
}

// SAFETY: the raw pointers are buffer addresses that are only used during a call.
unsafe impl Send for MpsEncoder {}

/// `struct MPS_CONFIG_TAB`
#[derive(Clone, Copy, Debug)]
pub(crate) struct MpsConfigTab {
    pub audio_object_type: AudioObjectType,
    pub channel_mode: ChannelMode,
    pub sbr_ratio: u32,
    pub sampling_rate: u32,
    pub bitrate_min: u32,
    pub bitrate_max: u32,
}

/// `FDK_MpegsEnc_Open(phMpsEnc)`
pub fn fdk_mpegs_enc_open(ph_mps_enc: Option<&mut Option<Box<MpsEncoder>>>) -> MpsEncoderError {
    let Some(ph) = ph_mps_enc else {
        return MPS_ENCODER_INVALID_HANDLE;
    };

    let mut h = Box::new(MpsEncoder {
        h_sac_encoder: None,
        audio_object_type: AOT_NULL_OBJECT,
        in_buf_desc_num_bufs: 0,
        out_buf_desc_num_bufs: 0,
        inargs: SacencInArgs::default(),
        outargs: SacencOutArgs::default(),
        p_in_buffer: [core::ptr::null_mut(); 1],
        p_in_buffer_size: [0; 1],
        p_in_buffer_el_size: [0; 1],
        p_in_buffer_type: [0; 1],
        p_out_buffer: [core::ptr::null_mut(); 2],
        p_out_buffer_size: [0; 2],
        p_out_buffer_el_size: [0; 2],
        p_out_buffer_type: [0; 2],
        sac_out_buffer: [0; SAC_OUT_BUFFER_SIZE],
    });

    if SACENC_OK != fdk_sacenc_open(Some(&mut h.h_sac_encoder)) {
        let mut hh = Some(h);
        fdk_mpegs_enc_close(Some(&mut hh));
        return MPS_ENCODER_MEMORY_ERROR;
    }

    // Return mps encoder instance
    *ph = Some(h);

    MPS_ENCODER_OK
}

/// `FDK_MpegsEnc_Close(phMpsEnc)`
pub fn fdk_mpegs_enc_close(ph_mps_enc: Option<&mut Option<Box<MpsEncoder>>>) -> MpsEncoderError {
    let Some(ph) = ph_mps_enc else {
        return MPS_ENCODER_INVALID_HANDLE;
    };

    if let Some(h) = ph.as_deref_mut() {
        fdk_sacenc_close(Some(&mut h.h_sac_encoder));
    }
    *ph = None;
    MPS_ENCODER_OK
}

/// `FDK_MpegsEnc_Init(hMpsEnc, audioObjectType, samplingrate, bitrate, sbrRatio, framelength,
/// inputBufferSizePerChannel, coreCoderDelay)`
#[allow(clippy::too_many_arguments)]
pub fn fdk_mpegs_enc_init(
    h_mps_enc: Option<&mut MpsEncoder>,
    audio_object_type: AudioObjectType,
    samplingrate: u32,
    bitrate: u32,
    sbr_ratio: u32,
    framelength: u32,
    input_buffer_size_per_channel: u32,
    core_coder_delay: u32,
) -> MpsEncoderError {
    let mut error = MPS_ENCODER_OK;
    let fs_low: u32 = 27713; // low MPS sampling frequencies
    let fs_high: u32 = 55426; // high MPS sampling frequencies

    let Some(h_mps_enc) = h_mps_enc else {
        return MPS_ENCODER_INVALID_HANDLE;
    };

    // Combine MPS with SBR only if the number of QMF band fits together.
    match sbr_ratio {
        1 => {
            // downsampled sbr - 32 QMF bands required
            if samplingrate >= fs_low {
                return MPS_ENCODER_INIT_ERROR;
            }
        }
        2 => {
            // dualrate - 64 QMF bands required
            if !((samplingrate >= fs_low) && (samplingrate < fs_high)) {
                return MPS_ENCODER_INIT_ERROR;
            }
        }
        _ => {} // time interface - no samplingrate restriction
    }

    // 32  QMF-Bands  ( fs < 27713 )
    // 64  QMF-Bands  ( 27713 >= fs <= 55426 )
    // 128 QMF-Bands  ( fs > 55426 )
    let n_qmf_bands_ld: u32 = if samplingrate < fs_low {
        5
    } else if samplingrate > fs_high {
        7
    } else {
        6
    };
    let n_time_slots = framelength >> n_qmf_bands_ld;

    // check if number of qmf bands is usable for given framelength
    if framelength != (n_time_slots << n_qmf_bands_ld) {
        return MPS_ENCODER_INIT_ERROR;
    }

    // is given bitrate intended to be supported
    if bitrate as i32
        != fdk_mpegs_enc_get_closest_bit_rate(
            audio_object_type,
            MODE_212,
            samplingrate,
            sbr_ratio,
            bitrate,
        )
    {
        return MPS_ENCODER_INIT_ERROR;
    }

    // init SAC library
    match audio_object_type {
        AOT_ER_AAC_ELD => {
            let no_inter_frame_coding: u32 = 0;
            let h = h_mps_enc.h_sac_encoder.as_deref_mut();
            let mut ok = true;
            let mut set = |p: SpaceEncParam, v: u32, h: Option<&mut Mp4SpaceEncoder>| {
                if ok && SACENC_OK != fdk_sacenc_set_param(h, p, v) {
                    ok = false;
                }
            };
            // The || chain of C stops at the first failure.
            let mut h = h;
            set(SACENC_LOWDELAY, if no_inter_frame_coding == 1 { 1 } else { 2 }, h.as_deref_mut());
            set(SACENC_ENC_MODE, SACENC_212 as u32, h.as_deref_mut());
            set(SACENC_SAMPLERATE, samplingrate, h.as_deref_mut());
            set(SACENC_FRAME_TIME_SLOTS, n_time_slots, h.as_deref_mut());
            set(SACENC_PARAM_BANDS, SACENC_BANDS_15 as u32, h.as_deref_mut());
            set(SACENC_TIME_DOM_DMX, 2, h.as_deref_mut());
            set(SACENC_COARSE_QUANT, 0, h.as_deref_mut());
            set(SACENC_QUANT_MODE, SACENC_QUANTMODE_FINE as u32, h.as_deref_mut());
            set(SACENC_TIME_ALIGNMENT, 0, h.as_deref_mut());
            set(SACENC_INDEPENDENCY_FACTOR, 20, h);
            if !ok {
                return MPS_ENCODER_INIT_ERROR;
            }
        }
        _ => return MPS_ENCODER_INIT_ERROR,
    }

    if SACENC_OK != fdk_sacenc_init(h_mps_enc.h_sac_encoder.as_deref_mut(), core_coder_delay as i32)
    {
        error = MPS_ENCODER_INIT_ERROR;
    }

    h_mps_enc.audio_object_type = audio_object_type;

    h_mps_enc.in_buf_desc_num_bufs = 1;
    h_mps_enc.out_buf_desc_num_bufs = 2;

    h_mps_enc.p_in_buffer[0] = core::ptr::null_mut();
    h_mps_enc.p_in_buffer_size[0] = 0;
    h_mps_enc.p_in_buffer_el_size[0] = core::mem::size_of::<IntPcm>() as u32;
    h_mps_enc.p_in_buffer_type[0] = FDK_BUF_TYPE_INPUT | FDK_BUF_TYPE_PCM_DATA;

    h_mps_enc.p_out_buffer[0] = core::ptr::null_mut();
    h_mps_enc.p_out_buffer_size[0] = 0;
    h_mps_enc.p_out_buffer_el_size[0] = core::mem::size_of::<IntPcm>() as u32;
    h_mps_enc.p_out_buffer_type[0] = FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_PCM_DATA;

    h_mps_enc.p_out_buffer[1] = core::ptr::null_mut();
    h_mps_enc.p_out_buffer_size[1] = 0;
    h_mps_enc.p_out_buffer_el_size[1] = 1;
    h_mps_enc.p_out_buffer_type[1] = FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA;

    h_mps_enc.inargs.is_input_interleaved = 0;
    h_mps_enc.inargs.input_buffer_size_per_channel = input_buffer_size_per_channel;

    error
}

/// `FDK_MpegsEnc_Process(hMpsEnc, pAudioSamples, nAudioSamples, pMpsExtPayload)`: encodes the
/// spatial parameters of one frame and writes the mono downmix over the input.
///
/// # Safety
/// `p_audio_samples` must be valid for reads of the two input channels
/// (`inputBufferSizePerChannel` apart, `nAudioSamples / 2` samples each) and for writes of the
/// downmix (see [`fdk_sacenc_encode`]).
pub unsafe fn fdk_mpegs_enc_process(
    h_mps_enc: Option<&mut MpsEncoder>,
    p_audio_samples: *mut IntPcm,
    n_audio_samples: i32,
    p_mps_ext_payload: &mut AacencExtPayload,
) -> MpsEncoderError {
    let Some(h_mps_enc) = h_mps_enc else {
        return MPS_ENCODER_INVALID_HANDLE;
    };
    let mut sac_header_flag = 1;
    let mut sac_out_buffer_offset: usize = 0;

    // In case of eld the ssc is explicit and doesn't need to be inband
    if h_mps_enc.audio_object_type == AOT_ER_AAC_ELD {
        sac_header_flag = 0;
    }

    // 4 bits nibble after extension type
    h_mps_enc.sac_out_buffer[0] = if sac_header_flag == 0 { 0x3 } else { 0x7 };
    sac_out_buffer_offset += 1;

    if sac_header_flag != 0 {
        let off = sac_out_buffer_offset;
        sac_out_buffer_offset += fdk_mpegs_enc_write_frame_header(h_mps_enc, off) as usize;
    }

    // Register input and output buffer.
    h_mps_enc.p_in_buffer[0] = p_audio_samples as *mut c_void;
    h_mps_enc.inargs.n_input_samples = n_audio_samples;

    h_mps_enc.p_out_buffer[0] = p_audio_samples as *mut c_void;
    // sizeof(INT_PCM) * nAudioSamples / 2 (size_t arithmetic, truncated to UINT)
    h_mps_enc.p_out_buffer_size[0] = ((core::mem::size_of::<IntPcm>() as u64)
        .wrapping_mul(n_audio_samples as i64 as u64)
        / 2) as u32;

    let off = sac_out_buffer_offset.min(SAC_OUT_BUFFER_SIZE);
    h_mps_enc.p_out_buffer[1] = h_mps_enc.sac_out_buffer[off..].as_mut_ptr() as *mut c_void;
    h_mps_enc.p_out_buffer_size[1] = (SAC_OUT_BUFFER_SIZE - off) as u32;

    let in_buf_desc = FdkBufDescr {
        pp_base: h_mps_enc.p_in_buffer.as_mut_ptr(),
        p_buf_size: h_mps_enc.p_in_buffer_size.as_mut_ptr(),
        p_ele_size: h_mps_enc.p_in_buffer_el_size.as_mut_ptr(),
        p_buf_type: h_mps_enc.p_in_buffer_type.as_mut_ptr(),
        num_bufs: h_mps_enc.in_buf_desc_num_bufs,
    };
    let out_buf_desc = FdkBufDescr {
        pp_base: h_mps_enc.p_out_buffer.as_mut_ptr(),
        p_buf_size: h_mps_enc.p_out_buffer_size.as_mut_ptr(),
        p_ele_size: h_mps_enc.p_out_buffer_el_size.as_mut_ptr(),
        p_buf_type: h_mps_enc.p_out_buffer_type.as_mut_ptr(),
        num_bufs: h_mps_enc.out_buf_desc_num_bufs,
    };

    // encode SAC frame
    // SAFETY: the descriptors point to the arrays of `h_mps_enc` (2 output entries), the PCM
    // buffer is the caller's guarantee, and `sac_out_buffer` does not overlap it.
    let r = unsafe {
        fdk_sacenc_encode(
            h_mps_enc.h_sac_encoder.as_deref_mut(),
            &in_buf_desc,
            &out_buf_desc,
            &h_mps_enc.inargs,
            &mut h_mps_enc.outargs,
        )
    };
    if SACENC_OK != r {
        return MPS_ENCODER_ENCODE_ERROR;
    }

    // export MPS payload
    p_mps_ext_payload.p_data = h_mps_enc.sac_out_buffer.as_ptr();
    p_mps_ext_payload.data_size = (h_mps_enc.outargs.n_output_bits as u32)
        .wrapping_add(8u32.wrapping_mul(sac_out_buffer_offset as u32).wrapping_sub(8));
    p_mps_ext_payload.data_type = EXT_LDSAC_DATA;
    p_mps_ext_payload.associated_ch_element = -1;

    MPS_ENCODER_OK
}

/// `FDK_MpegsEnc_WriteSpatialSpecificConfig(hMpsEnc, hBs)`: writes the SpatialSpecificConfig
/// (`h_bs == None`: only returns its length in bits).
pub fn fdk_mpegs_enc_write_spatial_specific_config(
    h_mps_enc: Option<&MpsEncoder>,
    h_bs: Option<&mut FdkBitStream>,
) -> i32 {
    let mut ssc_bits = 0;

    if let Some(h_mps_enc) = h_mps_enc {
        let mut info = Mp4SpaceEncInfo::default();
        fdk_sacenc_get_info(h_mps_enc.h_sac_encoder.as_deref(), Some(&mut info));
        // C dereferences pSscBuf unchecked; the encoder always has one.
        let Some(ssc) = info.p_ssc_buf else { return 0 };
        let byte = |i: usize| ssc.p_ssc.get(i).copied().unwrap_or(0);

        if let Some(h_bs) = h_bs {
            let mut written_bits = 0;
            let mut i = 0usize;
            while (i as i32) < (ssc.n_ssc_size_bits >> 3) {
                fdk_write_bits(h_bs, byte(i) as u32, 8);
                written_bits += 8;
                i += 1;
            }
            // The low bits of the last byte, as C writes them.
            fdk_write_bits(h_bs, byte(i) as u32, (ssc.n_ssc_size_bits - written_bits) as u32);
        } // hBS

        ssc_bits = ssc.n_ssc_size_bits;
    } // valid hMpsEnc

    ssc_bits
}

/// `FDK_MpegsEnc_WriteFrameHeader(hMpsEnc, pOutputBuffer, outputBufferSize)` with
/// `pOutputBuffer = &sacOutBuffer[offset]`: returns the number of bytes written.
fn fdk_mpegs_enc_write_frame_header(h_mps_enc: &mut MpsEncoder, offset: usize) -> i32 {
    let sac_time_align_flag = 0;

    // Initialize variables
    let mut bs = FdkBitStream::default();
    let offset = offset.min(SAC_OUT_BUFFER_SIZE);
    let output_buffer_size = (SAC_OUT_BUFFER_SIZE - offset) as u32;
    // The writer works on a copy of `sac_out_buffer[offset..]`, written back at the end.
    let window = BitMem::from_slice(&h_mps_enc.sac_out_buffer[offset..]);
    fdk_init_bit_stream(&mut bs, &window, output_buffer_size, 0, BS_WRITER);

    // Calculate SSC length information
    let cnt =
        ((fdk_mpegs_enc_write_spatial_specific_config(Some(h_mps_enc), None) + 7) >> 3) as u32;

    // Write SSC
    fdk_write_bits(&mut bs, sac_time_align_flag, 1);

    if cnt < 127 {
        fdk_write_bits(&mut bs, cnt, 7);
    } else {
        fdk_write_bits(&mut bs, 127, 7);
        fdk_write_bits(&mut bs, cnt.wrapping_sub(127), 16);
    }

    let align_anchor = fdk_get_valid_bits(&mut bs);
    fdk_mpegs_enc_write_spatial_specific_config(Some(h_mps_enc), Some(&mut bs));
    fdk_byte_align(&mut bs, align_anchor); // bsFillBits

    if sac_time_align_flag != 0 {
        fdk_assert!(true); // time alignment not supported
    }

    let num_bits = fdk_get_valid_bits(&mut bs) as i32;
    window.read(0, &mut h_mps_enc.sac_out_buffer[offset..]);

    (num_bits + 7) >> 3
}

/// `FDK_MpegsEnc_GetClosestBitRate(audioObjectType, channelMode, samplingrate, sbrRatio,
/// bitrate)`: the closest supported bitrate, -1 for an unsupported configuration.
pub fn fdk_mpegs_enc_get_closest_bit_rate(
    audio_object_type: AudioObjectType,
    channel_mode: ChannelMode,
    samplingrate: u32,
    sbr_ratio: u32,
    bitrate: u32,
) -> i32 {
    let mut target_bitrate: i32 = -1;

    for t in MPS_CONFIG_TAB.iter() {
        if (t.audio_object_type == audio_object_type)
            && (t.channel_mode == channel_mode)
            && (t.sbr_ratio == sbr_ratio)
            && (t.sampling_rate == samplingrate)
        {
            target_bitrate = f_min(f_max(bitrate, t.bitrate_min), t.bitrate_max) as i32;
        }
    }

    target_bitrate
}

/// `FDK_MpegsEnc_GetDelay(hMpsEnc)`
pub fn fdk_mpegs_enc_get_delay(h_mps_enc: Option<&MpsEncoder>) -> i32 {
    let mut delay = 0;

    if let Some(h) = h_mps_enc {
        let mut info = Mp4SpaceEncInfo::default();
        fdk_sacenc_get_info(h.h_sac_encoder.as_deref(), Some(&mut info));
        delay = info.n_codec_delay;
    }

    delay
}

/// `FDK_MpegsEnc_GetDecDelay(hMpsEnc)`
pub fn fdk_mpegs_enc_get_dec_delay(h_mps_enc: Option<&MpsEncoder>) -> i32 {
    let mut delay = 0;

    if let Some(h) = h_mps_enc {
        let mut info = Mp4SpaceEncInfo::default();
        fdk_sacenc_get_info(h.h_sac_encoder.as_deref(), Some(&mut info));
        delay = info.n_decoder_delay;
    }

    delay
}

/// `FDK_MpegsEnc_GetLibInfo(info)`
pub fn fdk_mpegs_enc_get_lib_info(info: Option<&mut [LibInfo]>) -> MpsEncoderError {
    match info {
        None => MPS_ENCODER_INVALID_HANDLE,
        Some(info) => {
            if SACENC_OK != fdk_sacenc_get_lib_info(Some(info)) {
                MPS_ENCODER_INIT_ERROR
            } else {
                MPS_ENCODER_OK
            }
        }
    }
}

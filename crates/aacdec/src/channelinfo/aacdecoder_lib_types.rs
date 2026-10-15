// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/include/aacdecoder_lib.h (AAC_DECODER_ERROR).

//! `AAC_DECODER_ERROR` of `aacdecoder_lib.h`, which `channelinfo.h` and every module of the
//! decoder core need. It is defined here; `crate::aacdecoder_lib` re-exports it
//! (`pub use crate::channelinfo::aacdecoder_lib_types::*;`).

/// `AAC_DECODER_ERROR`: AAC decoder error codes.
pub type AacDecoderError = i32;

/// `AAC_DEC_OK`: no error occurred. Output buffer is valid and error free.
pub const AAC_DEC_OK: AacDecoderError = 0x0000;
/// `AAC_DEC_OUT_OF_MEMORY`: heap returned NULL pointer. Output buffer is invalid.
pub const AAC_DEC_OUT_OF_MEMORY: AacDecoderError = 0x0002;
/// `AAC_DEC_UNKNOWN`: error condition is of unknown reason, or from a another module. Output
/// buffer is invalid.
pub const AAC_DEC_UNKNOWN: AacDecoderError = 0x0005;

/* Synchronization errors. Output buffer is invalid. */
/// `aac_dec_sync_error_start`
pub const aac_dec_sync_error_start: AacDecoderError = 0x1000;
/// `AAC_DEC_TRANSPORT_SYNC_ERROR`: the transport decoder had synchronization problems.
pub const AAC_DEC_TRANSPORT_SYNC_ERROR: AacDecoderError = 0x1001;
/// `AAC_DEC_NOT_ENOUGH_BITS`: the input buffer ran out of bits.
pub const AAC_DEC_NOT_ENOUGH_BITS: AacDecoderError = 0x1002;
/// `aac_dec_sync_error_end`
pub const aac_dec_sync_error_end: AacDecoderError = 0x1FFF;

/* Initialization errors. Output buffer is invalid. */
/// `aac_dec_init_error_start`
pub const aac_dec_init_error_start: AacDecoderError = 0x2000;
/// `AAC_DEC_INVALID_HANDLE`: the handle passed to the function call was invalid (NULL).
pub const AAC_DEC_INVALID_HANDLE: AacDecoderError = 0x2001;
/// `AAC_DEC_UNSUPPORTED_AOT`: the AOT found in the configuration is not supported.
pub const AAC_DEC_UNSUPPORTED_AOT: AacDecoderError = 0x2002;
/// `AAC_DEC_UNSUPPORTED_FORMAT`: the bitstream format is not supported.
pub const AAC_DEC_UNSUPPORTED_FORMAT: AacDecoderError = 0x2003;
/// `AAC_DEC_UNSUPPORTED_ER_FORMAT`: the error resilience tool format is not supported.
pub const AAC_DEC_UNSUPPORTED_ER_FORMAT: AacDecoderError = 0x2004;
/// `AAC_DEC_UNSUPPORTED_EPCONFIG`: the error protection format is not supported.
pub const AAC_DEC_UNSUPPORTED_EPCONFIG: AacDecoderError = 0x2005;
/// `AAC_DEC_UNSUPPORTED_MULTILAYER`: more than one layer for AAC scalable is not supported.
pub const AAC_DEC_UNSUPPORTED_MULTILAYER: AacDecoderError = 0x2006;
/// `AAC_DEC_UNSUPPORTED_CHANNELCONFIG`: the channel configuration (either number or
/// arrangement) is not supported.
pub const AAC_DEC_UNSUPPORTED_CHANNELCONFIG: AacDecoderError = 0x2007;
/// `AAC_DEC_UNSUPPORTED_SAMPLINGRATE`: the sample rate specified in the configuration is not
/// supported.
pub const AAC_DEC_UNSUPPORTED_SAMPLINGRATE: AacDecoderError = 0x2008;
/// `AAC_DEC_INVALID_SBR_CONFIG`: the SBR configuration is not supported.
pub const AAC_DEC_INVALID_SBR_CONFIG: AacDecoderError = 0x2009;
/// `AAC_DEC_SET_PARAM_FAIL`: the parameter could not be set.
pub const AAC_DEC_SET_PARAM_FAIL: AacDecoderError = 0x200A;
/// `AAC_DEC_NEED_TO_RESTART`: the decoder needs to be restarted, since the required
/// configuration change cannot be performed.
pub const AAC_DEC_NEED_TO_RESTART: AacDecoderError = 0x200B;
/// `AAC_DEC_OUTPUT_BUFFER_TOO_SMALL`: the provided output buffer is too small.
pub const AAC_DEC_OUTPUT_BUFFER_TOO_SMALL: AacDecoderError = 0x200C;
/// `aac_dec_init_error_end`
pub const aac_dec_init_error_end: AacDecoderError = 0x2FFF;

/* Decode errors. Output buffer is valid but concealed. */
/// `aac_dec_decode_error_start`
pub const aac_dec_decode_error_start: AacDecoderError = 0x4000;
/// `AAC_DEC_TRANSPORT_ERROR`: the transport decoder encountered an unexpected error.
pub const AAC_DEC_TRANSPORT_ERROR: AacDecoderError = 0x4001;
/// `AAC_DEC_PARSE_ERROR`: error while parsing the bitstream.
pub const AAC_DEC_PARSE_ERROR: AacDecoderError = 0x4002;
/// `AAC_DEC_UNSUPPORTED_EXTENSION_PAYLOAD`: error while parsing the extension payload of the
/// bitstream. The extension payload type found is not supported.
pub const AAC_DEC_UNSUPPORTED_EXTENSION_PAYLOAD: AacDecoderError = 0x4003;
/// `AAC_DEC_DECODE_FRAME_ERROR`: the parsed bitstream value is out of range.
pub const AAC_DEC_DECODE_FRAME_ERROR: AacDecoderError = 0x4004;
/// `AAC_DEC_CRC_ERROR`: the embedded CRC did not match.
pub const AAC_DEC_CRC_ERROR: AacDecoderError = 0x4005;
/// `AAC_DEC_INVALID_CODE_BOOK`: an invalid codebook was signaled.
pub const AAC_DEC_INVALID_CODE_BOOK: AacDecoderError = 0x4006;
/// `AAC_DEC_UNSUPPORTED_PREDICTION`: predictor found, but not supported in the AAC Low
/// Complexity profile.
pub const AAC_DEC_UNSUPPORTED_PREDICTION: AacDecoderError = 0x4007;
/// `AAC_DEC_UNSUPPORTED_CCE`: a CCE element was found which is not supported.
pub const AAC_DEC_UNSUPPORTED_CCE: AacDecoderError = 0x4008;
/// `AAC_DEC_UNSUPPORTED_LFE`: a LFE element was found which is not supported.
pub const AAC_DEC_UNSUPPORTED_LFE: AacDecoderError = 0x4009;
/// `AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA`: gain control data found but not supported.
pub const AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA: AacDecoderError = 0x400A;
/// `AAC_DEC_UNSUPPORTED_SBA`: SBA found, but currently not supported in the BSAC profile.
pub const AAC_DEC_UNSUPPORTED_SBA: AacDecoderError = 0x400B;
/// `AAC_DEC_TNS_READ_ERROR`: error while reading TNS data.
pub const AAC_DEC_TNS_READ_ERROR: AacDecoderError = 0x400C;
/// `AAC_DEC_RVLC_ERROR`: error while decoding error resilient data.
pub const AAC_DEC_RVLC_ERROR: AacDecoderError = 0x400D;
/// `aac_dec_decode_error_end`
pub const aac_dec_decode_error_end: AacDecoderError = 0x4FFF;

/* Ancillary data errors. Output buffer is valid. */
/// `aac_dec_anc_data_error_start`
pub const aac_dec_anc_data_error_start: AacDecoderError = 0x8000;
/// `AAC_DEC_ANC_DATA_ERROR`: non severe error concerning the ancillary data handling.
pub const AAC_DEC_ANC_DATA_ERROR: AacDecoderError = 0x8001;
/// `AAC_DEC_TOO_SMALL_ANC_BUFFER`: the registered ancillary data buffer is too small to
/// receive the parsed data.
pub const AAC_DEC_TOO_SMALL_ANC_BUFFER: AacDecoderError = 0x8002;
/// `AAC_DEC_TOO_MANY_ANC_ELEMENTS`: more than the allowed number of ancillary data elements
/// should be written to buffer.
pub const AAC_DEC_TOO_MANY_ANC_ELEMENTS: AacDecoderError = 0x8003;
/// `aac_dec_anc_data_error_end`
pub const aac_dec_anc_data_error_end: AacDecoderError = 0x8FFF;

/// `IS_INIT_ERROR(err)`: identifies initialization errors. Output buffer is invalid.
#[inline]
pub const fn is_init_error(err: AacDecoderError) -> i32 {
    ((err >= aac_dec_init_error_start) && (err <= aac_dec_init_error_end)) as i32
}

/// `IS_DECODE_ERROR(err)`: identifies decode errors. Output buffer is valid but concealed.
#[inline]
pub const fn is_decode_error(err: AacDecoderError) -> i32 {
    ((err >= aac_dec_decode_error_start) && (err <= aac_dec_decode_error_end)) as i32
}

/// `IS_OUTPUT_VALID(err)`: identifies if the audio output buffer contains valid samples after
/// calling `aacDecoder_DecodeFrame()`.
#[inline]
pub const fn is_output_valid(err: AacDecoderError) -> i32 {
    ((err == AAC_DEC_OK) || is_decode_error(err) != 0) as i32
}

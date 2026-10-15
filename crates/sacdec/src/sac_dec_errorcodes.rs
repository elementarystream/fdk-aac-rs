// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/include/sac_dec_errorcodes.h.

//! Port of `sac_dec_errorcodes.h`: the error codes of the MPEG Surround decoder.

/// `SACDEC_ERROR`
pub type SacdecError = i32;

/// `__mps_error_start`
pub const __MPS_ERROR_START: SacdecError = -1000;
/// `MPS_OK`
pub const MPS_OK: SacdecError = 0;

// generic/init errors
/// `MPS_NOTOK`
pub const MPS_NOTOK: SacdecError = __MPS_ERROR_START;
/// `MPS_OUTOFMEMORY`
pub const MPS_OUTOFMEMORY: SacdecError = -999;
/// `MPS_INVALID_HANDLE`
pub const MPS_INVALID_HANDLE: SacdecError = -998;
/// `MPS_INVALID_PARAMETER`: SetParam not successful
pub const MPS_INVALID_PARAMETER: SacdecError = -997;
/// `MPS_UNSUPPORTED_HRTFMODEL`: SetHRTFModel() not successful
pub const MPS_UNSUPPORTED_HRTFMODEL: SacdecError = -996;
/// `MPS_UNSUPPORTED_HRTFFREQ`: SetHRTFModel() not successful
pub const MPS_UNSUPPORTED_HRTFFREQ: SacdecError = -995;
/// `MPS_UNSUPPORTED_UPMIX_TYPE`: CheckLevelTreeUpmixType()
pub const MPS_UNSUPPORTED_UPMIX_TYPE: SacdecError = -994;
/// `MPS_UNSUPPORTED_FORMAT`: various functions; unknown aot or no_channels in filterbank
pub const MPS_UNSUPPORTED_FORMAT: SacdecError = -993;
/// `MPS_OUTPUT_BUFFER_TOO_SMALL`: size of provided output time buffer is too small
pub const MPS_OUTPUT_BUFFER_TOO_SMALL: SacdecError = -992;

// ssc errors
/// `MPS_INVALID_PARAMETERBANDS`: unsupported numParameterBands in SpatialDecDecodeHeader()
pub const MPS_INVALID_PARAMETERBANDS: SacdecError = -991;
/// `MPS_INVALID_TREECONFIG`
pub const MPS_INVALID_TREECONFIG: SacdecError = -990;
/// `MPS_INVALID_HRTFSET`: SpatialDecDecodeHeader()
pub const MPS_INVALID_HRTFSET: SacdecError = -989;
/// `MPS_INVALID_TTT`: SpatialDecDecodeHeader()
pub const MPS_INVALID_TTT: SacdecError = -988;
/// `MPS_INVALID_BOXIDX`: ecDataDec()
pub const MPS_INVALID_BOXIDX: SacdecError = -987;
/// `MPS_INVALID_SETIDX`: ecDataDec()
pub const MPS_INVALID_SETIDX: SacdecError = -986;
/// `MPS_INVALID_QUANTMODE`: SpatialDecParseSpecificConfig()
pub const MPS_INVALID_QUANTMODE: SacdecError = -985;
/// `MPS_UNEQUAL_SSC`: FDK_SpatialDecCompareSpatialSpecificConfigHeader()
pub const MPS_UNEQUAL_SSC: SacdecError = -984;
/// `MPS_UNSUPPORTED_CONFIG`: number of core channels; 3DStereoInversion
pub const MPS_UNSUPPORTED_CONFIG: SacdecError = -983;

// parse errors
/// `MPS_PARSE_ERROR`
pub const MPS_PARSE_ERROR: SacdecError = -982;
/// `MPS_INVALID_TEMPSHAPE`: SpatialDecParseFrameData()
pub const MPS_INVALID_TEMPSHAPE: SacdecError = -981;

// render errors
/// `MPS_WRONG_PARAMETERSETS`
pub const MPS_WRONG_PARAMETERSETS: SacdecError = -980;
/// `MPS_WRONG_PARAMETERBANDS`: decodeAndMapFrameSmg()
pub const MPS_WRONG_PARAMETERBANDS: SacdecError = -979;
/// `MPS_WRONG_TREECONFIG`
pub const MPS_WRONG_TREECONFIG: SacdecError = -978;
/// `MPS_WRONG_BLINDCONFIG`
pub const MPS_WRONG_BLINDCONFIG: SacdecError = -977;
/// `MPS_WRONG_OTT`
pub const MPS_WRONG_OTT: SacdecError = -976;
/// `MPS_WRONG_QUANTMODE`
pub const MPS_WRONG_QUANTMODE: SacdecError = -975;
/// `MPS_RESDEC_ERROR`
pub const MPS_RESDEC_ERROR: SacdecError = -974;
/// `MPS_APPLY_M2_ERROR`: error in applyM2x() selection
pub const MPS_APPLY_M2_ERROR: SacdecError = -973;
/// `__mps_error_end`
pub const __MPS_ERROR_END: SacdecError = -972;

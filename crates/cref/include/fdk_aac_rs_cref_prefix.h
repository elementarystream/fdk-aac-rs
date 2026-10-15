/* Force-included (-include) into every translation unit of the C++ reference and of the shims
   built by crates/cref/build.rs. Renames the extern "C" API of the reference to cref_*, so that
   the reference can be linked into one binary together with the C ABI crate, which exports the
   original names (PORTING.md, "Differential tests"). All other symbols of the reference are
   C++ symbols and cannot clash with Rust symbols. */
#ifndef FDK_AAC_RS_CREF_PREFIX_H
#define FDK_AAC_RS_CREF_PREFIX_H
#define aacDecoder_AncDataGet cref_aacDecoder_AncDataGet
#define aacDecoder_AncDataInit cref_aacDecoder_AncDataInit
#define aacDecoder_Close cref_aacDecoder_Close
#define aacDecoder_ConfigRaw cref_aacDecoder_ConfigRaw
#define aacDecoder_DecodeFrame cref_aacDecoder_DecodeFrame
#define aacDecoder_Fill cref_aacDecoder_Fill
#define aacDecoder_GetFreeBytes cref_aacDecoder_GetFreeBytes
#define aacDecoder_GetLibInfo cref_aacDecoder_GetLibInfo
#define aacDecoder_GetStreamInfo cref_aacDecoder_GetStreamInfo
#define aacDecoder_Open cref_aacDecoder_Open
#define aacDecoder_RawISOBMFFData cref_aacDecoder_RawISOBMFFData
#define aacDecoder_SetParam cref_aacDecoder_SetParam
#define aacEncClose cref_aacEncClose
#define aacEncEncode cref_aacEncEncode
#define aacEncGetLibInfo cref_aacEncGetLibInfo
#define aacEncInfo cref_aacEncInfo
#define aacEncOpen cref_aacEncOpen
#define aacEncoder_GetParam cref_aacEncoder_GetParam
#define aacEncoder_SetParam cref_aacEncoder_SetParam
#endif

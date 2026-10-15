// Differential tests of fdk_aac_rs_sbrenc::env_bit against the C++ reference.
//
// FDKsbrEnc_InitSbrBitstream() and FDKsbrEnc_AssembleSbrBitstream() (fill bits, SBR CRC and DRM
// CRC) are tested with every syntax flag around the payload writers in bit_sbr.rs, and with
// realistic payloads in the whole-encoder test (sbr_encoder.rs).

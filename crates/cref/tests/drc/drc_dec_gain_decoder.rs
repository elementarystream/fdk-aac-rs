// Differential tests of fdk_aac_rs_drc::drc_dec_gain_decoder against the C++ reference.
//
// The functions are tested through the public API in drc_dec_lib.rs, which compares the
// complete state of the gain decoder (DRC_GAIN_DECODER, byte by byte) and the audio after every
// call (time domain and STFT domain processing, concealment, channel gains, downmix).

// Differential tests of fdk_aac_rs_sbrenc::ps_encode against the C++ reference.
//
// FDKsbrEnc_InitPSEncode() and FDKsbrEnc_PSEncode() are tested through the parametric stereo
// encoder in ps_main.rs (random and realistic stereo signals, all band and envelope counts, the
// complete PS_ENCODE compared after every frame) and in the whole-encoder test.

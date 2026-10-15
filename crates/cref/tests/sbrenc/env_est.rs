// Differential tests of fdk_aac_rs_sbrenc::env_est against the C++ reference.
//
// The envelope extraction (energies, tonality, noise floor quantization and coupling, envelope
// calculation, stereo mode decision and coding) is internal to the encoder; it is compared
// through the whole-encoder test (sbr_encoder.rs), which dumps every SBR_ENV_DATA, SBR_EXTRACT_ENVELOPE
// and the dynamic RAM after every frame, on corpus clips and on noise, silence and full-scale
// signals, for every tuning (mono, LR, coupling and switched LR/coupling stereo, ELD).

// Differential tests of fdk_aac_rs_sbrdec::sbr_ram against the C++ reference.
//
// sbr_ram.cpp only allocates the decoder memory (GetRam_*/FreeRam_*); the port allocates with
// Box and Vec. The layouts of the structures that the tests pass as memory are checked by
// e2e.rs (layouts()) and by the tests that pass them.

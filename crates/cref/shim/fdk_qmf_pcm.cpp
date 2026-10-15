// Shims for the differential tests of fdk_aac_rs_fdk::qmf_pcm.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_qmf_pcm (see build.rs).

#include "cref_common.h"
//
// The functions of qmf_pcm.h are reached through the shims of fdk_qmf.cpp.

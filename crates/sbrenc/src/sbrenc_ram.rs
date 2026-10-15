// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/sbrenc_ram.h, libSBRenc/src/sbrenc_ram.cpp.

//! Port of `sbrenc_ram.h` and `sbrenc_ram.cpp`: the memory layout of the SBR encoder.
//!
//! The static memory areas (`C_ALLOC_MEM*`) become owned allocations of the structures that
//! use them. The dynamic RAM (`Ram_SbrDynamic_RAM`, shared by the QMF data, the energy buffers
//! and the PS hybrid data) is one `Vec<FixpDbl>` of [`SBR_ENC_DYN_RAM_DBL`] values owned by the
//! encoder; the `GetRam_Sbr_env*Buffer()` functions return offsets into it (in `FIXP_DBL`
//! values, not bytes).

use fdk_aac_rs_fdk::common_fix::FixpDbl;

use crate::ps_const::MAX_HYBRID_BANDS;
use crate::sbr_def::MAX_NUM_CHANNELS;

/// `ENV_R_BUFF_BYTE / sizeof(FIXP_DBL)`
pub const ENV_R_BUFF_DBL: usize = 32 * MAX_HYBRID_BANDS;
/// `ENV_I_BUFF_BYTE / sizeof(FIXP_DBL)`
pub const ENV_I_BUFF_DBL: usize = 32 * MAX_HYBRID_BANDS;
/// `Y_BUF_CH_BYTE / sizeof(FIXP_DBL)`
pub const Y_BUF_CH_DBL: usize = 2 * ((32 - 32 / 2) * MAX_HYBRID_BANDS);
/// `Y_2_BUF_BYTE / sizeof(FIXP_DBL)`
pub const Y_2_BUF_DBL: usize = Y_BUF_CH_DBL;
/// `BUF_NRG_SIZE / sizeof(FIXP_DBL)`
pub const BUF_NRG_DBL: usize = MAX_NUM_CHANNELS * Y_2_BUF_DBL;
/// `BUF_QMF_SIZE / sizeof(FIXP_DBL)`
pub const BUF_QMF_DBL: usize = ENV_R_BUFF_DBL + ENV_I_BUFF_DBL;
/// `SBR_ENC_DYN_RAM_SIZE / sizeof(FIXP_DBL)`: the size of the dynamic RAM in values.
pub const SBR_ENC_DYN_RAM_DBL: usize = BUF_QMF_DBL + BUF_NRG_DBL;
/// `OFFSET_QMF / sizeof(FIXP_DBL)`
pub const OFFSET_QMF_DBL: usize = 0;
/// `OFFSET_NRG / sizeof(FIXP_DBL)`
pub const OFFSET_NRG_DBL: usize = OFFSET_QMF_DBL + BUF_QMF_DBL;

/// The size of one channel of `Ram_Sbr_envYBuffer` (the static, persistent half of the energy
/// buffer) in values (`32 / 2 * 64`).
pub const ENV_Y_BUFFER_STATIC_DBL: usize = 32 / 2 * 64;

/// The number of values of `Ram_Sbr_QmfStatesAnalysis` per channel.
pub const QMF_STATES_ANALYSIS_SIZE: usize = 640;
/// The number of values of `Ram_PsQmfStatesSynthesis`.
pub const PS_QMF_STATES_SYNTHESIS_SIZE: usize = 640 / 2;

/// `GetRam_SbrDynamic_RAM()`: a new (cleared) dynamic RAM.
pub fn get_ram_sbr_dynamic_ram() -> Vec<FixpDbl> {
    vec![0; SBR_ENC_DYN_RAM_DBL]
}

/// `GetRam_Sbr_envYBuffer(n, dynamic_RAM)`: the offset of the dynamic energy buffer of channel
/// `n`.
#[inline]
pub fn get_ram_sbr_env_y_buffer(n: usize) -> usize {
    OFFSET_NRG_DBL + n * Y_2_BUF_DBL
}

/// `GetRam_Sbr_envRBuffer(n, dynamic_RAM)`: the offset of the real QMF buffer of channel `n`
/// (the SBR encoder always uses `n = 0`, PS uses both).
#[inline]
pub fn get_ram_sbr_env_r_buffer(n: usize) -> usize {
    OFFSET_QMF_DBL + n * (ENV_R_BUFF_DBL + ENV_I_BUFF_DBL)
}

/// `GetRam_Sbr_envIBuffer(n, dynamic_RAM)`: the offset of the imaginary QMF buffer of channel
/// `n`.
#[inline]
pub fn get_ram_sbr_env_i_buffer(n: usize) -> usize {
    OFFSET_QMF_DBL + ENV_R_BUFF_DBL + n * (ENV_R_BUFF_DBL + ENV_I_BUFF_DBL)
}

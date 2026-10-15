// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_rom.h, libSACdec/src/sac_rom.cpp.

//! Port of `sac_rom.h` and `sac_rom.cpp`: the tables of the MPEG Surround decoder.
//!
//! The tables are generated from the C++ sources by `sac_rom/gen_tables.py` with
//! `scripts/dump_table.py` ([`tables`]); the arm64 and x86_64 builds give the same values.
//! `FIXP_CFG` is `FIXP_DBL` in both reference builds.

pub mod tables;
pub use tables::*;

pub use crate::sac_bitdec::TreeProperties;

use fdk_aac_rs_fdk::common_fix::*;

/// `FIXP_CFG`
pub type FixpCfg = FixpDbl;

/// `FX_CFG2FX_DBL(x)`
#[inline(always)]
pub const fn fx_cfg2fx_dbl(x: FixpCfg) -> FixpDbl {
    x
}

/// `FX_CFG2FX_SGL(x)`
#[inline(always)]
pub const fn fx_cfg2fx_sgl(x: FixpCfg) -> FixpSgl {
    fx_dbl2fx_sgl(x)
}

/// `SCALE_INV_ICC`
pub const SCALE_INV_ICC: i32 = 2;
/// `G_dd_SCALE`
pub const G_DD_SCALE: i32 = 2;
/// `QCC_SCALE`
pub const QCC_SCALE: i32 = 1;

/// `SF_CLD_C1C2`
pub const SF_CLD_C1C2: i32 = 9;

/// `IPD_SCALE`
pub const IPD_SCALE: i32 = 5;
/// `PI__IPD`: `FL2FXCONST_DBL(3.1415926535897932f / (float)(1 << IPD_SCALE))` (a `float`
/// quotient)
pub const PI__IPD: FixpDbl = fl2fxconst_dbl((3.1415926535897932f32 / 32.0f32) as f64);
/// `PIx2__IPD`: `FL2FXCONST_DBL(3.1415926535897932f / (float)(1 << (IPD_SCALE - 1)))`, PI*2
/// for better precision in `SpatialDecApplyPhase()`
pub const PIX2__IPD: FixpDbl = fl2fxconst_dbl((3.1415926535897932f32 / 16.0f32) as f64);

/// `SpatialDequantGetCLDValues(index, cu, cl)`: the CLD values of a table index
/// (`(cu, cl)`).
///
/// Deviation: an index outside of `0..=30` (the library passes only indices of this range)
/// reads 0.
pub fn spatial_dequant_get_cld_values(index: i32) -> (FixpDbl, FixpDbl) {
    let at = |i: i32| DEQUANT_CLD_C_L.get(i as usize).copied().unwrap_or(0);
    (fx_cfg2fx_dbl(at(index)), fx_cfg2fx_dbl(at(31 - 1 - index)))
}

/// `SpatialDequantGetCLD2Values(idx, x)`
///
/// Deviation: an index outside of `0..=30` reads 0 (see
/// [`spatial_dequant_get_cld_values`]).
pub fn spatial_dequant_get_cld2_values(idx: i32) -> FixpDbl {
    fx_cfg2fx_dbl(DEQUANT_CLD__FDK.get(idx as usize).copied().unwrap_or(0))
}

/// `SacGetHybridSubbands(qmfSubbands)`
#[inline]
pub const fn sac_get_hybrid_subbands(qmf_subbands: i32) -> i32 {
    qmf_subbands - crate::sac_dec::MAX_QMF_BANDS_TO_HYBRID + 10
}

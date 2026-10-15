// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aac_rom.h, libAACdec/src/aac_rom.cpp.

//! Port of `aac_rom.h`: definition of constant tables.
//!
//! The tables are generated from the C++ sources by `aac_rom/gen_tables.py` ([`tables`]): the
//! tables of scalars with `scripts/dump_table.py`, the pointer tables (`sfbOffsetTables`,
//! `AACcodeBookDescriptionTable`, `AACcodeBookDescriptionSCL`, `aHuffTable`, `aQuantTable`) from
//! a program compiled against `aac_rom.cpp` that prints which table each pointer refers to.
//! Pointers are `&'static` slices, empty for `NULL`. The preprocessed `aac_rom.cpp` of the
//! `aarch64` and `x86` builds are identical apart from `FDK_ASSERT`.
//!
//! - The file-local (`static`) `sfb_*` tables are public here, like the other tables.
//! - `aStateConstant2State[]` holds pointers to the HCR state functions of `aacdec_hcrs.cpp`; the
//!   port represents a `STATEFUNC` by its state constant ([`crate::aacdec_hcr_types::StateFunc`]),
//!   so the table maps every constant to itself ([`A_STATE_CONSTANT_2_STATE`]).
//! - `pow2_div24minus1`, `offsetTab`, `channelMappingTablePassthrough` and
//!   `channelMappingTableWAV` are declared in `aac_rom.h` but defined nowhere in the library;
//!   they are not ported.

#[rustfmt::skip]
pub mod tables;

pub use tables::*;

use fdk_aac_rs_fdk::common_fix::{
    fx_dbl2fx_pcm, fx_pcm2fx_dbl, FixpDbl, DFRACT_BITS, MAXVAL_DBL, MINVAL_DBL,
};
use fdk_aac_rs_fdk::machine_type::IntPcm;

use crate::aacdec_hcr_types::StateFunc;

/// `PCM_AAC`
pub type PcmAac = i32;
/// `PCM_DEC`
pub type PcmDec = FixpDbl;
/// `MAXVAL_PCM_DEC`
pub const MAXVAL_PCM_DEC: PcmDec = MAXVAL_DBL;
/// `MINVAL_PCM_DEC`
pub const MINVAL_PCM_DEC: PcmDec = MINVAL_DBL;
/// `PCM_DEC_BITS`
pub const PCM_DEC_BITS: i32 = DFRACT_BITS;

/// `FIXP_DBL2PCM_DEC(x)`
#[inline(always)]
pub const fn fixp_dbl2pcm_dec(x: FixpDbl) -> PcmDec {
    x
}

/// `PCM_DEC2FIXP_DBL(x)`
#[inline(always)]
pub const fn pcm_dec2fixp_dbl(x: PcmDec) -> FixpDbl {
    x
}

/// `PCM_DEC2FX_PCM(x)`: `FX_DBL2FX_PCM(x)`
#[inline(always)]
pub const fn pcm_dec2fx_pcm(x: PcmDec) -> IntPcm {
    fx_dbl2fx_pcm(x)
}

/// `FX_PCM2PCM_DEC(x)`: `FX_PCM2FX_DBL(x)`
#[inline(always)]
pub const fn fx_pcm2pcm_dec(x: IntPcm) -> PcmDec {
    fx_pcm2fx_dbl(x)
}

/// `AACDEC_MAX_CH_CONF`
pub const AACDEC_MAX_CH_CONF: i32 = 14;
/// `AACDEC_CH_ELEMENTS_TAB_SIZE`: size of element tables
pub const AACDEC_CH_ELEMENTS_TAB_SIZE: i32 = 7;

/// `AAC_NF_NO_RANDOM_VAL`: size of random number array for noise floor
pub const AAC_NF_NO_RANDOM_VAL: i32 = 512;

/// `INV_QUANT_TABLESIZE`
pub const INV_QUANT_TABLESIZE: i32 = 256;

/// `NUM_LD_COEF_512`
pub const NUM_LD_COEF_512: i32 = 1536;
/// `NUM_LD_COEF_480`
pub const NUM_LD_COEF_480: i32 = 1440;
/* Window table partition exponents. */
/// `WTS0`
pub const WTS0: i32 = 1;
/// `WTS1`
pub const WTS1: i32 = 0;
/// `WTS2`
pub const WTS2: i32 = -2;

/// `SFB_INFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SfbInfo {
    /// `sfbOffsetLong` (empty for `NULL`)
    pub sfb_offset_long: &'static [i16],
    /// `sfbOffsetShort` (empty for `NULL`)
    pub sfb_offset_short: &'static [i16],
    /// `numberOfSfbLong`
    pub number_of_sfb_long: u8,
    /// `numberOfSfbShort`
    pub number_of_sfb_short: u8,
}

/* Huffman tables */
/// `HuffmanBits`
pub const HuffmanBits: i32 = 2;
/// `HuffmanEntries`
pub const HuffmanEntries: i32 = 1 << HuffmanBits;

/// `CodeBookDescription`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeBookDescription {
    /// `CodeBook` (`const USHORT (*)[HuffmanEntries]`; empty for `NULL`)
    pub code_book: &'static [[u16; HuffmanEntries as usize]],
    /// `Dimension`
    pub dimension: u8,
    /// `numBits`
    pub num_bits: u8,
    /// `Offset`
    pub offset: u8,
}

/// `FIXP_TCC`
pub type FixpTcc = FixpDbl;

/// `SF_FNA_COEFFS`: compile-time prescaler for MDST-filter coefficients.
pub const SF_FNA_COEFFS: i32 = 1;

/// `FIXP_FILT` (`CPLX_PRED_FILTER_16BIT` is not defined in the reference build)
pub type FixpFilt = FixpDbl;

/// `aStateConstant2State[]`: the HCR state function of each state constant (`NULL`,
/// `Hcr_State_BODY_ONLY`, `Hcr_State_BODY_SIGN__BODY`, `Hcr_State_BODY_SIGN__SIGN`,
/// `Hcr_State_BODY_SIGN_ESC__BODY`, `Hcr_State_BODY_SIGN_ESC__SIGN`,
/// `Hcr_State_BODY_SIGN_ESC__ESC_PREFIX`, `Hcr_State_BODY_SIGN_ESC__ESC_WORD`). A state
/// function is represented by its constant ([`StateFunc`]), so entry `i` is `i`; the
/// differential test checks that entry `i` of the C table is the `i`-th of these functions.
pub static A_STATE_CONSTANT_2_STATE: [StateFunc; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

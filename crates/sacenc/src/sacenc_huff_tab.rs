// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_huff_tab.h, libSACenc/src/sacenc_huff_tab.cpp.

//! Port of `sacenc_huff_tab.h` and `sacenc_huff_tab.cpp`: the Huffman tables of the MPEG
//! Surround encoder.
//!
//! The tables are dumped from the reference as their `ULONG` words in memory order
//! (`scripts/dump_table.py libSACenc/src/sacenc_huff_tab.cpp <table> UINT --hex`) and
//! reinterpreted as the C structures, which consist of `HUFF_ENTRY` words only (`#[repr(C)]`,
//! the sizes are checked at compile time).

/// `HUFF_ENTRY`: the Huffman code in bits 31..8, its length in bits 7..0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct HuffEntry {
    pub packed: u32,
}

/// `HUFF_VALUE(a)`
#[inline(always)]
pub const fn huff_value(a: HuffEntry) -> u32 {
    (a.packed >> 8) & 0x00FF_FFFF
}

/// `HUFF_LENGTH(a)`
#[inline(always)]
pub const fn huff_length(a: HuffEntry) -> u32 {
    a.packed & 0xFF
}

/// `LAV1_2D` .. `LAV12_2D`: a two-dimensional table of `N` x `N` entries and the escape code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Lav2d<const N: usize> {
    pub entry: [[HuffEntry; N]; N],
    pub escape: HuffEntry,
}

/// `LAV1_2D`
pub type Lav1_2d = Lav2d<2>;

mod tables;
pub use tables::*;
/// `LAV3_2D`
pub type Lav3_2d = Lav2d<4>;
/// `LAV5_2D`
pub type Lav5_2d = Lav2d<6>;
/// `LAV6_2D`
pub type Lav6_2d = Lav2d<7>;
/// `LAV7_2D`
pub type Lav7_2d = Lav2d<8>;
/// `LAV9_2D`
pub type Lav9_2d = Lav2d<10>;
/// `LAV12_2D`
pub type Lav12_2d = Lav2d<13>;

/// `HUFF_CLD_TAB_2D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffCldTab2d {
    pub lav3: Lav3_2d,
    pub lav5: Lav5_2d,
    pub lav7: Lav7_2d,
    pub lav9: Lav9_2d,
}

/// `HUFF_ICC_TAB_2D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffIccTab2d {
    pub lav1: Lav1_2d,
    pub lav3: Lav3_2d,
    pub lav5: Lav5_2d,
    pub lav7: Lav7_2d,
}

/// `HUFF_CLD_TABLE`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffCldTable {
    pub h1d: [[HuffEntry; 31]; 2],
    pub h2d: [[HuffCldTab2d; 2]; 2],
}

/// `HUFF_ICC_TABLE`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffIccTable {
    pub h1d: [[HuffEntry; 8]; 2],
    pub h2d: [[HuffIccTab2d; 2]; 2],
}

/// `HUFF_PT0_TABLE`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffPt0Table {
    pub cld: [HuffEntry; 31],
    pub icc: [HuffEntry; 8],
}

/// `HUFF_RES_TABLE`
pub type HuffResTable = [[HuffEntry; 8]; 5];

const _: () = assert!(core::mem::size_of::<HuffCldTable>() == 4 * HUFF_CLD_TAB_RAW.len());
const _: () = assert!(core::mem::size_of::<HuffIccTable>() == 4 * HUFF_ICC_TAB_RAW.len());
const _: () = assert!(core::mem::size_of::<HuffPt0Table>() == 4 * HUFF_PART0_TAB_RAW.len());

/// `fdk_sacenc_huffCLDTab`
// SAFETY: `HuffCldTable` is `#[repr(C)]` and consists of `u32` words only (size checked above).
pub static FDK_SACENC_HUFF_CLD_TAB: HuffCldTable =
    unsafe { core::mem::transmute::<[u32; 942], HuffCldTable>(HUFF_CLD_TAB_RAW) };
/// `fdk_sacenc_huffICCTab`
// SAFETY: as above.
pub static FDK_SACENC_HUFF_ICC_TAB: HuffIccTable =
    unsafe { core::mem::transmute::<[u32; 512], HuffIccTable>(HUFF_ICC_TAB_RAW) };
/// `fdk_sacenc_huffPart0Tab`
// SAFETY: as above.
pub static FDK_SACENC_HUFF_PART0_TAB: HuffPt0Table =
    unsafe { core::mem::transmute::<[u32; 39], HuffPt0Table>(HUFF_PART0_TAB_RAW) };

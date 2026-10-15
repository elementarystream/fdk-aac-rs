// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_tools_rom.h, libFDK/src/FDK_tools_rom.cpp.

//! Port of `FDK_tools_rom.h`: ROM tables used by the FDK tools.
//!
//! The tables are generated from the C++ sources by `tools_rom/gen_tables.py` with
//! `scripts/dump_table.py` ([`tables`], [`element_lists`]); their precision is the same in the
//! `aarch64` and `x86` builds (PORTING.md, "Architecture variants"), and the preprocessed
//! translation units of both builds are identical apart from `FDK_ASSERT`. The pointer tables
//! (`windowSlopes`, the `element_list_t` nodes) are rebuilt here with references.
//!
//! `invSqrtTab`, `sqrt_tab` and `invCount` are declared in `fixpoint_math.h` but defined in
//! `FDK_tools_rom.cpp`, so they are here ([`INV_SQRT_TAB`], [`SQRT_TAB`], [`INV_COUNT`]).

use crate::archdef::FixpWtp;
use crate::common_fix::{FixpDbl, FixpSgl};

#[rustfmt::skip]
pub mod element_lists;
#[rustfmt::skip]
pub mod tables;

pub use element_lists::RbdIdT;
pub use tables::*;

use element_lists::*;

use crate::audio::{
    AudioObjectType, AC_EL_USAC_LFE, AOT_AAC_LC, AOT_DRM_AAC, AOT_DRM_MPEG_PS, AOT_DRM_SBR,
    AOT_DRM_SURROUND, AOT_ER_AAC_ELD, AOT_ER_AAC_LC, AOT_ER_AAC_LD, AOT_ER_AAC_SCAL, AOT_PS,
    AOT_SBR, AOT_USAC,
};
use crate::common_fix::{f_normz, fx_dbl2fx_sgl, DFRACT_BITS};

// ---------------------------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------------------------

/// `LowOverlapWindow480` (`#define LowOverlapWindow480 SineWindow120`)
pub use tables::SINE_WINDOW_120 as LOW_OVERLAP_WINDOW_480;
/// `LowOverlapWindow512` (`#define LowOverlapWindow512 SineWindow128`)
pub use tables::SINE_WINDOW_128 as LOW_OVERLAP_WINDOW_512;

/// An entry of [`WINDOW_SLOPES`] (`const FIXP_WTP *`).
pub type WindowSlope = Option<&'static [FixpWtp]>;

/// `windowSlopes[2][4][9]`: helper table containing the length, raster and shape mapping to
/// individual window slope tables:
///
/// - `[0: sine][0: radix 2 raster][ceil(log2(length))]`: length 4 .. 1024
/// - `[0: sine][1: 10 ms raster][ceil(log2(length))]`: length 3.25 .. 960
/// - `[0: sine][2: 3/4 of radix 2 raster][ceil(log2(length))]`: length 3 .. 768
/// - `[1: KBD][0: radix 2 raster][ceil(log2(length))]`: length 128 .. 1024
/// - `[1: KBD][1: 10 ms raster][ceil(log2(length))]`: length 120 .. 960
/// - `[1: KBD][2: 3/4 of radix 2 raster][ceil(log2(length))]`: length 96 .. 768
///
/// You should prefer [`fdk_get_window_slope`]; this table is only public for the optimized
/// access in dct.cpp. Missing initializers are `None` (`NULL`), as in C.
pub static WINDOW_SLOPES: [[[WindowSlope; 9]; 4]; 2] = [
    // Sine
    [
        // Radix 2
        [
            None,
            Some(&SINE_WINDOW_8),
            Some(&SINE_WINDOW_16),
            Some(&SINE_WINDOW_32),
            Some(&SINE_WINDOW_64),
            Some(&SINE_WINDOW_128),
            Some(&SINE_WINDOW_256),
            Some(&SINE_WINDOW_512),
            Some(&SINE_WINDOW_1024),
        ],
        // 10ms raster
        [
            None, // 3.25
            None, // 7.5
            None,
            None,
            None,
            Some(&SINE_WINDOW_120),
            Some(&SINE_WINDOW_240),
            Some(&SINE_WINDOW_480),
            Some(&SINE_WINDOW_960),
        ],
        // 3/4 radix2 raster
        [
            None, // 3
            None, // 6
            Some(&SINE_WINDOW_12),
            Some(&SINE_WINDOW_24),
            Some(&SINE_WINDOW_48),
            Some(&SINE_WINDOW_96),
            Some(&SINE_WINDOW_192),
            Some(&SINE_WINDOW_384),
            Some(&SINE_WINDOW_768),
        ],
        // 3/4 radix2 raster
        [
            None,
            None, // 3
            None, // 6
            Some(&SINE_WINDOW_20),
            Some(&SINE_WINDOW_40),
            None,
            Some(&SINE_WINDOW_160),
            None,
            None,
        ],
    ],
    // KBD
    [
        // Radix 2
        [
            None,
            Some(&KBD_WINDOW_128),
            Some(&KBD_WINDOW_256),
            Some(&SINE_WINDOW_512),
            Some(&KBD_WINDOW_1024),
            None,
            None,
            None,
            None,
        ],
        // 10ms raster
        [
            None,
            Some(&KBD_WINDOW_120),
            None,
            Some(&SINE_WINDOW_480),
            Some(&KBD_WINDOW_960),
            None,
            None,
            None,
            None,
        ],
        // 3/4 radix2 raster
        [
            None,
            Some(&KBD_WINDOW_96),
            Some(&SINE_WINDOW_192), // This entry might be accessed for erred bit streams.
            None,
            Some(&KBD_WINDOW_768),
            None,
            None,
            None,
            None,
        ],
        [None, None, None, None, None, None, None, None, None],
    ],
];

/// `FDKgetWindowSlope(length, shape)`: the window slope of the given length and shape.
///
/// `shape` 0 selects the sine window, 1 the Kaiser-Bessel-derived window; any other value is
/// masked with 1 (but only `shape == 1` adjusts the index, as in C). Returns `None` (`NULL`) if
/// the window slope is not available.
///
/// C indexes `windowSlopes[shape & 1][raster][ld2_length]` without a range check, so an index
/// beyond the 9 entries of a row reads the following entries of the array; the port reproduces
/// this by indexing the flattened array. An index outside the whole array has no defined result
/// in C; the port returns `None` there (PORTING.md, "Deviations from the reference").
pub fn fdk_get_window_slope(length: i32, shape: i32) -> WindowSlope {
    // Get ld2 of length - 2 + 1
    //   -2: because first table entry is window of size 4
    //   +1: because we already include +1 because of ceil(log2(length))
    let mut ld2_length = DFRACT_BITS - 1 - f_normz(length) - 1;

    // Extract sort of "eigenvalue" (the 4 left most bits) of length.
    // The shift amount is negative for lengths below 8 (undefined in C); both reference builds
    // shift by the amount modulo 32 (`asr`, `sar`).
    let raster: i32 = match length.wrapping_shr((ld2_length - 2) as u32) {
        0x8 => {
            // radix 2
            ld2_length -= 1; // revert + 1 because of ceil(log2(length)) from above.
            0
        }
        0xf => 1, // 10 ms
        0xc => 2, // 3/4 of radix 2
        _ => 0,
    };

    // The table for sine windows (shape == 0) is 4 entries longer.
    if shape == 1 {
        ld2_length -= 4;
    }

    // Look up table
    let idx = (shape & 1) * 36 + raster * 9 + ld2_length;
    let flat = WINDOW_SLOPES.as_flattened().as_flattened();
    let w = if (0..flat.len() as i32).contains(&idx) { flat[idx as usize] } else { None };

    fdk_assert!(w.is_some());

    w
}

// ---------------------------------------------------------------------------------------------
// QMF filter and twiddle tables
// ---------------------------------------------------------------------------------------------

/// `FIXP_PFT` (`QMF_COEFF_16BIT`)
pub type FixpPft = FixpSgl;
/// `FIXP_QTW` (`QMF_COEFF_16BIT`)
pub type FixpQtw = FixpSgl;

/// `FX_DBL2FX_QTW(x)`
#[inline]
pub fn fx_dbl2fx_qtw(x: FixpDbl) -> FixpQtw {
    fx_dbl2fx_sgl(x)
}

/// `QMF_FILTER_PROTOTYPE_SIZE`
pub const QMF_FILTER_PROTOTYPE_SIZE: i32 = 640;
/// `QMF_NO_POLY`
pub const QMF_NO_POLY: i32 = 5;
/// `QMF640_PFT_TABLE_SIZE`
pub const QMF640_PFT_TABLE_SIZE: i32 = 640 / 2 + QMF_NO_POLY;
/// `QMF640_CLDFB_PFT_TABLE_SIZE`
pub const QMF640_CLDFB_PFT_TABLE_SIZE: i32 = 640;
/// `QMF320_CLDFB_PFT_TABLE_SIZE`
pub const QMF320_CLDFB_PFT_TABLE_SIZE: i32 = 320;
/// `QMF_CLDFB_PFT_SCALE`
pub const QMF_CLDFB_PFT_SCALE: i32 = 1;
/// `QMF160_CLDFB_PFT_TABLE_SIZE`
pub const QMF160_CLDFB_PFT_TABLE_SIZE: i32 = 160;
/// `QMF80_CLDFB_PFT_TABLE_SIZE`
pub const QMF80_CLDFB_PFT_TABLE_SIZE: i32 = 80;
/// `QMF320_MPSLDFB_PFT_TABLE_SIZE`
pub const QMF320_MPSLDFB_PFT_TABLE_SIZE: i32 = 320;
/// `QMF640_MPSLDFB_PFT_TABLE_SIZE`
pub const QMF640_MPSLDFB_PFT_TABLE_SIZE: i32 = 640;
/// `QMF_MPSLDFB_PFT_SCALE`
pub const QMF_MPSLDFB_PFT_SCALE: i32 = 1;

// ---------------------------------------------------------------------------------------------
// Bitstream element lists
// ---------------------------------------------------------------------------------------------

/// `AC_EL_GA_CCE`: GA AAC coupling channel element (CCE)
pub const AC_EL_GA_CCE: u32 = 0x00000001;

/// `rbd_id_t` (alias without the `T` suffix)
pub type RbdId = RbdIdT;

/// `struct element_list` / `element_list_t`: parser guidance structure.
#[derive(Debug, PartialEq, Eq)]
pub struct ElementList {
    pub id: &'static [RbdIdT],
    pub next: [Option<&'static ElementList>; 2],
}

/// `element_list_t`
pub type ElementListT = ElementList;

const fn node(id: &'static [RbdIdT]) -> ElementList {
    ElementList { id, next: [None, None] }
}

const fn link(
    id: &'static [RbdIdT],
    next0: &'static ElementList,
    next1: &'static ElementList,
) -> ElementList {
    ElementList { id, next: [Some(next0), Some(next1)] }
}

// AOT {2,5,29}, epConfig = -1
static NODE_AAC_SCE: ElementList = node(&EL_AAC_SCE);
// CCE
static NODE_AAC_CCE: ElementList = node(&EL_AAC_CCE);
static NODE_AAC_CPE0: ElementList = node(&EL_AAC_CPE0);
static NODE_AAC_CPE1: ElementList = node(&EL_AAC_CPE1);
static NODE_AAC_CPE: ElementList = link(&EL_AAC_CPE, &NODE_AAC_CPE0, &NODE_AAC_CPE1);

// AOT C- {17,23}, epConfig = 0,1
static NODE_AAC_SCE_EPC0: ElementList = node(&EL_AAC_SCE_EPC0);
static NODE_AAC_SCE_EPC1: ElementList = node(&EL_AAC_SCE_EPC1);
static NODE_AAC_CPE0_EPC0: ElementList = node(&EL_AAC_CPE0_EPC0);
static NODE_AAC_CPE1_EPC0: ElementList = node(&EL_AAC_CPE1_EPC0);
static NODE_AAC_CPE_EPC0: ElementList =
    link(&EL_AAC_CPE_EPC0, &NODE_AAC_CPE0_EPC0, &NODE_AAC_CPE1_EPC0);
static NODE_AAC_CPE0_EPC1: ElementList = node(&EL_AAC_CPE0_EPC1);
static NODE_AAC_CPE1_EPC1: ElementList = node(&EL_AAC_CPE1_EPC1);
// Note: node_aac_cpe_epc1 uses el_aac_cpe (with the ADTS CRC items), not el_aac_cpe_epc0.
static NODE_AAC_CPE_EPC1: ElementList = link(&EL_AAC_CPE, &NODE_AAC_CPE0_EPC1, &NODE_AAC_CPE1_EPC1);

// AOT = 20, epConfig = 0
static NODE_SCAL_SCE_EPC0: ElementList = node(&EL_SCAL_SCE_EPC0);
static NODE_SCAL_CPE_EPC0: ElementList = node(&EL_SCAL_CPE_EPC0);
// AOT = 20, epConfig = 1
static NODE_SCAL_SCE_EPC1: ElementList = node(&EL_SCAL_SCE_EPC1);
static NODE_SCAL_CPE_EPC1: ElementList = node(&EL_SCAL_CPE_EPC1);

// Pseudo AOT for DRM/DRM+ (similar to AOT 20)
static NODE_DRM_SCE: ElementList = node(&EL_DRM_SCE);
static NODE_DRM_CPE: ElementList = node(&EL_DRM_CPE);

// AOT = 39, epConfig = 0
static NODE_ELD_SCE_EPC0: ElementList = node(&EL_ELD_SCE_EPC0);
// #define node_eld_sce_epc1 node_eld_sce_epc0
static NODE_ELD_CPE_EPC0: ElementList = node(&EL_ELD_CPE_EPC0);
static NODE_ELD_CPE_EPC1: ElementList = node(&EL_ELD_CPE_EPC1);

// AOT = 42, epConfig = 0
static NODE_USAC_SCE0_EPC0: ElementList = node(&EL_USAC_SCE0_EPC0);
static NODE_USAC_SCE1_EPC0: ElementList = node(&EL_USAC_LPD_EPC0);
static NODE_USAC_SCE_EPC0: ElementList =
    link(&EL_USAC_COREMODE, &NODE_USAC_SCE0_EPC0, &NODE_USAC_SCE1_EPC0);
// core_mode0 = 0, core_mode1 = 0, common_window = 0, common_tw = 0
static NODE_USAC_CPE0000_EPC0: ElementList = node(&LIST_USAC_CPE0000_EPC0);
// core_mode0 = 0, core_mode1 = 0, common_window = 1, common_tw = 0
static NODE_USAC_CPE0010_EPC0: ElementList = node(&LIST_USAC_CPE0010_EPC0);
// core_mode0 = 0, core_mode1 = 0, common_window = 0, common_tw = 1
static NODE_USAC_CPE0001_EPC0: ElementList = node(&LIST_USAC_CPE0001_EPC0);
// core_mode0 = 0, core_mode1 = 0, common_window = 1, common_tw = 1
static NODE_USAC_CPE0011_EPC0: ElementList = node(&LIST_USAC_CPE0011_EPC0);
// core_mode0 = 0, core_mode1 = 0, common_window = 0
static NODE_USAC_CPE000_EPC0: ElementList =
    link(&EL_USAC_COMMON_TW, &NODE_USAC_CPE0000_EPC0, &NODE_USAC_CPE0001_EPC0);
static NODE_USAC_CPE001_EPC0: ElementList =
    link(&LIST_USAC_CPE001_EPC0, &NODE_USAC_CPE0010_EPC0, &NODE_USAC_CPE0011_EPC0);
// core_mode0 = 0, core_mode1 = 0
static NODE_USAC_CPE00_EPC0: ElementList =
    link(&LIST_USAC_CPE00_EPC0, &NODE_USAC_CPE000_EPC0, &NODE_USAC_CPE001_EPC0);
// core_mode0 = 1, core_mode1 = 0
static NODE_USAC_CPE10_EPC0: ElementList = node(&LIST_USAC_CPE10_EPC0);
static NODE_USAC_CPE01_EPC0: ElementList = node(&LIST_USAC_CPE01_EPC0);
static NODE_USAC_CPE11_EPC0: ElementList = node(&LIST_USAC_CPE11_EPC0);
// core_mode0 = 0
static NODE_USAC_CPE0_EPC0: ElementList =
    link(&EL_USAC_COREMODE, &NODE_USAC_CPE00_EPC0, &NODE_USAC_CPE01_EPC0);
// core_mode0 = 1
static NODE_USAC_CPE1_EPC0: ElementList =
    link(&EL_USAC_COREMODE, &NODE_USAC_CPE10_EPC0, &NODE_USAC_CPE11_EPC0);
static NODE_USAC_CPE_EPC0: ElementList =
    link(&EL_USAC_COREMODE, &NODE_USAC_CPE0_EPC0, &NODE_USAC_CPE1_EPC0);
static NODE_USAC_LFE_EPC0: ElementList = node(&EL_USAC_LFE_EPC0);

/// `getBitstreamElementList(aot, epConfig, nChannels, layer, elFlags)`: the elementary stream
/// pieces list for the given parameters, `None` (`NULL`) for other audio object types.
pub fn get_bitstream_element_list(
    aot: AudioObjectType,
    ep_config: i8,
    n_channels: u8,
    _layer: u8,
    el_flags: u32,
) -> Option<&'static ElementList> {
    match aot {
        AOT_AAC_LC | AOT_SBR | AOT_PS => {
            // FDK_ASSERT(epConfig == -1) is compiled out in the reference build.
            if el_flags & AC_EL_GA_CCE != 0 {
                Some(&NODE_AAC_CCE)
            } else if n_channels == 1 {
                Some(&NODE_AAC_SCE)
            } else {
                Some(&NODE_AAC_CPE)
            }
        }
        AOT_ER_AAC_LC | AOT_ER_AAC_LD => {
            if n_channels == 1 {
                if ep_config == 0 {
                    Some(&NODE_AAC_SCE_EPC0)
                } else {
                    Some(&NODE_AAC_SCE_EPC1)
                }
            } else if ep_config == 0 {
                Some(&NODE_AAC_CPE_EPC0)
            } else {
                Some(&NODE_AAC_CPE_EPC1)
            }
        }
        AOT_USAC => {
            if el_flags & AC_EL_USAC_LFE != 0 {
                // FDK_ASSERT(nChannels == 1) is compiled out in the reference build.
                return Some(&NODE_USAC_LFE_EPC0);
            }
            if n_channels == 1 {
                Some(&NODE_USAC_SCE_EPC0)
            } else {
                Some(&NODE_USAC_CPE_EPC0)
            }
        }
        AOT_ER_AAC_SCAL => {
            if n_channels == 1 {
                if ep_config <= 0 {
                    Some(&NODE_SCAL_SCE_EPC0)
                } else {
                    Some(&NODE_SCAL_SCE_EPC1)
                }
            } else if ep_config <= 0 {
                Some(&NODE_SCAL_CPE_EPC0)
            } else {
                Some(&NODE_SCAL_CPE_EPC1)
            }
        }
        AOT_ER_AAC_ELD => {
            if n_channels == 1 {
                // node_eld_sce_epc1 is #defined to node_eld_sce_epc0
                Some(&NODE_ELD_SCE_EPC0)
            } else if ep_config <= 0 {
                Some(&NODE_ELD_CPE_EPC0)
            } else {
                Some(&NODE_ELD_CPE_EPC1)
            }
        }
        AOT_DRM_AAC | AOT_DRM_SBR | AOT_DRM_MPEG_PS | AOT_DRM_SURROUND => {
            // FDK_ASSERT(epConfig == 1) is compiled out in the reference build.
            if n_channels == 1 {
                Some(&NODE_DRM_SCE)
            } else {
                Some(&NODE_DRM_CPE)
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Format converter
// ---------------------------------------------------------------------------------------------

/// `FDK_converter_formatid_t`
pub type FdkConverterFormatidT = i32;
/// mono
pub const FDK_FORMAT_1_0: FdkConverterFormatidT = 1;
/// stereo
pub const FDK_FORMAT_2_0: FdkConverterFormatidT = 2;
/// 3/0.0
pub const FDK_FORMAT_3_0_FC: FdkConverterFormatidT = 3;
/// 3/1.0
pub const FDK_FORMAT_3_1_0: FdkConverterFormatidT = 4;
/// 3/2.0
pub const FDK_FORMAT_5_0: FdkConverterFormatidT = 5;
/// 5.1
pub const FDK_FORMAT_5_1: FdkConverterFormatidT = 6;
/// 5/2.1 ALT
pub const FDK_FORMAT_7_1_ALT: FdkConverterFormatidT = 7;
/// 2/1.0
pub const FDK_FORMAT_3_0_RC: FdkConverterFormatidT = 9;
/// 2/2.0
pub const FDK_FORMAT_2_2_0: FdkConverterFormatidT = 10;
/// 3/3.1
pub const FDK_FORMAT_6_1: FdkConverterFormatidT = 11;
/// 3/4.1
pub const FDK_FORMAT_7_1: FdkConverterFormatidT = 12;
/// 22.2
pub const FDK_FORMAT_22_2: FdkConverterFormatidT = 13;
/// 5/2.1
pub const FDK_FORMAT_5_2_1: FdkConverterFormatidT = 14;
/// 5/5.2
pub const FDK_FORMAT_5_5_2: FdkConverterFormatidT = 15;
/// 5/4.1
pub const FDK_FORMAT_9_1: FdkConverterFormatidT = 16;
/// 6/5.1
pub const FDK_FORMAT_6_5_1: FdkConverterFormatidT = 17;
/// 6/7.1
pub const FDK_FORMAT_6_7_1: FdkConverterFormatidT = 18;
/// 5/6.1
pub const FDK_FORMAT_5_6_1: FdkConverterFormatidT = 19;
/// 7/6.1
pub const FDK_FORMAT_7_6_1: FdkConverterFormatidT = 20;
/// `FDK_FORMAT_IN_LISTOFCHANNELS`
pub const FDK_FORMAT_IN_LISTOFCHANNELS: FdkConverterFormatidT = 21;
/// `FDK_FORMAT_OUT_LISTOFCHANNELS`
pub const FDK_FORMAT_OUT_LISTOFCHANNELS: FdkConverterFormatidT = 22;
/// 20 formats + In & Out list of channels
pub const FDK_NFORMATS: FdkConverterFormatidT = 23;
/// `FDK_FORMAT_FAIL`
pub const FDK_FORMAT_FAIL: FdkConverterFormatidT = -1;

/// `SQRT_VALUES` (fixpoint_math.h): the size of [`INV_SQRT_TAB`].
pub const SQRT_VALUES: usize = 128 + 2;

// The sizes of the generated tables, as declared in the C++ headers.
const _: () = assert!(INV_SQRT_TAB.len() == SQRT_VALUES);
const _: () = assert!(FORMAT_NCHAN.len() == (FDK_NFORMATS + 9 - 2) as usize);
const _: () = assert!(QMF_PFILT640.len() == (QMF640_PFT_TABLE_SIZE + QMF_NO_POLY) as usize);
const _: () = assert!(QMF_CLDFB_640.len() == QMF640_CLDFB_PFT_TABLE_SIZE as usize);
const _: () = assert!(QMF_CLDFB_320.len() == QMF320_CLDFB_PFT_TABLE_SIZE as usize);
const _: () = assert!(QMF_CLDFB_160.len() == QMF160_CLDFB_PFT_TABLE_SIZE as usize);
const _: () = assert!(QMF_CLDFB_80.len() == QMF80_CLDFB_PFT_TABLE_SIZE as usize);
const _: () = assert!(QMF_MPSLDFB_320.len() == QMF320_MPSLDFB_PFT_TABLE_SIZE as usize);
const _: () = assert!(QMF_MPSLDFB_640.len() == QMF640_MPSLDFB_PFT_TABLE_SIZE as usize);

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_hcr_types.h.

//! Port of `aacdec_hcr_types.h`: HCR (Huffman codeword reordering) decoder: common defines and
//! structures.
//!
//! Integer constants are `i32` (the C `int` of the macros), the `errorLog` bits `u32`.
//!
//! # Pointers
//!
//! `HcrInit()` sets the pointers of the HCR state to memory of the channel that is decoded:
//!
//! | C | Points to | Port |
//! |---|---|---|
//! | `decInOut.pQuantizedSpectralCoefficientsBase` | `pAacDecoderChannelInfo->pSpectralCoefficient` | not stored; the HCR functions take the spectrum of the channel (see [`crate::channelinfo`]) |
//! | `nonPcwSideinfo.pResultBase` | `SPEC_LONG(pAacDecoderChannelInfo->pSpectralCoefficient)` | not stored (the same memory) |
//! | `decInOut.pNumLineInSect` | `pDynData->specificTo.aac.aNumLineInSec4Hcr` | not stored; taken from the channel's `CAacDecoderDynamicData` |
//! | `decInOut.pCodebook` | `pDynData->specificTo.aac.aCodeBooks4Hcr` | not stored; taken from the channel's `CAacDecoderDynamicData` |
//! | `nonPcwSideinfo.pState` (`STATEFUNC`) | `NULL` or one of the seven `Hcr_State_*` functions, always taken from `aStateConstant2State[]` | the state constant ([`StateFunc`]): 0 (`STOP_THIS_STATE`) for `NULL`, 1..=7 for `Hcr_State_BODY_ONLY` .. `Hcr_State_BODY_SIGN_ESC__ESC_WORD`; the HCR code dispatches on it |
//!
//! `HCR_CB_PAIRS` and `HCR_TABLE_INFO` (unused by the library) point to static tables:
//! `&'static` slices.

/* ----------- basic HCR configuration ------------ */

/// `LINES_PER_UNIT`
pub const LINES_PER_UNIT: i32 = 4;

/// `MAX_SFB_HCR`: `(8 * 16)` is not enough because sfbs are split in units for blocktype short
pub const MAX_SFB_HCR: i32 = ((1024 / 8) / LINES_PER_UNIT) * 8;
/// `NUMBER_OF_UNIT_GROUPS`
pub const NUMBER_OF_UNIT_GROUPS: i32 = LINES_PER_UNIT * 8;
/// `LINES_PER_UNIT_GROUP`
pub const LINES_PER_UNIT_GROUP: i32 = 1024 / NUMBER_OF_UNIT_GROUPS;

/// `FROM_LEFT_TO_RIGHT`
pub const FROM_LEFT_TO_RIGHT: i32 = 0;
/// `FROM_RIGHT_TO_LEFT`
pub const FROM_RIGHT_TO_LEFT: i32 = 1;

/// `MAX_CB_PAIRS`
pub const MAX_CB_PAIRS: i32 = 23;
/// `MAX_HCR_SETS`
pub const MAX_HCR_SETS: i32 = 14;

/// `ESCAPE_VALUE`
pub const ESCAPE_VALUE: i32 = 16;
/// `POSITION_OF_FLAG_A`
pub const POSITION_OF_FLAG_A: i32 = 21;
/// `POSITION_OF_FLAG_B`
pub const POSITION_OF_FLAG_B: i32 = 20;

/// `MAX_CB`: last used CB is cb #31 when VCB11 is used
pub const MAX_CB: i32 = 32;

/// `MAX_CB_CHECK`: support for VCB11 available -- is more general, could therefore used in both
/// cases
pub const MAX_CB_CHECK: i32 = 32;

/// `NUMBER_OF_BIT_IN_WORD`
pub const NUMBER_OF_BIT_IN_WORD: i32 = 32;

/* log */
/// `THIRTYTWO_LOG_DIV_TWO_LOG`
pub const THIRTYTWO_LOG_DIV_TWO_LOG: i32 = 5;
/// `EIGHT_LOG_DIV_TWO_LOG`
pub const EIGHT_LOG_DIV_TWO_LOG: i32 = 3;
/// `FOUR_LOG_DIV_TWO_LOG`
pub const FOUR_LOG_DIV_TWO_LOG: i32 = 2;

/* borders */
/// `CPE_TOP_LENGTH`
pub const CPE_TOP_LENGTH: i32 = 12288;
/// `SCE_TOP_LENGTH`
pub const SCE_TOP_LENGTH: i32 = 6144;
/// `LEN_OF_LONGEST_CW_TOP_LENGTH`
pub const LEN_OF_LONGEST_CW_TOP_LENGTH: i32 = 49;

/* qsc's of high level */
/// `Q_VALUE_INVALID`: mark a invalid line with this value (to be concealed later on)
pub const Q_VALUE_INVALID: i32 = 8192;
/// `HCR_DIRAC`: a line of high level
pub const HCR_DIRAC: i32 = 500;

/* masks */
/// `MASK_LEFT`
pub const MASK_LEFT: i32 = 0xFFF000;
/// `MASK_RIGHT`
pub const MASK_RIGHT: i32 = 0xFFF;
/// `CLR_BIT_10`
pub const CLR_BIT_10: i32 = 0x3FF;
/// `TEST_BIT_10`
pub const TEST_BIT_10: i32 = 0x400;

/// `LEFT_OFFSET`
pub const LEFT_OFFSET: i32 = 12;

/* -    insert HCR errors       - */
/// `ERROR_LORSD`: offset: error if different from zero
pub const ERROR_LORSD: i32 = 0;
/// `ERROR_LOLC`: offset: error if different from zero
pub const ERROR_LOLC: i32 = 0;
/// `ERROR_PCW_BODY`
pub const ERROR_PCW_BODY: i32 = 0;
/// `ERROR_PCW_BODY_SIGN`
pub const ERROR_PCW_BODY_SIGN: i32 = 0;
/// `ERROR_PCW_BODY_SIGN_ESC`
pub const ERROR_PCW_BODY_SIGN_ESC: i32 = 0;
/// `ERROR_PCW_BODY_ONLY_TOO_LONG`
pub const ERROR_PCW_BODY_ONLY_TOO_LONG: i32 = 0;
/// `ERROR_PCW_BODY_SIGN_TOO_LONG`
pub const ERROR_PCW_BODY_SIGN_TOO_LONG: i32 = 0;
/// `ERROR_PCW_BODY_SIGN_ESC_TOO_LONG`
pub const ERROR_PCW_BODY_SIGN_ESC_TOO_LONG: i32 = 0;

/// `MODULO_DIVISOR_HCR`
pub const MODULO_DIVISOR_HCR: i32 = 30;

/// `HCR_ERROR_CONCEALMENT`: if set to '1', HCR _mutes_ the erred quantized spectral
/// coefficients
pub const HCR_ERROR_CONCEALMENT: i32 = 1;

/* errorLog: a word of 32 bits used for logging possible errors within HCR in case of distorted
bitstreams */
/// `SEGMENT_OVERRIDE_ERR_PCW_BODY` (bit 31)
pub const SEGMENT_OVERRIDE_ERR_PCW_BODY: u32 = 0x80000000;
/// `SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN` (bit 30)
pub const SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN: u32 = 0x40000000;
/// `SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN_ESC` (bit 29)
pub const SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN_ESC: u32 = 0x20000000;
/// `EXTENDED_SORTED_COUNTER_OVERFLOW` (bit 28)
pub const EXTENDED_SORTED_COUNTER_OVERFLOW: u32 = 0x10000000;

/* special errors */
/// `TOO_MANY_PCW_BODY_BITS_DECODED` (bit 19)
pub const TOO_MANY_PCW_BODY_BITS_DECODED: u32 = 0x00080000;
/// `TOO_MANY_PCW_BODY_SIGN_BITS_DECODED` (bit 18)
pub const TOO_MANY_PCW_BODY_SIGN_BITS_DECODED: u32 = 0x00040000;
/// `TOO_MANY_PCW_BODY_SIGN_ESC_BITS_DECODED` (bit 17)
pub const TOO_MANY_PCW_BODY_SIGN_ESC_BITS_DECODED: u32 = 0x00020000;

/// `STATE_ERROR_BODY_ONLY` (bit 15)
pub const STATE_ERROR_BODY_ONLY: u32 = 0x00008000;
/// `STATE_ERROR_BODY_SIGN__BODY` (bit 14)
pub const STATE_ERROR_BODY_SIGN__BODY: u32 = 0x00004000;
/// `STATE_ERROR_BODY_SIGN__SIGN` (bit 13)
pub const STATE_ERROR_BODY_SIGN__SIGN: u32 = 0x00002000;
/// `STATE_ERROR_BODY_SIGN_ESC__BODY` (bit 12)
pub const STATE_ERROR_BODY_SIGN_ESC__BODY: u32 = 0x00001000;
/// `STATE_ERROR_BODY_SIGN_ESC__SIGN` (bit 11)
pub const STATE_ERROR_BODY_SIGN_ESC__SIGN: u32 = 0x00000800;
/// `STATE_ERROR_BODY_SIGN_ESC__ESC_PREFIX` (bit 10)
pub const STATE_ERROR_BODY_SIGN_ESC__ESC_PREFIX: u32 = 0x00000400;
/// `STATE_ERROR_BODY_SIGN_ESC__ESC_WORD` (bit 9)
pub const STATE_ERROR_BODY_SIGN_ESC__ESC_WORD: u32 = 0x00000200;
/// `HCR_SI_LENGTHS_FAILURE` (bit 8)
pub const HCR_SI_LENGTHS_FAILURE: u32 = 0x00000100;
/// `NUM_SECT_OUT_OF_RANGE_SHORT_BLOCK` (bit 7)
pub const NUM_SECT_OUT_OF_RANGE_SHORT_BLOCK: u32 = 0x00000080;
/// `NUM_SECT_OUT_OF_RANGE_LONG_BLOCK` (bit 6)
pub const NUM_SECT_OUT_OF_RANGE_LONG_BLOCK: u32 = 0x00000040;
/// `LINE_IN_SECT_OUT_OF_RANGE_SHORT_BLOCK` (bit 5)
pub const LINE_IN_SECT_OUT_OF_RANGE_SHORT_BLOCK: u32 = 0x00000020;
/// `CB_OUT_OF_RANGE_SHORT_BLOCK` (bit 4)
pub const CB_OUT_OF_RANGE_SHORT_BLOCK: u32 = 0x00000010;
/// `LINE_IN_SECT_OUT_OF_RANGE_LONG_BLOCK` (bit 3)
pub const LINE_IN_SECT_OUT_OF_RANGE_LONG_BLOCK: u32 = 0x00000008;
/// `CB_OUT_OF_RANGE_LONG_BLOCK` (bit 2)
pub const CB_OUT_OF_RANGE_LONG_BLOCK: u32 = 0x00000004;
/// `LAV_VIOLATION` (bit 1)
pub const LAV_VIOLATION: u32 = 0x00000002;
/// `BIT_IN_SEGMENTATION_ERROR` (bit 0)
pub const BIT_IN_SEGMENTATION_ERROR: u32 = 0x00000001;

/// `HCR_FATAL_PCW_ERROR_MASK`
pub const HCR_FATAL_PCW_ERROR_MASK: u32 = 0x100E01FC;

/// `PCW_TYPE`
pub type PcwType = i32;
/// `PCW_BODY`
pub const PCW_BODY: PcwType = 0;
/// `PCW_BODY_SIGN`
pub const PCW_BODY_SIGN: PcwType = 1;
/// `PCW_BODY_SIGN_ESC`
pub const PCW_BODY_SIGN_ESC: PcwType = 2;

/// `HCR_INPUT_OUTPUT`: interface Decoder <---> HCR. The pointers are not stored, see the
/// [module documentation](self).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HcrInputOutput {
    /// `errorLog`
    pub error_log: u32,
    /// `quantizedSpectralCoefficientsIdx`
    pub quantized_spectral_coefficients_idx: i32,
    /// `lengthOfReorderedSpectralData`
    pub length_of_reordered_spectral_data: i16,
    /// `numSection`
    pub num_section: i16,
    /// `bitstreamAnchor`
    pub bitstream_anchor: i32,
    /// `lengthOfLongestCodeword`
    pub length_of_longest_codeword: i8,
}

/// `HCR_CB_PAIRS`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HcrCbPairs {
    /// `pMinOfCbPair` (empty for `NULL`)
    pub p_min_of_cb_pair: &'static [u8],
    /// `pMaxOfCbPair` (empty for `NULL`)
    pub p_max_of_cb_pair: &'static [u8],
}

/// `HCR_TABLE_INFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HcrTableInfo {
    /// `pLargestAbsVal` (empty for `NULL`)
    pub p_largest_abs_val: &'static [u16],
    /// `pMaxCwLength` (empty for `NULL`)
    pub p_max_cw_length: &'static [u8],
    /// `pCbDimension` (empty for `NULL`)
    pub p_cb_dimension: &'static [u8],
    /// `pCbDimShift` (empty for `NULL`)
    pub p_cb_dim_shift: &'static [u8],
    /// `pCbSign` (empty for `NULL`)
    pub p_cb_sign: &'static [u8],
    /// `pCbPriority` (empty for `NULL`)
    pub p_cb_priority: &'static [u8],
}

/// Size of the bit fields of [`HcrSegmentInfo`]: `((1024 >> 1) / NUMBER_OF_BIT_IN_WORD + 1)`.
pub const HCR_SEGMENT_BITFIELD_SIZE: usize = ((1024 >> 1) / NUMBER_OF_BIT_IN_WORD + 1) as usize;

/// `HCR_SEGMENT_INFO`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HcrSegmentInfo {
    /// `numSegment`
    pub num_segment: u32,
    /// `pSegmentBitfield`
    pub p_segment_bitfield: [u32; HCR_SEGMENT_BITFIELD_SIZE],
    /// `pCodewordBitfield`
    pub p_codeword_bitfield: [u32; HCR_SEGMENT_BITFIELD_SIZE],
    /// `segmentOffset`
    pub segment_offset: u32,
    /// `pLeftStartOfSegment`
    pub p_left_start_of_segment: [i32; 1024 >> 1],
    /// `pRightStartOfSegment`
    pub p_right_start_of_segment: [i32; 1024 >> 1],
    /// `pRemainingBitsInSegment`
    pub p_remaining_bits_in_segment: [i8; 1024 >> 1],
    /// `readDirection`
    pub read_direction: u8,
    /// `numWordForBitfield`
    pub num_word_for_bitfield: u8,
    /// `pNumBitValidInLastWord`
    pub p_num_bit_valid_in_last_word: u16,
}

impl Default for HcrSegmentInfo {
    fn default() -> Self {
        HcrSegmentInfo {
            num_segment: 0,
            p_segment_bitfield: [0; HCR_SEGMENT_BITFIELD_SIZE],
            p_codeword_bitfield: [0; HCR_SEGMENT_BITFIELD_SIZE],
            segment_offset: 0,
            p_left_start_of_segment: [0; 1024 >> 1],
            p_right_start_of_segment: [0; 1024 >> 1],
            p_remaining_bits_in_segment: [0; 1024 >> 1],
            read_direction: 0,
            num_word_for_bitfield: 0,
            p_num_bit_valid_in_last_word: 0,
        }
    }
}

/// `HCR_SECTION_INFO`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HcrSectionInfo {
    /// `numCodeword`
    pub num_codeword: u32,
    /// `numSortedSection`
    pub num_sorted_section: u32,
    /// `pNumCodewordInSection`
    pub p_num_codeword_in_section: [u16; MAX_SFB_HCR as usize],
    /// `pNumSortedCodewordInSection`
    pub p_num_sorted_codeword_in_section: [u16; MAX_SFB_HCR as usize],
    /// `pNumExtendedSortedCodewordInSection`
    pub p_num_extended_sorted_codeword_in_section: [u16; (MAX_SFB_HCR + MAX_HCR_SETS) as usize],
    /// `numExtendedSortedCodewordInSectionIdx`
    pub num_extended_sorted_codeword_in_section_idx: i32,
    /// `pNumExtendedSortedSectionsInSets`
    pub p_num_extended_sorted_sections_in_sets: [u16; MAX_HCR_SETS as usize],
    /// `numExtendedSortedSectionsInSetsIdx`
    pub num_extended_sorted_sections_in_sets_idx: i32,
    /// `pReorderOffset`
    pub p_reorder_offset: [u16; MAX_SFB_HCR as usize],
    /// `pSortedCodebook`
    pub p_sorted_codebook: [u8; MAX_SFB_HCR as usize],

    /// `pExtendedSortedCodebook`
    pub p_extended_sorted_codebook: [u8; (MAX_SFB_HCR + MAX_HCR_SETS) as usize],
    /// `extendedSortedCodebookIdx`
    pub extended_sorted_codebook_idx: i32,
    /// `pMaxLenOfCbInExtSrtSec`
    pub p_max_len_of_cb_in_ext_srt_sec: [u8; (MAX_SFB_HCR + MAX_HCR_SETS) as usize],
    /// `maxLenOfCbInExtSrtSecIdx`
    pub max_len_of_cb_in_ext_srt_sec_idx: i32,
    /// `pCodebookSwitch`
    pub p_codebook_switch: [u8; MAX_SFB_HCR as usize],
}

impl Default for HcrSectionInfo {
    fn default() -> Self {
        HcrSectionInfo {
            num_codeword: 0,
            num_sorted_section: 0,
            p_num_codeword_in_section: [0; MAX_SFB_HCR as usize],
            p_num_sorted_codeword_in_section: [0; MAX_SFB_HCR as usize],
            p_num_extended_sorted_codeword_in_section: [0; (MAX_SFB_HCR + MAX_HCR_SETS) as usize],
            num_extended_sorted_codeword_in_section_idx: 0,
            p_num_extended_sorted_sections_in_sets: [0; MAX_HCR_SETS as usize],
            num_extended_sorted_sections_in_sets_idx: 0,
            p_reorder_offset: [0; MAX_SFB_HCR as usize],
            p_sorted_codebook: [0; MAX_SFB_HCR as usize],
            p_extended_sorted_codebook: [0; (MAX_SFB_HCR + MAX_HCR_SETS) as usize],
            extended_sorted_codebook_idx: 0,
            p_max_len_of_cb_in_ext_srt_sec: [0; (MAX_SFB_HCR + MAX_HCR_SETS) as usize],
            max_len_of_cb_in_ext_srt_sec_idx: 0,
            p_codebook_switch: [0; MAX_SFB_HCR as usize],
        }
    }
}

/// `STATEFUNC` (`UINT (*)(HANDLE_FDK_BITSTREAM, void *)`): the state constant of the function
/// (the index into `aStateConstant2State[]`), 0 for `NULL`. See the
/// [module documentation](self).
pub type StateFunc = u8;

/// `HCR_NON_PCW_SIDEINFO`: worst-case and 1024/4 non-PCWs exist in worst-case. `pResultBase` is
/// not stored, see the [module documentation](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HcrNonPcwSideinfo {
    /// `iNode`: helper indices for code books
    pub i_node: [u32; 1024 >> 2],
    /// `iResultPointer`: helper indices for accessing `pResultBase`
    pub i_result_pointer: [u16; 1024 >> 2],
    /// `pEscapeSequenceInfo`
    pub p_escape_sequence_info: [u32; 1024 >> 2],
    /// `codewordOffset`
    pub codeword_offset: u32,
    /// `pState`: the state constant (0: `NULL`)
    pub p_state: StateFunc,
    /// `pCodebook`
    pub p_codebook: [u8; 1024 >> 2],
    /// `pCntSign`
    pub p_cnt_sign: [u8; 1024 >> 2],
    /// `pSta`: this array holds the states coded as integer values within the range
    /// [0,1,..,7]
    pub p_sta: [i8; 1024 >> 2],
}

impl Default for HcrNonPcwSideinfo {
    fn default() -> Self {
        HcrNonPcwSideinfo {
            i_node: [0; 1024 >> 2],
            i_result_pointer: [0; 1024 >> 2],
            p_escape_sequence_info: [0; 1024 >> 2],
            codeword_offset: 0,
            p_state: 0,
            p_codebook: [0; 1024 >> 2],
            p_cnt_sign: [0; 1024 >> 2],
            p_sta: [0; 1024 >> 2],
        }
    }
}

/// `CErHcrInfo` (`*H_HCR_INFO`)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CErHcrInfo {
    /// `decInOut`
    pub dec_in_out: HcrInputOutput,
    /// `segmentInfo`
    pub segment_info: HcrSegmentInfo,
    /// `sectionInfo`
    pub section_info: HcrSectionInfo,
    /// `nonPcwSideinfo`
    pub non_pcw_sideinfo: HcrNonPcwSideinfo,
}

/// `H_HCR_INFO`
pub type HHcrInfo<'a> = &'a mut CErHcrInfo;

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/rvlc_info.h.

//! Port of `rvlc_info.h`: the structure of the RVLC (reversible variable length coding of the
//! scale factors) decoder.
//!
//! # Pointers
//!
//! - `pRvlBitCnt_RVL` and `pBitstrIndxRvl_RVL` point to `length_of_rvlc_sf_fwd` and
//!   `bitstreamIndexRvlFwd` (forward decoding) or to `length_of_rvlc_sf_bwd` and
//!   `bitstreamIndexRvlBwd` (backward decoding); `rvlc.cpp` always sets the two together. The
//!   port stores the direction ([`FWD`] or [`BWD`]) in [`CErRvlcInfo::p_rvl_bit_cnt_rvl`] and
//!   [`CErRvlcInfo::p_bitstr_indx_rvl_rvl`]; [`CErRvlcInfo::rvl_bit_cnt_rvl`] and
//!   [`CErRvlcInfo::bitstr_indx_rvl_rvl`] resolve them. The zero-initialized value (`NULL` in C,
//!   never dereferenced before it is set) is [`FWD`].
//! - `pHuffTreeRvlCodewds` and `pHuffTreeRvlcEscape` point to the static tables
//!   `aHuffTreeRvlCodewds` and `aHuffTreeRvlcEscape` of `aac_rom`: `&'static [u32]`, empty for
//!   `NULL`.

/// `FWD`: bitstream decoding direction forward (RVL coded part)
pub const FWD: u8 = 0;
/// `BWD`: bitstream decoding direction backward (RVL coded part)
pub const BWD: u8 = 1;

/// `MAX_RVL`: positive RVLC escape
pub const MAX_RVL: i32 = 7;
/// `MIN_RVL`: negative RVLC escape
pub const MIN_RVL: i32 = -7;
/// `MAX_ALLOWED_DPCM_INDEX`: the maximum allowed index of a decoded dpcm value (offset
/// `TABLE_OFFSET` incl --> must be subtracted)
pub const MAX_ALLOWED_DPCM_INDEX: i32 = 14;
/// `TABLE_OFFSET`: dpcm offset of valid output values of rvl table decoding, the rvl table only
/// returns positive values, therefore the offset
pub const TABLE_OFFSET: i32 = 7;
/// `MAX_LEN_RVLC_CODE_WORD`: max length of a RVL codeword in bits
pub const MAX_LEN_RVLC_CODE_WORD: i32 = 9;
/// `MAX_LEN_RVLC_ESCAPE_WORD`: max length of huffman coded RVLC escape word in bits
pub const MAX_LEN_RVLC_ESCAPE_WORD: i32 = 20;

/// `DPCM_NOISE_NRG_BITS`
pub const DPCM_NOISE_NRG_BITS: i32 = 9;
/// `SF_OFFSET`: offset for correcting scf value
pub const SF_OFFSET: i32 = 100;

/// `CONCEAL_MAX_INIT`: arbitrary value
pub const CONCEAL_MAX_INIT: i32 = 1311;
/// `CONCEAL_MIN_INIT`: arbitrary value
pub const CONCEAL_MIN_INIT: i32 = -1311;

/// `RVLC_MAX_SFB`
pub const RVLC_MAX_SFB: i32 = 8 * 16;

/// `CErRvlcInfo` (`RVLC_INFO`): sideinfo of RVLC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CErRvlcInfo {
    /* ------- ESC 1 Data: --------- */
    /* order of RVLC-bitstream components in bitstream (RVLC-initialization), every component
    appears only once in bitstream */
    /// `sf_concealment` (1)
    pub sf_concealment: i32,
    /// `rev_global_gain` (2)
    pub rev_global_gain: i32,
    /// `length_of_rvlc_sf` (3): original value, gets modified (subtract 9) in case of noise
    /// (PNS); is kept for later use
    pub length_of_rvlc_sf: i16,
    /// `dpcm_noise_nrg` (4, optional)
    pub dpcm_noise_nrg: i32,
    /// `sf_escapes_present` (5)
    pub sf_escapes_present: i32,
    /// `length_of_rvlc_escapes` (6, optional)
    pub length_of_rvlc_escapes: i16,
    /// `dpcm_noise_last_position` (7, optional)
    pub dpcm_noise_last_position: i32,

    /// `dpcm_is_last_position`
    pub dpcm_is_last_position: i32,

    /// `length_of_rvlc_sf_fwd`: `length_of_rvlc_sf` used for forward decoding
    pub length_of_rvlc_sf_fwd: i16,
    /// `length_of_rvlc_sf_bwd`: `length_of_rvlc_sf` used for backward decoding
    pub length_of_rvlc_sf_bwd: i16,

    /* for RVL-Codeword decoder to distinguish between fwd and bwd decoding */
    /// `pRvlBitCnt_RVL`: [`FWD`] (`&length_of_rvlc_sf_fwd`) or [`BWD`]
    /// (`&length_of_rvlc_sf_bwd`), see the [module documentation](self).
    pub p_rvl_bit_cnt_rvl: u8,
    /// `pBitstrIndxRvl_RVL`: [`FWD`] (`&bitstreamIndexRvlFwd`) or [`BWD`]
    /// (`&bitstreamIndexRvlBwd`), see the [module documentation](self).
    pub p_bitstr_indx_rvl_rvl: u8,

    /// `numWindowGroups`
    pub num_window_groups: u8,
    /// `maxSfbTransmitted`
    pub max_sfb_transmitted: u8,
    /// `first_noise_group`
    pub first_noise_group: u8,
    /// `first_noise_band`
    pub first_noise_band: u8,
    /// `direction`
    pub direction: u8,

    /* bitstream indices */
    /// `bsAnchor`: hcr bit buffer reference index
    pub bs_anchor: i32,
    /// `bitstreamIndexRvlFwd`: base address of RVL-coded-scalefactor data (ESC 2) for forward
    /// decoding
    pub bitstream_index_rvl_fwd: i32,
    /// `bitstreamIndexRvlBwd`: base address of RVL-coded-scalefactor data (ESC 2) for backward
    /// decoding
    pub bitstream_index_rvl_bwd: i32,
    /// `bitstreamIndexEsc`: base address where RVLC-escapes start (ESC 2)
    pub bitstream_index_esc: i32,

    /* decoding trees */
    /// `pHuffTreeRvlCodewds` (empty for `NULL`)
    pub p_huff_tree_rvl_codewds: &'static [u32],
    /// `pHuffTreeRvlcEscape` (empty for `NULL`)
    pub p_huff_tree_rvlc_escape: &'static [u32],

    /* escape counters */
    /// `numDecodedEscapeWordsFwd`: when decoding RVL-codes forward
    pub num_decoded_escape_words_fwd: u8,
    /// `numDecodedEscapeWordsBwd`: when decoding RVL-codes backward
    pub num_decoded_escape_words_bwd: u8,
    /// `numDecodedEscapeWordsEsc`: when decoding the escape-Words
    pub num_decoded_escape_words_esc: u8,

    /// `noise_used`
    pub noise_used: i8,
    /// `intensity_used`
    pub intensity_used: i8,
    /// `sf_used`
    pub sf_used: i8,

    /// `firstScf`
    pub first_scf: i16,
    /// `lastScf`
    pub last_scf: i16,
    /// `firstNrg`
    pub first_nrg: i16,
    /// `lastNrg`
    pub last_nrg: i16,
    /// `firstIs`
    pub first_is: i16,
    /// `lastIs`
    pub last_is: i16,

    /* ------ RVLC error detection ------ */
    /// `errorLogRvlc`: store RVLC errors
    pub error_log_rvlc: u32,
    /// `conceal_min`: is set at backward decoding
    pub conceal_min: i16,
    /// `conceal_max`: is set at forward decoding
    pub conceal_max: i16,
    /// `conceal_min_esc`: is set at backward decoding
    pub conceal_min_esc: i16,
    /// `conceal_max_esc`: is set at forward decoding
    pub conceal_max_esc: i16,
}

/// `RVLC_INFO`
pub type RvlcInfo = CErRvlcInfo;

impl Default for CErRvlcInfo {
    /// Zero-initialized (`NULL` pointers), like the zeroed memory the decoder allocates.
    fn default() -> Self {
        CErRvlcInfo {
            sf_concealment: 0,
            rev_global_gain: 0,
            length_of_rvlc_sf: 0,
            dpcm_noise_nrg: 0,
            sf_escapes_present: 0,
            length_of_rvlc_escapes: 0,
            dpcm_noise_last_position: 0,
            dpcm_is_last_position: 0,
            length_of_rvlc_sf_fwd: 0,
            length_of_rvlc_sf_bwd: 0,
            p_rvl_bit_cnt_rvl: FWD,
            p_bitstr_indx_rvl_rvl: FWD,
            num_window_groups: 0,
            max_sfb_transmitted: 0,
            first_noise_group: 0,
            first_noise_band: 0,
            direction: 0,
            bs_anchor: 0,
            bitstream_index_rvl_fwd: 0,
            bitstream_index_rvl_bwd: 0,
            bitstream_index_esc: 0,
            p_huff_tree_rvl_codewds: &[],
            p_huff_tree_rvlc_escape: &[],
            num_decoded_escape_words_fwd: 0,
            num_decoded_escape_words_bwd: 0,
            num_decoded_escape_words_esc: 0,
            noise_used: 0,
            intensity_used: 0,
            sf_used: 0,
            first_scf: 0,
            last_scf: 0,
            first_nrg: 0,
            last_nrg: 0,
            first_is: 0,
            last_is: 0,
            error_log_rvlc: 0,
            conceal_min: 0,
            conceal_max: 0,
            conceal_min_esc: 0,
            conceal_max_esc: 0,
        }
    }
}

impl CErRvlcInfo {
    /// `*pRvlBitCnt_RVL`: `length_of_rvlc_sf_fwd` or `length_of_rvlc_sf_bwd`.
    #[inline]
    pub fn rvl_bit_cnt_rvl(&mut self) -> &mut i16 {
        if self.p_rvl_bit_cnt_rvl == BWD {
            &mut self.length_of_rvlc_sf_bwd
        } else {
            &mut self.length_of_rvlc_sf_fwd
        }
    }

    /// `*pBitstrIndxRvl_RVL`: `bitstreamIndexRvlFwd` or `bitstreamIndexRvlBwd`.
    #[inline]
    pub fn bitstr_indx_rvl_rvl(&mut self) -> &mut i32 {
        if self.p_bitstr_indx_rvl_rvl == BWD {
            &mut self.bitstream_index_rvl_bwd
        } else {
            &mut self.bitstream_index_rvl_fwd
        }
    }
}

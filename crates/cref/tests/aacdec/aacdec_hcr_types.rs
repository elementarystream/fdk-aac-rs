// Differential tests of fdk_aac_rs_aacdec::aacdec_hcr_types against the C++ reference.

use fdk_aac_rs_aacdec::aacdec_hcr_types::*;
use fdk_aac_rs_cref::aacdec_aacdec_hcr_types as c;

#[test]
fn constants() {
    let mut out = [0i32; 128];
    let n = unsafe { c::cref_hcrt_constants(out.as_mut_ptr()) } as usize;
    let r = [
        LINES_PER_UNIT,
        MAX_SFB_HCR,
        NUMBER_OF_UNIT_GROUPS,
        LINES_PER_UNIT_GROUP,
        FROM_LEFT_TO_RIGHT,
        FROM_RIGHT_TO_LEFT,
        MAX_CB_PAIRS,
        MAX_HCR_SETS,
        ESCAPE_VALUE,
        POSITION_OF_FLAG_A,
        POSITION_OF_FLAG_B,
        MAX_CB,
        MAX_CB_CHECK,
        NUMBER_OF_BIT_IN_WORD,
        THIRTYTWO_LOG_DIV_TWO_LOG,
        EIGHT_LOG_DIV_TWO_LOG,
        FOUR_LOG_DIV_TWO_LOG,
        CPE_TOP_LENGTH,
        SCE_TOP_LENGTH,
        LEN_OF_LONGEST_CW_TOP_LENGTH,
        Q_VALUE_INVALID,
        HCR_DIRAC,
        MASK_LEFT,
        MASK_RIGHT,
        CLR_BIT_10,
        TEST_BIT_10,
        LEFT_OFFSET,
        ERROR_LORSD,
        ERROR_LOLC,
        ERROR_PCW_BODY,
        ERROR_PCW_BODY_SIGN,
        ERROR_PCW_BODY_SIGN_ESC,
        ERROR_PCW_BODY_ONLY_TOO_LONG,
        ERROR_PCW_BODY_SIGN_TOO_LONG,
        ERROR_PCW_BODY_SIGN_ESC_TOO_LONG,
        MODULO_DIVISOR_HCR,
        HCR_ERROR_CONCEALMENT,
        SEGMENT_OVERRIDE_ERR_PCW_BODY as i32,
        SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN as i32,
        SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN_ESC as i32,
        EXTENDED_SORTED_COUNTER_OVERFLOW as i32,
        TOO_MANY_PCW_BODY_BITS_DECODED as i32,
        TOO_MANY_PCW_BODY_SIGN_BITS_DECODED as i32,
        TOO_MANY_PCW_BODY_SIGN_ESC_BITS_DECODED as i32,
        STATE_ERROR_BODY_ONLY as i32,
        STATE_ERROR_BODY_SIGN__BODY as i32,
        STATE_ERROR_BODY_SIGN__SIGN as i32,
        STATE_ERROR_BODY_SIGN_ESC__BODY as i32,
        STATE_ERROR_BODY_SIGN_ESC__SIGN as i32,
        STATE_ERROR_BODY_SIGN_ESC__ESC_PREFIX as i32,
        STATE_ERROR_BODY_SIGN_ESC__ESC_WORD as i32,
        HCR_SI_LENGTHS_FAILURE as i32,
        NUM_SECT_OUT_OF_RANGE_SHORT_BLOCK as i32,
        NUM_SECT_OUT_OF_RANGE_LONG_BLOCK as i32,
        LINE_IN_SECT_OUT_OF_RANGE_SHORT_BLOCK as i32,
        CB_OUT_OF_RANGE_SHORT_BLOCK as i32,
        LINE_IN_SECT_OUT_OF_RANGE_LONG_BLOCK as i32,
        CB_OUT_OF_RANGE_LONG_BLOCK as i32,
        LAV_VIOLATION as i32,
        BIT_IN_SEGMENTATION_ERROR as i32,
        HCR_FATAL_PCW_ERROR_MASK as i32,
        PCW_BODY,
        PCW_BODY_SIGN,
        PCW_BODY_SIGN_ESC,
    ];
    assert_eq!(&out[..n], &r[..]);
}

#[test]
fn array_lengths() {
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_hcrt_array_lengths(out.as_mut_ptr()) } as usize;
    let h = CErHcrInfo::default();
    let (sg, sc, np) = (&h.segment_info, &h.section_info, &h.non_pcw_sideinfo);
    let r = [
        sg.p_segment_bitfield.len(),
        sg.p_codeword_bitfield.len(),
        sg.p_left_start_of_segment.len(),
        sg.p_right_start_of_segment.len(),
        sg.p_remaining_bits_in_segment.len(),
        sc.p_num_codeword_in_section.len(),
        sc.p_num_sorted_codeword_in_section.len(),
        sc.p_num_extended_sorted_codeword_in_section.len(),
        sc.p_num_extended_sorted_sections_in_sets.len(),
        sc.p_reorder_offset.len(),
        sc.p_sorted_codebook.len(),
        sc.p_extended_sorted_codebook.len(),
        sc.p_max_len_of_cb_in_ext_srt_sec.len(),
        sc.p_codebook_switch.len(),
        np.i_node.len(),
        np.i_result_pointer.len(),
        np.p_escape_sequence_info.len(),
        np.p_codebook.len(),
        np.p_cnt_sign.len(),
        np.p_sta.len(),
    ];
    let r: Vec<i32> = r.iter().map(|&x| x as i32).collect();
    assert_eq!(&out[..n], &r[..]);
}

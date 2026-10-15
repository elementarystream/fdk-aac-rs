// Differential tests of fdk_aac_rs_drc::drc_dec_rom against the C++ reference: every table is
// compared with the memory image of the C table.

use fdk_aac_rs_cref::drc_drc_dec_rom as c;
use fdk_aac_rs_drc::drc_dec_rom::*;
use fdk_aac_rs_drc::drc_dec_types::{pod_bytes, Pod};

fn c_table(t: i32) -> Vec<u8> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { c::cref_drc_rom_table(t, buf.as_mut_ptr(), buf.len() as i32) };
    assert!(n >= 0, "table {t}");
    buf.truncate(n as usize);
    buf
}

fn check<T: Pod>(t: i32, name: &str, table: &T) {
    assert_eq!(pod_bytes(table), &c_table(t)[..], "{name}");
}

#[test]
fn tables() {
    check(0, "deltaGain_codingProfile_0_1_huffman", &DELTA_GAIN_CODING_PROFILE_0_1_HUFFMAN);
    check(1, "deltaGain_codingProfile_2_huffman", &DELTA_GAIN_CODING_PROFILE_2_HUFFMAN);
    check(2, "slopeSteepness", &SLOPE_STEEPNESS);
    check(3, "slopeSteepness_huffman", &SLOPE_STEEPNESS_HUFFMAN);
    check(4, "downmixCoeff", &DOWNMIX_COEFF);
    check(5, "downmixCoeffV1", &DOWNMIX_COEFF_V1);
    check(6, "cicpDrcCharSigmoidLeft", &CICP_DRC_CHAR_SIGMOID_LEFT);
    check(7, "cicpDrcCharSigmoidRight", &CICP_DRC_CHAR_SIGMOID_RIGHT);
    check(8, "cicpDrcCharNodesLeft", &CICP_DRC_CHAR_NODES_LEFT);
    check(9, "cicpDrcCharNodesRight", &CICP_DRC_CHAR_NODES_RIGHT);
}

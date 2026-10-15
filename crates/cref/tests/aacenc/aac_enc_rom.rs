// Differential tests of fdk_aac_rs_aacenc::aac_enc_rom against the C++ reference.
//
// Every table of aacEnc_rom.cpp is compared as memory with the table of the reference library
// (SfbParamLong / SfbParamShort have the layout of SFB_PARAM_LONG / SFB_PARAM_SHORT).

use fdk_aac_rs_aacenc::aac_enc_rom::*;
use fdk_aac_rs_aacenc::psy_configuration::{SfbParamLong, SfbParamShort};
use fdk_aac_rs_cref::aacenc_aac_enc_rom as c;
use std::collections::HashSet;
use std::ffi::{c_void, CStr};

fn bytes<T>(t: &T) -> &[u8] {
    // SAFETY: the tables consist of integers (and of u8 structs without padding).
    unsafe { std::slice::from_raw_parts(t as *const T as *const u8, std::mem::size_of_val(t)) }
}

/// The Rust table of a C table name, as memory.
fn rust_table(name: &str) -> Option<&'static [u8]> {
    macro_rules! tables {
        ($($c:literal => $r:ident),* $(,)?) => {
            match name {
                $($c => Some(bytes(&$r)),)*
                _ => None,
            }
        };
    }
    tables!(
        "FDKaacEnc_huff_ltab1_2" => FDK_AAC_ENC_HUFF_LTAB_1_2,
        "FDKaacEnc_huff_ltab3_4" => FDK_AAC_ENC_HUFF_LTAB_3_4,
        "FDKaacEnc_huff_ltab5_6" => FDK_AAC_ENC_HUFF_LTAB_5_6,
        "FDKaacEnc_huff_ltab7_8" => FDK_AAC_ENC_HUFF_LTAB_7_8,
        "FDKaacEnc_huff_ltab9_10" => FDK_AAC_ENC_HUFF_LTAB_9_10,
        "FDKaacEnc_huff_ltab11" => FDK_AAC_ENC_HUFF_LTAB_11,
        "FDKaacEnc_huff_ltabscf" => FDK_AAC_ENC_HUFF_LTABSCF,
        "FDKaacEnc_huff_ctab1" => FDK_AAC_ENC_HUFF_CTAB_1,
        "FDKaacEnc_huff_ctab2" => FDK_AAC_ENC_HUFF_CTAB_2,
        "FDKaacEnc_huff_ctab3" => FDK_AAC_ENC_HUFF_CTAB_3,
        "FDKaacEnc_huff_ctab4" => FDK_AAC_ENC_HUFF_CTAB_4,
        "FDKaacEnc_huff_ctab5" => FDK_AAC_ENC_HUFF_CTAB_5,
        "FDKaacEnc_huff_ctab6" => FDK_AAC_ENC_HUFF_CTAB_6,
        "FDKaacEnc_huff_ctab7" => FDK_AAC_ENC_HUFF_CTAB_7,
        "FDKaacEnc_huff_ctab8" => FDK_AAC_ENC_HUFF_CTAB_8,
        "FDKaacEnc_huff_ctab9" => FDK_AAC_ENC_HUFF_CTAB_9,
        "FDKaacEnc_huff_ctab10" => FDK_AAC_ENC_HUFF_CTAB_10,
        "FDKaacEnc_huff_ctab11" => FDK_AAC_ENC_HUFF_CTAB_11,
        "FDKaacEnc_huff_ctabscf" => FDK_AAC_ENC_HUFF_CTABSCF,
        "FDKaacEnc_mTab_3_4" => FDK_AAC_ENC_M_TAB_3_4,
        "FDKaacEnc_quantTableQ" => FDK_AAC_ENC_QUANT_TABLE_Q,
        "FDKaacEnc_quantTableE" => FDK_AAC_ENC_QUANT_TABLE_E,
        "FDKaacEnc_mTab_4_3Elc" => FDK_AAC_ENC_M_TAB_4_3_ELC,
        "FDKaacEnc_specExpMantTableCombElc" => FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC,
        "FDKaacEnc_specExpTableComb" => FDK_AAC_ENC_SPEC_EXP_TABLE_COMB,
        "FDKaacEnc_sideInfoTabLong" => FDK_AAC_ENC_SIDE_INFO_TAB_LONG,
        "FDKaacEnc_sideInfoTabShort" => FDK_AAC_ENC_SIDE_INFO_TAB_SHORT,
        "p_FDKaacEnc_8000_long_1024" => P_FDK_AAC_ENC_8000_LONG_1024,
        "p_FDKaacEnc_8000_short_128" => P_FDK_AAC_ENC_8000_SHORT_128,
        "p_FDKaacEnc_11025_long_1024" => P_FDK_AAC_ENC_11025_LONG_1024,
        "p_FDKaacEnc_11025_short_128" => P_FDK_AAC_ENC_11025_SHORT_128,
        "p_FDKaacEnc_12000_long_1024" => P_FDK_AAC_ENC_12000_LONG_1024,
        "p_FDKaacEnc_12000_short_128" => P_FDK_AAC_ENC_12000_SHORT_128,
        "p_FDKaacEnc_16000_long_1024" => P_FDK_AAC_ENC_16000_LONG_1024,
        "p_FDKaacEnc_16000_short_128" => P_FDK_AAC_ENC_16000_SHORT_128,
        "p_FDKaacEnc_22050_long_1024" => P_FDK_AAC_ENC_22050_LONG_1024,
        "p_FDKaacEnc_22050_short_128" => P_FDK_AAC_ENC_22050_SHORT_128,
        "p_FDKaacEnc_24000_long_1024" => P_FDK_AAC_ENC_24000_LONG_1024,
        "p_FDKaacEnc_24000_short_128" => P_FDK_AAC_ENC_24000_SHORT_128,
        "p_FDKaacEnc_32000_long_1024" => P_FDK_AAC_ENC_32000_LONG_1024,
        "p_FDKaacEnc_32000_short_128" => P_FDK_AAC_ENC_32000_SHORT_128,
        "p_FDKaacEnc_44100_long_1024" => P_FDK_AAC_ENC_44100_LONG_1024,
        "p_FDKaacEnc_44100_short_128" => P_FDK_AAC_ENC_44100_SHORT_128,
        "p_FDKaacEnc_48000_long_1024" => P_FDK_AAC_ENC_48000_LONG_1024,
        "p_FDKaacEnc_48000_short_128" => P_FDK_AAC_ENC_48000_SHORT_128,
        "p_FDKaacEnc_64000_long_1024" => P_FDK_AAC_ENC_64000_LONG_1024,
        "p_FDKaacEnc_64000_short_128" => P_FDK_AAC_ENC_64000_SHORT_128,
        "p_FDKaacEnc_88200_long_1024" => P_FDK_AAC_ENC_88200_LONG_1024,
        "p_FDKaacEnc_88200_short_128" => P_FDK_AAC_ENC_88200_SHORT_128,
        "p_FDKaacEnc_96000_long_1024" => P_FDK_AAC_ENC_96000_LONG_1024,
        "p_FDKaacEnc_96000_short_128" => P_FDK_AAC_ENC_96000_SHORT_128,
        "FDKaacEnc_tnsEncCoeff3" => FDK_AAC_ENC_TNS_ENC_COEFF_3,
        "FDKaacEnc_tnsCoeff3Borders" => FDK_AAC_ENC_TNS_COEFF_3_BORDERS,
        "FDKaacEnc_tnsEncCoeff4" => FDK_AAC_ENC_TNS_ENC_COEFF_4,
        "FDKaacEnc_tnsCoeff4Borders" => FDK_AAC_ENC_TNS_COEFF_4_BORDERS,
        "ELDAnalysis512" => ELD_ANALYSIS_512,
        "ELDAnalysis480" => ELD_ANALYSIS_480,
        "ELDAnalysis256" => ELD_ANALYSIS_256,
        "ELDAnalysis240" => ELD_ANALYSIS_240,
        "ELDAnalysis128" => ELD_ANALYSIS_128,
        "ELDAnalysis120" => ELD_ANALYSIS_120,
    )
}

#[test]
fn tables_match() {
    assert_eq!(std::mem::size_of::<SfbParamLong>(), unsafe {
        c::cref_aac_enc_rom_sizeof_sfb_param_long()
    } as usize);
    assert_eq!(std::mem::size_of::<SfbParamShort>(), unsafe {
        c::cref_aac_enc_rom_sizeof_sfb_param_short()
    } as usize);
    let n = unsafe { c::cref_aac_enc_rom_count() };
    let mut seen = HashSet::new();
    for i in 0..n {
        let mut ptr: *const c_void = std::ptr::null();
        let mut nbytes = 0;
        let name = unsafe { CStr::from_ptr(c::cref_aac_enc_rom_table(i, &mut ptr, &mut nbytes)) }
            .to_str()
            .unwrap()
            .to_string();
        let cmem = unsafe { std::slice::from_raw_parts(ptr as *const u8, nbytes as usize) };
        let r = rust_table(&name).unwrap_or_else(|| panic!("no Rust table for {name}"));
        assert_eq!(r, cmem, "table {name}");
        seen.insert(name);
    }
    assert_eq!(seen.len(), 61);
}

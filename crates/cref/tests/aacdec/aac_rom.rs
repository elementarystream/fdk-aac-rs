// Differential tests of fdk_aac_rs_aacdec::aac_rom against the C++ reference.
//
// Every table of aac_rom.cpp is compared element by element (as memory) with the table of the
// reference library, or with a copy of aac_rom.cpp compiled into the shim for the tables that
// are file-local in the library. The pointer tables are compared entry by entry: the copy's
// pointer identifies the table by name, and the memory the library's pointer refers to is
// compared with the Rust table.

use fdk_aac_rs_aacdec::aac_rom::*;
use fdk_aac_rs_cref::aacdec_aac_rom as c;
use fdk_aac_rs_cref::testutil::Rng;
use std::collections::HashSet;
use std::ffi::CStr;

fn bytes<T>(t: &[T]) -> &[u8] {
    // SAFETY: the tables consist of integers (or arrays of integers).
    unsafe { std::slice::from_raw_parts(t.as_ptr() as *const u8, std::mem::size_of_val(t)) }
}

/// The Rust table of a C table name, as memory and element size (of the first dimension).
fn rust_table(name: &str) -> Option<(&'static [u8], usize)> {
    macro_rules! tables {
        ($($c:literal => $r:ident),* $(,)?) => {
            match name {
                $($c => Some((bytes(&$r[..]), std::mem::size_of_val(&$r[0]))),)*
                _ => None,
            }
        };
    }
    tables!(
        "InverseQuantTable" => INVERSE_QUANT_TABLE, "MantissaTable" => MANTISSA_TABLE,
        "ExponentTable" => EXPONENT_TABLE, "sfb_96_1024" => SFB_96_1024,
        "sfb_96_128" => SFB_96_128, "sfb_64_1024" => SFB_64_1024, "sfb_64_128" => SFB_64_128,
        "sfb_48_1024" => SFB_48_1024, "sfb_48_128" => SFB_48_128, "sfb_32_1024" => SFB_32_1024,
        "sfb_24_1024" => SFB_24_1024, "sfb_24_128" => SFB_24_128, "sfb_16_1024" => SFB_16_1024,
        "sfb_16_128" => SFB_16_128, "sfb_8_1024" => SFB_8_1024, "sfb_8_128" => SFB_8_128,
        "sfb_96_960" => SFB_96_960, "sfb_96_120" => SFB_96_120, "sfb_64_960" => SFB_64_960,
        "sfb_64_120" => SFB_64_120, "sfb_48_960" => SFB_48_960, "sfb_48_120" => SFB_48_120,
        "sfb_32_960" => SFB_32_960, "sfb_24_960" => SFB_24_960, "sfb_24_120" => SFB_24_120,
        "sfb_16_960" => SFB_16_960, "sfb_16_120" => SFB_16_120, "sfb_8_960" => SFB_8_960,
        "sfb_8_120" => SFB_8_120, "sfb_96_768" => SFB_96_768, "sfb_96_96" => SFB_96_96,
        "sfb_64_768" => SFB_64_768, "sfb_64_96" => SFB_64_96, "sfb_48_768" => SFB_48_768,
        "sfb_48_96" => SFB_48_96, "sfb_32_768" => SFB_32_768, "sfb_24_768" => SFB_24_768,
        "sfb_24_96" => SFB_24_96, "sfb_16_768" => SFB_16_768, "sfb_16_96" => SFB_16_96,
        "sfb_8_768" => SFB_8_768, "sfb_8_96" => SFB_8_96, "sfb_48_512" => SFB_48_512,
        "sfb_32_512" => SFB_32_512, "sfb_24_512" => SFB_24_512, "sfb_48_480" => SFB_48_480,
        "sfb_32_480" => SFB_32_480, "sfb_24_480" => SFB_24_480,
        "HuffmanCodeBook_1" => HUFFMAN_CODE_BOOK_1, "HuffmanCodeBook_2" => HUFFMAN_CODE_BOOK_2,
        "HuffmanCodeBook_3" => HUFFMAN_CODE_BOOK_3, "HuffmanCodeBook_4" => HUFFMAN_CODE_BOOK_4,
        "HuffmanCodeBook_5" => HUFFMAN_CODE_BOOK_5, "HuffmanCodeBook_6" => HUFFMAN_CODE_BOOK_6,
        "HuffmanCodeBook_7" => HUFFMAN_CODE_BOOK_7, "HuffmanCodeBook_8" => HUFFMAN_CODE_BOOK_8,
        "HuffmanCodeBook_9" => HUFFMAN_CODE_BOOK_9, "HuffmanCodeBook_10" => HUFFMAN_CODE_BOOK_10,
        "HuffmanCodeBook_11" => HUFFMAN_CODE_BOOK_11,
        "HuffmanCodeBook_SCL" => HUFFMAN_CODE_BOOK_SCL,
        "aHuffTree41" => A_HUFF_TREE_41, "aHuffTree42" => A_HUFF_TREE_42,
        "aHuffTree43" => A_HUFF_TREE_43, "aHuffTree44" => A_HUFF_TREE_44,
        "aHuffTree21" => A_HUFF_TREE_21, "aHuffTree22" => A_HUFF_TREE_22,
        "aHuffTree23" => A_HUFF_TREE_23, "aHuffTree24" => A_HUFF_TREE_24,
        "aHuffTree25" => A_HUFF_TREE_25, "aHuffTree26" => A_HUFF_TREE_26,
        "aHuffTree27" => A_HUFF_TREE_27,
        "aValTab41" => A_VAL_TAB_41, "aValTab42" => A_VAL_TAB_42, "aValTab21" => A_VAL_TAB_21,
        "aValTab22" => A_VAL_TAB_22, "aValTab23" => A_VAL_TAB_23, "aValTab24" => A_VAL_TAB_24,
        "aMaxCwLen" => A_MAX_CW_LEN, "aDimCb" => A_DIM_CB, "aDimCbShift" => A_DIM_CB_SHIFT,
        "aSignCb" => A_SIGN_CB, "aMinOfCbPair" => A_MIN_OF_CB_PAIR,
        "aMaxOfCbPair" => A_MAX_OF_CB_PAIR, "aCbPriority" => A_CB_PRIORITY,
        "aCodebook2StartInt" => A_CODEBOOK_2_START_INT,
        "aLargestAbsoluteValue" => A_LARGEST_ABSOLUTE_VALUE,
        "aHuffTreeRvlcEscape" => A_HUFF_TREE_RVLC_ESCAPE,
        "aHuffTreeRvlCodewds" => A_HUFF_TREE_RVL_CODEWDS,
        "LowDelaySynthesis256" => LOW_DELAY_SYNTHESIS_256,
        "LowDelaySynthesis240" => LOW_DELAY_SYNTHESIS_240,
        "LowDelaySynthesis160" => LOW_DELAY_SYNTHESIS_160,
        "LowDelaySynthesis128" => LOW_DELAY_SYNTHESIS_128,
        "LowDelaySynthesis120" => LOW_DELAY_SYNTHESIS_120,
        "LowDelaySynthesis512" => LOW_DELAY_SYNTHESIS_512,
        "LowDelaySynthesis480" => LOW_DELAY_SYNTHESIS_480,
        "tns_max_bands_tbl" => TNS_MAX_BANDS_TBL, "tns_max_bands_tbl_480" => TNS_MAX_BANDS_TBL_480,
        "tns_max_bands_tbl_512" => TNS_MAX_BANDS_TBL_512,
        "FDKaacDec_tnsCoeff3" => FDK_AAC_DEC_TNS_COEFF3,
        "FDKaacDec_tnsCoeff4" => FDK_AAC_DEC_TNS_COEFF4,
        "FDKaacDec_tnsCoeff3_gain_ld" => FDK_AAC_DEC_TNS_COEFF3_GAIN_LD,
        "FDKaacDec_tnsCoeff4_gain_ld" => FDK_AAC_DEC_TNS_COEFF4_GAIN_LD,
        "elementsTab" => ELEMENTS_TAB, "AacDec_randomSign" => AAC_DEC_RANDOM_SIGN,
        "mdst_filt_coef_curr" => MDST_FILT_COEF_CURR, "mdst_filt_coef_prev" => MDST_FILT_COEF_PREV,
    )
}

fn name(p: *const i8) -> String {
    unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string()
}

/// All 106 tables of scalars of aac_rom.cpp, element by element.
#[test]
fn tables() {
    let n = unsafe { c::cref_rom_num_tables() };
    assert_eq!(n, 106, "number of tables in the shim");
    let mut seen = HashSet::new();
    for i in 0..n {
        let tname = name(unsafe { c::cref_rom_table_name(i) });
        let (mut nbytes, mut elem) = (0, 0);
        let p = unsafe { c::cref_rom_table(i, &mut nbytes, &mut elem) };
        let ct = unsafe { std::slice::from_raw_parts(p as *const u8, nbytes as usize) };
        let (rt, relem) = rust_table(&tname).unwrap_or_else(|| panic!("no Rust table {tname}"));
        assert_eq!(relem, elem as usize, "{tname}: element size");
        assert_eq!(rt.len(), ct.len(), "{tname}: size");
        for (k, (a, b)) in rt.chunks(relem).zip(ct.chunks(relem)).enumerate() {
            assert_eq!(a, b, "{tname}[{k}]");
        }
        assert!(seen.insert(tname));
    }
}

/// Checks a pointer entry: the copy's target (by name, "NULL" for NULL) is the Rust slice, and
/// the library's pointer refers to memory with the same content.
fn check_ptr<T: PartialEq + std::fmt::Debug>(
    what: &str,
    copy_name: *const i8,
    lib: *const T,
    rust: &'static [T],
) {
    let cn = name(copy_name);
    if cn == "NULL" {
        assert!(rust.is_empty(), "{what}: NULL");
        assert!(lib.is_null(), "{what}: library NULL");
        return;
    }
    let (rt, _) = rust_table(&cn).unwrap_or_else(|| panic!("{what}: unknown table {cn}"));
    assert_eq!(rt.as_ptr(), rust.as_ptr() as *const u8, "{what}: points to {cn}");
    assert_eq!(rt.len(), std::mem::size_of_val(rust), "{what}: length of {cn}");
    assert!(!lib.is_null(), "{what}: library pointer");
    // SAFETY: the library's table has the size of the copy's (compared in `tables`).
    let l = unsafe { std::slice::from_raw_parts(lib, rust.len()) };
    assert_eq!(l, rust, "{what}: library content");
}

#[test]
fn sfb_offset_tables() {
    for i in 0..5 {
        for j in 0..16 {
            let (mut ln, mut sn) = (std::ptr::null(), std::ptr::null());
            let (mut ll, mut ls) = (std::ptr::null(), std::ptr::null());
            let (mut nl, mut ns) = (0, 0);
            unsafe {
                c::cref_rom_sfb_info(i, j, &mut ln, &mut sn, &mut ll, &mut ls, &mut nl, &mut ns)
            };
            let r = &SFB_OFFSET_TABLES[i as usize][j as usize];
            check_ptr(&format!("sfbOffsetTables[{i}][{j}].sfbOffsetLong"), ln, ll, r.sfb_offset_long);
            check_ptr(&format!("sfbOffsetTables[{i}][{j}].sfbOffsetShort"), sn, ls, r.sfb_offset_short);
            assert_eq!(r.number_of_sfb_long as i32, nl, "sfbOffsetTables[{i}][{j}].numberOfSfbLong");
            assert_eq!(r.number_of_sfb_short as i32, ns, "sfbOffsetTables[{i}][{j}].numberOfSfbShort");
        }
    }
}

#[test]
fn code_book_descriptions() {
    for i in 0..14 {
        let mut cn = std::ptr::null();
        let mut lib: *const u16 = std::ptr::null();
        let mut f = [0i32; 3];
        unsafe { c::cref_rom_codebook(i, &mut cn, &mut lib, f.as_mut_ptr()) };
        let r = if i < 13 { &AAC_CODE_BOOK_DESCRIPTION_TABLE[i as usize] } else { &AAC_CODE_BOOK_DESCRIPTION_SCL };
        check_ptr(&format!("codebook {i}"), cn, lib as *const [u16; 4], r.code_book);
        assert_eq!([r.dimension as i32, r.num_bits as i32, r.offset as i32], f, "codebook {i}");
    }
}

#[test]
fn huff_and_quant_tables() {
    let n = unsafe { c::cref_rom_num_huff_table() };
    assert_eq!(n as usize, A_HUFF_TABLE.len());
    for i in 0..n {
        let mut lib = std::ptr::null();
        let cn = unsafe { c::cref_rom_huff_table(i, &mut lib) };
        check_ptr(&format!("aHuffTable[{i}]"), cn, lib, A_HUFF_TABLE[i as usize]);
    }
    let n = unsafe { c::cref_rom_num_quant_table() };
    assert_eq!(n as usize, A_QUANT_TABLE.len());
    for i in 0..n {
        let mut lib = std::ptr::null();
        let cn = unsafe { c::cref_rom_quant_table(i, &mut lib) };
        check_ptr(&format!("aQuantTable[{i}]"), cn, lib, A_QUANT_TABLE[i as usize]);
    }
}

/// `aStateConstant2State[i]` is the `i`-th state function (`NULL` for 0), so the identity table
/// of the port is exact; the state constants of aacdec_hcrs.h are 0..=7.
#[test]
fn state_constant2state() {
    let n = unsafe { c::cref_rom_num_state_constant2state() };
    assert_eq!(n as usize, A_STATE_CONSTANT_2_STATE.len());
    for i in 0..n {
        let k = unsafe { c::cref_rom_state_constant2state(i) };
        assert_eq!(k, A_STATE_CONSTANT_2_STATE[i as usize] as i32, "aStateConstant2State[{i}]");
    }
}

#[test]
fn constants() {
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_rom_constants(out.as_mut_ptr()) } as usize;
    let r = [
        AACDEC_MAX_CH_CONF,
        AACDEC_CH_ELEMENTS_TAB_SIZE,
        AAC_NF_NO_RANDOM_VAL,
        INV_QUANT_TABLESIZE,
        NUM_LD_COEF_512,
        NUM_LD_COEF_480,
        WTS0,
        WTS1,
        WTS2,
        HuffmanBits,
        HuffmanEntries,
        SF_FNA_COEFFS,
        std::mem::size_of::<PcmAac>() as i32,
        std::mem::size_of::<PcmDec>() as i32,
        MAXVAL_PCM_DEC,
        MINVAL_PCM_DEC,
        PCM_DEC_BITS,
        std::mem::size_of::<FixpTcc>() as i32,
        std::mem::size_of::<FixpFilt>() as i32,
        0,
        1,
        2,
        3,
        4,
        5,
        6,
        7,
    ];
    assert_eq!(&out[..n], &r[..]);
}

#[test]
fn pcm_macros() {
    let mut rng = Rng::new(0xa0c_0001);
    for _ in 0..10000 {
        let x = rng.i32();
        let s = rng.i16();
        assert_eq!(fixp_dbl2pcm_dec(x), unsafe { c::cref_rom_fixp_dbl2pcm_dec(x) });
        assert_eq!(pcm_dec2fixp_dbl(x), unsafe { c::cref_rom_pcm_dec2fixp_dbl(x) });
        assert_eq!(pcm_dec2fx_pcm(x), unsafe { c::cref_rom_pcm_dec2fx_pcm(x) }, "{x}");
        assert_eq!(fx_pcm2pcm_dec(s), unsafe { c::cref_rom_fx_pcm2pcm_dec(s) }, "{s}");
    }
}

// Differential tests of fdk_aac_rs_fdk::tools_rom against the C++ reference.
//
// Every table is compared element by element (as memory: FixpSpk has the layout of FIXP_SPK)
// with the table of the reference library, or with a copy of FDK_tools_rom.cpp compiled into the
// shim for the tables that are file-local in the library. The pointer tables (windowSlopes, the
// element list nodes) are compared entry by entry.

use fdk_aac_rs_cref::fdk_tools_rom as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::archdef::FixpWtp;
use fdk_aac_rs_fdk::tools_rom::*;
use std::collections::HashSet;
use std::ffi::CStr;

fn bytes<T>(t: &[T]) -> &[u8] {
    // SAFETY: the tables consist of integers (FixpSpk is two i16 fields, repr(C)).
    unsafe { std::slice::from_raw_parts(t.as_ptr() as *const u8, std::mem::size_of_val(t)) }
}

/// The Rust table of a C table name, as memory and element size.
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
        "SineTable80" => SINE_TABLE_80, "SineTable384" => SINE_TABLE_384,
        "SineTable480" => SINE_TABLE_480, "SineTable512" => SINE_TABLE_512,
        "SineTable1024" => SINE_TABLE_1024,
        "RotVectorReal6" => ROT_VECTOR_REAL_6, "RotVectorImag6" => ROT_VECTOR_IMAG_6,
        "RotVectorReal12" => ROT_VECTOR_REAL_12, "RotVectorImag12" => ROT_VECTOR_IMAG_12,
        "RotVectorReal24" => ROT_VECTOR_REAL_24, "RotVectorImag24" => ROT_VECTOR_IMAG_24,
        "RotVectorReal48" => ROT_VECTOR_REAL_48, "RotVectorImag48" => ROT_VECTOR_IMAG_48,
        "RotVectorReal80" => ROT_VECTOR_REAL_80, "RotVectorImag80" => ROT_VECTOR_IMAG_80,
        "RotVectorReal96" => ROT_VECTOR_REAL_96, "RotVectorImag96" => ROT_VECTOR_IMAG_96,
        "RotVectorReal384" => ROT_VECTOR_REAL_384, "RotVectorImag384" => ROT_VECTOR_IMAG_384,
        "RotVectorReal60" => ROT_VECTOR_REAL_60, "RotVectorImag60" => ROT_VECTOR_IMAG_60,
        "RotVectorReal120" => ROT_VECTOR_REAL_120, "RotVectorImag120" => ROT_VECTOR_IMAG_120,
        "RotVectorReal192" => ROT_VECTOR_REAL_192, "RotVectorImag192" => ROT_VECTOR_IMAG_192,
        "RotVectorReal240" => ROT_VECTOR_REAL_240, "RotVectorImag240" => ROT_VECTOR_IMAG_240,
        "RotVectorReal480" => ROT_VECTOR_REAL_480, "RotVectorImag480" => ROT_VECTOR_IMAG_480,
        "RotVectorReal20" => ROT_VECTOR_REAL_20, "RotVectorImag20" => ROT_VECTOR_IMAG_20,
        "SineWindow8" => SINE_WINDOW_8, "SineWindow12" => SINE_WINDOW_12,
        "SineWindow16" => SINE_WINDOW_16, "SineWindow20" => SINE_WINDOW_20,
        "SineWindow24" => SINE_WINDOW_24, "SineWindow32" => SINE_WINDOW_32,
        "SineWindow40" => SINE_WINDOW_40, "SineWindow48" => SINE_WINDOW_48,
        "SineWindow64" => SINE_WINDOW_64, "SineWindow96" => SINE_WINDOW_96,
        "SineWindow120" => SINE_WINDOW_120, "SineWindow128" => SINE_WINDOW_128,
        "SineWindow160" => SINE_WINDOW_160, "SineWindow192" => SINE_WINDOW_192,
        "SineWindow240" => SINE_WINDOW_240, "SineWindow256" => SINE_WINDOW_256,
        "SineWindow384" => SINE_WINDOW_384, "SineWindow480" => SINE_WINDOW_480,
        "SineWindow512" => SINE_WINDOW_512, "SineWindow768" => SINE_WINDOW_768,
        "SineWindow960" => SINE_WINDOW_960, "SineWindow1024" => SINE_WINDOW_1024,
        "KBDWindow96" => KBD_WINDOW_96, "KBDWindow120" => KBD_WINDOW_120,
        "KBDWindow128" => KBD_WINDOW_128, "KBDWindow256" => KBD_WINDOW_256,
        "KBDWindow768" => KBD_WINDOW_768, "KBDWindow960" => KBD_WINDOW_960,
        "KBDWindow1024" => KBD_WINDOW_1024,
        "qmf_pfilt120" => QMF_PFILT120, "qmf_pfilt200" => QMF_PFILT200,
        "qmf_phaseshift_cos40" => QMF_PHASESHIFT_COS40,
        "qmf_phaseshift_sin40" => QMF_PHASESHIFT_SIN40,
        "qmf_pfilt400" => QMF_PFILT400,
        "qmf_phaseshift_cos16" => QMF_PHASESHIFT_COS16,
        "qmf_phaseshift_sin16" => QMF_PHASESHIFT_SIN16,
        "qmf_pfilt240" => QMF_PFILT240,
        "qmf_phaseshift_cos24" => QMF_PHASESHIFT_COS24,
        "qmf_phaseshift_sin24" => QMF_PHASESHIFT_SIN24,
        "qmf_phaseshift_cos32" => QMF_PHASESHIFT_COS32,
        "qmf_phaseshift_sin32" => QMF_PHASESHIFT_SIN32,
        "qmf_phaseshift_cos_downsamp32" => QMF_PHASESHIFT_COS_DOWNSAMP32,
        "qmf_phaseshift_sin_downsamp32" => QMF_PHASESHIFT_SIN_DOWNSAMP32,
        "qmf_pfilt640" => QMF_PFILT640, "qmf_pfilt640_vector" => QMF_PFILT640_VECTOR,
        "qmf_phaseshift_cos64" => QMF_PHASESHIFT_COS64,
        "qmf_phaseshift_sin64" => QMF_PHASESHIFT_SIN64,
        "qmf_phaseshift_cos32_cldfb_ana" => QMF_PHASESHIFT_COS32_CLDFB_ANA,
        "qmf_phaseshift_cos32_cldfb_syn" => QMF_PHASESHIFT_COS32_CLDFB_SYN,
        "qmf_phaseshift_sin32_cldfb" => QMF_PHASESHIFT_SIN32_CLDFB,
        "qmf_phaseshift_cos16_cldfb_ana" => QMF_PHASESHIFT_COS16_CLDFB_ANA,
        "qmf_phaseshift_cos16_cldfb_syn" => QMF_PHASESHIFT_COS16_CLDFB_SYN,
        "qmf_phaseshift_sin16_cldfb" => QMF_PHASESHIFT_SIN16_CLDFB,
        "qmf_phaseshift_cos8_cldfb_ana" => QMF_PHASESHIFT_COS8_CLDFB_ANA,
        "qmf_phaseshift_cos8_cldfb_syn" => QMF_PHASESHIFT_COS8_CLDFB_SYN,
        "qmf_phaseshift_sin8_cldfb" => QMF_PHASESHIFT_SIN8_CLDFB,
        "qmf_phaseshift_cos64_cldfb" => QMF_PHASESHIFT_COS64_CLDFB,
        "qmf_phaseshift_sin64_cldfb" => QMF_PHASESHIFT_SIN64_CLDFB,
        "qmf_cldfb_640" => QMF_CLDFB_640, "qmf_cldfb_320" => QMF_CLDFB_320,
        "qmf_cldfb_160" => QMF_CLDFB_160, "qmf_cldfb_80" => QMF_CLDFB_80,
        "qmf_mpsldfb_320" => QMF_MPSLDFB_320, "qmf_mpsldfb_640" => QMF_MPSLDFB_640,
        "sin_twiddle_L64" => SIN_TWIDDLE_L64, "sqrt_tab" => SQRT_TAB,
        "invCount" => INV_COUNT, "invSqrtTab" => INV_SQRT_TAB,
        "format_nchan" => FORMAT_NCHAN,
    )
}

fn name(p: *const i8) -> String {
    unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string()
}

/// All 100 tables of FDK_tools_rom.cpp, element by element.
#[test]
fn tables() {
    let n = unsafe { c::cref_tr_num_tables() };
    assert_eq!(n, 100, "number of tables in the shim");
    let mut seen = HashSet::new();
    for i in 0..n {
        let tname = name(unsafe { c::cref_tr_table_name(i) });
        let (mut nbytes, mut elem) = (0, 0);
        let p = unsafe { c::cref_tr_table(i, &mut nbytes, &mut elem) };
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

/// The Rust window table of a C window table name.
fn rust_window(n: &str) -> &'static [FixpWtp] {
    let (b, _) = rust_table(n).unwrap();
    // SAFETY: the window tables are FixpWtp arrays.
    unsafe { std::slice::from_raw_parts(b.as_ptr() as *const FixpWtp, b.len() / 4) }
}

fn same(a: WindowSlope, b: WindowSlope) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.as_ptr() == b.as_ptr() && a.len() == b.len(),
        _ => false,
    }
}

/// The C window slope with index `idx` of the shim's window list.
fn c_window(idx: i32) -> WindowSlope {
    match idx {
        -1 => None,
        i if i >= 0 => Some(rust_window(&name(unsafe { c::cref_tr_window_name(i) }))),
        _ => panic!("unknown window table"),
    }
}

#[test]
fn window_slopes() {
    for s in 0..2 {
        for r in 0..4 {
            for i in 0..9 {
                let e = c_window(unsafe { c::cref_tr_window_slopes(s, r, i) });
                assert!(same(WINDOW_SLOPES[s as usize][r as usize][i as usize], e), "windowSlopes[{s}][{r}][{i}]");
            }
        }
    }
    assert!(same(Some(&LOW_OVERLAP_WINDOW_512[..]), Some(&SINE_WINDOW_128[..])));
    assert!(same(Some(&LOW_OVERLAP_WINDOW_480[..]), Some(&SINE_WINDOW_120[..])));
}

fn check_window_slope(length: i32, shape: i32) {
    let mut lib: *const i16 = std::ptr::null();
    let idx = unsafe { c::cref_tr_get_window_slope(length, shape, &mut lib) };
    let e = c_window(idx);
    // FDK_ASSERT(w != NULL) is a debug_assert in the port: only valid lengths in `checked`.
    if e.is_none() && cfg!(debug_assertions) {
        assert!(lib.is_null());
        return;
    }
    let r = fdk_get_window_slope(length, shape);
    assert!(same(r, e), "FDKgetWindowSlope({length}, {shape})");
    match r {
        None => assert!(lib.is_null()),
        Some(w) => {
            // The library's result has the same content.
            let l = unsafe { std::slice::from_raw_parts(lib as *const FixpWtp, w.len()) };
            assert_eq!(w, l, "FDKgetWindowSlope({length}, {shape}) of the library");
        }
    }
}

#[test]
fn fdk_get_window_slope_() {
    // All lengths up to 4096.
    for shape in [-3, -2, -1, 0, 1, 2, 3] {
        for length in 4..=4096 {
            check_window_slope(length, shape);
        }
    }
    // Shape 0 for lengths below 4 down to 2, and longer lengths with a defined reference
    // result.
    for length in [2, 3, 5000, 6000, 8191] {
        check_window_slope(length, 0);
    }
}

/// Compares the element list trees of C and Rust; collects the visited Rust id lists.
fn compare_list(r: Option<&'static ElementList>, c: *const core::ffi::c_void, seen: &mut HashSet<usize>, what: &str) {
    match r {
        None => assert!(c.is_null(), "{what}: C has a node, Rust none"),
        Some(r) => {
            assert!(!c.is_null(), "{what}: Rust has a node, C none");
            seen.insert(r.id.as_ptr() as usize);
            let ids = unsafe { c::cref_tr_element_list_ids(c) };
            let mut k = 0;
            loop {
                let v = unsafe { *ids.add(k) };
                assert!(k < r.id.len(), "{what}: Rust list too short");
                assert_eq!(r.id[k] as i32, v, "{what}: id[{k}]");
                k += 1;
                if v == RbdIdT::end_of_sequence as i32 || v == RbdIdT::link_sequence as i32 {
                    break;
                }
            }
            assert_eq!(r.id.len(), k, "{what}: Rust list too long");
            for i in 0..2 {
                let cn = unsafe { c::cref_tr_element_list_next(c, i) };
                compare_list(r.next[i as usize], cn, seen, &format!("{what}.next[{i}]"));
            }
        }
    }
}

#[test]
fn get_bitstream_element_list_() {
    let mut seen = HashSet::new();
    let mut rng = Rng::new(301);
    let mut aots: Vec<i32> = (-1..=50).chain(140..=150).collect();
    aots.extend((0..20).map(|_| rng.next_u32() as i32));
    for &aot in &aots {
        for ep in [-128i32, -2, -1, 0, 1, 2, 3, 127] {
            for nch in [0u8, 1, 2, 3, 255] {
                for layer in [0u8, 1, 7] {
                    for flags in [0u32, 1, 0x20, 0x21, 0x10, 0xFFFF_FFFF, rng.next_u32()] {
                        let r = get_bitstream_element_list(aot, ep as i8, nch, layer, flags);
                        let cl = unsafe {
                            c::cref_tr_get_bitstream_element_list(aot, ep, nch as i32, layer as i32, flags)
                        };
                        compare_list(r, cl, &mut seen, &format!("getBitstreamElementList({aot}, {ep}, {nch}, {layer}, {flags:#x})"));
                    }
                }
            }
        }
    }
    // Every element list of FDK_tools_rom.cpp is reachable.
    assert_eq!(seen.len(), 35);
}

#[test]
fn rbd_id_values() {
    let n = unsafe { c::cref_tr_num_rbd_ids() };
    assert_eq!(n, 41);
    for i in 0..n {
        let mut p: *const i8 = std::ptr::null();
        let v = unsafe { c::cref_tr_rbd_id(i, &mut p) };
        assert_eq!(v, i, "the enumerators are consecutive");
        // SAFETY: RbdIdT is repr(i32) with the discriminants 0..=40.
        let r: RbdIdT = unsafe { std::mem::transmute::<i32, RbdIdT>(i) };
        assert_eq!(format!("{r:?}"), name(p), "rbd_id_t value {i}");
    }
}

#[test]
fn fx_dbl2fx_qtw_() {
    let mut rng = Rng::new(302);
    for _ in 0..100_000 {
        let x = rng.i32();
        assert_eq!(fx_dbl2fx_qtw(x), unsafe { c::cref_tr_fx_dbl2fx_qtw(x) }, "FX_DBL2FX_QTW({x})");
    }
}

#[test]
fn constants() {
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_tr_constants(out.as_mut_ptr()) } as usize;
    let r = [
        QMF_FILTER_PROTOTYPE_SIZE,
        QMF_NO_POLY,
        QMF640_PFT_TABLE_SIZE,
        QMF640_CLDFB_PFT_TABLE_SIZE,
        QMF320_CLDFB_PFT_TABLE_SIZE,
        QMF_CLDFB_PFT_SCALE,
        QMF160_CLDFB_PFT_TABLE_SIZE,
        QMF80_CLDFB_PFT_TABLE_SIZE,
        QMF320_MPSLDFB_PFT_TABLE_SIZE,
        QMF640_MPSLDFB_PFT_TABLE_SIZE,
        QMF_MPSLDFB_PFT_SCALE,
        AC_EL_GA_CCE as i32,
        FDK_FORMAT_1_0,
        FDK_FORMAT_2_0,
        FDK_FORMAT_3_0_FC,
        FDK_FORMAT_3_1_0,
        FDK_FORMAT_5_0,
        FDK_FORMAT_5_1,
        FDK_FORMAT_7_1_ALT,
        FDK_FORMAT_3_0_RC,
        FDK_FORMAT_2_2_0,
        FDK_FORMAT_6_1,
        FDK_FORMAT_7_1,
        FDK_FORMAT_22_2,
        FDK_FORMAT_5_2_1,
        FDK_FORMAT_5_5_2,
        FDK_FORMAT_9_1,
        FDK_FORMAT_6_5_1,
        FDK_FORMAT_6_7_1,
        FDK_FORMAT_5_6_1,
        FDK_FORMAT_7_6_1,
        FDK_FORMAT_IN_LISTOFCHANNELS,
        FDK_FORMAT_OUT_LISTOFCHANNELS,
        FDK_NFORMATS,
        FDK_FORMAT_FAIL,
        SQRT_VALUES as i32,
    ];
    assert_eq!(&r[..], &out[..n]);
}

/// The element lists of FDK_tools_rom.cpp (file-static; the copy compiled into the shim),
/// element by element (`get_bitstream_element_list_` compares them as the trees the function
/// returns).
#[test]
fn element_lists_() {
    use fdk_aac_rs_fdk::tools_rom::element_lists;
    let rust = |name: &str| -> &'static [RbdIdT] {
        match name {
        "el_aac_sce" => &element_lists::EL_AAC_SCE,
        "el_aac_cce" => &element_lists::EL_AAC_CCE,
        "el_aac_cpe" => &element_lists::EL_AAC_CPE,
        "el_aac_cpe0" => &element_lists::EL_AAC_CPE0,
        "el_aac_cpe1" => &element_lists::EL_AAC_CPE1,
        "el_aac_sce_epc0" => &element_lists::EL_AAC_SCE_EPC0,
        "el_aac_sce_epc1" => &element_lists::EL_AAC_SCE_EPC1,
        "el_aac_cpe_epc0" => &element_lists::EL_AAC_CPE_EPC0,
        "el_aac_cpe0_epc0" => &element_lists::EL_AAC_CPE0_EPC0,
        "el_aac_cpe1_epc0" => &element_lists::EL_AAC_CPE1_EPC0,
        "el_aac_cpe0_epc1" => &element_lists::EL_AAC_CPE0_EPC1,
        "el_aac_cpe1_epc1" => &element_lists::EL_AAC_CPE1_EPC1,
        "el_scal_sce_epc0" => &element_lists::EL_SCAL_SCE_EPC0,
        "el_scal_cpe_epc0" => &element_lists::EL_SCAL_CPE_EPC0,
        "el_scal_sce_epc1" => &element_lists::EL_SCAL_SCE_EPC1,
        "el_scal_cpe_epc1" => &element_lists::EL_SCAL_CPE_EPC1,
        "el_drm_sce" => &element_lists::EL_DRM_SCE,
        "el_drm_cpe" => &element_lists::EL_DRM_CPE,
        "el_eld_sce_epc0" => &element_lists::EL_ELD_SCE_EPC0,
        "el_eld_cpe_epc0" => &element_lists::EL_ELD_CPE_EPC0,
        "el_eld_cpe_epc1" => &element_lists::EL_ELD_CPE_EPC1,
        "el_usac_coremode" => &element_lists::EL_USAC_COREMODE,
        "el_usac_sce0_epc0" => &element_lists::EL_USAC_SCE0_EPC0,
        "el_usac_lfe_epc0" => &element_lists::EL_USAC_LFE_EPC0,
        "el_usac_lpd_epc0" => &element_lists::EL_USAC_LPD_EPC0,
        "list_usac_cpe00_epc0" => &element_lists::LIST_USAC_CPE00_EPC0,
        "el_usac_common_tw" => &element_lists::EL_USAC_COMMON_TW,
        "list_usac_cpe0000_epc0" => &element_lists::LIST_USAC_CPE0000_EPC0,
        "list_usac_cpe0001_epc0" => &element_lists::LIST_USAC_CPE0001_EPC0,
        "list_usac_cpe001_epc0" => &element_lists::LIST_USAC_CPE001_EPC0,
        "list_usac_cpe0010_epc0" => &element_lists::LIST_USAC_CPE0010_EPC0,
        "list_usac_cpe0011_epc0" => &element_lists::LIST_USAC_CPE0011_EPC0,
        "list_usac_cpe10_epc0" => &element_lists::LIST_USAC_CPE10_EPC0,
        "list_usac_cpe01_epc0" => &element_lists::LIST_USAC_CPE01_EPC0,
        "list_usac_cpe11_epc0" => &element_lists::LIST_USAC_CPE11_EPC0,
            _ => panic!("unknown element list {name}"),
        }
    };
    let mut i = 0;
    loop {
        let (mut ids, mut n) = (core::ptr::null(), 0);
        let name = unsafe { c::cref_tr_element_list(i, &mut ids, &mut n) };
        if name.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap();
        let c: Vec<i32> = unsafe { std::slice::from_raw_parts(ids, n as usize) }.to_vec();
        let r: Vec<i32> = rust(name).iter().map(|&x| x as i32).collect();
        assert_eq!(r, c, "{name}");
        i += 1;
    }
    assert_eq!(i, 35);
}

// Differential tests of fdk_aac_rs_aacdec::usacdec_rom against the C++ reference.
//
// Every table of usacdec_rom.cpp is compared element by element (as memory) with the table of the
// reference library.

use fdk_aac_rs_aacdec::usacdec_rom::*;
use fdk_aac_rs_cref::aacdec_usacdec_rom as c;
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
        "fdk_dec_tab_factorial" => FDK_DEC_TAB_FACTORIAL, "fdk_dec_Da" => FDK_DEC_DA,
        "fdk_dec_Ds" => FDK_DEC_DS, "fdk_dec_Ns" => FDK_DEC_NS, "fdk_dec_Ia" => FDK_DEC_IA,
        "fdk_dec_Is" => FDK_DEC_IS, "fdk_dec_A3" => FDK_DEC_A3, "fdk_dec_A4" => FDK_DEC_A4,
        "fdk_dec_I3" => FDK_DEC_I3, "fdk_dec_I4" => FDK_DEC_I4,
        "fdk_dec_lsf_init" => FDK_DEC_LSF_INIT,
        "fdk_dec_dico_lsf_abs_8b" => FDK_DEC_DICO_LSF_ABS_8B,
        "fdk_t_qua_gain7b" => FDK_T_QUA_GAIN7B, "lsp_interpol_factor" => LSP_INTERPOL_FACTOR,
        "fdk_dec_filt_lp" => FDK_DEC_FILT_LP, "FacWindowSynth128" => FAC_WINDOW_SYNTH_128,
        "FacWindowZir128" => FAC_WINDOW_ZIR_128, "FacWindowSynth64" => FAC_WINDOW_SYNTH_64,
        "FacWindowZir64" => FAC_WINDOW_ZIR_64, "FacWindowSynth32" => FAC_WINDOW_SYNTH_32,
        "FacWindowZir32" => FAC_WINDOW_ZIR_32, "FacWindowSynth96" => FAC_WINDOW_SYNTH_96,
        "FacWindowZir96" => FAC_WINDOW_ZIR_96, "FacWindowSynth48" => FAC_WINDOW_SYNTH_48,
        "FacWindowZir48" => FAC_WINDOW_ZIR_48,
    )
}

fn name(p: *const i8) -> String {
    unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string()
}

/// All 25 tables of usacdec_rom.cpp, element by element.
#[test]
fn tables() {
    let n = unsafe { c::cref_urom_num_tables() };
    assert_eq!(n, 25, "number of tables in the shim");
    let mut seen = HashSet::new();
    for i in 0..n {
        let tname = name(unsafe { c::cref_urom_table_name(i) });
        let (mut nbytes, mut elem) = (0, 0);
        let p = unsafe { c::cref_urom_table(i, &mut nbytes, &mut elem) };
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

#[test]
fn constants() {
    let mut out = [0i32; 16];
    let n = unsafe { c::cref_urom_constants(out.as_mut_ptr()) } as usize;
    let r = [NB_SPHERE, NB_LEADER, NB_LDSIGN, NB_LDQ3, NB_LDQ4, LSF_SCALE, SF_QUA_GAIN7B, L_FILT];
    assert_eq!(&out[..n], &r[..]);
}

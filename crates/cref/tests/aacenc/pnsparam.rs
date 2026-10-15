// Differential tests of fdk_aac_rs_aacenc::pnsparam against the C++ reference.
//
// The PNS parameters are computed for the scalefactor band tables of all configurations of the
// encoder (from FDKaacEnc_InitPsyConfiguration) and compared as complete NOISEPARAMS structs
// (copied from C into the port's #[repr(C)] struct), including the values that the function
// leaves unchanged.

use crate::psy_configuration::{block_types, init_psy_checked, GRANULES, RATES};
use core::ffi::c_void;
use fdk_aac_rs_aacenc::pnsparam::tables::*;
use fdk_aac_rs_aacenc::pnsparam::*;
use fdk_aac_rs_cref::aacenc_pnsparam as c;
use fdk_aac_rs_cref::testutil::Rng;
use std::ffi::CStr;

fn bytes<T: ?Sized>(t: &T) -> &[u8] {
    // SAFETY: the tables consist of integers (padding is explicit).
    unsafe { core::slice::from_raw_parts(t as *const T as *const u8, size_of_val(t)) }
}

#[test]
fn tables_match() {
    let n = unsafe { c::cref_pnsparam_table_count() };
    for i in 0..n {
        let mut p = core::ptr::null();
        let mut len = 0;
        let name = unsafe { CStr::from_ptr(c::cref_pnsparam_table(i, &mut p, &mut len)) };
        let name = name.to_str().unwrap();
        let cb = unsafe { core::slice::from_raw_parts(p as *const u8, len as usize) };
        let rb = match name {
            "levelTable_mono" => bytes(&LEVEL_TABLE_MONO[..]),
            "levelTable_stereo" => bytes(&LEVEL_TABLE_STEREO[..]),
            "levelTable_lowComplexity" => bytes(&LEVEL_TABLE_LOW_COMPLEXITY[..]),
            "pnsInfoTab" => bytes(&PNS_INFO_TAB[..]),
            "pnsInfoTab_lowComplexity" => bytes(&PNS_INFO_TAB_LOW_COMPLEXITY[..]),
            _ => panic!("unknown table {name}"),
        };
        assert_eq!(rb, cb, "{name}");
    }
    assert_eq!(n, 5);
}

/// Bit rates with a defined reference result (see fdk_aac_enc_look_up_pns_use).
fn bitrates(r: &mut Rng) -> Vec<i32> {
    let mut v: Vec<i32> = (0..1_000_000).step_by(997).collect();
    for b in [0, 11999, 12000, 19999, 20000, 27999, 28000, 28999, 29000, 31999, 32000, 40999,
        41000, 47999, 48000, 48001, 55999, 56000, 61999, 62000, 79999, 80000, 99999, 100000, 999999]
    {
        v.push(b);
    }
    for _ in 0..200 {
        v.push(r.range(0, 999_999));
    }
    v
}

#[test]
fn look_up_pns_use() {
    let mut r = Rng::new(0x7a51_0001);
    for br in bitrates(&mut r) {
        for sr in RATES.iter().copied().chain([7350, 0, -1, 192000]) {
            for ch in 0..4 {
                for lc in [0, 1, 5] {
                    let cc = unsafe { c::cref_pnsparam_lookUpPnsUse(br, sr, ch, lc) };
                    assert_eq!(fdk_aac_enc_look_up_pns_use(br, sr, ch, lc), cc, "br {br} sr {sr} ch {ch} lc {lc}");
                }
            }
        }
    }
}

fn random_np(r: &mut Rng) -> NoiseParams {
    let mut np = NoiseParams::default();
    if r.range(0, 1) == 0 {
        np.start_sfb = r.i16();
        np.detection_algorithm_flags = r.i16() as u16;
        np.ref_power = r.i32();
        np.ref_tonality = r.i32();
        np.tns_gain_threshold = r.i32();
        np.tns_pns_gain_threshold = r.i32();
        np.min_sfb_width = r.i32();
        for x in np.pow_dist_psd_curve.iter_mut() {
            *x = r.i16();
        }
        np.gap_fill_thr = r.i16();
    }
    np
}

fn get_pns_param_checked(
    np0: &NoiseParams,
    a: (i32, i32, i32, &[i32], i32, i32, i32),
) -> (i32, NoiseParams, i32) {
    let (br, sr, cnt, off, use_pns, ch, lc) = a;
    let (mut nc, mut nr) = (*np0, *np0);
    let (mut uc, mut ur) = (use_pns, use_pns);
    let ec = unsafe {
        c::cref_pnsparam_GetPnsParam(&mut nc as *mut NoiseParams as *mut c_void, br, sr, cnt, off.as_ptr(), &mut uc, ch, lc)
    };
    let er = fdk_aac_enc_get_pns_param(&mut nr, br, sr, cnt, off, &mut ur, ch, lc);
    let w = format!("br {br} sr {sr} cnt {cnt} usePns {use_pns} ch {ch} lc {lc}");
    assert_eq!(er, ec, "{w}: return value");
    assert_eq!(ur, uc, "{w}: usePns");
    assert_eq!(nr, nc, "{w}: NOISEPARAMS");
    (er, nr, ur)
}

#[test]
fn get_pns_param() {
    let mut r = Rng::new(0x7a51_0002);
    let mut active = 0;
    for &sr in &RATES {
        for &(gl, fb) in &GRANULES {
            for &bt in block_types(gl) {
                let (_, p) = init_psy_checked(64000, sr, sr / 2, bt, gl, 0, 0, fb);
                for _ in 0..60 {
                    let br = match r.range(0, 1) {
                        0 => [12000, 16000, 24000, 28000, 32000, 40000, 48000, 56000, 64000][r.range(0, 8) as usize],
                        _ => r.range(0, 999_999),
                    };
                    let use_pns = [1, 1, 1, 0, -1, 2][r.range(0, 5) as usize];
                    let ch = r.range(1, 2);
                    let lc = if r.range(0, 3) == 0 { r.range(0, 1) } else { (fb == 0) as i32 };
                    let np0 = random_np(&mut r);
                    let (_, _, u) = get_pns_param_checked(&np0, (br, sr, p.sfb_cnt, &p.sfb_offset, use_pns, ch, lc));
                    active += (u > 0) as i32;
                }
            }
        }
    }
    assert!(active > 1000, "{active}");
    // sfbCnt values whose reads the port reproduces from the C layout of NOISEPARAMS
    for cnt in [0, 1, 2, 59, 60, 61] {
        let off: Vec<i32> = (0..64).map(|i| i * 16).collect();
        for _ in 0..200 {
            let np0 = random_np(&mut r);
            let br = r.range(0, 999_999);
            let sr = RATES[r.range(0, 11) as usize];
            get_pns_param_checked(&np0, (br, sr, cnt, &off, 1, r.range(1, 2), r.range(0, 1)));
        }
    }
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The encoder C ABI against the straight port: the same call sequences through
//! `fdk_aac::enc` and `fdk_aac_rs_aacenc::aacenc_lib` give the same results, including the NULL
//! paths and `LIB_INFO` fields the library does not write. (`scripts/capi_test.sh` compares the
//! C ABI with the reference library.)

use core::ffi::{c_char, c_void, CStr};
use core::ptr::{null, null_mut};
use fdk_aac::enc::*;
use fdk_aac::lib_info::LIB_INFO;
use fdk_aac_rs_aacenc::aacenc_lib as lib;
use fdk_aac_rs_fdk::audio::{fdk_init_lib_info, LibInfo, FDK_MODULE_LAST};

#[test]
fn null_arguments() {
    unsafe {
        assert_eq!(aacEncOpen(null_mut(), 0, 0), lib::AACENC_INVALID_HANDLE);
        assert_eq!(aacEncClose(null_mut()), lib::AACENC_INVALID_HANDLE);
        let mut h: HANDLE_AACENCODER = null_mut();
        assert_eq!(aacEncClose(&mut h), lib::AACENC_OK);
        assert_eq!(
            aacEncEncode(null_mut(), null(), null(), null(), null_mut()),
            lib::AACENC_INVALID_HANDLE
        );
        let mut info = AACENC_InfoStruct::default();
        assert_eq!(aacEncInfo(null_mut(), &mut info), lib::AACENC_INVALID_HANDLE);
        assert_eq!(aacEncoder_SetParam(null_mut(), lib::AACENC_AOT, 2), lib::AACENC_INVALID_HANDLE);
        assert_eq!(aacEncoder_GetParam(null_mut(), lib::AACENC_AOT), 0);
        assert_eq!(aacEncGetLibInfo(null_mut()), lib::AACENC_INVALID_HANDLE);

        // A failing open leaves the handle unchanged.
        let mut h = 0x1234usize as HANDLE_AACENCODER;
        assert_eq!(aacEncOpen(&mut h, 0, 9), lib::AACENC_INVALID_CONFIG);
        assert_eq!(h as usize, 0x1234);

        assert_eq!(aacEncOpen(&mut h, 0, 2), lib::AACENC_OK);
        assert!(!h.is_null());
        assert_eq!(aacEncInfo(h, null_mut()), lib::AACENC_INVALID_HANDLE);
        // Only inargs given: AACENC_UNSUPPORTED_PARAMETER after the initialization.
        let inargs = AACENC_InArgs::default();
        let mut s = None;
        assert_eq!(lib::aac_enc_open(Some(&mut s), 0, 2), lib::AACENC_OK);
        let e = lib::aac_enc_encode(s.as_deref_mut(), None, None, Some(&inargs), None);
        assert_eq!(aacEncEncode(h, null(), null(), &inargs, null_mut()), e);
        assert_eq!(aacEncoder_GetParam(h, lib::AACENC_CONTROL_STATE), {
            lib::aac_encoder_get_param(s.as_deref(), lib::AACENC_CONTROL_STATE)
        });
        assert_eq!(aacEncClose(&mut h), lib::AACENC_OK);
        assert!(h.is_null());
        lib::aac_enc_close(Some(&mut s));
    }
}

/// Encodes a signal through the C ABI and the straight port with the same descriptors.
#[test]
fn encode_sequence_matches_the_straight_port() {
    let params: [(i32, u32); 6] = [
        (lib::AACENC_AOT, 5),
        (lib::AACENC_SAMPLERATE, 44_100),
        (lib::AACENC_CHANNELMODE, 2),
        (lib::AACENC_BITRATE, 64_000),
        (lib::AACENC_TRANSMUX, 10),
        (lib::AACENC_SAMPLERATE, 12_345),
    ];
    unsafe {
        let mut h: HANDLE_AACENCODER = null_mut();
        let mut s = None;
        assert_eq!(aacEncOpen(&mut h, 0, 0), lib::aac_enc_open(Some(&mut s), 0, 0));
        for (p, v) in params {
            assert_eq!(
                aacEncoder_SetParam(h, p, v),
                lib::aac_encoder_set_param(s.as_deref_mut(), p, v)
            );
        }
        assert_eq!(
            aacEncEncode(h, null(), null(), null(), null_mut()),
            lib::aac_enc_encode(s.as_deref_mut(), None, None, None, None)
        );
        let (mut ia, mut ib) = (AACENC_InfoStruct::default(), AACENC_InfoStruct::default());
        assert_eq!(aacEncInfo(h, &mut ia), lib::aac_enc_info(s.as_deref_mut(), Some(&mut ib)));
        assert_eq!(ia, ib);

        let mut pcm: Vec<i16> =
            (0..2 * 44_100).map(|i| ((i * 7919) % 20011 - 10005) as i16).collect();
        let (mut out_a, mut out_b) = (vec![0u8; 4096], vec![0u8; 4096]);
        let mut pos = 0usize;
        for k in 0.. {
            let flush = pos >= pcm.len();
            let n = if flush { 0 } else { (pcm.len() - pos).min(1500) };
            let mut in_ptr = pcm.as_mut_ptr().add(pos) as *mut c_void;
            let mut in_id = lib::IN_AUDIO_DATA;
            let mut in_size = (n * 2) as i32;
            let mut in_el = 2;
            let in_desc = AACENC_BufDesc {
                num_bufs: 1,
                bufs: &mut in_ptr,
                buffer_identifiers: &mut in_id,
                buf_sizes: &mut in_size,
                buf_el_sizes: &mut in_el,
            };
            let mut results = Vec::new();
            for out in [&mut out_a, &mut out_b] {
                let mut out_ptr = out.as_mut_ptr() as *mut c_void;
                let mut out_id = lib::OUT_BITSTREAM_DATA;
                let mut out_size = out.len() as i32;
                let mut out_el = 1;
                let out_desc = AACENC_BufDesc {
                    num_bufs: 1,
                    bufs: &mut out_ptr,
                    buffer_identifiers: &mut out_id,
                    buf_sizes: &mut out_size,
                    buf_el_sizes: &mut out_el,
                };
                let in_args = AACENC_InArgs {
                    num_in_samples: if flush { -1 } else { n as i32 },
                    num_anc_bytes: 0,
                };
                let mut out_args = AACENC_OutArgs::default();
                let e = if results.is_empty() {
                    aacEncEncode(h, &in_desc, &out_desc, &in_args, &mut out_args)
                } else {
                    lib::aac_enc_encode(
                        s.as_deref_mut(),
                        Some(&in_desc),
                        Some(&out_desc),
                        Some(&in_args),
                        Some(&mut out_args),
                    )
                };
                results.push((e, out_args));
            }
            assert_eq!(results[0], results[1], "call {k}");
            assert_eq!(out_a, out_b, "call {k}");
            let (e, oa) = results[0];
            if e == lib::AACENC_ENCODE_EOF {
                break;
            }
            assert_eq!(e, lib::AACENC_OK);
            pos += oa.num_in_samples as usize;
        }
        for p in [0x100, 0x101, 0x102, 0x103, 0x104, 0x106, 0x108, 0x300, 0x302, 0xFF00, 7] {
            assert_eq!(aacEncoder_GetParam(h, p), lib::aac_encoder_get_param(s.as_deref(), p));
        }
        assert_eq!(aacEncClose(&mut h), lib::aac_enc_close(Some(&mut s)));
    }
}

fn c_str(p: *const c_char) -> String {
    assert!(!p.is_null());
    unsafe { CStr::from_ptr(p) }.to_str().expect("utf-8").to_string()
}

#[test]
fn lib_info_writes_what_the_library_writes() {
    const N: usize = FDK_MODULE_LAST as usize;
    static UNTOUCHED: &CStr = c"untouched";
    let fill = LIB_INFO {
        title: UNTOUCHED.as_ptr(),
        build_date: UNTOUCHED.as_ptr(),
        build_time: UNTOUCHED.as_ptr(),
        module_id: 0,
        version: 0x5a5a5a5a,
        flags: 0xdeadbeef,
        versionStr: [0x41; 32],
    };
    let mut c = [fill; N];
    assert_eq!(unsafe { aacEncGetLibInfo(c.as_mut_ptr()) }, lib::AACENC_OK);

    let mut r = [LibInfo::default(); N];
    fdk_init_lib_info(&mut r);
    for e in r.iter_mut() {
        e.version = 0x5a5a5a5a;
        e.flags = 0xdeadbeef;
        e.version_str = [0x41; 32];
    }
    assert_eq!(lib::aac_enc_get_lib_info(Some(&mut r)), lib::AACENC_OK);
    let mut filled = 0;
    for (a, b) in c.iter().zip(r.iter()) {
        assert_eq!(a.module_id, b.module_id);
        assert_eq!(a.version, b.version);
        assert_eq!(a.flags, b.flags);
        assert_eq!(a.versionStr.map(|x| x as u8), b.version_str);
        if b.module_id != 0 {
            filled += 1;
            assert_eq!(c_str(a.title), b.title);
            assert_eq!(c_str(a.build_date), b.build_date);
            assert_eq!(c_str(a.build_time), b.build_time);
        } else {
            assert_eq!(a.title, UNTOUCHED.as_ptr());
            assert_eq!(a.build_date, UNTOUCHED.as_ptr());
        }
    }
    assert!(filled >= 5, "{filled} modules");
    // The same string has the same address on every call.
    let mut d = [fill; N];
    assert_eq!(unsafe { aacEncGetLibInfo(d.as_mut_ptr()) }, lib::AACENC_OK);
    for (a, b) in c.iter().zip(d.iter()) {
        assert_eq!(a.title, b.title);
    }
}

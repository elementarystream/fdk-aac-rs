// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The decoder C ABI against the straight port: the same call sequences through `fdk_aac::dec`
//! and `fdk_aac_rs_aacdec::aacdecoder_lib` give the same results (return codes, bytes valid,
//! the whole output buffer, every `CStreamInfo` field, ancillary data), including the NULL
//! paths, the `CStreamInfo` pointer that stays valid, and `LIB_INFO` fields the library does not
//! write. The streams are made with the encoder. (`scripts/capi_test.sh` compares the C ABI with
//! the reference library.)

use core::ffi::{c_char, c_int, c_uchar, c_uint, c_void, CStr};
use core::ptr::{null, null_mut};
use fdk_aac::dec::*;
use fdk_aac::lib_info::LIB_INFO;
use fdk_aac_rs_aacdec::aacdecoder::AacDecoderInstance;
use fdk_aac_rs_aacdec::aacdecoder_lib as lib;
use fdk_aac_rs_aacenc::aacenc_lib as enc;
use fdk_aac_rs_fdk::audio::{fdk_init_lib_info, LibInfo, FDK_MODULE_LAST};

// HANDLE_AACDECODER points to an opaque object, as in C.
#[allow(improper_ctypes)]
extern "C" {
    // Defined in assembly (global in the rlib's object, as in the static library).
    fn aacDecoder_RawISOBMFFData(
        self_: HANDLE_AACDECODER,
        buffer: *mut c_uchar,
        length: c_uint,
    ) -> AAC_DECODER_ERROR;
}

/// Encodes a test signal with the straight-port encoder: the access units and the
/// configuration (`confBuf`).
fn encode(
    params: &[(i32, u32)],
    channels: usize,
    frames: usize,
    anc: bool,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut h = None;
    assert_eq!(enc::aac_enc_open(Some(&mut h), 0, 0), enc::AACENC_OK);
    for &(p, v) in params {
        assert_eq!(enc::aac_encoder_set_param(h.as_deref_mut(), p, v), enc::AACENC_OK, "{p:x}");
    }
    assert_eq!(unsafe { enc::aac_enc_encode(h.as_deref_mut(), None, None, None, None) }, 0);
    let mut info = enc::AacencInfoStruct::default();
    assert_eq!(enc::aac_enc_info(h.as_deref_mut(), Some(&mut info)), enc::AACENC_OK);
    let mut pcm: Vec<i16> = (0..frames * channels)
        .map(|i| (((i * 7919 + i / channels * 31) % 20011) as i32 - 10005) as i16)
        .collect();
    let mut out = vec![0u8; 8192];
    let mut aus = Vec::new();
    let mut pos = 0usize;
    let mut k = 0usize;
    loop {
        let flush = pos >= pcm.len();
        let n = if flush { 0 } else { (pcm.len() - pos).min(1024 * channels) };
        let mut a: Vec<u8> = (0..(k % 4) * 5).map(|i| (0x30 + i + k) as u8).collect();
        let mut bufs: Vec<*mut c_void> = vec![unsafe { pcm.as_mut_ptr().add(pos) } as *mut c_void];
        let mut ids = vec![enc::IN_AUDIO_DATA];
        let mut sizes = vec![(n * 2) as i32];
        let mut el = vec![2];
        if anc && !flush && !a.is_empty() {
            bufs.push(a.as_mut_ptr() as *mut c_void);
            ids.push(enc::IN_ANCILLRY_DATA);
            sizes.push(a.len() as i32);
            el.push(1);
        }
        let in_desc = enc::AacencBufDesc {
            num_bufs: bufs.len() as i32,
            bufs: bufs.as_mut_ptr(),
            buffer_identifiers: ids.as_mut_ptr(),
            buf_sizes: sizes.as_mut_ptr(),
            buf_el_sizes: el.as_mut_ptr(),
        };
        let mut out_ptr = out.as_mut_ptr() as *mut c_void;
        let mut out_id = enc::OUT_BITSTREAM_DATA;
        let mut out_size = out.len() as i32;
        let mut out_el = 1;
        let out_desc = enc::AacencBufDesc {
            num_bufs: 1,
            bufs: &mut out_ptr,
            buffer_identifiers: &mut out_id,
            buf_sizes: &mut out_size,
            buf_el_sizes: &mut out_el,
        };
        let in_args = enc::AacencInArgs {
            num_in_samples: if flush { -1 } else { n as i32 },
            num_anc_bytes: if anc && !flush { a.len() as i32 } else { 0 },
        };
        let mut out_args = enc::AacencOutArgs::default();
        let e = unsafe {
            enc::aac_enc_encode(
                h.as_deref_mut(),
                Some(&in_desc),
                Some(&out_desc),
                Some(&in_args),
                Some(&mut out_args),
            )
        };
        if e == enc::AACENC_ENCODE_EOF {
            break;
        }
        assert_eq!(e, enc::AACENC_OK);
        if out_args.num_out_bytes > 0 {
            aus.push(out[..out_args.num_out_bytes as usize].to_vec());
        }
        pos += out_args.num_in_samples as usize;
        k += 1;
    }
    enc::aac_enc_close(Some(&mut h));
    (aus, info.conf_buf[..info.conf_size as usize].to_vec())
}

/// The fields of a C `CStreamInfo` (with the arrays it points to) as the straight port's type.
fn from_c(si: &CStreamInfo) -> lib::CStreamInfo {
    let mut ct = [0; 8];
    let mut ci = [0; 8];
    unsafe {
        ct.copy_from_slice(core::slice::from_raw_parts(si.pChannelType, 8));
        ci.copy_from_slice(core::slice::from_raw_parts(si.pChannelIndices, 8));
    }
    lib::CStreamInfo {
        sample_rate: si.sampleRate,
        frame_size: si.frameSize,
        num_channels: si.numChannels,
        p_channel_type: ct,
        p_channel_indices: ci,
        aac_sample_rate: si.aacSampleRate,
        profile: si.profile,
        aot: si.aot,
        channel_config: si.channelConfig,
        bit_rate: si.bitRate,
        aac_samples_per_frame: si.aacSamplesPerFrame,
        aac_num_channels: si.aacNumChannels,
        ext_aot: si.extAot,
        ext_sampling_rate: si.extSamplingRate,
        output_delay: si.outputDelay,
        flags: si.flags,
        ep_config: si.epConfig,
        num_lost_access_units: si.numLostAccessUnits,
        num_total_bytes: si.numTotalBytes,
        num_bad_bytes: si.numBadBytes,
        num_total_access_units: si.numTotalAccessUnits,
        num_bad_access_units: si.numBadAccessUnits,
        drc_prog_ref_lev: si.drcProgRefLev,
        drc_pres_mode: si.drcPresMode,
        output_loudness: si.outputLoudness,
    }
}

/// A C ABI decoder and a straight-port decoder driven with the same calls.
struct Pair {
    h: HANDLE_AACDECODER,
    s: Option<Box<AacDecoderInstance>>,
    /// The pointer of the first `aacDecoder_GetStreamInfo()`, read after every call.
    si: *const CStreamInfo,
    anc_c: Vec<u8>,
    anc_s: usize,
}

impl Pair {
    fn open(transport: i32) -> Pair {
        let h = aacDecoder_Open(transport, 1);
        let s = lib::aac_decoder_open(transport, 1);
        assert!(!h.is_null() && s.is_some());
        let si = unsafe { aacDecoder_GetStreamInfo(h) };
        let p = Pair { h, s, si, anc_c: Vec::new(), anc_s: 0 };
        p.check("open");
        p
    }

    /// The stream information of both sides, through the first pointer.
    fn check(&self, what: &str) {
        unsafe {
            assert_eq!(aacDecoder_GetStreamInfo(self.h) as *const CStreamInfo, self.si);
            let s = lib::aac_decoder_get_stream_info(self.s.as_deref()).unwrap();
            assert_eq!(&from_c(&*self.si), s, "{what}: CStreamInfo");
        }
    }

    fn config(&mut self, asc: &[u8]) {
        let mut a = asc.to_vec();
        let mut conf = [a.as_mut_ptr()];
        let len = [a.len() as c_uint];
        let e1 = unsafe { aacDecoder_ConfigRaw(self.h, conf.as_mut_ptr(), len.as_ptr()) };
        let e2 = lib::aac_decoder_config_raw(self.s.as_deref_mut().unwrap(), &[asc], &[len[0]]);
        assert_eq!(e1, e2, "config");
        self.check("config");
    }

    fn set(&mut self, p: c_int, v: c_int) {
        let e1 = unsafe { aacDecoder_SetParam(self.h, p, v) };
        let e2 = lib::aac_decoder_set_param(self.s.as_deref_mut(), p, v);
        assert_eq!(e1, e2, "set {p:x} {v}");
        self.check("set");
    }

    fn anc_init(&mut self, size: usize) {
        self.anc_c = vec![0u8; size];
        self.anc_s = size;
        let e1 = unsafe { aacDecoder_AncDataInit(self.h, self.anc_c.as_mut_ptr(), size as c_int) };
        let e2 = lib::aac_decoder_anc_data_init(
            self.s.as_deref_mut().unwrap(),
            Some(vec![0u8; size].into_boxed_slice()),
            size as i32,
        );
        assert_eq!(e1, e2, "anc init");
    }

    /// Fills `data`, returns the bytes consumed.
    fn fill(&mut self, data: &[u8]) -> usize {
        let (mut f1, mut f2) = (0xa5a5a5a5u32, 0u32);
        let e1 = unsafe { aacDecoder_GetFreeBytes(self.h, &mut f1) };
        let e2 = lib::aac_decoder_get_free_bytes(self.s.as_deref_mut(), &mut f2);
        assert_eq!((e1, f1), (e2, f2), "free bytes");
        let mut d = data.to_vec();
        let mut bufs = [d.as_mut_ptr()];
        let size = [d.len() as c_uint];
        let mut v1 = [d.len() as c_uint];
        let mut v2 = [d.len() as u32];
        let e1 =
            unsafe { aacDecoder_Fill(self.h, bufs.as_mut_ptr(), size.as_ptr(), v1.as_mut_ptr()) };
        let e2 = lib::aac_decoder_fill(self.s.as_deref_mut().unwrap(), &[data], &size, &mut v2);
        assert_eq!((e1, v1), (e2, v2), "fill");
        self.check("fill");
        data.len() - v1[0] as usize
    }

    /// Decodes one frame into `size` samples; returns the code.
    fn decode(&mut self, size: usize, flags: c_uint, what: &str) -> i32 {
        let mut p1 = vec![0x1234i16; size];
        let mut p2 = vec![0x1234i16; size];
        let e1 = unsafe { aacDecoder_DecodeFrame(self.h, p1.as_mut_ptr(), size as c_int, flags) };
        let e2 = lib::aac_decoder_decode_frame(self.s.as_deref_mut(), &mut p2, size as i32, flags);
        assert_eq!(e1, e2, "{what}: decode");
        assert!(p1 == p2, "{what}: PCM");
        self.check(what);
        if self.anc_s > 0 && lib::is_output_valid(e1) != 0 {
            for i in -1..9 {
                let (mut ptr, mut n) = (core::ptr::dangling_mut::<c_uchar>(), -7);
                let a1 = unsafe { aacDecoder_AncDataGet(self.h, i, &mut ptr, &mut n) };
                let d = self.s.as_deref().unwrap();
                let (mut off, mut m) = (None, 0);
                let a2 = lib::aac_decoder_anc_data_get(d, i, &mut off, &mut m);
                assert_eq!((a1, n), (a2, m), "{what}: anc {i}");
                match off {
                    None => assert!(ptr.is_null()),
                    Some(o) => {
                        assert_eq!(ptr, self.anc_c.as_mut_ptr().wrapping_add(o));
                        let b = lib::aac_decoder_anc_data_buffer(d).unwrap();
                        assert_eq!(&self.anc_c[o..o + m as usize], &b[o..o + m as usize]);
                    }
                }
            }
        }
        e1
    }

    fn close(self) {
        unsafe { aacDecoder_Close(self.h) };
        lib::aac_decoder_close(self.s);
    }
}

#[test]
fn null_arguments() {
    unsafe {
        let mut pcm = [7i16; 16];
        assert_eq!(
            aacDecoder_DecodeFrame(null_mut(), pcm.as_mut_ptr(), 16, 0),
            lib::AAC_DEC_INVALID_HANDLE
        );
        assert_eq!(pcm, [7; 16]);
        assert_eq!(
            aacDecoder_SetParam(null_mut(), lib::AAC_PCM_LIMITER_ENABLE, 0),
            lib::AAC_DEC_INVALID_HANDLE
        );
        let mut free = 99;
        assert_eq!(aacDecoder_GetFreeBytes(null_mut(), &mut free), lib::AAC_DEC_INVALID_HANDLE);
        assert_eq!(free, 0);
        assert_eq!(aacDecoder_GetFreeBytes(null_mut(), null_mut()), lib::AAC_DEC_INVALID_HANDLE);
        assert!(aacDecoder_GetStreamInfo(null_mut()).is_null());
        aacDecoder_Close(null_mut());
        assert_eq!(aacDecoder_GetLibInfo(null_mut()), -1);
        // Where C does not define the result of a NULL argument, the port returns an error.
        assert_eq!(
            aacDecoder_Fill(null_mut(), null_mut(), null(), null_mut()),
            lib::AAC_DEC_INVALID_HANDLE
        );
        assert_eq!(
            aacDecoder_ConfigRaw(null_mut(), null_mut(), null()),
            lib::AAC_DEC_INVALID_HANDLE
        );
        assert_eq!(aacDecoder_AncDataInit(null_mut(), null_mut(), 0), lib::AAC_DEC_INVALID_HANDLE);
        assert_eq!(
            aacDecoder_AncDataGet(null_mut(), 0, null_mut(), null_mut()),
            lib::AAC_DEC_INVALID_HANDLE
        );
        assert_eq!(
            aacDecoder_RawISOBMFFData(null_mut(), null_mut(), 0),
            lib::AAC_DEC_INVALID_HANDLE
        );

        // Unsupported transport types: NULL, as the straight port.
        for t in [-1, 3, 4, 5, 8, 9, 11, 13, 99] {
            assert_eq!(
                aacDecoder_Open(t, 1).is_null(),
                lib::aac_decoder_open(t, 1).is_none(),
                "{t}"
            );
        }

        let h = aacDecoder_Open(2, 1);
        assert!(!h.is_null());
        assert_eq!(aacDecoder_Fill(h, null_mut(), null(), null_mut()), lib::AAC_DEC_UNKNOWN);
        assert_eq!(aacDecoder_ConfigRaw(h, null_mut(), null()), lib::AAC_DEC_UNKNOWN);
        let mut free = 0;
        assert_eq!(aacDecoder_GetFreeBytes(h, null_mut()), lib::AAC_DEC_OK);
        assert_eq!(aacDecoder_GetFreeBytes(h, &mut free), lib::AAC_DEC_OK);
        assert!(free > 0);
        assert_eq!(aacDecoder_AncDataGet(h, 0, null_mut(), null_mut()), lib::AAC_DEC_OK);
        let (mut p, mut n) = (0x10usize as *mut c_uchar, 5);
        assert_eq!(aacDecoder_AncDataGet(h, 0, &mut p, &mut n), lib::AAC_DEC_OK);
        assert!(p.is_null() && n == 0);
        assert_eq!(aacDecoder_AncDataInit(h, null_mut(), -1), lib::AAC_DEC_ANC_DATA_ERROR);
        // No input: no valid output; the whole buffer (timeDataSize samples) is cleared.
        let mut pcm = [7i16; 16];
        assert_eq!(
            aacDecoder_DecodeFrame(h, pcm.as_mut_ptr(), 12, 0),
            lib::AAC_DEC_NOT_ENOUGH_BITS
        );
        assert_eq!(&pcm[..12], &[0; 12]);
        assert_eq!(&pcm[12..], &[7; 4]);
        assert_eq!(aacDecoder_DecodeFrame(h, null_mut(), 12, 0), lib::AAC_DEC_NOT_ENOUGH_BITS);
        assert_eq!(
            aacDecoder_DecodeFrame(h, pcm.as_mut_ptr(), -5, 0),
            lib::AAC_DEC_NOT_ENOUGH_BITS
        );
        assert_eq!(&pcm[12..], &[7; 4]);
        aacDecoder_Close(h);
    }
}

#[test]
fn adts_sequence_matches_the_straight_port() {
    let (aus, _) = encode(
        &[
            (enc::AACENC_AOT, 5),
            (enc::AACENC_SAMPLERATE, 44_100),
            (enc::AACENC_CHANNELMODE, 2),
            (enc::AACENC_BITRATE, 64_000),
            (enc::AACENC_TRANSMUX, 2),
            (enc::AACENC_METADATA_MODE, 3),
        ],
        2,
        44_100,
        true,
    );
    let bytes = aus.concat();
    let mut p = Pair::open(2);
    p.set(lib::AAC_PCM_LIMITER_ENABLE, 0);
    p.set(lib::AAC_DRC_REFERENCE_LEVEL, 1000);
    p.set(lib::AAC_TPDEC_CLEAR_BUFFER, 1);
    p.anc_init(48);
    let chunks = [700usize, 3000, 13, 1500];
    let (mut pos, mut c, mut k, mut ok) = (0usize, 0usize, 0u32, 0);
    while pos < bytes.len() {
        let n = chunks[c % chunks.len()].min(bytes.len() - pos);
        c += 1;
        pos += p.fill(&bytes[pos..pos + n]);
        loop {
            let flags = match k {
                4 => lib::AACDEC_CONCEAL,
                8 => lib::AACDEC_INTR,
                12 => lib::AACDEC_CLRHIST,
                _ => 0,
            };
            let size = if k == 15 { 1000 } else { 4096 };
            let e = p.decode(size, flags, &format!("frame {k}"));
            k += 1;
            if e == 0 {
                ok += 1;
            }
            if lib::is_output_valid(e) == 0 {
                break;
            }
        }
    }
    for _ in 0..3 {
        p.decode(4096, lib::AACDEC_FLUSH, "flush");
    }
    assert!(ok > 15, "{ok} frames");
    p.set(lib::AAC_TPDEC_CLEAR_BUFFER, 1);
    p.close();
}

#[test]
fn raw_sequence_matches_the_straight_port() {
    for params in [
        // ELD with SBR, ELDv2 (MPEG Surround 212), LD.
        &[
            (enc::AACENC_AOT, 39),
            (enc::AACENC_SBR_MODE, 1),
            (enc::AACENC_SAMPLERATE, 48_000),
            (enc::AACENC_CHANNELMODE, 2),
            (enc::AACENC_BITRATE, 64_000),
            (enc::AACENC_TRANSMUX, 0),
        ][..],
        &[
            (enc::AACENC_AOT, 39),
            (enc::AACENC_SAMPLERATE, 48_000),
            (enc::AACENC_CHANNELMODE, 128),
            (enc::AACENC_BITRATE, 48_000),
            (enc::AACENC_TRANSMUX, 0),
        ][..],
        &[
            (enc::AACENC_AOT, 23),
            (enc::AACENC_SAMPLERATE, 32_000),
            (enc::AACENC_CHANNELMODE, 1),
            (enc::AACENC_BITRATE, 48_000),
            (enc::AACENC_TRANSMUX, 0),
        ][..],
    ] {
        let channels = if params.contains(&(enc::AACENC_CHANNELMODE, 1)) { 1 } else { 2 };
        let (aus, asc) = encode(params, channels, 24_000, false);
        let mut p = Pair::open(0);
        p.config(&asc);
        p.set(lib::AAC_PCM_MIN_OUTPUT_CHANNELS, 2);
        for (k, au) in aus.iter().enumerate() {
            let mut au = au.clone();
            if k % 7 == 6 {
                for b in au.iter_mut().skip(2).step_by(3) {
                    *b ^= 0x55;
                }
            }
            p.fill(&au);
            p.decode(2048, if k == 10 { lib::AACDEC_INTR } else { 0 }, &format!("au {k}"));
        }
        p.decode(2048, lib::AACDEC_FLUSH, "flush");
        // A second configuration while decoding.
        p.config(&asc);
        p.close();
    }
}

#[test]
fn raw_isobmff_data() {
    unsafe {
        let h = aacDecoder_Open(0, 1);
        let mut s = lib::aac_decoder_open(0, 1);
        let boxes: [&[u8]; 4] = [
            &[0, 0, 0, 8],
            &[0, 0, 0, 12, b'f', b'r', b'e', b'e', 1, 2, 3, 4],
            &[0, 0, 0, 12, b'l', b'u', b'd', b't', 0, 0, 0, 0],
            &[0, 0, 0, 16, b'd', b'm', b'i', b'x', 0, 0, 0, 0, 1, 2, 3, 4],
        ];
        for b in boxes {
            let mut d = b.to_vec();
            let e1 = aacDecoder_RawISOBMFFData(h, d.as_mut_ptr(), d.len() as c_uint);
            let e2 =
                lib::aac_decoder_raw_isobmff_data(s.as_deref_mut().unwrap(), b, b.len() as u32);
            assert_eq!(e1, e2, "{b:?}");
        }
        assert_eq!(aacDecoder_RawISOBMFFData(h, null_mut(), 8), lib::AAC_DEC_UNKNOWN);
        aacDecoder_Close(h);
        lib::aac_decoder_close(s);
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
    assert_eq!(unsafe { aacDecoder_GetLibInfo(c.as_mut_ptr()) }, 0);

    let mut r = [LibInfo::default(); N];
    fdk_init_lib_info(&mut r);
    for e in r.iter_mut() {
        e.version = 0x5a5a5a5a;
        e.flags = 0xdeadbeef;
        e.version_str = [0x41; 32];
    }
    assert_eq!(lib::aac_decoder_get_lib_info(Some(&mut r)), 0);
    let mut untouched_flags = 0;
    for (a, b) in c.iter().zip(r.iter()) {
        assert_eq!(a.module_id, b.module_id);
        assert_eq!(a.version, b.version);
        assert_eq!(a.flags, b.flags);
        assert_eq!(a.versionStr.map(|x| x as u8), b.version_str);
        if b.module_id != 0 {
            assert_eq!(c_str(a.title), b.title);
            assert_eq!(c_str(a.build_date), b.build_date);
            if a.flags == 0xdeadbeef {
                untouched_flags += 1;
            }
        } else {
            assert_eq!(a.title, UNTOUCHED.as_ptr());
        }
    }
    // The DRC decoder's entry keeps the caller's flags (PORTING.md, "Definition of bit-exact").
    assert_eq!(untouched_flags, 1);
    // No free entry.
    let mut full = [LIB_INFO { module_id: 1, ..fill }; N];
    assert_eq!(unsafe { aacDecoder_GetLibInfo(full.as_mut_ptr()) }, -1);
}

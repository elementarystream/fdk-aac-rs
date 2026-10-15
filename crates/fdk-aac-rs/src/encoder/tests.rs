// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The safe API against the straight port: every call sequence of the safe API is repeated with
//! the straight-port functions and buffer descriptors built as upstream's `aac-enc.c` builds
//! them, and every result is compared (return codes, output bytes, `AACENC_OutArgs`,
//! `AACENC_InfoStruct`, all parameters).

use super::*;
use fdk_aac_rs_fdk::audio::{fdk_init_lib_info, FDK_MODULE_LAST};

/// Deterministic test signal: a sine per channel plus noise (xorshift).
fn signal(frames: usize, channels: usize, seed: u32) -> Vec<i16> {
    let mut x = seed | 1;
    let mut v = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        for c in 0..channels {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let t = i as f64 * (440.0 + 110.0 * c as f64) / 48_000.0;
            let s = 9000.0 * (2.0 * core::f64::consts::PI * t).sin() + ((x >> 20) as f64 - 2048.0);
            v.push(s as i16);
        }
    }
    v
}

/// The straight-port side: `HANDLE_AACENCODER` and the calls of `aac-enc.c`.
struct Straight {
    h: Option<Box<AacEncoder>>,
}

impl Straight {
    fn open(modules: u32, max_channels: u32) -> (i32, Straight) {
        let mut h = None;
        let e = lib::aac_enc_open(Some(&mut h), modules, max_channels);
        (e, Straight { h })
    }

    fn set(&mut self, id: u32, value: u32) -> i32 {
        lib::aac_encoder_set_param(self.h.as_deref_mut(), id as i32, value)
    }

    fn get(&self, id: u32) -> u32 {
        lib::aac_encoder_get_param(self.h.as_deref(), id as i32)
    }

    fn init(&mut self) -> i32 {
        unsafe { lib::aac_enc_encode(self.h.as_deref_mut(), None, None, None, None) }
    }

    fn info(&mut self) -> (i32, AacencInfoStruct) {
        let mut i = AacencInfoStruct::default();
        let e = lib::aac_enc_info(self.h.as_deref_mut(), Some(&mut i));
        (e, i)
    }

    /// `aacEncEncode()` with the descriptors of `aac-enc.c`, plus ancillary data and metadata
    /// buffers when given. `num_in_samples` is -1 for a flush.
    fn encode(
        &mut self,
        pcm: &mut [i16],
        num_in_samples: i32,
        anc: &mut [u8],
        md: Option<&mut MetaData>,
        out: &mut [u8],
    ) -> (i32, AacencOutArgs) {
        let mut bufs: Vec<*mut c_void> = vec![pcm.as_mut_ptr() as *mut c_void];
        let mut ids = vec![lib::IN_AUDIO_DATA];
        let mut sizes = vec![(pcm.len() * 2) as i32];
        let mut el = vec![2];
        if !anc.is_empty() {
            bufs.push(anc.as_mut_ptr() as *mut c_void);
            ids.push(lib::IN_ANCILLRY_DATA);
            sizes.push(anc.len() as i32);
            el.push(1);
        }
        if let Some(md) = md {
            bufs.push(md as *mut MetaData as *mut c_void);
            ids.push(lib::IN_METADATA_SETUP);
            sizes.push(core::mem::size_of::<MetaData>() as i32);
            el.push(core::mem::size_of::<MetaData>() as i32);
        }
        let in_desc = AacencBufDesc {
            num_bufs: bufs.len() as i32,
            bufs: bufs.as_mut_ptr(),
            buffer_identifiers: ids.as_mut_ptr(),
            buf_sizes: sizes.as_mut_ptr(),
            buf_el_sizes: el.as_mut_ptr(),
        };
        let mut out_ptr = out.as_mut_ptr() as *mut c_void;
        let mut out_id = lib::OUT_BITSTREAM_DATA;
        let mut out_size = out.len() as i32;
        let mut out_el = 1;
        let out_desc = AacencBufDesc {
            num_bufs: 1,
            bufs: &mut out_ptr,
            buffer_identifiers: &mut out_id,
            buf_sizes: &mut out_size,
            buf_el_sizes: &mut out_el,
        };
        let in_args = AacencInArgs { num_in_samples, num_anc_bytes: anc.len() as i32 };
        let mut out_args = AacencOutArgs::default();
        let e = unsafe {
            lib::aac_enc_encode(
                self.h.as_deref_mut(),
                Some(&in_desc),
                Some(&out_desc),
                Some(&in_args),
                Some(&mut out_args),
            )
        };
        (e, out_args)
    }
}

impl Drop for Straight {
    fn drop(&mut self) {
        assert_eq!(lib::aac_enc_close(Some(&mut self.h)), lib::AACENC_OK);
    }
}

fn code<T>(r: &Result<T, EncoderError>) -> i32 {
    match r {
        Ok(_) => lib::AACENC_OK,
        Err(e) => e.code(),
    }
}

const ALL_IDS: [ParamId; 22] = [
    ParamId::Aot,
    ParamId::Bitrate,
    ParamId::BitrateMode,
    ParamId::SampleRate,
    ParamId::SbrMode,
    ParamId::GranuleLength,
    ParamId::ChannelMode,
    ParamId::ChannelOrder,
    ParamId::SbrRatio,
    ParamId::Afterburner,
    ParamId::Bandwidth,
    ParamId::PeakBitrate,
    ParamId::Transmux,
    ParamId::HeaderPeriod,
    ParamId::SignalingMode,
    ParamId::TpSubframes,
    ParamId::AudioMuxVersion,
    ParamId::Protection,
    ParamId::AncillaryBitrate,
    ParamId::MetadataMode,
    ParamId::ControlState,
    ParamId::None,
];

fn compare_params(enc: &Encoder, s: &Straight, what: &str) {
    for id in ALL_IDS {
        assert_eq!(enc.get(id), s.get(id as u32), "{what}: get {id:?}");
    }
    assert_eq!(enc.get_raw(0x1234), s.get(0x1234), "{what}: get unknown");
}

/// One configuration: parameters, input, chunking of the input, extras.
struct Case {
    name: &'static str,
    modules: Modules,
    max_channels: u32,
    params: Vec<Param>,
    channels: usize,
    frames: usize,
    /// Samples per channel of each `encode` call (cycled).
    chunks: &'static [usize],
    out_size: usize,
    anc: bool,
    metadata: bool,
}

impl Case {
    fn new(name: &'static str, channels: usize, params: Vec<Param>) -> Case {
        Case {
            name,
            modules: Modules::ALL,
            max_channels: 0,
            params,
            channels,
            frames: 48_000,
            chunks: &[1024, 700, 2048, 1],
            out_size: 8192,
            anc: false,
            metadata: false,
        }
    }
}

fn metadata(i: usize) -> MetaData {
    MetaData {
        drc_profile: lib::AACENC_METADATA_DRC_FILMSTANDARD,
        comp_profile: lib::AACENC_METADATA_DRC_MUSICLIGHT,
        drc_target_ref_level: -(((16 + (i % 8)) as i32) << 16),
        comp_target_ref_level: -(20 << 16),
        prog_ref_level_present: 1,
        prog_ref_level: -(((20 + (i % 4)) as i32) << 16),
        pce_mixdown_idx_present: 0,
        etsi_dmx_lvl_present: 1,
        center_mix_level: 2,
        surround_mix_level: 3,
        dolby_surround_mode: 0,
        drc_presentation_mode: 0,
        ..MetaData::default()
    }
}

/// Runs a case through both APIs and compares every result. Returns the number of output bytes.
fn run(case: &Case) -> usize {
    let what = case.name;
    let mut b = Encoder::builder().modules(case.modules).max_channels(case.max_channels);
    for p in &case.params {
        b = b.param(*p);
    }
    let built = b.build();

    let (e, mut s) = Straight::open(case.modules.bits(), case.max_channels);
    assert_eq!(e, lib::AACENC_OK, "{what}: open");
    let mut set_err = None;
    for p in &case.params {
        let e = s.set(p.id() as u32, p.value());
        if e != lib::AACENC_OK {
            set_err = Some((p.id() as u32, p.value(), e));
            break;
        }
    }
    let mut enc = match (built, set_err) {
        (Err(BuildError::SetParam { id, value, error }), Some((sid, sv, se))) => {
            assert_eq!((id, value, error.code()), (sid, sv, se), "{what}: set error");
            return 0;
        }
        (Ok(enc), None) => enc,
        (Err(BuildError::Initialize(e)), None) => {
            assert_eq!(e.code(), s.init(), "{what}: init error");
            return 0;
        }
        (r, se) => panic!("{what}: build {r:?} vs set {se:?}"),
    };
    assert_eq!(s.init(), lib::AACENC_OK, "{what}: init");
    compare_params(&enc, &s, what);

    let (e, si) = s.info();
    let i = enc.info();
    assert_eq!(code(&i), e, "{what}: info code");
    if let Ok(i) = i {
        assert_eq!(*i.raw(), si, "{what}: info");
        assert_eq!(i.config(), &si.conf_buf[..si.conf_size as usize]);
    }

    let mut pcm = signal(case.frames, case.channels, 0x1234_5678);
    let mut out_a = vec![0xa5u8; case.out_size];
    let mut out_b = vec![0x5au8; case.out_size];
    let mut total = 0;
    let mut pos = 0;
    let mut k = 0;
    while pos < pcm.len() {
        let n = (case.chunks[k % case.chunks.len()] * case.channels).min(pcm.len() - pos);
        let mut anc: Vec<u8> =
            if case.anc { (0..(k % 7) as u8 * 3).map(|x| x ^ 0x5c).collect() } else { vec![] };
        let md = metadata(k);
        let input = EncodeInput {
            pcm: &pcm[pos..pos + n],
            ancillary: &anc,
            metadata: if case.metadata { Some(&md) } else { None },
        };
        let ra = enc.encode_with(&input, &mut out_a);
        let mut md2 = md;
        let (e, oa) = s.encode(
            &mut pcm[pos..pos + n],
            n as i32,
            &mut anc,
            if case.metadata { Some(&mut md2) } else { None },
            &mut out_b,
        );
        assert_eq!(code(&ra), e, "{what}: encode {k} code");
        if let Ok(r) = ra {
            assert_eq!(*r.raw(), oa, "{what}: encode {k} out args");
            assert_eq!(r.out_bytes(), oa.num_out_bytes as usize);
            total += r.out_bytes();
            pos += r.in_samples();
            if r.in_samples() == 0 && r.out_bytes() == 0 {
                // A chunk shorter than the channel count consumes nothing; skip it.
                pos += n;
            }
        } else {
            pos += n;
        }
        assert_eq!(out_a, out_b, "{what}: encode {k} output buffer");
        k += 1;
        if k % 5 == 0 {
            compare_params(&enc, &s, what);
            let (e, si) = s.info();
            let i = enc.info();
            assert_eq!(code(&i), e);
            if let Ok(i) = i {
                assert_eq!(*i.raw(), si, "{what}: info {k}");
            }
        }
    }
    let mut empty: [i16; 0] = [];
    for f in 0.. {
        let ra = enc.flush(&mut out_a);
        let (e, oa) = s.encode(&mut empty, -1, &mut [], None, &mut out_b);
        assert_eq!(code(&ra), e, "{what}: flush {f} code");
        assert_eq!(out_a, out_b, "{what}: flush {f} output buffer");
        match ra {
            Ok(r) => {
                assert_eq!(*r.raw(), oa, "{what}: flush {f} out args");
                total += r.out_bytes();
            }
            Err(_) => break,
        }
        assert!(f < 100, "{what}: flush does not end");
    }
    // After the end of the flush.
    let ra = enc.flush(&mut out_a);
    let (e, _) = s.encode(&mut empty, -1, &mut [], None, &mut out_b);
    assert_eq!(code(&ra), e, "{what}: flush after EOF");
    compare_params(&enc, &s, what);
    total
}

#[test]
fn lc_stereo_adts_cbr() {
    let n = run(&Case::new(
        "lc stereo adts",
        2,
        vec![
            Param::Aot(Aot::AacLc),
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::ChannelOrder(ChannelOrder::Wave),
            Param::Bitrate(128_000),
            Param::Transmux(Transport::Adts),
            Param::Afterburner(true),
        ],
    ));
    assert!(n > 0);
}

#[test]
fn he_aac_loas_vbr() {
    let n = run(&Case::new(
        "he-aac loas vbr",
        2,
        vec![
            Param::Aot(Aot::HeAac),
            Param::SampleRate(44_100),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::BitrateMode(BitrateMode::VbrMedium),
            Param::Transmux(Transport::Loas),
            Param::SignalingMode(SignalingMode::ExplicitHierarchical),
            Param::AudioMuxVersion(1),
        ],
    ));
    assert!(n > 0);
}

#[test]
fn he_aac_v2_raw() {
    let n = run(&Case::new(
        "he-aac v2 raw",
        2,
        vec![
            Param::Aot(Aot::HeAacV2),
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Bitrate(32_000),
            Param::Transmux(Transport::Raw),
        ],
    ));
    assert!(n > 0);
}

#[test]
fn ld_mono_latm() {
    let mut c = Case::new(
        "ld mono latm",
        1,
        vec![
            Param::Aot(Aot::AacLd),
            Param::SampleRate(32_000),
            Param::ChannelMode(ChannelMode::Mono),
            Param::GranuleLength(480),
            Param::Bitrate(48_000),
            Param::Transmux(Transport::LatmMcp1),
            Param::HeaderPeriod(3),
        ],
    );
    c.chunks = &[480, 100, 1000];
    assert!(run(&c) > 0);
}

#[test]
fn eld_sbr_and_eld_v2() {
    let mut c = Case::new(
        "eld sbr",
        2,
        vec![
            Param::Aot(Aot::AacEld),
            Param::SbrMode(SbrMode::Enabled),
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Bitrate(64_000),
            Param::Transmux(Transport::Loas),
        ],
    );
    c.chunks = &[512, 333];
    assert!(run(&c) > 0);
    let mut c = Case::new(
        "eld v2 212",
        2,
        vec![
            Param::Aot(Aot::AacEld),
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Mode212),
            Param::Bitrate(48_000),
            Param::Transmux(Transport::Raw),
        ],
    );
    c.chunks = &[512];
    assert!(run(&c) > 0);
}

#[test]
fn multichannel_with_ancillary_data_and_metadata() {
    let mut c = Case::new(
        "5.1 anc metadata",
        6,
        vec![
            Param::Aot(Aot::Mpeg2AacLc),
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Mode1_2_2_1),
            Param::Bitrate(320_000),
            Param::Transmux(Transport::Adts),
            Param::Protection(true),
            Param::MetadataMode(MetadataMode::DynamicRangeAndAncillary),
        ],
    );
    c.anc = true;
    c.metadata = true;
    c.frames = 8000;
    assert!(run(&c) > 0);
    let mut c = Case::new(
        "stereo anc bitrate",
        2,
        vec![
            Param::Aot(Aot::AacLc),
            Param::SampleRate(44_100),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Bitrate(96_000),
            Param::Transmux(Transport::Adts),
            Param::AncillaryBitrate(2000),
        ],
    );
    c.anc = true;
    assert!(run(&c) > 0);
}

#[test]
fn errors_match() {
    // Invalid sample rate: the build stops at the failing parameter.
    let mut c = Case::new(
        "bad rate",
        2,
        vec![Param::Aot(Aot::AacLc), Param::SampleRate(44_000), Param::Bitrate(1)],
    );
    assert_eq!(run(&c), 0);
    // An AOT the allocated modules do not support.
    c = Case::new("aot vs modules", 2, vec![Param::Aot(Aot::HeAac)]);
    c.modules = Modules::AAC;
    assert_eq!(run(&c), 0);
    // Too many channels for maxChannels.
    c = Case::new("channels", 6, vec![Param::ChannelMode(ChannelMode::Mode1_2_2_1)]);
    c.max_channels = 2;
    assert_eq!(run(&c), 0);
    // An initialization error (HE-AAC v2 needs stereo).
    c = Case::new(
        "init error",
        1,
        vec![Param::Aot(Aot::HeAacV2), Param::ChannelMode(ChannelMode::Mono)],
    );
    assert_eq!(run(&c), 0);
    // An output buffer too small for the frames: AACENC_ENCODE_ERROR, then reinitialization.
    c = Case::new(
        "small output",
        2,
        vec![
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Bitrate(256_000),
        ],
    );
    c.out_size = 300;
    c.frames = 6000;
    run(&c);

    // Open errors.
    let r = Encoder::open(Modules::ALL, 9);
    let (e, _s) = Straight::open(0, 9);
    assert_eq!(r.as_ref().map(|_| ()).map_err(|e| e.code()), Err(e));
    let r = Encoder::builder().max_channels(9).build();
    assert_eq!(r.unwrap_err(), BuildError::Open(EncoderError::InvalidConfig));

    // Raw parameters, including unknown IDs.
    let mut enc = Encoder::open(Modules::ALL, 0).unwrap();
    let (_, mut s) = Straight::open(0, 0);
    for (id, v) in [(0x9999, 1), (0x0103, 7), (0x0300, 3), (0x0302, 3), (0xFF00, 0x2000)] {
        assert_eq!(code(&enc.set_raw(id, v)), s.set(id, v), "set_raw {id:#x}");
    }
    compare_params(&enc, &s, "raw");
    // AACENC_CONTROL_STATE clears the pending initialization: encode on an encoder whose core
    // was never initialized (a deviation of the straight port, AACENC_ENCODE_ERROR).
    let mut out = [0u8; 16];
    let mut pcm = [0i16; 2048];
    assert_eq!(code(&enc.set(Param::ControlState(0))), s.set(0xFF00, 0));
    let ra = enc.encode(&pcm, &mut out);
    let (e, _) = s.encode(&mut pcm, 2048, &mut [], None, &mut [0u8; 16]);
    assert_eq!(code(&ra), e);
}

#[test]
fn i32_input_equals_shifted_i16_input() {
    let params = [
        Param::SampleRate(44_100),
        Param::ChannelMode(ChannelMode::Stereo),
        Param::Bitrate(128_000),
        Param::Transmux(Transport::Adts),
    ];
    let mut a = Encoder::builder();
    let mut b = Encoder::builder();
    for p in params {
        a = a.param(p);
        b = b.param(p);
    }
    let (mut a, mut b) = (a.build().unwrap(), b.build().unwrap());
    let pcm16 = signal(5000, 2, 99);
    // (SHORT)(x >> 16) of these values gives the 16-bit samples.
    let pcm32: Vec<i32> =
        pcm16.iter().enumerate().map(|(i, &s)| ((s as i32) << 16) | (i as i32 & 0xffff)).collect();
    let (mut oa, mut ob) = (vec![0u8; 4096], vec![0u8; 4096]);
    let (mut pa, mut pb) = (0, 0);
    while pa < pcm16.len() {
        let ra = a.encode(&pcm16[pa..], &mut oa).unwrap();
        let rb = b.encode(&pcm32[pb..], &mut ob).unwrap();
        assert_eq!(ra, rb);
        assert_eq!(oa, ob);
        pa += ra.in_samples();
        pb += rb.in_samples();
    }
}

#[test]
fn lib_info_matches() {
    let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    fdk_init_lib_info(&mut info);
    assert_eq!(lib::aac_enc_get_lib_info(Some(&mut info)), lib::AACENC_OK);
    let filled: Vec<LibInfo> = info.iter().copied().take_while(|e| e.module_id != 0).collect();
    let got = lib_info().unwrap();
    assert_eq!(got, filled);
    assert!(got.iter().any(|e| e.title == "AAC Encoder"));
}

#[test]
fn param_discriminants_are_the_c_ids() {
    assert_eq!(ParamId::Aot as u32, 0x0100);
    assert_eq!(ParamId::ControlState as u32, 0xFF00);
    assert_eq!(Aot::HeAacV2 as u32, 29);
    assert_eq!(ChannelMode::Mode212 as u32, 128);
    assert_eq!(Transport::Loas as u32, 10);
    assert_eq!(Param::SbrMode(SbrMode::Auto).value(), u32::MAX);
    // The discriminant of a Param is its ID.
    let p = Param::Bitrate(5);
    // SAFETY: `Param` is `repr(u32)`: its first field is the discriminant.
    let d = unsafe { *(&p as *const Param as *const u32) };
    assert_eq!(d, ParamId::Bitrate as u32);
    for e in [EncoderError::InvalidHandle, EncoderError::EncodeEof, EncoderError::Other(7)] {
        assert_eq!(EncoderError::from_code(e.code()), Some(e));
    }
    assert_eq!(EncoderError::from_code(0), None);
}

#[test]
fn encoder_is_send() {
    fn send<T: Send>() {}
    send::<Encoder>();
}

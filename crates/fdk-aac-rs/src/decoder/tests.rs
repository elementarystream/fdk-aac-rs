// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The safe API against the straight port: every call sequence of the safe API is repeated with
//! the straight-port functions, and every result is compared (return codes, consumed bytes, the
//! whole output buffer, `CStreamInfo`, ancillary data, free bytes). The streams are made with
//! the encoder of this crate.

use super::*;
use crate::encoder::{
    Aot, ChannelMode, EncodeInput, Encoder, MetadataMode, Param, SbrMode, Transport as EncTransport,
};
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

/// An encoded stream: the access units (one per non-empty encoder output) and the
/// configuration of `aacEncInfo()`.
struct Stream {
    aus: Vec<Vec<u8>>,
    config: Vec<u8>,
}

impl Stream {
    fn bytes(&self) -> Vec<u8> {
        self.aus.concat()
    }
}

fn encode(params: &[Param], channels: usize, frames: usize, anc: bool) -> Stream {
    let mut b = Encoder::builder();
    for &p in params {
        b = b.param(p);
    }
    let mut enc = b.build().expect("encoder");
    let info = enc.info().expect("info");
    let pcm = signal(frames, channels, 0x2545_f491);
    let mut out = vec![0u8; info.max_out_buf_bytes() as usize];
    let mut aus = Vec::new();
    let mut input = &pcm[..];
    let mut k = 0usize;
    while !input.is_empty() {
        let n = input.len().min(1024 * channels);
        let a: Vec<u8> = (0..(k % 5) * 3).map(|i| (0x41 + i + k) as u8).collect();
        let ein =
            EncodeInput { pcm: &input[..n], ancillary: if anc { &a } else { &[] }, metadata: None };
        let r = enc.encode_with(&ein, &mut out).expect("encode");
        if r.out_bytes() > 0 {
            aus.push(out[..r.out_bytes()].to_vec());
        }
        input = &input[r.in_samples()..];
        k += 1;
    }
    while let Ok(r) = enc.flush(&mut out) {
        if r.out_bytes() > 0 {
            aus.push(out[..r.out_bytes()].to_vec());
        }
    }
    Stream { aus, config: info.config().to_vec() }
}

/// Flips bytes of `data` (deterministic), keeping the first `keep` bytes.
fn corrupt(data: &mut [u8], keep: usize) {
    let mut x = 0x9e37_79b9u32;
    for (i, b) in data.iter_mut().enumerate().skip(keep) {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        if x % 61 == 0 {
            *b ^= (x >> 8) as u8 | 1;
        }
        if i % 997 == 0 {
            *b = 0xff;
        }
    }
}

/// One call and its results.
#[derive(Debug, PartialEq)]
enum Ev {
    Open(bool),
    Config(i32),
    Set(i32, i32, i32),
    AncInit(i32),
    Fill(i32, usize),
    Free(i32, u32),
    Decode(u32, i32, Vec<i16>, CStreamInfo),
    Anc(Vec<Vec<u8>>),
}

/// What a test feeds to the decoder.
struct Run<'a> {
    transport: Transport,
    config: Option<&'a [u8]>,
    params: &'a [(i32, i32)],
    anc: usize,
    /// Access units, filled one by one (packet mode), or a byte stream filled in chunks.
    aus: Option<&'a [Vec<u8>]>,
    bytes: &'a [u8],
    chunks: &'a [usize],
    out_size: usize,
}

/// The flags of decode call `k` (concealment, interruption and history clearing on the way).
fn flags_of(k: usize) -> u32 {
    match k {
        5 => lib::AACDEC_CONCEAL,
        9 => lib::AACDEC_INTR,
        13 => lib::AACDEC_CLRHIST,
        _ => 0,
    }
}

const POISON: i16 = 0x5a5a;

/// The calls of `run` through the straight port.
fn straight(run: &Run<'_>) -> Vec<Ev> {
    let mut ev = Vec::new();
    let mut h = lib::aac_decoder_open(run.transport as i32, 1);
    ev.push(Ev::Open(h.is_some()));
    if let Some(c) = run.config {
        ev.push(Ev::Config(lib::aac_decoder_config_raw(
            h.as_deref_mut().unwrap(),
            &[c],
            &[c.len() as u32],
        )));
    }
    for &(id, v) in run.params {
        ev.push(Ev::Set(id, v, lib::aac_decoder_set_param(h.as_deref_mut(), id, v)));
    }
    if run.anc > 0 {
        let b = vec![0u8; run.anc].into_boxed_slice();
        ev.push(Ev::AncInit(lib::aac_decoder_anc_data_init(
            h.as_deref_mut().unwrap(),
            Some(b),
            run.anc as i32,
        )));
    }
    let mut pcm = vec![0i16; run.out_size];
    let mut k = 0usize;
    let mut decode = |h: &mut Option<Box<AacDecoderInstance>>, ev: &mut Vec<Ev>, flags: u32| {
        pcm.fill(POISON);
        let n = pcm.len() as i32;
        let e = lib::aac_decoder_decode_frame(h.as_deref_mut(), &mut pcm, n, flags);
        let si = *lib::aac_decoder_get_stream_info(h.as_deref()).unwrap();
        ev.push(Ev::Decode(flags, e, pcm.clone(), si));
        if lib::is_output_valid(e) != 0 && run.anc > 0 {
            let d = h.as_deref().unwrap();
            let buf = lib::aac_decoder_anc_data_buffer(d).unwrap();
            let mut v = Vec::new();
            for i in 0.. {
                let (mut off, mut size) = (None, 0);
                lib::aac_decoder_anc_data_get(d, i, &mut off, &mut size);
                match off {
                    Some(o) => v.push(buf[o..o + size as usize].to_vec()),
                    None => break,
                }
            }
            ev.push(Ev::Anc(v));
        }
        e
    };
    let fill = |h: &mut Option<Box<AacDecoderInstance>>, ev: &mut Vec<Ev>, data: &[u8]| {
        let mut free = 0;
        let e = lib::aac_decoder_get_free_bytes(h.as_deref_mut(), &mut free);
        ev.push(Ev::Free(e, free));
        let mut valid = [data.len() as u32];
        let e = lib::aac_decoder_fill(
            h.as_deref_mut().unwrap(),
            &[data],
            &[data.len() as u32],
            &mut valid,
        );
        let consumed = data.len() - valid[0] as usize;
        ev.push(Ev::Fill(e, consumed));
        consumed
    };
    match run.aus {
        Some(aus) => {
            for au in aus {
                fill(&mut h, &mut ev, au);
                decode(&mut h, &mut ev, flags_of(k));
                k += 1;
            }
        }
        None => {
            let mut pos = 0;
            let mut c = 0;
            while pos < run.bytes.len() {
                let n = run.chunks[c % run.chunks.len()].min(run.bytes.len() - pos);
                c += 1;
                pos += fill(&mut h, &mut ev, &run.bytes[pos..pos + n]);
                for _ in 0..8 {
                    let e = decode(&mut h, &mut ev, flags_of(k));
                    k += 1;
                    if e == lib::AAC_DEC_NOT_ENOUGH_BITS || lib::is_output_valid(e) == 0 {
                        break;
                    }
                }
            }
        }
    }
    for _ in 0..3 {
        decode(&mut h, &mut ev, lib::AACDEC_FLUSH);
    }
    lib::aac_decoder_close(h);
    ev
}

fn code<T>(r: &Result<T, DecoderError>) -> i32 {
    match r {
        Ok(_) => lib::AAC_DEC_OK,
        Err(e) => e.code(),
    }
}

/// The calls of `run` through the safe API.
fn safe(run: &Run<'_>) -> Vec<Ev> {
    let mut ev = Vec::new();
    let mut dec = Decoder::new(run.transport).expect("open");
    ev.push(Ev::Open(true));
    if let Some(c) = run.config {
        ev.push(Ev::Config(code(&dec.config_raw(c))));
    }
    for &(id, v) in run.params {
        ev.push(Ev::Set(id, v, code(&dec.set_raw(id, v))));
    }
    if run.anc > 0 {
        ev.push(Ev::AncInit(code(&dec.set_ancillary_buffer(run.anc))));
    }
    let mut pcm = vec![0i16; run.out_size];
    let mut k = 0usize;
    let mut decode = |dec: &mut Decoder, ev: &mut Vec<Ev>, flags: u32| {
        pcm.fill(POISON);
        let r = dec.decode_frame(&mut pcm, DecodeFlags::from_bits(flags));
        let e = match &r {
            Ok(f) => {
                // The samples are the valid part of the buffer.
                let si = dec.stream_info();
                assert_eq!(f.samples().len(), (si.frame_size() * si.num_channels()) as usize);
                f.error().map_or(lib::AAC_DEC_OK, DecoderError::code)
            }
            Err(e) => {
                assert!(!e.is_output_valid());
                e.code()
            }
        };
        ev.push(Ev::Decode(flags, e, pcm.clone(), *dec.stream_info().raw()));
        if lib::is_output_valid(e) != 0 && run.anc > 0 {
            ev.push(Ev::Anc(dec.ancillary_elements().map(<[u8]>::to_vec).collect()));
        }
        e
    };
    let fill = |dec: &mut Decoder, ev: &mut Vec<Ev>, data: &[u8]| {
        let r = dec.free_bytes();
        ev.push(Ev::Free(code(&r), r.unwrap_or(0)));
        let r = dec.fill(data);
        let consumed = *r.as_ref().unwrap_or(&0);
        ev.push(Ev::Fill(code(&r), consumed));
        consumed
    };
    match run.aus {
        Some(aus) => {
            for au in aus {
                fill(&mut dec, &mut ev, au);
                decode(&mut dec, &mut ev, flags_of(k));
                k += 1;
            }
        }
        None => {
            let mut pos = 0;
            let mut c = 0;
            while pos < run.bytes.len() {
                let n = run.chunks[c % run.chunks.len()].min(run.bytes.len() - pos);
                c += 1;
                pos += fill(&mut dec, &mut ev, &run.bytes[pos..pos + n]);
                for _ in 0..8 {
                    let e = decode(&mut dec, &mut ev, flags_of(k));
                    k += 1;
                    if e == lib::AAC_DEC_NOT_ENOUGH_BITS || lib::is_output_valid(e) == 0 {
                        break;
                    }
                }
            }
        }
    }
    for _ in 0..3 {
        decode(&mut dec, &mut ev, lib::AACDEC_FLUSH);
    }
    ev
}

fn compare(name: &str, run: &Run<'_>) -> Vec<Ev> {
    let a = straight(run);
    let b = safe(run);
    assert_eq!(a.len(), b.len(), "{name}: number of calls");
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(x, y, "{name}: call {i}");
    }
    a
}

fn decoded(ev: &[Ev]) -> usize {
    ev.iter().filter(|e| matches!(e, Ev::Decode(_, 0, _, _))).count()
}

const STREAM_CHUNKS: &[usize] = &[1000, 333, 4096, 17, 2500];

#[test]
fn lc_adts_with_ancillary_data() {
    let s = encode(
        &[
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Bitrate(128_000),
            Param::Transmux(EncTransport::Adts),
            Param::MetadataMode(MetadataMode::Ancillary),
        ],
        2,
        48_000,
        true,
    );
    let bytes = s.bytes();
    let run = Run {
        transport: Transport::Adts,
        config: None,
        params: &[],
        anc: 64,
        aus: None,
        bytes: &bytes,
        chunks: STREAM_CHUNKS,
        out_size: 4096,
    };
    let ev = compare("lc-adts", &run);
    assert!(decoded(&ev) > 40);
    let anc: usize =
        ev.iter().map(|e| if let Ev::Anc(v) = e { v.iter().map(Vec::len).sum() } else { 0 }).sum();
    assert!(anc > 0, "ancillary data decoded");
    // With a small ancillary buffer: AAC_DEC_TOO_SMALL_ANC_BUFFER is not output-valid.
    let run = Run { anc: 4, ..run };
    compare("lc-adts-small-anc", &run);
}

#[test]
fn he_aac_loas_with_parameters() {
    let s = encode(
        &[
            Param::Aot(Aot::HeAac),
            Param::SampleRate(44_100),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Bitrate(64_000),
            Param::Transmux(EncTransport::Loas),
        ],
        2,
        44_100,
        false,
    );
    let bytes = s.bytes();
    let params = [
        (lib::AAC_PCM_LIMITER_ENABLE, 1),
        (lib::AAC_PCM_MAX_OUTPUT_CHANNELS, 1),
        (lib::AAC_CONCEAL_METHOD, 2),
        (lib::AAC_DRC_REFERENCE_LEVEL, 200),
        (lib::AAC_PCM_LIMITER_ATTACK_TIME, 0),
        (0x7777, 1),
    ];
    let run = Run {
        transport: Transport::Loas,
        config: None,
        params: &params,
        anc: 0,
        aus: None,
        bytes: &bytes,
        chunks: STREAM_CHUNKS,
        out_size: 8192,
    };
    let ev = compare("he-loas", &run);
    assert!(decoded(&ev) > 15);
}

#[test]
fn he_aac_v2_adts_corrupt() {
    let s = encode(
        &[
            Param::Aot(Aot::HeAacV2),
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Bitrate(32_000),
            Param::Transmux(EncTransport::Adts),
        ],
        2,
        48_000,
        false,
    );
    let bytes = s.bytes();
    let run = Run {
        transport: Transport::Adts,
        config: None,
        params: &[],
        anc: 0,
        aus: None,
        bytes: &bytes,
        chunks: STREAM_CHUNKS,
        out_size: 4096,
    };
    assert!(decoded(&compare("hev2-adts", &run)) > 15);
    let mut bad = bytes.clone();
    corrupt(&mut bad, 2000);
    let run = Run { bytes: &bad, ..run };
    let ev = compare("hev2-adts-corrupt", &run);
    assert!(ev.iter().any(
        |e| matches!(e, Ev::Decode(_, c, _, _) if *c != 0 && *c != lib::AAC_DEC_NOT_ENOUGH_BITS)
    ));
}

#[test]
fn eld_raw_access_units() {
    for (name, params, channels) in [
        (
            "eld-sbr",
            vec![
                Param::Aot(Aot::AacEld),
                Param::SbrMode(SbrMode::Enabled),
                Param::SampleRate(48_000),
                Param::ChannelMode(ChannelMode::Stereo),
                Param::Bitrate(64_000),
                Param::Transmux(EncTransport::Raw),
            ],
            2,
        ),
        (
            "eld-v2",
            vec![
                Param::Aot(Aot::AacEld),
                Param::SampleRate(48_000),
                Param::ChannelMode(ChannelMode::Mode212),
                Param::Bitrate(48_000),
                Param::Transmux(EncTransport::Raw),
            ],
            2,
        ),
        (
            "ld-mono",
            vec![
                Param::Aot(Aot::AacLd),
                Param::SampleRate(32_000),
                Param::ChannelMode(ChannelMode::Mono),
                Param::GranuleLength(480),
                Param::Bitrate(48_000),
                Param::Transmux(EncTransport::Raw),
            ],
            1,
        ),
    ] {
        let s = encode(&params, channels, 24_000, false);
        let aus = s.aus.clone();
        let run = Run {
            transport: Transport::Raw,
            config: Some(&s.config),
            params: &[(lib::AAC_PCM_MIN_OUTPUT_CHANNELS, 2)],
            anc: 0,
            aus: Some(&aus),
            bytes: &[],
            chunks: &[],
            out_size: 2048,
        };
        assert!(decoded(&compare(name, &run)) > 20, "{name}");
        // Corrupt access units, and an output buffer that is too small for one frame.
        let mut bad = aus.clone();
        for au in bad.iter_mut().skip(3) {
            corrupt(au, 0);
        }
        let run = Run { aus: Some(&bad), out_size: 300, ..run };
        compare(name, &run);
    }
}

#[test]
fn multichannel_downmix_and_mapping() {
    let s = encode(
        &[
            Param::SampleRate(48_000),
            Param::ChannelMode(ChannelMode::Mode1_2_2_1),
            Param::Bitrate(256_000),
            Param::Transmux(EncTransport::Adts),
        ],
        6,
        24_000,
        false,
    );
    let bytes = s.bytes();
    for params in [
        &[(lib::AAC_PCM_MAX_OUTPUT_CHANNELS, 2)][..],
        &[(lib::AAC_PCM_OUTPUT_CHANNEL_MAPPING, 0), (lib::AAC_PCM_LIMITER_ENABLE, 0)][..],
        &[(lib::AAC_PCM_MAX_OUTPUT_CHANNELS, 9)][..],
    ] {
        let run = Run {
            transport: Transport::Adts,
            config: None,
            params,
            anc: 0,
            aus: None,
            bytes: &bytes,
            chunks: STREAM_CHUNKS,
            out_size: 16384,
        };
        assert!(decoded(&compare("c51", &run)) > 10);
    }
}

#[test]
fn small_output_buffer_is_cleared() {
    let s = encode(
        &[
            Param::SampleRate(44_100),
            Param::ChannelMode(ChannelMode::Stereo),
            Param::Transmux(EncTransport::Adts),
        ],
        2,
        8192,
        false,
    );
    let mut dec = Decoder::new(Transport::Adts).unwrap();
    let bytes = s.bytes();
    assert_eq!(dec.fill(&bytes).unwrap(), bytes.len());
    let mut pcm = vec![7i16; 1000];
    assert_eq!(
        dec.decode_frame(&mut pcm, DecodeFlags::empty()),
        Err(DecoderError::OutputBufferTooSmall)
    );
    assert!(pcm.iter().all(|&x| x == 0));
    let mut pcm = vec![7i16; 3000];
    let f = dec.decode_frame(&mut pcm, DecodeFlags::empty()).unwrap();
    assert_eq!(f.samples().len(), 2048);
    assert_eq!(f.error(), None);
    // Nothing is written past the frame.
    assert!(pcm[2048..].iter().all(|&x| x == 7));
}

#[test]
fn errors_and_parameters() {
    // A configuration with an unsupported AOT (AAC Main): aacDecoder_ConfigRaw() reports
    // AAC_DEC_UNSUPPORTED_FORMAT.
    assert_eq!(Decoder::with_config(&[0x0a, 0x10]).unwrap_err(), DecoderError::UnsupportedFormat);
    // An empty configuration is ignored by aacDecoder_ConfigRaw().
    let mut dec = Decoder::new(Transport::Raw).unwrap();
    assert_eq!(dec.config_raw(&[]), Ok(()));
    let mut h = lib::aac_decoder_open(lib_tt_raw(), 1);
    for p in [
        DecoderParam::DualChannelOutputMode(DualChannelMode::Mix),
        DecoderParam::OutputChannelMapping(ChannelMapping::Mpeg),
        DecoderParam::Limiter(LimiterMode::Auto),
        DecoderParam::Limiter(false.into()),
        DecoderParam::LimiterAttackTime(5),
        DecoderParam::LimiterAttackTime(0),
        DecoderParam::LimiterReleaseTime(20),
        DecoderParam::MinOutputChannels(2),
        DecoderParam::MaxOutputChannels(9),
        DecoderParam::MetadataProfile(MetadataProfile::AribJapan),
        DecoderParam::MetadataExpiryTime(100),
        DecoderParam::ConcealMethod(ConcealMethod::NoiseSubstitution),
        DecoderParam::DrcBoostFactor(127),
        DecoderParam::DrcAttenuationFactor(128),
        DecoderParam::DrcReferenceLevel(-1),
        DecoderParam::DrcHeavyCompression(true),
        DecoderParam::DrcDefaultPresentationMode(DrcPresentationMode::Mode2),
        DecoderParam::DrcEncTargetLevel(127),
        DecoderParam::UniDrcSetEffect(DrcEffect::GeneralCompression),
        DecoderParam::UniDrcAlbumMode(true),
        DecoderParam::QmfLowPower(QmfMode::LowPower),
        DecoderParam::TpdecClearBuffer,
    ] {
        let c = lib::aac_decoder_set_param(h.as_deref_mut(), p.id().code(), p.value());
        assert_eq!(code(&dec.set(p)), c, "{p:?}");
    }
    assert_eq!(dec.set_raw(0x1234, 0), Err(DecoderError::SetParamFail));
    assert_eq!(
        dec.stream_info(),
        StreamInfo(*lib::aac_decoder_get_stream_info(h.as_deref()).unwrap())
    );
    lib::aac_decoder_close(h);

    // Raw ISO BMFF data: too short, and a box with an unknown type.
    assert_eq!(dec.raw_isobmff_data(&[0, 0, 0, 8]), Err(DecoderError::Unknown));
    assert_eq!(dec.raw_isobmff_data(&[0, 0, 0, 12, b'f', b'r', b'e', b'e', 1, 2, 3, 4]), Ok(()));
    assert_eq!(
        dec.raw_isobmff_data(&[0, 0, 0, 13, b'f', b'r', b'e', b'e', 1, 2, 3, 4]),
        Err(DecoderError::Unknown)
    );
    // Without an ancillary buffer there are no elements.
    assert_eq!(dec.ancillary_data(0), None);
    assert_eq!(dec.set_ancillary_buffer(16), Ok(()));
    assert_eq!(dec.ancillary_elements().count(), 0);
    assert_eq!(dec.clear_ancillary_buffer(), Ok(()));
    assert!(dec.free_bytes().unwrap() > 0);
}

fn lib_tt_raw() -> i32 {
    Transport::Raw as i32
}

#[test]
fn error_codes_round_trip() {
    for c in (0..0x9000).filter(|&c| {
        DecoderError::from_code(c).is_some_and(|e| !matches!(e, DecoderError::Other(_)))
    }) {
        let e = DecoderError::from_code(c).unwrap();
        assert_eq!(e.code(), c);
        assert_eq!(e.is_output_valid(), lib::is_output_valid(c) != 0);
        assert_eq!(e.is_decode_error(), lib::is_decode_error(c) != 0);
        assert_eq!(e.is_init_error(), lib::is_init_error(c) != 0);
        assert!(e.to_string().contains(&format!("0x{c:04x}")));
    }
    assert_eq!(DecoderError::from_code(0), None);
    assert_eq!(DecoderError::from_code(0x1234), Some(DecoderError::Other(0x1234)));
    assert!(DecoderError::NotEnoughBits.is_sync_error());
    assert_eq!(
        (DecodeFlags::CONCEAL | DecodeFlags::FLUSH | DecodeFlags::INTR | DecodeFlags::CLRHIST)
            .bits(),
        15
    );
    assert!(DecodeFlags::from_bits(3).contains(DecodeFlags::FLUSH));
}

#[test]
fn lib_info_matches_the_straight_port() {
    let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    fdk_init_lib_info(&mut info);
    assert_eq!(lib::aac_decoder_get_lib_info(Some(&mut info)), 0);
    let expected: Vec<LibInfo> =
        info.into_iter().filter(|e| e.module_id != fdk_aac_rs_fdk::audio::FDK_NONE).collect();
    assert_eq!(lib_info().unwrap(), expected);
}

#[test]
fn decoder_is_send() {
    fn send<T: Send>() {}
    send::<Decoder>();
    // Separate decoders on separate threads.
    let s = encode(
        &[
            Param::SampleRate(32_000),
            Param::ChannelMode(ChannelMode::Mono),
            Param::Transmux(EncTransport::Adts),
        ],
        1,
        16_000,
        false,
    );
    let bytes = s.bytes();
    let run = || {
        let mut dec = Decoder::new(Transport::Adts).unwrap();
        let mut pcm = vec![0i16; 2048];
        let mut all = Vec::new();
        let mut input = &bytes[..];
        while !input.is_empty() {
            input = &input[dec.fill(input).unwrap()..];
            while let Ok(f) = dec.decode_frame(&mut pcm, DecodeFlags::empty()) {
                all.extend_from_slice(f.samples());
            }
        }
        all
    };
    let (a, b) = std::thread::scope(|sc| {
        let x = sc.spawn(run);
        let y = sc.spawn(run);
        (x.join().unwrap(), y.join().unwrap())
    });
    assert_eq!(a, b);
    assert!(!a.is_empty());
}

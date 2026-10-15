// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The opt-in frame pipelining of the decoder: with any thread count, the frames (return value,
//! samples, stream information) must be those of one thread, in the same order, one call later;
//! also with concealment calls, parameter changes, damaged streams, and under load (many
//! decoders with random thread counts at once).

#![cfg(feature = "encoder")]

use fdk_aac_rs::decoder::{
    DecodeFlags, Decoder, DecoderError, DecoderParam, LimiterMode, Transport as DecTransport,
};
use fdk_aac_rs::encoder::{Aot, Bitrate, ChannelMode, Encoder, Param, SbrMode, Transport};
use fdk_aac_rs_fdk::par::{tune::MAX_MODES, Tuning};

/// A test signal: tones, noise, clicks and silence.
fn signal(seconds: f64, rate: u32, channels: usize, seed: u32) -> Vec<i16> {
    let n = (seconds * rate as f64) as usize;
    let mut x = seed | 1;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        (x >> 8) as f64 / (1u32 << 24) as f64 - 0.5
    };
    let mut v = Vec::with_capacity(n * channels);
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let seg = (t * 4.0) as usize % 4;
        for c in 0..channels {
            let f = 220.0 * (1.0 + c as f64 * 0.37) * (1.0 + t);
            let tone = (2.0 * core::f64::consts::PI * f * t).sin();
            let s = match seg {
                0 => 12000.0 * tone + 300.0 * rnd(),
                1 => 9000.0 * rnd(),
                2 => {
                    if i % (rate as usize / 7) < 40 {
                        30000.0 * rnd()
                    } else {
                        200.0 * tone
                    }
                }
                _ => 0.0,
            };
            v.push(s.clamp(-32768.0, 32767.0) as i16);
        }
    }
    v
}

/// A stream: (name, transport of the decoder, bytes).
type Stream = (String, DecTransport, Vec<u8>);

fn encode(
    name: &str,
    aot: Aot,
    mode: ChannelMode,
    channels: usize,
    rate: u32,
    bitrate: u32,
    transport: Transport,
    extra: &[Param],
) -> Vec<u8> {
    let mut b = Encoder::builder()
        .aot(aot)
        .sample_rate(rate)
        .channel_mode(mode)
        .bitrate(Bitrate::Cbr(bitrate))
        .transport(transport);
    for p in extra {
        b = b.param(*p);
    }
    let mut enc = b.build().unwrap_or_else(|e| panic!("{name}: {e}"));
    let pcm = signal(3.0, rate, channels, 7);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    let frame = 1024 * channels;
    for chunk in pcm.chunks(frame) {
        let r = enc.encode(chunk, &mut buf).unwrap();
        out.extend_from_slice(&buf[..r.out_bytes()]);
    }
    loop {
        match enc.flush(&mut buf) {
            Ok(r) if r.out_bytes() > 0 => out.extend_from_slice(&buf[..r.out_bytes()]),
            _ => break,
        }
    }
    out
}

fn streams() -> Vec<Stream> {
    use ChannelMode as M;
    let mut v: Vec<Stream> = vec![
        (
            "lc-stereo".into(),
            DecTransport::Adts,
            encode("lc", Aot::AacLc, M::Stereo, 2, 44_100, 128_000, Transport::Adts, &[]),
        ),
        (
            "lc-mono".into(),
            DecTransport::Adts,
            encode("lc-mono", Aot::AacLc, M::Mono, 1, 48_000, 64_000, Transport::Adts, &[]),
        ),
        (
            "he".into(),
            DecTransport::Adts,
            encode("he", Aot::HeAac, M::Stereo, 2, 44_100, 64_000, Transport::Adts, &[]),
        ),
        (
            "hev2".into(),
            DecTransport::Adts,
            encode("hev2", Aot::HeAacV2, M::Stereo, 2, 44_100, 32_000, Transport::Adts, &[]),
        ),
        (
            "lc-5.1".into(),
            DecTransport::Adts,
            encode("lc-51", Aot::AacLc, M::Mode1_2_2_1, 6, 48_000, 320_000, Transport::Adts, &[]),
        ),
        (
            "ld".into(),
            DecTransport::Loas,
            encode("ld", Aot::AacLd, M::Stereo, 2, 48_000, 128_000, Transport::Loas, &[]),
        ),
        (
            "eld".into(),
            DecTransport::Loas,
            encode("eld", Aot::AacEld, M::Stereo, 2, 48_000, 96_000, Transport::Loas, &[]),
        ),
        (
            "he-loas".into(),
            DecTransport::Loas,
            encode("he-loas", Aot::HeAac, M::Stereo, 2, 48_000, 64_000, Transport::Loas, &[]),
        ),
        (
            "eld-sbr".into(),
            DecTransport::Loas,
            encode(
                "eld-sbr",
                Aot::AacEld,
                M::Stereo,
                2,
                48_000,
                64_000,
                Transport::Loas,
                &[Param::SbrMode(SbrMode::Enabled)],
            ),
        ),
        (
            "he-5.1".into(),
            DecTransport::Adts,
            encode("he-51", Aot::HeAac, M::Mode1_2_2_1, 6, 48_000, 160_000, Transport::Adts, &[]),
        ),
    ];
    // Damaged copies: bytes flipped after the start (sync errors, CRC-less garbage, concealment).
    let damaged: Vec<Stream> = v
        .iter()
        .map(|(n, t, d)| {
            let mut d = d.clone();
            let mut x = 0x2545_f491u32;
            for i in 400..d.len() {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                if x % 97 == 0 {
                    d[i] ^= (x >> 8) as u8 | 1;
                }
            }
            (format!("{n}-damaged"), *t, d)
        })
        .collect();
    v.extend(damaged);
    v
}

/// One frame of the output: return value (0 for `AAC_DEC_OK`), samples, stream information.
type Frame = (i32, Vec<i16>, fdk_aac_rs::decoder::StreamInfo);

/// A deterministic random sequence.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Decodes `data` with `threads` threads, feeding it in random chunks and decoding until the
/// decoder needs input, as an application does; every 23rd frame is a concealment call, and
/// the limiter is switched at frame 40 (the calls of the sequential decoder are emulated as in
/// fdk-aac-rs-dec: a call that starts a frame but returns nothing yet is not the end of the
/// input). Returns the frames in order.
fn decode(transport: DecTransport, data: &[u8], threads: usize, seed: u64) -> Vec<Frame> {
    decode_tuned(transport, data, threads, seed, None).0
}

/// [`decode`] with the given choice of the thread mode (else the default); also returns the
/// frames per mode.
fn decode_tuned(
    transport: DecTransport,
    data: &[u8],
    threads: usize,
    seed: u64,
    tuning: Option<Tuning>,
) -> (Vec<Frame>, [u64; MAX_MODES]) {
    let mut dec = Decoder::new(transport).unwrap();
    dec.set_threads(threads).unwrap();
    if let Some(t) = tuning {
        dec.set_tuning(t);
    }
    let mut rng = Rng(seed | 1);
    let mut pcm = vec![0i16; 8 * 4096];
    let mut frames = Vec::new();
    let mut started = 0usize; // frames started (calls of the sequential decoder with a frame)
    let push = |frames: &mut Vec<Frame>,
                r: Result<fdk_aac_rs::decoder::DecodedFrame<'_>, DecoderError>,
                info| {
        match r {
            Ok(f) => frames.push((f.error().map_or(0, |e| e.code()), f.samples().to_vec(), info)),
            Err(e) => frames.push((e.code(), Vec::new(), info)),
        }
    };
    let mut pos = 0;
    let mut switched = false;
    while pos < data.len() {
        let n = (1 + rng.next() % 3000) as usize;
        let end = (pos + n).min(data.len());
        pos += dec.fill(&data[pos..end]).unwrap();
        loop {
            if started == 40 && !switched {
                dec.set(DecoderParam::Limiter(LimiterMode::Off)).unwrap();
                switched = true;
            }
            let flags =
                if started % 23 == 22 { DecodeFlags::CONCEAL } else { DecodeFlags::empty() };
            let before = dec.pending_frames();
            let r = dec.decode_frame(&mut pcm, flags);
            let after = dec.pending_frames();
            let info = dec.stream_info();
            match r {
                Err(DecoderError::NotEnoughBits) if after > before => {
                    started += 1; // a frame started; nothing to return yet
                }
                Err(DecoderError::NotEnoughBits) => break,
                r if after < before => {
                    // A drain: the call found not enough input (as the sequential decoder) and
                    // returned the pending frame.
                    push(&mut frames, r, info);
                    break;
                }
                r => {
                    // (with one thread: the frame of this call; otherwise of the previous one,
                    // and this call started a frame)
                    started += 1;
                    push(&mut frames, r, info);
                }
            }
            if frames.len() > 10_000 {
                panic!("decoder does not stop");
            }
        }
    }
    loop {
        let r = dec.drain(&mut pcm);
        let info = dec.stream_info();
        match r {
            Ok(None) => break,
            Ok(Some(f)) => {
                frames.push((f.error().map_or(0, |e| e.code()), f.samples().to_vec(), info))
            }
            Err(e) => frames.push((e.code(), Vec::new(), info)),
        }
    }
    (frames, dec.mode_frames())
}

fn check(name: &str, a: &[Frame], b: &[Frame], threads: usize) {
    assert_eq!(a.len(), b.len(), "{name}, {threads} threads: number of frames");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(x.0, y.0, "{name}, {threads} threads: return value of frame {i}");
        assert!(x.1 == y.1, "{name}, {threads} threads: samples of frame {i}");
        assert_eq!(x.2, y.2, "{name}, {threads} threads: stream information of frame {i}");
    }
}

#[test]
fn every_thread_count_gives_the_frames_of_one_thread() {
    for (name, t, data) in streams() {
        let reference = decode(t, &data, 1, 11);
        assert!(reference.len() > 40, "{name}: {} frames", reference.len());
        for threads in [2, 3, 4, 8] {
            check(&name, &reference, &decode(t, &data, threads, 11), threads);
        }
    }
}

/// Every thread mode of the decoder, and every switch between two modes (the cycle schedule of
/// `fdk_aac_rs_fdk::par::tune`), gives the frames of one thread.
#[test]
fn every_thread_mode_gives_the_frames_of_one_thread() {
    for (name, t, data) in streams() {
        let reference = decode(t, &data, 1, 13);
        for threads in [2, 4] {
            let modes = if threads == 2 { 4 } else { 5 };
            let (frames, per_mode) = decode_tuned(t, &data, threads, 13, Some(Tuning::Cycle));
            check(&name, &reference, &frames, threads);
            assert!(per_mode[..modes].iter().all(|&n| n > 0), "{name}: {per_mode:?}");
            for m in 0..modes as u8 {
                let (frames, per_mode) =
                    decode_tuned(t, &data, threads, 13, Some(Tuning::Fixed(m)));
                check(&name, &reference, &frames, threads);
                assert_eq!(per_mode.iter().sum::<u64>(), per_mode[m as usize], "{name}");
            }
        }
    }
}

/// With more than two threads the channels of a frame are decoded in parallel (the parallel
/// paths of `fdk_aac_rs_aacdec::pipeline::chan` and the SBR decoder are taken), and the frames
/// are those of one thread.
#[test]
fn per_channel_parallelism_is_used() {
    use fdk_aac_rs_aacdec::pipeline::par_stats;
    let streams = streams();
    let find = |n: &str| streams.iter().find(|s| s.0 == n).unwrap();
    // (stream, which counter must grow: element decoding, inverse transform, SBR)
    for (name, want) in [
        ("lc-5.1", [true, true, false]),
        ("he", [false, false, true]),
        ("he-5.1", [true, true, true]),
        ("eld-sbr", [false, false, true]),
    ] {
        let (_, t, data) = find(name);
        let reference = decode(*t, data, 1, 3);
        for threads in [3, 4] {
            let before = par_stats();
            // (the mode with every parallel feature)
            let frames = decode_tuned(*t, data, threads, 3, Some(Tuning::FULL)).0;
            let after = par_stats();
            check(name, &reference, &frames, threads);
            let grew = [after.0 > before.0, after.1 > before.1, after.2 > before.2];
            for k in 0..3 {
                // (other tests may run at the same time: only growth is checked)
                assert!(!want[k] || grew[k], "{name}, {threads} threads: counter {k} {grew:?}");
            }
        }
    }
}

/// Many decoders with random thread counts at once, on all cores.
#[test]
fn concurrent_decoders_with_random_thread_counts() {
    let streams = std::sync::Arc::new(streams());
    let references: Vec<Vec<Frame>> = streams.iter().map(|(_, t, d)| decode(*t, d, 1, 5)).collect();
    let references = std::sync::Arc::new(references);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 16);
    let handles: Vec<_> = (0..workers)
        .map(|w| {
            let streams = streams.clone();
            let references = references.clone();
            std::thread::spawn(move || {
                let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (w as u64 * 0x1000_0001));
                for _ in 0..6 {
                    let i = (rng.next() % streams.len() as u64) as usize;
                    let threads = 1 + (rng.next() % 8) as usize;
                    let (name, t, data) = &streams[i];
                    let frames = decode(*t, data, threads, 5);
                    check(name, &references[i], &frames, threads);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn thread_count_changes_and_drain() {
    let (_, t, data) = &streams()[2]; // HE-AAC
    let mut dec = Decoder::new(*t).unwrap();
    let mut pcm = vec![0i16; 8 * 4096];
    dec.set_threads(2).unwrap();
    assert_eq!(dec.threads(), 2);
    let consumed = dec.fill(&data[..4000]).unwrap();
    assert!(consumed > 0);
    // The first frame: nothing to return yet.
    assert_eq!(
        dec.decode_frame(&mut pcm, DecodeFlags::empty()).err(),
        Some(DecoderError::NotEnoughBits)
    );
    assert_eq!(dec.pending_frames(), 1);
    // One thread again only without a pending frame.
    assert_eq!(dec.set_threads(1), Err(DecoderError::SetParamFail));
    assert!(dec.drain(&mut pcm).unwrap().is_some());
    assert_eq!(dec.pending_frames(), 0);
    assert!(dec.drain(&mut pcm).unwrap().is_none());
    dec.set_threads(1).unwrap();
    assert_eq!(dec.threads(), 1);
    // More threads at any time.
    dec.set_threads(4).unwrap();
    let _ = dec.decode_frame(&mut pcm, DecodeFlags::empty());
    dec.set_threads(2).unwrap();
}

/// Decodes `data` as an application that fills what fits and decodes until the decoder needs
/// input, and after the input calls `decode_frame` until no frame is pending (the end of input
/// protocol of the module documentation, without `drain`).
fn decode_greedy(transport: DecTransport, data: &[u8], threads: usize) -> Vec<Frame> {
    let mut dec = Decoder::new(transport).unwrap();
    dec.set_threads(threads).unwrap();
    let mut pcm = vec![0i16; 8 * 4096];
    let mut frames: Vec<Frame> = Vec::new();
    let mut input = data;
    let mut push = |r: Result<fdk_aac_rs::decoder::DecodedFrame<'_>, DecoderError>, info| match r {
        Ok(f) => frames.push((f.error().map_or(0, |e| e.code()), f.samples().to_vec(), info)),
        Err(e) => frames.push((e.code(), Vec::new(), info)),
    };
    while !input.is_empty() {
        let consumed = dec.fill(input).unwrap();
        input = &input[consumed..];
        loop {
            let r = dec.decode_frame(&mut pcm, DecodeFlags::empty());
            let info = dec.stream_info();
            match r {
                Err(DecoderError::NotEnoughBits) => break,
                r => push(r, info),
            }
        }
    }
    let mut calls = 0;
    loop {
        let r = dec.decode_frame(&mut pcm, DecodeFlags::empty());
        let info = dec.stream_info();
        match r {
            Err(DecoderError::NotEnoughBits) if dec.pending_frames() == 0 => break,
            Err(DecoderError::NotEnoughBits) => {}
            r => push(r, info),
        }
        calls += 1;
        assert!(calls < 100_000, "decoder does not stop");
    }
    frames
}

/// [`check`] for [`decode_greedy`]. The caller fills its input whenever a call returns
/// `NotEnoughBits`, and with more than one thread the first frame comes one call later, so the
/// input is filled at other calls than with one thread. On a damaged stream the transport decoder
/// may then skip to the next access unit in the call that decodes it instead of an earlier call,
/// which moves the skipped bits between the bit rates of two calls (`bitRate` counts the bits of
/// the call; the byte counters agree). On damaged streams the bit rate is not compared.
fn check_greedy(name: &str, a: &[Frame], b: &[Frame], threads: usize) {
    if !name.ends_with("-damaged") {
        return check(name, a, b, threads);
    }
    assert_eq!(a.len(), b.len(), "{name}, {threads} threads: number of frames");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(x.0, y.0, "{name}, {threads} threads: return value of frame {i}");
        assert!(x.1 == y.1, "{name}, {threads} threads: samples of frame {i}");
        let (mut p, mut q) = (*x.2.raw(), *y.2.raw());
        (p.bit_rate, q.bit_rate) = (0, 0);
        assert_eq!(p, q, "{name}, {threads} threads: stream information of frame {i}");
    }
}

#[test]
fn end_of_input_protocol_gives_all_frames() {
    for (name, t, data) in streams() {
        let reference = decode_greedy(t, &data, 1);
        for threads in [2, 4] {
            check_greedy(&name, &reference, &decode_greedy(t, &data, threads), threads);
        }
    }
}

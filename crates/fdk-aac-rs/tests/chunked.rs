// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: tests of the chunked parallel decoding.

//! The chunked decoding ([`fdk_aac_rs::chunked::decode_chunked`]) returns exactly the output of
//! the sequential decoding ([`fdk_aac_rs::chunked::decode_sequential`]).

use fdk_aac_rs::chunked::*;
use fdk_aac_rs::decoder::{DecoderError, StreamInfo, Transport};

/// The output of a decoding, frame by frame.
#[derive(Debug, PartialEq)]
struct Out {
    samples: Vec<i16>,
    error: Option<DecoderError>,
    info: StreamInfo,
    ancillary: Vec<Vec<u8>>,
}

fn sequential(setup: &DecoderSetup, input: Input) -> Vec<Out> {
    let mut v = Vec::new();
    decode_sequential(setup, input, &mut |f| {
        v.push(Out {
            samples: f.samples.to_vec(),
            error: f.error,
            info: *f.info,
            ancillary: f.ancillary.to_vec(),
        });
        Ok::<(), ()>(())
    })
    .unwrap();
    v
}

fn chunked(setup: &DecoderSetup, input: Input, opts: &ChunkedOptions) -> (Vec<Out>, ChunkedStats) {
    let mut v = Vec::new();
    let stats = decode_chunked(setup, input, opts, &mut |f| {
        v.push(Out {
            samples: f.samples.to_vec(),
            error: f.error,
            info: *f.info,
            ancillary: f.ancillary.to_vec(),
        });
        Ok::<(), ()>(())
    })
    .unwrap();
    (v, stats)
}

/// The index of the first differing frame.
fn first_difference(a: &[Out], b: &[Out]) -> Option<usize> {
    (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i))
}

/// A test signal: tones, noise, clicks (short blocks), silence and loud passages (the limiter).
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
        let seg = (t * 1.3) as usize % 6;
        for c in 0..channels {
            let f = 220.0 * (1.0 + c as f64 * 0.37) * (1.0 + (t * 0.1).sin().abs());
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
                3 => 0.0,
                4 => 32000.0 * tone.signum() + 4000.0 * rnd(),
                _ => 6000.0 * tone + 6000.0 * (2.0 * core::f64::consts::PI * 3.0 * f * t).sin(),
            };
            v.push(s.clamp(-32768.0, 32767.0) as i16);
        }
    }
    v
}

#[cfg(feature = "encoder")]
mod enc {
    use super::signal;
    use fdk_aac_rs::encoder::{Aot, Bitrate, ChannelMode, Encoder, Transport};

    /// An encoded stream: the bytes of a byte-stream transport, or the access units and the
    /// configuration of the raw transport.
    pub struct Encoded {
        pub bytes: Vec<u8>,
        pub aus: Vec<Vec<u8>>,
        pub config: Vec<u8>,
    }

    pub fn encode(
        aot: Aot,
        rate: u32,
        channels: usize,
        bitrate: u32,
        transport: Transport,
        seconds: f64,
    ) -> Encoded {
        let mode = match channels {
            1 => ChannelMode::Mono,
            _ => ChannelMode::Stereo,
        };
        let mut e = Encoder::builder()
            .aot(aot)
            .sample_rate(rate)
            .channel_mode(mode)
            .bitrate(Bitrate::Cbr(bitrate))
            .transport(transport)
            .build()
            .unwrap();
        let pcm = signal(seconds, rate, channels, 7);
        let mut out = vec![0u8; 1 << 16];
        let config = e.info().unwrap().config().to_vec();
        let mut enc = Encoded { bytes: Vec::new(), aus: Vec::new(), config };
        let frame = 1024 * channels;
        for block in pcm.chunks(frame) {
            let r = e.encode(block, &mut out).unwrap();
            if r.out_bytes() > 0 {
                enc.bytes.extend_from_slice(&out[..r.out_bytes()]);
                enc.aus.push(out[..r.out_bytes()].to_vec());
            }
        }
        while let Ok(r) = e.flush(&mut out) {
            if r.out_bytes() == 0 {
                break;
            }
            enc.bytes.extend_from_slice(&out[..r.out_bytes()]);
            enc.aus.push(out[..r.out_bytes()].to_vec());
        }
        enc
    }
}

/// Exploration: `CHUNK_PROBE=file,transport,chunks,threads[;...]` (transport: adts, loas,
/// latm-mcp1, adif) compares the chunked with the sequential decoding of files and prints the
/// statistics (`FDK_AAC_RS_CHUNK_LOG`, `FDK_AAC_RS_CHUNK_DEBUG` for details).
#[test]
#[ignore]
fn probe_file() {
    let Ok(specs) = std::env::var("CHUNK_PROBE") else { return };
    let mut failed = Vec::new();
    for spec in specs.split(';') {
        eprintln!("== {spec}");
        if !probe_one(spec) {
            failed.push(spec.to_string());
        }
    }
    assert!(failed.is_empty(), "different output: {failed:?}");
}

fn probe_one(spec: &str) -> bool {
    let a: Vec<&str> = spec.split(',').collect();
    let data = std::fs::read(a[0]).unwrap();
    let transport = match a.get(1).copied().unwrap_or("adts") {
        "loas" => Transport::Loas,
        "latm-mcp1" => Transport::LatmMcp1,
        "adif" => Transport::Adif,
        _ => Transport::Adts,
    };
    let chunks: usize = a.get(2).map_or(0, |x| x.parse().unwrap());
    let threads: usize = a.get(3).map_or(8, |x| x.parse().unwrap());
    let setup = DecoderSetup::new(transport);
    let input = Input::Stream { data: &data, fill_size: 4096 };
    let t = std::time::Instant::now();
    let seq = sequential(&setup, input);
    let ts = t.elapsed();
    let opts = ChunkedOptions { threads, chunks, log: true, ..Default::default() };
    let t = std::time::Instant::now();
    let (chk, stats) = chunked(&setup, input, &opts);
    let tc = t.elapsed();
    let diff = first_difference(&seq, &chk);
    eprintln!(
        "sequential {ts:?}, chunked {tc:?}, {} handoffs, {} rejected, {} fallbacks, \
         sequential from {:?}, first difference {diff:?}",
        stats.handoffs, stats.rejected, stats.fallbacks, stats.sequential_from
    );
    diff.is_none()
}

/// Options that split even short inputs into many chunks.
fn forced(chunks: usize, threads: usize) -> ChunkedOptions {
    ChunkedOptions { threads, chunks, ..Default::default() }
}

/// Compares the chunked with the sequential decoding for several chunk and thread counts;
/// returns the number of handoffs.
fn check(setup: &DecoderSetup, input: Input, configs: &[(usize, usize)]) -> usize {
    check_stats(setup, input, configs).0
}

/// [`check`], returning the numbers of handoffs and fallbacks (hazards).
fn check_stats(setup: &DecoderSetup, input: Input, configs: &[(usize, usize)]) -> (usize, usize) {
    let seq = sequential(setup, input);
    assert!(!seq.is_empty());
    let mut handoffs = 0;
    let mut fallbacks = 0;
    for &(chunks, threads) in configs {
        let (chk, stats) = chunked(setup, input, &forced(chunks, threads));
        assert_eq!(
            first_difference(&seq, &chk),
            None,
            "{chunks} chunks, {threads} threads: {stats:?}, {} vs {} frames",
            seq.len(),
            chk.len()
        );
        handoffs += stats.handoffs;
        fallbacks += stats.fallbacks;
    }
    (handoffs, fallbacks)
}

#[cfg(feature = "encoder")]
mod with_encoder {
    use super::enc::encode;
    use super::*;
    use fdk_aac_rs::encoder::{Aot, Transport as EncTransport};

    /// AAC-LC without PNS: the states become equal, the boundaries are taken.
    #[test]
    fn aac_lc_adts() {
        let e = encode(Aot::AacLc, 44_100, 2, 192_000, EncTransport::Adts, 40.0);
        let setup = DecoderSetup::new(Transport::Adts);
        for fill_size in [4096, 1500] {
            let input = Input::Stream { data: &e.bytes, fill_size };
            let h = check(&setup, input, &[(2, 2), (5, 3), (9, 8), (20, 4)]);
            assert!(h > 0, "no boundary taken");
        }
    }

    /// AAC-LC at a low bit rate, with PNS: the PNS random seed never becomes equal, the output is
    /// the sequential one anyway.
    #[test]
    fn aac_lc_pns() {
        let e = encode(Aot::AacLc, 44_100, 2, 64_000, EncTransport::Adts, 30.0);
        let setup = DecoderSetup::new(Transport::Adts);
        let input = Input::Stream { data: &e.bytes, fill_size: 4096 };
        check(&setup, input, &[(4, 4), (9, 8)]);
    }

    /// Access units with an out-of-band configuration and ancillary data, many small chunks and
    /// verification points (and a tiny buffer limit: the speculative runs wait for their turn).
    #[test]
    fn aac_lc_packets() {
        let e = encode(Aot::AacLc, 48_000, 2, 128_000, EncTransport::Raw, 30.0);
        let aus: Vec<&[u8]> = e.aus.iter().map(|a| a.as_slice()).collect();
        let mut setup = DecoderSetup::new(Transport::Raw);
        setup.config = Some(e.config.clone());
        setup.ancillary_buffer = 512;
        let input = Input::Packets(&aus);
        assert!(check(&setup, input, &[(3, 2), (11, 6)]) > 0);
        let seq = sequential(&setup, input);
        let opts = ChunkedOptions {
            threads: 5,
            chunks: 12,
            verify_offsets: vec![0, 1, 3],
            max_buffered_bytes: 1,
            ..Default::default()
        };
        let (chk, stats) = chunked(&setup, input, &opts);
        assert_eq!(first_difference(&seq, &chk), None, "{stats:?}");
    }

    /// AAC-LD in LOAS.
    #[test]
    fn aac_ld_loas() {
        let e = encode(Aot::AacLd, 48_000, 2, 128_000, EncTransport::Loas, 20.0);
        let setup = DecoderSetup::new(Transport::Loas);
        let input = Input::Stream { data: &e.bytes, fill_size: 4096 };
        assert!(check(&setup, input, &[(4, 4), (13, 8)]) > 0);
    }

    /// HE-AAC and HE-AAC v2: the states of the SBR decoder never become equal (the noise and
    /// harmonic phases); the output is the sequential one anyway.
    #[test]
    fn he_aac() {
        for (aot, ch, br) in [(Aot::HeAac, 2, 64_000), (Aot::HeAacV2, 2, 32_000)] {
            let e = encode(aot, 44_100, ch, br, EncTransport::Adts, 20.0);
            let setup = DecoderSetup::new(Transport::Adts);
            let input = Input::Stream { data: &e.bytes, fill_size: 4096 };
            check(&setup, input, &[(6, 4), (12, 8)]);
        }
    }

    /// Damaged input: corrupt bytes and a cut near the chunk boundaries (errors, concealment:
    /// hazards, after which the true run takes over).
    #[test]
    fn damaged_input() {
        let e = encode(Aot::AacLc, 44_100, 2, 192_000, EncTransport::Adts, 40.0);
        let mut data = e.bytes.clone();
        let n = data.len();
        for k in 1..12 {
            let p = k * n / 12 + 37 * k;
            for b in &mut data[p..p + 40] {
                *b ^= 0x5a;
            }
        }
        data.drain(n / 3..n / 3 + 1000);
        let setup = DecoderSetup::new(Transport::Adts);
        let input = Input::Stream { data: &data, fill_size: 4096 };
        let (handoffs, fallbacks) = check_stats(&setup, input, &[(3, 2), (6, 4), (12, 8), (24, 8)]);
        assert!(handoffs > 0 && fallbacks > 0, "{handoffs} handoffs, {fallbacks} fallbacks");
    }

    /// Negative control: accepting every boundary without comparing the states gives a
    /// different output where the states differ (HE-AAC), and the comparison prevents it.
    #[test]
    fn negative_control_without_verification() {
        let e = encode(Aot::HeAac, 44_100, 2, 64_000, EncTransport::Adts, 20.0);
        let setup = DecoderSetup::new(Transport::Adts);
        let input = Input::Stream { data: &e.bytes, fill_size: 4096 };
        let seq = sequential(&setup, input);
        let opts = ChunkedOptions { threads: 4, chunks: 6, no_verify: true, ..Default::default() };
        let (chk, stats) = chunked(&setup, input, &opts);
        assert!(stats.handoffs > 0);
        assert!(first_difference(&seq, &chk).is_some(), "{stats:?}");
        check(&setup, input, &[(6, 4)]);
    }

    /// Automatic chunk count: none for few threads or short inputs.
    #[test]
    fn automatic_chunk_count() {
        let o = |threads| ChunkedOptions { threads, ..Default::default() };
        assert_eq!(o(1).num_chunks(100_000, 1), 1);
        assert_eq!(o(3).num_chunks(100_000, 1), 1);
        assert_eq!(o(8).num_chunks(200, 1), 1);
        assert_eq!(o(8).num_chunks(100_000, 1), 32);
        assert_eq!(o(8).num_chunks(1000, 1), 7);
        let e = encode(Aot::AacLc, 44_100, 2, 96_000, EncTransport::Adts, 10.0);
        let setup = DecoderSetup::new(Transport::Adts);
        let input = Input::Stream { data: &e.bytes, fill_size: 4096 };
        let (chk, stats) = chunked(&setup, input, &o(8));
        assert_eq!(stats.chunks, 1);
        assert_eq!(first_difference(&sequential(&setup, input), &chk), None);
    }
}

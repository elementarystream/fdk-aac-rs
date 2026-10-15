// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The opt-in multi-threading of the encoder: the output (every access unit and every
//! `AACENC_OutArgs`) must be that of the single-threaded encoder for every thread count, one
//! call later once the encoder delays its output (see `Encoder::output_delay`), also when the
//! count changes during a stream, after reconfigurations and encoding errors, and under load
//! (many encoders with random thread counts at once).

#![cfg(feature = "encoder")]

use core::ffi::c_void;
use fdk_aac_rs::encoder::{
    Aot, Bitrate, BitrateMode, ChannelMode, EncodeInput, Encoder, EncoderError, MetadataMode,
    Param, SbrMode, SbrRatio, Transport,
};
use fdk_aac_rs_aacenc::aacenc_lib as lib;
use fdk_aac_rs_fdk::par::{tune::MAX_MODES, Tuning};

/// A test signal that reaches the different paths of the psychoacoustic model and the
/// quantization: tones, noise (PNS), clicks (short blocks), silence (zero spectra), loud
/// passages, and channels that differ (M/S, intensity stereo, LFE).
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
        let seg = (t * 4.0) as usize % 6; // 250 ms segments
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
                3 => 0.0,
                4 => 32000.0 * tone.signum() * (c % 2) as f64 + 4000.0 * rnd(),
                _ => 6000.0 * tone + 6000.0 * (2.0 * core::f64::consts::PI * 3.0 * f * t).sin(),
            };
            v.push(s.clamp(-32768.0, 32767.0) as i16);
        }
    }
    v
}

/// One configuration of the safe API.
#[derive(Clone)]
struct Config {
    name: &'static str,
    aot: Aot,
    mode: ChannelMode,
    channels: usize,
    rate: u32,
    bitrate: Bitrate,
    /// further parameters (after the transport, so they may replace it)
    extra: Vec<Param>,
    /// ancillary data with every call
    anc: bool,
}

fn cfg(
    name: &'static str,
    aot: Aot,
    mode: ChannelMode,
    channels: usize,
    rate: u32,
    bitrate: Bitrate,
    extra: Vec<Param>,
) -> Config {
    Config { name, aot, mode, channels, rate, bitrate, extra, anc: false }
}

fn configs() -> Vec<Config> {
    use ChannelMode as M;
    vec![
        cfg("lc-stereo", Aot::AacLc, M::Stereo, 2, 44_100, Bitrate::Cbr(128_000), vec![]),
        cfg(
            "lc-stereo-noab-low",
            Aot::AacLc,
            M::Stereo,
            2,
            48_000,
            Bitrate::Cbr(24_000),
            vec![Param::Afterburner(false)],
        ),
        cfg(
            "lc-stereo-vbr",
            Aot::AacLc,
            M::Stereo,
            2,
            48_000,
            Bitrate::Vbr(BitrateMode::VbrHigh),
            vec![],
        ),
        cfg("lc-mono", Aot::AacLc, M::Mono, 1, 48_000, Bitrate::Cbr(64_000), vec![]),
        cfg("lc-3", Aot::AacLc, M::Mode1_2, 3, 48_000, Bitrate::Cbr(192_000), vec![]),
        cfg("lc-4", Aot::AacLc, M::Mode1_2_1, 4, 48_000, Bitrate::Cbr(160_000), vec![]),
        cfg("lc-5.1", Aot::AacLc, M::Mode1_2_2_1, 6, 48_000, Bitrate::Cbr(320_000), vec![]),
        cfg("lc-5.1-low", Aot::AacLc, M::Mode1_2_2_1, 6, 48_000, Bitrate::Cbr(64_000), vec![]),
        cfg("lc-7.1", Aot::AacLc, M::Mode7_1Back, 8, 48_000, Bitrate::Cbr(448_000), vec![]),
        cfg("ld-stereo", Aot::AacLd, M::Stereo, 2, 48_000, Bitrate::Cbr(96_000), vec![]),
        cfg(
            "eld-sbr-stereo",
            Aot::AacEld,
            M::Stereo,
            2,
            48_000,
            Bitrate::Cbr(64_000),
            vec![Param::SbrMode(SbrMode::Enabled)],
        ),
        cfg(
            "eld-sbr-dualrate-mono",
            Aot::AacEld,
            M::Mono,
            1,
            32_000,
            Bitrate::Cbr(48_000),
            vec![Param::SbrMode(SbrMode::Enabled), Param::SbrRatio(SbrRatio::DualRate)],
        ),
        cfg("eld-5.1", Aot::AacEld, M::Mode1_2_2_1, 6, 48_000, Bitrate::Cbr(256_000), vec![]),
        cfg("he-stereo", Aot::HeAac, M::Stereo, 2, 48_000, Bitrate::Cbr(64_000), vec![]),
        cfg(
            "he-stereo-latm",
            Aot::HeAac,
            M::Stereo,
            2,
            44_100,
            Bitrate::Cbr(48_000),
            vec![Param::Transmux(Transport::LatmMcp1), Param::HeaderPeriod(3)],
        ),
        cfg(
            "he-stereo-vbr-raw",
            Aot::HeAac,
            M::Stereo,
            2,
            32_000,
            Bitrate::Vbr(BitrateMode::VbrLow),
            vec![Param::Transmux(Transport::Raw)],
        ),
        cfg("he-mono", Aot::HeAac, M::Mono, 1, 24_000, Bitrate::Cbr(24_000), vec![]),
        cfg("he-5.1", Aot::HeAac, M::Mode1_2_2_1, 6, 48_000, Bitrate::Cbr(160_000), vec![]),
        cfg("hev2", Aot::HeAacV2, M::Stereo, 2, 48_000, Bitrate::Cbr(32_000), vec![]),
        cfg(
            "hev2-loas",
            Aot::HeAacV2,
            M::Stereo,
            2,
            44_100,
            Bitrate::Cbr(24_000),
            vec![Param::Transmux(Transport::Loas)],
        ),
        Config {
            anc: true,
            ..cfg(
                "he-stereo-anc-metadata",
                Aot::HeAac,
                M::Stereo,
                2,
                48_000,
                Bitrate::Cbr(96_000),
                vec![Param::MetadataMode(MetadataMode::DynamicRangeAndAncillary)],
            )
        },
        Config {
            anc: true,
            ..cfg(
                "hev2-anc",
                Aot::HeAacV2,
                M::Stereo,
                2,
                48_000,
                Bitrate::Cbr(48_000),
                vec![Param::AncillaryBitrate(2000)],
            )
        },
    ]
}

/// One call's result: the access unit, `numInSamples`, `bitResState` and `numAncBytes`.
type Rec = (Vec<u8>, usize, i32, usize);

/// Collects the results of a run in the order of the single-threaded encoder: with the output
/// delay a call returns the result of the previous one (the first delayed call: none), and
/// `numInSamples` belongs to the call itself.
#[derive(Default)]
struct Realign {
    res: Vec<Rec>,
    /// the call whose result is still to come
    pending: Option<usize>,
}

impl Realign {
    /// Adds a call: `delayed_before`/`delayed_after` the output delay before and after it;
    /// `out` its result (`None`: `AACENC_ENCODE_EOF`). Returns false when the single-threaded
    /// encoder would have returned the end of the flush.
    fn add(
        &mut self,
        delayed_before: bool,
        delayed_after: bool,
        in_samples: usize,
        out: Option<Rec>,
    ) -> bool {
        if !delayed_after {
            match out {
                Some(r) => self.res.push(r),
                None => return false,
            }
            return true;
        }
        let this = self.res.len();
        self.res.push((Vec::new(), in_samples, 0, 0));
        if delayed_before {
            let p = self.pending.expect("a delayed call follows a delayed call");
            match out {
                Some((bytes, _, br, anc)) => {
                    self.res[p].0 = bytes;
                    self.res[p].2 = br;
                    self.res[p].3 = anc;
                }
                None => {
                    // the end of the flush was the previous call's result
                    self.res.truncate(p);
                    return false;
                }
            }
        }
        self.pending = Some(this);
        true
    }
}

/// The results of an encode of `pcm` with `threads` threads (`switch`: the thread counts to
/// switch to every 7 calls instead), in the order of the single-threaded encoder.
fn encode(c: &Config, pcm: &[i16], threads: u32, switch: &[u32]) -> Vec<Rec> {
    encode_tuned(c, pcm, threads, switch, None).0
}

/// [`encode`] with the given choice of the thread mode (else the default); also returns the
/// frames per mode.
fn encode_tuned(
    c: &Config,
    pcm: &[i16],
    threads: u32,
    switch: &[u32],
    tuning: Option<Tuning>,
) -> (Vec<Rec>, [u64; MAX_MODES]) {
    let mut b = Encoder::builder()
        .aot(c.aot)
        .channel_mode(c.mode)
        .sample_rate(c.rate)
        .bitrate(c.bitrate)
        .transport(if matches!(c.aot, Aot::AacLd | Aot::AacEld) {
            Transport::Loas
        } else {
            Transport::Adts
        })
        .threads(threads);
    for p in &c.extra {
        b = b.param(*p);
    }
    let mut enc = b.build().unwrap_or_else(|e| panic!("{}: {e:?}", c.name));
    assert_eq!(enc.threads(), threads.max(1));
    if let Some(t) = tuning {
        enc.set_tuning(t);
    }
    let info = enc.info().unwrap();
    let mut out = vec![0u8; info.max_out_buf_bytes() as usize];
    let mut res = Realign::default();
    // calls of about 1.5 frames
    let chunk = (info.frame_length() as usize * 3 / 2) * c.channels;
    let mut input = pcm;
    let mut call = 0;
    let anc: Vec<u8> = (0..23u8).map(|i| i.wrapping_mul(37) ^ 0x5a).collect();
    while !input.is_empty() {
        if !switch.is_empty() && call % 7 == 0 {
            enc.set_threads(switch[(call / 7) % switch.len()]);
        }
        let before = enc.output_delay() != 0;
        let n = chunk.min(input.len());
        let a = if c.anc { &anc[..call % anc.len()] } else { &[][..] };
        let r = enc
            .encode_with(&EncodeInput { pcm: &input[..n], ancillary: a, metadata: None }, &mut out)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.name));
        call += 1;
        let rec = (out[..r.out_bytes()].to_vec(), r.in_samples(), r.bit_res_state(), r.anc_bytes());
        res.add(before, enc.output_delay() != 0, r.in_samples(), Some(rec));
        input = &input[r.in_samples()..];
    }
    loop {
        let before = enc.output_delay() != 0;
        let r = match enc.flush(&mut out) {
            Ok(r) => Some((
                out[..r.out_bytes()].to_vec(),
                r.in_samples(),
                r.bit_res_state(),
                r.anc_bytes(),
            )),
            Err(EncoderError::EncodeEof) => None,
            Err(e) => panic!("{}: flush: {e:?}", c.name),
        };
        if !res.add(before, enc.output_delay() != 0, 0, r) {
            break;
        }
    }
    // (the output delay: with more than one thread and SBR)
    let sbr = matches!(c.aot, Aot::HeAac | Aot::HeAacV2)
        || c.extra.contains(&Param::SbrMode(SbrMode::Enabled));
    let threaded = threads > 1 || switch.iter().any(|&t| t > 1);
    assert_eq!(enc.output_delay(), (sbr && threaded) as u32, "{}", c.name);
    (res.res, enc.mode_frames())
}

/// Every thread mode of the encoder, and every switch between two modes (the cycle schedule of
/// `fdk_aac_rs_fdk::par::tune`), gives the output of one thread.
#[test]
fn same_output_in_every_thread_mode() {
    for (k, c) in configs().iter().enumerate() {
        let pcm = signal(1.6, c.rate, c.channels, 0x4321 + k as u32);
        let reference = encode(c, &pcm, 1, &[]);
        for threads in [2, 4] {
            let (got, per_mode) = encode_tuned(c, &pcm, threads, &[], Some(Tuning::Cycle));
            assert!(got == reference, "{}: {threads} threads, cycle", c.name);
            // (the modes of the configuration: 1, 2 or 4)
            let modes = per_mode.iter().filter(|&&n| n > 0).count();
            assert!(modes != 0 && modes != 3, "{}: {per_mode:?}", c.name);
            for m in 0..4u8 {
                let (got, _) = encode_tuned(c, &pcm, threads, &[], Some(Tuning::Fixed(m)));
                assert!(got == reference, "{}: {threads} threads, mode {m}", c.name);
            }
        }
    }
}

#[test]
fn same_output_for_every_thread_count() {
    for (k, c) in configs().iter().enumerate() {
        let pcm = signal(1.6, c.rate, c.channels, 0x1234 + k as u32);
        let reference = encode(c, &pcm, 1, &[]);
        assert!(reference.iter().map(|r| r.0.len()).sum::<usize>() > 0, "{}", c.name);
        for threads in [2, 3, 4, 8] {
            assert!(encode(c, &pcm, threads, &[]) == reference, "{}: {threads} threads", c.name);
        }
        // the thread count changed during the stream (the pools are replaced; the output delay
        // starts with the first call or with the switch from 1 thread)
        assert!(encode(c, &pcm, 1, &[3, 1, 2, 8]) == reference, "{}: switching", c.name);
        assert!(encode(c, &pcm, 1, &[1, 4, 1]) == reference, "{}: switching from 1", c.name);
    }
}

/// Many encoders with random thread counts at once (more threads than cores): every output
/// equals the single-threaded one.
#[test]
fn stress_concurrent_encoders() {
    let cfgs = configs();
    let inputs: Vec<Vec<i16>> = cfgs
        .iter()
        .enumerate()
        .map(|(k, c)| signal(0.8, c.rate, c.channels, 99 + k as u32))
        .collect();
    let expected: Vec<_> = cfgs.iter().zip(&inputs).map(|(c, p)| encode(c, p, 1, &[])).collect();
    let workers = 12;
    std::thread::scope(|s| {
        for w in 0..workers {
            let (cfgs, inputs, expected) = (&cfgs, &inputs, &expected);
            s.spawn(move || {
                let mut x = 0x9e37_79b9u32 ^ (w as u32 * 7919);
                let mut rnd = move |n: u32| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x % n
                };
                for _ in 0..6 {
                    let k = rnd(cfgs.len() as u32) as usize;
                    let threads = 1 + rnd(8);
                    let got = encode(&cfgs[k], &inputs[k], threads, &[]);
                    assert!(got == expected[k], "{}: {threads} threads", cfgs[k].name);
                }
            });
        }
    });
}

// ---- The straight-port API: errors, reconfigurations, aacEncInfo() between calls ------------

/// What a script does before a call.
#[derive(Clone, Copy, Debug)]
enum Op {
    /// `aacEncoder_SetParam()`
    Set(u32, u32),
    /// `aacEncInfo()`
    Info,
    /// `aacEncEncode(h, NULL, NULL, NULL, NULL)`
    Init,
}

/// A call's result with the straight-port API: return code, `AACENC_OutArgs` (with
/// `numInSamples`) and the whole output buffer. For [`Op`]s: their return codes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Encode(u32, lib::AacencOutArgs, Vec<u8>),
    Op(u32),
}

/// A script: `calls` `aacEncEncode()` calls with an output buffer of `out_size` bytes, call
/// `k` with `chunk(k, frame)` samples (`frame`: the samples of a frame; the input advances by
/// `numInSamples`, and at its end the calls flush), and `ops(k)` before call `k`.
struct Script<'a> {
    out_size: usize,
    calls: usize,
    chunk: &'a dyn Fn(usize, usize) -> usize,
    ops: &'a dyn Fn(usize) -> Vec<Op>,
}

/// Runs a script on `pcm` and returns the events in the order of the single-threaded encoder,
/// what the thread-local counters counted, and the pipeline statistics.
fn drive(
    params: &[(u32, u32)],
    channels: usize,
    pcm: &[i16],
    threads: u32,
    sc: &Script<'_>,
) -> (Vec<Event>, Vec<u64>, (u64, u64)) {
    let (out_size, calls, ops) = (sc.out_size, sc.calls, sc.ops);
    let before = counters();
    let mut h: Option<Box<lib::AacEncoder>> = None;
    assert_eq!(lib::aac_enc_open(Some(&mut h), 0, channels as u32), lib::AACENC_OK);
    for &(id, v) in params {
        lib::aac_encoder_set_param(h.as_deref_mut(), id as lib::AacencParam, v);
    }
    lib::aac_enc_set_threads(h.as_deref_mut().unwrap(), threads);
    // (the scripts test the pipelining: the mode with every parallel feature)
    lib::aac_enc_set_tuning(h.as_deref_mut().unwrap(), Tuning::FULL);
    // SAFETY: no buffers.
    let r = unsafe { lib::aac_enc_encode(h.as_deref_mut(), None, None, None, None) };
    assert_eq!(r, lib::AACENC_OK);
    let frame = {
        let mut info = lib::AacencInfoStruct::default();
        lib::aac_enc_info(h.as_deref_mut(), Some(&mut info));
        info.frame_length as usize * channels
    };

    let mut events: Vec<Event> = Vec::new();
    // the index of the pending call's event, and the events after it (they follow the result)
    let mut pending: Option<usize> = None;
    let mut out = vec![0u8; out_size];
    let mut anc = [0u8; 40];
    let mut pos = 0usize;
    for call in 0..calls {
        for op in ops(call) {
            let r = match op {
                Op::Set(id, v) => {
                    lib::aac_encoder_set_param(h.as_deref_mut(), id as lib::AacencParam, v)
                }
                Op::Info => {
                    let mut info = lib::AacencInfoStruct::default();
                    lib::aac_enc_info(h.as_deref_mut(), Some(&mut info))
                }
                // SAFETY: no buffers.
                Op::Init => unsafe {
                    lib::aac_enc_encode(h.as_deref_mut(), None, None, None, None)
                },
            };
            events.push(Event::Op(r as u32));
        }
        let n = (sc.chunk)(call, frame).min(pcm.len() - pos);
        for (i, a) in anc.iter_mut().enumerate() {
            *a = (call * 7 + i) as u8;
        }
        let mut in_ptr: [*mut c_void; 2] =
            [pcm[pos..].as_ptr() as *mut c_void, anc.as_mut_ptr() as *mut c_void];
        let mut in_id = [lib::IN_AUDIO_DATA, lib::IN_ANCILLRY_DATA];
        let mut in_size = [(n * 2) as i32, anc.len() as i32];
        let mut in_el = [2i32, 1];
        let in_desc = lib::AacencBufDesc {
            num_bufs: 2,
            bufs: in_ptr.as_mut_ptr(),
            buffer_identifiers: in_id.as_mut_ptr(),
            buf_sizes: in_size.as_mut_ptr(),
            buf_el_sizes: in_el.as_mut_ptr(),
        };
        let mut out_ptr = out.as_mut_ptr() as *mut c_void;
        let mut out_id = lib::OUT_BITSTREAM_DATA;
        let mut out_sz = out_size as i32;
        let mut out_el = 1i32;
        let out_desc = lib::AacencBufDesc {
            num_bufs: 1,
            bufs: &mut out_ptr,
            buffer_identifiers: &mut out_id,
            buf_sizes: &mut out_sz,
            buf_el_sizes: &mut out_el,
        };
        let in_args = lib::AacencInArgs {
            num_in_samples: if n > 0 { n as i32 } else { -1 },
            num_anc_bytes: (call % 4 * 10) as i32,
        };
        out.fill(0xa5);
        let mut out_args = lib::AacencOutArgs {
            num_out_bytes: -7,
            num_in_samples: -7,
            num_anc_bytes: -7,
            bit_res_state: -7,
        };
        let before = lib::aac_enc_get_output_delay(h.as_deref().unwrap()) != 0;
        // SAFETY: the descriptors point to live buffers of the announced sizes.
        let r = unsafe {
            lib::aac_enc_encode(
                h.as_deref_mut(),
                Some(&in_desc),
                Some(&out_desc),
                Some(&in_args),
                Some(&mut out_args),
            )
        };
        let after = lib::aac_enc_get_output_delay(h.as_deref().unwrap()) != 0;
        pos += out_args.num_in_samples.max(0) as usize;
        if !after {
            events.push(Event::Encode(r as u32, out_args, out.clone()));
            continue;
        }
        // this call's event, completed by the next call
        let this = events.len();
        let mut own = out_args;
        own.num_out_bytes = 0;
        own.num_anc_bytes = 0;
        own.bit_res_state = 0;
        events.push(Event::Encode(0, own, Vec::new()));
        if before {
            let p = pending.unwrap();
            if let Event::Encode(ret, args, buf) = &mut events[p] {
                *ret = r as u32;
                args.num_out_bytes = out_args.num_out_bytes;
                args.num_anc_bytes = out_args.num_anc_bytes;
                args.bit_res_state = out_args.bit_res_state;
                *buf = out.clone();
            }
        }
        pending = Some(this);
    }
    if let Some(p) = pending {
        // (the last call's result is not known)
        events.truncate(p);
    }
    // (aacEncInfo() runs the core of a pending frame)
    let mut info = lib::AacencInfoStruct::default();
    lib::aac_enc_info(h.as_deref_mut(), Some(&mut info));
    let after = counters();
    let stats = lib::aac_enc_get_pipeline_stats(h.as_deref().unwrap());
    lib::aac_enc_close(Some(&mut h));
    let counted = after.iter().zip(&before).map(|(a, b)| a.wrapping_sub(*b)).collect();
    (events, counted, stats)
}

/// The thread-local deviation and test counters of the encoder (carried from the workers to
/// the calling thread; the pipelining takes back what an undone speculative part counted).
fn counters() -> Vec<u64> {
    use fdk_aac_rs_aacenc::{bit_cnt, metadata_main, psy_main, qc_main};
    let (w, r, n) = fdk_aac_rs_fdk::mdct::mdct_block_dropped_accesses();
    vec![
        fdk_aac_rs_fdk::fdk_assert::violations(),
        w,
        r,
        n,
        bit_cnt::huff_rom_outside_reads(),
        qc_main::channel_element_write_calls(),
        metadata_main::metadata_input_overruns(),
        psy_main::psy_main_stale_element_mappings(),
        psy_main::psy_main_negative_lowpass_clears(),
        psy_main::psy_main_negative_final_shifts(),
        psy_main::psy_main_stale_short_blocks(),
        psy_main::psy_main_mixed_block_pairs(),
    ]
}

/// Runs a script with 1 thread and with `threads`: the events must be equal (the
/// single-threaded run without its last call, whose delayed counterpart is unknown).
fn check_script(
    name: &str,
    params: &[(u32, u32)],
    channels: usize,
    rate: u32,
    sc: &Script<'_>,
) -> (u64, u64) {
    let pcm = signal(1.2, rate, channels, 77);
    let (mut reference, ref_counted, _) = drive(params, channels, &pcm, 1, sc);
    // without the events of the last call
    while let Some(Event::Op(_)) = reference.last() {
        reference.pop();
    }
    reference.pop();
    while let Some(Event::Op(_)) = reference.last() {
        reference.pop();
    }
    let mut stats = (0, 0);
    for threads in [2, 4] {
        let (mut got, counted, s) = drive(params, channels, &pcm, threads, sc);
        assert_eq!(counted, ref_counted, "{name}: {threads} threads, counters");
        while let Some(Event::Op(_)) = got.last() {
            got.pop();
        }
        assert_eq!(got.len(), reference.len(), "{name}: {threads} threads");
        for (i, (a, b)) in reference.iter().zip(&got).enumerate() {
            assert_eq!(a, b, "{name}: {threads} threads, event {i}");
        }
        stats = (stats.0 + s.0, stats.1 + s.1);
    }
    stats
}

const AOT: u32 = lib::AACENC_AOT as u32;
const RATE: u32 = lib::AACENC_SAMPLERATE as u32;
const MODE: u32 = lib::AACENC_CHANNELMODE as u32;
const BITRATE: u32 = lib::AACENC_BITRATE as u32;
const TRANSMUX: u32 = lib::AACENC_TRANSMUX as u32;
const CONTROL: u32 = lib::AACENC_CONTROL_STATE as u32;
const SBR_MODE: u32 = lib::AACENC_SBR_MODE as u32;

/// A frame and a little more per call (each call completes a frame).
fn frame_plus(call: usize, frame: usize) -> usize {
    frame + 6 * (call % 5)
}

/// Output buffers too small for some access units: the encoding errors (which make the next
/// call reinitialize the encoder) come one call later, and the pipelined calls after them are
/// undone; everything else is unchanged.
#[test]
fn encoding_errors_and_reinitialization() {
    let none = |_: usize| vec![];
    let he = [(AOT, 5), (RATE, 48_000), (MODE, 2), (BITRATE, 64_000), (TRANSMUX, 2)];
    let sc = Script { out_size: 360, calls: 80, chunk: &frame_plus, ops: &none };
    let (overlapped, undone) = check_script("he small buffer", &he, 2, 48_000, &sc);
    assert!(overlapped > 0 && undone > 0, "{overlapped} {undone}");
    // ELD with SBR, LOAS (in-band configuration with the SBR header call-back)
    let eld =
        [(AOT, 39), (SBR_MODE, 1), (RATE, 48_000), (MODE, 2), (BITRATE, 48_000), (TRANSMUX, 10)];
    let sc = Script { out_size: 100, calls: 200, chunk: &frame_plus, ops: &none };
    let (overlapped, undone) = check_script("eld-sbr small buffer", &eld, 2, 48_000, &sc);
    assert!(overlapped > 0 && undone > 0, "{overlapped} {undone}");
    // calls of 2/3 frame (no pipelining while a call does not complete a frame)
    let sc = Script {
        out_size: 360,
        calls: 120,
        chunk: &|call, frame| frame * 2 / 3 + 6 * (call % 5),
        ops: &none,
    };
    check_script("he small buffer, short calls", &he, 2, 48_000, &sc);
}

/// Parameter changes, aacEncInfo() and initialization calls between the calls, and
/// AACENC_CONTROL_STATE (with AACENC_RESET_INBUFFER).
#[test]
fn reconfiguration_between_calls() {
    let he = [(AOT, 5), (RATE, 44_100), (MODE, 2), (BITRATE, 64_000), (TRANSMUX, 6)];
    let ops = |call: usize| match call {
        9 => vec![Op::Info],
        14 => vec![Op::Set(BITRATE, 48_000)],
        20 => vec![Op::Set(BITRATE, 56_000), Op::Init],
        27 => vec![Op::Set(AOT, 29)],
        33 => vec![Op::Set(CONTROL, 0x7 | 0x2000)],
        41 => vec![Op::Set(AOT, 2), Op::Info],
        50 => vec![Op::Set(AOT, 5), Op::Set(MODE, 1)],
        58 => vec![Op::Info, Op::Set(TRANSMUX, 2)],
        _ if call % 11 == 0 => vec![Op::Info],
        _ => vec![],
    };
    let chunk =
        |call: usize, frame: usize| if call % 7 == 3 { frame / 3 } else { frame_plus(call, frame) };
    let sc = Script { out_size: 2048, calls: 90, chunk: &chunk, ops: &ops };
    let (overlapped, _) = check_script("he reconfigured", &he, 2, 44_100, &sc);
    assert!(overlapped > 0);
    // an invalid configuration on the way (initialization errors in a delayed encoder)
    let ops = |call: usize| match call {
        8 => vec![Op::Set(BITRATE, 1)],
        12 => vec![Op::Set(BITRATE, 64_000)],
        16 => vec![Op::Set(CONTROL, 0)],
        _ => vec![],
    };
    let sc = Script { out_size: 2048, calls: 40, chunk: &frame_plus, ops: &ops };
    check_script("he invalid", &he, 2, 44_100, &sc);
    // AAC-LC (no output delay) switched to HE-AAC (the delay starts) and back
    let lc = [(AOT, 2), (RATE, 44_100), (MODE, 2), (BITRATE, 96_000), (TRANSMUX, 2)];
    let ops = |call: usize| match call {
        10 => vec![Op::Set(AOT, 5), Op::Set(BITRATE, 48_000)],
        25 => vec![Op::Set(AOT, 2)],
        _ => vec![],
    };
    // (calls of two AAC-LC frames: one HE-AAC frame)
    let chunk = |call: usize, frame: usize| frame_plus(call, 2 * frame);
    let sc = Script { out_size: 2048, calls: 40, chunk: &chunk, ops: &ops };
    let (overlapped, _) = check_script("lc to he", &lc, 2, 44_100, &sc);
    assert!(overlapped > 0);
}

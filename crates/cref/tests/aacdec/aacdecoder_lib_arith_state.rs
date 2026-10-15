// Differential tests of the decoder (through its public API, see aacdecoder_lib.rs) for the
// state of the USAC arithmetic decoder across changes of the core frame length.
//
// CArco_DecodeArithData() maps the context of the previous spectrum (m_numberLinesPrev lines)
// to the current one (lg_max lines). Within one core frame length N (768 or 1024) the lengths
// are N (FD long, TCX 80), N/8 (FD short), N/4 and N/2 (TCX 20, TCX 40), so the ratio is always
// 2, 4 or 8. A change of N is a configuration change: the transport decoder calls
// aacDecoder_FreeMemCallback() and runs the AC_CM_ALLOC_MEM pass of CAacDecoder_Init(), which
// creates a zeroed CArcoData (CArco_Create()) and sets granuleLength for every channel.
//
// The tests switch between USAC streams with core frame lengths 768 and 1024 in the middle of
// the streams (mostly at frames without usacIndependencyFlag, which keep the arithmetic
// context), through ConfigRaw and through the StreamMuxConfig of a LOAS stream. Besides
// comparing every call with the reference, they check in the port after every call that the
// arithmetic state of each channel is consistent with the current core frame length, and after
// every configuration change of the frame length that it was reset.

use crate::aacdecoder_lib::{demux, every, usac_mp4, Pair};
use fdk_aac_rs_aacdec::aacdecoder::AacDecoderInstance;
use fdk_aac_rs_aacdec::aacdecoder_lib::{is_output_valid, AAC_DEC_NOT_ENOUGH_BITS};
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::{TT_MP4_LOAS, TT_MP4_RAW};

/// The core frame length of a USAC conformance file name (`_c<coreSbrFrameLengthIndex>_`).
fn core_length(name: &str) -> Option<i32> {
    let i = name.find("_c")?;
    match name.as_bytes().get(i + 2)? {
        b'0' | b'2' => Some(768),
        b'1' | b'3' | b'4' => Some(1024),
        _ => None,
    }
}

/// Checks the arithmetic decoder state of every channel against the core frame length; returns
/// the number of channels with an arithmetic decoder.
fn check_arith_state(d: &AacDecoderInstance, ctx: &str) -> usize {
    let n = d.stream_info.aac_samples_per_frame;
    let mut channels = 0;
    for ch in 0..d.p_aac_decoder_static_channel_info.len() {
        let Some(sci) = d.p_aac_decoder_static_channel_info[ch].as_deref() else { continue };
        let Some(arco) = sci.h_ar_co.as_deref() else { continue };
        channels += 1;
        let prev = arco.m_number_lines_prev as i32;
        assert!(
            [0, n, n / 2, n / 4, n / 8].contains(&prev),
            "{ctx}: channel {ch}: m_numberLinesPrev {prev} with a core frame length of {n}"
        );
        if let Some(ci) = d.p_aac_decoder_channel_info[ch].as_deref() {
            assert_eq!(ci.granule_length, n / 8, "{ctx}: channel {ch}: granuleLength");
        }
    }
    channels
}

/// Checks that the arithmetic decoder state of every channel is the zeroed state of
/// CArco_Create().
fn check_arith_reset(d: &AacDecoderInstance, ctx: &str) {
    for ch in 0..d.p_aac_decoder_static_channel_info.len() {
        let Some(sci) = d.p_aac_decoder_static_channel_info[ch].as_deref() else { continue };
        let Some(arco) = sci.h_ar_co.as_deref() else { continue };
        assert_eq!(arco.m_number_lines_prev, 0, "{ctx}: channel {ch}: m_numberLinesPrev after the change");
        assert!(arco.c_prev.iter().all(|&c| c == 0), "{ctx}: channel {ch}: c_prev after the change");
    }
}

/// Feeds access units one by one (Fill and DecodeFrame), checking the arithmetic state after
/// every call. Returns the number of DecodeFrame calls.
fn feed(pair: &mut Pair, aus: &[Vec<u8>], ctx: &str) -> usize {
    let mut n = 0;
    for au in aus {
        pair.fill(au);
        pair.decode(0);
        check_arith_state(pair.port(), ctx);
        n += 1;
    }
    n
}

#[test]
fn arith_state_across_frame_length_changes_config_raw() {
    // FD, LPD, eSBR and MPS212 streams with both core frame lengths.
    let files = usac_mp4(|n| {
        (n.starts_with("Fd_") || n.starts_with("Lpd_") || n.starts_with("eSbr_") || n.starts_with("Mps_"))
            && core_length(n).is_some()
    });
    let short: Vec<String> = every(&files.iter().filter(|f| core_length(f) == Some(768)).cloned().collect::<Vec<_>>(), 9);
    let long: Vec<String> = every(&files.iter().filter(|f| core_length(f) == Some(1024)).cloned().collect::<Vec<_>>(), 9);
    let load = |v: &[String]| -> Vec<(String, Vec<u8>, Vec<Vec<u8>>)> {
        v.iter().filter_map(|f| demux(f).map(|(asc, aus)| (f.clone(), asc, aus))).filter(|(_, _, aus)| aus.len() > 12).collect()
    };
    let (short, long) = (load(&short), load(&long));
    if short.is_empty() || long.is_empty() {
        eprintln!("arith_state_across_frame_length_changes_config_raw: USAC conformance streams missing, skipped");
        return;
    }
    let (mut switches, mut calls, mut arith_channels) = (0, 0, 0);
    for seed in 0..12u64 {
        let mut rng = Rng::new(0x3131 + seed);
        let mut pair = Pair::open(TT_MP4_RAW, 1, &format!("arith state, sequence {seed}")).unwrap();
        let mut prev_len = 0;
        for step in 0..10 {
            // alternate the core frame length
            let group = if step % 2 == (seed % 2) as usize { &short } else { &long };
            let (name, asc, aus) = &group[(rng.next_u32() as usize) % group.len()];
            let ctx = format!("arith state, sequence {seed}, step {step}: {name}");
            pair.ctx = ctx.clone();
            pair.config_raw(asc);
            let len = pair.port().stream_info.aac_samples_per_frame;
            if prev_len != 0 && len != prev_len {
                // the frame length changed: the arithmetic decoder state must be fresh
                check_arith_reset(pair.port(), &ctx);
                switches += 1;
            }
            prev_len = len;
            // continue somewhere in the middle of the stream (usually a frame that depends on
            // the arithmetic context of the previous one)
            let start = 1 + (rng.next_u32() as usize) % (aus.len() - 10);
            let count = 3 + (rng.next_u32() as usize) % 6;
            calls += feed(&mut pair, &aus[start..start + count], &ctx);
            arith_channels += check_arith_state(pair.port(), &ctx);
        }
    }
    eprintln!(
        "arith_state_across_frame_length_changes_config_raw: {} + {} streams, {switches} frame length changes, {calls} DecodeFrame calls, {arith_channels} channel states checked",
        short.len(),
        long.len()
    );
    assert!(switches > 0 && arith_channels > 0);
}

/// A bit writer (MSB first).
#[derive(Default)]
struct Bits {
    v: Vec<u8>,
    n: usize,
}

impl Bits {
    fn put(&mut self, x: u32, n: u32) {
        for i in (0..n).rev() {
            if self.n % 8 == 0 {
                self.v.push(0);
            }
            if (x >> i) & 1 != 0 {
                *self.v.last_mut().unwrap() |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }

    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.put(x as u32, 8);
        }
    }

    /// LatmGetValue(): bytesForValue (2 bits) and the value in whole bytes.
    fn latm_value(&mut self, v: u32) {
        let nb = if v < 1 << 8 { 1 } else if v < 1 << 16 { 2 } else { 3 };
        self.put(nb - 1, 2);
        self.put(v, 8 * nb);
    }
}

/// One LOAS frame (AudioSyncStream) with one access unit: AudioMuxElement(1) with a
/// StreamMuxConfig (audioMuxVersion 1, the AudioSpecificConfig with its length) if `asc` is
/// given, else useSameStreamMux.
fn loas_frame(asc: Option<&[u8]>, au: &[u8]) -> Vec<u8> {
    let mut m = Bits::default();
    match asc {
        Some(asc) => {
            m.put(0, 1); // useSameStreamMux
            m.put(1, 1); // audioMuxVersion
            m.put(0, 1); // audioMuxVersionA
            m.latm_value(0xff); // taraBufferFullness
            m.put(1, 1); // allStreamsSameTimeFraming
            m.put(0, 6); // numSubFrames - 1
            m.put(0, 4); // numProgram - 1
            m.put(0, 3); // numLayer - 1
            m.latm_value(asc.len() as u32 * 8); // ascLen
            m.bytes(asc);
            m.put(0, 3); // frameLengthType
            m.put(0xff, 8); // latmBufferFullness
            m.put(0, 1); // otherDataPresent
            m.put(0, 1); // crcCheckPresent
        }
        None => m.put(1, 1),
    }
    let mut len = au.len();
    while len >= 255 {
        m.put(255, 8);
        len -= 255;
    }
    m.put(len as u32, 8);
    m.bytes(au);
    let mut f = Bits::default();
    f.put(0x2b7, 11);
    f.put(m.v.len() as u32, 13);
    f.bytes(&m.v);
    f.v
}

#[test]
fn arith_state_across_frame_length_changes_loas() {
    // The streams of both core frame lengths in LOAS, one after the other in one byte stream,
    // the second without its first access units: the change is detected in the LOAS
    // StreamMuxConfig (CLatmDemux_ReadStreamMuxConfig(), AC_CM_DET_CFG_CHANGE, then
    // aacDecoder_FreeMemCallback() and AC_CM_ALLOC_MEM).
    let files = usac_mp4(|n| (n.starts_with("Fd_") || n.starts_with("Lpd_")) && core_length(n).is_some());
    let short: Vec<String> = every(&files.iter().filter(|f| core_length(f) == Some(768)).cloned().collect::<Vec<_>>(), 15);
    let long: Vec<String> = every(&files.iter().filter(|f| core_length(f) == Some(1024)).cloned().collect::<Vec<_>>(), 15);
    let load = |v: &[String]| -> Vec<(String, Vec<u8>, Vec<Vec<u8>>)> {
        v.iter().filter_map(|f| demux(f).map(|(asc, aus)| (f.clone(), asc, aus))).filter(|(_, _, aus)| aus.len() > 30).collect()
    };
    let (short, long) = (load(&short), load(&long));
    if short.is_empty() || long.is_empty() {
        eprintln!("arith_state_across_frame_length_changes_loas: USAC conformance streams missing, skipped");
        return;
    }
    let (mut runs, mut switches, mut valid_after, mut calls) = (0, 0, 0, 0);
    let mut rng = Rng::new(0x3132);
    for i in 0..short.len().max(long.len()) * 2 {
        let (a, b) = (&short[i % short.len()], &long[(i / 2) % long.len()]);
        let (first, second) = if i % 2 == 0 { (a, b) } else { (b, a) };
        let mut data = Vec::new();
        for (k, au) in first.2.iter().take(12).enumerate() {
            data.extend(loas_frame(if k % 5 == 0 { Some(&first.1) } else { None }, au));
        }
        // the second stream from a frame in the middle, with its configuration in every frame
        // or only in the first
        let start = 1 + (rng.next_u32() as usize) % 15;
        let every_frame = rng.next_u32() % 2 == 0;
        for (k, au) in second.2[start..start + 12].iter().enumerate() {
            data.extend(loas_frame(if k == 0 || every_frame { Some(&second.1) } else { None }, au));
        }
        let ctx = format!("arith state loas {i}: {} -> {} from {start}", first.0, second.0);
        let mut pair = Pair::open(TT_MP4_LOAS, 1, &ctx).unwrap();
        let len1 = core_length(&first.0).unwrap();
        let mut seen_first = false;
        let mut pos = 0;
        while pos < data.len() && pair.n_dec < 200 {
            let chunk = (data.len() - pos).min(1 + (rng.next_u32() as usize) % 3000);
            pos += pair.fill(&data[pos..pos + chunk]) as usize;
            for _ in 0..30 {
                let before = pair.port().stream_info.aac_samples_per_frame;
                let r = pair.decode(0);
                calls += 1;
                let after = pair.port().stream_info.aac_samples_per_frame;
                check_arith_state(pair.port(), &ctx);
                if after == len1 {
                    seen_first = true;
                }
                if before == len1 && after != len1 && after != 0 {
                    switches += 1;
                }
                if seen_first && after != len1 && is_output_valid(r) != 0 {
                    valid_after += 1;
                }
                if r == AAC_DEC_NOT_ENOUGH_BITS {
                    break;
                }
            }
        }
        runs += 1;
    }
    eprintln!(
        "arith_state_across_frame_length_changes_loas: {runs} runs, {switches} frame length changes, {valid_after} valid frames after them, {calls} DecodeFrame calls"
    );
    assert!(switches > 0 && valid_after > 0);
}

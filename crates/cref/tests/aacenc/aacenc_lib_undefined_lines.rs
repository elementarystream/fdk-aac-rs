// Differential tests of fdk_aac_rs_aacenc::aacenc_lib: spectral lines without a defined value in
// the reference that a later frame reads.
//
// FDKaacEnc_groupShortData() leaves the lines of the bands above sfbActive of every group of a
// short block without a defined value; the port writes 0 there and records the lines
// (QcOut::mdct_undefined). The port counts a frame that reads such lines
// (psy_main_undefined_spectrum_reads), and the comparison stops before it (Pair::stop_undefined).

use crate::aacenc_lib::{signal, EncCall, Pair};
use crate::adj_thr_enc::Signal;
use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_aacenc::psy_main::psy_main_undefined_spectrum_reads;

/// AAC-LC stereo frames of `lc_sig`, then AAC-ELD 16 kHz with 240 lines, compared frame by frame
/// until the port counts a read of undefined lines. Returns (Some(whether it stopped at a read of
/// undefined lines) if it stopped, None if not; ELD frames compared). Without
/// AACENC_INIT_STATES, the comparison may stop at another counter of `deviation_events` first.
fn lc_then_eld_240(
    what: &str,
    lc_bitrate: u32,
    lc_sig: Signal,
    lc_frames: usize,
    init_flags: Option<u32>,
) -> (Option<bool>, usize) {
    let mut p = Pair::open(0, 2, what).unwrap();
    p.stop_undefined = true;
    for &(a, v) in &[
        (AACENC_AOT, 2),
        (AACENC_SAMPLERATE, 44100),
        (AACENC_CHANNELMODE, 2),
        (AACENC_BITRATE, lc_bitrate),
        (AACENC_TRANSMUX, 10),
    ] {
        p.set(a, v);
    }
    let pcm = signal(lc_sig, 44100, 2, 1024 * lc_frames);
    for f in pcm.chunks(2048) {
        assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK, "{what}");
        assert!(!p.stopped_undefined, "{what}");
    }
    for &(a, v) in &[
        (AACENC_AOT, 39),
        (AACENC_SAMPLERATE, 16000),
        (AACENC_GRANULE_LENGTH, 240),
        (AACENC_BITRATE, 64000),
    ] {
        p.set(a, v);
    }
    if let Some(f) = init_flags {
        p.set(AACENC_CONTROL_STATE, f);
    }
    assert_eq!(p.init(), AACENC_OK, "{what}");
    let before = psy_main_undefined_spectrum_reads();
    let pcm = signal(Signal::Music(0), 16000, 2, 240 * 16);
    let mut frames = 0;
    for f in pcm.chunks(480) {
        let e = p.encode(&EncCall::pcm(f.to_vec(), 20480)).0;
        if p.stopped_undefined {
            return (Some(psy_main_undefined_spectrum_reads() > before), frames);
        }
        assert_eq!(e, AACENC_OK, "{what}");
        frames += 1;
    }
    assert_eq!(psy_main_undefined_spectrum_reads(), before, "{what}");
    (None, frames)
}

/// AAC-LC frames, then AAC-ELD with 240 lines: the comparison stops before a frame in which the
/// port counts a read of undefined lines; otherwise every frame is compared.
#[test]
fn eld_240_reads_lines_of_short_blocks() {
    let mut stopped = 0;
    let mut compared = 0;
    for (k, (sig, frames)) in [
        (Signal::Clicks, 4),
        (Signal::Clicks, 6),
        (Signal::Square, 5),
        (Signal::Clicks, 9),
        (Signal::Music(0), 4),
        (Signal::MultiTone, 4),
        (Signal::Silence, 3),
    ]
    .into_iter()
    .enumerate()
    {
        for br in [24000u32, 48000, 128000] {
            for init in [None, Some(0x2001u32), Some(0xFFFF)] {
                let what =
                    format!("eld_240_reads_lines_of_short_blocks {k} {sig:?} {frames} {br} {init:?}");
                let (s, n) = lc_then_eld_240(&what, br, sig, frames, init);
                match s {
                    Some(true) => stopped += 1,
                    Some(false) => assert_eq!(init, Some(0x2001), "{what}: another deviation"),
                    None => compared += n,
                }
            }
        }
    }
    // (measured on aarch64: 12 of the 63 sequences stop there, 5 at another counter
    // before, 46 compare all 16 AAC-ELD frames)
    assert!(stopped >= 6, "{stopped} reads of undefined lines");
    assert!(compared >= 400, "{compared} AAC-ELD frames compared");
}

/// A fresh AAC-ELD encoder with 240 lines at 16 kHz reads the lines 240..256 as well (the
/// spectrum buffer of the encoder is zeroed): compared, nothing counted.
#[test]
fn eld_240_fresh_encoder() {
    for (mode, ch) in [(1u32, 1usize), (2, 2), (7, 8)] {
        let what = format!("eld_240_fresh_encoder mode {mode}");
        let mut p = Pair::open(0, 8, &what).unwrap();
        p.stop_undefined = true;
        for &(a, v) in &[
            (AACENC_AOT, 39),
            (AACENC_SAMPLERATE, 16000),
            (AACENC_GRANULE_LENGTH, 240),
            (AACENC_CHANNELMODE, mode),
            (AACENC_BITRATE, 32000 * ch as u32),
            (AACENC_TRANSMUX, 10),
        ] {
            p.set(a, v);
        }
        let before = psy_main_undefined_spectrum_reads();
        for sig in [Signal::Music(0), Signal::Clicks, Signal::Noise] {
            let pcm = signal(sig, 16000, ch, 240 * 12);
            for f in pcm.chunks(240 * ch) {
                assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK, "{what}");
                assert!(!p.stopped_undefined, "{what}");
            }
        }
        assert_eq!(psy_main_undefined_spectrum_reads(), before, "{what}");
    }
}

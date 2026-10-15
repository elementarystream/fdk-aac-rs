// Differential tests of fdk_aac_rs_aacenc::aacenc_lib: the element mappings of the
// psychoacoustic model (PSY_ELEMENT::psyStatic[], PSY_OUT_ELEMENT::psyOutChannel[]) after an
// initialization that failed once FDKaacEnc_Initialize() had set encoderMode.
//
// The next initialization without AACENC_INIT_STATES sees an unchanged channel mode and skips
// FDKaacEnc_psyInit(), so the elements keep the mapping of the last FDKaacEnc_psyInit().
// Statics that are not consecutive are compared. Mappings without a defined reference result
// are refused by the port (AACENC_INIT_AAC_ERROR, `psy_main_stale_element_mappings`), and the
// comparison stops before them (`Pair::stop_undefined`).

use crate::aacenc_lib::{signal, EncCall, Pair};
use crate::adj_thr_enc::Signal;
use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_aacenc::adj_thr::adj_thr_stale_ah_flag_reads;
use fdk_aac_rs_aacenc::psy_main::{
    psy_main_lfe_flag_pairs, psy_main_mixed_block_pairs, psy_main_stale_element_mappings,
    UNASSIGNED,
};
use fdk_aac_rs_cref::aacenc_aacenc_lib_psy_map as m;
use fdk_aac_rs_fdk::audio::{MODE_1_2_2_2_1, MODE_7_1_BACK, MODE_7_1_TOP_FRONT};

/// The port alone (for sequences that the reference does not define).
struct Port(Option<Box<AacEncoder>>);

impl Port {
    fn open(max_ch: u32) -> Port {
        let mut h = None;
        assert_eq!(aac_enc_open(Some(&mut h), 0, max_ch), AACENC_OK);
        Port(h)
    }

    fn set(&mut self, p: AacencParam, v: u32) {
        assert_eq!(aac_encoder_set_param(self.0.as_deref_mut(), p, v), AACENC_OK, "SetParam({p:#x}, {v})");
    }

    fn init(&mut self) -> i32 {
        // SAFETY: no buffers.
        unsafe { aac_enc_encode(self.0.as_deref_mut(), None, None, None, None) }
    }

    fn encode(&mut self, pcm: &[i16]) -> i32 {
        let mut buf = pcm.to_vec();
        let mut out = vec![0u8; 20480];
        let mut in_ptr = [buf.as_mut_ptr() as *mut c_void];
        let mut in_id = [IN_AUDIO_DATA];
        let mut in_size = [buf.len() as i32 * 2];
        let mut in_el = [2i32];
        let in_desc = AacencBufDesc {
            num_bufs: 1,
            bufs: in_ptr.as_mut_ptr(),
            buffer_identifiers: in_id.as_mut_ptr(),
            buf_sizes: in_size.as_mut_ptr(),
            buf_el_sizes: in_el.as_mut_ptr(),
        };
        let mut out_ptr = [out.as_mut_ptr() as *mut c_void];
        let mut out_id = [OUT_BITSTREAM_DATA];
        let mut out_size = [out.len() as i32];
        let mut out_el = [1i32];
        let out_desc = AacencBufDesc {
            num_bufs: 1,
            bufs: out_ptr.as_mut_ptr(),
            buffer_identifiers: out_id.as_mut_ptr(),
            buf_sizes: out_size.as_mut_ptr(),
            buf_el_sizes: out_el.as_mut_ptr(),
        };
        let in_args = AacencInArgs { num_in_samples: buf.len() as i32, num_anc_bytes: 0 };
        let mut out_args = AacencOutArgs::default();
        // SAFETY: the descriptors point to live buffers of the announced sizes.
        unsafe {
            aac_enc_encode(self.0.as_deref_mut(), Some(&in_desc), Some(&out_desc), Some(&in_args), Some(&mut out_args))
        }
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        aac_enc_close(Some(&mut self.0));
    }
}

/// The parameter steps of `psy_elements_unassigned`.
const STEPS_UNDEFINED: [&[(AacencParam, u32)]; 3] = [
    &[(AACENC_AOT, 2), (AACENC_SAMPLERATE, 44100), (AACENC_CHANNELMODE, 1), (AACENC_BITRATE, 64000), (AACENC_TRANSMUX, 10)],
    &[(AACENC_AOT, 39), (AACENC_SAMPLERATE, 11025), (AACENC_CHANNELMODE, 7), (AACENC_BITRATE, 56000), (AACENC_ANCILLARY_BITRATE, 4000)],
    &[(AACENC_AOT, 2), (AACENC_SAMPLERATE, 44100), (AACENC_ANCILLARY_BITRATE, 0), (AACENC_BITRATE, 384000), (AACENC_CONTROL_STATE, 0x1001)],
];

/// The 7.1 channel pairs (elements 1 to 3) have no statics. The reference has no defined result
/// here; the port refuses the initialization and every following encode call, without a panic.
#[test]
fn psy_elements_unassigned() {
    // the port alone: the reference does not define the last initialization
    let mut p = Port::open(8);
    for &(a, v) in STEPS_UNDEFINED[0] {
        p.set(a, v);
    }
    let pcm = signal(Signal::Music(0), 44100, 1, 1024);
    assert_eq!(p.encode(&pcm), AACENC_OK);
    for &(a, v) in STEPS_UNDEFINED[1] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR, "the AAC-ELD configuration must fail");
    for &(a, v) in STEPS_UNDEFINED[2] {
        p.set(a, v);
    }
    let before = psy_main_stale_element_mappings();
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR, "an initialization without a defined reference result");
    assert_eq!(psy_main_stale_element_mappings(), before + 1);
    for sig in [Signal::Clicks, Signal::Music(0), Signal::Noise] {
        let pcm = signal(sig, 44100, 8, 1024 * 8);
        for f in pcm.chunks(1024 * 8) {
            assert_eq!(p.encode(f), AACENC_INIT_AAC_ERROR);
        }
    }

    // both: compared up to the last initialization, which the reference must not run
    let mut p = Pair::open(0, 8, "psy_elements_unassigned").unwrap();
    p.stop_undefined = true;
    for &(a, v) in STEPS_UNDEFINED[0] {
        p.set(a, v);
    }
    let pcm = signal(Signal::Music(0), 44100, 1, 1024);
    assert_eq!(p.encode(&EncCall::pcm(pcm, 20480)).0, AACENC_OK);
    for &(a, v) in STEPS_UNDEFINED[1] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR);
    assert!(!p.stopped_undefined);
    for &(a, v) in STEPS_UNDEFINED[2] {
        p.set(a, v);
    }
    assert_eq!(port_statics(&p, 1), [UNASSIGNED; 2], "the case under test");
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR);
    assert!(p.stopped_undefined, "the port must refuse the initialization");
}

/// The port's statics of element `el`, after checking that the mappings of all elements
/// (statics and output channels) are those of the reference.
fn port_statics(p: &Pair, el: usize) -> [usize; 2] {
    let enc = p.r.as_deref().unwrap().h_aac_enc.as_deref().unwrap();
    let psy = enc.psy_kernel.as_deref().unwrap();
    let out = enc.psy_out[0].as_deref().unwrap();
    let idx = |i: usize| if i == UNASSIGNED { -1 } else { i as i32 };
    for e in 0..psy.psy_element.len() {
        let (mut st, mut po) = ([0i32; 2], [0i32; 2]);
        // SAFETY: valid handle, two-element output arrays.
        let r = unsafe { m::cref_aacenc_lib_psy_map_element(p.c, e as i32, st.as_mut_ptr(), po.as_mut_ptr()) };
        assert_eq!(r, 0);
        assert_eq!(psy.psy_element[e].psy_static.map(idx), st, "{}: statics of element {e}", p.what);
        assert_eq!(out.psy_out_element[e].psy_out_channel.map(idx), po, "{}: output channels of element {e}", p.what);
    }
    psy.psy_element[el].psy_static
}

/// AAC-LC stereo with 8 channels allocated (element 0 = statics 1 and 2), then mono (element 0 =
/// static 0, `psyStatic[1]` stays 2), then a stereo AAC-ELD configuration at 8000 Hz whose
/// initialization fails in FDKaacEnc_DetermineBandWidth after setting encoderMode, then AAC-LC
/// stereo with AACENC_CONTROL_STATE without AACENC_INIT_STATES: C encodes the channel pair with
/// statics 0 and 2, and so must the port.
#[test]
fn psy_elements_not_consecutive() {
    // (Signal::Clicks is left out: the port stops at a counter of `deviation_events` there)
    for (sig, cs) in [
        (Signal::Music(0), 0x1001u32),
        (Signal::Music(1), 0x1001),
        (Signal::Sweep, 0x1001),
        (Signal::MultiTone, 0x1001),
        (Signal::Noise, 0x1001),
        (Signal::Square, 0x1001),
        (Signal::Music(1), 0x3001),
        (Signal::Sweep, 0x2001),
    ] {
        let what = format!("psy_elements_not_consecutive {sig:?} {cs:#x}");
        let mut p = Pair::open(0, 8, &what).unwrap();
        p.stop_undefined = true;
        for &(a, v) in &[
            (AACENC_AOT, 2),
            (AACENC_SAMPLERATE, 44100),
            (AACENC_CHANNELMODE, 2),
            (AACENC_BITRATE, 128000),
            (AACENC_TRANSMUX, 10),
        ] {
            p.set(a, v);
        }
        let pcm = signal(Signal::Music(0), 44100, 2, 1024 * 3);
        for f in pcm.chunks(2048) {
            assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK);
        }
        p.set(AACENC_CHANNELMODE, 1);
        let pcm = signal(sig, 44100, 1, 1024 * 3);
        for f in pcm.chunks(1024) {
            assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK);
        }
        for &(a, v) in &[
            (AACENC_AOT, 39),
            (AACENC_SAMPLERATE, 8000),
            (AACENC_CHANNELMODE, 2),
            (AACENC_BITRATE, 17000),
            (AACENC_ANCILLARY_BITRATE, 2000),
        ] {
            p.set(a, v);
        }
        assert_eq!(p.init(), AACENC_INIT_AAC_ERROR);
        for &(a, v) in &[
            (AACENC_AOT, 2),
            (AACENC_SAMPLERATE, 44100),
            (AACENC_ANCILLARY_BITRATE, 0),
            (AACENC_BITRATE, 128000),
            (AACENC_CONTROL_STATE, cs),
        ] {
            p.set(a, v);
        }
        assert_eq!(p.init(), AACENC_OK, "{what}");
        assert_eq!(port_statics(&p, 0), [0, 2], "{what}: the case under test");
        let pcm = signal(sig, 44100, 2, 1024 * 24);
        for f in pcm.chunks(2048) {
            assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK, "{what}");
            assert!(!p.stopped_undefined, "{what}");
        }
    }
}

/// AAC-LC 7.1 with the LFE before the last pair (MODE_7_1_TOP_FRONT: element 4 = statics 6 and
/// 7), then MODE_1_2_2_2_1 (element 4 = the LFE, static 7), then a MODE_7_1_TOP_FRONT AAC-ELD
/// configuration whose initialization fails after setting encoderMode, then AAC-LC
/// MODE_7_1_TOP_FRONT with AACENC_CONTROL_STATE = AACENC_INIT_CONFIG | AACENC_INIT_TRANSPORT:
/// element 4 is a channel pair with static 7 for both channels (and output channel 7 for both).
/// The port refuses the initialization (PORTING.md, "Deviations from the reference").
#[test]
fn psy_elements_shared_static() {
    let mut p = Pair::open(0, 8, "psy_elements_shared_static").unwrap();
    p.stop_undefined = true;
    for &(a, v) in &[
        (AACENC_AOT, 2),
        (AACENC_SAMPLERATE, 44100),
        (AACENC_CHANNELMODE, MODE_7_1_TOP_FRONT as u32),
        (AACENC_BITRATE, 384000),
        (AACENC_TRANSMUX, 10),
    ] {
        p.set(a, v);
    }
    let pcm = signal(Signal::Music(0), 44100, 8, 1024 * 2);
    for f in pcm.chunks(8192) {
        assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK);
    }
    assert_eq!(port_statics(&p, 4), [6, 7]);
    p.set(AACENC_CHANNELMODE, MODE_1_2_2_2_1 as u32);
    for f in pcm.chunks(8192) {
        assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK);
    }
    for &(a, v) in &[
        (AACENC_AOT, 39),
        (AACENC_SAMPLERATE, 11025),
        (AACENC_CHANNELMODE, MODE_7_1_TOP_FRONT as u32),
        (AACENC_BITRATE, 56000),
        (AACENC_ANCILLARY_BITRATE, 4000),
    ] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR);
    for &(a, v) in &[
        (AACENC_AOT, 2),
        (AACENC_SAMPLERATE, 44100),
        (AACENC_ANCILLARY_BITRATE, 0),
        (AACENC_BITRATE, 384000),
        (AACENC_CONTROL_STATE, 0x1001),
    ] {
        p.set(a, v);
    }
    assert_eq!(port_statics(&p, 4), [7, 7], "the case under test");
    let before = psy_main_stale_element_mappings();
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR);
    assert!(p.stopped_undefined, "the port must refuse static 7 for both channels of a pair");
    assert_eq!(psy_main_stale_element_mappings(), before + 1);
}

/// AAC-ELD `MODE_7_1_BACK` (element 3 = a channel pair: statics and output channels 5 and 6),
/// then `MODE_7_1_TOP_FRONT` (element 3 = the LFE: 5, `psyStatic[1]` and `psyOutChannel[1]` stay
/// 6; element 4 = a pair: 6 and 7), then a `MODE_1_2_2_2_1` AAC-ELD configuration whose
/// initialization fails after setting encoderMode, then `MODE_1_2_2_2_1` with AAC-LD or AAC-ELD
/// and AACENC_CONTROL_STATE without AACENC_INIT_STATES: `FDKaacEnc_psyInit()` is skipped, so
/// element 3 (a pair) uses statics and output channels 5 and 6, and element 4 (the LFE) 6 as
/// well. C shares static 6 and `PSY_OUT_CHANNEL` 6 between the two elements, and the array
/// pointers of `PSY_OUT_CHANNEL` 6 stay at the LFE's `QC_OUT_CHANNEL` (7): the threshold
/// adaptation (CBR and VBR) and the scalefactor estimation of element 3 read the LFE's energies
/// and thresholds through them. The port follows the pointers as well (`PsyOut::p_array_qc`,
/// `adj_thr::PsyArrayViews`); the test fails without them. Found by the encode fuzz target.
#[test]
fn psy_out_channel_shared_between_elements() {
    let mut aliased = 0;
    for (k, &(hist, aot, rate, mode, br, sig, quiet)) in SHARED_CASES.iter().enumerate() {
        let what = format!("psy_out_channel_shared_between_elements {k}");
        let (n, stopped) = shared_case(&what, hist, aot, rate, mode, br, sig, quiet, 12);
        assert!(!stopped, "{what}");
        assert!(n > 0, "{what}: no frame with the shared PSY_OUT_CHANNEL");
        aliased += n;
    }
    assert!(aliased >= 50, "{aliased} frames with the shared PSY_OUT_CHANNEL");
}

/// The cases of `psy_out_channel_shared_between_elements`: (history, AOT, sample rate, bitrate
/// mode, bitrate, signal, input channels 5 and 6 silent). Every value that the reference reads
/// in them is defined (see `psy_out_channel_shared_pns` for another case).
#[allow(clippy::type_complexity)]
const SHARED_CASES: [(Signal, u32, u32, u32, u32, Signal, bool); 9] = [
    (Signal::Silence, 23, 11025, 0, 192000, Signal::Music(1), false),
    (Signal::Silence, 23, 11025, 3, 192000, Signal::Noise, false),
    (Signal::Music(1), 23, 11025, 0, 192000, Signal::Music(1), false),
    (Signal::Noise, 23, 11025, 3, 192000, Signal::Music(1), false),
    (Signal::Noise, 39, 48000, 3, 192000, Signal::Noise, false),
    (Signal::MultiTone, 39, 48000, 3, 192000, Signal::Music(1), false),
    (Signal::MultiTone, 39, 11025, 3, 128000, Signal::MultiTone, false),
    (Signal::Square, 23, 11025, 0, 192000, Signal::Noise, false),
    (Signal::Clicks, 39, 48000, 0, 192000, Signal::MultiTone, true),
];

/// 8-channel PCM with input channels 5 and 6 silent.
fn silent_5_6(mut pcm: Vec<i16>) -> Vec<i16> {
    for f in pcm.chunks_mut(8) {
        f[5] = 0;
        f[6] = 0;
    }
    pcm
}

/// One case of `psy_out_channel_shared_between_elements`: the history `hist`, then `frames`
/// frames of `sig` with the given AOT, sample rate, bitrate mode and bitrate (input channels 5
/// and 6 silent with `quiet`). Returns the frames encoded with the shared `PSY_OUT_CHANNEL`
/// (up to the first error: C's bit count of the shared channel then often differs from what it
/// writes, AAC_ENC_WRITTEN_BITS_ERROR, and the next frame reinitializes everything), and
/// whether the comparison stopped at a frame that `psy_main_lfe_flag_pairs` counts.
#[allow(clippy::too_many_arguments)]
fn shared_case(what: &str, hist: Signal, aot: u32, rate: u32, mode: u32, br: u32, sig: Signal, quiet: bool, frames: usize) -> (usize, bool) {
    let mut aliased = 0;
    let q = |v: Vec<i16>| if quiet { silent_5_6(v) } else { v };
    let mut p = Pair::open(0, 8, what).unwrap();
    p.stop_undefined = true;
    for &(a, v) in &[
        (AACENC_AOT, 39),
        (AACENC_SAMPLERATE, 48000),
        (AACENC_CHANNELMODE, MODE_7_1_BACK as u32),
        (AACENC_BITRATE, 384000),
        (AACENC_TRANSMUX, 10),
    ] {
        p.set(a, v);
    }
    let pcm = q(signal(hist, 48000, 8, 512 * 3));
    for f in pcm.chunks(4096) {
        assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK, "{what}");
    }
    p.set(AACENC_CHANNELMODE, MODE_7_1_TOP_FRONT as u32);
    for f in pcm.chunks(4096) {
        assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK, "{what}");
    }
    for &(a, v) in &[
        (AACENC_AOT, 39),
        (AACENC_SAMPLERATE, 11025),
        (AACENC_CHANNELMODE, MODE_1_2_2_2_1 as u32),
        (AACENC_BITRATE, 56000),
        (AACENC_ANCILLARY_BITRATE, 4000),
    ] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR, "{what}");
    for &(a, v) in &[
        (AACENC_AOT, aot),
        (AACENC_SAMPLERATE, rate),
        (AACENC_BITRATEMODE, mode),
        (AACENC_METADATA_MODE, 1),
        (AACENC_BITRATE, br),
        (AACENC_CONTROL_STATE, 0x1001),
    ] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_OK, "{what}");
    assert_eq!(port_statics(&p, 3), [5, 6], "{what}: the case under test");
    assert_eq!(port_statics(&p, 4)[0], 6, "{what}: the case under test");
    let frame = p.info("frame length").frame_length as usize;
    let pcm = q(signal(sig, rate as i32, 8, frame * frames));
    for f in pcm.chunks(frame * 8) {
        let lfe_flag_pairs = psy_main_lfe_flag_pairs();
        let (e, _) = p.encode(&EncCall::pcm(f.to_vec(), 20480));
        if p.stopped_undefined {
            assert!(psy_main_lfe_flag_pairs() > lfe_flag_pairs, "{what}: another deviation");
            return (aliased, true);
        }
        if e != AACENC_OK {
            break;
        }
        let enc = p.r.as_deref().unwrap().h_aac_enc.as_deref().unwrap();
        let po = enc.psy_out[0].as_deref().unwrap();
        let qc = enc.qc_out[0].as_deref().unwrap();
        // PSY_OUT_CHANNEL 6: element 3, channel 1 (QC_OUT_CHANNEL 6), and the LFE (7)
        assert_eq!(qc.qc_element[3].qc_out_channel[0] + 1, 6, "{what}");
        assert_eq!(po.p_array_qc[6], 7, "{what}: the pointers of PSY_OUT_CHANNEL 6");
        aliased += 1;
    }
    (aliased, false)
}

/// An interleaved 8-channel click train: sample `n` of the interleaved stream (from `start`) is
/// 32767 if `n % period == phase`, else 0.
fn click_train(start: usize, len: usize, period: usize, phase: usize) -> Vec<i16> {
    (start..start + len).map(|n| if n % period == phase { 32767 } else { 0 }).collect()
}

/// The steps of `psy_out_channel_shared_ah_flags` up to the encode calls. Returns the pair.
fn ah_flags_pair(what: &str, br: u32) -> Pair {
    let mut p = Pair::open(0, 8, what).unwrap();
    p.stop_undefined = true;
    for &(a, v) in &[
        (AACENC_AOT, 2),
        (AACENC_CHANNELMODE, 12),
        (AACENC_SAMPLERATE, 64000),
        (AACENC_BITRATE, 128000),
    ] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_OK, "{what}");
    for &(a, v) in &[
        (AACENC_CONTROL_STATE, 0x2001),
        (AACENC_BITRATE, 112000),
        (AACENC_CHANNELMODE, 11),
        (AACENC_AFTERBURNER, 1),
        (AACENC_ANCILLARY_BITRATE, 8000),
    ] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_OK, "{what}");
    for &(a, v) in &[
        (AACENC_CONTROL_STATE, 3),
        (AACENC_CHANNELMODE, MODE_1_2_2_2_1 as u32),
        (AACENC_BITRATE, 8000),
        (AACENC_SAMPLERATE, 11025),
        (AACENC_TRANSMUX, 7),
        (AACENC_AOT, 39),
    ] {
        p.set(a, v);
    }
    assert_eq!(p.init(), AACENC_INIT_AAC_ERROR, "{what}: the AAC-ELD configuration must fail");
    for &(a, v) in &[(AACENC_CONTROL_STATE, 0x1001), (AACENC_BITRATE, br), (AACENC_SBR_RATIO, 0)] {
        p.set(a, v);
    }
    p
}

/// After a skipped `FDKaacEnc_psyInit()`, element 3 (a pair) of `MODE_1_2_2_2_1` shares
/// static 6 and `PSY_OUT_CHANNEL` 6 with the LFE, as in `psy_out_channel_shared_between_elements`
/// (here after `MODE_7_1_REAR_SURROUND` and `MODE_6_1`). The port counts the frames without a
/// defined reference result (`adj_thr_stale_ah_flag_reads`; PORTING.md, "Deviations from the
/// reference"), and the comparison stops before them; the frames before are compared.
#[test]
fn psy_out_channel_shared_ah_flags() {
    let (mut c_nonzero, mut stops) = (0, 0);
    let cases = [
        (64000u32, 997usize, 853usize),
        (64000, 997, 0),
        (64000, 509, 7),
        (64000, 1499, 3),
        (64000, 2003, 11),
        (96000, 997, 853),
        (96000, 509, 7),
        (96000, 1499, 3),
    ];
    for (k, &(br, period, phase)) in cases.iter().enumerate() {
        let what = format!("psy_out_channel_shared_ah_flags {k}");
        let mut p = ah_flags_pair(&what, br);
        let frame = p.info("frame length").frame_length as usize;
        assert_eq!(port_statics(&p, 3), [5, 6], "{what}: the case under test");
        assert_eq!(port_statics(&p, 4)[0], 6, "{what}: the case under test");
        let mut pos = 0;
        let mut stopped = false;
        for _ in 0..16 {
            // the flags of element 3, channel 1 before the reference's next call
            let mut c_flags = [0u8; 60];
            // SAFETY: valid handle, 60-byte output.
            let r = unsafe { m::cref_aacenc_lib_psy_map_ah_flags(p.c, 3, 1, c_flags.as_mut_ptr()) };
            assert_eq!(r, 0);
            let (ah, lfe) = (adj_thr_stale_ah_flag_reads(), psy_main_lfe_flag_pairs());
            let (e, _) = p.encode(&EncCall::pcm(click_train(pos, frame * 8, period, phase), 20480));
            pos += frame * 8;
            if p.stopped_undefined {
                assert!(adj_thr_stale_ah_flag_reads() > ah, "{what}: another deviation");
                assert_eq!(psy_main_lfe_flag_pairs(), lfe, "{what}: another deviation");
                // the entries without a defined reference value
                let enc = p.r.as_deref().unwrap().h_aac_enc.as_deref().unwrap();
                let po = enc.psy_out[0].as_deref().unwrap();
                let (c0, c1) = (&po.p_psy_out_channels[5], &po.p_psy_out_channels[6]);
                let ms = &po.psy_out_element[3].tools_info.ms_mask;
                assert!(c1.max_sfb_per_group < c0.max_sfb_per_group, "{what}");
                if (c1.max_sfb_per_group..c0.max_sfb_per_group)
                    .any(|i| ms[i as usize] != 0 && c_flags[i as usize] != 0)
                {
                    c_nonzero += 1;
                }
                stopped = true;
                break;
            }
            assert_eq!(e, AACENC_OK, "{what}");
        }
        // (whether and when a case stops depends on the variant: a case without a stop
        // compares all its frames)
        stops += stopped as usize;
    }
    assert!(2 * stops >= cases.len(), "{stops} of {} cases stopped", cases.len());
    assert!(c_nonzero > 0, "no case where the reference's flags differ from the port's");
}

/// As `psy_out_channel_shared_between_elements`, with PNS on noise (AAC-LD 48 kHz, 256 kbit/s).
/// The port counts the first frame without a defined reference result
/// (`psy_main_lfe_flag_pairs`), and the comparison stops there; the frames before it are
/// compared.
#[test]
fn psy_out_channel_shared_pns() {
    let what = "psy_out_channel_shared_pns";
    let (n, stopped) =
        shared_case(what, Signal::Silence, 23, 48000, 0, 256000, Signal::Noise, false, 6);
    assert!(stopped, "{what}: no stop");
    assert!(n >= 1, "{what}: {n} frames compared");
}

/// As `psy_elements_not_consecutive`, but the mono configuration is AAC-LD (512 lines), so
/// static 0 is last reset without `allowShortFrames` and static 2 with it. The port refuses the
/// frames without a defined reference result (`psy_main_mixed_block_pairs`), without a panic.
#[test]
fn psy_pair_mixed_blocks() {
    let mut hits = 0;
    for sig in [Signal::Clicks, Signal::Square, Signal::Music(0), Signal::Noise] {
        let what = format!("psy_pair_mixed_blocks {sig:?}");
        let mut p = Pair::open(0, 8, &what).unwrap();
        p.stop_undefined = true;
        for &(a, v) in &[
            (AACENC_AOT, 2),
            (AACENC_SAMPLERATE, 44100),
            (AACENC_CHANNELMODE, 2),
            (AACENC_BITRATE, 128000),
            (AACENC_TRANSMUX, 10),
        ] {
            p.set(a, v);
        }
        let pcm = signal(Signal::Music(0), 44100, 2, 1024 * 3);
        for f in pcm.chunks(2048) {
            assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK);
        }
        for &(a, v) in &[(AACENC_AOT, 23), (AACENC_GRANULE_LENGTH, 512), (AACENC_CHANNELMODE, 1), (AACENC_BITRATE, 64000)] {
            p.set(a, v);
        }
        let pcm = signal(Signal::Music(0), 44100, 1, 512 * 4);
        for f in pcm.chunks(512) {
            assert_eq!(p.encode(&EncCall::pcm(f.to_vec(), 20480)).0, AACENC_OK);
        }
        for &(a, v) in &[
            (AACENC_AOT, 39),
            (AACENC_SAMPLERATE, 8000),
            (AACENC_CHANNELMODE, 2),
            (AACENC_BITRATE, 17000),
            (AACENC_ANCILLARY_BITRATE, 2000),
        ] {
            p.set(a, v);
        }
        assert_eq!(p.init(), AACENC_INIT_AAC_ERROR, "{what}");
        for &(a, v) in &[
            (AACENC_AOT, 2),
            (AACENC_GRANULE_LENGTH, 1024),
            (AACENC_SAMPLERATE, 44100),
            (AACENC_ANCILLARY_BITRATE, 0),
            (AACENC_BITRATE, 128000),
            (AACENC_CONTROL_STATE, 0x1001),
        ] {
            p.set(a, v);
        }
        assert_eq!(p.init(), AACENC_OK, "{what}");
        assert_eq!(port_statics(&p, 0), [0, 2], "{what}: the case under test");
        let before = psy_main_mixed_block_pairs();
        let pcm = signal(sig, 44100, 2, 1024 * 24);
        for f in pcm.chunks(2048) {
            p.encode(&EncCall::pcm(f.to_vec(), 20480));
            if p.stopped_undefined {
                break;
            }
        }
        if psy_main_mixed_block_pairs() > before {
            assert!(p.stopped_undefined, "{what}");
            hits += 1;
        }
    }
    assert!(hits > 0, "no refused frame");
}

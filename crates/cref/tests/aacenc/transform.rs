// Differential tests of fdk_aac_rs_aacenc::transform against the C++ reference.
//
// The transforms run over many consecutive frames as in FDKaacEnc_psyMain(): the block switching
// (ported, compared with C in block_switch.rs) chooses the window sequence and shape, the time
// signal goes through PSY_STATIC::psyInputBuffer with the delay of the filter bank, and the MDCT
// state (or the ELD overlap buffer) persists between frames. After every frame the return value,
// the spectrum, the exponent, the window shape and the complete MDCT state are compared. Further
// tests drive the transform with every window transition the block switching can produce, with
// both window shapes, and with invalid parameters.
//
// Before every AAC-LC/LD transform the tests assert that the left window slope (the right slope
// of the previous transform) is not longer than the transform.

use crate::block_switch::{corpus_signals, synthetic_signals};
use fdk_aac_rs_aacenc::block_switch::*;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_aacenc::psy_data::OVERLAP_ADD_BUFFER_SIZE;
use fdk_aac_rs_aacenc::transform::*;
use fdk_aac_rs_cref::aacenc_transform as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::mdct::{mdct_init, MdctT};
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;
use std::ffi::c_void;

/// Window slopes (length, shape) that identify `prev_wrs`.
const SLOPES: &[(i32, i32)] = &[
    (1024, 0),
    (1024, 1),
    (960, 0),
    (960, 1),
    (512, 0),
    (512, 1),
    (480, 0),
    (480, 1),
    (128, 0),
    (128, 1),
    (120, 0),
    (120, 1),
    (64, 0),
    (60, 0),
    (256, 0),
    (240, 0),
];

/// The port's and C's MDCT state of one channel.
pub struct Mdct {
    pub r: MdctT,
    c: *mut c_void,
}

impl Drop for Mdct {
    fn drop(&mut self) {
        unsafe { c::cref_transform_mdct_free(self.c) };
    }
}

impl Mdct {
    /// `mdct_init(&mdctPers, NULL, 0)` on a zeroed state, as `FDKaacEnc_psyInit()`.
    pub fn new() -> Self {
        let mut r = MdctT::default();
        mdct_init(&mut r, 0);
        Mdct { r, c: unsafe { c::cref_transform_mdct_new() } }
    }

    fn check(&self, what: &str) {
        let lens: Vec<i32> = SLOPES.iter().map(|s| s.0).collect();
        let shapes: Vec<i32> = SLOPES.iter().map(|s| s.1).collect();
        let mut f = [0i32; 6];
        let id = unsafe {
            c::cref_transform_mdct_state(
                self.c,
                f.as_mut_ptr(),
                lens.as_ptr(),
                shapes.as_ptr(),
                SLOPES.len() as i32,
            )
        };
        let r = &self.r;
        let rid = match r.prev_wrs {
            None => -1,
            Some(w) => SLOPES
                .iter()
                .position(|&(l, s)| {
                    fdk_get_window_slope(l, s).is_some_and(|x| x.as_ptr() == w.as_ptr())
                })
                .map_or(-2, |i| i as i32),
        };
        let rf = [
            r.prev_tl,
            r.prev_nr,
            r.prev_fr,
            r.ov_offset,
            r.ov_size,
            !r.overlap.is_empty() as i32,
        ];
        assert_eq!((rf, rid), (f, id), "{what}: MDCT state (prev_tl, prev_nr, prev_fr, ...)");
    }
}

/// `FDKaacEnc_Transform_Real` on both; compares everything. `garbage` fills the spectrum
/// buffers before the call (to see writes outside the frame).
#[allow(clippy::too_many_arguments)]
fn transform_real(
    m: &mut Mdct,
    time: &[i16],
    block_type: i32,
    window_shape: i32,
    prev_shape: &mut i32,
    frame_length: i32,
    garbage: i32,
    what: &str,
) -> (Vec<i32>, i32) {
    let mut spec_r = vec![garbage; 1024 + 16];
    let mut spec_c = spec_r.clone();
    let (mut e_r, mut e_c) = (-99, -99);
    let (mut ps_r, mut ps_c) = (*prev_shape, *prev_shape);
    let ret_r = fdk_aac_enc_transform_real(
        time,
        &mut spec_r,
        block_type,
        window_shape,
        &mut ps_r,
        &mut m.r,
        frame_length,
        &mut e_r,
        0,
    );
    let ret_c = unsafe {
        c::cref_transform_Transform_Real(
            time.as_ptr(),
            spec_c.as_mut_ptr(),
            block_type,
            window_shape,
            &mut ps_c,
            m.c,
            frame_length,
            &mut e_c,
            0,
        )
    };
    assert_eq!((ret_r, e_r, ps_r), (ret_c, e_c, ps_c), "{what}: return, exponent, shape");
    assert!(spec_r == spec_c, "{what}: spectrum");
    m.check(what);
    *prev_shape = ps_r;
    spec_r.truncate(frame_length as usize);
    (spec_r, e_r)
}

/// `FDKaacEnc_Transform_Real_Eld` on both; compares everything.
#[allow(clippy::too_many_arguments)]
fn transform_real_eld(
    time: &[i16],
    block_type: i32,
    window_shape: i32,
    prev_shape: &mut i32,
    frame_length: i32,
    oab: &mut [i32; OVERLAP_ADD_BUFFER_SIZE],
    garbage: i32,
    what: &str,
) -> (Vec<i32>, i32) {
    let mut spec_r = vec![garbage; 1024 + 16];
    let mut spec_c = spec_r.clone();
    let (mut e_r, mut e_c) = (-99, -99);
    let (mut ps_r, mut ps_c) = (*prev_shape, *prev_shape);
    let mut oab_c = *oab;
    let ret_r = fdk_aac_enc_transform_real_eld(
        time,
        &mut spec_r,
        block_type,
        window_shape,
        &mut ps_r,
        frame_length,
        &mut e_r,
        2,
        oab,
    );
    let ret_c = unsafe {
        c::cref_transform_Transform_Real_Eld(
            time.as_ptr(),
            spec_c.as_mut_ptr(),
            block_type,
            window_shape,
            &mut ps_c,
            frame_length,
            &mut e_c,
            2,
            oab_c.as_mut_ptr(),
        )
    };
    assert_eq!((ret_r, e_r, ps_r), (ret_c, e_c, ps_c), "{what}: return, exponent, shape");
    assert!(spec_r == spec_c, "{what}: spectrum");
    assert!(*oab == oab_c, "{what}: overlap add buffer");
    *prev_shape = ps_r;
    spec_r.truncate(frame_length as usize);
    (spec_r, e_r)
}

/// The filter bank stages of `FDKaacEnc_psyMain()` for one channel element.
pub struct Analysis {
    pub filterbank: i32,
    pub frame_length: usize,
    pub is_lfe: i32,
    pub bsc: [BlockSwitchingControl; 2],
    pub mdct: [Mdct; 2],
    pub oab: [[i32; OVERLAP_ADD_BUFFER_SIZE]; 2],
    /// `psyInputBuffer` (2 * frame length used)
    pub input: [Vec<i16>; 2],
    pub n_channels: usize,
    /// (prev_fr, tl) of every LC/LD transform
    pub mdct_transitions: std::collections::BTreeSet<(i32, i32, i32, i32)>,
}

impl Analysis {
    pub fn new(filterbank: i32, frame_length: usize, n_channels: usize, is_lfe: i32) -> Self {
        let mut bsc = [BlockSwitchingControl::default(); 2];
        for b in bsc.iter_mut() {
            // isLowDelay(audioObjectType): LD and ELD
            fdk_aac_enc_init_block_switching(b, (filterbank != FB_LC) as i32);
        }
        Analysis {
            filterbank,
            frame_length,
            is_lfe,
            bsc,
            mdct: [Mdct::new(), Mdct::new()],
            oab: [[0; OVERLAP_ADD_BUFFER_SIZE]; 2],
            input: [vec![0; 2 * frame_length], vec![0; 2 * frame_length]],
            n_channels,
            mdct_transitions: Default::default(),
        }
    }

    /// One frame: `pcm[ch]` holds `frame_length` new samples per channel. Returns the spectra
    /// and exponents (compared with C) of the channels.
    pub fn frame(&mut self, pcm: &[&[i16]], what: &str) -> Vec<(Vec<i32>, i32)> {
        let n = self.frame_length;
        let block_switching_offset = match self.filterbank {
            FB_LC => n + (9 * n / (2 * TRANS_FAC as usize)),
            _ => n,
        };
        let nch = self.n_channels;
        if self.filterbank != FB_ELD {
            for ch in 0..nch {
                fdk_aac_enc_block_switching(&mut self.bsc[ch], n as i32, self.is_lfe, pcm[ch]);
                self.input[ch][block_switching_offset..2 * n]
                    .copy_from_slice(&pcm[ch][..2 * n - block_switching_offset]);
            }
            let (b0, b1) = self.bsc.split_at_mut(1);
            let err = fdk_aac_enc_sync_block_switching(
                &mut b0[0],
                (nch > 1).then_some(&mut b1[0]),
                nch as i32,
                1,
            );
            assert_eq!(err, 0);
        } else {
            for ch in 0..nch {
                self.input[ch][block_switching_offset..block_switching_offset + n]
                    .copy_from_slice(pcm[ch]);
            }
        }
        let mut out = Vec::new();
        for ch in 0..nch {
            let bsc = &mut self.bsc[ch];
            let what = format!("{what} channel {ch}");
            let res = if self.filterbank == FB_ELD {
                transform_real_eld(
                    &self.input[ch],
                    bsc.last_window_sequence,
                    bsc.window_shape,
                    &mut bsc.last_window_shape,
                    n as i32,
                    &mut self.oab[ch],
                    0x5555_5555,
                    &what,
                )
            } else {
                // The left window slope must fit into the transform.
                let tl = if bsc.last_window_sequence == SHORT_WINDOW { n / 8 } else { n } as i32;
                let prev_fr = self.mdct[ch].r.prev_fr;
                assert!(prev_fr <= tl, "{what}: transform {tl} after a slope of {prev_fr}");
                self.mdct_transitions.insert((
                    prev_fr,
                    tl,
                    bsc.last_window_sequence,
                    bsc.window_shape,
                ));
                let r = transform_real(
                    &mut self.mdct[ch],
                    &self.input[ch],
                    bsc.last_window_sequence,
                    bsc.window_shape,
                    &mut bsc.last_window_shape,
                    n as i32,
                    0x5555_5555,
                    &what,
                );
                assert!(r.1 >= 0);
                r
            };
            out.push(res);
            // rotate internal time samples ... and get remaining samples from input buffer
            self.input[ch].copy_within(n..2 * n, 0);
            self.input[ch][n..block_switching_offset]
                .copy_from_slice(&pcm[ch][2 * n - block_switching_offset..n]);
        }
        out
    }
}

impl Default for Mdct {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs the analysis over all frames of `signal`.
fn run(signal: &[Vec<i16>], filterbank: i32, n: usize, nch: usize, is_lfe: i32, what: &str) -> Analysis {
    let mut a = Analysis::new(filterbank, n, nch, is_lfe);
    let frames = signal[0].len() / n;
    for f in 0..frames {
        let pcm: Vec<&[i16]> = (0..nch).map(|ch| &signal[ch][f * n..(f + 1) * n]).collect();
        a.frame(&pcm, &format!("{what} frame {f}"));
    }
    a
}

#[test]
fn analysis_sequences() {
    let n_samples = 1024 * 100;
    let mut signals = corpus_signals(n_samples);
    signals.extend(synthetic_signals(n_samples, 11));
    let mut lc = std::collections::BTreeSet::new();
    let mut ld = std::collections::BTreeSet::new();
    for (name, s) in &signals {
        for (fb, n) in [
            (FB_LC, 1024),
            (FB_LC, 960),
            (FB_LD, 512),
            (FB_LD, 480),
            (FB_ELD, 512),
            (FB_ELD, 480),
            (FB_ELD, 256),
            (FB_ELD, 240),
            (FB_ELD, 128),
            (FB_ELD, 120),
        ] {
            for (nch, is_lfe) in [(2, 0), (1, 0), (1, 1)] {
                if fb == FB_ELD && is_lfe != 0 {
                    continue;
                }
                let what = format!("{name} filterbank {fb} frame length {n} channels {nch} lfe {is_lfe}");
                let a = run(s, fb, n, nch, is_lfe, &what);
                if fb == FB_LC {
                    lc.extend(a.mdct_transitions.iter().map(|t| (t.0 * 1024 / n as i32, t.1 * 1024 / n as i32, t.2, t.3)));
                } else {
                    ld.extend(a.mdct_transitions.iter().map(|t| (t.0 * 512 / n as i32, t.1 * 512 / n as i32, t.2, t.3)));
                }
            }
        }
    }
    eprintln!("LC (prev_fr, tl, block type, shape): {lc:?}");
    eprintln!("LD (prev_fr, tl, block type, shape): {ld:?}");
    // Every block type of AAC-LC, and the low-overlap window of AAC-LD.
    for bt in [LONG_WINDOW, START_WINDOW, SHORT_WINDOW, STOP_WINDOW] {
        assert!(lc.iter().any(|t| t.2 == bt), "LC block type {bt} not covered");
    }
    assert!(lc.contains(&(128, 128, SHORT_WINDOW, SINE_WINDOW)));
    assert!(ld.contains(&(512, 512, LONG_WINDOW, LOL_WINDOW)));
    assert!(ld.contains(&(128, 512, LONG_WINDOW, SINE_WINDOW)));
}

/// The window sequences of the AAC-LC block switching with random shapes (sine or KBD for every
/// block type), and AAC-LD's random sequences of long windows with the sine and the low overlap
/// shape (the only ones the LD block switching produces; there are no KBD slopes of 512 and 480
/// lines), with random PCM including extreme values.
#[test]
fn transform_real_random_sequences() {
    let mut r = Rng::new(12);
    for &(n, lowdelay) in &[(1024, false), (960, false), (512, true), (480, true)] {
        for run in 0..6 {
            let mut m = Mdct::new();
            let mut prev_shape = r.range(0, 2);
            let mut bt = LONG_WINDOW;
            let mut time = vec![0i16; 2 * n];
            for f in 0..300 {
                // next block type: LONG -> {LONG, START}, START -> SHORT, SHORT -> {SHORT, STOP},
                // STOP -> {LONG, START}; only long windows in LD
                let next = r.next_u32() % 2 == 0;
                bt = match bt {
                    _ if lowdelay => LONG_WINDOW,
                    LONG_WINDOW | STOP_WINDOW => {
                        if next {
                            START_WINDOW
                        } else {
                            LONG_WINDOW
                        }
                    }
                    START_WINDOW => SHORT_WINDOW,
                    _ => {
                        if next {
                            STOP_WINDOW
                        } else {
                            SHORT_WINDOW
                        }
                    }
                };
                let shape = if lowdelay {
                    [SINE_WINDOW, LOL_WINDOW][(r.next_u32() % 2) as usize]
                } else {
                    r.range(0, 1)
                };
                let shift = r.next_u32() % 16;
                match r.next_u32() % 4 {
                    0 => time.iter_mut().for_each(|x| *x = r.i16()),
                    1 => time.iter_mut().for_each(|x| {
                        *x = if r.next_u32() % 2 == 0 { i16::MAX } else { i16::MIN }
                    }),
                    2 => time.iter_mut().for_each(|x| *x = 0),
                    _ => time.iter_mut().for_each(|x| *x = (r.next_u32() as i16) >> shift),
                }
                let tl = if bt == SHORT_WINDOW { n / 8 } else { n } as i32;
                assert!(m.r.prev_fr <= tl);
                let what = format!("n {n} run {run} frame {f} block type {bt} shape {shape}");
                transform_real(&mut m, &time, bt, shape, &mut prev_shape, n as i32, r.i32(), &what);
            }
        }
    }
}

#[test]
fn transform_real_invalid_block_type() {
    let mut r = Rng::new(13);
    let time: Vec<i16> = (0..2048).map(|_| r.i16()).collect();
    for bt in [_LOWOV_WINDOW, WRONG_WINDOW, -1, 6, i32::MIN] {
        let mut m = Mdct::new();
        let mut prev_shape = 7;
        transform_real(&mut m, &time, bt, SINE_WINDOW, &mut prev_shape, 1024, 3, "invalid");
        assert_eq!(prev_shape, 7);
    }
}

#[test]
fn transform_real_eld_random() {
    let mut r = Rng::new(14);
    for &n in &[512, 480, 256, 240, 128, 120] {
        for run in 0..4 {
            let mut oab = [0i32; OVERLAP_ADD_BUFFER_SIZE];
            if run == 3 {
                // an arbitrary overlap state
                oab.iter_mut().for_each(|x| *x = r.i32());
            }
            let mut prev_shape = 0;
            let mut time = vec![0i16; 2 * n];
            for f in 0..200 {
                let shift = r.next_u32() % 16;
                match r.next_u32() % 4 {
                    0 => time.iter_mut().for_each(|x| *x = r.i16()),
                    1 => time.iter_mut().for_each(|x| *x = if r.next_u32() % 2 == 0 { i16::MAX } else { i16::MIN }),
                    2 => time.iter_mut().for_each(|x| *x = 0),
                    _ => time.iter_mut().for_each(|x| *x = (r.next_u32() as i16) >> shift),
                }
                let shape = r.range(0, 2);
                let what = format!("n {n} run {run} frame {f}");
                transform_real_eld(&time, LONG_WINDOW, shape, &mut prev_shape, n as i32, &mut oab, r.i32(), &what);
            }
        }
    }
}

#[test]
fn transform_real_eld_invalid() {
    let mut r = Rng::new(15);
    let time: Vec<i16> = (0..2048).map(|_| r.i16()).collect();
    let mut oab = [0i32; OVERLAP_ADD_BUFFER_SIZE];
    oab.iter_mut().for_each(|x| *x = r.i32());
    for (bt, n) in [(START_WINDOW, 512), (SHORT_WINDOW, 480), (STOP_WINDOW, 256), (LONG_WINDOW, 1024), (LONG_WINDOW, 960), (LONG_WINDOW, 64), (LONG_WINDOW, 0)] {
        let mut prev_shape = 7;
        transform_real_eld(&time, bt, SINE_WINDOW, &mut prev_shape, n, &mut oab, 3, "invalid");
        assert_eq!(prev_shape, 7);
    }
}

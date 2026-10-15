// The optimized imlt_block against its straight port (imlt_block_ref).
//
// imlt_block runs its element loops on slices where every access is inside the buffers (NEON on
// aarch64) and falls back to the element-wise code of the straight port otherwise. The tests run
// both on the same decoder sequences and on random, also inconsistent, parameters (short output
// and overlap buffers, mismatched window slopes, alias symmetry flags, FAC ZIR, asymmetric
// overlap) and compare the return value, the output, the spectrum and the complete state.

use crate::fft::signal;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::archdef::FixpWtp;
use fdk_aac_rs_fdk::mdct::*;
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;

fn slope(len: i32, shape: i32) -> Option<&'static [FixpWtp]> {
    if len > 0 {
        fdk_get_window_slope(len, shape)
    } else {
        None
    }
}

fn same_state(a: &MdctT, b: &MdctT, what: &str) {
    assert_eq!(a.overlap, b.overlap, "{what}: overlap");
    let f = |m: &MdctT| {
        (
            m.prev_tl,
            m.prev_nr,
            m.prev_fr,
            m.ov_offset,
            m.ov_size,
            m.prev_alias_symmetry,
            m.prev_prev_alias_symmetry,
            m.p_fac_zir.clone(),
            m.p_asym_ovlp.clone(),
            m.prev_wrs.map(|w| (w.as_ptr(), w.len())),
        )
    };
    assert_eq!(f(a), f(b), "{what}: state");
}

#[allow(clippy::too_many_arguments)]
fn run_both(
    fast: &mut MdctT,
    straight: &mut MdctT,
    r: &mut Rng,
    out_len: usize,
    spectrum: &[i32],
    scalefactor: &[i16],
    n_spec: i32,
    no_out_samples: i32,
    tl: i32,
    wls: Option<&'static [FixpWtp]>,
    fl: i32,
    wrs: Option<&'static [FixpWtp]>,
    fr: i32,
    gain: i32,
    flags: i32,
    what: &str,
) {
    let garbage: Vec<i32> = (0..out_len).map(|_| r.i32()).collect();
    let (mut out_a, mut out_b) = (garbage.clone(), garbage);
    let (mut spec_a, mut spec_b) = (spectrum.to_vec(), spectrum.to_vec());
    let na = imlt_block(
        fast, &mut out_a, &mut spec_a, scalefactor, n_spec, no_out_samples, tl, wls, fl, wrs, fr,
        gain, flags,
    );
    let nb = imlt_block_ref(
        straight, &mut out_b, &mut spec_b, scalefactor, n_spec, no_out_samples, tl, wls, fl, wrs,
        fr, gain, flags,
    );
    assert_eq!(na, nb, "{what}: return value");
    assert_eq!(spec_a, spec_b, "{what}: spectrum");
    assert_eq!(out_a, out_b, "{what}: output");
    same_state(fast, straight, what);
}

fn scalefactors(r: &mut Rng, n: usize) -> Vec<i16> {
    (0..n)
        .map(|_| match r.next_u32() % 8 {
            0 => r.range(-40, 40) as i16,
            1 => r.i16(),
            _ => r.range(-4, 12) as i16,
        })
        .collect()
}

fn spectrum(r: &mut Rng, n: usize) -> Vec<i32> {
    let k = if r.next_u32() % 8 == 0 { r.range(0, 8) as usize } else { 9 };
    signal(r, k, n)
}

fn gain(r: &mut Rng) -> i32 {
    match r.next_u32() % 4 {
        0 => r.i32(),
        1 => r.range(0x2000_0000, 0x7fff_ffff),
        _ => 0,
    }
}

/// Decoder-like sequences (long, start, short, stop blocks; frame lengths of AAC, USAC and
/// AAC-LD/ELD; diverted output and drains).
#[test]
fn imlt_block_fast_equals_straight_port_sequences() {
    let configs: &[(i32, &[i32], bool, bool)] = &[
        (1024, &[0, 1], false, false),
        (960, &[0, 1], false, false),
        (1024, &[0, 1], false, true),
        (768, &[0, 1], false, true),
        (512, &[0, 2], true, false),
        (480, &[0, 2], true, false),
    ];
    for (ci, &(frame_len, shapes, long_only, usac)) in configs.iter().enumerate() {
        let mut r = Rng::new(0x1a18_0000 + ci as u64);
        for run in 0..16 {
            let fill = if run % 2 == 0 { 0 } else { r.i32() };
            let mut a = MdctT { overlap: vec![fill; 768], ..Default::default() };
            mdct_init(&mut a, 768);
            let mut b = a.clone();
            let mut seq = BLOCK_LONG;
            for frame in 0..60 {
                let what = format!("config {ci} run {run} frame {frame}");
                seq = if long_only {
                    BLOCK_LONG
                } else if r.next_u32() % 4 == 0 {
                    r.range(0, 3)
                } else {
                    match seq {
                        BLOCK_LONG | BLOCK_STOP => [BLOCK_START, BLOCK_LONG, BLOCK_LONG][r.range(0, 2) as usize],
                        _ => [BLOCK_SHORT, BLOCK_STOP][r.range(0, 1) as usize],
                    }
                };
                let shape = shapes[r.range(0, shapes.len() as i32 - 1) as usize];
                let mut tl = frame_len;
                let mut n_spec = 1;
                let (fl, fr) = match seq {
                    BLOCK_STOP => (frame_len >> 3, frame_len),
                    BLOCK_START => (frame_len, frame_len >> 3),
                    BLOCK_SHORT => {
                        tl >>= 3;
                        n_spec = 8;
                        (frame_len >> 3, frame_len >> 3)
                    }
                    _ => {
                        let fr = frame_len - if shape == 2 { (frame_len * 3) >> 2 } else { 0 };
                        let fl = if a.prev_tl == 0 { fr } else { frame_len };
                        (fl, fr)
                    }
                };
                let flags = if usac { (r.next_u32() % 2) as i32 } else { 0 };
                // Fewer output samples than the frame now and then (the rest is diverted to the
                // overlap buffer and drained afterwards).
                let no_out = if seq == BLOCK_LONG && frame_len == 1024 && r.next_u32() % 8 == 0 {
                    frame_len / 2
                } else {
                    frame_len
                };
                let spec = spectrum(&mut r, (n_spec * tl) as usize);
                let sf = scalefactors(&mut r, n_spec as usize);
                let g = gain(&mut r);
                run_both(
                    &mut a,
                    &mut b,
                    &mut r,
                    frame_len as usize,
                    &spec,
                    &sf,
                    n_spec,
                    no_out,
                    tl,
                    slope(fl, shape),
                    fl,
                    slope(fr, shape),
                    fr,
                    g,
                    flags,
                    &what,
                );
                if no_out < frame_len || r.next_u32() % 8 == 0 {
                    let mut oa = vec![0; frame_len as usize];
                    let mut ob = oa.clone();
                    let na = imdct_drain(&mut a, &mut oa, frame_len);
                    let nb = imdct_drain(&mut b, &mut ob, frame_len);
                    assert_eq!((na, &oa), (nb, &ob), "{what}: drain");
                }
            }
        }
    }
}

/// Random states and parameters, also inconsistent ones that make the loops access outside of
/// the buffers (the straight port skips such accesses; the optimized code must fall back to it).
#[test]
fn imlt_block_fast_equals_straight_port_random() {
    const LENS: &[i32] = &[1024, 960, 768, 512, 480, 256, 240, 128, 120, 96, 64, 48, 32];
    let mut r = Rng::new(0x1a18_1000);
    for case in 0..20_000 {
        let what = format!("case {case}");
        let tl = LENS[r.range(0, LENS.len() as i32 - 1) as usize];
        let n_spec = if r.next_u32() % 3 == 0 { r.range(1, 8) } else { 1 };
        let shape = r.range(0, 2);
        let pick = |r: &mut Rng| match r.next_u32() % 4 {
            0 => LENS[r.range(0, LENS.len() as i32 - 1) as usize],
            1 => tl >> 3,
            _ => tl,
        };
        let (fl, fr) = (pick(&mut r), pick(&mut r));
        let ov_size = [768, 1024, 512, 2048, 64, tl / 2][r.range(0, 5) as usize];
        let ovl: Vec<i32> = signal(&mut r, 9, ov_size as usize);
        let mut a = MdctT { overlap: ovl, ..Default::default() };
        mdct_init(&mut a, ov_size);
        let wl = pick(&mut r);
        a.prev_tl = if r.next_u32() % 4 == 0 { 0 } else { pick(&mut r) };
        a.prev_fr = if r.next_u32() % 2 == 0 { fl } else { wl };
        a.prev_nr = match r.next_u32() % 4 {
            0 => r.range(-64, 1024),
            1 => fl / 2,
            _ => (tl - a.prev_fr) >> 1,
        };
        a.prev_wrs = slope(a.prev_fr, shape);
        a.ov_offset = if r.next_u32() % 4 == 0 { r.range(0, ov_size) } else { 0 };
        a.prev_alias_symmetry = (r.next_u32() % 2) as i32;
        a.prev_prev_alias_symmetry = (r.next_u32() % 2) as i32;
        if r.next_u32() % 6 == 0 {
            a.p_fac_zir = Some((0..512).map(|_| r.i32() >> r.range(0, 8)).collect());
        }
        if r.next_u32() % 6 == 0 {
            a.p_asym_ovlp = Some(signal(&mut r, 9, (fl / 2).max(0) as usize));
        }
        let mut b = a.clone();
        let out_len = match r.next_u32() % 4 {
            0 => r.range(0, 2 * tl) as usize,
            _ => (n_spec * tl) as usize,
        };
        let no_out = match r.next_u32() % 4 {
            0 => r.range(0, 2 * n_spec * tl),
            1 => (n_spec * tl) / 2,
            _ => n_spec * tl,
        };
        let spec = spectrum(&mut r, (n_spec * tl) as usize);
        let sf = scalefactors(&mut r, n_spec as usize);
        let g = gain(&mut r);
        let flags = (r.next_u32() % 2) as i32;
        run_both(
            &mut a,
            &mut b,
            &mut r,
            out_len,
            &spec,
            &sf,
            n_spec,
            no_out,
            tl,
            slope(fl, shape),
            fl,
            slope(fr, shape),
            fr,
            g,
            flags,
            &what,
        );
    }
}

/// mdct_block against its straight port on encoder-like sequences and random, also
/// inconsistent, parameters (left slopes longer than the transform, missing slopes, short time
/// signals): output, exponents, return value, state and the counters of the deviations.
#[test]
fn mdct_block_fast_equals_straight_port() {
    const LENS: &[i32] = &[1024, 960, 768, 512, 480, 256, 240, 128, 120, 96, 64, 60, 48, 32];
    let mut r = Rng::new(0x1a18_2000);
    for run in 0..300 {
        let mut a = MdctT::default();
        mdct_init(&mut a, 0);
        let mut b = a.clone();
        let frame_len = [1024, 960, 768, 512, 480][r.range(0, 4) as usize];
        for frame in 0..20 {
            let what = format!("run {run} frame {frame}");
            let (n_spec, tl) = match r.next_u32() % 4 {
                0 => (8, frame_len >> 3),
                1 => (r.range(1, 3), LENS[r.range(0, LENS.len() as i32 - 1) as usize]),
                _ => (1, frame_len),
            };
            let shape = r.range(0, 2);
            let fr = match r.next_u32() % 4 {
                0 => LENS[r.range(0, LENS.len() as i32 - 1) as usize],
                1 => tl >> 3,
                _ => tl,
            };
            let no_in = match r.next_u32() % 4 {
                0 => r.range(0, 2 * tl),
                _ => tl,
            };
            let n_time = match r.next_u32() % 4 {
                0 => r.range(0, (n_spec + 2) * tl) as usize,
                _ => ((n_spec + 1) * tl) as usize + (no_in - tl).max(0) as usize,
            };
            let time: Vec<i16> = match r.next_u32() % 8 {
                0 => vec![i16::MIN; n_time],
                1 => vec![i16::MAX; n_time],
                _ => (0..n_time).map(|_| r.i16()).collect(),
            };
            let garbage: Vec<i32> = (0..(n_spec * tl) as usize + 8).map(|_| r.i32()).collect();
            let (mut ma, mut mb) = (garbage.clone(), garbage);
            let (mut ea, mut eb) = ([0x55i16; 8], [0x55i16; 8]);
            let wrs = if r.next_u32() % 16 == 0 { None } else { slope(fr, shape) };
            let run_a = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let d0 = mdct_block_dropped_accesses();
                let n = mdct_block(&mut a, &time, no_in, &mut ma, n_spec, tl, wrs, fr, &mut ea);
                let d1 = mdct_block_dropped_accesses();
                (n, d1.0 - d0.0, d1.1 - d0.1, d1.2 - d0.2)
            }));
            let run_b = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let d0 = mdct_block_dropped_accesses();
                let n = mdct_block_ref(&mut b, &time, no_in, &mut mb, n_spec, tl, wrs, fr, &mut eb);
                let d1 = mdct_block_dropped_accesses();
                (n, d1.0 - d0.0, d1.1 - d0.1, d1.2 - d0.2)
            }));
            match (run_a, run_b) {
                (Ok(x), Ok(y)) => assert_eq!(x, y, "{what}: return value, deviations"),
                // Both index outside of the time signal (the C code reads outside of it); the
                // states may differ from here on, start a new run.
                (Err(_), Err(_)) => break,
                (x, y) => panic!("{what}: only one panicked: {} {}", x.is_err(), y.is_err()),
            }
            assert_eq!(ma, mb, "{what}: mdctData");
            assert_eq!(ea, eb, "{what}: exponents");
            same_state(&a, &b, &what);
        }
    }
}

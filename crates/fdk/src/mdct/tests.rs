// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The optimized `imlt_block` and `mdct_block` against their straight ports, with the arithmetic
//! of the selected variant (these unit tests need no C reference, so they run with every
//! `FDK_AAC_RS_ARCH` variant on the host; the `cref` tests run the C reference's variant).

use super::*;
use crate::fft::testutil::*;
use crate::tools_rom::fdk_get_window_slope;

const LENS: &[i32] = &[1024, 960, 768, 512, 480, 256, 240, 128, 120, 96, 64, 48, 32];

fn slope(len: i32, shape: i32) -> Option<&'static [FixpWtp]> {
    if len > 0 {
        fdk_get_window_slope(len, shape)
    } else {
        None
    }
}

/// imlt_block on random, also inconsistent, parameters.
#[test]
fn imlt_block_equals_straight_port() {
    let mut r = Rng(0x5eed_0005);
    for case in 0..6000 {
        let tl = LENS[r.range(0, LENS.len() as i32 - 1) as usize];
        let n_spec = if r.next() % 3 == 0 { r.range(1, 8) } else { 1 };
        let shape = r.range(0, 2);
        let pick = |r: &mut Rng| match r.next() % 4 {
            0 => LENS[r.range(0, LENS.len() as i32 - 1) as usize],
            1 => tl >> 3,
            _ => tl,
        };
        let (fl, fr) = (pick(&mut r), pick(&mut r));
        let ov_size = [768, 1024, 2048, 64][r.range(0, 3) as usize];
        let mut a = MdctT { overlap: signal(&mut r, 9, ov_size as usize), ..Default::default() };
        mdct_init(&mut a, ov_size);
        a.prev_tl = pick(&mut r);
        a.prev_fr = if r.next() % 2 == 0 { fl } else { pick(&mut r) };
        a.prev_nr = (tl - a.prev_fr) >> 1;
        a.prev_wrs = slope(a.prev_fr, shape);
        a.prev_alias_symmetry = (r.next() % 2) as i32;
        a.prev_prev_alias_symmetry = (r.next() % 2) as i32;
        let mut b = a.clone();
        let n = (n_spec * tl) as usize;
        let k = if r.next() % 8 == 0 { r.range(0, 5) as usize } else { 9 };
        let spec = signal(&mut r, k, n);
        let sf: Vec<i16> = (0..n_spec).map(|_| r.range(-40, 40) as i16).collect();
        let gain = if r.next() % 4 == 0 { r.i32() } else { 0 };
        let flags = (r.next() % 2) as i32;
        let out_len = if r.next() % 4 == 0 { r.range(0, 2 * tl) as usize } else { n };
        let g: Vec<i32> = (0..out_len).map(|_| r.i32()).collect();
        let (mut oa, mut ob) = (g.clone(), g);
        let (mut sa, mut sb) = (spec.clone(), spec);
        let w = |l| slope(l, shape);
        let no_out = n_spec * tl;
        let na = imlt_block(
            &mut a,
            &mut oa,
            &mut sa,
            &sf,
            n_spec,
            no_out,
            tl,
            w(fl),
            fl,
            w(fr),
            fr,
            gain,
            flags,
        );
        let nb = imlt_block_ref(
            &mut b,
            &mut ob,
            &mut sb,
            &sf,
            n_spec,
            no_out,
            tl,
            w(fl),
            fl,
            w(fr),
            fr,
            gain,
            flags,
        );
        assert_eq!((na, &oa, &sa), (nb, &ob, &sb), "case {case}");
        assert_eq!(a.overlap, b.overlap, "case {case}");
        assert_eq!((a.ov_offset, a.prev_nr, a.prev_fr), (b.ov_offset, b.prev_nr, b.prev_fr));
    }
}

/// mdct_block on encoder-like parameters.
#[test]
fn mdct_block_equals_straight_port() {
    let mut r = Rng(0x5eed_0006);
    for run in 0..100 {
        let mut a = MdctT::default();
        mdct_init(&mut a, 0);
        let mut b = a.clone();
        let frame_len = [1024, 960, 768, 512, 480][r.range(0, 4) as usize];
        for frame in 0..20 {
            let short = r.next() % 4 == 0;
            let (n_spec, tl) = if short { (8, frame_len >> 3) } else { (1, frame_len) };
            let shape = r.range(0, 2);
            let fr = if r.next() % 3 == 0 { tl >> 3 } else { tl };
            let time: Vec<i16> =
                (0..2 * frame_len as usize).map(|_| (r.i32() >> 16) as i16).collect();
            let g: Vec<i32> = (0..(n_spec * tl) as usize).map(|_| r.i32()).collect();
            let (mut ma, mut mb) = (g.clone(), g);
            let (mut ea, mut eb) = ([0i16; 8], [0i16; 8]);
            let w = slope(fr, shape);
            let na = mdct_block(&mut a, &time, tl, &mut ma, n_spec, tl, w, fr, &mut ea);
            let nb = mdct_block_ref(&mut b, &time, tl, &mut mb, n_spec, tl, w, fr, &mut eb);
            assert_eq!((na, &ma, ea), (nb, &mb, eb), "run {run} frame {frame}");
        }
    }
}

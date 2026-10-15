// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The optimized QMF slot functions against their straight ports, with the arithmetic of the
//! selected variant (these unit tests run with every `FDK_AAC_RS_ARCH` variant on the host).

use super::*;
use crate::fft::testutil::*;

const FLAGS: &[u32] = &[
    0,
    QMF_FLAG_LP,
    QMF_FLAG_DOWNSAMPLED,
    QMF_FLAG_CLDFB,
    QMF_FLAG_CLDFB | QMF_FLAG_LP,
    QMF_FLAG_MPSLDFB,
    QMF_FLAG_MPSLDFB | QMF_FLAG_CLDFB,
];

fn value(r: &mut Rng, kind: u64) -> i32 {
    match kind {
        0 => r.i32(),
        1 => r.i32() >> r.range(1, 31),
        2 => [i32::MIN, i32::MAX, i32::MIN + 1, -1, 0, 1][r.range(0, 5) as usize],
        _ => 0,
    }
}

fn check_synthesis<T: QmfPcm + PartialEq + core::fmt::Debug>(seed: u64, conv: fn(i32) -> T) {
    let mut r = Rng(seed);
    for nch in [8, 16, 20, 24, 32, 64] {
        for &flags in FLAGS {
            for run in 0..12 {
                let mut fb = QmfFilterBank::default();
                let mut st = vec![0i32; 9 * 64];
                if qmf_init_synthesis_filter_bank(&mut fb, Some(&mut st), 32, nch, nch, nch, flags)
                    != 0
                    || fb.p_filter.is_empty()
                {
                    continue;
                }
                let lp = fb.flags & QMF_FLAG_LP != 0;
                if !lp && fb.flags & QMF_FLAG_CLDFB != 0 && fb.t_cos.is_empty() {
                    continue;
                }
                fb.out_scalefactor = r.range(-40, 60);
                fb.out_gain_m = if r.next() % 3 == 0 { i32::MIN } else { r.i32() };
                fb.out_gain_e = r.range(-8, 8);
                fb.lsb = r.range(0, nch);
                fb.usb = r.range(fb.lsb, nch);
                let kind = r.next() % 4;
                let mut sa: Vec<i32> = (0..9 * 64).map(|_| value(&mut r, kind)).collect();
                let mut sb = sa.clone();
                let stride = r.range(1, 2);
                for slot in 0..4 {
                    let re: Vec<i32> = (0..64).map(|_| value(&mut r, kind)).collect();
                    let im: Vec<i32> = (0..64).map(|_| value(&mut r, kind)).collect();
                    let (sfl, sfh) = (r.range(-40, 40), r.range(-40, 40));
                    let n = (nch * stride) as usize + 3;
                    let g: Vec<T> = (0..n).map(|_| conv(r.i32())).collect();
                    let (mut oa, mut ob) = (g.clone(), g);
                    let (mut wa, mut wb) = (vec![0i32; 128], vec![0i32; 128]);
                    let ims: &[i32] = if lp { &[] } else { &im };
                    qmf_synthesis_filtering_slot(
                        &fb, &mut sa, &re, ims, sfl, sfh, &mut oa, stride, &mut wa,
                    );
                    qmf_synthesis_filtering_slot_ref(
                        &fb, &mut sb, &re, ims, sfl, sfh, &mut ob, stride, &mut wb,
                    );
                    let what = format!("nch {nch} flags {flags:#x} run {run} slot {slot}");
                    assert_eq!(oa, ob, "{what}: output");
                    assert_eq!((&sa, &wa), (&sb, &wb), "{what}: states, work buffer");
                }
            }
        }
    }
}

#[test]
fn synthesis_slot_equals_straight_port() {
    check_synthesis::<i16>(0x5eed_0007, |v| (v >> 16) as i16);
    check_synthesis::<i32>(0x5eed_0008, |v| v);
}

fn check_analysis<T: QmfPcm<Qas = T> + PartialEq + core::fmt::Debug>(
    seed: u64,
    conv: fn(i32) -> T,
) {
    let mut r = Rng(seed);
    for nch in [8, 16, 20, 24, 32, 40, 64] {
        for &flags in FLAGS {
            for run in 0..12 {
                let mut fb = QmfFilterBank::default();
                let kind = r.next() % 4;
                let mut sa: Vec<T> = (0..10 * 64).map(|_| conv(value(&mut r, kind))).collect();
                if qmf_init_analysis_filter_bank(
                    &mut fb,
                    Some(&mut sa),
                    32,
                    r.range(0, nch),
                    nch,
                    nch,
                    flags | QMF_FLAG_KEEP_STATES,
                ) != 0
                    || fb.p_filter.is_empty()
                {
                    continue;
                }
                let lp = fb.flags & QMF_FLAG_LP != 0;
                let rotates = !lp
                    && (nch != 64 || fb.flags & (QMF_FLAG_CLDFB | QMF_FLAG_MPSLDFB) != 0)
                    && fb.flags & QMF_FLAG_MPSLDFB_OPTIMIZE_MODULATION == 0;
                if rotates && fb.t_cos.is_empty() {
                    continue;
                }
                let mut sb = sa.clone();
                let stride = r.range(1, 2);
                for slot in 0..4 {
                    let tin: Vec<T> = (0..128).map(|_| conv(value(&mut r, kind))).collect();
                    let g: Vec<i32> = (0..64).map(|_| r.i32()).collect();
                    let (mut ra, mut rb) = (g.clone(), g.clone());
                    let (mut ia, mut ib) = (g.clone(), g);
                    let (mut wa, mut wb) = (vec![0i32; 128], vec![0i32; 128]);
                    let (ia_s, ib_s): (&mut [i32], &mut [i32]) =
                        if lp { (&mut [], &mut []) } else { (&mut ia, &mut ib) };
                    qmf_analysis_filtering_slot(&fb, &mut sa, &mut ra, ia_s, &tin, stride, &mut wa);
                    qmf_analysis_filtering_slot_ref(
                        &fb, &mut sb, &mut rb, ib_s, &tin, stride, &mut wb,
                    );
                    let what = format!("nch {nch} flags {flags:#x} run {run} slot {slot}");
                    assert_eq!((&ra, &ia, &wa), (&rb, &ib, &wb), "{what}: output");
                    assert_eq!(sa, sb, "{what}: states");
                }
            }
        }
    }
}

#[test]
fn analysis_slot_equals_straight_port() {
    check_analysis::<i16>(0x5eed_0009, |v| (v >> 16) as i16);
    check_analysis::<i32>(0x5eed_000a, |v| v);
}

/// `qmf_analysis_filtering` (the states in a window buffer, shifted once per block of slots)
/// against the straight port (shifted after every slot), for 1 to 70 slots per call and
/// several calls in a row.
fn check_analysis_multi<T: QmfPcm<Qas = T> + PartialEq + core::fmt::Debug>(
    seed: u64,
    conv: fn(i32) -> T,
) {
    let mut r = Rng(seed);
    for nch in [8, 12, 16, 20, 24, 32, 40, 64] {
        for &flags in FLAGS {
            for run in 0..6 {
                let mut fb = QmfFilterBank::default();
                let kind = r.next() % 4;
                let mut sa: Vec<T> = (0..10 * 64).map(|_| conv(value(&mut r, kind))).collect();
                let no_col = [1, 2, 15, 16, 17, 32, 33, 64, r.range(1, 70)][r.range(0, 8) as usize];
                if qmf_init_analysis_filter_bank(
                    &mut fb,
                    Some(&mut sa),
                    no_col,
                    r.range(0, nch),
                    nch,
                    nch,
                    flags | QMF_FLAG_KEEP_STATES,
                ) != 0
                    || fb.p_filter.is_empty()
                {
                    continue;
                }
                let lp = fb.flags & QMF_FLAG_LP != 0;
                let rotates = !lp
                    && (nch != 64 || fb.flags & (QMF_FLAG_CLDFB | QMF_FLAG_MPSLDFB) != 0)
                    && fb.flags & QMF_FLAG_MPSLDFB_OPTIMIZE_MODULATION == 0;
                if rotates && fb.t_cos.is_empty() {
                    continue;
                }
                let mut sb = sa.clone();
                let stride = r.range(1, 2);
                for call in 0..3 {
                    let n = (no_col * nch * stride) as usize + 64;
                    let tin: Vec<T> = (0..n).map(|_| conv(value(&mut r, kind))).collect();
                    let g: Vec<Vec<i32>> =
                        (0..no_col).map(|_| (0..64).map(|_| r.i32()).collect()).collect();
                    let (mut ra, mut rb, mut ia, mut ib) = (g.clone(), g.clone(), g.clone(), g);
                    let (mut wa, mut wb) = (vec![0i32; 128], vec![0i32; 128]);
                    let (mut sca, mut scb) = (QmfScaleFactor::default(), QmfScaleFactor::default());
                    let e = r.range(-3, 3);
                    qmf_analysis_filtering(
                        &fb, &mut sa, &mut ra, &mut ia, &mut sca, &tin, e, stride, &mut wa,
                    );
                    qmf_analysis_filtering_ref(
                        &fb, &mut sb, &mut rb, &mut ib, &mut scb, &tin, e, stride, &mut wb,
                    );
                    let what = format!("nch {nch} flags {flags:#x} run {run} call {call}");
                    assert_eq!((&ra, &ia, &wa), (&rb, &ib, &wb), "{what}: output");
                    assert_eq!(sa, sb, "{what}: states");
                    assert_eq!(sca.lb_scale, scb.lb_scale, "{what}: scale");
                }
            }
        }
    }
}

#[test]
fn analysis_filtering_equals_straight_port() {
    check_analysis_multi::<i16>(0x5eed_000b, |v| (v >> 16) as i16);
    check_analysis_multi::<i32>(0x5eed_000c, |v| v);
}

// The optimized QMF kernels against their straight ports (the `_ref` functions).
//
// Every filter bank configuration of the library (all channel counts and flags), random output
// scale factors and gains (including the values the PCM formatting saturates for and the gain
// -1.0 that is not applied), both PCM types, strides 1 and 2, and input signals with edge
// values. Outputs and the complete filter states are compared after every slot.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_pcm::*;

const FLAGS: &[u32] = &[
    0,
    QMF_FLAG_LP,
    QMF_FLAG_DOWNSAMPLED,
    QMF_FLAG_CLDFB,
    QMF_FLAG_CLDFB | QMF_FLAG_LP,
    QMF_FLAG_MPSLDFB,
    QMF_FLAG_MPSLDFB | QMF_FLAG_CLDFB,
];

/// A value of the kind `kind`: random full scale, scaled random, extremes, zero.
fn value(r: &mut Rng, kind: u32) -> i32 {
    match kind {
        0 => r.i32(),
        1 => r.i32() >> r.range(1, 31),
        2 => [i32::MIN, i32::MAX, i32::MIN + 1, -1, 0, 1][r.range(0, 5) as usize],
        3 => 0,
        _ => {
            if r.next_u32() % 2 == 0 {
                i32::MIN
            } else {
                i32::MAX
            }
        }
    }
}

trait Pcm: QmfPcm + core::fmt::Debug + PartialEq {
    fn garbage(r: &mut Rng) -> Self;
}
impl Pcm for i16 {
    fn garbage(r: &mut Rng) -> i16 {
        r.i16()
    }
}
impl Pcm for i32 {
    fn garbage(r: &mut Rng) -> i32 {
        r.i32()
    }
}

/// A synthesis filter bank of `nch` channels with `flags` and a random output scale and gain;
/// `None` if the configuration has no prototype filter.
fn syn_bank(r: &mut Rng, nch: i32, flags: u32) -> Option<QmfFilterBank> {
    let mut fb = QmfFilterBank::default();
    let mut st = vec![0i32; 9 * 64];
    if qmf_init_synthesis_filter_bank(&mut fb, Some(&mut st), 32, nch, nch, nch, flags) != 0
        || fb.p_filter.is_empty()
    {
        return None;
    }
    fb.out_scalefactor = match r.next_u32() % 4 {
        0 => r.range(-40, 60),
        _ => r.range(0, 20),
    };
    fb.out_gain_m = match r.next_u32() % 3 {
        0 => i32::MIN,
        1 => r.i32(),
        _ => [i32::MAX, -i32::MAX, 0x4000_0000, 0x7fff_0000][r.range(0, 3) as usize],
    };
    fb.out_gain_e = r.range(-8, 8);
    Some(fb)
}

fn check_syn_fir<T: Pcm>(seed: u64) {
    let mut r = Rng::new(seed);
    for nch in [8, 12, 16, 20, 24, 32, 40, 64] {
        for &flags in FLAGS {
            for run in 0..40 {
                let Some(fb) = syn_bank(&mut r, nch, flags) else { continue };
                let nonsym = fb.flags & QMF_FLAG_NONSYMMETRIC != 0;
                let kind = r.next_u32() % 5;
                let mut sa: Vec<i32> = (0..9 * 64).map(|_| value(&mut r, kind)).collect();
                let mut sb = sa.clone();
                let stride = r.range(1, 2);
                for slot in 0..4 {
                    let what = format!("nch {nch} flags {flags:#x} run {run} slot {slot}");
                    let re: Vec<i32> = (0..64).map(|_| value(&mut r, kind)).collect();
                    let im: Vec<i32> = (0..64).map(|_| value(&mut r, kind)).collect();
                    let n = (nch * stride) as usize + 3;
                    let g: Vec<T> = (0..n).map(|_| T::garbage(&mut r)).collect();
                    let (mut oa, mut ob) = (g.clone(), g);
                    if nonsym {
                        qmf_syn_prototype_fir_slot_non_symmetric(&fb, &mut sa, &re, &im, &mut oa, stride);
                        qmf_syn_prototype_fir_slot_non_symmetric_ref(&fb, &mut sb, &re, &im, &mut ob, stride);
                    } else {
                        qmf_syn_prototype_fir_slot(&fb, &mut sa, &re, &im, &mut oa, stride);
                        qmf_syn_prototype_fir_slot_ref(&fb, &mut sb, &re, &im, &mut ob, stride);
                    }
                    assert_eq!(oa, ob, "{what}: output");
                    assert_eq!(sa, sb, "{what}: states");
                }
            }
        }
    }
}

#[test]
fn qmf_syn_prototype_fir_equals_straight_port() {
    check_syn_fir::<i16>(0x9f00);
    check_syn_fir::<i32>(0x9f01);
}

/// The complete synthesis slot (inverse modulation and prototype filter) against the straight
/// port, with random band limits and scale factors.
fn check_syn_slot<T: Pcm>(seed: u64) {
    let mut r = Rng::new(seed);
    for nch in [8, 12, 16, 20, 24, 32, 40, 64] {
        for &flags in FLAGS {
            for run in 0..30 {
                let Some(mut fb) = syn_bank(&mut r, nch, flags) else { continue };
                // The modulations need their DCT and twiddle tables (see the qmf tests).
                let lp = fb.flags & QMF_FLAG_LP != 0;
                let cldfb = fb.flags & QMF_FLAG_CLDFB != 0;
                if !lp && cldfb && fb.t_cos.is_empty() {
                    continue;
                }
                fb.lsb = r.range(0, nch);
                fb.usb = r.range(fb.lsb, nch);
                let kind = r.next_u32() % 5;
                let mut sa: Vec<i32> = (0..9 * 64).map(|_| value(&mut r, kind)).collect();
                let mut sb = sa.clone();
                let stride = r.range(1, 2);
                for slot in 0..4 {
                    let what = format!("nch {nch} flags {flags:#x} run {run} slot {slot}");
                    let re: Vec<i32> = (0..64).map(|_| value(&mut r, kind)).collect();
                    let im: Vec<i32> = (0..64).map(|_| value(&mut r, kind)).collect();
                    let sc_range = if r.next_u32() % 4 == 0 { 45 } else { 12 };
                    let sfl = r.range(-sc_range, sc_range);
                    let sfh = r.range(-sc_range, sc_range);
                    let n = (nch * stride) as usize + 3;
                    let g: Vec<T> = (0..n).map(|_| T::garbage(&mut r)).collect();
                    let (mut oa, mut ob) = (g.clone(), g);
                    let mut wa = vec![0i32; 128];
                    let mut wb = vec![0i32; 128];
                    let ims: &[i32] = if lp { &[] } else { &im };
                    qmf_synthesis_filtering_slot(&fb, &mut sa, &re, ims, sfl, sfh, &mut oa, stride, &mut wa);
                    qmf_synthesis_filtering_slot_ref(&fb, &mut sb, &re, ims, sfl, sfh, &mut ob, stride, &mut wb);
                    assert_eq!(oa, ob, "{what}: output");
                    assert_eq!(sa, sb, "{what}: states");
                    assert_eq!(wa, wb, "{what}: work buffer");
                }
            }
        }
    }
}

#[test]
fn qmf_synthesis_slot_equals_straight_port() {
    check_syn_slot::<i16>(0x9f02);
    check_syn_slot::<i32>(0x9f03);
}

/// An analysis state or input value of the PCM type (full scale for `i16`, as for `i32`).
trait AnaPcm: Pcm {
    fn value(r: &mut Rng, kind: u32) -> Self;
}
impl AnaPcm for i16 {
    fn value(r: &mut Rng, kind: u32) -> i16 {
        (value(r, kind) >> 16) as i16
    }
}
impl AnaPcm for i32 {
    fn value(r: &mut Rng, kind: u32) -> i32 {
        value(r, kind)
    }
}

fn check_analysis<T>(seed: u64)
where
    T: AnaPcm + QmfPcm<Qas = T>,
{
    let mut r = Rng::new(seed);
    for nch in [8, 12, 16, 20, 24, 32, 40, 64] {
        for &flags in FLAGS {
            for run in 0..20 {
                let mut fb = QmfFilterBank::default();
                let kind = r.next_u32() % 5;
                let mut sa: Vec<T> = (0..10 * 64).map(|_| T::value(&mut r, kind)).collect();
                let lsb = r.range(0, nch);
                if qmf_init_analysis_filter_bank(
                    &mut fb,
                    Some(&mut sa),
                    32,
                    lsb,
                    nch,
                    nch,
                    flags | QMF_FLAG_KEEP_STATES,
                ) != 0
                    || fb.p_filter.is_empty()
                {
                    continue;
                }
                // The modulations need their tables (see the qmf tests); skip the
                // configurations without them.
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
                    let what = format!("nch {nch} flags {flags:#x} run {run} slot {slot}");
                    let tin: Vec<T> = (0..64 * 2).map(|_| T::value(&mut r, kind)).collect();
                    let g: Vec<i32> = (0..64).map(|_| r.i32()).collect();
                    let (mut ra, mut rb) = (g.clone(), g.clone());
                    let (mut ia, mut ib) = (g.clone(), g);
                    let mut wa = vec![0i32; 128];
                    let mut wb = vec![0i32; 128];
                    let (ia_s, ib_s): (&mut [i32], &mut [i32]) =
                        if lp { (&mut [], &mut []) } else { (&mut ia, &mut ib) };
                    qmf_analysis_filtering_slot(&fb, &mut sa, &mut ra, ia_s, &tin, stride, &mut wa);
                    qmf_analysis_filtering_slot_ref(&fb, &mut sb, &mut rb, ib_s, &tin, stride, &mut wb);
                    assert_eq!(ra, rb, "{what}: real");
                    assert_eq!(ia, ib, "{what}: imag");
                    assert_eq!(sa, sb, "{what}: states");
                    assert_eq!(wa, wb, "{what}: work buffer");
                }
            }
        }
    }
}

#[test]
fn qmf_analysis_slot_equals_straight_port() {
    check_analysis::<i16>(0x9f10);
    check_analysis::<i32>(0x9f11);
}

// Differential tests of fdk_aac_rs_fdk::qmf (and qmf_pcm) against the C++ reference.
//
// Every configuration of the library is run on a C++ filter bank and on the port side by side:
// initialization (all flags, channel counts, KEEP_STATES), then many consecutive frames of
// analysis or synthesis with random, full-scale, silent and structured signals, changes of the
// output scale factor and gain and of lsb/usb. Outputs, scale factors, the filter bank fields
// and the complete filter states are compared after every call.

use crate::fft::FFT_LENGTHS;
use core::ffi::c_void;
use fdk_aac_rs_cref::fdk_qmf::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::dct::dct_get_tables;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_pcm::qmf_syn_prototype_fir_slot;
use fdk_aac_rs_fdk::tools_rom::*;

/// The QMF tables in the order of `qmf_tables` of the shim.
fn qmf_tables() -> [&'static [i16]; 34] {
    [
        &QMF_PFILT120,
        &QMF_PFILT200,
        &QMF_PFILT240,
        &QMF_PFILT400,
        &QMF_PFILT640,
        &QMF_CLDFB_640,
        &QMF_CLDFB_320,
        &QMF_CLDFB_160,
        &QMF_CLDFB_80,
        &QMF_MPSLDFB_320,
        &QMF_MPSLDFB_640,
        &QMF_PHASESHIFT_COS16,
        &QMF_PHASESHIFT_SIN16,
        &QMF_PHASESHIFT_COS24,
        &QMF_PHASESHIFT_SIN24,
        &QMF_PHASESHIFT_COS32,
        &QMF_PHASESHIFT_SIN32,
        &QMF_PHASESHIFT_COS_DOWNSAMP32,
        &QMF_PHASESHIFT_SIN_DOWNSAMP32,
        &QMF_PHASESHIFT_COS40,
        &QMF_PHASESHIFT_SIN40,
        &QMF_PHASESHIFT_COS64,
        &QMF_PHASESHIFT_SIN64,
        &QMF_PHASESHIFT_COS32_CLDFB_ANA,
        &QMF_PHASESHIFT_COS32_CLDFB_SYN,
        &QMF_PHASESHIFT_SIN32_CLDFB,
        &QMF_PHASESHIFT_COS16_CLDFB_ANA,
        &QMF_PHASESHIFT_COS16_CLDFB_SYN,
        &QMF_PHASESHIFT_SIN16_CLDFB,
        &QMF_PHASESHIFT_COS8_CLDFB_ANA,
        &QMF_PHASESHIFT_COS8_CLDFB_SYN,
        &QMF_PHASESHIFT_SIN8_CLDFB,
        &QMF_PHASESHIFT_COS64_CLDFB,
        &QMF_PHASESHIFT_SIN64_CLDFB,
    ]
}

fn table_id(t: &[i16]) -> i32 {
    if t.is_empty() {
        return 0;
    }
    match qmf_tables()
        .iter()
        .position(|x| x.as_ptr() == t.as_ptr() && x.len() == t.len())
    {
        Some(i) => i as i32 + 1,
        None => -1,
    }
}

/// The fields of a filter bank in the layout of `cref_qmf_dump`.
pub fn dump(fb: &QmfFilterBank) -> [i32; 16] {
    [
        table_id(fb.p_filter),
        fb.filter_states as i32,
        fb.filter_size,
        table_id(fb.t_cos),
        table_id(fb.t_sin),
        fb.filter_scale,
        fb.no_channels,
        fb.no_col,
        fb.lsb,
        fb.usb,
        fb.syn_scalefactor,
        fb.out_scalefactor,
        fb.out_gain_m,
        fb.out_gain_e,
        fb.flags as i32,
        fb.p_stride as i32,
    ]
}

/// A C++ `QMF_FILTER_BANK`.
pub struct CQmf(pub *mut c_void);

impl CQmf {
    pub fn new() -> Self {
        CQmf(unsafe { cref_qmf_new() })
    }
    pub fn dump(&self) -> [i32; 16] {
        let mut out = [0i32; 16];
        unsafe { cref_qmf_dump(self.0, out.as_mut_ptr()) };
        out
    }
}

impl Drop for CQmf {
    fn drop(&mut self) {
        unsafe { cref_qmf_free(self.0) }
    }
}

const SLOT: usize = 64;
const MAX_COLS: usize = 64;

/// The PCM types of the two instantiations of `qmf_pcm.h`.
trait TestPcm: QmfPcm<Qas = Self> + PartialEq + core::fmt::Debug {
    const NAME: &'static str;
    fn sample(rng: &mut Rng, kind: u32, n: usize, amp: i32) -> Self;
    unsafe fn c_init_ana(h: *mut c_void, s: *mut Self, cols: i32, lsb: i32, usb: i32, nch: i32, flags: i32) -> i32;
    unsafe fn c_ana_slot(h: *mut c_void, re: *mut i32, im: *mut i32, t: *const Self, stride: i32, w: *mut i32);
    unsafe fn c_ana(h: *mut c_void, re: *mut i32, im: *mut i32, sc: *mut i32, t: *const Self, e: i32, stride: i32, w: *mut i32);
    unsafe fn c_syn_slot(h: *mut c_void, re: *const i32, im: *const i32, sfl: i32, sfh: i32, t: *mut Self, stride: i32, w: *mut i32);
    unsafe fn c_syn(h: *mut c_void, re: *mut i32, im: *mut i32, sc: *const i32, ov: i32, t: *mut Self, stride: i32, w: *mut i32);
    unsafe fn c_proto(h: *mut c_void, re: *mut i32, im: *mut i32, t: *mut Self, stride: i32);
}

/// A sample of a test signal of the given kind: 0 random full scale, 1 silence, 2 random with
/// less amplitude, 3 sine, 4 alternating extremes, 5 sparse impulses.
fn signal_value(rng: &mut Rng, kind: u32, n: usize, bits: u32, amp: i32) -> i64 {
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    match kind {
        0 => (rng.next_u64() as i64) >> (64 - bits),
        1 => 0,
        2 => ((rng.next_u64() as i64) >> (64 - bits)) >> (amp as u32 % (bits - 1)),
        3 => {
            let a = (max >> (amp as u32 % 8)) as f64;
            (a * ((n as f64) * 0.05 * (1.0 + amp as f64 * 0.37)).sin()) as i64
        }
        4 => {
            if n % 2 == 0 {
                max
            } else {
                min
            }
        }
        _ => {
            if rng.next_u32() % 23 == 0 {
                if rng.next_u32() % 2 == 0 {
                    max
                } else {
                    min
                }
            } else {
                0
            }
        }
    }
}

impl TestPcm for i16 {
    const NAME: &'static str = "INT_PCM";
    fn sample(rng: &mut Rng, kind: u32, n: usize, amp: i32) -> i16 {
        signal_value(rng, kind, n, 16, amp) as i16
    }
    unsafe fn c_init_ana(h: *mut c_void, s: *mut i16, cols: i32, lsb: i32, usb: i32, nch: i32, flags: i32) -> i32 {
        cref_qmf_init_analysis_16(h, s, cols, lsb, usb, nch, flags)
    }
    unsafe fn c_ana_slot(h: *mut c_void, re: *mut i32, im: *mut i32, t: *const i16, stride: i32, w: *mut i32) {
        cref_qmf_analysis_slot_16(h, re, im, t, stride, w)
    }
    unsafe fn c_ana(h: *mut c_void, re: *mut i32, im: *mut i32, sc: *mut i32, t: *const i16, e: i32, stride: i32, w: *mut i32) {
        cref_qmf_analysis_16(h, re, im, sc, t, e, stride, w)
    }
    unsafe fn c_syn_slot(h: *mut c_void, re: *const i32, im: *const i32, sfl: i32, sfh: i32, t: *mut i16, stride: i32, w: *mut i32) {
        cref_qmf_synthesis_slot_16(h, re, im, sfl, sfh, t, stride, w)
    }
    unsafe fn c_syn(h: *mut c_void, re: *mut i32, im: *mut i32, sc: *const i32, ov: i32, t: *mut i16, stride: i32, w: *mut i32) {
        cref_qmf_synthesis_16(h, re, im, sc, ov, t, stride, w)
    }
    unsafe fn c_proto(h: *mut c_void, re: *mut i32, im: *mut i32, t: *mut i16, stride: i32) {
        cref_qmf_syn_prototype_fir_slot_16(h, re, im, t, stride)
    }
}

impl TestPcm for i32 {
    const NAME: &'static str = "LONG";
    fn sample(rng: &mut Rng, kind: u32, n: usize, amp: i32) -> i32 {
        signal_value(rng, kind, n, 32, amp) as i32
    }
    unsafe fn c_init_ana(h: *mut c_void, s: *mut i32, cols: i32, lsb: i32, usb: i32, nch: i32, flags: i32) -> i32 {
        cref_qmf_init_analysis_32(h, s, cols, lsb, usb, nch, flags)
    }
    unsafe fn c_ana_slot(h: *mut c_void, re: *mut i32, im: *mut i32, t: *const i32, stride: i32, w: *mut i32) {
        cref_qmf_analysis_slot_32(h, re, im, t, stride, w)
    }
    unsafe fn c_ana(h: *mut c_void, re: *mut i32, im: *mut i32, sc: *mut i32, t: *const i32, e: i32, stride: i32, w: *mut i32) {
        cref_qmf_analysis_32(h, re, im, sc, t, e, stride, w)
    }
    unsafe fn c_syn_slot(h: *mut c_void, re: *const i32, im: *const i32, sfl: i32, sfh: i32, t: *mut i32, stride: i32, w: *mut i32) {
        cref_qmf_synthesis_slot_32(h, re, im, sfl, sfh, t, stride, w)
    }
    unsafe fn c_syn(h: *mut c_void, re: *mut i32, im: *mut i32, sc: *const i32, ov: i32, t: *mut i32, stride: i32, w: *mut i32) {
        cref_qmf_synthesis_32(h, re, im, sc, ov, t, stride, w)
    }
    unsafe fn c_proto(h: *mut c_void, re: *mut i32, im: *mut i32, t: *mut i32, stride: i32) {
        cref_qmf_syn_prototype_fir_slot_32(h, re, im, t, stride)
    }
}

/// Whether the DCT of length `l` exists (`type4`: DCT-IV/DST-IV, otherwise DCT-II/DCT-III);
/// the reference has no defined result for the other lengths.
fn dct_ok(l: i32, type4: bool) -> bool {
    if l <= 0 || l % 2 != 0 || !FFT_LENGTHS.contains(&(l / 2)) {
        return false;
    }
    let (twiddle, sin_twiddle, _) = dct_get_tables(l);
    sin_twiddle.is_some() && (!type4 || twiddle.is_some())
}

/// Whether a filter bank configuration has a defined result in the reference (all tables set).
fn runnable(nch: i32, flags: u32, synthesis: bool) -> bool {
    let lp = flags & QMF_FLAG_LP != 0;
    let mps = flags & QMF_FLAG_MPSLDFB != 0;
    // The CLDFB tables are only set without MPSLDFB, but the CLDFB flag stays set.
    let cldfb_tables = flags & QMF_FLAG_CLDFB != 0 && !mps;
    let cldfb_flag = flags & QMF_FLAG_CLDFB != 0;
    let filter = if mps {
        nch == 64 || nch == 32
    } else if cldfb_tables {
        matches!(nch, 64 | 32 | 16 | 8)
    } else {
        matches!(nch, 64 | 32 | 24 | 16 | 12 | 8 | 20) || (nch == 40 && !synthesis)
    };
    if !filter {
        return false;
    }
    let has_twiddles = cldfb_tables || (!mps && matches!(nch, 64 | 32 | 24 | 16 | 40));
    if lp {
        // LP: odd modulation (DCT-IV) with the CLDFB flag, even modulation (DCT-II/III)
        // otherwise; no twiddles.
        dct_ok(nch, cldfb_flag)
    } else {
        // HQ: the analysis rotates with t_cos/t_sin except for 64 bands in standard mode and
        // with MPSLDFB (OPTIMIZE_MODULATION); the synthesis uses them with the CLDFB flag.
        let needs_twiddles = if synthesis {
            cldfb_flag
        } else {
            !mps && (nch != 64 || cldfb_flag)
        };
        dct_ok(nch, true) && (has_twiddles || !needs_twiddles)
    }
}

fn configs(synthesis: bool) -> Vec<(i32, u32)> {
    let mut v = Vec::new();
    for nch in [8, 12, 16, 20, 24, 32, 40, 64] {
        for flags in [
            0,
            QMF_FLAG_LP,
            QMF_FLAG_DOWNSAMPLED,
            QMF_FLAG_DOWNSAMPLED | QMF_FLAG_LP,
            QMF_FLAG_CLDFB,
            QMF_FLAG_CLDFB | QMF_FLAG_LP,
            QMF_FLAG_MPSLDFB,
            QMF_FLAG_MPSLDFB | QMF_FLAG_LP,
            QMF_FLAG_MPSLDFB | QMF_FLAG_CLDFB,
            QMF_FLAG_MPSLDFB | QMF_FLAG_CLDFB | QMF_FLAG_LP,
        ] {
            if runnable(nch, flags, synthesis) {
                v.push((nch, flags));
            }
        }
    }
    v
}

#[test]
fn qmf_init() {
    let mut rng = Rng::new(0x9f1);
    let flag_sets = [
        0,
        QMF_FLAG_LP,
        QMF_FLAG_CLDFB,
        QMF_FLAG_CLDFB | QMF_FLAG_LP,
        QMF_FLAG_MPSLDFB,
        QMF_FLAG_MPSLDFB | QMF_FLAG_CLDFB,
        QMF_FLAG_DOWNSAMPLED,
        QMF_FLAG_DOWNSAMPLED | QMF_FLAG_LP,
        QMF_FLAG_KEEP_STATES,
        QMF_FLAG_NONSYMMETRIC,
        QMF_FLAG_MPSLDFB_OPTIMIZE_MODULATION | QMF_FLAG_LP,
    ];
    let channels = [0, 1, 7, 8, 12, 16, 20, 24, 32, 40, 48, 64, 128];
    for _ in 0..3000 {
        let nch = channels[(rng.next_u32() as usize) % channels.len()];
        let mut flags = flag_sets[(rng.next_u32() as usize) % flag_sets.len()];
        if rng.next_u32() % 4 == 0 {
            flags |= QMF_FLAG_KEEP_STATES;
        }
        let cols = rng.range(0, 64);
        let lsb = rng.range(-2, 140);
        let usb = rng.range(-2, 140);
        let with_states = rng.next_u32() % 5 != 0;
        let kind = rng.next_u32() % 3; // 0 synthesis, 1 analysis 16, 2 analysis 32

        let c = CQmf::new();
        let mut fb = QmfFilterBank::default();
        // A first initialization gives the old outScalefactor that KEEP_STATES uses.
        if rng.next_u32() % 2 == 0 {
            let (n0, f0) = (channels[(rng.next_u32() as usize) % channels.len()], flag_sets[1]);
            unsafe { cref_qmf_init_synthesis(c.0, core::ptr::null_mut(), 1, 0, 0, n0, f0 as i32) };
            qmf_init_synthesis_filter_bank(&mut fb, None, 1, 0, 0, n0, f0);
            let sf = rng.range(-12, 12);
            unsafe { cref_qmf_change_out_scalefactor(c.0, sf) };
            qmf_change_out_scalefactor(&mut fb, &mut [], sf);
            assert_eq!(dump(&fb), c.dump());
        }
        match kind {
            0 => {
                let mut cs: Vec<i32> = (0..640).map(|_| rng.i32()).collect();
                let mut rs = cs.clone();
                let ec = unsafe {
                    cref_qmf_init_synthesis(
                        c.0,
                        if with_states { cs.as_mut_ptr() } else { core::ptr::null_mut() },
                        cols,
                        lsb,
                        usb,
                        nch,
                        flags as i32,
                    )
                };
                let er = qmf_init_synthesis_filter_bank(
                    &mut fb,
                    if with_states { Some(&mut rs[..]) } else { None },
                    cols,
                    lsb,
                    usb,
                    nch,
                    flags,
                );
                assert_eq!(er, ec, "init synthesis nch {nch} flags {flags:#x}");
                assert_eq!(dump(&fb), c.dump(), "init synthesis nch {nch} flags {flags:#x}");
                assert_eq!(rs, cs, "states nch {nch} flags {flags:#x}");
                assert_eq!(qmf_get_out_scalefactor(&fb), unsafe { cref_qmf_get_out_scalefactor(c.0) });
            }
            1 => {
                let mut cs: Vec<i16> = (0..640).map(|_| rng.i16()).collect();
                let mut rs = cs.clone();
                let ec = unsafe {
                    cref_qmf_init_analysis_16(
                        c.0,
                        if with_states { cs.as_mut_ptr() } else { core::ptr::null_mut() },
                        cols,
                        lsb,
                        usb,
                        nch,
                        flags as i32,
                    )
                };
                let er = qmf_init_analysis_filter_bank(
                    &mut fb,
                    if with_states { Some(&mut rs[..]) } else { None },
                    cols,
                    lsb,
                    usb,
                    nch,
                    flags,
                );
                assert_eq!(er, ec);
                assert_eq!(dump(&fb), c.dump(), "init analysis nch {nch} flags {flags:#x}");
                assert_eq!(rs, cs);
            }
            _ => {
                let mut cs: Vec<i32> = (0..640).map(|_| rng.i32()).collect();
                let mut rs = cs.clone();
                let ec = unsafe {
                    cref_qmf_init_analysis_32(
                        c.0,
                        if with_states { cs.as_mut_ptr() } else { core::ptr::null_mut() },
                        cols,
                        lsb,
                        usb,
                        nch,
                        flags as i32,
                    )
                };
                let er = qmf_init_analysis_filter_bank(
                    &mut fb,
                    if with_states { Some(&mut rs[..]) } else { None },
                    cols,
                    lsb,
                    usb,
                    nch,
                    flags,
                );
                assert_eq!(er, ec);
                assert_eq!(dump(&fb), c.dump(), "init analysis nch {nch} flags {flags:#x}");
                assert_eq!(rs, cs);
            }
        }
    }
}

fn analysis<T: TestPcm>(seed: u64, frames: usize) {
    let mut rng = Rng::new(seed);
    for (nch, flags) in configs(false) {
        for run in 0..3 {
            let c = CQmf::new();
            let mut fb = QmfFilterBank::default();
            // Room for 10 * 64 states; the filter bank uses 10 * nch.
            let mut cs: Vec<T> = (0..640).map(|n| T::sample(&mut rng, 0, n, 0)).collect();
            let mut rs = cs.clone();
            let cols = [1, 2, 15, 16, 30, 32, 64][(rng.next_u32() % 7) as usize];
            let lsb = rng.range(0, nch + 4);
            let usb = rng.range(0, nch + 4);
            let init_flags = flags | if run == 2 { QMF_FLAG_KEEP_STATES } else { 0 };
            let ec = unsafe {
                T::c_init_ana(c.0, cs.as_mut_ptr(), cols, lsb, usb, nch, init_flags as i32)
            };
            let er = qmf_init_analysis_filter_bank(
                &mut fb,
                Some(&mut rs[..]),
                cols,
                lsb,
                usb,
                nch,
                init_flags,
            );
            let what = format!("analysis {} nch {nch} flags {flags:#x} run {run}", T::NAME);
            assert_eq!(er, ec, "{what}");
            assert_eq!(dump(&fb), c.dump(), "{what}");
            assert_eq!(rs, cs, "{what}");

            for frame in 0..frames {
                let kind = rng.next_u32() % 6;
                let amp = rng.range(0, 30);
                let stride = rng.range(1, 2);
                let n = (cols * nch * stride) as usize + 2 * stride as usize;
                let time_in: Vec<T> = (0..n).map(|k| T::sample(&mut rng, kind, k, amp)).collect();
                let init: Vec<i32> = (0..SLOT * MAX_COLS).map(|_| rng.i32()).collect();
                let mut cre = init.clone();
                let mut cim: Vec<i32> = init.iter().map(|x| x ^ 0x5555).collect();
                let mut rre = cre.clone();
                let mut rim = cim.clone();
                let mut cw = vec![0i32; 2 * SLOT];
                let mut rw = vec![0i32; 2 * SLOT];
                let wf = format!("{what} frame {frame} kind {kind}");
                if rng.next_u32() % 3 == 0 {
                    // Slot by slot.
                    for s in 0..cols as usize {
                        let t = &time_in[s * (nch * stride) as usize..];
                        unsafe {
                            T::c_ana_slot(
                                c.0,
                                cre[s * SLOT..].as_mut_ptr(),
                                cim[s * SLOT..].as_mut_ptr(),
                                t.as_ptr(),
                                stride,
                                cw.as_mut_ptr(),
                            )
                        };
                        let rim_slot: &mut [i32] = if flags & QMF_FLAG_LP == 0 {
                            &mut rim[s * SLOT..(s + 1) * SLOT]
                        } else {
                            &mut []
                        };
                        qmf_analysis_filtering_slot(
                            &fb,
                            &mut rs,
                            &mut rre[s * SLOT..(s + 1) * SLOT],
                            rim_slot,
                            t,
                            stride,
                            &mut rw,
                        );
                    }
                } else {
                    let e = rng.range(-4, 4);
                    let mut csc = [rng.i32(), rng.i32(), rng.i32(), rng.i32()];
                    let mut rsc = QmfScaleFactor {
                        lb_scale: csc[0],
                        ov_lb_scale: csc[1],
                        hb_scale: csc[2],
                        ov_hb_scale: csc[3],
                    };
                    unsafe {
                        T::c_ana(
                            c.0,
                            cre.as_mut_ptr(),
                            cim.as_mut_ptr(),
                            csc.as_mut_ptr(),
                            time_in.as_ptr(),
                            e,
                            stride,
                            cw.as_mut_ptr(),
                        )
                    };
                    let mut re_slots: Vec<&mut [i32]> = rre.chunks_mut(SLOT).collect();
                    let mut im_slots: Vec<&mut [i32]> = rim.chunks_mut(SLOT).collect();
                    qmf_analysis_filtering(
                        &fb,
                        &mut rs,
                        &mut re_slots,
                        &mut im_slots,
                        &mut rsc,
                        &time_in,
                        e,
                        stride,
                        &mut rw,
                    );
                    assert_eq!(
                        [rsc.lb_scale, rsc.ov_lb_scale, rsc.hb_scale, rsc.ov_hb_scale],
                        csc,
                        "{wf}"
                    );
                }
                assert_eq!(rre, cre, "{wf}: real");
                assert_eq!(rim, cim, "{wf}: imag");
                assert_eq!(rs, cs, "{wf}: states");
                assert_eq!(rw, cw, "{wf}: work buffer");
                assert_eq!(dump(&fb), c.dump(), "{wf}");
            }
        }
    }
}

#[test]
fn qmf_analysis_int_pcm() {
    analysis::<i16>(0xa16, 40);
}

#[test]
fn qmf_analysis_long() {
    analysis::<i32>(0xa32, 40);
}

/// A random QMF value of the given kind (see [`signal_value`]).
fn qmf_value(rng: &mut Rng, kind: u32, n: usize, amp: i32) -> i32 {
    signal_value(rng, kind, n, 32, amp) as i32
}

fn synthesis<T: TestPcm>(seed: u64, frames: usize) {
    let mut rng = Rng::new(seed);
    for (nch, flags) in configs(true) {
        for run in 0..3 {
            let c = CQmf::new();
            let mut fb = QmfFilterBank::default();
            let mut cs: Vec<i32> = (0..9 * 64).map(|_| rng.i32() >> rng.range(0, 31)).collect();
            let mut rs = cs.clone();
            let cols = [1, 2, 15, 16, 30, 32, 64][(rng.next_u32() % 7) as usize];
            let lsb = rng.range(0, nch);
            let usb = rng.range(lsb, nch + 4);
            let what = format!("synthesis {} nch {nch} flags {flags:#x} run {run}", T::NAME);
            let init = |c: &CQmf, fb: &mut QmfFilterBank, cs: &mut Vec<i32>, rs: &mut Vec<i32>, f: u32| {
                let ec = unsafe {
                    cref_qmf_init_synthesis(c.0, cs.as_mut_ptr(), cols, lsb, usb, nch, f as i32)
                };
                let er = qmf_init_synthesis_filter_bank(fb, Some(&mut rs[..]), cols, lsb, usb, nch, f);
                assert_eq!(er, ec, "{what}");
                assert_eq!(dump(fb), c.dump(), "{what}");
                assert_eq!(*rs, *cs, "{what}");
            };
            init(&c, &mut fb, &mut cs, &mut rs, flags | if run == 2 { QMF_FLAG_KEEP_STATES } else { 0 });

            for frame in 0..frames {
                let wf = format!("{what} frame {frame}");
                // Changes of the configuration between frames.
                match rng.next_u32() % 8 {
                    0 => {
                        let sf = rng.range(-20, 20);
                        unsafe { cref_qmf_change_out_scalefactor(c.0, sf) };
                        qmf_change_out_scalefactor(&mut fb, &mut rs, sf);
                    }
                    1 => {
                        let m = if rng.next_u32() % 3 == 0 { i32::MIN } else { rng.i32() };
                        let e = rng.range(-4, 4);
                        unsafe { cref_qmf_change_out_gain(c.0, m, e) };
                        qmf_change_out_gain(&mut fb, m, e);
                    }
                    2 => {
                        // Re-initialization with KEEP_STATES (scales the states).
                        init(&c, &mut fb, &mut cs, &mut rs, flags | QMF_FLAG_KEEP_STATES);
                    }
                    3 => {
                        // lsb <= usb (other bands have no defined result in the reference).
                        let l = rng.range(0, nch);
                        let u = rng.range(l, nch);
                        unsafe { cref_qmf_set_bands(c.0, l, u) };
                        fb.lsb = l;
                        fb.usb = u;
                    }
                    _ => {}
                }
                assert_eq!(rs, cs, "{wf}: states after the change");
                assert_eq!(qmf_get_out_scalefactor(&fb), unsafe { cref_qmf_get_out_scalefactor(c.0) });

                let kind = rng.next_u32() % 6;
                let amp = rng.range(0, 30);
                let stride = rng.range(1, 2);
                let mut cre: Vec<i32> =
                    (0..SLOT * MAX_COLS).map(|k| qmf_value(&mut rng, kind, k, amp)).collect();
                let mut cim: Vec<i32> =
                    (0..SLOT * MAX_COLS).map(|k| qmf_value(&mut rng, kind, k + 7, amp)).collect();
                let rre = cre.clone();
                let rim = cim.clone();
                let n = (cols * nch * stride) as usize + 4;
                let mut cout = vec![T::default(); n];
                let mut rout = vec![T::default(); n];
                let mut cw = vec![0i32; 2 * SLOT];
                let mut rw = vec![0i32; 2 * SLOT];
                let sc_range = if rng.next_u32() % 4 == 0 { 45 } else { 12 };
                match rng.next_u32() % 3 {
                    0 => {
                        // Slot by slot.
                        for s in 0..cols as usize {
                            let sfl = rng.range(-sc_range, sc_range);
                            let sfh = rng.range(-sc_range, sc_range);
                            let off = s * (nch * stride) as usize;
                            unsafe {
                                T::c_syn_slot(
                                    c.0,
                                    cre[s * SLOT..].as_ptr(),
                                    cim[s * SLOT..].as_ptr(),
                                    sfl,
                                    sfh,
                                    cout[off..].as_mut_ptr(),
                                    stride,
                                    cw.as_mut_ptr(),
                                )
                            };
                            let rim_slot: &[i32] = if flags & QMF_FLAG_LP == 0 {
                                &rim[s * SLOT..(s + 1) * SLOT]
                            } else {
                                &[]
                            };
                            qmf_synthesis_filtering_slot(
                                &fb,
                                &mut rs,
                                &rre[s * SLOT..(s + 1) * SLOT],
                                rim_slot,
                                sfl,
                                sfh,
                                &mut rout[off..],
                                stride,
                                &mut rw,
                            );
                        }
                    }
                    1 => {
                        let sc = [
                            rng.range(-sc_range, sc_range),
                            rng.range(-sc_range, sc_range),
                            rng.range(-sc_range, sc_range),
                            rng.range(-sc_range, sc_range),
                        ];
                        let ov_len = rng.range(0, cols);
                        unsafe {
                            T::c_syn(
                                c.0,
                                cre.as_mut_ptr(),
                                cim.as_mut_ptr(),
                                sc.as_ptr(),
                                ov_len,
                                cout.as_mut_ptr(),
                                stride,
                                cw.as_mut_ptr(),
                            )
                        };
                        let re_slots: Vec<&[i32]> = rre.chunks(SLOT).collect();
                        let im_slots: Vec<&[i32]> = rim.chunks(SLOT).collect();
                        let rsc = QmfScaleFactor {
                            lb_scale: sc[0],
                            ov_lb_scale: sc[1],
                            hb_scale: sc[2],
                            ov_hb_scale: sc[3],
                        };
                        qmf_synthesis_filtering(
                            &fb,
                            &mut rs,
                            &re_slots,
                            &im_slots,
                            &rsc,
                            ov_len,
                            &mut rout,
                            stride,
                            &mut rw,
                        );
                    }
                    _ => {
                        // The prototype filter alone (symmetric filters only).
                        if flags & QMF_FLAG_NONSYMMETRIC == 0 && fb.flags & QMF_FLAG_NONSYMMETRIC == 0 {
                            for s in 0..cols as usize {
                                let off = s * (nch * stride) as usize;
                                unsafe {
                                    T::c_proto(
                                        c.0,
                                        cre[s * SLOT..].as_mut_ptr(),
                                        cim[s * SLOT..].as_mut_ptr(),
                                        cout[off..].as_mut_ptr(),
                                        stride,
                                    )
                                };
                                qmf_syn_prototype_fir_slot(
                                    &fb,
                                    &mut rs,
                                    &rre[s * SLOT..(s + 1) * SLOT],
                                    &rim[s * SLOT..(s + 1) * SLOT],
                                    &mut rout[off..],
                                    stride,
                                );
                            }
                        }
                    }
                }
                assert_eq!(rout, cout, "{wf}: output");
                assert_eq!(rs, cs, "{wf}: states");
                assert_eq!(rw, cw, "{wf}: work buffer");
                assert_eq!(dump(&fb), c.dump(), "{wf}");
            }
        }
    }
}

#[test]
fn qmf_synthesis_int_pcm() {
    synthesis::<i16>(0x516, 40);
}

#[test]
fn qmf_synthesis_long() {
    synthesis::<i32>(0x532, 40);
}

/// Analysis followed by synthesis (the SBR decoder's chain) over many frames of a structured
/// signal, comparing the time signal of both sides.
#[test]
fn qmf_analysis_synthesis_chain() {
    let mut rng = Rng::new(0xc4a);
    for (nch, flags) in [
        (32, 0),
        (32, QMF_FLAG_LP),
        (32, QMF_FLAG_DOWNSAMPLED),
        (64, 0),
        (64, QMF_FLAG_LP),
        (64, QMF_FLAG_CLDFB),
        (32, QMF_FLAG_CLDFB),
        (16, QMF_FLAG_CLDFB),
        (16, 0),
    ] {
        let ca = CQmf::new();
        let cs_ = CQmf::new();
        let mut fa = QmfFilterBank::default();
        let mut fs = QmfFilterBank::default();
        let cols = 32;
        let mut ca_st = vec![0i32; 640];
        let mut ra_st = ca_st.clone();
        let mut cs_st = vec![0i32; 576];
        let mut rs_st = cs_st.clone();
        unsafe { cref_qmf_init_analysis_32(ca.0, ca_st.as_mut_ptr(), cols, nch, nch, nch, flags as i32) };
        qmf_init_analysis_filter_bank(&mut fa, Some(&mut ra_st[..]), cols, nch, nch, nch, flags);
        unsafe { cref_qmf_init_synthesis(cs_.0, cs_st.as_mut_ptr(), cols, nch, nch, nch, flags as i32) };
        qmf_init_synthesis_filter_bank(&mut fs, Some(&mut rs_st[..]), cols, nch, nch, nch, flags);
        for frame in 0..40 {
            let kind = [3, 0, 2, 5, 4, 1][frame % 6];
            let time_in: Vec<i32> = (0..(cols * nch) as usize)
                .map(|k| qmf_value(&mut rng, kind, k + frame * 1000, 3) >> 3)
                .collect();
            let mut cre = vec![0i32; SLOT * MAX_COLS];
            let mut cim = vec![0i32; SLOT * MAX_COLS];
            let mut rre = cre.clone();
            let mut rim = cim.clone();
            let mut csc = [0i32; 4];
            let mut rsc = QmfScaleFactor::default();
            let mut cw = vec![0i32; 2 * SLOT];
            let mut rw = vec![0i32; 2 * SLOT];
            unsafe {
                cref_qmf_analysis_32(ca.0, cre.as_mut_ptr(), cim.as_mut_ptr(), csc.as_mut_ptr(), time_in.as_ptr(), 0, 1, cw.as_mut_ptr())
            };
            {
                let mut re_slots: Vec<&mut [i32]> = rre.chunks_mut(SLOT).collect();
                let mut im_slots: Vec<&mut [i32]> = rim.chunks_mut(SLOT).collect();
                qmf_analysis_filtering(&fa, &mut ra_st, &mut re_slots, &mut im_slots, &mut rsc, &time_in, 0, 1, &mut rw);
            }
            assert_eq!(rre, cre);
            assert_eq!(rim, cim);
            csc[1] = csc[0];
            csc[2] = csc[0];
            rsc.ov_lb_scale = rsc.lb_scale;
            rsc.hb_scale = rsc.lb_scale;
            let mut cout = vec![0i16; (cols * nch) as usize];
            let mut rout = cout.clone();
            unsafe {
                cref_qmf_synthesis_16(cs_.0, cre.as_mut_ptr(), cim.as_mut_ptr(), csc.as_ptr(), 0, cout.as_mut_ptr(), 1, cw.as_mut_ptr())
            };
            let re_slots: Vec<&[i32]> = rre.chunks(SLOT).collect();
            let im_slots: Vec<&[i32]> = rim.chunks(SLOT).collect();
            qmf_synthesis_filtering(&fs, &mut rs_st, &re_slots, &im_slots, &rsc, 0, &mut rout, 1, &mut rw);
            assert_eq!(rout, cout, "nch {nch} flags {flags:#x} frame {frame}");
            assert_eq!(rs_st, cs_st);
            assert_eq!(ra_st, ca_st);
        }
    }
}


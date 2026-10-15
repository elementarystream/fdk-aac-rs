// Differential tests of fdk_aac_rs_fdk::decorrelate against the C++ reference.
//
// A Rust decorrelator and a C++ one (behind an opaque handle) run the same sequences of
// FDKdecorrelateOpen/Init/Apply/Close calls: the PS configurations of libSBRdec and the USAC and
// LD configurations of libSACdec, re-initializations with and without clearing the states (which
// then keep the buffer content), invalid parameters, explicit ducker types, in-place and
// separate output buffers, a known direct signal maximum. After every call the return value, the
// outputs, the complete state (see `dump()`) and the complete buffer are compared.

use fdk_aac_rs_cref::fdk_decorrelate as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::decorrelate::*;
use std::ffi::{c_void, CString};

/// Length of the data arrays (71 hybrid bands plus one).
const N: usize = 72;

const ABSENT: i32 = -100;

/// The C++ decorrelator.
struct CDec(*mut c_void, usize);

impl CDec {
    fn new(buf_len: usize) -> Self {
        CDec(unsafe { c::cref_decorrelate_new(buf_len as i32) }, buf_len)
    }
    fn state(&self) -> Vec<i32> {
        let mut out = vec![0i32; 4096];
        let n = unsafe { c::cref_decorrelate_state(self.0, out.as_mut_ptr()) };
        out.truncate(n as usize);
        out
    }
    fn buffer(&self) -> Vec<i32> {
        let mut out = vec![0i32; self.1];
        unsafe { c::cref_decorrelate_get_buffer(self.0, out.as_mut_ptr(), self.1 as i32) };
        out
    }
}

impl Drop for CDec {
    fn drop(&mut self) {
        unsafe { c::cref_decorrelate_free(self.0) };
    }
}

/// The state of the Rust decorrelator in the layout of `cref_decorrelate_state()`.
fn dump(d: &DecorrDec) -> Vec<i32> {
    let mut o = vec![
        d.l_state_buffer_cplx,
        if d.state_buffer_cplx.is_some() { 0 } else { -1 },
        d.l_delay_buffer_cplx,
        d.delay_buffer_cplx.map_or(-1, |v| v as i32),
    ];
    let four = |o: &mut Vec<i32>, v: Option<[i32; 4]>| o.extend(v.unwrap_or([ABSENT; 4]));
    four(&mut o, d.rev_filt_type.copied());
    four(&mut o, d.rev_band_offset.map(|t| t.map(|v| v as i32)));
    four(&mut o, d.rev_delay.map(|t| t.map(|v| v as i32)));
    four(&mut o, d.rev_filter_order.map(|t| t.map(|v| v as i32)));
    o.extend(d.reverb_band_delay_buffer_index);
    o.extend(d.state_buffer_offset.map(|v| v as i32));
    o.push(d.numbins);
    o.push(d.partially_complex);
    let k = &d.ducker;
    o.extend([k.hybrid_bands, k.parameter_bands, k.partially_complex, k.ducker_type]);
    o.push(match (k.qs_next, k.map_proc_bands2_hyb_bands) {
        (Some(q), Some(m)) => (q.as_ptr() as isize - m.as_ptr() as isize) as i32,
        _ => -1,
    });
    for i in 0..21 {
        o.push(k.map_proc_bands2_hyb_bands.map_or(ABSENT, |t| t[i] as i32));
    }
    for i in 0..65 {
        o.push(k.map_hyb_bands2_proc_bands.map_or(ABSENT, |t| t[i] as i32));
    }
    o.extend(k.smooth_dir_rev_nrg);
    o.extend(k.peak_decay);
    o.extend(k.peak_diff);
    o.extend([k.max_val_direct_data, k.max_val_reverb_data]);
    o.extend([
        k.scale_direct_nrg as i32,
        k.scale_reverb_nrg as i32,
        k.scale_smooth_dir_rev_nrg as i32,
        k.headroom_smooth_dir_rev_nrg as i32,
    ]);
    for f in &d.filter {
        o.push(f.state_cplx.map_or(-1, |v| v as i32));
        o.push(f.delay_buffer_cplx.map_or(-1, |v| v as i32));
        for j in 0..3 {
            o.push(f.numerator_real.map_or(ABSENT, |t| t[j] as i32));
        }
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[0].re as i32));
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[0].im as i32));
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[3].re as i32));
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[3].im as i32));
        o.push(f.denominator_real.is_some() as i32);
    }
    o
}

/// A pair of decorrelators.
struct Pair {
    rs: DecorrDec,
    c: CDec,
    open: bool,
}

impl Pair {
    fn new(buf_len: usize) -> Self {
        Pair { rs: DecorrDec::default(), c: CDec::new(buf_len), open: false }
    }

    fn check(&self, what: &str) {
        assert_eq!(dump(&self.rs), self.c.state(), "state after {what}");
        if let Some(b) = &self.rs.state_buffer_cplx {
            assert_eq!(&b[..], &self.c.buffer()[..], "buffer after {what}");
        }
    }

    fn open(&mut self, rng: &mut Rng) -> i32 {
        // the content of the caller's buffer is kept (C++: calloc'ed memory)
        let buf = vec![0i32; self.c.1].into_boxed_slice();
        let r = fdk_decorrelate_open(&mut self.rs, buf);
        let e = unsafe { c::cref_decorrelate_open(self.c.0) };
        assert_eq!(r, e, "FDKdecorrelateOpen");
        self.open = r == 0;
        let _ = rng;
        self.check("FDKdecorrelateOpen");
        r
    }

    fn init(&mut self, p: [i32; 9]) -> i32 {
        let [nr, ty, du, cfg, seed, pc, fd, legacy, init] = p;
        let r = fdk_decorrelate_init(&mut self.rs, nr, ty, du, cfg, seed, pc, fd, legacy, init);
        let e = unsafe { c::cref_decorrelate_init(self.c.0, nr, ty, du, cfg, seed, pc, fd, legacy, init) };
        assert_eq!(r, e, "FDKdecorrelateInit{p:?}");
        self.check(&format!("FDKdecorrelateInit{p:?}"));
        r
    }

    fn apply(&mut self, rng: &mut Rng, frame: &Frame, start: i32, in_place: bool, what: &str) {
        let (mut out_re, mut out_im) = (frame.out_re, frame.out_im);
        let (mut c_out_re, mut c_out_im) = (frame.out_re, frame.out_im);
        let (r, e);
        if in_place {
            let (mut re, mut im) = (frame.re, frame.im);
            r = fdk_decorrelate_apply_in_place(&mut self.rs, &mut re, &mut im, start);
            out_re = re;
            out_im = im;
            let (mut cre, mut cim) = (frame.re, frame.im);
            e = unsafe {
                c::cref_decorrelate_apply(
                    self.c.0,
                    cre.as_mut_ptr(),
                    cim.as_mut_ptr(),
                    cre.as_mut_ptr(),
                    cim.as_mut_ptr(),
                    start,
                )
            };
            c_out_re = cre;
            c_out_im = cim;
        } else {
            r = fdk_decorrelate_apply(&mut self.rs, &frame.re, &frame.im, &mut out_re, &mut out_im, start);
            let (mut re, mut im) = (frame.re, frame.im);
            e = unsafe {
                c::cref_decorrelate_apply(
                    self.c.0,
                    re.as_mut_ptr(),
                    im.as_mut_ptr(),
                    c_out_re.as_mut_ptr(),
                    c_out_im.as_mut_ptr(),
                    start,
                )
            };
        }
        let what = format!("{what}, start {start}, in place {in_place}");
        assert_eq!(r, e, "FDKdecorrelateApply, {what}");
        assert_eq!(out_re, c_out_re, "real output, {what}");
        assert_eq!(out_im, c_out_im, "imaginary output, {what}");
        self.check(&format!("FDKdecorrelateApply, {what}"));
        let _ = rng;
    }

    fn set_max_val(&mut self, v: i32) {
        *get_addr_direct_signal_max_val(&mut self.rs) = v;
        unsafe { c::cref_decorrelate_set_max_val(self.c.0, v) };
    }
}

/// One slot of hybrid input data and the previous content of the output buffers.
struct Frame {
    re: [i32; N],
    im: [i32; N],
    out_re: [i32; N],
    out_im: [i32; N],
}

/// Random hybrid data. `edge` allows extreme values (MINVAL_DBL, ...), which make the reference
/// call sqrtFixp()/invSqrtNorm2() with negative values (FDK_ASSERTs, debug_assert!s in the port).
fn frame(rng: &mut Rng, edge: bool) -> Frame {
    let level = match rng.next_u32() % 8 {
        0 => 31, // silence
        1 => 1,
        _ => rng.range(2, 14) as u32,
    };
    let v = |rng: &mut Rng| -> i32 {
        if edge && rng.next_u32() % 16 == 0 {
            return [i32::MIN, i32::MAX, -i32::MAX, 0][(rng.next_u32() % 4) as usize];
        }
        if level == 31 {
            0
        } else {
            (rng.next_u32() as i32) >> (level + rng.next_u32() % 3)
        }
    };
    let mut f = Frame { re: [0; N], im: [0; N], out_re: [0; N], out_im: [0; N] };
    for i in 0..N {
        f.re[i] = v(rng);
        f.im[i] = v(rng);
        f.out_re[i] = rng.next_u32() as i32;
        f.out_im[i] = rng.next_u32() as i32;
    }
    // a band region of silence
    if rng.next_u32() % 4 == 0 {
        let a = rng.range(0, N as i32 - 1) as usize;
        let b = rng.range(a as i32, N as i32) as usize;
        f.re[a..b].fill(0);
        f.im[a..b].fill(0);
    }
    f
}

/// Init parameters of the library: [nrHybBands, decorrType, duckerType, decorrConfig, seed,
/// partiallyComplex, useFractDelay, isLegacyPS, initStatesFlag].
fn valid_config(rng: &mut Rng) -> [i32; 9] {
    match rng.next_u32() % 3 {
        0 => [71, DECORR_PS, DUCKER_AUTOMATIC, 0, 0, rng.range(0, 1), 0, rng.range(0, 1), 1],
        1 => {
            let nr = [71, 71, 64, 40][(rng.next_u32() % 4) as usize];
            [nr, DECORR_USAC, DUCKER_AUTOMATIC, rng.range(0, 2), 0, 0, 0, 0, 1]
        }
        _ => {
            let nr = [64, 32][(rng.next_u32() % 2) as usize];
            [nr, DECORR_LD, DUCKER_AUTOMATIC, rng.range(0, 2), rng.range(0, 3), 0, 0, 0, 1]
        }
    }
}

/// Random (mostly invalid) init parameters for which the reference has a defined result.
fn any_config(rng: &mut Rng) -> [i32; 9] {
    let mut p = valid_config(rng);
    match rng.next_u32() % 8 {
        0 => p[1] = [DECORR_MPS, 7, -1][(rng.next_u32() % 3) as usize],
        1 => p[5] = 1,                         // partiallyComplex
        2 => p[6] = 1,                         // useFractDelay
        3 => p[4] = rng.range(0, 6),           // seed
        4 => p[0] = rng.range(16, 71),         // nrHybBands (LD only accepts 32 and 64)
        5 => p[2] = rng.range(0, 2),           // explicit ducker type
        6 => p[3] = if p[1] == DECORR_USAC { rng.range(0, 2) } else { rng.range(0, 5) },
        _ => p[8] = 0,                         // keep the states
    }
    // A PS ducker with 20 parameter bands needs 71 hybrid bands (FDK_ASSERT).
    if p[1] == DECORR_PS && p[7] != 0 {
        p[0] = 71;
    }
    p
}

/// Hybrid start band of a configuration: 0 for PS, any value up to nrHybBands otherwise.
fn start_band(rng: &mut Rng, p: &[i32; 9]) -> i32 {
    if p[1] == DECORR_PS || rng.next_u32() % 3 == 0 {
        0
    } else {
        rng.range(0, p[0])
    }
}

/// The start band of the next frame. The library keeps it constant for a configuration
/// (`param2hyb[residualBands[]]` of libSACdec). A smaller start band than in the previous frame
/// rescales smoothed energies of bands that the headroom computation did not cover; their
/// overflow makes the reference call sqrtFixp()/invSqrtNorm2() with negative values
/// (FDK_ASSERTs, debug_assert!s in the port), so it is only tested without debug assertions.
fn next_start_band(rng: &mut Rng, p: &[i32; 9], current: i32) -> i32 {
    if !cfg!(debug_assertions) && rng.next_u32() % 4 == 0 {
        start_band(rng, p)
    } else {
        current
    }
}

fn session(seed: u64, frames: usize, reconfigure: bool) {
    let mut rng = Rng::new(seed);
    let edge = !cfg!(debug_assertions);
    let mut p = Pair::new(DECORR_BUFFER_LEN as usize + (rng.next_u32() % 4) as usize);
    assert_eq!(p.open(&mut rng), 0);
    let mut cfg = valid_config(&mut rng);
    assert_eq!(p.init(cfg), 0, "{cfg:?}");
    let mut valid = cfg;
    let mut start = start_band(&mut rng, &valid);
    for f in 0..frames {
        if reconfigure && rng.next_u32() % 16 == 0 {
            cfg = if rng.next_u32() % 2 == 0 { any_config(&mut rng) } else { valid_config(&mut rng) };
            if rng.next_u32() % 2 == 0 {
                cfg[8] = 0;
            }
            if p.init(cfg) == 0 {
                valid = cfg;
            } else {
                // A failed init can leave a partial configuration; continue with a valid one.
                valid = valid_config(&mut rng);
                valid[8] = (rng.next_u32() % 2) as i32;
                assert_eq!(p.init(valid), 0, "{valid:?}");
            }
            start = start_band(&mut rng, &valid);
        }
        if rng.next_u32() % 32 == 0 {
            let v = match rng.next_u32() % 3 {
                0 => i32::MIN, // FL2FXCONST_DBL(-1.0f): not known
                1 => rng.next_u32() as i32 >> rng.range(0, 30),
                _ => rng.i32(),
            };
            p.set_max_val(v);
        }
        let e = edge && rng.next_u32() % 8 == 0;
        let fr = frame(&mut rng, e);
        start = next_start_band(&mut rng, &valid, start);
        let in_place = rng.next_u32() % 2 == 0;
        p.apply(&mut rng, &fr, start, in_place, &format!("seed {seed} frame {f} config {valid:?}"));
    }
    let r = fdk_decorrelate_close(&mut p.rs);
    let e = unsafe { c::cref_decorrelate_close(p.c.0) };
    assert_eq!(r, e, "FDKdecorrelateClose");
    p.check("FDKdecorrelateClose");
}

#[test]
fn tables_match() {
    let mut out = vec![0i32; 1024];
    let mut cmp = |name: &str, rs: Vec<i32>| {
        let cname = CString::new(name).unwrap();
        let n = unsafe { c::cref_decorrelate_table(cname.as_ptr(), out.as_mut_ptr()) };
        assert!(n >= 0, "{name} not found in the shim");
        assert_eq!(rs, &out[..n as usize], "{name}");
    };
    let u = |t: &[u8]| t.iter().map(|&v| v as i32).collect::<Vec<_>>();
    let s = |t: &[i8]| t.iter().map(|&v| v as i32).collect::<Vec<_>>();
    let h = |t: &[i16]| t.iter().map(|&v| v as i32).collect::<Vec<_>>();
    cmp("REV_bandOffset_MPS_HQ", u(REV_BAND_OFFSET_MPS_HQ.as_flattened()));
    cmp("REV_bandOffset_PS_HQ", u(&REV_BAND_OFFSET_PS_HQ));
    cmp("REV_bandOffset_PS_LP", u(&REV_BAND_OFFSET_PS_LP));
    cmp("REV_bandOffset_LD", u(REV_BAND_OFFSET_LD.as_flattened()));
    cmp("REV_delay_MPS", u(&REV_DELAY_MPS));
    cmp("REV_filterOrder_MPS", s(&REV_FILTER_ORDER_MPS));
    cmp("REV_delay_PS_HQ", u(&REV_DELAY_PS_HQ));
    cmp("REV_delay_PS_LP", u(&REV_DELAY_PS_LP));
    cmp("REV_filterOrder_PS", s(&REV_FILTER_ORDER_PS));
    cmp("REV_delay_USAC", u(&REV_DELAY_USAC));
    cmp("REV_filterOrder_USAC", s(&REV_FILTER_ORDER_USAC));
    cmp("REV_filtType_MPS", REV_FILT_TYPE_MPS.to_vec());
    cmp("REV_filtType_PS", REV_FILT_TYPE_PS.to_vec());
    cmp("stateBufferOffsetInit", u(&STATE_BUFFER_OFFSET_INIT));
    cmp("REV_filtType_LD", REV_FILT_TYPE_LD.to_vec());
    cmp("kernels_20_to_71_PS", u(&KERNELS_20_TO_71_PS));
    cmp("kernels_20_to_71_offset_PS", u(&KERNELS_20_TO_71_OFFSET_PS));
    cmp("kernels_28_to_71", u(&KERNELS_28_TO_71));
    cmp("kernels_28_to_71_offset", u(&KERNELS_28_TO_71_OFFSET));
    cmp("kernels_23_to_64", u(&KERNELS_23_TO_64));
    cmp("kernels_23_to_64_offset", u(&KERNELS_23_TO_64_OFFSET));
    cmp("DecorrNumeratorReal0_USAC", h(DECORR_NUMERATOR_REAL0_USAC.as_flattened()));
    cmp("DecorrNumeratorReal1_USAC", h(DECORR_NUMERATOR_REAL1_USAC.as_flattened()));
    cmp("DecorrNumeratorReal2_USAC", h(DECORR_NUMERATOR_REAL2_USAC.as_flattened()));
    cmp("DecorrNumeratorReal3_USAC", h(DECORR_NUMERATOR_REAL3_USAC.as_flattened()));
    cmp("DecorrNumeratorReal1_LD", h(DECORR_NUMERATOR_REAL1_LD.as_flattened()));
    cmp("DecorrNumeratorReal2_LD", h(DECORR_NUMERATOR_REAL2_LD.as_flattened()));
    cmp("DecorrNumeratorReal3_LD", h(DECORR_NUMERATOR_REAL3_LD.as_flattened()));
    let ps: Vec<i32> = DECORR_PS_COEFFS_CPLX
        .as_flattened()
        .iter()
        .flat_map(|c| [c.re as i32, c.im as i32])
        .collect();
    cmp("DecorrPsCoeffsCplx", ps);
}

#[test]
fn open_buffer_length() {
    let mut rng = Rng::new(40);
    for len in [0usize, 1, DECORR_BUFFER_LEN as usize - 1, DECORR_BUFFER_LEN as usize, 3000] {
        let mut p = Pair::new(len);
        p.open(&mut rng);
    }
}

#[test]
fn ps_sessions() {
    for seed in 0..12 {
        let mut rng = Rng::new(100 + seed);
        let edge = !cfg!(debug_assertions);
        let mut p = Pair::new(DECORR_BUFFER_LEN as usize);
        assert_eq!(p.open(&mut rng), 0);
        let cfg = [71, DECORR_PS, DUCKER_AUTOMATIC, 0, 0, (seed % 2) as i32, 0, (seed / 2 % 2) as i32, 1];
        assert_eq!(p.init(cfg), 0);
        for f in 0..300 {
            let fr = frame(&mut rng, edge && f % 16 == 7);
            p.apply(&mut rng, &fr, 0, f % 3 == 0, &format!("PS seed {seed} frame {f}"));
        }
    }
}

#[test]
fn usac_ld_sessions() {
    for seed in 0..24u64 {
        let mut rng = Rng::new(200 + seed);
        let edge = !cfg!(debug_assertions);
        let mut p = Pair::new(DECORR_BUFFER_LEN as usize);
        assert_eq!(p.open(&mut rng), 0);
        let cfg = if seed < 12 {
            [[71, 64, 40][(seed % 3) as usize], DECORR_USAC, DUCKER_AUTOMATIC, (seed / 3 % 3) as i32, 0, 0, 0, 0, 1]
        } else {
            [[64, 32][(seed % 2) as usize], DECORR_LD, DUCKER_AUTOMATIC, (seed / 2 % 3) as i32, (seed % 4) as i32, 0, 0, 0, 1]
        };
        assert_eq!(p.init(cfg), 0, "{cfg:?}");
        let mut start = start_band(&mut rng, &cfg);
        for f in 0..300 {
            let fr = frame(&mut rng, edge && f % 16 == 7);
            start = next_start_band(&mut rng, &cfg, start);
            p.apply(&mut rng, &fr, start, f % 3 == 0, &format!("{cfg:?} frame {f}"));
        }
    }
}

#[test]
fn reconfiguration_sessions() {
    for seed in 0..40 {
        session(300 + seed, 400, true);
    }
}

#[test]
fn apply_pass_without_imaginary_part() {
    let mut rng = Rng::new(41);
    for _ in 0..2000 {
        let delay = rng.range(1, 14);
        let start = rng.range(0, 70);
        let stop = rng.range(start, 71);
        let index = 2 * rng.range(0, delay - 1);
        let len = (2 * delay * (stop - start + 1) + 2 * delay + 4) as usize;
        let mut buf_r: Vec<i32> = (0..len).map(|_| rng.next_u32() as i32).collect();
        let mut buf_c = buf_r.clone();
        let inp: Vec<i32> = (0..N).map(|_| rng.next_u32() as i32).collect();
        let mut out_r: Vec<i32> = (0..N).map(|_| rng.next_u32() as i32).collect();
        let mut out_c = out_r.clone();
        let mut out_im = vec![0i32; N];
        let offset = rng.range(0, 3) as usize;
        let mut filter = [DecorrFilterInstance::default(); 71];
        filter[start as usize].delay_buffer_cplx = Some(offset);
        let r = decorr_filter_apply_pass(
            &filter, &mut buf_r, &inp, None, &mut out_r, &mut out_im, start, stop, delay, index,
        );
        let mut inp_c = inp.clone();
        let e = unsafe {
            c::cref_decorrelate_apply_pass_real(
                buf_c.as_mut_ptr(),
                offset as i32,
                inp_c.as_mut_ptr(),
                out_c.as_mut_ptr(),
                start,
                stop,
                delay,
                index,
            )
        };
        assert_eq!(r, e);
        assert_eq!(out_r, out_c, "start {start} stop {stop} delay {delay} index {index}");
        assert_eq!(buf_r, buf_c, "start {start} stop {stop} delay {delay} index {index}");
    }
}

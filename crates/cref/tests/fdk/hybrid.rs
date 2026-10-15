// Differential tests of fdk_aac_rs_fdk::hybrid against the C++ reference.
//
// The analysis filter bank is run on random configurations (all modes, QMF and complex band
// counts, memory sizes that fail the checks, with and without HF memory) over many slots of
// random, full-scale, silent and structured QMF data, with re-initializations that keep the
// states, state scaling and the HF mode. Outputs, return codes, all fields and the complete LF
// and HF memory are compared after every call. The library's FDK_hybrid.cpp is called, so the
// fft_8 inlined there is covered.

use core::ffi::c_void;
use fdk_aac_rs_cref::fdk_hybrid::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::hybrid::*;

const NULL_OFFSET: i32 = -100000;

struct CAna(*mut c_void);
impl Drop for CAna {
    fn drop(&mut self) {
        unsafe { cref_hybrid_ana_free(self.0) }
    }
}
impl CAna {
    fn state(&self) -> [i32; 46] {
        let mut out = [0i32; 46];
        unsafe { cref_hybrid_ana_state(self.0, out.as_mut_ptr()) };
        out
    }
}

struct CSyn(*mut c_void);
impl Drop for CSyn {
    fn drop(&mut self) {
        unsafe { cref_hybrid_syn_free(self.0) }
    }
}

/// The state of the port in the layout of `cref_hybrid_ana_state`, with the offsets of `NULL`
/// pointers of the reference (`NULL_OFFSET`) mapped to the port's 0.
fn ana_state(f: &FdkAnaHybFilter, c: &[i32; 46]) -> [i32; 46] {
    let mut out = [0i32; 46];
    let mut o = 0;
    let off = |v: isize, c: i32| if c == NULL_OFFSET && v == 0 { NULL_OFFSET } else { v as i32 };
    for k in 0..3 {
        out[o] = off(f.buffer_lf_real[k], c[o]);
        o += 1;
    }
    for k in 0..3 {
        out[o] = off(f.buffer_lf_imag[k], c[o]);
        o += 1;
    }
    for k in 0..13 {
        out[o] = off(f.buffer_hf_real[k], c[o]);
        o += 1;
    }
    for k in 0..13 {
        out[o] = off(f.buffer_hf_imag[k], c[o]);
        o += 1;
    }
    out[o] = f.buffer_lf_pos;
    out[o + 1] = f.buffer_hf_pos;
    out[o + 2] = f.nr_bands;
    out[o + 3] = f.cplx_bands;
    out[o + 4] = f.hf_mode as i32;
    out[o + 5] = !f.p_lf_memory.is_empty() as i32;
    out[o + 6] = !f.p_hf_memory.is_empty() as i32;
    out[o + 7] = f.lf_memory_size as i32;
    out[o + 8] = f.hf_memory_size as i32;
    out[o + 9] = f.p_setup.is_some() as i32;
    out
}

/// A QMF or hybrid value of a test signal: 0 random full scale, 1 silence, 2 random with less
/// amplitude, 3 sine, 4 alternating extremes, 5 sparse impulses.
fn value(rng: &mut Rng, kind: u32, n: usize, amp: i32) -> i32 {
    match kind {
        0 => rng.next_u32() as i32,
        1 => 0,
        2 => (rng.next_u32() as i32) >> (amp as u32 % 31),
        3 => {
            let a = (i32::MAX >> (amp as u32 % 8)) as f64;
            (a * ((n as f64) * 0.07 * (1.0 + amp as f64 * 0.31)).sin()) as i32
        }
        4 => {
            if n % 2 == 0 {
                i32::MAX
            } else {
                i32::MIN
            }
        }
        _ => {
            if rng.next_u32() % 17 == 0 {
                rng.i32()
            } else {
                0
            }
        }
    }
}

#[test]
fn hybrid_constants() {
    let mut c = [0i32; 5];
    unsafe { cref_hybrid_consts(c.as_mut_ptr()) };
    assert_eq!([HYB_C0, HYB_CM7, HYB_CP7, HYB_CM1, HYB_CP1], c);
}

#[test]
fn hybrid_setups() {
    // The setups as the library's filter bank points to them.
    for (mode, setup) in [(THREE_TO_TEN, &SETUP_3_10), (THREE_TO_TWELVE, &SETUP_3_12), (THREE_TO_SIXTEEN, &SETUP_3_16)] {
        let c = CAna(unsafe { cref_hybrid_ana_new(78, 0) });
        unsafe {
            cref_hybrid_ana_open(c.0, 78 * 4, 0, 0);
            assert_eq!(cref_hybrid_ana_init(c.0, mode, 3, 3, 1), 0);
        }
        let mut out = [0i32; 14];
        let mut idx = [0i32; 26];
        assert_eq!(unsafe { cref_hybrid_ana_setup(c.0, out.as_mut_ptr(), idx.as_mut_ptr()) }, 1);
        let mut exp = vec![setup.nr_qmf_bands as i32];
        exp.extend(setup.n_hyb_bands.iter().map(|&x| x as i32));
        exp.extend(setup.syn_hyb_scale.iter().map(|&x| x as i32));
        exp.extend(setup.k_hybrid.iter().map(|&x| x as i32));
        exp.push(setup.proto_len as i32);
        exp.push(setup.filter_delay as i32);
        assert_eq!(&out[..12], &exp[..], "mode {mode}");
        assert_eq!(&idx[..], setup.p_read_idx_table, "mode {mode}");
    }
}

#[test]
fn hybrid_analysis() {
    let mut rng = Rng::new(0x4b1d);
    let modes = [THREE_TO_TEN, THREE_TO_TWELVE, THREE_TO_SIXTEEN];
    for run in 0..400 {
        let mode = if rng.next_u32() % 25 == 0 { [3, -1][run % 2] } else { modes[run % 3] };
        let use_hf = rng.next_u32() % 5 != 0;
        let qmf_bands = if use_hf { rng.range(3, 64) } else { 3 };
        let cplx_bands = if rng.next_u32() % 3 == 0 { qmf_bands } else { rng.range(3, 64) };
        let mut lf_len = 2 * 3 * 13;
        let mut hf_len = if use_hf { 6 * ((qmf_bands - 3) + (cplx_bands - 3)) } else { 0 };
        // Memory larger than needed, or too small (Init fails).
        match rng.next_u32() % 8 {
            0 => lf_len -= rng.range(1, 10),
            1 if use_hf => hf_len -= rng.range(1, 6),
            2 => {
                lf_len += 20;
                hf_len += 60;
            }
            _ => {}
        }
        if use_hf && hf_len <= 0 {
            hf_len = 1;
        }
        let lf_bytes = (lf_len * 4) as u32;
        let hf_bytes = if use_hf { (hf_len * 4) as u32 } else { 0 };
        let what = format!("run {run} mode {mode} q {qmf_bands} c {cplx_bands} lf {lf_len} hf {hf_len} use_hf {use_hf}");

        let c = CAna(unsafe { cref_hybrid_ana_new(lf_len, hf_len.max(1)) });
        let lf: Vec<i32> = (0..lf_len).map(|_| rng.i32()).collect();
        let hf: Vec<i32> = (0..hf_len.max(1)).map(|_| rng.i32()).collect();
        unsafe { cref_hybrid_ana_set_mem(c.0, lf.as_ptr(), hf.as_ptr()) };
        let mut f = FdkAnaHybFilter::default();
        let ec = unsafe { cref_hybrid_ana_open(c.0, lf_bytes, use_hf as i32, hf_bytes) };
        let er = fdk_hybrid_analysis_open(
            &mut f,
            lf.clone(),
            lf_bytes,
            if use_hf { hf[..hf_len as usize].to_vec() } else { Vec::new() },
            hf_bytes,
        );
        assert_eq!(er, ec);

        let compare = |f: &FdkAnaHybFilter, c: &CAna, w: &str| {
            let cs = c.state();
            assert_eq!(ana_state(f, &cs), cs, "{w}: state");
            let mut clf = vec![0i32; lf_len as usize];
            let mut chf = vec![0i32; hf_len.max(1) as usize];
            unsafe { cref_hybrid_ana_get_mem(c.0, clf.as_mut_ptr(), chf.as_mut_ptr()) };
            assert_eq!(f.p_lf_memory, clf, "{w}: LF memory");
            if use_hf {
                assert_eq!(f.p_hf_memory, chf[..hf_len as usize], "{w}: HF memory");
            }
        };

        let init_flag = (rng.next_u32() % 4 != 0) as i32;
        let ec = unsafe { cref_hybrid_ana_init(c.0, mode, qmf_bands, cplx_bands, init_flag) };
        let er = fdk_hybrid_analysis_init(&mut f, mode, qmf_bands, cplx_bands, init_flag);
        assert_eq!(er, ec, "{what}: init");
        compare(&f, &c, &what);
        if er != 0 {
            // No defined result in the reference after a failed init.
            continue;
        }

        let mut nr = qmf_bands;
        let mut nc = cplx_bands;
        for slot in 0..200 {
            let w = format!("{what} slot {slot}");
            match rng.next_u32() % 16 {
                0 => {
                    let sv = if rng.next_u32() % 4 == 0 { rng.range(-40, 40) } else { rng.range(-3, 3) };
                    let ec = unsafe { cref_hybrid_ana_scale_states(c.0, sv) };
                    let er = fdk_hybrid_analysis_scale_states(Some(&mut f), sv);
                    assert_eq!(er, ec, "{w}: scale states");
                }
                1 => {
                    // Re-initialization that keeps the states (as the MPS decoder does), with band
                    // counts that fit into the memory.
                    let q2 = if use_hf { rng.range(3, qmf_bands) } else { 3 };
                    let c2 = rng.range(3, cplx_bands);
                    let flag = (rng.next_u32() % 3 == 0) as i32;
                    let ec = unsafe { cref_hybrid_ana_init(c.0, mode, q2, c2, flag) };
                    let er = fdk_hybrid_analysis_init(&mut f, mode, q2, c2, flag);
                    assert_eq!(er, ec, "{w}: re-init");
                    nr = q2;
                    nc = c2;
                }
                2 => {
                    let m = rng.range(0, 1);
                    unsafe { cref_hybrid_ana_set_hf_mode(c.0, m) };
                    f.hf_mode = m as u8;
                }
                _ => {}
            }
            compare(&f, &c, &w);
            let _ = (nr, nc);

            let kind = rng.next_u32() % 6;
            let amp = rng.range(0, 30);
            let re: Vec<i32> = (0..64).map(|n| value(&mut rng, kind, n + slot, amp)).collect();
            let im: Vec<i32> = (0..64).map(|n| value(&mut rng, kind, n + 2 * slot, amp)).collect();
            let init: Vec<i32> = (0..128).map(|_| rng.i32()).collect();
            let mut chr = init.clone();
            let mut chi: Vec<i32> = init.iter().map(|x| !x).collect();
            let mut rhr = chr.clone();
            let mut rhi = chi.clone();
            let ec = unsafe {
                cref_hybrid_ana_apply(c.0, re.as_ptr(), im.as_ptr(), chr.as_mut_ptr(), chi.as_mut_ptr())
            };
            let er = fdk_hybrid_analysis_apply(&mut f, &re, &im, &mut rhr, &mut rhi);
            assert_eq!(er, ec, "{w}: apply");
            assert_eq!(rhr, chr, "{w}: hybrid real");
            assert_eq!(rhi, chi, "{w}: hybrid imag");
            compare(&f, &c, &w);
        }

        let ec = unsafe { cref_hybrid_ana_close(c.0) };
        let er = fdk_hybrid_analysis_close(Some(&mut f));
        assert_eq!(er, ec);
        let cs = c.state();
        assert_eq!(ana_state(&f, &cs)[32..], cs[32..], "{what}: closed");
    }
    assert_eq!(fdk_hybrid_analysis_scale_states(None, 1), unsafe {
        cref_hybrid_ana_scale_states(core::ptr::null_mut(), 1)
    });
    assert_eq!(fdk_hybrid_analysis_close(None), unsafe {
        cref_hybrid_ana_close(core::ptr::null_mut())
    });
}

#[test]
fn hybrid_synthesis() {
    let mut rng = Rng::new(0x5e7);
    for run in 0..300 {
        let mode = if rng.next_u32() % 20 == 0 { 7 } else { (run % 3) as i32 };
        let qmf_bands = rng.range(3, 64);
        let cplx_bands = if rng.next_u32() % 3 == 0 { qmf_bands } else { rng.range(3, 64) };
        let c = CSyn(unsafe { cref_hybrid_syn_new() });
        let mut f = FdkSynHybFilter::default();
        let ec = unsafe { cref_hybrid_syn_init(c.0, mode, qmf_bands, cplx_bands) };
        let er = fdk_hybrid_synthesis_init(&mut f, mode, qmf_bands, cplx_bands);
        assert_eq!(er, ec);
        let mut cs = [0i32; 3];
        unsafe { cref_hybrid_syn_state(c.0, cs.as_mut_ptr()) };
        assert_eq!([f.nr_bands, f.cplx_bands, f.p_setup.is_some() as i32], cs);
        if er != 0 {
            continue;
        }
        for slot in 0..50 {
            let kind = rng.next_u32() % 6;
            let amp = rng.range(0, 30);
            let hre: Vec<i32> = (0..128).map(|n| value(&mut rng, kind, n + slot, amp)).collect();
            let him: Vec<i32> = (0..128).map(|n| value(&mut rng, kind, n + 3 * slot, amp)).collect();
            let init: Vec<i32> = (0..64).map(|_| rng.i32()).collect();
            let mut cre = init.clone();
            let mut cim: Vec<i32> = init.iter().map(|x| x ^ 0x1234).collect();
            let mut rre = cre.clone();
            let mut rim = cim.clone();
            unsafe {
                cref_hybrid_syn_apply(c.0, hre.as_ptr(), him.as_ptr(), cre.as_mut_ptr(), cim.as_mut_ptr())
            };
            fdk_hybrid_synthesis_apply(&f, &hre, &him, &mut rre, &mut rim);
            assert_eq!(rre, cre, "run {run} slot {slot}: real");
            assert_eq!(rim, cim, "run {run} slot {slot}: imag");
        }
    }
}

/// The file-static table `name` of FDK_hybrid.cpp (a copy compiled into the shim).
fn c_table<T: Copy>(name: &str) -> Vec<T> {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut bytes = 0;
    let p = unsafe { fdk_aac_rs_cref::fdk_hybrid_tables::cref_hybrid_table(cname.as_ptr(), &mut bytes) };
    assert!(!p.is_null(), "{name}");
    assert_eq!(bytes as usize % core::mem::size_of::<T>(), 0, "{name}");
    unsafe { core::slice::from_raw_parts(p as *const T, bytes as usize / core::mem::size_of::<T>()) }.to_vec()
}

/// The file-static tables of FDK_hybrid.cpp, element by element: RINGBUFF_IDX_TAB
/// (`ringbuffIdxTab`, which every setup points to), HYB_FILTER_COEF8_RAW and HYB_FILTER_COEF8
/// (`HybFilterCoef8`), HYB_FILTER_COEF2 and HYB_FILTER_COEF4.
///
/// CR and CI (`cr`, `ci`) are function-local in `fourChannelFiltering()` and cannot be named
/// from the shim; they are compared through that function by `hybrid_analysis` (modes
/// THREE_TO_TWELVE and THREE_TO_SIXTEEN). Their elements 2 and 10 are multiplied by
/// `HybFilterCoef4[10]` and `HybFilterCoef4[2]`, which are 0, so they do not affect any output.
#[test]
fn hybrid_tables() {
    assert_eq!(c_table::<i32>("ringbuffIdxTab"), RINGBUFF_IDX_TAB.to_vec());
    for s in [&SETUP_3_10, &SETUP_3_12, &SETUP_3_16] {
        assert!(core::ptr::eq(s.p_read_idx_table, &RINGBUFF_IDX_TAB[..]));
    }
    let c8 = c_table::<i16>("HybFilterCoef8");
    assert_eq!(c8, HYB_FILTER_COEF8_RAW.to_vec());
    let r8: Vec<i16> = HYB_FILTER_COEF8.iter().flat_map(|x| [x.re, x.im]).collect();
    assert_eq!(c8, r8);
    assert_eq!(c_table::<i16>("HybFilterCoef2"), HYB_FILTER_COEF2.to_vec());
    assert_eq!(c_table::<i16>("HybFilterCoef4"), HYB_FILTER_COEF4.to_vec());
    assert_eq!((HYB_FILTER_COEF4[2], HYB_FILTER_COEF4[10]), (0, 0));
    assert_eq!((CR.len(), CI.len()), (13, 13));
}

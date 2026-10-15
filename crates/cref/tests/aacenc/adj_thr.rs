// Differential tests of fdk_aac_rs_aacenc::adj_thr against the C++ reference.
//
// Per-function tests (the static functions through the namespace include of adj_thr.cpp in
// shim/aacenc_adj_thr.cpp) on
// - realistic element data: the quantizer input of `sf_estim_data` (corpus PCM through the
//   ported MDCT, band energies of the reference's helpers), run through the ported
//   FDKaacEnc_peCalculation, with stereo elements, MS masks and intensity bands,
// - the same data with edge values (MINVAL_DBL, MAXVAL_DBL, 0, ...) mixed in, and
// - fully random data (structurally valid band layouts, every array value random),
// comparing every output and the complete state (all QC_OUT_CHANNEL bytes, PE_DATA,
// ATS_ELEMENT, the ahFlag/thrExp arrays). The C functions get the dynamic RAM buffers filled
// with two different poison bytes; both runs must agree (no stale reads).
//
// The stateful tests on real encoder states are in adj_thr_stateful.rs (harness: adj_thr_enc.rs).

use fdk_aac_rs_aacenc::adj_thr::*;
use fdk_aac_rs_aacenc::adj_thr_data::*;
use fdk_aac_rs_aacenc::interface::{PsyOut, PsyOutChannel, PsyOutElement, ToolsInfo};
use fdk_aac_rs_aacenc::line_pe::PeData;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_aacenc::qc_data::{ChannelMapping, QcOut, QcOutChannel, QcOutElement};
use fdk_aac_rs_cref::aacenc_adj_thr as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};
use std::ffi::c_void;

pub type Ah = [[u8; MAX_GROUPED_SFB]; 2];
pub type Te = [[FixpDbl; MAX_GROUPED_SFB]; 2];

fn vp<T>(x: &T) -> *const c_void {
    x as *const T as *const c_void
}
fn mp<T>(x: &mut T) -> *mut c_void {
    x as *mut T as *mut c_void
}

/// Edge values of `FIXP_DBL`.
const EDGES: [i32; 9] =
    [0, 1, -1, MAXVAL_DBL, MINVAL_DBL, MINVAL_DBL + 1, MAXVAL_DBL - 1, 0x4000_0000, -0x4000_0000];

/// Data flavour of the generated elements.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Flavour {
    /// realistic data
    Real,
    /// realistic data with edge values
    Edge,
    /// every value random
    Random,
}

fn edge_i32(rng: &mut Rng, v: i32, f: Flavour) -> i32 {
    match f {
        Flavour::Real => v,
        Flavour::Edge => {
            if rng.range(0, 9) == 0 {
                if rng.range(0, 1) == 0 {
                    EDGES[rng.range(0, 8) as usize]
                } else {
                    rng.i32()
                }
            } else {
                v
            }
        }
        Flavour::Random => rng.i32(),
    }
}

fn mutate(rng: &mut Rng, a: &mut [i32], f: Flavour) {
    for x in a.iter_mut() {
        *x = edge_i32(rng, *x, f);
    }
}

/// Whether the C++ reference of this test binary was compiled by Apple clang for x86-64
/// (`--target x86_64-apple-darwin`, ref-x86_64-mac). Its valley branch of
/// FDKaacEnc_initAvoidHoleFlag differs from ref-x86_64 (Debian clang 19), which the port's `x86`
/// variant follows, where the additions overflow; see `mutate_ld`.
const APPLE_CLANG_X86: bool = cfg!(all(target_arch = "x86_64", target_vendor = "apple"));

/// `mutate` for LD data (`CalcLdData()` of a non-negative value: not positive).
///
/// The encoder's LD energies, thresholds, form factors and SNRs are not positive and its
/// energies not negative. Outside this domain, the valley branch of FDKaacEnc_initAvoidHoleFlag
/// (adj_thr.cpp:604-613, `avgEnLdData - sfbEnLdData - SnrLdMin4 + sfbMinSnrLdData` and
/// `sfbMinSnrLdData + SnrLdMin2`) overflows, undefined behavior that the reference builds compile
/// differently: Apple clang 21 (ref-arm64 and ref-x86_64-mac) as
/// `fixMin(fixMin(avgEnLdData - sfbEnLdData, SnrLdMin2 + SnrLdMin4) + (minSnr - SnrLdMin4),
/// SnrLdFac)`, Debian clang 19 (ref-x86_64, ref-i686) as
/// `fixMin(fixMin(avgEnLdData - sfbEnLdData + minSnr - SnrLdMin4, minSnr + SnrLdMin2), SnrLdFac)`
/// with wrapping additions. The port reproduces ref-arm64, ref-x86_64 and ref-i686, so the data
/// keep every sign there. Only where the reference is ref-x86_64-mac (`APPLE_CLANG_X86`) are
/// values of the wrong sign mapped to the right one (`!x`, so `MAXVAL_DBL` becomes
/// `MINVAL_DBL`). The encoder does not reach the overflow: an UndefinedBehaviorSanitizer build of
/// the reference (arm64) encoded all 785 encoder entries of the e2e corpus without a report in
/// adj_thr.cpp.
fn mutate_ld(rng: &mut Rng, a: &mut [i32], f: Flavour) {
    for x in a.iter_mut() {
        let v = edge_i32(rng, *x, f);
        *x = if APPLE_CLANG_X86 && v > 0 { !v } else { v };
    }
}

/// `mutate` for energies (not negative where the reference is ref-x86_64-mac; see `mutate_ld`).
fn mutate_en(rng: &mut Rng, a: &mut [i32], f: Flavour) {
    for x in a.iter_mut() {
        let v = edge_i32(rng, *x, f);
        *x = if APPLE_CLANG_X86 && v < 0 { !v } else { v };
    }
}

/// One element (1 or 2 channels with the same band layout for 2 channels, as the encoder's
/// common window CPEs), realistic or random.
pub fn gen_element(
    rng: &mut Rng,
    n: usize,
    f: Flavour,
) -> (Vec<PsyOutChannel>, Vec<QcOutChannel>, ToolsInfo) {
    let short = rng.range(0, 3) == 0;
    // Inputs without a defined result in FDKaacEnc_reduceThresholdsVBR of the reference are
    // skipped.
    let d0 = loop {
        let d = crate::sf_estim_data::channel(rng, short);
        if d.group_len.iter().all(|&g| g < 8) {
            break d;
        }
    };
    let mut psy = vec![PsyOutChannel::default(); n];
    let mut qc = vec![QcOutChannel::default(); n];
    for ch in 0..n {
        let d = if ch == 0 { d0.clone() } else { crate::sf_estim_data::channel(rng, short) };
        let p = &mut psy[ch];
        p.sfb_cnt = d0.sfb_cnt;
        p.sfb_per_group = d0.sfb_per_group;
        p.max_sfb_per_group = d0.max_sfb_per_group;
        p.last_window_sequence = if short {
            SHORT_WINDOW
        } else {
            [LONG_WINDOW, LONG_WINDOW, START_WINDOW, STOP_WINDOW][rng.range(0, 3) as usize]
        };
        p.window_shape = rng.range(0, 1);
        p.grouping_mask = rng.range(0, 255);
        p.sfb_offsets = d0.sfb_offsets;
        p.mdct_scale = rng.range(0, 10);
        p.group_len = d0.group_len;
        let q = &mut qc[ch];
        q.mdct_spectrum = d.spectrum;
        for i in 0..1024 {
            q.quant_spec[i] = rng.i16();
        }
        q.sfb_form_factor_ld_data = d.form_factor_ld;
        q.sfb_threshold_ld_data = d.thr_ld;
        q.sfb_min_snr_ld_data = d.min_snr_ld;
        q.sfb_energy_ld_data = d.energy_ld;
        q.sfb_energy = d.energy;
        q.sfb_spread_energy = d.spread_energy;
        for i in 0..MAX_GROUPED_SFB {
            q.sfb_weighted_energy_ld_data[i] = d.energy_ld[i];
            q.sfb_en_fac_ld[i] = -(rng.range(0, 1 << 24));
            q.scf[i] = rng.range(-10, 60);
            q.max_value_in_sfb[i] = rng.next_u32() % 9000;
        }
        q.global_gain = rng.range(0, 255);
        if ch == 1 && rng.range(0, 3) == 0 {
            // intensity bands
            for i in 0..MAX_GROUPED_SFB {
                if rng.range(0, 4) == 0 {
                    p.is_book[i] = [14, 15][rng.range(0, 1) as usize];
                    p.is_scale[i] = rng.range(-30, 30);
                }
            }
        }
        for i in 0..MAX_GROUPED_SFB {
            p.noise_nrg[i] = rng.range(-100, 100);
        }
        if f != Flavour::Real {
            mutate_ld(rng, &mut q.sfb_form_factor_ld_data, f);
            mutate_ld(rng, &mut q.sfb_threshold_ld_data, f);
            mutate_ld(rng, &mut q.sfb_min_snr_ld_data, f);
            mutate_ld(rng, &mut q.sfb_energy_ld_data, f);
            mutate_en(rng, &mut q.sfb_energy, f);
            mutate_en(rng, &mut q.sfb_spread_energy, f);
            mutate_ld(rng, &mut q.sfb_weighted_energy_ld_data, f);
            mutate_ld(rng, &mut q.sfb_en_fac_ld, f);
        }
    }
    let mut tools = ToolsInfo::default();
    if n == 2 {
        tools.ms_digest = rng.range(0, 2);
        let p = rng.range(0, 3);
        for i in 0..MAX_GROUPED_SFB {
            tools.ms_mask[i] = (rng.range(0, 3) < p) as i32;
        }
    }
    (psy, qc, tools)
}

/// A random ATS_ELEMENT state (realistic init values, then randomized state).
pub fn gen_ats(rng: &mut Rng, f: Flavour) -> AtsElement {
    let mut a = AtsElement {
        pe_min: rng.range(0, 3000),
        pe_max: rng.range(0, 6000),
        pe_offset: [0, 50, 75, 100][rng.range(0, 3) as usize],
        bits2_pe_factor_m: rng.range(0x0800_0000, 0x6000_0000),
        bits2_pe_factor_e: rng.range(1, 3),
        ah_param: if rng.range(0, 1) == 0 {
            AhParam { modify_min_snr: 1, start_sfb_l: 15, start_sfb_s: 3 }
        } else {
            AhParam { modify_min_snr: 0, start_sfb_l: 0, start_sfb_s: 0 }
        },
        min_snr_adapt_param: MinsnrAdaptParam {
            max_red: 0x0080_0000,
            start_ratio: 0x06a4_e6c9,
            max_ratio: 0,
            red_ratio_fac: -0x3000_0000,
            red_offs: 0x02c0_0000,
        },
        pe_last: rng.range(0, 5000),
        dyn_bits_last: rng.range(-1, 8000),
        pe_correction_factor_m: rng.range(0x3000_0000, 0x4a00_0000),
        pe_correction_factor_e: 1,
        vbr_qual_factor: [0x1333_3333, 0x14bc_6a7f, 0x1687_2b02, 0x0f5c_28f6, 0x08f5_c28f]
            [rng.range(0, 4) as usize],
        chaos_measure_old: rng.range(0, 0x7fff_ffff),
        chaos_measure_en_fac: [rng.range(0, 0x7fff_ffff), rng.range(0, 0x7fff_ffff)],
        last_en_fac_patch: [rng.range(0, 1), rng.range(0, 1)],
    };
    if a.pe_min > a.pe_max {
        std::mem::swap(&mut a.pe_min, &mut a.pe_max);
    }
    if f == Flavour::Random {
        a.chaos_measure_old = rng.i32();
        a.pe_last = rng.i32();
        a.dyn_bits_last = rng.i32();
        a.pe_correction_factor_m = rng.i32();
    }
    a
}

fn psy_ptr(p: &[PsyOutChannel]) -> *const c_void {
    p.as_ptr() as *const c_void
}

fn assert_qc_eq(a: &[QcOutChannel], b: &[QcOutChannel], what: &str) {
    for ch in 0..a.len() {
        let (x, y) = (&a[ch], &b[ch]);
        if x != y {
            for (name, u, v) in [
                ("threshold", &x.sfb_threshold_ld_data[..], &y.sfb_threshold_ld_data[..]),
                ("min_snr", &x.sfb_min_snr_ld_data[..], &y.sfb_min_snr_ld_data[..]),
                ("spread", &x.sfb_spread_energy[..], &y.sfb_spread_energy[..]),
                ("weighted", &x.sfb_weighted_energy_ld_data[..], &y.sfb_weighted_energy_ld_data[..]),
                ("en_fac", &x.sfb_en_fac_ld[..], &y.sfb_en_fac_ld[..]),
            ] {
                if u != v {
                    let i = (0..u.len()).find(|&i| u[i] != v[i]).unwrap();
                    panic!("{what}: ch {ch} {name}[{i}]: rust {:#x} c {:#x}", u[i], v[i]);
                }
            }
            if x.quant_spec != y.quant_spec {
                let i = (0..1024).find(|&i| x.quant_spec[i] != y.quant_spec[i]).unwrap();
                panic!("{what}: ch {ch} quant_spec[{i}]: {} {}", x.quant_spec[i], y.quant_spec[i]);
            }
            panic!("{what}: ch {ch} QC_OUT_CHANNEL differs");
        }
    }
}

const FLAVOURS: [Flavour; 3] = [Flavour::Real, Flavour::Edge, Flavour::Random];

/// Iterations per flavour (`ADJ_THR_ITERS` overrides the default for longer local runs).
fn iters() -> usize {
    std::env::var("ADJ_THR_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(150)
}

#[test]
fn psy_prefix_size() {
    assert_eq!(unsafe { c::cref_adj_thr_psy_prefix_size() } as usize, size_of::<PsyOutChannel>());
}

#[test]
fn tables() {
    use fdk_aac_rs_aacenc::adj_thr::tables::*;
    let get = |i: i32| -> Vec<i32> {
        let mut n = 0;
        let p = unsafe { c::cref_adj_thr_table(i, &mut n) };
        unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec()
    };
    assert_eq!(get(0), INV_INT.to_vec());
    assert_eq!(get(1), INV_SQRT_4.to_vec());
    let scal = [
        SNR_LD_MIN_1,
        SNR_LD_MIN_2,
        SNR_LD_FAC,
        SNR_LD_MIN_3,
        SNR_LD_MIN_4,
        SNR_LD_MIN_5,
        LIMIT_THR_REDUCED_LD_DATA,
    ];
    for (k, v) in scal.iter().enumerate() {
        assert_eq!(get(2 + k as i32), vec![*v], "scalar {k}");
    }
    assert_eq!(get(15), BITS2_PE_CONFIG_TAB.iter().map(|t| t.sample_rate).collect::<Vec<_>>());
    for (k, t) in BITS2_PE_CONFIG_TAB.iter().enumerate() {
        let flat: Vec<i32> = t
            .p_pe_tab
            .iter()
            .flat_map(|e| {
                [
                    e.bitrate,
                    e.bits2_pe_factor[0][0],
                    e.bits2_pe_factor[0][1],
                    e.bits2_pe_factor[1][0],
                    e.bits2_pe_factor[1][1],
                ]
            })
            .collect();
        assert_eq!(get(9 + k as i32), flat, "pe tab {k}");
    }
    // The pPeTab of the entries are S_BITS_2_PE_TAB_16000 (`sBits2PeTab16000`), ..._22050,
    // ..._24000, ..._32000, ..._44100 and ..._48000, compared element by element above.
    let pe_tabs: [&[BitPeSfac]; 6] = [
        &S_BITS_2_PE_TAB_16000,
        &S_BITS_2_PE_TAB_22050,
        &S_BITS_2_PE_TAB_24000,
        &S_BITS_2_PE_TAB_32000,
        &S_BITS_2_PE_TAB_44100,
        &S_BITS_2_PE_TAB_48000,
    ];
    for (k, t) in BITS2_PE_CONFIG_TAB.iter().enumerate() {
        assert!(core::ptr::eq(t.p_pe_tab, pe_tabs[k]), "pPeTab of entry {k}");
    }
}

#[test]
fn init_bits2_pe_factor() {
    let rates = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
    let mut rng = Rng::new(1);
    let mut cases = Vec::new();
    for &sr in &rates {
        for n in 1..=3 {
            for adv in 0..2 {
                for dz in 0..2 {
                    for inv in [0, 2] {
                        for br in (0..400_000).step_by(3700) {
                            cases.push((br, n, sr, adv, dz, inv));
                        }
                        for b in [10000, 16000, 20000, 24000, 32000, 40000, 148000, 320000, 0] {
                            for d in [-1, 0, 1] {
                                cases.push((b * n + d, n, sr, adv, dz, inv));
                            }
                        }
                    }
                }
            }
        }
    }
    for _ in 0..2000 {
        cases.push((rng.range(1, i32::MAX - 1), rng.range(1, 2), rng.range(0, 200000), rng.range(0, 1), rng.range(0, 1), rng.range(0, 2)));
    }
    for (br, n, sr, adv, dz, inv) in cases {
        let (mut m0, mut e0, mut m1, mut e1) = (0, 0, 0, 0);
        fdk_aac_enc_init_bits2_pe_factor(&mut m0, &mut e0, br, n, sr, adv, dz, inv);
        unsafe { c::cref_adj_thr_InitBits2PeFactor(&mut m1, &mut e1, br, n, sr, adv, dz, inv) };
        assert_eq!((m0, e0), (m1, e1), "{br} {n} {sr} {adv} {dz} {inv}");
    }
}

#[test]
fn bits2pe2() {
    let mut rng = Rng::new(2);
    for k in 0..200_000 {
        let bits = if k % 2 == 0 { rng.range(-100, 100_000) } else { rng.i32() };
        let m = rng.i32();
        let e = if k % 3 == 0 { rng.range(-40, 40) } else { rng.range(0, 10) };
        assert_eq!(
            fdk_aac_enc_bits2pe2(bits, m, e),
            unsafe { c::cref_adj_thr_bits2pe2(bits, m, e) },
            "{bits} {m} {e}"
        );
    }
}

#[test]
fn calc_thresh_exp() {
    let mut rng = Rng::new(3);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, _) = gen_element(&mut rng, n, f);
            let mut t0: Te = [[0; MAX_GROUPED_SFB]; 2];
            let mut t1 = t0;
            fdk_aac_enc_calc_thresh_exp(&mut t0, &qc, &psy, n as i32);
            unsafe {
                c::cref_adj_thr_calcThreshExp(t1.as_mut_ptr() as *mut i32, qc.as_mut_ptr() as *mut c_void, psy_ptr(&psy), n as i32)
            };
            assert_eq!(t0, t1);
        }
    }
}

#[test]
fn adapt_min_snr() {
    let mut rng = Rng::new(4);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, _) = gen_element(&mut rng, n, f);
            let mut msa = gen_ats(&mut rng, f).min_snr_adapt_param;
            if f == Flavour::Random {
                msa.max_red = rng.i32();
                msa.start_ratio = rng.i32();
                msa.red_ratio_fac = rng.i32();
                msa.red_offs = rng.i32();
            }
            let mut qc1 = qc.clone();
            fdk_aac_enc_adapt_min_snr(&mut qc, &psy, &msa, n as i32);
            unsafe { c::cref_adj_thr_adaptMinSnr(qc1.as_mut_ptr() as *mut c_void, psy_ptr(&psy), vp(&msa), n as i32) };
            assert_qc_eq(&qc, &qc1, "adaptMinSnr");
        }
    }
}

#[test]
fn init_avoid_hole_flag() {
    let mut rng = Rng::new(5);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, tools) = gen_element(&mut rng, n, f);
            let ahp = gen_ats(&mut rng, f).ah_param;
            let mut qc1 = qc.clone();
            let mut a0: Ah = [[0x55; MAX_GROUPED_SFB]; 2];
            let mut a1 = a0;
            fdk_aac_enc_init_avoid_hole_flag(&mut qc, &psy, &mut a0, &tools, n as i32, &ahp);
            unsafe {
                c::cref_adj_thr_initAvoidHoleFlag(qc1.as_mut_ptr() as *mut c_void, psy_ptr(&psy), a1.as_mut_ptr() as *mut u8, vp(&tools), n as i32, vp(&ahp))
            };
            assert_qc_eq(&qc, &qc1, "initAvoidHoleFlag");
            assert_eq!(a0, a1);
        }
    }
}

/// The valley branch of FDKaacEnc_initAvoidHoleFlag where its additions overflow:
/// realistic elements whose energies, LD energies and minimum SNRs are replaced, in random bands,
/// by values that pass the valley test (`sfbEnLdData + SnrLdMin4 < avgEnLdData`, `sfbEn > 0`)
/// with `avgEnLdData - sfbEnLdData - SnrLdMin4 + sfbMinSnrLdData` or `sfbMinSnrLdData +
/// SnrLdMin2` out of range.
#[test]
fn init_avoid_hole_flag_valley_overflow() {
    if APPLE_CLANG_X86 {
        eprintln!("skipped: the reference is ref-x86_64-mac (Apple clang)");
        return;
    }
    const SNR_LD_MIN2: i64 = 0x0351_e1a2;
    const SNR_LD_MIN4: i64 = 0x0200_0000;
    let mut rng = Rng::new(0x44);
    let mut overflows = 0;
    for _ in 0..600 {
        let n = rng.range(1, 2) as usize;
        let (psy, mut qc, tools) = gen_element(&mut rng, n, Flavour::Real);
        for q in qc.iter_mut() {
            for i in 0..MAX_GROUPED_SFB {
                if rng.range(0, 2) != 0 {
                    continue;
                }
                q.sfb_energy[i] = [1, 2, 0x100, rng.range(1, i32::MAX)][rng.range(0, 3) as usize];
                q.sfb_energy_ld_data[i] =
                    [MINVAL_DBL, MINVAL_DBL + 1, -0x7000_0000, rng.range(i32::MIN, -0x4000_0000)]
                        [rng.range(0, 3) as usize];
                q.sfb_min_snr_ld_data[i] = [
                    MAXVAL_DBL,
                    MAXVAL_DBL - 0x0351_e1a1,
                    MAXVAL_DBL - 0x0351_e1a2,
                    0x4000_0000,
                    0x7000_0000,
                    rng.range(0, i32::MAX),
                    rng.i32(),
                ][rng.range(0, 6) as usize];
            }
        }
        // count the bands of the layout whose values make the valley additions overflow
        for (p, q) in psy.iter().zip(&qc) {
            for i in 0..(p.sfb_cnt.max(0) as usize).min(MAX_GROUPED_SFB) {
                let ms = q.sfb_min_snr_ld_data[i] as i64;
                let x = -(q.sfb_energy_ld_data[i] as i64) - SNR_LD_MIN4 + ms;
                if q.sfb_energy[i] > 0 && (x > i32::MAX as i64 || ms + SNR_LD_MIN2 > i32::MAX as i64)
                {
                    overflows += 1;
                }
            }
        }
        let mut ahp = gen_ats(&mut rng, Flavour::Real).ah_param;
        ahp.modify_min_snr = 1;
        let mut qc1 = qc.clone();
        let mut a0: Ah = [[0x55; MAX_GROUPED_SFB]; 2];
        let mut a1 = a0;
        fdk_aac_enc_init_avoid_hole_flag(&mut qc, &psy, &mut a0, &tools, n as i32, &ahp);
        unsafe {
            c::cref_adj_thr_initAvoidHoleFlag(qc1.as_mut_ptr() as *mut c_void, psy_ptr(&psy), a1.as_mut_ptr() as *mut u8, vp(&tools), n as i32, vp(&ahp))
        };
        assert_qc_eq(&qc, &qc1, "initAvoidHoleFlag valley overflow");
        assert_eq!(a0, a1);
    }
    eprintln!("initAvoidHoleFlag valley overflow: {overflows} bands with overflowing values");
    assert!(overflows > 1000, "{overflows}");
}

#[test]
fn prepare_pe_calc_weighting_calc_pe() {
    let mut rng = Rng::new(6);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, tools) = gen_element(&mut rng, n, f);
            let mut qc1 = qc.clone();
            let offset = rng.range(0, 100);
            let mut pe0 = PeData::default();
            let mut pe1 = PeData::default();
            fdk_aac_enc_prepare_pe(&mut pe0, &psy, &qc, n as i32, offset);
            unsafe { c::cref_adj_thr_preparePe(mp(&mut pe1), psy_ptr(&psy), qc1.as_mut_ptr() as *mut c_void, n as i32, offset) };
            assert_eq!(pe0, pe1, "preparePe");

            let mut ats0 = gen_ats(&mut rng, f);
            let mut ats1 = ats0;
            let use_patch = rng.range(0, 3).min(1);
            fdk_aac_enc_calc_weighting(&pe0, &psy, &mut qc, &tools, &mut ats0, n as i32, use_patch);
            unsafe {
                c::cref_adj_thr_calcWeighting(vp(&pe1), psy_ptr(&psy), qc1.as_mut_ptr() as *mut c_void, vp(&tools), mp(&mut ats1), n as i32, use_patch)
            };
            assert_qc_eq(&qc, &qc1, "calcWeighting");
            assert_eq!(ats0, ats1, "calcWeighting");

            fdk_aac_enc_calc_pe(&psy, &qc, &mut pe0, n as i32);
            unsafe { c::cref_adj_thr_calcPe(psy_ptr(&psy), qc1.as_mut_ptr() as *mut c_void, mp(&mut pe1), n as i32) };
            assert_eq!(pe0, pe1, "calcPe");
        }
    }
}

#[test]
fn pe_calculation() {
    let mut rng = Rng::new(7);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, tools) = gen_element(&mut rng, n, f);
            let mut qc1 = qc.clone();
            let mut ats0 = gen_ats(&mut rng, f);
            let mut ats1 = ats0;
            let mut pe0 = PeData::default();
            let mut pe1 = PeData::default();
            fdk_aac_enc_pe_calculation(&mut pe0, &psy, &mut qc, &tools, &mut ats0, n as i32);
            unsafe {
                c::cref_adj_thr_peCalculation(mp(&mut pe1), psy_ptr(&psy), qc1.as_mut_ptr() as *mut c_void, vp(&tools), mp(&mut ats1), n as i32)
            };
            assert_qc_eq(&qc, &qc1, "peCalculation");
            assert_eq!(ats0, ats1);
            assert_eq!(pe0, pe1);
        }
    }
}

fn gen_ah(rng: &mut Rng) -> Ah {
    let mut a: Ah = [[0; MAX_GROUPED_SFB]; 2];
    for ch in a.iter_mut() {
        for x in ch.iter_mut() {
            *x = rng.range(0, 2) as u8;
        }
    }
    a
}

fn gen_te(rng: &mut Rng, f: Flavour, qc: &[QcOutChannel], psy: &[PsyOutChannel]) -> Te {
    let mut t: Te = [[0; MAX_GROUPED_SFB]; 2];
    fdk_aac_enc_calc_thresh_exp(&mut t, qc, psy, qc.len() as i32);
    for ch in t.iter_mut() {
        mutate(rng, ch, f);
    }
    t
}

/// A realistic element after the pe calculation (PE_DATA from the ported peCalculation).
fn gen_element_pe(
    rng: &mut Rng,
    n: usize,
    f: Flavour,
) -> (Vec<PsyOutChannel>, Vec<QcOutChannel>, ToolsInfo, PeData, AtsElement) {
    let (psy, mut qc, tools) = gen_element(rng, n, f);
    let mut ats = gen_ats(rng, f);
    let mut pe = PeData::default();
    fdk_aac_enc_pe_calculation(&mut pe, &psy, &mut qc, &tools, &mut ats, n as i32);
    if f == Flavour::Random {
        for ch in 0..2 {
            let p = &mut pe.pe_channel_data[ch];
            mutate(rng, &mut p.sfb_pe, f);
            mutate(rng, &mut p.sfb_const_part, f);
            // (line counts in the encoder's range: others have no defined x86 reference result)
            for x in p.sfb_n_active_lines.iter_mut() {
                *x = rng.range(0, 1100);
            }
            mutate(rng, &mut p.sfb_n_lines, f);
        }
    }
    (psy, qc, tools, pe, ats)
}

#[test]
fn calc_pe_no_ah() {
    let mut rng = Rng::new(8);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let n = rng.range(1, 2) as usize;
            let (psy, _, _, pe, _) = gen_element_pe(&mut rng, n, f);
            let ah = gen_ah(&mut rng);
            let mut r = [0i32; 3];
            let mut o = [0i32; 3];
            {
                let [a, b, cc] = &mut r;
                fdk_aac_enc_calc_pe_no_ah(a, b, cc, &pe, &ah, &psy, n as i32);
            }
            unsafe { c::cref_adj_thr_calcPeNoAH(o.as_mut_ptr(), vp(&pe), ah.as_ptr() as *const u8, psy_ptr(&psy), n as i32) };
            assert_eq!(r, o);
        }
    }
}

#[test]
fn reduce_thresholds_cbr() {
    let mut rng = Rng::new(9);
    for f in FLAVOURS {
        for _ in 0..iters() * 3 {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, _, _, _) = gen_element_pe(&mut rng, n, f);
            let te = gen_te(&mut rng, f, &qc, &psy);
            let mut a0 = gen_ah(&mut rng);
            let mut a1 = a0;
            let mut qc1 = qc.clone();
            let m = match rng.range(0, 3) {
                0 => rng.i32().max(0),
                1 => rng.i32(),
                _ => rng.range(0, 0x7fff_ffff),
            };
            let e = if rng.range(0, 3) == 0 { rng.range(-128, 127) } else { rng.range(-8, 8) };
            // the straight port (the reference of the optimized function)
            let (mut qc2, mut a2) = (qc.clone(), a0);
            fdk_aac_enc_reduce_thresholds_cbr_straight(&mut qc2, &psy, &mut a2, &te, n as i32, m, e as i8);
            fdk_aac_enc_reduce_thresholds_cbr(&mut qc, &psy, &mut a0, &te, n as i32, m, e as i8);
            assert_qc_eq(&qc, &qc2, &format!("reduceThresholdsCBR straight m={m:#x} e={e}"));
            assert_eq!(a0, a2);
            unsafe {
                c::cref_adj_thr_reduceThresholdsCBR(qc1.as_mut_ptr() as *mut c_void, psy_ptr(&psy), a1.as_mut_ptr() as *mut u8, te.as_ptr() as *const i32, n as i32, m, e)
            };
            assert_qc_eq(&qc, &qc1, &format!("reduceThresholdsCBR m={m:#x} e={e}"));
            assert_eq!(a0, a1);
        }
    }
}

#[test]
fn calc_chaos_measure() {
    let mut rng = Rng::new(10);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let (psy, mut qc, _) = gen_element(&mut rng, 1, f);
            let r = fdk_aac_enc_calc_chaos_measure(&psy[0], &qc[0]);
            let o = unsafe { c::cref_adj_thr_calcChaosMeasure(psy_ptr(&psy), qc.as_mut_ptr() as *mut c_void) };
            assert_eq!(r, o);
        }
    }
}

#[test]
fn reduce_thresholds_vbr() {
    let mut rng = Rng::new(11);
    for f in FLAVOURS {
        for _ in 0..iters() * 2 {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, _, _, ats) = gen_element_pe(&mut rng, n, f);
            let te = gen_te(&mut rng, f, &qc, &psy);
            let mut a0 = gen_ah(&mut rng);
            let mut a1 = a0;
            let mut qc1 = qc.clone();
            let q = if f == Flavour::Random { rng.i32() } else { ats.vbr_qual_factor };
            let (mut old0, mut old1) = (ats.chaos_measure_old, ats.chaos_measure_old);
            fdk_aac_enc_reduce_thresholds_vbr(&mut qc, &psy, &mut a0, &te, n as i32, q, &mut old0);
            unsafe {
                c::cref_adj_thr_reduceThresholdsVBR(qc1.as_mut_ptr() as *mut c_void, psy_ptr(&psy), a1.as_mut_ptr() as *mut u8, te.as_ptr() as *const i32, n as i32, q, &mut old1)
            };
            assert_eq!(old0, old1, "chaosMeasureOld {f:?} n={n} lws={} q={q:#x}", psy[0].last_window_sequence);
            assert_qc_eq(&qc, &qc1, &format!("reduceThresholdsVBR {f:?}"));
            assert_eq!(a0, a1);
        }
    }
}

#[test]
fn reset_ah_flags() {
    let mut rng = Rng::new(12);
    for _ in 0..iters() {
        let n = rng.range(1, 2) as usize;
        let (psy, _, _) = gen_element(&mut rng, n, Flavour::Real);
        let mut a0 = gen_ah(&mut rng);
        let mut a1 = a0;
        fdk_aac_enc_reset_ah_flags(&mut a0, n as i32, &psy);
        unsafe { c::cref_adj_thr_resetAHFlags(a1.as_mut_ptr() as *mut u8, n as i32, psy_ptr(&psy)) };
        assert_eq!(a0, a1);
    }
}

#[test]
fn red_val_power() {
    let mut rng = Rng::new(13);
    for k in 0..100_000 {
        let num = if k % 2 == 0 { rng.range(-200_000, 200_000) } else { rng.i32() };
        let den = if k % 3 == 0 { rng.range(1, 400_000) } else { rng.i32().max(1) };
        let (mut s0, mut s1) = (0, 0);
        let r = fdk_aac_rs_aacenc::adj_thr::calc_red_val_power(num, den, &mut s0);
        let o = unsafe { c::cref_adj_thr_CalcRedValPower(num, den, &mut s1) };
        assert_eq!((r, s0), (o, s1), "{num} {den}");
    }
}

#[test]
fn adapt_thresholds_vbr() {
    let mut rng = Rng::new(14);
    for f in FLAVOURS {
        for _ in 0..iters() {
            let n = rng.range(1, 2) as usize;
            let (psy, mut qc, tools, _, mut ats0) = gen_element_pe(&mut rng, n, f);
            let mut ats1 = ats0;
            let mut qc1 = qc.clone();
            fdk_aac_enc_adapt_thresholds_vbr(&mut qc, &psy, &mut ats0, &tools, n as i32);
            unsafe {
                c::cref_adj_thr_AdaptThresholdsVBR(qc1.as_mut_ptr() as *mut c_void, psy_ptr(&psy), mp(&mut ats1), vp(&tools), n as i32)
            };
            assert_qc_eq(&qc, &qc1, "AdaptThresholdsVBR");
            assert_eq!(ats0, ats1);
        }
    }
}

#[test]
fn calc_bit_save_spend() {
    let mut rng = Rng::new(15);
    for _ in 0..100_000 {
        let a: [i32; 6] = std::array::from_fn(|_| rng.i32());
        assert_eq!(
            fdk_aac_enc_calc_bit_save(a[0], a[1], a[2], a[3], a[4], a[5]),
            unsafe { c::cref_adj_thr_calcBitSave(a[0], a[1], a[2], a[3], a[4], a[5]) }
        );
        assert_eq!(
            fdk_aac_enc_calc_bit_spend(a[0], a[1], a[2], a[3], a[4], a[5]),
            unsafe { c::cref_adj_thr_calcBitSpend(a[0], a[1], a[2], a[3], a[4], a[5]) }
        );
    }
}

#[test]
fn adjust_pe_min_max() {
    let mut rng = Rng::new(16);
    for k in 0..200_000 {
        let (pe, mut mn, mut mx) = if k % 3 == 0 {
            (rng.i32(), rng.i32(), rng.i32())
        } else {
            (rng.range(0, 20000), rng.range(0, 10000), rng.range(0, 20000))
        };
        let (mut mn1, mut mx1) = (mn, mx);
        fdk_aac_enc_adjust_pe_min_max(pe, &mut mn, &mut mx);
        unsafe { c::cref_adj_thr_adjustPeMinMax(pe, &mut mn1, &mut mx1) };
        assert_eq!((mn, mx), (mn1, mx1), "{pe}");
    }
}

fn gen_bres(rng: &mut Rng, f: Flavour) -> [BresParam; 2] {
    let mut st = AdjThrState::default();
    fdk_aac_enc_adj_thr_new(&mut st, 1);
    let mut cm = ChannelMapping::default();
    cm.n_elements = 1;
    cm.el_info[0].n_channels_in_el = 1;
    cm.el_info[0].relative_bits = MAXVAL_DBL;
    fdk_aac_enc_adj_thr_init(&mut st, 1000, 0, &cm, 48000, 64000, 0, 0, 0, 0, 0);
    let mut b = [st.bres_param_long, st.bres_param_short];
    if f == Flavour::Random {
        for p in b.iter_mut() {
            p.clip_save_low = rng.i32();
            p.clip_save_high = rng.i32();
            p.max_bit_save = rng.i32();
            p.min_bit_spend = rng.i32();
            p.clip_spend_low = rng.i32();
            p.clip_spend_high = rng.i32();
        }
    }
    b
}

#[test]
fn bitres_calc_bit_fac() {
    let mut rng = Rng::new(17);
    for f in FLAVOURS {
        for _ in 0..30_000 {
            let bres = gen_bres(&mut rng, f);
            let mut st = AdjThrState::default();
            st.bres_param_long = bres[0];
            st.bres_param_short = bres[1];
            let mut ats0 = gen_ats(&mut rng, f);
            if f == Flavour::Random {
                ats0.pe_min = rng.i32();
                ats0.pe_max = rng.i32();
                if ats0.pe_min > ats0.pe_max {
                    std::mem::swap(&mut ats0.pe_min, &mut ats0.pe_max);
                }
            }
            let mut ats1 = ats0;
            let max_bitres = rng.range(0, 6144 * 8);
            let bitres = if f == Flavour::Real { rng.range(0, max_bitres) } else { rng.range(0, 6144 * 8) };
            let pe = if f == Flavour::Random { rng.i32() } else { rng.range(0, 15000) };
            let lws = [LONG_WINDOW, START_WINDOW, SHORT_WINDOW, STOP_WINDOW][rng.range(0, 3) as usize];
            let avg = rng.range(1, 12000);
            let mbf = if f == Flavour::Random { rng.i32() } else { rng.range(0x0100_0000, 0x0800_0000) };
            let (mut fac0, mut e0, mut fac1, mut e1) = (0, 0, 0, 0);
            fdk_aac_enc_bitres_calc_bit_fac(bitres, max_bitres, pe, lws, avg, mbf, &st, &mut ats0, &mut fac0, &mut e0);
            unsafe {
                c::cref_adj_thr_bitresCalcBitFac(bitres, max_bitres, pe, lws, avg, mbf, bres.as_ptr() as *const c_void, mp(&mut ats1), &mut fac1, &mut e1)
            };
            assert_eq!((fac0, e0), (fac1, e1), "{bitres} {max_bitres} {pe} {avg} {mbf:#x} {ats0:?}");
            assert_eq!(ats0, ats1);
        }
    }
}

/// CHANNEL_MAPPING of a channel mode (C FDKaacEnc_InitChannelMapping).
pub fn channel_mapping(mode: ChannelMode) -> ChannelMapping {
    let mut cm = ChannelMapping::default();
    assert_eq!(unsafe { c::cref_adj_thr_channel_mapping(mode, mp(&mut cm)) }, 0, "mode {mode}");
    cm
}

pub const MODES: [ChannelMode; 11] = [
    MODE_1,
    MODE_2,
    MODE_1_2,
    MODE_1_2_1,
    MODE_1_2_2,
    MODE_1_2_2_1,
    MODE_1_2_2_2_1,
    MODE_6_1,
    MODE_7_1_BACK,
    MODE_7_1_TOP_FRONT,
    MODE_7_1_REAR_SURROUND,
];

#[test]
fn adj_thr_new_init() {
    let mut rng = Rng::new(18);
    for &mode in MODES.iter().chain([MODE_7_1_FRONT_CENTER].iter()) {
        let cm = channel_mapping(mode);
        for _ in 0..200 {
            let mean_pe = rng.range(1, 20000);
            let inv = [0, 2][rng.range(0, 1) as usize];
            let sr = [8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 96000][rng.range(0, 8) as usize];
            let br = rng.range(8000, 800_000);
            let ld = rng.range(0, 1);
            let brm = rng.range(0, 2);
            let dz = rng.range(0, 1);
            let bdm = rng.range(0, 2);
            let vbr = rng.i32();
            let mut st = AdjThrState::default();
            assert_eq!(fdk_aac_enc_adj_thr_new(&mut st, cm.n_elements), 0);
            fdk_aac_enc_adj_thr_init(&mut st, mean_pe, inv, &cm, sr, br, ld, brm, dz, bdm, vbr);
            let mut bres = [BresParam::default(); 2];
            let mut ats = vec![AtsElement::default(); cm.n_elements as usize];
            let mut mi = [0i32; 2];
            let err = unsafe {
                c::cref_adj_thr_AdjThrInit(bres.as_mut_ptr() as *mut c_void, ats.as_mut_ptr() as *mut c_void, mi.as_mut_ptr(), cm.n_elements, mean_pe, inv, vp(&cm), sr, br, ld, brm, dz, bdm, vbr)
            };
            assert_eq!(err, 0);
            assert_eq!(st.bres_param_long, bres[0]);
            assert_eq!(st.bres_param_short, bres[1]);
            assert_eq!(st.adj_thr_state_elem, ats);
            assert_eq!([st.bit_distribution_mode, st.max_iter_2nd_guess], mi);
        }
    }
}

#[test]
fn calc_pe_correction() {
    let mut rng = Rng::new(19);
    for k in 0..300_000 {
        let real = k % 3 != 0;
        let mut m0 = if real { rng.range(0x3000_0000, 0x4a00_0000) } else { rng.i32() };
        let mut e0 = rng.range(0, 3);
        let pe_last = if real { rng.range(0, 8000) } else { rng.i32() };
        let pe_act = if real { pe_last + rng.range(-pe_last, pe_last) } else { rng.i32() };
        let bits_last = if real { rng.range(-1, 9000) } else { rng.i32() };
        let bm = if real { rng.range(0x0800_0000, 0x6000_0000) } else { rng.i32() };
        let be = if real { rng.range(1, 3) } else { rng.range(-5, 20) };
        let (mut m1, mut e1) = (m0, e0);
        // Inputs without a defined x86 reference result are skipped.
        let div0 = fdk_aac_enc_bits2pe2(bits_last, bm, be) == 0 && pe_last != 0;
        fdk_aac_enc_calc_pe_correction(&mut m0, &mut e0, pe_act, pe_last, bits_last, bm, be);
        if !div0 {
            unsafe { c::cref_adj_thr_calcPeCorrection(&mut m1, &mut e1, pe_act, pe_last, bits_last, bm, be) };
            assert_eq!((m0, e0), (m1, e1), "{pe_act} {pe_last} {bits_last} {bm:#x} {be}");
        }

        let mut m0 = if real { rng.range(0x3000_0000, 0x4000_0000) } else { rng.i32() };
        let (mut m1, mut e0, mut e1) = (m0, 0, 0);
        let lvl = if real { rng.range(0, 30000) } else { rng.i32() };
        let n = rng.range(1, 2);
        let headroom = fdk_aac_enc_bits2pe2(if lvl >= 50 * n { 0 } else { 100 * n }, bm, be);
        let div0 = fdk_aac_enc_bits2pe2(lvl, bm, be).wrapping_add(headroom) == 0;
        fdk_aac_enc_calc_pe_correction_low_bit_res(&mut m0, &mut e0, pe_last, bits_last, lvl, n, bm, be);
        if !div0 {
            unsafe { c::cref_adj_thr_calcPeCorrectionLowBitRes(&mut m1, &mut e1, pe_last, bits_last, lvl, n, bm, be) };
            assert_eq!((m0, e0), (m1, e1), "low {pe_last} {bits_last} {lvl} {bm:#x} {be}");
        }
    }
}

#[test]
fn distribute_bits() {
    let mut rng = Rng::new(20);
    for f in FLAVOURS {
        for _ in 0..iters() * 4 {
            let n = rng.range(1, 2) as usize;
            let (psy, _, _, mut pe0, ats) = gen_element_pe(&mut rng, n, f);
            let bres = gen_bres(&mut rng, f);
            let mut st = AdjThrState::default();
            st.bres_param_long = bres[0];
            st.bres_param_short = bres[1];
            st.adj_thr_state_elem = vec![ats];
            let mut ats1 = ats;
            let mut pe1 = pe0;
            let cw = rng.range(0, 1);
            let avg = if rng.range(0, 7) == 0 { rng.range(-10, 1) } else { rng.range(1, 12000) };
            let max_bitres = rng.range(0, 6144 * 8);
            let bitres = rng.range(0, max_bitres);
            let mbf = rng.range(0x0100_0000, 0x0800_0000);
            let brm = rng.range(0, 2);
            let (mut gp, mut gpc) = (0, 0);
            let mut o = [0i32; 2];
            fdk_aac_enc_distribute_bits(&mut st, 0, &psy, &mut pe0, &mut gp, &mut gpc, n as i32, cw, avg, bitres, max_bitres, mbf, brm);
            unsafe {
                c::cref_adj_thr_DistributeBits(bres.as_ptr() as *const c_void, mp(&mut ats1), psy_ptr(&psy), mp(&mut pe1), o.as_mut_ptr(), n as i32, cw, avg, bitres, max_bitres, mbf, brm)
            };
            assert_eq!([gp, gpc], o);
            assert_eq!(st.adj_thr_state_elem[0], ats1);
            assert_eq!(pe0, pe1);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Multi-element functions
// ---------------------------------------------------------------------------------------------

/// The state of the multi-element functions: channel mapping, QC_OUT, PSY_OUT and the
/// ATS_ELEMENTs.
#[derive(Clone)]
pub struct Multi {
    pub cm: ChannelMapping,
    pub qc_out: QcOut,
    pub psy_out: PsyOut,
    pub ats: Vec<AtsElement>,
}

pub fn gen_multi(rng: &mut Rng, mode: ChannelMode, f: Flavour) -> Multi {
    let cm = channel_mapping(mode);
    let mut qc_out = QcOut::default();
    let mut psy_out = PsyOut::default();
    let mut ats = Vec::new();
    for i in 0..cm.n_elements as usize {
        let n = cm.el_info[i].n_channels_in_el as usize;
        let (psy, qc, tools, pe, a) = gen_element_pe(rng, n, f);
        let start = qc_out.p_qc_out_channels.len();
        qc_out.qc_element.push(QcOutElement {
            static_bits_used: rng.range(0, 400),
            dyn_bits_used: rng.range(0, 4000),
            ext_bits_used: rng.range(0, 100),
            granted_pe: rng.range(0, 6000),
            granted_pe_corr: rng.range(0, 6000),
            pe_data: pe,
            qc_out_channel: [start, start + 1],
            ..Default::default()
        });
        psy_out.psy_out_element.push(PsyOutElement {
            psy_out_channel: [start, start + 1],
            common_window: (n == 2) as i32,
            tools_info: tools,
        });
        qc_out.p_qc_out_channels.extend(qc);
        psy_out.p_psy_out_channels.extend(psy);
        ats.push(a);
    }
    qc_out.total_no_red_pe = qc_out.qc_element.iter().map(|e| e.pe_data.pe).sum();
    qc_out.total_granted_pe_corr = rng.range(0, qc_out.total_no_red_pe.max(1) * 2);
    Multi { cm, qc_out, psy_out, ats }
}

/// Runs a multi-element C function on a copy of `m` and returns the copy.
fn run_c(m: &Multi, f: impl FnOnce(&Multi, *mut c_void, *mut c_void, *mut c_void, *mut i32)) -> Multi {
    let mut r = m.clone();
    let mut pe: Vec<PeData> = r.qc_out.qc_element.iter().map(|e| e.pe_data).collect();
    let tools: Vec<ToolsInfo> = r.psy_out.psy_out_element.iter().map(|e| e.tools_info).collect();
    let mut sc: Vec<i32> = r
        .qc_out
        .qc_element
        .iter()
        .zip(r.psy_out.psy_out_element.iter())
        .flat_map(|(q, p)| {
            [q.static_bits_used, q.ext_bits_used, q.granted_pe, q.granted_pe_corr, p.common_window]
        })
        .collect();
    let qcp = r.qc_out.p_qc_out_channels.as_mut_ptr() as *mut c_void;
    f(m, qcp, pe.as_mut_ptr() as *mut c_void, tools.as_ptr() as *mut c_void, sc.as_mut_ptr());
    for (i, e) in r.qc_out.qc_element.iter_mut().enumerate() {
        e.pe_data = pe[i];
        e.static_bits_used = sc[5 * i];
        e.ext_bits_used = sc[5 * i + 1];
        e.granted_pe = sc[5 * i + 2];
        e.granted_pe_corr = sc[5 * i + 3];
    }
    r
}

fn assert_multi_eq(r: &Multi, c: &Multi, what: &str) {
    assert_qc_eq(&r.qc_out.p_qc_out_channels, &c.qc_out.p_qc_out_channels, what);
    for (i, (a, b)) in r.qc_out.qc_element.iter().zip(c.qc_out.qc_element.iter()).enumerate() {
        assert_eq!(a.pe_data, b.pe_data, "{what}: element {i} PE_DATA");
    }
    assert_eq!(r.qc_out, c.qc_out, "{what}: QC_OUT");
    assert_eq!(r.ats, c.ats, "{what}: ATS_ELEMENT");
}

/// An AdjThrDynMem with every byte `b` (as the poisoned C buffer).
fn dyn_mem_of(b: u8) -> Box<AdjThrDynMem> {
    // SAFETY: AdjThrDynMem consists of integer arrays only.
    Box::new(unsafe { std::mem::transmute::<[u8; size_of::<AdjThrDynMem>()], AdjThrDynMem>([b; size_of::<AdjThrDynMem>()]) })
}

fn dyn_bytes(d: &AdjThrDynMem) -> Vec<u8> {
    // SAFETY: as in dyn_mem_of.
    unsafe { std::slice::from_raw_parts(d as *const AdjThrDynMem as *const u8, size_of::<AdjThrDynMem>()) }.to_vec()
}

#[test]
fn dyn_mem_size() {
    assert_eq!(unsafe { c::cref_adj_thr_dyn_size() } as usize, size_of::<AdjThrDynMem>());
}

fn gen_ah8(rng: &mut Rng) -> [Ah; 8] {
    std::array::from_fn(|_| gen_ah(rng))
}

#[test]
fn correct_thresh() {
    let mut rng = Rng::new(21);
    for f in FLAVOURS {
        for _ in 0..iters() / 2 {
            let mode = MODES[rng.range(0, MODES.len() as i32 - 1) as usize];
            let mut m = gen_multi(&mut rng, mode, f);
            let ne = m.cm.n_elements;
            let (off, proc_) = if rng.range(0, 1) == 0 { (0, ne) } else { (rng.range(0, ne - 1), 1) };
            let mut te = [[[0; MAX_GROUPED_SFB]; 2]; 8];
            for i in 0..ne as usize {
                let s = m.qc_out.qc_element[i].qc_out_channel[0];
                let n = m.cm.el_info[i].n_channels_in_el as usize;
                te[i] = gen_te(&mut rng, f, &m.qc_out.p_qc_out_channels[s..s + n], &m.psy_out.p_psy_out_channels[s..s + n]);
            }
            let mut a0 = gen_ah8(&mut rng);
            let red_m = if f == Flavour::Random { rng.i32() } else { rng.range(0, 0x7fff_ffff) };
            let red_e = rng.range(-20, 20);
            // The library calls FDKaacEnc_correctThresh() only with redPeGlobal > desiredPe, and
            // compiles it for deltaPe < 0 only (inlined into FDKaacEnc_adaptThresholdsToPe).
            let delta = rng.range(-3000, -1);
            let mut outs = Vec::new();
            for poison in [0x00, 0xA5] {
                let mut a1 = a0;
                let cm = m.cm;
                let mut dyn_c = vec![0u8; size_of::<AdjThrDynMem>()];
                let r = run_c(&m, |_, qcp, pe, tools, sc| unsafe {
                    c::cref_adj_thr_correctThresh(vp(&cm), qcp, psy_ptr(&m.psy_out.p_psy_out_channels), pe, tools, sc, a1.as_mut_ptr() as *mut u8, te.as_ptr() as *const i32, red_m, red_e, delta, proc_, off, poison, dyn_c.as_mut_ptr())
                });
                outs.push((r, a1, dyn_c));
            }
            assert_multi_eq(&outs[0].0, &outs[1].0, "correctThresh poison");
            let mut d = dyn_mem_of(0);
            fdk_aac_enc_correct_thresh(&m.cm, &mut m.qc_out, &m.psy_out, &mut a0, &te, red_m, red_e as i8, delta, proc_, off, &mut d.sfb_n_active_lines_ld_data);
            assert_multi_eq(&m, &outs[0].0, "correctThresh");
            assert!(dyn_bytes(&d) == outs[0].2, "correctThresh: dynMem");
            assert_eq!(a0, outs[0].1);
        }
    }
}

#[test]
fn reduce_min_snr() {
    let mut rng = Rng::new(22);
    for f in FLAVOURS {
        for _ in 0..iters() / 2 {
            let mode = MODES[rng.range(0, MODES.len() as i32 - 1) as usize];
            let mut m = gen_multi(&mut rng, mode, f);
            let ne = m.cm.n_elements;
            let (off, proc_) = if rng.range(0, 1) == 0 { (0, ne) } else { (rng.range(0, ne - 1), 1) };
            let a = gen_ah8(&mut rng);
            let red: i32 = m.qc_out.qc_element.iter().map(|e| e.pe_data.pe).sum();
            let desired = red - rng.range(-100, red.abs() + 100);
            let (mut r0, mut r1) = (red, red);
            let cm = m.cm;
            let r = run_c(&m, |_, qcp, pe, tools, sc| unsafe {
                c::cref_adj_thr_reduceMinSnr(vp(&cm), qcp, psy_ptr(&m.psy_out.p_psy_out_channels), pe, tools, sc, a.as_ptr() as *const u8, desired, &mut r1, proc_, off, std::ptr::null_mut())
            });
            fdk_aac_enc_reduce_min_snr(&m.cm, &mut m.qc_out, &m.psy_out, &a, desired, &mut r0, proc_, off);
            assert_multi_eq(&m, &r, "reduceMinSnr");
            assert_eq!(r0, r1);
        }
    }
}

#[test]
fn allow_more_holes() {
    let mut rng = Rng::new(23);
    for f in FLAVOURS {
        for _ in 0..iters() / 2 {
            let mode = MODES[rng.range(0, MODES.len() as i32 - 1) as usize];
            let mut m = gen_multi(&mut rng, mode, f);
            let ne = m.cm.n_elements;
            let (off, proc_) = if rng.range(0, 1) == 0 { (0, ne) } else { (rng.range(0, ne - 1), 1) };
            let mut a0 = gen_ah8(&mut rng);
            let mut a1 = a0;
            let cur: i32 = m.qc_out.qc_element.iter().map(|e| e.pe_data.pe).sum();
            let desired = cur - rng.range(-100, cur.abs() + 100);
            let cm = m.cm;
            let ats = m.ats.clone();
            let r = run_c(&m, |_, qcp, pe, tools, sc| unsafe {
                c::cref_adj_thr_allowMoreHoles(vp(&cm), qcp, psy_ptr(&m.psy_out.p_psy_out_channels), pe, tools, sc, ats.as_ptr() as *const c_void, a1.as_mut_ptr() as *mut u8, desired, cur, proc_, off, std::ptr::null_mut())
            });
            fdk_aac_enc_allow_more_holes(&m.cm, &mut m.qc_out, &m.psy_out, &ats, &mut a0, desired, cur, proc_, off);
            assert_multi_eq(&m, &r, "allowMoreHoles");
            assert_eq!(a0, a1);
        }
    }
}

#[test]
fn adapt_thresholds_to_pe() {
    let mut rng = Rng::new(24);
    for f in FLAVOURS {
        for _ in 0..iters() / 2 {
            let mode = MODES[rng.range(0, MODES.len() as i32 - 1) as usize];
            let mut m = gen_multi(&mut rng, mode, f);
            let ne = m.cm.n_elements;
            let (off, proc_) = if rng.range(0, 1) == 0 { (0, ne) } else { (rng.range(0, ne - 1), 1) };
            let cur: i32 = m.qc_out.qc_element.iter().map(|e| e.pe_data.pe).sum();
            let desired = (cur - rng.range(-100, cur.abs() + 100)).max(1);
            let iter = rng.range(1, 3);
            let mut outs = Vec::new();
            for poison in [0x00, 0xFF] {
                let cm = m.cm;
                let mut ats = m.ats.clone();
                let mut dyn_c = vec![0u8; size_of::<AdjThrDynMem>()];
                let r = run_c(&m, |_, qcp, pe, tools, sc| unsafe {
                    c::cref_adj_thr_adaptThresholdsToPe(vp(&cm), qcp, psy_ptr(&m.psy_out.p_psy_out_channels), pe, tools, sc, ats.as_mut_ptr() as *mut c_void, desired, iter, proc_, off, poison, dyn_c.as_mut_ptr())
                });
                outs.push((r, dyn_c));
            }
            assert_multi_eq(&outs[0].0, &outs[1].0, "adaptThresholdsToPe poison");
            let mut d = dyn_mem_of(0xFF);
            fdk_aac_enc_adapt_thresholds_to_pe(&m.cm, &m.ats, &mut m.qc_out, &m.psy_out, desired, iter, proc_, off, &mut d);
            assert_multi_eq(&m, &outs[0].0, "adaptThresholdsToPe");
            assert!(dyn_bytes(&d) == outs[1].1, "adaptThresholdsToPe: dynMem");
        }
    }
}

#[test]
fn adjust_thresholds() {
    // The library function: covers the static functions as the library compiles them (inlined
    // into FDKaacEnc_adaptThresholdsToPe() / FDKaacEnc_AdjustThresholds()).
    let mut rng = Rng::new(25);
    for f in FLAVOURS {
        for _ in 0..iters() * 4 {
            let mode = MODES[rng.range(0, MODES.len() as i32 - 1) as usize];
            let mut m = gen_multi(&mut rng, mode, f);
            // desired pe: a fraction of the pe (reaches the 2nd guess, correctThresh,
            // reduceMinSnr and allowMoreHoles)
            let frac = [5, 20, 50, 80, 95, 100, 120][rng.range(0, 6) as usize];
            // (inputs without a defined x86 reference result are left out)
            for e in m.qc_out.qc_element.iter_mut() {
                e.granted_pe_corr =
                    ((e.pe_data.pe as i64 * frac / 100) as i32 + rng.range(-20, 20)).max(1);
            }
            m.qc_out.total_granted_pe_corr =
                ((m.qc_out.total_no_red_pe as i64 * frac / 100) as i32 + rng.range(-20, 20)).max(1);
            let bres = gen_bres(&mut rng, f);
            let bdm = rng.range(0, 1);
            let iter = rng.range(1, 3);
            let cbr = rng.range(0, 2).min(1);
            let mut st = AdjThrState {
                bres_param_long: bres[0],
                bres_param_short: bres[1],
                adj_thr_state_elem: m.ats.clone(),
                bit_distribution_mode: bdm,
                max_iter_2nd_guess: iter,
            };
            let tot = [m.qc_out.total_granted_pe_corr, m.qc_out.total_no_red_pe];
            let mut outs = Vec::new();
            for poison in [0x00, 0x5A] {
                let cm = m.cm;
                let mut ats = m.ats.clone();
                let mut dyn_c = vec![0u8; size_of::<AdjThrDynMem>()];
                let mut r = run_c(&m, |_, qcp, pe, tools, sc| unsafe {
                    c::cref_adj_thr_AdjustThresholds(bres.as_ptr() as *const c_void, ats.as_mut_ptr() as *mut c_void, bdm, iter, vp(&cm), qcp, psy_ptr(&m.psy_out.p_psy_out_channels), pe, tools, sc, tot.as_ptr(), cbr, poison, dyn_c.as_mut_ptr())
                });
                r.ats = ats;
                outs.push((r, dyn_c));
            }
            assert_multi_eq(&outs[0].0, &outs[1].0, "AdjustThresholds poison");
            let mut psy = m.psy_out.clone();
            let mut d = dyn_mem_of(0x5A);
            fdk_aac_enc_adjust_thresholds_dyn_mem(&mut st, &mut m.qc_out, &mut psy, cbr, &m.cm, &mut d);
            assert!(dyn_bytes(&d) == outs[1].1, "AdjustThresholds: dynMem");
            assert_eq!(psy, m.psy_out);
            m.ats = st.adj_thr_state_elem.clone();
            assert_multi_eq(&m, &outs[0].0, &format!("AdjustThresholds cbr={cbr} bdm={bdm}"));
        }
    }
}

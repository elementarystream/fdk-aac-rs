// Differential tests of fdk_aac_rs_aacenc::aacenc_tns (and the tns_func.h functions) against the
// C++ reference.
//
// The TNS configurations come from the real init chain (FDKaacEnc_InitPsyConfiguration, then
// FDKaacEnc_InitTnsConfiguration) for all sample rates, frame lengths, filter banks and block
// types. TnsDetect, TnsSync and TnsEncode run on MDCT spectra of real PCM (long and short
// blocks, LD) and on random spectra with edge values; after every call the complete TNS_DATA,
// TNS_INFO (copied from C into the port's #[repr(C)] structs) and the spectrum are compared.

use crate::aacenc_tns_spectra::spectra;
use crate::psy_configuration::{block_types, init_psy_checked, GRANULES, RATES};
use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc_tns::tables::*;
use fdk_aac_rs_aacenc::aacenc_tns::*;
use fdk_aac_rs_aacenc::psy_configuration::PsyConfiguration;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_cref::aacenc_aacenc_tns as c;
use fdk_aac_rs_cref::testutil::Rng;
use std::ffi::CStr;

fn vp<T>(t: &mut T) -> *mut c_void {
    t as *mut T as *mut c_void
}

fn cvp<T>(t: &T) -> *const c_void {
    t as *const T as *const c_void
}

fn bytes<T>(t: &T) -> &[u8] {
    // SAFETY: the compared structs consist of integers (padding is explicit).
    unsafe { core::slice::from_raw_parts(t as *const T as *const u8, size_of_val(t)) }
}

fn zeroed<T>() -> T {
    // SAFETY: only used for the integer-only C-layout structs.
    unsafe { core::mem::zeroed() }
}

/// A random spectrum value: edge values, small and large magnitudes.
fn spec_value(r: &mut Rng) -> i32 {
    match r.range(0, 9) {
        0 => [0, 1, -1, i32::MAX, i32::MIN, i32::MIN + 1, 0x4000_0000, -0x4000_0000][r.range(0, 7) as usize],
        1..=4 => r.i32() >> r.range(0, 31),
        5..=6 => r.i32() >> r.range(8, 20),
        _ => r.i32(),
    }
}

/// A random spectrum of n lines (with zero or saturated regions now and then).
fn random_spectrum(r: &mut Rng, n: usize) -> Vec<i32> {
    let mode = r.range(0, 9);
    let shift = r.range(0, 24);
    let mut v: Vec<i32> = (0..n)
        .map(|_| match mode {
            0 => 0,
            1 => [i32::MIN, i32::MAX][r.range(0, 1) as usize],
            2 => r.i32(),
            _ => spec_value(r) >> shift,
        })
        .collect();
    if r.range(0, 3) == 0 {
        // a zero region
        let a = r.range(0, n as i32 - 1) as usize;
        let b = r.range(a as i32, n as i32) as usize;
        v[a..b].fill(0);
    }
    v
}

#[test]
fn tables_match() {
    let n = unsafe { c::cref_aacenc_tns_table_count() };
    let mut seen = 0;
    for i in 0..n {
        let mut p = core::ptr::null();
        let mut len = 0;
        let name = unsafe { CStr::from_ptr(c::cref_aacenc_tns_table(i, &mut p, &mut len)) };
        let name = name.to_str().unwrap();
        let cb = unsafe { core::slice::from_raw_parts(p as *const u8, len as usize) };
        let rb = match name {
            "acfWindowLong" => bytes(&ACF_WINDOW_LONG),
            "acfWindowShort" => bytes(&ACF_WINDOW_SHORT),
            "tnsInfoTab" => bytes(&TNS_INFO_TAB),
            "tnsMaxBandsTab1024" => bytes(&TNS_MAX_BANDS_TAB_1024),
            "tnsMaxBandsTab120" => bytes(&TNS_MAX_BANDS_TAB_120),
            "tnsMaxBandsTab128" => bytes(&TNS_MAX_BANDS_TAB_128),
            "tnsMaxBandsTab240" => bytes(&TNS_MAX_BANDS_TAB_240),
            "tnsMaxBandsTab256" => bytes(&TNS_MAX_BANDS_TAB_256),
            "tnsMaxBandsTab480" => bytes(&TNS_MAX_BANDS_TAB_480),
            "tnsMaxBandsTab512" => bytes(&TNS_MAX_BANDS_TAB_512),
            _ => panic!("unknown table {name}"),
        };
        assert_eq!(rb, cb, "{name}");
        seen += 1;
    }
    assert_eq!(seen, 10);
}

/// All PSY_CONFIGURATIONs of the encoder (long and short blocks) for one bit rate.
fn psy_configs(bitrate: i32) -> Vec<(i32, i32, i32, PsyConfiguration)> {
    let mut v = Vec::new();
    for &sr in &RATES {
        for &(gl, fb) in &GRANULES {
            for &bt in block_types(gl) {
                if bt == START_WINDOW || bt == STOP_WINDOW {
                    continue;
                }
                let (_, p) = init_psy_checked(bitrate, sr, sr / 2, bt, gl, 1, 1, fb);
                v.push((sr, gl, bt, p));
                let (_, p) = init_psy_checked(bitrate, sr, 0x3fff & (sr / 3), bt, gl, 1, 1, fb);
                v.push((sr, gl, bt, p));
            }
        }
    }
    v
}

#[test]
fn freq_to_band_width_rounding() {
    let mut r = Rng::new(0x7a50_0001);
    for (sr, _, _, p) in psy_configs(64000) {
        for _ in 0..200 {
            let freq = match r.range(0, 2) {
                0 => r.range(0, sr),
                1 => r.range(0, 30000),
                _ => r.range(-1000, 2 * sr),
            };
            let n = r.range(0, p.sfb_cnt);
            let fs = [sr, sr * 2, r.range(1, 400_000)][r.range(0, 2) as usize];
            let cc = unsafe {
                c::cref_aacenc_tns_FreqToBandWidthRounding(freq, fs, n, p.sfb_offset.as_ptr())
            };
            let rr = fdk_aac_enc_freq_to_band_width_rounding(freq, fs, n, &p.sfb_offset);
            assert_eq!(rr, cc, "freq {freq} fs {fs} n {n}");
        }
    }
}

#[test]
fn get_tns_param_and_max_bands() {
    for br in (-2000..400_000).step_by(499).chain([16000, 32000, 32001, 60000, 60001, 384000, 384001, 13500, 28000, 28001, 52000, 52001]) {
        for ch in -1..4 {
            for sbr in [0, 1, 7] {
                let mut cp: TnsParameterTabulated = zeroed();
                let found = unsafe { c::cref_aacenc_tns_GetTnsParam(br, ch, sbr, vp(&mut cp)) };
                let rp = fdk_aac_enc_get_tns_param(br, ch, sbr);
                assert_eq!(rp.is_some() as i32, found, "br {br} ch {ch} sbr {sbr}");
                if let Some(rp) = rp {
                    assert_eq!(*rp, cp, "br {br} ch {ch} sbr {sbr}");
                }
            }
        }
    }
    for sr in (0..200_000).step_by(997).chain(RATES).chain([22049, 22050, 22051, 9390, 9391, 0, -5]) {
        for gl in [1024, 960, 512, 480, 256, 240, 128, 120, 0, 2048] {
            for short in [0, 1, 2] {
                let cc = unsafe { c::cref_aacenc_tns_getTnsMaxBands(sr, gl, short) };
                assert_eq!(get_tns_max_bands(sr, gl, short), cc, "sr {sr} gl {gl} short {short}");
            }
        }
    }
}

/// Runs FDKaacEnc_InitTnsConfiguration on C and Rust copies of `p` (`tC = &pC->tnsConf`, as the
/// encoder calls it) and compares the results; returns the port's configuration.
fn init_tns_checked(
    p: &PsyConfiguration,
    a: (i32, i32, i32, i32, i32, i32, i32, i32, i32),
) -> (i32, PsyConfiguration) {
    let (br, sr, ch, bt, gl, ld, sbr, act, peak) = a;
    let mut pc = *p;
    let pcp = vp(&mut pc);
    let tcp = vp(&mut pc.tns_conf);
    let ec = unsafe {
        c::cref_aacenc_tns_InitTnsConfiguration(br, sr, ch, bt, gl, ld, sbr, tcp, pcp, act, peak)
    };
    let mut pr = *p;
    let mut tc = pr.tns_conf;
    let er = fdk_aac_enc_init_tns_configuration(br, sr, ch, bt, gl, ld, sbr, &mut tc, &pr, act, peak);
    pr.tns_conf = tc;
    assert_eq!(er, ec, "{a:?}: return value");
    assert_eq!(pr, pc, "{a:?}: PSY_CONFIGURATION / TNS_CONFIG");
    (er, pr)
}

#[test]
fn init_tns_configuration() {
    let mut r = Rng::new(0x7a50_0002);
    let mut n = 0;
    for &br in &[8000, 15999, 16000, 32000, 48000, 64000, 128000, 320000] {
        for (sr, gl, bt, p) in psy_configs(br) {
            for ch in [0, 1, 2] {
                for ld in [0, 1] {
                    for sbr in [0, 1] {
                        let tns_br = if r.range(0, 1) == 0 { br * ch.max(1) } else { r.range(0, 500_000) };
                        let act = r.range(0, 1) * 2;
                        let mut p0 = p;
                        if r.range(0, 1) == 0 {
                            // stale contents of the configuration (the short acfWindow copy
                            // leaves the upper half unchanged)
                            for w in p0.tns_conf.acf_window.iter_mut().flatten() {
                                *w = r.i32();
                            }
                            p0.tns_conf.conf_tab.filter_start_freq = [r.i32(), r.i32()];
                        }
                        init_tns_checked(&p0, (tns_br, sr, ch, bt, gl, ld, sbr, act, 8));
                        n += 1;
                    }
                }
            }
        }
    }
    assert!(n > 10000);
}

#[test]
fn scale_up_spectrum_and_auto_corr() {
    let mut r = Rng::new(0x7a50_0003);
    for _ in 0..20_000 {
        let n = 1024;
        let src = random_spectrum(&mut r, n);
        let start = r.range(0, n as i32);
        let stop = r.range(start.max(0) - 4, n as i32);
        let (mut dc, mut dr) = (vec![0x5a5a_5a5ai32; n], vec![0x5a5a_5a5ai32; n]);
        let sc = unsafe { c::cref_aacenc_tns_ScaleUpSpectrum(dc.as_mut_ptr(), src.as_ptr(), start, stop) };
        let sr = fdk_aac_enc_scale_up_spectrum(&mut dr, &src, start, stop);
        assert_eq!((sr, &dr), (sc, &dc), "ScaleUpSpectrum {start}..{stop}");

        let lag = r.range(0, 13);
        let scale = r.range(0, 12);
        let cc = unsafe { c::cref_aacenc_tns_CalcAutoCorrValue(dc.as_ptr(), start, stop, lag, scale) };
        let rr = fdk_aac_enc_calc_auto_corr_value(&dr, start, stop, lag, scale);
        assert_eq!(rr, cc, "CalcAutoCorrValue {start}..{stop} lag {lag} scale {scale}");
    }
    for i in 0..200_000 {
        let value = if i % 2 == 0 { spec_value(&mut r).wrapping_abs() } else { r.i32() };
        let scale = match r.range(0, 2) {
            0 => r.range(-62, 12),
            1 => r.range(-40, 40),
            _ => r.i32() >> r.range(0, 31),
        };
        let sc0 = r.range(-5, 5);
        let (mut scc, mut scr) = (sc0, sc0);
        let cc = unsafe { c::cref_aacenc_tns_AutoCorrNormFac(value, scale, &mut scc) };
        let rr = fdk_aac_enc_auto_corr_norm_fac(value, scale, &mut scr);
        assert_eq!((rr, scr), (cc, scc), "AutoCorrNormFac {value:#x} scale {scale}");
    }
}

/// The TNS configurations of the encoder (LC long/short, LD, ELD) for a few bit rates.
fn tns_configs() -> Vec<(i32, i32, i32, PsyConfiguration)> {
    let mut v = Vec::new();
    for &br in &[12000, 24000, 48000, 64000, 128000] {
        for (sr, gl, bt, p) in psy_configs(br) {
            if !(gl == 1024 || gl == 512 || gl == 480) {
                continue;
            }
            let ld = (gl <= 512) as i32;
            for (ch, sbr) in [(1, 0), (2, 0), (2, 1)] {
                let (e, p) = init_tns_checked(&p, (br * ch, sr, ch, bt, gl, ld, sbr, 2, 0));
                if e == 0 && p.tns_conf.tns_active != 0 {
                    v.push((sr, gl, bt, p));
                }
            }
        }
    }
    v
}

#[test]
fn merged_auto_correlation() {
    let mut r = Rng::new(0x7a50_0004);
    let configs = tns_configs();
    assert!(configs.len() > 200);
    for (k, (_, gl, bt, p)) in configs.iter().enumerate() {
        let t = &p.tns_conf;
        let n = if *bt == SHORT_WINDOW { (*gl / 8) as usize } else { *gl as usize };
        let frames = if k % 4 == 0 { spectra(&mut r, k, *gl, *gl <= 512, 2) } else { Vec::new() };
        for it in 0..6 {
            let spec = match frames.get(it) {
                Some(f) if f.block_type == *bt || *bt != SHORT_WINDOW => {
                    f.spectrum[..n].iter().map(|&x| x >> r.range(0, 4)).collect()
                }
                _ => random_spectrum(&mut r, n),
            };
            let mut t = *t;
            if it == 5 {
                // other splits and orders
                t.max_order = r.range(1, 12);
                t.is_low_delay = r.range(0, 1);
                t.conf_tab.acf_split = [[-1, -1], [3, 1], [-1, 1], [3, -1]][r.range(0, 3) as usize];
            }
            let fill = r.i32();
            let (mut c1, mut c2) = ([fill; 13], [fill; 13]);
            let (mut r1, mut r2) = ([fill; 13], [fill; 13]);
            unsafe {
                c::cref_aacenc_tns_MergedAutoCorrelation(
                    spec.as_ptr(),
                    t.is_low_delay,
                    t.acf_window.as_ptr() as *const i32,
                    t.lpc_start_line.as_ptr(),
                    t.lpc_stop_line,
                    t.max_order,
                    t.conf_tab.acf_split.as_ptr(),
                    c1.as_mut_ptr(),
                    c2.as_mut_ptr(),
                )
            };
            fdk_aac_enc_merged_auto_correlation(
                &spec,
                t.is_low_delay,
                &t.acf_window,
                &t.lpc_start_line,
                t.lpc_stop_line,
                t.max_order,
                &t.conf_tab.acf_split,
                &mut r1,
                &mut r2,
            );
            assert_eq!((r1, r2), (c1, c2), "config {k} it {it}");
        }
    }
}

#[test]
fn calc_gauss_window() {
    let mut r = Rng::new(0x7a50_0005);
    let check = |r: &mut Rng, size: i32, sr: i32, res: i32, tr: i32, tre: i32| {
        let fill = r.i32();
        let (mut wc, mut wr) = ([fill; 16], [fill; 16]);
        unsafe { c::cref_aacenc_tns_CalcGaussWindow(wc.as_mut_ptr(), size, sr, res, tr, tre) };
        fdk_aac_enc_calc_gauss_window(&mut wr, size, sr, res, tr, tre);
        assert_eq!(wr, wc, "size {size} sr {sr} res {res} tr {tr:#x} e {tre}");
    };
    for e in TNS_INFO_TAB.iter() {
        for pt in e.param_tab.iter() {
            for &tr in &pt.tns_time_resolution {
                for &sr in &RATES {
                    for res in [480, 512, 1024, 240, 256, 120, 128] {
                        for size in [1, 4, 6, 9, 11, 13, 15] {
                            check(&mut r, size, sr, res, tr, 1);
                        }
                    }
                }
            }
        }
    }
    for _ in 0..20_000 {
        let size = r.range(0, 15);
        let sr = r.range(1, 400_000);
        let res = r.range(1, 2048);
        let tr = r.range(1, i32::MAX);
        let tre = r.range(-3, 4);
        check(&mut r, size, sr, res, tr, tre);
    }
}

#[test]
fn search_and_index() {
    for p in i16::MIN..=i16::MAX {
        assert_eq!(fdk_aac_enc_search3(p), unsafe { c::cref_aacenc_tns_Search3(p) }, "{p}");
        assert_eq!(fdk_aac_enc_search4(p), unsafe { c::cref_aacenc_tns_Search4(p) }, "{p}");
    }
    let mut r = Rng::new(0x7a50_0006);
    for _ in 0..20_000 {
        let order = r.range(0, 12);
        let bits = [3, 4, 5][r.range(0, 2) as usize];
        let parcor: Vec<i16> = (0..12).map(|_| r.i16()).collect();
        let fill = r.i32();
        let (mut ic, mut ir) = ([fill; 12], [fill; 12]);
        unsafe { c::cref_aacenc_tns_Parcor2Index(parcor.as_ptr(), ic.as_mut_ptr(), order, bits) };
        fdk_aac_enc_parcor2_index(&parcor, &mut ir, order, bits);
        assert_eq!(ir, ic);
        // Index2Parcor needs indices of the resolution
        let bits = [3, 4][r.range(0, 1) as usize];
        let idx: Vec<i32> =
            (0..12).map(|_| if bits == 4 { r.range(-8, 7) } else { r.range(-4, 3) }).collect();
        let fill = r.i16();
        let (mut pc, mut pr) = ([fill; 12], [fill; 12]);
        unsafe { c::cref_aacenc_tns_Index2Parcor(idx.as_ptr(), pc.as_mut_ptr(), order, bits) };
        fdk_aac_enc_index2_parcor(&idx, &mut pr, order, bits);
        assert_eq!(pr, pc);
    }
}

/// The TNS state of one channel, in C and in the port.
#[derive(Clone)]
struct Chan {
    dc: TnsData,
    ic: TnsInfo,
    dr: TnsData,
    ir: TnsInfo,
}

impl Chan {
    fn new(r: &mut Rng) -> Self {
        // stale contents of earlier frames
        let mut d: TnsData = zeroed();
        let mut i: TnsInfo = zeroed();
        if r.range(0, 1) == 0 {
            for w in d.data_raw.0.iter_mut() {
                *w = r.range(-3, 3);
            }
            for c in i.coef.iter_mut().flatten().flatten() {
                *c = r.range(-8, 7);
            }
            for x in i.coef_compress.iter_mut().flatten() {
                *x = r.range(0, 1);
            }
        }
        Chan { dc: d, ic: i, dr: d, ir: i }
    }

    fn check(&self, what: &str) {
        assert_eq!(self.dr, self.dc, "{what}: TNS_DATA");
        assert_eq!(self.ir, self.ic, "{what}: TNS_INFO");
    }

    fn detect(&mut self, t: &TnsConfig, sfb_cnt: i32, spec: &[i32], w: i32, bt: i32, what: &str) {
        let ec = unsafe {
            c::cref_aacenc_tns_TnsDetect(vp(&mut self.dc), cvp(t), vp(&mut self.ic), sfb_cnt, spec.as_ptr(), w, bt)
        };
        let er = fdk_aac_enc_tns_detect(&mut self.dr, t, &mut self.ir, sfb_cnt, spec, w, bt);
        assert_eq!(er, ec, "{what}: TnsDetect return value");
        self.check(&format!("{what}: TnsDetect"));
    }
}

/// TnsDetect for every window, TnsSync, TnsEncode for every window of a stereo frame, as
/// FDKaacEnc_psyMain calls them; compares all state and the spectra.
fn run_frame(
    ch: &mut [Chan; 2],
    p: &PsyConfiguration,
    gl: i32,
    bt: i32,
    spec: &mut [Vec<i32>; 2],
    what: &str,
) {
    let t = &p.tns_conf;
    let (n_windows, wlen) = if bt == SHORT_WINDOW { (8, gl / 8) } else { (1, gl) };
    for c in 0..2 {
        for w in 0..n_windows {
            let off = (w * wlen) as usize;
            ch[c].detect(t, p.sfb_cnt, &spec[c][off..], w, bt, &format!("{what} ch {c} w {w}"));
        }
    }
    // TnsSync(tnsData[1], tnsData[0], ...)
    {
        let (a, b) = ch.split_at_mut(1);
        let (src, dst) = (&mut a[0], &mut b[0]);
        unsafe {
            c::cref_aacenc_tns_TnsSync(vp(&mut dst.dc), cvp(&src.dc), vp(&mut dst.ic), vp(&mut src.ic), bt, bt, cvp(t))
        };
        fdk_aac_enc_tns_sync(&mut dst.dr, &src.dr, &mut dst.ir, &src.ir, bt, bt, t);
        dst.check(&format!("{what}: TnsSync dest"));
        src.check(&format!("{what}: TnsSync src"));
    }
    for c in 0..2 {
        let mut sc = spec[c].clone();
        for w in 0..n_windows {
            let off = (w * wlen) as usize;
            let lp = p.sfb_offset[p.sfb_active as usize];
            let ec = unsafe {
                c::cref_aacenc_tns_TnsEncode(vp(&mut ch[c].ic), vp(&mut ch[c].dc), p.sfb_cnt, cvp(t), lp, sc[off..].as_mut_ptr(), w, bt)
            };
            let er = fdk_aac_enc_tns_encode(&ch[c].ir, &ch[c].dr, p.sfb_cnt, t, lp, &mut spec[c][off..], w, bt);
            assert_eq!(er, ec, "{what} ch {c} w {w}: TnsEncode return value");
        }
        assert_eq!(spec[c], sc, "{what} ch {c}: TnsEncode spectrum");
        ch[c].check(&format!("{what} ch {c}: TnsEncode"));
    }
}

#[test]
fn tns_realistic_spectra() {
    let mut r = Rng::new(0x7a50_0007);
    let configs = tns_configs();
    let mut active = [0usize; 5];
    let mut k = 0;
    for &(sr, gl) in &[(48000, 1024), (44100, 1024), (96000, 1024), (16000, 1024), (8000, 1024), (22050, 1024), (48000, 512), (44100, 480), (24000, 512), (32000, 480)] {
        let long: Vec<&PsyConfiguration> = configs.iter().filter(|c| c.0 == sr && c.1 == gl && c.2 == LONG_WINDOW).map(|c| &c.3).collect();
        let short: Vec<&PsyConfiguration> = configs.iter().filter(|c| c.0 == sr && c.1 == gl && c.2 == SHORT_WINDOW).map(|c| &c.3).collect();
        for _ in 0..12 {
            let pl = long[r.range(0, long.len() as i32 - 1) as usize];
            let ps = short.get(r.range(0, short.len().max(1) as i32 - 1) as usize).copied();
            let f0 = spectra(&mut r, k, gl, gl <= 512, 32);
            let f1 = spectra(&mut r, k + 1, gl, gl <= 512, 32);
            k += 1;
            let mut ch = [Chan::new(&mut r), Chan::new(&mut r)];
            for (i, fr) in f0.iter().enumerate() {
                let bt = fr.block_type;
                let p = if bt == SHORT_WINDOW { ps.unwrap() } else { pl };
                let sh = r.range(0, 3);
                // channel 1: similar to channel 0, so that the filters are synchronized
                let mix = r.range(0, 2);
                let s1: Vec<i32> = fr
                    .spectrum
                    .iter()
                    .zip(&f1[i].spectrum)
                    .map(|(&a, &b)| match mix {
                        0 => a >> 1,
                        1 => (a >> 1) + (b >> 3),
                        _ => b >> 1,
                    })
                    .collect();
                let mut spec = [fr.spectrum.iter().map(|&x| x >> sh).collect(), s1];
                run_frame(&mut ch, p, gl, bt, &mut spec, &format!("sr {sr} gl {gl} frame {i} bt {bt}"));
                for c in &ch {
                    let o = if bt == SHORT_WINDOW { TnsDataRaw::short_sub_block_info(0) } else { TnsDataRaw::LONG_SUB_BLOCK_INFO };
                    active[0] += 1;
                    active[1] += (c.dr.data_raw.tns_active(o, HIFILT) != 0) as usize;
                    active[2] += (c.dr.data_raw.tns_active(o, LOFILT) != 0) as usize;
                    active[3] += (c.dr.filters_merged != 0) as usize;
                    active[4] += (c.ir.num_of_filters[0] == 2) as usize;
                }
            }
        }
    }
    eprintln!("TNS (frames, HIFILT, LOFILT, merged, 2 filters): {active:?}, corpus {}", crate::aacenc_tns_spectra::work_dir().join("corpus").is_dir());
    // TNS must actually be exercised.
    assert!(active[1] > active[0] / 20, "{active:?}");
    assert!(active[2] > 0 && active[3] > 0 && active[4] > 0, "{active:?}");
}

#[test]
fn tns_random_spectra() {
    let mut r = Rng::new(0x7a50_0008);
    let configs = tns_configs();
    for it in 0..3000 {
        let (_, gl, bt, p) = &configs[r.range(0, configs.len() as i32 - 1) as usize];
        let mut p = *p;
        if it % 5 == 0 {
            p.tns_conf.max_order = r.range(0, 12);
            p.tns_conf.conf_tab.tns_limit_order = [r.range(0, 12), r.range(0, 12)];
            p.tns_conf.conf_tab.tns_filter_direction = [r.range(0, 1), r.range(0, 1)];
            p.tns_conf.conf_tab.thresh_on = [r.range(900, 2000), r.range(900, 2000)];
        }
        let mut ch = [Chan::new(&mut r), Chan::new(&mut r)];
        let s0 = random_spectrum(&mut r, *gl as usize);
        let s1 = if r.range(0, 1) == 0 { s0.iter().map(|&x| x >> 1).collect() } else { random_spectrum(&mut r, *gl as usize) };
        let mut spec = [s0, s1];
        run_frame(&mut ch, &p, *gl, *bt, &mut spec, &format!("it {it}"));
    }
}

/// TnsSync with different block types and random states.
#[test]
fn tns_sync_states() {
    let mut r = Rng::new(0x7a50_0009);
    let configs = tns_configs();
    for it in 0..20_000 {
        let (_, _, _, p) = &configs[r.range(0, configs.len() as i32 - 1) as usize];
        let t = &p.tns_conf;
        let mut a = Chan::new(&mut r);
        let mut b = Chan::new(&mut r);
        for c in [&mut a, &mut b] {
            for w in c.dc.data_raw.0.iter_mut() {
                *w = r.range(0, 1);
            }
            for x in c.ic.num_of_filters.iter_mut() {
                *x = r.range(0, 2);
            }
            for x in c.ic.order.iter_mut().flatten() {
                *x = r.range(0, 12);
            }
            for x in c.ic.coef.iter_mut().flatten().flatten() {
                *x = r.range(-2, 2);
            }
            c.dc.filters_merged = r.range(0, 1);
            c.dr = c.dc;
            c.ir = c.ic;
        }
        let bt_d = [LONG_WINDOW, SHORT_WINDOW, START_WINDOW][r.range(0, 2) as usize];
        let bt_s = if r.range(0, 3) == 0 { [LONG_WINDOW, SHORT_WINDOW][r.range(0, 1) as usize] } else { bt_d };
        unsafe {
            c::cref_aacenc_tns_TnsSync(vp(&mut a.dc), cvp(&b.dc), vp(&mut a.ic), vp(&mut b.ic), bt_d, bt_s, cvp(t))
        };
        fdk_aac_enc_tns_sync(&mut a.dr, &b.dr, &mut a.ir, &b.ir, bt_d, bt_s, t);
        a.check(&format!("it {it}: dest"));
        b.check(&format!("it {it}: src"));
    }
}

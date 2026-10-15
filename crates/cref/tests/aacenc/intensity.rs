// Differential tests of fdk_aac_rs_aacenc::intensity against the C++ reference.
//
// Inputs are random layouts and data, and realistic stereo frames: MDCT spectra (long and
// grouped short blocks, through the ported mdct_block) of real stereo PCM from the corpus in the
// main checkout's work/ (skipped if absent), partly re-panned so that intensity stereo is used,
// with band energies from the reference's FDKaacEnc_CalcBandEnergyOptimLong.

use fdk_aac_rs_aacenc::aac_enc_rom::*;
use fdk_aac_rs_aacenc::aacenc_pns::PnsData;
use fdk_aac_rs_aacenc::intensity::*;
use fdk_aac_rs_aacenc::psy_configuration::{SfbParamLong, SfbParamShort};
use fdk_aac_rs_aacenc::psy_const::MAX_GROUPED_SFB;
use fdk_aac_rs_cref::aacenc_intensity as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};
use fdk_aac_rs_fdk::mdct::{mdct_block, mdct_init, MdctT};
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;
use std::io::{Read, Seek, SeekFrom};

const N: usize = MAX_GROUPED_SFB;

/// All inputs and outputs of FDKaacEnc_IntensityStereoProcessing().
#[derive(Clone, Debug, PartialEq)]
struct Case {
    e_l: Vec<FixpDbl>,
    e_r: Vec<FixpDbl>,
    spec_l: Vec<FixpDbl>,
    spec_r: Vec<FixpDbl>,
    thr_l: Vec<FixpDbl>,
    thr_r: Vec<FixpDbl>,
    thr_ld_r: Vec<FixpDbl>,
    spr_l: Vec<FixpDbl>,
    spr_r: Vec<FixpDbl>,
    ld_l: Vec<FixpDbl>,
    ld_r: Vec<FixpDbl>,
    ms_digest: i32,
    ms_mask: Vec<i32>,
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb: i32,
    offs: Vec<i32>,
    allow_is: i32,
    is_book: Vec<i32>,
    is_scale: Vec<i32>,
    /// 0: both PNS_DATA, 1: none (NULL, NULL)
    pns_mode: i32,
    pns: [PnsData; 2],
}

impl Case {
    fn new(rng: &mut Rng, sfb_cnt: i32, sfb_per_group: i32, max_sfb: i32, offs: Vec<i32>) -> Self {
        let garbage = |rng: &mut Rng, n: usize| (0..n).map(|_| rng.i32()).collect::<Vec<_>>();
        let mut pns = [PnsData::default(), PnsData::default()];
        for p in pns.iter_mut() {
            for i in 0..N {
                p.pns_flag[i] = (rng.next_u32() % 2) as i32;
                p.noise_energy_correlation[i] = rng.i32();
                p.noise_fuzzy_measure[i] = rng.i16();
            }
        }
        Case {
            e_l: vec![0; N],
            e_r: vec![0; N],
            spec_l: vec![0; 1024],
            spec_r: vec![0; 1024],
            thr_l: vec![0; N],
            thr_r: vec![0; N],
            thr_ld_r: garbage(rng, N),
            spr_l: garbage(rng, N),
            spr_r: garbage(rng, N),
            ld_l: vec![0; N],
            ld_r: vec![0; N],
            ms_digest: rng.range(0, 2),
            ms_mask: garbage(rng, N),
            sfb_cnt,
            sfb_per_group,
            max_sfb,
            offs,
            allow_is: if rng.next_u32() % 8 == 0 { 0 } else { [1, 1, 1, -3][(rng.next_u32() % 4) as usize] },
            is_book: garbage(rng, N),
            is_scale: garbage(rng, N),
            pns_mode: if rng.next_u32() % 4 == 0 { 1 } else { 0 },
            pns,
        }
    }

    fn run_c(&mut self) {
        let (p0, p1) = if self.pns_mode == 0 {
            let (a, b) = self.pns.split_at_mut(1);
            (&mut a[0] as *mut PnsData as *mut _, &mut b[0] as *mut PnsData as *mut _)
        } else {
            (core::ptr::null_mut(), core::ptr::null_mut())
        };
        unsafe {
            c::cref_intensity_IntensityStereoProcessing(
                self.e_l.as_mut_ptr(),
                self.e_r.as_mut_ptr(),
                self.spec_l.as_mut_ptr(),
                self.spec_r.as_mut_ptr(),
                self.thr_l.as_mut_ptr(),
                self.thr_r.as_mut_ptr(),
                self.thr_ld_r.as_mut_ptr(),
                self.spr_l.as_mut_ptr(),
                self.spr_r.as_mut_ptr(),
                self.ld_l.as_mut_ptr(),
                self.ld_r.as_mut_ptr(),
                &mut self.ms_digest,
                self.ms_mask.as_mut_ptr(),
                self.sfb_cnt,
                self.sfb_per_group,
                self.max_sfb,
                self.offs.as_ptr(),
                self.allow_is,
                self.is_book.as_mut_ptr(),
                self.is_scale.as_mut_ptr(),
                p0,
                p1,
            )
        }
    }

    fn run_rust(&mut self) {
        let pns = if self.pns_mode == 0 {
            let (a, b) = self.pns.split_at_mut(1);
            [Some(&mut a[0]), Some(&mut b[0])]
        } else {
            [None, None]
        };
        fdk_aac_enc_intensity_stereo_processing(
            &self.e_l,
            &mut self.e_r,
            &mut self.spec_l,
            &mut self.spec_r,
            &self.thr_l,
            &mut self.thr_r,
            &mut self.thr_ld_r,
            &self.spr_l,
            &mut self.spr_r,
            &self.ld_l,
            &mut self.ld_r,
            &mut self.ms_digest,
            &mut self.ms_mask,
            self.sfb_cnt,
            self.sfb_per_group,
            self.max_sfb,
            &self.offs,
            self.allow_is,
            &mut self.is_book,
            &mut self.is_scale,
            pns,
        );
    }

    /// Runs both and compares everything; returns the number of IS bands and of those out of
    /// phase.
    fn check(&self, what: &str) -> (usize, usize) {
        let mut cc = self.clone();
        let mut rc = self.clone();
        cc.run_c();
        rc.run_rust();
        assert_eq!(rc, cc, "{what}");
        let is = (0..self.sfb_cnt as usize).filter(|&i| cc.is_book[i] != 0);
        (is.clone().count(), is.filter(|&i| cc.ms_mask[i] != 0).count())
    }

    /// The static helpers on the same inputs (with the parameters of FDKaacEnc_initIsParams or
    /// random ones).
    fn check_helpers(&self, rng: &mut Rng, what: &str) {
        let mut params = IntensityParameters::default();
        fdk_aac_enc_init_is_params(&mut params);
        if rng.next_u32() % 4 == 0 {
            params = IntensityParameters {
                corr_thresh: rng.i32() >> 1,
                total_error_thresh: rng.i32() >> 4,
                local_error_thresh: rng.i32() >> 4,
                direction_deviation_thresh: rng.i32(),
                is_region_min_loudness: rng.i32() >> 2,
                min_is_sfbs: rng.range(-1, 10),
                left_right_ratio_threshold: rng.i32(),
            };
        }
        // hrrErr: cleared by the caller; sometimes garbage or the marker value.
        let mut hrr0 = vec![0; N];
        if rng.next_u32() % 4 == 0 {
            for h in hrr0.iter_mut() {
                *h = match rng.next_u32() % 3 {
                    0 => 0,
                    1 => 0x1000_0000,
                    _ => rng.i32(),
                };
            }
        }
        let mut c_hrr = hrr0.clone();
        let mut c_mask = vec![0; N];
        let mut c_real = vec![0x55; N];
        let mut c_norm = vec![0x66; N];
        unsafe {
            c::cref_intensity_prepareIntensityDecision(
                self.e_l.as_ptr(),
                self.e_r.as_ptr(),
                self.ld_l.as_ptr(),
                self.ld_r.as_ptr(),
                self.spec_l.as_ptr(),
                self.spec_r.as_ptr(),
                &params as *const _ as *const _,
                c_hrr.as_mut_ptr(),
                c_mask.as_mut_ptr(),
                c_real.as_mut_ptr(),
                c_norm.as_mut_ptr(),
                self.sfb_cnt,
                self.sfb_per_group,
                self.max_sfb,
                self.offs.as_ptr(),
            );
        }
        let mut r_hrr = hrr0.clone();
        let mut r_mask = vec![0; N];
        let mut r_real = vec![0x55; N];
        let mut r_norm = vec![0x66; N];
        fdk_aac_enc_prepare_intensity_decision(
            &self.e_l,
            &self.e_r,
            &self.ld_l,
            &self.ld_r,
            &self.spec_l,
            &self.spec_r,
            &params,
            &mut r_hrr,
            &mut r_mask,
            &mut r_real,
            &mut r_norm,
            self.sfb_cnt,
            self.sfb_per_group,
            self.max_sfb,
            &self.offs,
        );
        assert_eq!((&r_hrr, &r_mask, &r_real, &r_norm), (&c_hrr, &c_mask, &c_real, &c_norm), "{what}: prepare");

        // Finalize on these outputs, sometimes with a randomized mask.
        if rng.next_u32() % 3 == 0 {
            for m in c_mask.iter_mut() {
                *m = (rng.next_u32() % 2) as i32;
            }
        }
        let mut r_mask = c_mask.clone();
        unsafe {
            c::cref_intensity_finalizeIntensityDecision(
                c_hrr.as_ptr(),
                c_mask.as_mut_ptr(),
                c_real.as_ptr(),
                c_norm.as_ptr(),
                &params as *const _ as *const _,
                self.sfb_cnt,
                self.sfb_per_group,
                self.max_sfb,
            );
        }
        fdk_aac_enc_finalize_intensity_decision(
            &c_hrr,
            &mut r_mask,
            &c_real,
            &c_norm,
            &params,
            self.sfb_cnt,
            self.sfb_per_group,
            self.max_sfb,
        );
        assert_eq!(r_mask, c_mask, "{what}: finalize");
    }
}

fn long_offsets(p: &SfbParamLong) -> Vec<i32> {
    let mut offs = vec![0i32; N + 1];
    for i in 0..p.sfb_cnt as usize {
        offs[i + 1] = offs[i] + p.sfb_width[i] as i32;
    }
    offs
}

/// Grouped offsets (`groupedSfbOffset`) of `group_len` windows of a short table.
fn grouped_offsets(p: &SfbParamShort, group_len: &[usize]) -> Vec<i32> {
    let mut offs = vec![0i32];
    for &gl in group_len {
        for i in 0..p.sfb_cnt as usize {
            let last = *offs.last().unwrap();
            offs.push(last + p.sfb_width[i] as i32 * gl as i32);
        }
    }
    offs.resize(N + 1, *offs.last().unwrap());
    offs
}

const LONG_TABLES: [&SfbParamLong; 6] = [
    &P_FDK_AAC_ENC_8000_LONG_1024,
    &P_FDK_AAC_ENC_16000_LONG_1024,
    &P_FDK_AAC_ENC_22050_LONG_1024,
    &P_FDK_AAC_ENC_32000_LONG_1024,
    &P_FDK_AAC_ENC_44100_LONG_1024,
    &P_FDK_AAC_ENC_48000_LONG_1024,
];
const SHORT_TABLES: [&SfbParamShort; 4] = [
    &P_FDK_AAC_ENC_8000_SHORT_128,
    &P_FDK_AAC_ENC_22050_SHORT_128,
    &P_FDK_AAC_ENC_44100_SHORT_128,
    &P_FDK_AAC_ENC_48000_SHORT_128,
];

/// A random partition of the 8 short windows into 1 to 4 groups.
fn random_groups(rng: &mut Rng) -> Vec<usize> {
    loop {
        let n = rng.range(1, 4) as usize;
        let mut g = vec![1usize; n];
        for _ in 0..8 - n {
            let i = (rng.next_u32() as usize) % n;
            g[i] += 1;
        }
        if g.iter().sum::<usize>() == 8 {
            return g;
        }
    }
}

/// A random layout: (sfbCnt, sfbPerGroup, maxSfbPerGroup, offsets, short window groups).
fn random_layout(rng: &mut Rng) -> (i32, i32, i32, Vec<i32>, Option<Vec<usize>>) {
    if rng.next_u32() % 2 == 0 {
        let p = LONG_TABLES[(rng.next_u32() as usize) % LONG_TABLES.len()];
        let cnt = p.sfb_cnt as i32;
        let max = if rng.next_u32() % 4 == 0 { rng.range(0, cnt) } else { rng.range(cnt / 2, cnt) };
        (cnt, cnt, max, long_offsets(p), None)
    } else {
        let p = SHORT_TABLES[(rng.next_u32() as usize) % SHORT_TABLES.len()];
        let g = random_groups(rng);
        let per = p.sfb_cnt as i32;
        let max = if rng.next_u32() % 4 == 0 { rng.range(0, per) } else { rng.range(per / 2, per) };
        (per * g.len() as i32, per, max, grouped_offsets(p, &g), Some(g))
    }
}

/// Band energies with the reference's FDKaacEnc_CalcBandEnergyOptimLong.
fn energies(spec: &[FixpDbl], offs: &[i32], n: i32) -> (Vec<FixpDbl>, Vec<FixpDbl>) {
    let mut e = vec![0; N];
    let mut ld = vec![0; N];
    let mut max_scale = vec![0; N];
    unsafe {
        c::cref_intensity_CalcBandEnergyOptimLong(
            spec.as_ptr(),
            max_scale.as_mut_ptr(),
            offs.as_ptr(),
            n,
            e.as_mut_ptr(),
            ld.as_mut_ptr(),
        );
    }
    (e, ld)
}

/// Thresholds: the energy scaled by a random factor (sometimes above the energy).
fn thresholds(rng: &mut Rng, e: &[FixpDbl]) -> Vec<FixpDbl> {
    e.iter()
        .map(|&x| match rng.next_u32() % 4 {
            0 => x >> rng.range(0, 12),
            1 => x.saturating_mul(2),
            2 => fdk_aac_rs_fdk::common_fix::f_mult(x, rng.i32() & 0x7fff_ffff),
            _ => x >> 3,
        })
        .collect()
}

fn ld_data(v: &[FixpDbl]) -> Vec<FixpDbl> {
    v.iter().map(|&x| unsafe { c::cref_intensity_CalcLdData(x) }).collect()
}

/// Fills energies, ld data and thresholds of a case from its spectra.
fn fill_from_spectra(rng: &mut Rng, k: &mut Case) {
    let (e_l, ld_l) = energies(&k.spec_l, &k.offs, k.sfb_cnt);
    let (e_r, ld_r) = energies(&k.spec_r, &k.offs, k.sfb_cnt);
    k.thr_l = thresholds(rng, &e_l);
    k.thr_r = thresholds(rng, &e_r);
    k.thr_ld_r = ld_data(&k.thr_r);
    k.spr_l = thresholds(rng, &e_l);
    k.spr_r = thresholds(rng, &e_r);
    k.e_l = e_l;
    k.e_r = e_r;
    k.ld_l = ld_l;
    k.ld_r = ld_r;
}

#[test]
fn calc_sfb_max_scale_matches() {
    let mut rng = Rng::new(0x6973_6d73);
    for _ in 0..20000 {
        let mut spec: Vec<FixpDbl> = (0..128).map(|_| rng.i32() >> rng.range(0, 31)).collect();
        if rng.next_u32() % 4 == 0 {
            let i = (rng.next_u32() % 128) as usize;
            spec[i] = [MINVAL_DBL, MAXVAL_DBL, -MAXVAL_DBL, 0, 1, -1][(rng.next_u32() % 6) as usize];
        }
        if rng.next_u32() % 8 == 0 {
            spec.iter_mut().for_each(|x| *x = 0);
        }
        if rng.next_u32() % 8 == 0 {
            spec.iter_mut().for_each(|x| *x = MINVAL_DBL);
        }
        let l1 = rng.range(0, 128);
        let l2 = rng.range(0, 128);
        let want = unsafe { c::cref_intensity_calcSfbMaxScale(spec.as_ptr(), l1, l2) };
        assert_eq!(calc_sfb_max_scale(&spec, l1, l2), want, "{l1}..{l2}");
    }
}

#[test]
fn init_is_params() {
    let mut c_params = [0i32; 7];
    let size = unsafe { c::cref_intensity_initIsParams(c_params.as_mut_ptr() as *mut _) };
    assert_eq!(size as usize, core::mem::size_of::<IntensityParameters>());
    let mut r = IntensityParameters::default();
    fdk_aac_enc_init_is_params(&mut r);
    let r_arr: [i32; 7] = unsafe { core::mem::transmute(r) };
    assert_eq!(r_arr, c_params);
}

#[test]
fn random_data() {
    let mut rng = Rng::new(0x6973_7264);
    let mut is_bands = (0, 0);
    for iter in 0..20000 {
        let (cnt, per, max, offs, _) = random_layout(&mut rng);
        let mut k = Case::new(&mut rng, cnt, per, max, offs);
        let shift_sp = rng.range(0, 24);
        for i in 0..1024 {
            k.spec_l[i] = rng.i32() >> shift_sp;
            k.spec_r[i] = match rng.next_u32() % 4 {
                // correlated: a scaled and possibly inverted copy
                0 | 1 => {
                    let g = rng.range(-8, 8);
                    if g >= 0 { k.spec_l[i] >> g } else { (k.spec_l[i] >> -g).wrapping_neg() }
                }
                _ => rng.i32() >> shift_sp,
            };
        }
        if rng.next_u32() % 2 == 0 {
            // Spectra and energies of the same frame, with a common panning per frame.
            let g = rng.range(1, 6);
            let inv = rng.next_u32() % 2 == 0;
            for i in 0..1024 {
                let v = k.spec_l[i] >> g;
                k.spec_r[i] = if inv { v.wrapping_neg() } else { v };
            }
            fill_from_spectra(&mut rng, &mut k);
        } else {
            for i in 0..N {
                k.e_l[i] = if rng.next_u32() % 16 == 0 { rng.i32() } else { rng.i32() & 0x7fff_ffff } >> rng.range(0, 30);
                k.e_r[i] = if rng.next_u32() % 16 == 0 { rng.i32() } else { rng.i32() & 0x7fff_ffff } >> rng.range(0, 30);
                k.ld_l[i] = if rng.next_u32() % 8 == 0 { rng.i32() } else { -(rng.i32() & 0x7fff_ffff) >> 1 };
                k.ld_r[i] = if rng.next_u32() % 8 == 0 { rng.i32() } else { -(rng.i32() & 0x7fff_ffff) >> 1 };
                k.thr_l[i] = k.e_l[i] >> rng.range(0, 8);
                k.thr_r[i] = k.e_r[i] >> rng.range(0, 8);
            }
        }
        if rng.next_u32() % 50 == 0 {
            // Edge values in the spectra.
            for _ in 0..8 {
                let i = (rng.next_u32() % 1024) as usize;
                k.spec_l[i] = [MINVAL_DBL, MAXVAL_DBL, 0, -1][(rng.next_u32() % 4) as usize];
                let i = (rng.next_u32() % 1024) as usize;
                k.spec_r[i] = [MINVAL_DBL, MAXVAL_DBL, 0, -1][(rng.next_u32() % 4) as usize];
            }
        }
        let (a, b) = k.check(&format!("random {iter}"));
        is_bands = (is_bands.0 + a, is_bands.1 + b);
        k.check_helpers(&mut rng, &format!("random {iter}"));
    }
    eprintln!("random_data: {} IS bands, {} out of phase", is_bands.0, is_bands.1);
    assert!(is_bands.0 > 0 && is_bands.1 > 0);
}

/// Stereo s16le files of the corpus in the main checkout's work/corpus.
fn corpus_files() -> Vec<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    let common = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let dir = common.join("..").join("work").join("corpus");
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut files: Vec<_> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.ends_with(".raw") && !n.contains("mono") && !n.starts_with("c51")
        })
        .collect();
    files.sort();
    files
}

/// `frames` stereo frames starting at frame `pos` (modulo the file length).
fn read_stereo(path: &std::path::Path, pos: u64, frames: usize) -> Option<(Vec<i16>, Vec<i16>)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len() / 4;
    if len < frames as u64 {
        return None;
    }
    let start = pos % (len - frames as u64 + 1);
    f.seek(SeekFrom::Start(start * 4)).ok()?;
    let mut buf = vec![0u8; frames * 4];
    f.read_exact(&mut buf).ok()?;
    let l = buf.chunks(4).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let r = buf.chunks(4).map(|c| i16::from_le_bytes([c[2], c[3]])).collect();
    Some((l, r))
}

/// MDCT of 2048 samples: one long block, or 8 short blocks regrouped like
/// FDKaacEnc_groupShortData() (per group, per band, the lines of each window).
fn spectrum(time: &[i16], short: Option<(&SfbParamShort, &[usize])>) -> Vec<FixpDbl> {
    let mut m = MdctT::default();
    mdct_init(&mut m, 0);
    let mut spec = vec![0; 1024];
    let mut e = [0i16; 8];
    match short {
        None => {
            mdct_block(&mut m, time, 1024, &mut spec, 1, 1024, fdk_get_window_slope(1024, 0), 1024, &mut e);
            spec
        }
        Some((p, groups)) => {
            mdct_block(&mut m, time, 1024, &mut spec, 8, 128, fdk_get_window_slope(128, 0), 128, &mut e);
            let mut out = Vec::with_capacity(1024);
            let mut w0 = 0;
            for &gl in groups {
                let mut off = 0usize;
                for i in 0..p.sfb_cnt as usize {
                    let wd = p.sfb_width[i] as usize;
                    for w in w0..w0 + gl {
                        out.extend_from_slice(&spec[w * 128 + off..w * 128 + off + wd]);
                    }
                    off += wd;
                }
                w0 += gl;
            }
            out.resize(1024, 0);
            out
        }
    }
}

#[test]
fn real_stereo() {
    let files = corpus_files();
    if files.is_empty() {
        eprintln!("real_stereo: work/corpus not available, skipped");
        return;
    }
    let mut rng = Rng::new(0x6973_7265);
    let mut is_bands = (0, 0);
    let mut cases = 0;
    for iter in 0..3000 {
        let path = &files[iter % files.len()];
        let pos = rng.next_u64() % 10_000_000;
        let Some((mut l, mut r)) = read_stereo(path, pos, 2048) else { continue };
        match rng.next_u32() % 4 {
            0 | 1 => {
                // Re-pan a mono downmix (intensity stereo friendly), optionally out of phase,
                // with a little of the original right channel.
                let gl = rng.range(1, 16);
                let gr = rng.range(1, 16);
                let inv = rng.next_u32() % 3 == 0;
                let leak = rng.range(0, 64);
                for i in 0..2048 {
                    let m = (l[i] as i32 + r[i] as i32) / 2;
                    let a = m * gl / 16;
                    let b = m * gr / 16 * if inv { -1 } else { 1 } + r[i] as i32 * leak / 1024;
                    l[i] = a.clamp(-32768, 32767) as i16;
                    r[i] = b.clamp(-32768, 32767) as i16;
                }
            }
            2 => {
                // Original stereo, the right channel attenuated.
                let sh = rng.range(0, 6);
                r.iter_mut().for_each(|x| *x >>= sh);
            }
            _ => {}
        }
        let (cnt, per, max, offs, groups) = random_layout(&mut rng);
        let mut k = Case::new(&mut rng, cnt, per, max, offs);
        let short = groups.as_ref().map(|g| {
            let p = SHORT_TABLES.iter().find(|p| {
                let o = grouped_offsets(p, g);
                o == k.offs
            });
            (*p.unwrap(), g.as_slice())
        });
        k.spec_l = spectrum(&l, short);
        k.spec_r = spectrum(&r, short);
        fill_from_spectra(&mut rng, &mut k);
        let what = format!("{} pos {pos} iter {iter}", path.display());
        let (a, b) = k.check(&what);
        is_bands = (is_bands.0 + a, is_bands.1 + b);
        k.check_helpers(&mut rng, &what);
        cases += 1;
    }
    eprintln!("real_stereo: {cases} frames, {} IS bands, {} out of phase", is_bands.0, is_bands.1);
    assert!(is_bands.0 > 0 && is_bands.1 > 0);
}

/// The intensity positions of a channel, in the order in which
/// FDKaacEnc_scfCount() (dyn_bits.cpp), FDKaacEnc_encodeScaleFactorData() (bitenc.cpp) and
/// FDKaacEnc_calcSfbPe() (line_pe.cpp) code them as deltas from 0 and from the previous position.
/// Returns the number of positions, or an error if they do not all have the same sign, are
/// outside 1..=60 in magnitude, or give a delta outside -60..=60 (the range of
/// FDKaacEnc_huff_ltabscf in bitCountScalefactorDelta()).
fn is_positions_bounded(k: &Case) -> Result<usize, String> {
    let pos: Vec<i32> =
        (0..k.sfb_cnt as usize).filter(|&i| k.is_book[i] != 0).map(|i| k.is_scale[i]).collect();
    let mut last = 0;
    for &p in &pos {
        if !(1..=60).contains(&p.abs()) {
            return Err(format!("position {p} in {pos:?}"));
        }
        if (p - last).abs() > 60 {
            return Err(format!("delta {} in {pos:?}", p - last));
        }
        last = p;
    }
    if pos.iter().any(|&p| p > 0) && pos.iter().any(|&p| p < 0) {
        return Err(format!("both signs in {pos:?}"));
    }
    Ok(pos.len())
}

/// FDKaacEnc_IntensityStereoProcessing() never produces intensity positions whose deltas
/// leave -60..=60 when its energies and their ld values belong together (as in
/// FDKaacEnc_psyMain(); FDKaacEnc_CalcBandEnergyOptimLong() here). A band only becomes an
/// intensity band if its linear energies are at least 1/0.7 apart (otherwise `hrrErr` is 1/8,
/// which neither seeds nor extends an intensity region), so its |realIsScale| is above
/// log2(1/0.7)/64 * 2^31 > 2^24, 1.0 in units of the intensity position (`ld_ratio_margin`),
/// and FDKaacEnc_finalizeIntensityDecision() keeps consecutive intensity bands of a channel
/// less than 2^25 (2.0) apart, across regions and groups. So all positions of a channel have
/// the same sign and lie in 1..=60 (realIsScale is limited to +-60): consecutive positions
/// differ by at most 59, the first one (coded as a delta from 0) is at most 60.
///
/// The bands are panned by factors close to the 0.7 threshold (and others), alternately to the
/// left and to the right, in runs, or randomly, in phase or inverted, with silent bands, edge
/// values and random thresholds (which drop bands from the intensity regions). The reference's
/// output is checked and compared with the port.
#[test]
fn is_positions() {
    let mut rng = Rng::new(0x4832_3269);
    let (mut frames, mut bands, mut ones, mut both_signs_seeded) = (0, 0usize, 0usize, 0usize);
    // amplitude factors (x / 2^15) around sqrt(0.7) = 0.83666 * 2^15 = 27416
    let near = [27410, 27413, 27415, 27416, 27417, 27419, 27422, 27430, 27380];
    for iter in 0..30000 {
        let (cnt, per, max, offs, _) = random_layout(&mut rng);
        let mut k = Case::new(&mut rng, cnt, per, max, offs);
        k.allow_is = 1;
        let sh = rng.range(0, 20);
        for i in 0..1024 {
            k.spec_l[i] = rng.i32() >> sh;
        }
        let pattern = rng.range(0, 2);
        let mut left = rng.next_u32() % 2 == 0;
        let inv = rng.next_u32() % 2 == 0;
        for b in 0..k.sfb_cnt as usize {
            left = match pattern {
                0 => !left,
                1 => left ^ (rng.next_u32() % 6 == 0),
                _ => rng.next_u32() % 2 == 0,
            };
            let g: i64 = match rng.next_u32() % 8 {
                0 => 0,
                1 => rng.range(1, 30000) as i64,
                2 => 32768,
                _ => near[(rng.next_u32() as usize) % near.len()],
            };
            let (a, z) = (k.offs[b] as usize, (k.offs[b + 1] as usize).min(1024));
            for i in a..z {
                let y = (k.spec_l[i] as i64 * g) >> 15;
                let y = if inv { -y } else { y } as i32;
                if left {
                    k.spec_r[i] = y;
                } else {
                    k.spec_r[i] = k.spec_l[i];
                    k.spec_l[i] = y;
                }
            }
        }
        if rng.next_u32() % 20 == 0 {
            for _ in 0..8 {
                let i = (rng.next_u32() % 1024) as usize;
                k.spec_l[i] = [MINVAL_DBL, MAXVAL_DBL, 0, -1][(rng.next_u32() % 4) as usize];
                let i = (rng.next_u32() % 1024) as usize;
                k.spec_r[i] = [MINVAL_DBL, MAXVAL_DBL, 0, -1][(rng.next_u32() % 4) as usize];
            }
        }
        fill_from_spectra(&mut rng, &mut k);
        // seeds of both signs in the reference's prepareIntensityDecision()
        {
            let mut params = IntensityParameters::default();
            fdk_aac_enc_init_is_params(&mut params);
            let (mut hrr, mut mask, mut real, mut norm) = (vec![0; N], vec![0; N], vec![0; N], vec![0; N]);
            unsafe {
                c::cref_intensity_prepareIntensityDecision(
                    k.e_l.as_ptr(),
                    k.e_r.as_ptr(),
                    k.ld_l.as_ptr(),
                    k.ld_r.as_ptr(),
                    k.spec_l.as_ptr(),
                    k.spec_r.as_ptr(),
                    &params as *const _ as *const _,
                    hrr.as_mut_ptr(),
                    mask.as_mut_ptr(),
                    real.as_mut_ptr(),
                    norm.as_mut_ptr(),
                    k.sfb_cnt,
                    k.sfb_per_group,
                    k.max_sfb,
                    k.offs.as_ptr(),
                );
            }
            let seeded = || (0..N).filter(|&i| mask[i] != 0);
            if seeded().any(|i| real[i] > 0) && seeded().any(|i| real[i] < 0) {
                both_signs_seeded += 1;
            }
        }
        let mut cc = k.clone();
        cc.run_c();
        match is_positions_bounded(&cc) {
            Ok(n) => bands += n,
            Err(e) => panic!("iter {iter}: {e}"),
        }
        ones += (0..cc.sfb_cnt as usize).filter(|&i| cc.is_book[i] != 0 && cc.is_scale[i].abs() == 1).count();
        k.check(&format!("is positions {iter}"));
        frames += 1;
    }
    eprintln!(
        "is_positions: {frames} frames, {bands} intensity bands ({ones} with |position| 1), \
         {both_signs_seeded} frames with seeds of both signs"
    );
    assert!(bands > 3000 && ones > 500 && both_signs_seeded > 5000);
}

/// For linear energies that are not in the "middle" (`a < fMult(0.7, b)`, the condition of
/// FDKaacEnc_prepareIntensityDecision() for left and right), CalcLdData(b) - CalcLdData(a) of
/// the reference is above 2^24, the unit of an intensity position (log2(1/0.7)/64 * 2^31 is
/// 17 270 087). `a` is the largest value that meets the condition (with the arm64 fMult(), which
/// is not below the x86 one), and CalcLdData() is non-decreasing (checked on the same values).
/// Sampled here; the `#[ignore]`d `ld_ratio_margin_exhaustive` checks every `b`.
#[test]
fn ld_ratio_margin() {
    let mut rng = Rng::new(0x6c64_6d72);
    let mut bs: Vec<i32> = (1..31).flat_map(|e| [(1i32 << e) - 1, 1i32 << e, (1i32 << e) + 1]).collect();
    bs.extend((0..2_000_000).map(|_| (rng.i32() & 0x7fff_ffff) >> rng.range(0, 29)));
    let min = ld_margin(bs.into_iter());
    eprintln!("ld_ratio_margin: minimum difference {min}");
}

#[test]
#[ignore]
fn ld_ratio_margin_exhaustive() {
    let min = ld_margin(2..=i32::MAX);
    eprintln!("ld_ratio_margin_exhaustive: minimum difference {min}");
}

/// The smallest CalcLdData(b) - CalcLdData(fMult(0.7, b) - 1) over `bs` (values below 2 are
/// skipped); asserts that it is above 2^24 and that CalcLdData(a) <= CalcLdData(a + 1).
fn ld_margin(bs: impl Iterator<Item = i32>) -> i64 {
    let ld = |x: i32| unsafe { c::cref_intensity_CalcLdData(x) } as i64;
    let mut min = i64::MAX;
    for b in bs {
        if b < 2 {
            continue;
        }
        let a = ((0x5999_999a_i64 * b as i64) >> 31) as i32 - 1;
        if a < 1 {
            continue;
        }
        let d = ld(b) - ld(a);
        assert!(d > 1 << 24, "b {b}: CalcLdData difference {d}");
        assert!(ld(a) <= ld(a + 1), "CalcLdData({a}) > CalcLdData({})", a + 1);
        min = min.min(d);
    }
    min
}

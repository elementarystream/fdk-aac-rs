// Differential tests of fdk_aac_rs_aacenc::ms_stereo against the C++ reference.
//
// The shim builds the PSY_DATA and PSY_OUT_CHANNEL structures of FDKaacEnc_psyMain() around the
// port's PsyData and QcOutChannel (same layout), so the complete structures of both channels
// are compared after every call, together with msMask and msDigest. Inputs: random ld energies
// and thresholds (long blocks and grouped short blocks, with and without intensity codebooks),
// and the energies, thresholds and spectra of stereo MDCT frames of real and synthetic PCM
// computed with the ported band energy and grouping functions as in FDKaacEnc_psyMain().

use crate::band_nrg::{configs, frames, normalize};
use fdk_aac_rs_aacenc::band_nrg::*;
use fdk_aac_rs_aacenc::grp_data::fdk_aac_enc_group_short_data;
use fdk_aac_rs_aacenc::ms_stereo::*;
use fdk_aac_rs_aacenc::psy_configuration::C_RATIO;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_aacenc::psy_data::PsyData;
use fdk_aac_rs_aacenc::qc_data::QcOutChannel;
use fdk_aac_rs_cref::aacenc_ms_stereo as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fixpoint_math::ld_data_vector;
use std::ffi::c_void;

#[derive(Clone)]
struct Pair {
    psy: [PsyData; 2],
    qc: Box<[QcOutChannel; 2]>,
    ms_mask: [i32; MAX_GROUPED_SFB],
    ms_digest: i32,
}

#[allow(clippy::too_many_arguments)]
fn check(
    p: &Pair,
    is_book: Option<&[i32]>,
    allow_ms: i32,
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
    offsets: &[i32],
    what: &str,
) -> Pair {
    let (mut a, mut b) = (p.clone(), p.clone());
    fdk_aac_enc_ms_stereo_processing(
        &mut a.psy,
        &mut a.qc[..],
        is_book,
        &mut a.ms_digest,
        &mut a.ms_mask,
        allow_ms,
        sfb_cnt,
        sfb_per_group,
        max_sfb_per_group,
        offsets,
    );
    let (p0, p1) = b.psy.split_at_mut(1);
    let (q0, q1) = b.qc.split_at_mut(1);
    unsafe {
        c::cref_ms_stereo_MsStereoProcessing(
            &mut p0[0] as *mut PsyData as *mut c_void,
            &mut p1[0] as *mut PsyData as *mut c_void,
            &mut q0[0] as *mut QcOutChannel as *mut c_void,
            &mut q1[0] as *mut QcOutChannel as *mut c_void,
            is_book.map_or(std::ptr::null(), |x| x.as_ptr()),
            &mut b.ms_digest,
            b.ms_mask.as_mut_ptr(),
            allow_ms,
            sfb_cnt,
            sfb_per_group,
            max_sfb_per_group,
            offsets.as_ptr(),
        )
    };
    assert_eq!((a.ms_digest, a.ms_mask), (b.ms_digest, b.ms_mask), "{what}: msDigest, msMask");
    assert!(a.psy == b.psy, "{what}: PSY_DATA");
    assert!(a.qc == b.qc, "{what}: QC_OUT_CHANNEL");
    a
}

fn ld(r: &mut Rng) -> i32 {
    match r.next_u32() % 8 {
        0 => r.i32(),
        1 => [MINVAL_DBL, MAXVAL_DBL, 0, fl2fxconst_dbl(-0.515625)][(r.next_u32() % 4) as usize],
        _ => -((r.next_u32() >> 2) as i32),
    }
}

fn random_pair(r: &mut Rng) -> Pair {
    let mut p = Pair {
        psy: [PsyData::default(); 2],
        qc: Box::new([QcOutChannel::default(); 2]),
        ms_mask: [0; MAX_GROUPED_SFB],
        ms_digest: -3,
    };
    for ch in 0..2 {
        let ps = &mut p.psy[ch];
        for i in 0..ps.sfb_energy.len() {
            ps.sfb_energy[i] = (r.next_u32() >> 1) as i32 >> (r.next_u32() % 16);
            ps.sfb_threshold[i] = (r.next_u32() >> 1) as i32 >> (r.next_u32() % 16);
            ps.sfb_energy_ms[i] = (r.next_u32() >> 1) as i32 >> (r.next_u32() % 16);
            ps.sfb_spread_energy[i] = r.i32();
            ps.sfb_energy_ld_data[i] = r.i32();
        }
        for i in 0..MAX_GROUPED_SFB {
            ps.sfb_energy_ms_ld_data[i] = ld(r);
        }
        let q = &mut p.qc[ch];
        for x in q.mdct_spectrum.iter_mut() {
            *x = r.i32();
        }
        for i in 0..MAX_GROUPED_SFB {
            q.sfb_energy_ld_data[i] = ld(r);
            q.sfb_threshold_ld_data[i] = ld(r);
            q.sfb_energy[i] = r.i32();
        }
    }
    for m in p.ms_mask.iter_mut() {
        *m = (r.next_u32() % 4 == 0) as i32;
    }
    p
}

#[test]
fn ms_stereo_random() {
    let mut r = Rng::new(91);
    for it in 0..6000 {
        let p = random_pair(&mut r);
        let short = r.next_u32() % 2 == 0;
        let (sfb_cnt, sfb_per_group, max_sfb, offsets) = if short {
            let groups = r.range(1, 4);
            let per = r.range(1, 15);
            let max = r.range(0, per);
            let mut o = vec![0i32];
            for _ in 0..60 {
                let last = *o.last().unwrap();
                o.push((last + r.range(0, 16)).min(1024));
            }
            (groups * per, per, max, o)
        } else {
            let n = r.range(0, 51);
            let max = r.range(0, n);
            let mut o = vec![0i32];
            for _ in 0..60 {
                let last = *o.last().unwrap();
                o.push((last + r.range(0, 40)).min(1024));
            }
            (n, n.max(1), max, o)
        };
        let book: Vec<i32> = (0..64).map(|_| if r.next_u32() % 8 == 0 { r.range(14, 15) } else { 0 }).collect();
        let is_book = match r.next_u32() % 3 {
            0 => None,
            1 => Some(&book[..]),
            _ => Some(&[0i32; 64][..]),
        };
        let allow_ms = (r.next_u32() % 8 != 0) as i32;
        let what = format!("iteration {it}");
        check(&p, is_book, allow_ms, sfb_cnt, sfb_per_group, max_sfb, &offsets, &what);
    }
}

#[test]
fn ms_stereo_realistic() {
    let mut r = Rng::new(92);
    let mut digests = [0; 3];
    for conf in configs() {
        for mut frame in frames(&conf, 10) {
            let pc = *frame.conf(&conf);
            let (nw, wl) = frame.windows(&conf);
            let scale = normalize(&mut frame, &conf);
            let max_sfb = if frame.short { MAX_SFB_SHORT } else { MAX_GROUPED_SFB };
            let mut p = Pair {
                psy: [PsyData::default(); 2],
                qc: Box::new([QcOutChannel::default(); 2]),
                ms_mask: [0; MAX_GROUPED_SFB],
                ms_digest: 0,
            };
            for ch in 0..2 {
                p.qc[ch].mdct_spectrum[..conf.granule].copy_from_slice(&frame.spectrum[ch]);
            }
            // energies, thresholds and mid/side energies per window (.Short or .Long)
            for w in 0..nw {
                let off = w * max_sfb;
                let n = pc.sfb_active;
                for ch in 0..2 {
                    let spec = &frame.spectrum[ch][w * wl..];
                    let ps = &mut p.psy[ch];
                    let mut ld = vec![0i32; 64];
                    if frame.short {
                        fdk_aac_enc_calc_band_energy_optim_short(spec, &scale[ch][off..], &pc.sfb_offset, n, &mut ps.sfb_energy[off..]);
                    } else {
                        fdk_aac_enc_calc_band_energy_optim_long(spec, &scale[ch][off..], &pc.sfb_offset, n, &mut ps.sfb_energy[off..], &mut ld);
                    }
                    for i in 0..n as usize {
                        ps.sfb_threshold[off + i] = f_mult(ps.sfb_energy[off + i], C_RATIO);
                        ps.sfb_spread_energy[off + i] = ps.sfb_energy[off + i];
                    }
                }
                let (p0, p1) = p.psy.split_at_mut(1);
                let mut mid_ld = vec![0i32; 64];
                let mut side_ld = vec![0i32; 64];
                fdk_aac_enc_calc_band_nrg_ms_opt(
                    &frame.spectrum[0][w * wl..],
                    &frame.spectrum[1][w * wl..],
                    &scale[0][off..],
                    &scale[1][off..],
                    &pc.sfb_offset,
                    n,
                    &mut p0[0].sfb_energy_ms[off..],
                    &mut p1[0].sfb_energy_ms[off..],
                    (!frame.short) as i32,
                    &mut mid_ld,
                    &mut side_ld,
                );
                if !frame.short {
                    p0[0].sfb_energy_ms_ld_data[..n as usize].copy_from_slice(&mid_ld[..n as usize]);
                    p1[0].sfb_energy_ms_ld_data[..n as usize].copy_from_slice(&side_ld[..n as usize]);
                }
            }
            let (sfb_cnt, sfb_per_group, max_sfb_per_group, offsets);
            if frame.short {
                let mut maxs = [0i32; 2];
                let mut grouped = vec![0i32; 61];
                for ch in 0..2 {
                    let ps = &mut p.psy[ch];
                    let q = &mut p.qc[ch];
                    let mut min_snr = vec![0i32; 60];
                    fdk_aac_enc_group_short_data(
                        &mut q.mdct_spectrum,
                        &mut ps.sfb_threshold,
                        &mut ps.sfb_energy,
                        &mut ps.sfb_energy_ms,
                        &mut ps.sfb_spread_energy,
                        pc.sfb_cnt,
                        pc.sfb_active,
                        &pc.sfb_offset,
                        &pc.sfb_min_snr_ld_data,
                        &mut ps.grouped_sfb_offset,
                        &mut maxs[ch],
                        &mut min_snr,
                        frame.groups.0,
                        &frame.groups.1,
                        conf.granule as i32,
                    );
                    grouped.copy_from_slice(&ps.grouped_sfb_offset);
                    let no_sfb = (frame.groups.0 * pc.sfb_cnt) as usize;
                    let a = pc.sfb_active as usize;
                    for g in (0..no_sfb).step_by(pc.sfb_cnt as usize) {
                        ld_data_vector(&ps.sfb_energy[g..g + a], &mut q.sfb_energy_ld_data[g..]);
                        ld_data_vector(&ps.sfb_threshold[g..g + a], &mut q.sfb_threshold_ld_data[g..]);
                        ld_data_vector(&ps.sfb_energy_ms[g..g + a], &mut ps.sfb_energy_ms_ld_data[g..]);
                    }
                }
                sfb_cnt = frame.groups.0 * pc.sfb_cnt;
                sfb_per_group = pc.sfb_cnt;
                max_sfb_per_group = maxs[0].max(maxs[1]);
                offsets = grouped;
            } else {
                let n = pc.sfb_active as usize;
                for ch in 0..2 {
                    let ps = &p.psy[ch];
                    let q = &mut p.qc[ch];
                    ld_data_vector(&ps.sfb_energy[..n], &mut q.sfb_energy_ld_data);
                    ld_data_vector(&ps.sfb_threshold[..n], &mut q.sfb_threshold_ld_data);
                }
                sfb_cnt = pc.sfb_active;
                sfb_per_group = pc.sfb_active;
                max_sfb_per_group = r.range(pc.sfb_active.min(5), pc.sfb_active);
                offsets = pc.sfb_offset.to_vec();
            }
            for q in p.qc.iter_mut() {
                for x in q.sfb_threshold_ld_data.iter_mut() {
                    *x = fix_max(*x, fl2fxconst_dbl(-0.515625));
                }
            }
            let zero = [0i32; 64];
            let what = format!("{} short {}", conf.name, frame.short);
            let out = check(&p, Some(&zero), conf.long.allow_ms, sfb_cnt, sfb_per_group, max_sfb_per_group, &offsets, &what);
            digests[out.ms_digest as usize] += 1;
            check(&p, None, 0, sfb_cnt, sfb_per_group, max_sfb_per_group, &offsets, &what);
        }
    }
    eprintln!("msDigest NONE/SOME/ALL: {digests:?}");
    assert!(digests.iter().all(|&d| d > 0), "{digests:?}");
}

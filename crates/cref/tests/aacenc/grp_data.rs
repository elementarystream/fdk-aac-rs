// Differential tests of fdk_aac_rs_aacenc::grp_data against the C++ reference.
//
// FDKaacEnc_groupShortData is compared with the short block band layouts of real configurations
// and random ones, the groupings of the block switching (and random partitions of the eight
// windows), random energies (including saturating sums) and short block spectra of real and
// synthetic PCM. All outputs are compared completely (the entries C skips keep their previous
// contents in both), except the spectral lines without a defined value in the reference: the
// port writes 0 there, which is checked.

use crate::band_nrg::{configs, frames};
use fdk_aac_rs_aacenc::block_switch::SUGGESTED_GROUPING_TABLE;
use fdk_aac_rs_aacenc::grp_data::*;
use fdk_aac_rs_aacenc::psy_configuration::PsyConfiguration;
use fdk_aac_rs_aacenc::psy_data::SFB_UNION_LEN;
use fdk_aac_rs_cref::aacenc_grp_data as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;

#[derive(Clone, PartialEq, Debug)]
struct Data {
    spec: Vec<i32>,
    arr: [[i32; SFB_UNION_LEN]; 4],
    grouped_offset: Vec<i32>,
    max_sfb: i32,
    grouped_min_snr: Vec<i32>,
}

#[allow(clippy::too_many_arguments)]
fn check(
    d: &Data,
    sfb_cnt: i32,
    sfb_active: i32,
    offsets: &[i32],
    min_snr: &[i32],
    groups: (i32, [i32; 4]),
    granule: i32,
    what: &str,
) {
    let (mut a, mut b) = (d.clone(), d.clone());
    {
        let [t, e, ms, sp] = &mut a.arr;
        fdk_aac_enc_group_short_data(
            &mut a.spec,
            t,
            e,
            ms,
            sp,
            sfb_cnt,
            sfb_active,
            offsets,
            min_snr,
            &mut a.grouped_offset,
            &mut a.max_sfb,
            &mut a.grouped_min_snr,
            groups.0,
            &groups.1,
            granule,
        );
    }
    {
        let [t, e, ms, sp] = &mut b.arr;
        unsafe {
            c::cref_grp_data_groupShortData(
                b.spec.as_mut_ptr(),
                t.as_mut_ptr(),
                e.as_mut_ptr(),
                ms.as_mut_ptr(),
                sp.as_mut_ptr(),
                sfb_cnt,
                sfb_active,
                offsets.as_ptr(),
                min_snr.as_ptr(),
                b.grouped_offset.as_mut_ptr(),
                &mut b.max_sfb,
                b.grouped_min_snr.as_mut_ptr(),
                groups.0,
                groups.1.as_ptr(),
                granule,
            )
        };
    }
    // The lines without a defined value in the reference.
    let mut defined = vec![false; a.spec.len()];
    let mut base = 0;
    for g in 0..groups.0 as usize {
        let len = groups.1[g] * offsets[sfb_active as usize];
        for k in base..base + len {
            defined[k as usize] = true;
        }
        base += groups.1[g] * offsets[sfb_cnt as usize];
    }
    for k in 0..granule as usize {
        if !defined[k] {
            assert_eq!(a.spec[k], 0, "{what}: line {k} without a defined reference value");
            a.spec[k] = b.spec[k];
        }
    }
    assert_eq!(a, b, "{what}");
}

fn random_groups(r: &mut Rng) -> (i32, [i32; 4]) {
    match r.next_u32() % 4 {
        0 => (2, [4, 4, 0, 0]),
        1 => {
            // a random partition of the 8 windows into 1..=4 groups
            let n = r.range(1, 4) as usize;
            let mut g = [0; 4];
            let mut left = 8;
            for i in 0..n {
                g[i] = if i == n - 1 { left } else { r.range(1, left - (n - 1 - i) as i32) };
                left -= g[i];
            }
            (n as i32, g)
        }
        _ => {
            let row = (r.next_u32() % 8) as usize;
            let mut g = [0; 4];
            g.copy_from_slice(&SUGGESTED_GROUPING_TABLE[row * 4..row * 4 + 4]);
            (4, g)
        }
    }
}

fn random_data(r: &mut Rng, spec: Vec<i32>) -> Data {
    let mut arr = [[0i32; SFB_UNION_LEN]; 4];
    for a in arr.iter_mut() {
        let shift = r.next_u32() % 8;
        for x in a.iter_mut() {
            *x = match r.next_u32() % 8 {
                0 => r.i32(),
                1 => MAXVAL_DBL - (r.next_u32() % 1000) as i32,
                2 => 0,
                _ => ((r.next_u32() >> 1) >> shift) as i32,
            };
        }
    }
    Data {
        spec,
        arr,
        grouped_offset: (0..61).map(|_| r.i32()).collect(),
        max_sfb: -5,
        grouped_min_snr: (0..60).map(|_| r.i32()).collect(),
    }
}

fn short_confs() -> Vec<(String, PsyConfiguration, i32)> {
    configs()
        .into_iter()
        .filter_map(|c| c.short.map(|s| (c.name.clone(), s, c.granule as i32)))
        .collect()
}

fn random_spec(r: &mut Rng, n: usize) -> Vec<i32> {
    let mut s = vec![0i32; n];
    let mut k = 0;
    while k < n {
        let run = r.range(1, 40) as usize;
        let zero = r.next_u32() % 3 == 0;
        for x in s.iter_mut().skip(k).take(run) {
            *x = if zero { 0 } else { r.i32() };
        }
        k += run;
    }
    s
}

#[test]
fn group_short_data_random() {
    let mut r = Rng::new(81);
    let confs = short_confs();
    for it in 0..6000 {
        let (name, pc, granule) = &confs[(r.next_u32() as usize) % confs.len()];
        let (sfb_cnt, offsets, granule) = if r.next_u32() % 4 == 0 {
            // a random layout of up to 15 bands within the short window
            let gls = granule / 8;
            let n = r.range(1, 15);
            let mut o = vec![0i32];
            for _ in 0..n {
                let last = *o.last().unwrap();
                o.push((last + r.range(0, 16)).min(gls));
            }
            o.resize(16, gls);
            (n, o, *granule)
        } else {
            (pc.sfb_cnt, pc.sfb_offset.to_vec(), *granule)
        };
        let sfb_active = if r.next_u32() % 2 == 0 { pc.sfb_active.min(sfb_cnt) } else { r.range(0, sfb_cnt) };
        let spec = random_spec(&mut r, 1024);
        let d = random_data(&mut r, spec);
        let min_snr: Vec<i32> = (0..64).map(|_| r.i32()).collect();
        let groups = random_groups(&mut r);
        let what = format!("iteration {it} {name} sfbCnt {sfb_cnt} sfbActive {sfb_active} groups {groups:?}");
        check(&d, sfb_cnt, sfb_active, &offsets, &min_snr, groups, granule, &what);
    }
}

#[test]
fn group_short_data_realistic() {
    let mut r = Rng::new(82);
    let mut count = 0;
    for conf in configs() {
        let Some(pc) = conf.short else { continue };
        for frame in frames(&conf, 16) {
            if !frame.short {
                continue;
            }
            for ch in 0..2 {
                let mut spec = frame.spectrum[ch].clone();
                spec.resize(1024, 0);
                let d = random_data(&mut r, spec);
                let what = format!("{} groups {:?}", conf.name, frame.groups);
                check(
                    &d,
                    pc.sfb_cnt,
                    pc.sfb_active,
                    &pc.sfb_offset,
                    &pc.sfb_min_snr_ld_data,
                    frame.groups,
                    conf.granule as i32,
                    &what,
                );
                count += 1;
            }
        }
    }
    assert!(count > 50, "{count}");
}

#[test]
fn union_size() {
    assert_eq!(unsafe { c::cref_grp_data_sizeof_sfb_threshold() } as usize, 4 * SFB_UNION_LEN);
}

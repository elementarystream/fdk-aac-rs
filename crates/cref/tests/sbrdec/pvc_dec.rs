// Differential tests of fdk_aac_rs_sbrdec::pvc_dec against the C++ reference.
//
// Sequences of PVC frames as in sbr_dec() (pvcInitFrame, pvcDecodeFrame, pvcEndFrame, then
// expandPredEsg for every time slot) with random modes (also legacy SBR and invalid ones),
// smoothing lengths (also invalid ones), rates, crossover bands, borders, PVC IDs, QMF data and
// exponents, starting from random static data; the static data, the complete dynamic data and
// the expanded energies are compared after every frame. Real data: tests/sbrdec/e2e.rs.

use super::env_extr::ptr;
use fdk_aac_rs_cref::sbrdec_pvc_dec::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::pvc_dec::*;
use fdk_aac_rs_sbrdec::sbr_ram::{pod_bytes, pod_bytes_mut};

/// The values of the dynamic data in the order of dump_dynamic() of shim/sbrdec_pvc_dec.cpp.
fn dynamic(d: &PvcDynamicData, id_set: bool) -> Vec<i32> {
    let mut v = vec![d.pvc_mode as i32, d.pvc_border0 as i32, d.kx as i32, d.rate as i32, d.ns as i32];
    for k in 0..PVC_NTIMESLOT {
        v.push(if id_set { d.p_pvc_id[k] as i32 } else { -999 });
    }
    v.push(d.past_esg_slots_avail as i32);
    if d.p_sc_coeffs.is_empty() {
        v.push(-999);
    } else {
        let nsc = if [16, 12, 4, 3].contains(&d.ns) { d.ns as usize } else { 0 };
        v.push(nsc as i32);
        v.extend(d.p_sc_coeffs[..nsc].iter().map(|&x| x as i32));
    }
    v.extend(d.sg_offset_low.iter().map(|&x| x as i32));
    v.extend(d.sg_offset_high_kx.iter().map(|&x| x as i32));
    v.push(d.nb_high as i32);
    for k in 0..=PVC_NBLOW {
        v.push(d.p_scaling_coef.get(k).map_or(-999, |&x| x as i32));
    }
    v.push(d.p_pvc_tab1.get(1).map_or(-999, |&x| x as i32));
    v.push(d.p_pvc_tab2.get(1).map_or(-999, |&x| x as i32));
    for k in 0..2 {
        v.push(d.p_pvc_tab1_dp.get(k).map_or(-999, |&x| x as i32));
    }
    for t in 0..PVC_NTIMESLOT {
        v.extend(d.pred_esg[t].iter());
    }
    v.extend(d.pred_esg_exp.iter());
    v.push(d.pred_esg_exp_max);
    v
}

#[test]
fn pvc_frames() {
    let mut rng = Rng::new(0x9c);
    let mut decoded = 0;
    for it in 0..3000 {
        let mut sc = PvcStaticData::ZERO;
        for b in pod_bytes_mut(&mut sc).iter_mut() {
            *b = rng.next_u32() as u8;
        }
        sc.esg_slot_index = rng.range(0, 15) as u8;
        sc.pvc_border0 = rng.range(0, 15) as u8;
        sc.pvc_mode_last = rng.range(0, 2) as u8;
        sc.kx_last = rng.range(8, 40) as u8;
        for row in sc.esg.iter_mut() {
            for x in row.iter_mut() {
                *x = rng.i32() >> rng.range(0, 8);
            }
        }
        let mut sp = sc;
        let rate = [2, 4][rng.range(0, 1) as usize];
        let mut kx = rng.range(8, 40);
        for frame in 0..8 {
            let mode = match rng.next_u32() % 10 {
                0 => 0,
                1 => 3,
                k => 1 + (k as i32 & 1),
            };
            let ns = match rng.next_u32() % 12 {
                0 => rng.range(0, 20),
                k => [16, 12, 4, 3][(k % 4) as usize],
            };
            if rng.next_u32() % 4 == 0 {
                kx = rng.range(8, 40);
            }
            let border0 = rng.range(0, 15);
            let mut id = [0u8; PVC_NTIMESLOT];
            let base = rng.range(0, 127) as u8;
            for x in id.iter_mut() {
                *x = if rng.next_u32() % 3 == 0 { rng.range(0, 127) as u8 } else { base };
            }
            let n_slots = 16 * rate + 12;
            let shift = rng.range(0, 30);
            let re: Vec<i32> = (0..n_slots * 64).map(|_| rng.i32() >> shift).collect();
            let im: Vec<i32> = (0..n_slots * 64).map(|_| rng.i32() >> shift).collect();
            let overlap = [0, 6, 12][rng.range(0, 2) as usize];
            let (e_ov, e_cur) = (rng.range(-20, 20), rng.range(-20, 20));
            // (the decoder passes MAX_FREQ_COEFFS; at least sg_offset_high_kx[nbHigh] <= 36 values
            // are written)
            let len_out = rng.range(36, 56);

            let mut dc = vec![0i32; 512];
            let mut dn = 0;
            let mut xc = vec![0i32; 16 * len_out as usize];
            let mut xec = vec![0i8; 16 * len_out as usize];
            let (mut rc, mut ic) = (re.clone(), im.clone());
            let mut idc = id;
            let res_c = unsafe {
                cref_sbrdec_pvc_frame(
                    ptr(&mut sc),
                    mode,
                    ns,
                    rate,
                    kx,
                    border0,
                    idc.as_mut_ptr(),
                    rc.as_mut_ptr(),
                    ic.as_mut_ptr(),
                    n_slots,
                    overlap,
                    e_ov,
                    e_cur,
                    dc.as_mut_ptr(),
                    &mut dn,
                    len_out,
                    xc.as_mut_ptr(),
                    xec.as_mut_ptr(),
                )
            };
            dc.truncate(dn as usize);

            let mut d = PvcDynamicData::default();
            let mut re2 = re.clone();
            let mut im2 = im.clone();
            let res_p = pvc_init_frame(&mut sp, &mut d, mode as u8, ns as u8, rate, kx, border0, &id);
            let mut xp = vec![0i32; 16 * len_out as usize];
            let mut xep = vec![0i8; 16 * len_out as usize];
            {
                let r: Vec<&mut [i32]> = re2.chunks_mut(64).collect();
                let i: Vec<&mut [i32]> = im2.chunks_mut(64).collect();
                if res_p == 0 && mode > 0 {
                    pvc_decode_frame(&mut sp, &mut d, &r, &i, overlap, e_ov, e_cur);
                    decoded += 1;
                }
            }
            pvc_end_frame(&mut sp, &mut d);
            if res_p == 0 && mode > 0 {
                for t in 0..PVC_NTIMESLOT {
                    let o = t * len_out as usize;
                    expand_pred_esg(&d, t as i32, len_out, &mut xp[o..o + len_out as usize], &mut xep[o..o + len_out as usize]);
                }
            }
            // (pPvcID is set by the reference only after the mode check)
            let id_set = mode == 1 || mode == 2;
            let what = format!("{it}/{frame}: mode {mode} ns {ns} rate {rate} kx {kx} border0 {border0}");
            assert_eq!(res_p, res_c, "{what}: result");
            assert_eq!(pod_bytes(&sp), pod_bytes(&sc), "{what}: static data");
            let dp = dynamic(&d, id_set);
            if let Some(k) = (0..dp.len().max(dc.len())).find(|&k| dp.get(k) != dc.get(k)) {
                panic!("{what}: dynamic data value {k}: port {:?}, reference {:?}", dp.get(k), dc.get(k));
            }
            assert_eq!((xp, xep), (xc, xec), "{what}: expandPredEsg");
        }
    }
    assert!(decoded > 10_000, "{decoded} decoded frames");
}

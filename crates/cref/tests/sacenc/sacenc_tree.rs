// Differential tests of fdk_aac_rs_sacenc::sacenc_tree against the C++ reference.
//
// The initialization with every band configuration, quantization and tree mode (valid and
// invalid), and sequences of applications to stereo hybrid data with the analysis windows of
// the encoder (and random ones), frame keeping with every avoid_keep count, both parameter
// sets: the spatial frame, the windowed hybrid data and the whole tree state compared.

use fdk_aac_rs_cref::sacenc_sacenc_tree as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::FixpDpk;
use fdk_aac_rs_sacenc::sacenc_bitstream::SpatialFrame;
use fdk_aac_rs_sacenc::sacenc_paramextract::fdk_sacenc_calc_parameter_band2_hybrid_band_offset;
use fdk_aac_rs_sacenc::sacenc_tree as r;
use fdk_aac_rs_sacenc::sacenc_tree::*;

use super::sacenc_bitstream::{frame_bytes, frame_from, rand_frame, rand_ssc};
use super::sacenc_paramextract::{stereo_hybrid, tto_ints};
use super::sacenc_vectorfunctions::flat;

pub const TREE_IMAGE: usize = 224;

pub fn tree_ints(t: &SpaceTree) -> Vec<i32> {
    let mut v = vec![t.mode, t.descr.n_ott_boxes as i32, t.descr.n_in_channels as i32, t.descr.n_out_channels as i32];
    match t.tto_box[0].as_deref() {
        Some(b) => v.extend(tto_ints(b)),
        None => v.extend(std::iter::repeat_n(-12345, 156)),
    }
    v.extend([
        t.n_param_bands as i32,
        t.b_use_coarse_quant_tto_icc as i32,
        t.b_use_coarse_quant_tto_cld as i32,
        t.quant_mode,
        t.frame_count,
        t.b_frame_keep as i32,
    ]);
    v.extend(t.p_cld_prev[0].iter().map(|&x| x as i32));
    v.extend(t.p_icc_prev[0].iter().map(|&x| x as i32));
    v.push(t.n_channels_in_max as i32);
    v.push(t.n_hybrid_bands_max as i32);
    v
}

fn check(h: *mut core::ffi::c_void, t: &SpaceTree, what: &str) {
    let mut v = vec![0i32; TREE_IMAGE];
    let n = unsafe { c::cref_sacenc_tree_state(h, v.as_mut_ptr()) } as usize;
    assert_eq!(n, TREE_IMAGE);
    assert_eq!(v, tree_ints(t), "{what}: tree state");
}

/// An analysis window as the encoder builds it (frame keeping or not), or a random one.
fn window(rng: &mut Rng, nts: i32, keep: bool) -> Vec<i32> {
    let mut w = vec![0i32; 48];
    let t = nts as usize;
    match rng.range(0, 3) {
        0 | 1 => {
            // no transient: [0 .. T/2) taper (max), [T/2 .. 1.5T) max, then zeros
            for v in w.iter_mut().take(3 * t / 2) {
                *v = 0x7FFF_FFFF;
            }
            if keep {
                let mut k = vec![0i32; 48];
                k[t..(2 * t + t)].copy_from_slice(&w[..(2 * t)]);
                for v in k.iter_mut().take(t) {
                    *v = if rng.range(0, 1) == 0 { 0 } else { 0x7FFF_FFFF };
                }
                w = k;
            }
        }
        _ => {
            for v in w.iter_mut().take(if keep { 3 * t } else { 2 * t }) {
                *v = match rng.range(0, 2) {
                    0 => 0,
                    1 => 0x7FFF_FFFF,
                    _ => rng.range(0, i32::MAX),
                };
            }
        }
    }
    w
}

#[test]
fn init_and_apply() {
    let mut rng = Rng::new(1);
    let mut offsets = vec![0u8; 28];
    for seq in 0..600 {
        let h = unsafe { c::cref_sacenc_tree_open() };
        let mut t = None;
        assert_eq!(r::fdk_sacenc_space_tree_open(Some(&mut t)), 0);
        let mut t = t.unwrap();
        check(h, &t, "open");

        let nb = if rng.range(0, 10) == 0 { [0, 3, 40][rng.range(0, 2) as usize] } else { [4, 5, 7, 9, 12, 15, 23][rng.range(0, 6) as usize] };
        let nh = [32, 64][rng.range(0, 1) as usize];
        fdk_sacenc_calc_parameter_band2_hybrid_band_offset(nb, nh, &mut offsets);
        let setup = SpaceTreeSetup {
            n_param_bands: nb as u8,
            b_use_coarse_quant_tto_cld: rng.range(0, 1) as u8,
            b_use_coarse_quant_tto_icc: rng.range(0, 1) as u8,
            quant_mode: if rng.range(0, 8) == 0 { rng.range(-1, 3) } else { 0 },
            mode: if rng.range(0, 10) == 0 { rng.range(0, 9) } else { SPACETREE_212 },
            n_channels_in_max: 2,
            n_hybrid_bands_max: nh as u8,
        };
        let keep = rng.range(0, 1);
        let ec = unsafe {
            c::cref_sacenc_tree_init(
                h, nb, setup.b_use_coarse_quant_tto_cld as i32, setup.b_use_coarse_quant_tto_icc as i32,
                setup.quant_mode, setup.mode, 2, nh, offsets.as_mut_ptr(), keep,
            )
        };
        let er = r::fdk_sacenc_space_tree_init(&mut t, &setup, true, keep);
        assert_eq!(ec, er, "init seq {seq}");
        check(h, &t, &format!("init seq {seq}"));
        let mut d = [0u8; 3];
        unsafe { c::cref_sacenc_tree_getDescription(h, d.as_mut_ptr()) };
        let mut dr = SpaceTreeDescription::default();
        r::fdk_sacenc_space_tree_get_description(&t, &mut dr);
        assert_eq!(d, [dr.n_ott_boxes, dr.n_in_channels, dr.n_out_channels]);
        if er != 0 || t.tto_box[0].as_ref().unwrap().n_parameter_bands == 0 {
            unsafe { c::cref_sacenc_tree_close(h) };
            continue;
        }

        let nfts = rng.range(1, 16);
        let ssc = rand_ssc(&mut rng);
        for fr in 0..8 {
            let mut frame = rand_frame(&mut rng, &ssc);
            for ps in 0..2 {
                let (a, b) = stereo_hybrid(&mut rng, 40);
                let mut hyb: Vec<Vec<Vec<FixpDpk>>> = vec![a, b];
                let mut hc: Vec<i32> = hyb.iter().flat_map(|ch| flat(ch)).collect();
                let w = window(&mut rng, nfts, keep != 0);
                let mut wc = w.clone();
                let nts = {
                    let mut i = 47;
                    while i >= 0 && w[i as usize] == 0 {
                        i -= 1;
                    }
                    (i + 1).min(40)
                };
                let st = w.iter().position(|&x| x != 0).unwrap_or(48).min(nts as usize) as i32;
                let nch = if rng.range(0, 20) == 0 { 1 } else { 2 };
                let nhb = if rng.range(0, 20) == 0 { nh + 1 } else { nh };
                let avoid = rng.range(0, 3);
                let mut scale = [rng.range(5, 16), rng.range(5, 16)];
                let mut fc = frame_bytes(&frame);
                let ec = unsafe {
                    c::cref_sacenc_tree_apply(
                        h, ps, nch, nts, st, nhb, wc.as_mut_ptr(), hc.as_mut_ptr(), 40, 64, fc.as_mut_ptr(), avoid, scale.as_mut_ptr(),
                    )
                };
                let er = r::fdk_sacenc_space_tree_apply(&mut t, &offsets, ps, nch, nts, st, nhb, &w, &mut hyb, &mut frame, avoid, &scale);
                let what = format!("apply seq {seq} frame {fr} ps {ps}");
                assert_eq!(ec, er, "{what}");
                assert_eq!(fc, frame_bytes(&frame), "{what}: frame");
                assert!(hc == hyb.iter().flat_map(|ch| flat(ch)).collect::<Vec<_>>(), "{what}: hybrid data");
                check(h, &t, &what);
                frame = frame_from(&fc);
            }
            let _ = SpatialFrame::default();
        }
        assert_eq!(unsafe { c::cref_sacenc_tree_close(h) }, r::fdk_sacenc_space_tree_close(Some(&mut Some(t))));
    }
    assert_eq!(unsafe { c::cref_sacenc_tree_open_null() }, r::fdk_sacenc_space_tree_open(None));
    assert_eq!(unsafe { c::cref_sacenc_tree_close_null() }, r::fdk_sacenc_space_tree_close(None));
    for nb in [0, 4, 5, 7, 9, 12, 15, 23, 40] {
        for band in -1..66 {
            assert_eq!(unsafe { c::cref_sacenc_tree_Hybrid2ParamBand(nb, band) }, r::fdk_sacenc_space_tree_hybrid2_param_band(nb, band));
        }
    }
}

/// The table `id` of the reference as INTs.
fn c_table(f: unsafe extern "C" fn(i32, *mut i32) -> i32, id: i32) -> Vec<i32> {
    let mut v = vec![0i32; 1024];
    let n = unsafe { f(id, v.as_mut_ptr()) } as usize;
    v.truncate(n);
    v
}

#[test]
fn tables() {
    use fdk_aac_rs_sacenc::sacenc_tree::{TREE_CONFIG_TABLE_RAW, TREE_SETUP_TABLE_RAW};
    let f = c::cref_sacenc_tree_table;
    assert_eq!(c_table(f, 0), TREE_CONFIG_TABLE_RAW.to_vec(), "treeConfigTable");
    assert_eq!(c_table(f, 1), TREE_SETUP_TABLE_RAW.to_vec(), "treeSetupTable");
    // The structures built from the dumps, through the functions that use them: the 212 tree
    // description and the 212 setup (2 channels in, 1 TTO box on channels 0 and 1).
    let mut t = None;
    r::fdk_sacenc_space_tree_open(Some(&mut t));
    let mut t = t.unwrap();
    for (mode, ok) in [(SPACETREE_212, true), (SPACETREE_INVALID_MODE, false)] {
        let setup = SpaceTreeSetup { n_param_bands: 15, mode, n_channels_in_max: 2, n_hybrid_bands_max: 64, ..Default::default() };
        let e = r::fdk_sacenc_space_tree_init(&mut t, &setup, true, 0);
        assert_eq!(e == 0, ok);
        if ok {
            let mut d = SpaceTreeDescription::default();
            r::fdk_sacenc_space_tree_get_description(&t, &mut d);
            let k = 4;
            assert_eq!([d.n_ott_boxes as i32, d.n_in_channels as i32, d.n_out_channels as i32], TREE_CONFIG_TABLE_RAW[k + 1..k + 4]);
        }
    }
}

// Differential tests of fdk_aac_rs_sacenc::sacenc_paramextract against the C++ reference.
//
// The table lookups for every band configuration, the parameter band offsets for every band
// configuration and hybrid band count, ICC (correlation and coherence), CLD, quantization and
// broadband cues on random input, and the TTO box (initialization with every configuration,
// application to hybrid data of two channels with level differences, correlation and
// anti-correlation, every window range) with the outputs and the whole box state compared.

use fdk_aac_rs_cref::sacenc_sacenc_paramextract as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::FixpDpk;
use fdk_aac_rs_sacenc::sacenc_paramextract as r;
use fdk_aac_rs_sacenc::sacenc_paramextract::*;

use super::sacenc_vectorfunctions::{flat, val};

pub const TTO_IMAGE: usize = 156;

/// The TTO_BOX image of `sacenc_tto_state()`.
pub fn tto_ints(b: &TtoBox) -> Vec<i32> {
    let mut v = Vec::with_capacity(TTO_IMAGE);
    v.extend_from_slice(&b.p_cld_fdk);
    v.extend_from_slice(&b.p_icc_fdk);
    v.extend_from_slice(&b.p_cld_quant_fdk);
    let id = |coarse: u8| if !b.tables_set { 0 } else if coarse != 0 { 2 } else { 1 };
    v.push(id(b.b_use_coarse_quant_icc));
    v.push(id(b.b_use_coarse_quant_cld));
    v.push(id(b.b_use_coarse_quant_cld));
    v.extend(b.p_cld_eb_q_idx.iter().map(|&x| x as i32));
    v.extend(b.p_icc_downmix_idx.iter().map(|&x| x as i32));
    v.push(b.p_parameter_band2_hybrid_band_offset_set as i32);
    v.push(b.p_subband_imag_sign_set as i32);
    for x in [
        b.n_hybrid_bands_max as i32,
        b.n_parameter_bands as i32,
        b.b_frame_keep as i32,
        b.icc_correlation_coherence_border as i32,
        b.box_quant_mode,
        b.n_icc_quant_steps as i32,
        b.n_icc_quant_offset as i32,
        b.n_cld_quant_steps as i32,
        b.n_cld_quant_offset as i32,
        b.b_use_coarse_quant_cld as i32,
        b.b_use_coarse_quant_icc as i32,
    ] {
        v.push(x);
    }
    v
}

const CONFIGS: [i32; 10] = [4, 5, 7, 9, 12, 15, 23, 0, 3, 40];

#[test]
fn tables_and_offsets() {
    for &cfg in &CONFIGS {
        let mut v = [0i32; 3];
        let rc = unsafe { c::cref_sacenc_paramextract_getBoxSubbandSetup(cfg, v.as_mut_ptr()) };
        match r::get_box_subband_setup(cfg) {
            None => assert_eq!(rc, -1),
            Some(s) => {
                assert_eq!(rc, 0);
                assert_eq!(v, [s.subband_config, s.n_parameter_bands as i32, s.icc_correlation_coherence_border as i32]);
            }
        }
        assert_eq!(unsafe { c::cref_sacenc_paramextract_getNumberParameterBands(cfg) }, r::get_number_parameter_bands(cfg));
        let mut t = [0u8; 64];
        let has = unsafe { c::cref_sacenc_paramextract_getSubband2ParameterIndex(cfg, t.as_mut_ptr()) };
        assert_eq!(has != 0, r::get_subband2_parameter_index(cfg).is_some());
        if let Some(rt) = r::get_subband2_parameter_index(cfg) {
            assert_eq!(&t, rt);
        }
        for co in 0..2 {
            assert_eq!(
                unsafe { c::cref_sacenc_paramextract_getIccCorrelationCoherenceBorder(cfg, co) },
                r::get_icc_correlation_coherence_border(cfg, co)
            );
        }
        for sb in -2..66 {
            assert_eq!(unsafe { c::cref_sacenc_paramextract_subband2ParamBand(cfg, sb) }, r::fdk_sacenc_subband2_param_band(cfg, sb));
            assert_eq!(
                r::fdk_sacenc_subband2_param_band(cfg, sb),
                fdk_aac_rs_sacenc::sacenc_tree::fdk_sacenc_space_tree_hybrid2_param_band(cfg, sb)
            );
        }
        // Configurations without a setup: arm64 only (no defined x86 reference result; see
        // sacenc_paramextract).
        if r::get_box_subband_setup(cfg).is_some() || !cfg!(any(target_arch = "x86_64", target_arch = "x86")) {
            for nh in 0..=64 {
                let mut oc = [0x77u8; 28];
                let mut or = oc;
                unsafe { c::cref_sacenc_paramextract_calcParameterBand2HybridBandOffset(cfg, nh, oc.as_mut_ptr()) };
                r::fdk_sacenc_calc_parameter_band2_hybrid_band_offset(cfg, nh, &mut or);
                assert_eq!(oc, or, "offsets cfg {cfg} nh {nh}");
            }
        }
    }
    // Every configuration value (arm64 only, see above).
    if !cfg!(any(target_arch = "x86_64", target_arch = "x86")) {
        for cfg in -3..=45 {
            for nh in [1, 32, 64] {
                let mut oc = [0x77u8; 28];
                let mut or = oc;
                unsafe { c::cref_sacenc_paramextract_calcParameterBand2HybridBandOffset(cfg, nh, oc.as_mut_ptr()) };
                r::fdk_sacenc_calc_parameter_band2_hybrid_band_offset(cfg, nh, &mut or);
                assert_eq!(oc, or, "offsets cfg {cfg} nh {nh}");
            }
        }
    }
    let mut s = [0i32; 64];
    unsafe { c::cref_sacenc_paramextract_getSubbandImagSign(s.as_mut_ptr()) };
    assert_eq!(&s, r::fdk_sacenc_get_subband_imag_sign());
}

fn pos(rng: &mut Rng) -> i32 {
    match rng.range(0, 4) {
        0 => 0,
        1 => rng.range(1, i32::MAX),
        2 => rng.range(1, 1000),
        _ => rng.range(1, i32::MAX) >> rng.range(0, 31),
    }
}

#[test]
fn icc_cld_quant() {
    let mut rng = Rng::new(1);
    for it in 0..20000 {
        let n = rng.range(0, 28);
        let border = rng.range(0, 30);
        let mag = rng.next_u32() % 32;
        let p1: Vec<i32> = (0..28).map(|_| pos(&mut rng)).collect();
        let p2: Vec<i32> = (0..28).map(|_| pos(&mut rng)).collect();
        let re: Vec<i32> = (0..28).map(|_| val(&mut rng, mag)).collect();
        let im: Vec<i32> = (0..28).map(|_| val(&mut rng, mag)).collect();
        let mut ic = vec![0x5555i32; 28];
        let mut ir = ic.clone();
        let ec = unsafe { c::cref_sacenc_paramextract_calculateIccFDK(n, border, p1.as_ptr(), p2.as_ptr(), re.as_ptr(), im.as_ptr(), ic.as_mut_ptr()) };
        let er = r::calculate_icc_fdk(n, border, &p1, &p2, &re, &im, &mut ir);
        assert_eq!((ec, &ic), (er, &ir), "calculateIccFDK it {it}");

        let s1 = rng.range(-20, 20);
        let s2 = rng.range(-20, 20);
        let pb1: Vec<i32> = (0..28).map(|_| -rng.range(0, 31)).collect();
        let pb2: Vec<i32> = (0..28).map(|_| -rng.range(0, 31)).collect();
        let mut cc = vec![0x5555i32; 28];
        let mut cr = cc.clone();
        unsafe { c::cref_sacenc_paramextract_CalculateCldFDK(cc.as_mut_ptr(), p1.as_ptr(), p2.as_ptr(), s1, pb1.as_ptr(), s2, pb2.as_ptr(), n) };
        r::calculate_cld_fdk(&mut cr, &p1, &p2, s1, &pb1, s2, &pb2, n);
        assert_eq!(cc, cr, "CalculateCldFDK it {it}");

        // Quantization of the values as the box produces them, and of random ones.
        let tab = rng.range(0, 5);
        let (table, offset, steps): (&[i32], i32, i32) = match tab {
            0 => (&CLD_QUANT_TABLE_FINE_ENC_FDK, 15, 31),
            1 => (&CLD_QUANT_TABLE_COARSE_ENC_FDK, 7, 15),
            2 => (&CLD_QUANT_TABLE_FINE_DEC_FDK, 15, 31),
            3 => (&CLD_QUANT_TABLE_COARSE_DEC_FDK, 7, 15),
            4 => (&ICC_QUANT_TABLE_FINE_FDK, 0, 8),
            _ => (&ICC_QUANT_TABLE_COARSE_FDK, 0, 4),
        };
        let input: Vec<i32> = match rng.range(0, 2) {
            0 => if tab >= 4 { ir.clone() } else { cr.clone() },
            1 => (0..28).map(|_| rng.i32() >> 2).collect(),
            _ => (0..28).map(|_| table[rng.range(0, steps - 1) as usize].wrapping_add(rng.range(-3, 3))).collect(),
        };
        let mut qc = [0x55i8; 28];
        let mut qr = qc;
        unsafe { c::cref_sacenc_paramextract_QuantizeCoefFDK(input.as_ptr(), n, tab, offset, steps, qc.as_mut_ptr()) };
        r::quantize_coef_fdk(&input, n, table, offset, steps, &mut qr);
        assert_eq!(qc, qr, "QuantizeCoefFDK it {it}");
        let mut dc = vec![0x5555i32; 28];
        let mut dr = dc.clone();
        unsafe { c::cref_sacenc_paramextract_deQuantizeCoefFDK(qc.as_ptr(), n, tab, offset, dc.as_mut_ptr()) };
        r::de_quantize_coef_fdk(&qr, n, table, offset, &mut dr);
        assert_eq!(dc, dr, "deQuantizeCoefFDK it {it}");

        let nb = [4, 5, 7, 9, 12, 15, 23][rng.range(0, 6) as usize];
        let mut bc = input.clone();
        let mut br = input.clone();
        unsafe { c::cref_sacenc_paramextract_ApplyBBCuesFDK(bc.as_mut_ptr(), nb) };
        r::apply_bb_cues_fdk(&mut br, nb);
        assert_eq!(bc, br, "ApplyBBCuesFDK it {it}");
    }
}

/// Hybrid data of two channels: `slots` x 64 bands, with a level difference and a
/// correlation between the channels that change over the bands.
pub fn stereo_hybrid(rng: &mut Rng, slots: usize) -> (Vec<Vec<FixpDpk>>, Vec<Vec<FixpDpk>>) {
    let l1 = rng.range(2, 24) as u32;
    let l2 = rng.range(2, 24) as u32;
    let mode = rng.range(0, 4);
    let mut a = vec![vec![FixpDpk::default(); 64]; slots];
    let mut b = vec![vec![FixpDpk::default(); 64]; slots];
    for ts in 0..slots {
        for k in 0..64 {
            let x = FixpDpk::new((rng.next_u32() as i32) >> l1, (rng.next_u32() as i32) >> l1);
            let n = FixpDpk::new((rng.next_u32() as i32) >> l2, (rng.next_u32() as i32) >> l2);
            a[ts][k] = x;
            b[ts][k] = match mode {
                0 => n,
                1 => FixpDpk::new((x.re >> 1).wrapping_add(n.re >> 2), (x.im >> 1).wrapping_add(n.im >> 2)),
                2 => FixpDpk::new((x.re >> 2).wrapping_neg(), (x.im >> 2).wrapping_neg()),
                3 => if k < 20 { FixpDpk::default() } else { n },
                _ => FixpDpk::new(x.im >> 3, x.re >> 3),
            };
        }
    }
    (a, b)
}

#[test]
fn tto_box() {
    let mut rng = Rng::new(2);
    let mut offsets = vec![0u8; 28];
    for it in 0..3000 {
        let h = unsafe { c::cref_sacenc_paramextract_create() };
        let mut bx = None;
        r::fdk_sacenc_create_tto_box(Some(&mut bx));
        let mut bx = bx.unwrap();
        let mut img = vec![0i32; TTO_IMAGE];
        unsafe { c::cref_sacenc_paramextract_state(h, img.as_mut_ptr()) };
        assert_eq!(img, tto_ints(&bx), "create");

        let cfg = if rng.range(0, 10) == 0 { CONFIGS[rng.range(0, 9) as usize] } else { [4, 5, 7, 9, 12, 15, 23][rng.range(0, 6) as usize] };
        let nh = [32, 64][rng.range(0, 1) as usize];
        if rng.range(0, 1) == 0 {
            // A stale entry beyond the bands of 32 hybrid bands (see sacenc_lib).
            r::fdk_sacenc_calc_parameter_band2_hybrid_band_offset(cfg, 64, &mut offsets);
        }
        r::fdk_sacenc_calc_parameter_band2_hybrid_band_offset(cfg, nh, &mut offsets);
        let cc = rng.range(0, 1);
        let ci = rng.range(0, 1);
        let co = rng.range(0, 1);
        let qm = if rng.range(0, 5) == 0 { rng.range(-1, 4) } else { rng.range(0, 2) };
        let keep = rng.range(0, 1);
        let has = rng.range(0, 20) != 0;
        let ec = unsafe {
            c::cref_sacenc_paramextract_init(h, cc, ci, co, cfg, qm, nh, keep, if has { offsets.as_mut_ptr() } else { core::ptr::null_mut() })
        };
        let cfgr = TtoBoxConfig {
            b_use_coarse_quant_cld: cc as u8,
            b_use_coarse_quant_icc: ci as u8,
            b_use_coherence_icc_only: co as u8,
            subband_config: cfg,
            box_quant_mode: qm,
            n_hybrid_bands_max: nh as u8,
            b_frame_keep: keep as u8,
        };
        let er = r::fdk_sacenc_init_tto_box(&mut bx, &cfgr, has);
        assert_eq!(ec, er, "init it {it}");
        unsafe { c::cref_sacenc_paramextract_state(h, img.as_mut_ptr()) };
        assert_eq!(img, tto_ints(&bx), "init it {it}");
        if er != 0 || bx.n_parameter_bands == 0 {
            unsafe { c::cref_sacenc_paramextract_destroy(h) };
            continue;
        }

        for k in 0..3 {
            let slots = 40usize;
            let (d1, d2) = stereo_hybrid(&mut rng, slots);
            let nts = rng.range(1, 40);
            let st = if rng.range(0, 2) == 0 { 0 } else { rng.range(0, nts - 1) };
            let nhb = if rng.range(0, 10) == 0 { rng.range(-1, 65) } else { nh };
            let bb = (rng.range(0, 7) == 0) as i32;
            let (s1, s2) = (rng.range(5, 16), rng.range(5, 16));
            let mut icc_c = [0x11i8; 23];
            let mut cld_c = [0x11i8; 23];
            let (mut icq_c, mut clq_c) = (9u8, 9u8);
            let mut icc_r = icc_c;
            let mut cld_r = cld_c;
            let (mut icq_r, mut clq_r) = (9u8, 9u8);
            let ec = unsafe {
                c::cref_sacenc_paramextract_apply(
                    h, nts, st, nhb, flat(&d1).as_ptr(), flat(&d2).as_ptr(), slots as i32, 64,
                    icc_c.as_mut_ptr(), &mut icq_c, cld_c.as_mut_ptr(), &mut clq_c, bb, s1, s2,
                )
            };
            let er = r::fdk_sacenc_apply_tto_box(
                &mut bx, &offsets, nts, st, nhb, &d1, &d2, &mut icc_r, &mut icq_r, &mut cld_r, &mut clq_r, bb, s1, s2,
            );
            let what = format!("apply it {it} k {k} cfg {cfg} nh {nh} qm {qm}");
            assert_eq!(ec, er, "{what}");
            assert_eq!((icc_c, cld_c, icq_c, clq_c), (icc_r, cld_r, icq_r, clq_r), "{what}");
            unsafe { c::cref_sacenc_paramextract_state(h, img.as_mut_ptr()) };
            assert_eq!(img, tto_ints(&bx), "{what}: state");
        }
        assert_eq!(unsafe { c::cref_sacenc_paramextract_destroy(h) }, 0);
    }
    assert_eq!(unsafe { c::cref_sacenc_paramextract_create_null() }, r::fdk_sacenc_create_tto_box(None));
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
    let f = c::cref_sacenc_paramextract_table;
    let i = |t: &[u8]| t.iter().map(|&x| x as i32).collect::<Vec<_>>();
    let sub: [&[u8; 64]; 7] = [
        &SUBBAND2_PARAMETER4_LD,
        &SUBBAND2_PARAMETER5_LD,
        &SUBBAND2_PARAMETER7_LD,
        &SUBBAND2_PARAMETER9_LD,
        &SUBBAND2_PARAMETER12_LD,
        &SUBBAND2_PARAMETER15_LD,
        &SUBBAND2_PARAMETER23_LD,
    ];
    for (k, t) in sub.iter().enumerate() {
        assert_eq!(c_table(f, k as i32), i(&t[..]), "subband2Parameter table {k}");
        // SUBBAND2_PARAMETER_TABLES: the index order of BOX_SUBBAND_SETUP_RAW.
        assert!(core::ptr::eq(*t, SUBBAND2_PARAMETER_TABLES[k]), "SUBBAND2_PARAMETER_TABLES[{k}]");
    }
    assert_eq!(c_table(f, 7), SUBBAND_IMAG_SIGN_LD.to_vec(), "subbandImagSign_Ld");
    let q: [&[i32]; 6] = [
        &CLD_QUANT_TABLE_FINE_ENC_FDK,
        &CLD_QUANT_TABLE_COARSE_ENC_FDK,
        &CLD_QUANT_TABLE_FINE_DEC_FDK,
        &CLD_QUANT_TABLE_COARSE_DEC_FDK,
        &ICC_QUANT_TABLE_FINE_FDK,
        &ICC_QUANT_TABLE_COARSE_FDK,
    ];
    for (k, t) in q.iter().enumerate() {
        assert_eq!(c_table(f, 8 + k as i32), t.to_vec(), "quantization table {k}");
    }
    let mut bss = Vec::new();
    for s in &BOX_SUBBAND_SETUP {
        let k = sub.iter().position(|t| core::ptr::eq(*t, s.p_subband2_parameter_index_ld)).map_or(-1, |k| k as i32);
        bss.extend([s.subband_config, s.n_parameter_bands as i32, k, s.icc_correlation_coherence_border as i32]);
    }
    assert_eq!(c_table(f, 14), bss, "boxSubbandSetup");
    assert_eq!(c_table(f, 14), BOX_SUBBAND_SETUP_RAW.to_vec(), "boxSubbandSetup (dump)");
}

// Differential tests of fdk_aac_rs_sacenc::sacenc_bitstream against the C++ reference.
//
// The structures are exchanged as byte images (layout checked with offsetof/sizeof). The
// helpers (stride, index lookups, fine/coarse conversion, parameter set duplication) on random
// input; ecData, writeFramingInfo and writeSmgData after a random number of bits; the
// SpatialSpecificConfig for every band count and sampling rate (and invalid ones); and sequences
// of random spatial frames written by a formatter instance on each side (frame data, the
// previous data, the output and the bit counts compared after every frame, with the buffer
// sizes of the encoder and of the AAC encoder).

use fdk_aac_rs_cref::sacenc_sacenc_bitstream as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacenc::sacenc_bitstream as r;
use fdk_aac_rs_sacenc::sacenc_bitstream::*;

use super::sacenc_nlc_enc::Bs;

/// Size of the SPATIALFRAME image.
pub const FRAME_SIZE: usize = 199;
/// Size of the PREV_OTTDATA image.
pub const PREV_SIZE: usize = 50;

fn lossless_bytes(l: &LosslessData, out: &mut Vec<u8>) {
    out.extend_from_slice(&l.bs_xxx_data_mode[0]);
    out.extend_from_slice(&l.bs_data_pair[0]);
    out.extend_from_slice(&l.bs_quant_coarse_xxx[0]);
    out.extend_from_slice(&l.bs_freq_res_stride_xxx[0]);
}

fn lossless_from(b: &[u8]) -> LosslessData {
    let mut l = LosslessData::default();
    l.bs_xxx_data_mode[0].copy_from_slice(&b[0..2]);
    l.bs_data_pair[0].copy_from_slice(&b[2..4]);
    l.bs_quant_coarse_xxx[0].copy_from_slice(&b[4..6]);
    l.bs_freq_res_stride_xxx[0].copy_from_slice(&b[6..8]);
    l
}

fn smg_bytes(s: &SmgData, out: &mut Vec<u8>) {
    out.extend_from_slice(&s.bs_smooth_mode);
    out.extend_from_slice(&s.bs_smooth_time);
    out.extend_from_slice(&s.bs_freq_res_stride);
    for r in &s.bs_smg_data {
        out.extend_from_slice(r);
    }
}

fn smg_from(b: &[u8]) -> SmgData {
    let mut s = SmgData::default();
    s.bs_smooth_mode.copy_from_slice(&b[0..2]);
    s.bs_smooth_time.copy_from_slice(&b[2..4]);
    s.bs_freq_res_stride.copy_from_slice(&b[4..6]);
    s.bs_smg_data[0].copy_from_slice(&b[6..29]);
    s.bs_smg_data[1].copy_from_slice(&b[29..52]);
    s
}

fn i8s(v: &[i8]) -> impl Iterator<Item = u8> + '_ {
    v.iter().map(|&x| x as u8)
}

/// The SPATIALFRAME image.
pub fn frame_bytes(f: &SpatialFrame) -> Vec<u8> {
    let mut o = vec![
        f.framing_info.bs_framing_type,
        f.framing_info.num_param_sets,
        f.framing_info.bs_param_slots[0],
        f.framing_info.bs_param_slots[1],
        f.bs_independency_flag,
    ];
    for ps in 0..2 {
        o.extend(i8s(&f.ott_data.cld[0][ps]));
    }
    for ps in 0..2 {
        o.extend(i8s(&f.ott_data.icc[0][ps]));
    }
    smg_bytes(&f.smg_data, &mut o);
    o.push(f.temp_shape_data.bs_env_shape_channel[0]);
    o.extend_from_slice(&f.temp_shape_data.bs_env_shape_data[0]);
    lossless_bytes(&f.cld_lossless_data, &mut o);
    lossless_bytes(&f.icc_lossless_data, &mut o);
    o.push(f.b_use_bb_cues);
    assert_eq!(o.len(), FRAME_SIZE);
    o
}

/// The SPATIALFRAME of an image.
pub fn frame_from(b: &[u8]) -> SpatialFrame {
    let mut f = SpatialFrame::default();
    f.framing_info.bs_framing_type = b[0];
    f.framing_info.num_param_sets = b[1];
    f.framing_info.bs_param_slots = [b[2], b[3]];
    f.bs_independency_flag = b[4];
    for ps in 0..2 {
        for k in 0..MAX_NUM_BINS {
            f.ott_data.cld[0][ps][k] = b[5 + ps * 23 + k] as i8;
            f.ott_data.icc[0][ps][k] = b[51 + ps * 23 + k] as i8;
        }
    }
    f.smg_data = smg_from(&b[97..149]);
    f.temp_shape_data.bs_env_shape_channel[0] = b[149];
    f.temp_shape_data.bs_env_shape_data[0].copy_from_slice(&b[150..182]);
    f.cld_lossless_data = lossless_from(&b[182..190]);
    f.icc_lossless_data = lossless_from(&b[190..198]);
    f.b_use_bb_cues = b[198];
    f
}

/// The PREV_OTTDATA image.
pub fn prev_bytes(p: &PrevOttData) -> Vec<u8> {
    let mut o: Vec<u8> = i8s(&p.cld_old[0]).collect();
    o.extend(i8s(&p.icc_old[0]));
    o.extend_from_slice(&p.quant_coarse_cld_prev[0]);
    o.extend_from_slice(&p.quant_coarse_icc_prev[0]);
    o
}

pub fn prev_from(b: &[u8]) -> PrevOttData {
    let mut p = PrevOttData::default();
    for k in 0..MAX_NUM_BINS {
        p.cld_old[0][k] = b[k] as i8;
        p.icc_old[0][k] = b[23 + k] as i8;
    }
    p.quant_coarse_cld_prev[0].copy_from_slice(&b[46..48]);
    p.quant_coarse_icc_prev[0].copy_from_slice(&b[48..50]);
    p
}

/// The SPATIALSPECIFICCONFIG as INTs.
pub fn ssc_ints(s: &SpatialSpecificConfig) -> [i32; 12] {
    [
        s.bs_sampling_frequency,
        s.bs_frame_length,
        s.num_bands,
        s.bs_tree_config,
        s.bs_quant_mode,
        s.bs_fixed_gain_dmx,
        s.bs_env_quant_mode,
        s.bs_decorr_config,
        s.tree_description.num_ott_boxes,
        s.tree_description.num_in_chan,
        s.tree_description.num_out_chan,
        s.ott_config[0].bs_ott_bands,
    ]
}

pub fn ssc_from(v: &[i32; 12]) -> SpatialSpecificConfig {
    let mut s = SpatialSpecificConfig {
        bs_sampling_frequency: v[0],
        bs_frame_length: v[1],
        num_bands: v[2],
        bs_tree_config: v[3],
        bs_quant_mode: v[4],
        bs_fixed_gain_dmx: v[5],
        bs_env_quant_mode: v[6],
        bs_decorr_config: v[7],
        ..Default::default()
    };
    s.tree_description = TreeDescription { num_ott_boxes: v[8], num_in_chan: v[9], num_out_chan: v[10] };
    s.ott_config[0].bs_ott_bands = v[11];
    s
}

fn rand_bytes(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng.next_u32() as u8).collect()
}

#[test]
fn layout() {
    let mut v = [0i32; 13];
    let n = unsafe { c::cref_sacenc_bitstream_layout(v.as_mut_ptr()) } as usize;
    assert_eq!(&v[..n], &[199, 4, 5, 97, 149, 182, 190, 198, 50, 48, 48, 247, 300]);
    let mut rng = Rng::new(1);
    for _ in 0..100 {
        let b = rand_bytes(&mut rng, FRAME_SIZE);
        assert_eq!(frame_bytes(&frame_from(&b)), b);
        let p = rand_bytes(&mut rng, PREV_SIZE);
        assert_eq!(prev_bytes(&prev_from(&p)), p);
    }
}

#[test]
fn helpers() {
    let mut rng = Rng::new(2);
    for it in 0..5000 {
        // duplicateParameterSet (library) and DuplicateLosslessData
        let from = rand_bytes(&mut rng, FRAME_SIZE);
        let to = rand_bytes(&mut rng, FRAME_SIZE);
        let (sf, st) = (rng.range(0, 1), rng.range(0, 1));
        let mut tc = to.clone();
        let rc = unsafe { c::cref_sacenc_bitstream_duplicateParameterSet(from.as_ptr(), sf, tc.as_mut_ptr(), st) };
        let mut tr = frame_from(&to);
        let rr = r::fdk_sacenc_duplicate_parameter_set(&frame_from(&from), sf, &mut tr, st);
        assert_eq!((rc, tc), (rr, frame_bytes(&tr)), "duplicateParameterSet it {it}");

        let lf = rand_bytes(&mut rng, 8);
        let lt = rand_bytes(&mut rng, 8);
        let (b0, b1) = (rng.range(0, 1), rng.range(0, 1));
        let mut ltc = lt.clone();
        let rc = unsafe { c::cref_sacenc_bitstream_DuplicateLosslessData(b0, b1, lf.as_ptr(), sf, ltc.as_mut_ptr(), st) };
        let mut ltr = lossless_from(&lt);
        let rr = r::duplicate_lossless_data(b0, b1, &lossless_from(&lf), sf, &mut ltr, st);
        let mut lb = Vec::new();
        lossless_bytes(&ltr, &mut lb);
        assert_eq!((rc, ltc), (rr, lb));

        // clearFrame
        let mut fc = from.clone();
        unsafe { c::cref_sacenc_bitstream_clearFrame(fc.as_mut_ptr()) };
        let mut fr = frame_from(&from);
        r::clear_frame(&mut fr);
        assert_eq!(fc, frame_bytes(&fr));

        // fine2coarse, coarse2fine
        let d: Vec<i8> = (0..MAX_NUM_BINS).map(|_| rng.range(-32, 31) as i8).collect();
        let dt = rng.range(0, 1);
        let sb = rng.range(0, 22);
        let nb = rng.range(0, 23 - sb);
        let (mut dc, mut dr) = (d.clone(), d.clone());
        unsafe { c::cref_sacenc_bitstream_fine2coarse(dc.as_mut_ptr(), dt, sb, nb) };
        r::fine2coarse(&mut dr, dt, sb, nb);
        assert_eq!(dc, dr);
        unsafe { c::cref_sacenc_bitstream_coarse2fine(dc.as_mut_ptr(), dt, sb, nb) };
        r::coarse2fine(&mut dr, dt, sb, nb);
        assert_eq!(dc, dr);

        let idx = rng.range(-5, 8);
        assert_eq!(unsafe { c::cref_sacenc_bitstream_getBsFreqResStride(idx) }, r::get_bs_freq_res_stride(idx));
        let nb = [0, 4, 5, 7, 9, 12, 15, 23, 40, 28, rng.range(-50, 50)][rng.range(0, 10) as usize];
        let (mut ic, mut ir) = (0, 0);
        let ec = unsafe { c::cref_sacenc_bitstream_getBsFreqResIndex(nb, &mut ic) };
        let er = r::get_bs_freq_res_index(nb, &mut ir);
        assert_eq!((ec, ic), (er, ir));
        let fs = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350, 0, 47999, rng.range(0, 100000)]
            [rng.range(0, 15) as usize];
        let ec = unsafe { c::cref_sacenc_bitstream_getSamplingFrequencyIndex(fs, &mut ic) };
        let er = r::get_sampling_frequency_index(fs, &mut ir);
        assert_eq!((ec, ic), (er, ir));
    }
    for t in -2..4 {
        let mut f = SpatialFrame::default();
        let mut inst = BsfInstance::default();
        let p = r::fdk_sacenc_get_spatial_frame(Some(&mut inst), t).is_some() as i32;
        let h = unsafe { c::cref_sacenc_bitstream_create() };
        assert_eq!(unsafe { c::cref_sacenc_bitstream_getSpatialFrame(h, t) }, p);
        unsafe { c::cref_sacenc_bitstream_destroy(h) };
        let _ = &mut f;
    }
}

/// Random data of a box in the range of its quantizer (`coarse`: per parameter set).
fn data_for(rng: &mut Rng, cld: bool, coarse: &[u8; 2]) -> [[i8; MAX_NUM_BINS]; 2] {
    let mut d = [[0i8; MAX_NUM_BINS]; 2];
    let smooth = rng.range(0, 2);
    for ps in 0..2 {
        let (lo, hi) = match (cld, coarse[ps] != 0) {
            (true, false) => (-15, 15),
            (true, true) => (-7, 7),
            (false, false) => (0, 7),
            (false, true) => (0, 3),
        };
        let mut x = rng.range(lo, hi);
        for v in d[ps].iter_mut() {
            x = match smooth {
                0 => rng.range(lo, hi),
                1 => (x + rng.range(-1, 1)).clamp(lo, hi),
                _ => x,
            };
            *v = x as i8;
        }
        if rng.range(0, 3) == 0 && ps == 1 {
            d[1] = d[0];
            if coarse[0] != coarse[1] {
                for v in d[1].iter_mut() {
                    *v = (*v as i32).clamp(lo, hi) as i8;
                }
            }
        }
    }
    d
}

/// Old data that matches the previous quantization (`qprev`).
fn old_for(rng: &mut Rng, cld: bool, qprev: u8) -> [i8; MAX_NUM_BINS] {
    let d = data_for(rng, cld, &[qprev, qprev]);
    d[0]
}

#[test]
fn ec_data() {
    let mut rng = Rng::new(3);
    for it in 0..20000 {
        let cld = rng.range(0, 1) == 0;
        let dt = if cld { 0 } else { 1 };
        let nps = rng.range(1, 2);
        let mut l = LosslessData::default();
        for ps in 0..2 {
            l.bs_xxx_data_mode[0][ps] = rng.range(0, 3) as u8;
            // A pair flag survives only where ecData sets it (the frames are cleared).
            l.bs_data_pair[0][ps] = 0;
            l.bs_quant_coarse_xxx[0][ps] = rng.range(0, 1) as u8;
            l.bs_freq_res_stride_xxx[0][ps] = rng.range(0, 3) as u8;
        }
        if rng.range(0, 1) == 0 {
            l.bs_quant_coarse_xxx[0][1] = l.bs_quant_coarse_xxx[0][0];
            l.bs_freq_res_stride_xxx[0][1] = l.bs_freq_res_stride_xxx[0][0];
        }
        let data = data_for(&mut rng, cld, &l.bs_quant_coarse_xxx[0]);
        let mut qprev = [rng.range(0, 1) as u8, rng.range(0, 1) as u8];
        let mut old = old_for(&mut rng, cld, qprev[0]);
        if rng.range(0, 3) == 0 {
            old = data[0];
            qprev[0] = l.bs_quant_coarse_xxx[0][0];
        }
        let indep = rng.range(0, 1);
        let sb = 0;
        let eb = [4, 5, 7, 9, 12, 15, 23, rng.range(1, 23)][rng.range(0, 7) as usize];
        let def = if cld { 15 } else { 0 };

        let mut b = Bs::new(&mut rng, 512);
        let dflat: Vec<i8> = data.iter().flatten().copied().collect();
        let (mut oc, mut qc) = (old, qprev);
        let mut lc = Vec::new();
        lossless_bytes(&l, &mut lc);
        unsafe {
            c::cref_sacenc_bitstream_ecData(
                b.cbuf.as_mut_ptr(),
                b.size,
                b.pre,
                b.preval,
                dflat.as_ptr(),
                oc.as_mut_ptr(),
                qc.as_mut_ptr(),
                lc.as_mut_ptr(),
                dt,
                0,
                nps,
                indep,
                sb,
                eb,
                def,
                b.state.as_mut_ptr(),
            )
        };
        let mut bs = b.rust();
        let (mut or, mut qr, mut lr) = (old, qprev, l);
        r::ec_data(&mut bs, &data, &mut or, &mut qr, &mut lr, dt, 0, nps, indep, sb, eb, def);
        let mut lb = Vec::new();
        lossless_bytes(&lr, &mut lb);
        assert_eq!((oc, qc, lc), (or, qr, lb), "ecData it {it}");
        b.check(&bs, &format!("ecData it {it}"));
    }
}

#[test]
fn framing_info_and_smg_data() {
    let mut rng = Rng::new(4);
    for it in 0..10000 {
        let fl = [rng.range(0, 31), rng.range(-3, 1000), 0, 15][rng.range(0, 3) as usize];
        let fi = [rng.range(0, 1) as u8, rng.range(1, 2) as u8, rng.next_u32() as u8, rng.next_u32() as u8];
        let mut b = Bs::new(&mut rng, 256);
        let ec = unsafe {
            c::cref_sacenc_bitstream_writeFramingInfo(b.cbuf.as_mut_ptr(), b.size, b.pre, b.preval, fi.as_ptr(), fl, b.state.as_mut_ptr())
        };
        let mut bs = b.rust();
        let f = FramingInfo { bs_framing_type: fi[0], num_param_sets: fi[1], bs_param_slots: [fi[2], fi[3]] };
        let er = r::write_framing_info(&mut bs, &f, fl);
        assert_eq!(ec, er);
        b.check(&bs, &format!("writeFramingInfo it {it}"));

        let mut smg = rand_bytes(&mut rng, 52);
        for k in 0..2 {
            smg[k] = rng.range(0, 3) as u8;
            smg[2 + k] = rng.range(0, 3) as u8;
            smg[4 + k] = rng.range(0, 5) as u8;
        }
        let nps = rng.range(0, 2);
        let bands = rng.range(0, 23);
        let mut b = Bs::new(&mut rng, 256);
        let ec = unsafe {
            c::cref_sacenc_bitstream_writeSmgData(b.cbuf.as_mut_ptr(), b.size, b.pre, b.preval, smg.as_ptr(), nps, bands, b.state.as_mut_ptr())
        };
        let mut bs = b.rust();
        let er = r::write_smg_data(&mut bs, &smg_from(&smg), nps, bands);
        assert_eq!(ec, er);
        b.check(&bs, &format!("writeSmgData it {it}"));
    }
}

/// A random SpatialSpecificConfig as the encoder fills it (sometimes with invalid values).
pub fn rand_ssc(rng: &mut Rng) -> SpatialSpecificConfig {
    let nb = [4, 5, 7, 9, 12, 15, 23, 15, 3, 0][rng.range(0, 9) as usize];
    let fs = [16000, 22050, 24000, 32000, 44100, 48000, rng.range(0, 48000), 96000, 7350][rng.range(0, 8) as usize];
    SpatialSpecificConfig {
        bs_sampling_frequency: fs,
        bs_frame_length: rng.range(0, 15),
        num_bands: nb,
        bs_tree_config: if rng.range(0, 9) == 0 { rng.range(0, 15) } else { TREE_212 },
        bs_quant_mode: rng.range(0, 2),
        bs_fixed_gain_dmx: rng.range(0, 7),
        bs_env_quant_mode: 0,
        bs_decorr_config: rng.range(0, 2),
        tree_description: TreeDescription { num_ott_boxes: 1, num_in_chan: 1, num_out_chan: 2 },
        ott_config: [OttConfig { bs_ott_bands: nb }],
    }
}

#[test]
fn spatial_specific_config() {
    let mut rng = Rng::new(5);
    for it in 0..5000 {
        let s = rand_ssc(&mut rng);
        let size = [64, 64, 4, 8][rng.range(0, 3) as usize];
        let init = rand_bytes(&mut rng, 64);
        let (mut bc, mut br) = (init.clone(), init.clone());
        let (mut nc, mut nr) = (-7, -7);
        let ec = unsafe {
            c::cref_sacenc_bitstream_writeSpatialSpecificConfig(ssc_ints(&s).as_ptr(), bc.as_mut_ptr(), size, &mut nc)
        };
        let er = r::fdk_sacenc_write_spatial_specific_config(&s, &mut br, size, &mut nr);
        assert_eq!((ec, nc, &bc), (er, nr, &br), "SSC it {it}");
        // A slice shorter than the bit buffer: the part of the slice is the same.
        let mut bs = init[..20].to_vec();
        let mut ns = -7;
        let es = r::fdk_sacenc_write_spatial_specific_config(&s, &mut bs, size, &mut ns);
        assert_eq!((es, ns, &bs[..]), (ec, nc, &bc[..20]), "SSC short slice it {it}");
    }
}

/// A random spatial frame as the encoder fills it.
pub fn rand_frame(rng: &mut Rng, ssc: &SpatialSpecificConfig) -> SpatialFrame {
    let mut f = SpatialFrame::default();
    let nps = rng.range(1, 2) as u8;
    f.framing_info.num_param_sets = nps;
    f.framing_info.bs_framing_type = if nps == 2 { 1 } else { rng.range(0, 1) as u8 };
    let mut s0 = rng.range(0, ssc.bs_frame_length.max(0)) as u8;
    let mut s1 = rng.range(0, ssc.bs_frame_length.max(0)) as u8;
    if s0 > s1 {
        core::mem::swap(&mut s0, &mut s1);
    }
    f.framing_info.bs_param_slots = [s0, s1];
    f.bs_independency_flag = (rng.range(0, 4) == 0) as u8;
    for l in [&mut f.cld_lossless_data, &mut f.icc_lossless_data] {
        for ps in 0..2 {
            l.bs_quant_coarse_xxx[0][ps] = (rng.range(0, 3) == 0) as u8;
            l.bs_freq_res_stride_xxx[0][ps] = if rng.range(0, 3) == 0 { rng.range(0, 3) as u8 } else { 0 };
        }
    }
    let qc = f.cld_lossless_data.bs_quant_coarse_xxx[0];
    let qi = f.icc_lossless_data.bs_quant_coarse_xxx[0];
    f.ott_data.cld[0] = data_for(rng, true, &qc);
    f.ott_data.icc[0] = data_for(rng, false, &qi);
    f.b_use_bb_cues = (rng.range(0, 5) == 0) as u8;
    f
}

#[test]
fn spatial_frames() {
    let mut rng = Rng::new(6);
    for seq in 0..300 {
        let h = unsafe { c::cref_sacenc_bitstream_create() };
        let mut inst: HandleBsfInstance = None;
        assert_eq!(r::fdk_sacenc_create_spatial_bitstream_encoder(Some(&mut inst)), 0);
        let inst = inst.as_mut().unwrap();
        assert_eq!(unsafe { c::cref_sacenc_bitstream_init(h) }, r::fdk_sacenc_init_spatial_bitstream_encoder(Some(inst)));

        let mut ssc = rand_ssc(&mut rng);
        if ssc.num_bands < 4 {
            ssc.num_bands = 15;
            ssc.ott_config[0].bs_ott_bands = 15;
        }
        let size = [16384, 1023, 4096][rng.range(0, 2) as usize];
        for fr in 0..40 {
            let f = rand_frame(&mut rng, &ssc);
            let (mut sc, mut fc, mut pc) = ([0i32; 12], vec![0u8; FRAME_SIZE], vec![0u8; PREV_SIZE]);
            unsafe { c::cref_sacenc_bitstream_get(h, sc.as_mut_ptr(), fc.as_mut_ptr(), pc.as_mut_ptr()) };
            unsafe { c::cref_sacenc_bitstream_set(h, ssc_ints(&ssc).as_ptr(), frame_bytes(&f).as_ptr(), pc.as_ptr()) };
            inst.spatial_specific_config = ssc;
            inst.frame = f;

            let init = rand_bytes(&mut rng, 64);
            let mut bc = vec![0u8; size];
            bc[..64].copy_from_slice(&init);
            let mut br = bc.clone();
            let (mut nc, mut nr) = (-1, -1);
            let ec = unsafe { c::cref_sacenc_bitstream_writeSpatialFrame(h, bc.as_mut_ptr(), 16384, &mut nc) };
            let er = r::fdk_sacenc_write_spatial_frame(&mut br, 16384, &mut nr, inst);
            assert_eq!((ec, nc), (er, nr), "seq {seq} frame {fr}");
            assert!(bc == br, "seq {seq} frame {fr}: output");
            unsafe { c::cref_sacenc_bitstream_get(h, sc.as_mut_ptr(), fc.as_mut_ptr(), pc.as_mut_ptr()) };
            assert_eq!(fc, frame_bytes(&inst.frame), "seq {seq} frame {fr}: frame");
            assert_eq!(pc, prev_bytes(&inst.prev_frame_data.prev_ott_data), "seq {seq} frame {fr}: prev");
            assert_eq!(sc, ssc_ints(&inst.spatial_specific_config));
        }
        assert_eq!(unsafe { c::cref_sacenc_bitstream_destroy(h) }, 0);
    }
    assert_eq!(unsafe { c::cref_sacenc_bitstream_create_null() }, r::fdk_sacenc_create_spatial_bitstream_encoder(None));
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
    let f = c::cref_sacenc_bitstream_table;
    let i = |t: &[u8]| t.iter().map(|&x| x as i32).collect::<Vec<_>>();
    assert_eq!(c_table(f, 0), SAMPLE_RATE_TABLE.to_vec(), "SampleRateTable");
    assert_eq!(c_table(f, 1), i(&FREQ_RES_BIN_TABLE_LD), "FreqResBinTable_LD");
    assert_eq!(c_table(f, 2), i(&FREQ_RES_STRIDE_TABLE_LD), "FreqResStrideTable_LD");
}

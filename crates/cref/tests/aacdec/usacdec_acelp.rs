// Differential tests of the functions of fdk_aac_rs_aacdec::usacdec_acelp against the
// C++ reference. The tests of the types are in usacdec_acelp_types.rs.

use crate::usacdec_lpd_util::{rand_dbl_vec, rand_lsf, Bs};
use fdk_aac_rs_aacdec::usacdec_acelp::*;
use fdk_aac_rs_aacdec::usacdec_lpc::{clpc_conceal, e_lpc_f_lsp_a_conversion};
use fdk_aac_rs_cref::aacdec_usacdec_acelp as c;
use fdk_aac_rs_cref::testutil::Rng;

/// `CAcelpStaticMem` as the INT array of the shim.
pub fn mem_to_vec(m: &CAcelpStaticMem) -> Vec<i32> {
    let mut v = Vec::with_capacity(470);
    v.extend_from_slice(&m.old_exc_mem);
    v.extend_from_slice(&m.old_syn_mem);
    v.extend(m.a.iter().map(|&x| x as i32));
    v.extend_from_slice(&[
        m.a_exp,
        m.gc_threshold,
        m.de_emph_mem,
        m.past_gpit as i32,
        m.past_gcode,
        m.old_t0 as i32,
        m.old_t0_frac as i32,
        m.deemph_mem_wsyn,
        m.wsyn_rms,
        m.seed_ace as i32,
    ]);
    v
}

/// The INT array of the shim as `CAcelpStaticMem`.
pub fn mem_from_vec(v: &[i32]) -> CAcelpStaticMem {
    let mut m = CAcelpStaticMem::default();
    m.old_exc_mem.copy_from_slice(&v[..428]);
    m.old_syn_mem.copy_from_slice(&v[428..444]);
    for i in 0..16 {
        m.a[i] = v[444 + i] as i16;
    }
    let t = &v[460..470];
    m.a_exp = t[0];
    m.gc_threshold = t[1];
    m.de_emph_mem = t[2];
    m.past_gpit = t[3] as i16;
    m.past_gcode = t[4];
    m.old_t0 = t[5] as u16;
    m.old_t0_frac = t[6] as u8;
    m.deemph_mem_wsyn = t[7];
    m.wsyn_rms = t[8];
    m.seed_ace = t[9] as i16;
    m
}

/// `CAcelpChannelData` as the INT array of the shim.
pub fn data_to_vec(d: &CAcelpChannelData) -> Vec<i32> {
    let mut v = vec![d.acelp_core_mode as i32, d.mean_energy as i32];
    v.extend(d.t0.iter().map(|&x| x as i32));
    v.extend(d.t0_frac.iter().map(|&x| x as i32));
    v.extend(d.ltp_filtering_flag.iter().map(|&x| x as i32));
    for row in &d.icb_index {
        v.extend(row.iter().map(|&x| x as i32));
    }
    v.extend(d.gains.iter().map(|&x| x as i32));
    v
}

/// The INT array of the shim as `CAcelpChannelData`.
pub fn data_from_vec(v: &[i32]) -> CAcelpChannelData {
    let mut d = CAcelpChannelData::default();
    d.acelp_core_mode = v[0] as u8;
    d.mean_energy = v[1] as u8;
    for i in 0..4 {
        d.t0[i] = v[2 + i] as u16;
        d.t0_frac[i] = v[6 + i] as u8;
        d.ltp_filtering_flag[i] = v[10 + i] as u8;
        for k in 0..8 {
            d.icb_index[i][k] = v[14 + 8 * i + k] as i16;
        }
        d.gains[i] = v[46 + i] as u8;
    }
    d
}

/// A random ACELP memory with excitation and synthesis values of plausible magnitude.
pub fn rand_mem(rng: &mut Rng) -> CAcelpStaticMem {
    let mut m = CAcelpStaticMem::default();
    let sh = rng.range(4, 20) as u32;
    for x in m.old_exc_mem.iter_mut() {
        *x = (rng.next_u32() as i32) >> sh;
    }
    for x in m.old_syn_mem.iter_mut() {
        *x = (rng.next_u32() as i32) >> sh;
    }
    for x in m.a.iter_mut() {
        *x = rng.i16();
    }
    m.a_exp = rng.range(-4, 9);
    m.gc_threshold = (rng.next_u32() >> rng.range(1, 31)) as i32;
    m.de_emph_mem = (rng.next_u32() as i32) >> sh;
    m.past_gpit = (rng.next_u32() % 16384) as i16;
    m.past_gcode = (rng.next_u32() >> rng.range(1, 31)) as i32;
    m.old_t0 = rng.range(16, 420) as u16;
    m.old_t0_frac = (rng.next_u32() % 4) as u8;
    m.deemph_mem_wsyn = (rng.next_u32() as i32) >> sh;
    m.wsyn_rms = (rng.next_u32() >> rng.range(1, 31)) as i32;
    m.seed_ace = rng.i16();
    m
}

/// A LSP vector of a random sorted LSF set (as `CLpc_Read()` produces it).
pub fn rand_lsp(rng: &mut Rng) -> [i16; 16] {
    let mut lsp = [[0i16; 16]; 5];
    let mut lpc4 = rand_lsf(rng);
    let mean = rand_lsf(rng);
    clpc_conceal(&mut lsp, &mut lpc4, &mean, 0);
    lsp[(rng.next_u32() % 5) as usize]
}

#[test]
fn random_and_small_filters() {
    let mut rng = Rng::new(0x12AE_0001);
    for seed in i16::MIN..=i16::MAX {
        let mut s = seed;
        let a = e_util_random(&mut s);
        let b = e_util_random(&mut s);
        let r = unsafe { c::cref_acelp_random(seed) };
        assert_eq!(((a as i32) << 16) | (b as u16 as i32), r);
    }
    for it in 0..20_000 {
        let buf = rand_dbl_vec(&mut rng, 400);
        let off = rng.range(1, 300) as usize;
        let l = rng.range(0, 99);
        let mut or = vec![0i32; 100];
        let mut oc = vec![0i32; 100];
        e_util_preemph(&buf, off, &mut or, l);
        unsafe { c::cref_acelp_preemph(buf.as_ptr(), off as i32, oc.as_mut_ptr(), l) };
        assert_eq!(or, oc, "preemph");

        let code: [i16; 64] = core::array::from_fn(|_| if it % 2 == 0 { rng.i16() } else { rng.range(-2048, 2048) as i16 });
        let mut cr = code;
        let mut cc = code;
        preemph_code(&mut cr);
        unsafe { c::cref_acelp_preemph_code(cc.as_mut_ptr()) };
        assert_eq!(cr, cc, "Preemph_code");
        let lag = rng.range(0, 80);
        pit_shrp(&mut cr, lag);
        unsafe { c::cref_acelp_pit_shrp(cc.as_mut_ptr(), lag) };
        assert_eq!(cr, cc, "Pit_shrp {lag}");

        let x = rand_dbl_vec(&mut rng, 64);
        let mut yr = vec![0i32; 64];
        let mut yc = vec![0i32; 64];
        let mem0 = rng.i32();
        let (mut mr, mut mc) = (mem0, mem0);
        let l = rng.range(0, 64);
        deemph(&x, &mut yr, l, &mut mr);
        let mut xc = x.clone();
        unsafe { c::cref_acelp_deemph(xc.as_mut_ptr(), yc.as_mut_ptr(), l, &mut mc) };
        assert_eq!((yr, mr), (yc, mc), "Deemph");
    }
}

#[test]
fn syn_filt_residu_int_lpc_random() {
    let mut rng = Rng::new(0x12AE_0002);
    for it in 0..20_000 {
        let a: [i16; 16] = core::array::from_fn(|_| rng.i16());
        let a_exp = if it % 4 == 0 { rng.range(-22, 9) } else { rng.range(-3, 9) };
        let x = rand_dbl_vec(&mut rng, 128);
        let y = rand_dbl_vec(&mut rng, 160);
        let len = [64, 128, rng.range(0, 128)][(rng.next_u32() % 3) as usize];
        let mut yr = y.clone();
        let mut yc = y.clone();
        syn_filt(&a, a_exp, len, &x, &mut yr, 16);
        let mut xc = x.clone();
        unsafe { c::cref_acelp_syn_filt(a.as_ptr(), a_exp, len, xc.as_mut_ptr(), yc.as_mut_ptr(), 16) };
        assert_eq!(yr, yc, "Syn_filt");

        let mut rr = vec![0i32; 128];
        let mut rc = vec![0i32; 128];
        e_util_residu(&a, a_exp, &y, 16, &mut rr, len);
        let mut yc2 = y.clone();
        unsafe { c::cref_acelp_residu(a.as_ptr(), a_exp, yc2.as_mut_ptr(), 16, rc.as_mut_ptr(), len) };
        assert_eq!(rr, rc, "E_UTIL_residu");

        let lsp_old = rand_lsp(&mut rng);
        let lsp_new = if it % 3 == 0 { core::array::from_fn(|_| rng.i16()) } else { rand_lsp(&mut rng) };
        let nb = 3 + (rng.next_u32() % 2) as i32;
        let sf = rng.range(0, nb - 1);
        let mut ar = [0i16; 16];
        let mut er = 0;
        int_lpc_acelp(&lsp_old, &lsp_new, sf, nb, &mut ar, &mut er);
        let mut ac = [0i16; 16];
        let mut ec = 0;
        unsafe { c::cref_acelp_int_lpc_acelp(lsp_old.as_ptr(), lsp_new.as_ptr(), sf, nb, ac.as_mut_ptr(), &mut ec) };
        assert_eq!((ar, er), (ac, ec), "int_lpc_acelp");
    }
}

#[test]
fn gains_period_factor_noise_enhancer_excitation_random() {
    let mut rng = Rng::new(0x12AE_0003);
    for it in 0..50_000 {
        let code: [i16; 64] = match it % 4 {
            0 => core::array::from_fn(|_| rng.i16()),
            1 => [0; 64],
            _ => core::array::from_fn(|_| rng.range(-1024, 1024) as i16),
        };
        let index = (rng.next_u32() % 128) as i32;
        let mean = (rng.next_u32() % 4) as i32;
        let bfi = (rng.next_u32() % 2) as i32;
        let io0 = [rng.i16() as i32, rng.i32(), (rng.next_u32() % 32768) as i32, (rng.next_u32() >> rng.range(0, 31)) as i32, 0, 0];
        let mut io_r = io0;
        let (mut gp, mut gc, mut pgp, mut pgc, mut en, mut ene) =
            (io_r[0] as i16, io_r[1], io_r[2] as i16, io_r[3], io_r[4], io_r[5]);
        d_gain2_plus(index, &code, &mut gp, &mut gc, mean, bfi, &mut pgp, &mut pgc, &mut en, &mut ene);
        io_r = [gp as i32, gc, pgp as i32, pgc, en, ene];
        let mut io_c = io0;
        let mut cc = code;
        unsafe { c::cref_acelp_d_gain2_plus(index, cc.as_mut_ptr(), mean, bfi, io_c.as_mut_ptr()) };
        assert_eq!(io_r, io_c, "D_gain2_plus index {index} bfi {bfi}");

        let exc = rand_dbl_vec(&mut rng, 64);
        let pf_r = calc_period_factor(&exc, gp, gc, en, ene);
        let mut excc = exc.clone();
        let pf_c = unsafe { c::cref_acelp_calc_period_factor(excc.as_mut_ptr(), gp, gc, en, ene) };
        assert_eq!(pf_r, pf_c, "calc_period_factor");

        let stab = (rng.next_u32() % 16385) as i16;
        let thr = (rng.next_u32() >> rng.range(0, 31)) as i32;
        let (mut tr, mut tc) = (thr, thr);
        let gr = noise_enhancer(gc, pf_r, stab, &mut tr);
        let gcc = unsafe { c::cref_acelp_noise_enhancer(gc, pf_r, stab, &mut tc) };
        assert_eq!((gr, tr), (gcc, tc), "noise_enhancer");

        let exc_buf = rand_dbl_vec(&mut rng, 685);
        let off = 428 + 64 * (rng.next_u32() % 4) as usize;
        let mut er = exc_buf.clone();
        let mut ec = exc_buf;
        let mut x2r = vec![0i32; 64];
        let mut x2c = vec![0i32; 64];
        build_adaptive_excitation(&code, &mut er, off, gp, gc, gr, pf_r, &mut x2r);
        let mut cc = code;
        unsafe { c::cref_acelp_build_adaptive_excitation(cc.as_mut_ptr(), ec.as_mut_ptr(), off as i32, gp, gc, gr, pf_r, x2c.as_mut_ptr()) };
        assert_eq!((er, x2r), (ec, x2c), "BuildAdaptiveExcitation");
    }
}

#[test]
fn read_and_pitch_lag_random_bitstreams() {
    let mut rng = Rng::new(0x12AE_0004);
    for it in 0..60_000 {
        let mut b = Bs::random(&mut rng, 64);
        // i_offset of the sampling rates 6000..=24000 (and beyond for the error path)
        let sr = if it % 10 == 0 { rng.range(0, 96000) } else { rng.range(6000, 24000) };
        let i_offset = (sr * 34 + 6400) / 12800 - 34;
        let ccfl = if it % 2 == 0 { 1024 } else { 768 };
        let mode = (rng.next_u32() % 8) as i32;
        let d0 = CAcelpChannelData::default();
        let mut dr = d0;
        let er = clpd_acelp_read(&mut b.rs, &mut dr, mode, ccfl, i_offset);
        let mut dc = data_to_vec(&d0);
        let (st, buf) = b.c();
        let ec = unsafe { c::cref_acelp_read(st, buf, dc.as_mut_ptr(), mode, ccfl, i_offset) };
        assert_eq!(er, ec);
        assert_eq!(data_to_vec(&dr), dc, "acelp data mode {mode} ccfl {ccfl} i_offset {i_offset}");
        b.check("CLpd_AcelpRead");

        let mut b = Bs::random(&mut rng, 16);
        let bits = if it % 2 == 0 { 9 } else { 6 };
        let pit_min = 34 + i_offset;
        let io0 = [rng.range(0, 500), rng.range(0, 3), rng.range(0, 500), rng.range(0, 500)];
        let mut ior = io0;
        let rr = {
            let [a, bb, cc, dd] = &mut ior;
            decode_pitch_lag(&mut b.rs, bits as u8, pit_min, 128 - i_offset, 160, 231 + 6 * i_offset, a, bb, cc, dd)
        };
        let mut ioc = io0;
        let (st, buf) = b.c();
        let rc = unsafe { c::cref_acelp_decode_pitch_lag(st, buf, bits, pit_min, 128 - i_offset, 160, 231 + 6 * i_offset, ioc.as_mut_ptr()) };
        assert_eq!((rr, ior), (rc, ioc), "DecodePitchLag");
        b.check("DecodePitchLag");

        let mut m = rand_mem(&mut rng);
        let pit_max = 231 + 6 * i_offset;
        let (mut t0r, mut t0fr) = (0, 0);
        let mut mc = mem_to_vec(&m);
        conceal_pitch_lag(&mut m, pit_max, &mut t0r, &mut t0fr);
        let (mut t0c, mut t0fc) = (0, 0);
        unsafe { c::cref_acelp_conceal_pitch_lag(mc.as_mut_ptr(), pit_max, &mut t0c, &mut t0fc) };
        assert_eq!((t0r, t0fr, mem_to_vec(&m)), (t0c, t0fc, mc), "ConcealPitchLag");
    }
}

/// Random ACELP data as the bitstream reader produces it.
fn rand_acelp_data(rng: &mut Rng, ccfl: i32, i_offset: i32) -> CAcelpChannelData {
    let mut b = Bs::random(rng, 64);
    let mut d = CAcelpChannelData::default();
    let mode = (rng.next_u32() % 8) as i32;
    clpd_acelp_read(&mut b.rs, &mut d, mode, ccfl, i_offset);
    d
}

#[test]
fn acelp_decode_frames() {
    let mut rng = Rng::new(0x12AE_0005);
    for _ in 0..300 {
        let ccfl = if rng.next_u32() % 2 == 0 { 1024 } else { 768 };
        let sr = rng.range(6000, 24000);
        let i_offset = (sr * 34 + 6400) / 12800 - 34;
        let mut mem = rand_mem(&mut rng);
        if rng.next_u32() % 2 == 0 {
            clpd_acelp_reset(&mut mem);
            let mut mc = mem_to_vec(&mem);
            let mut m2 = mem_to_vec(&rand_mem(&mut rng));
            m2.copy_from_slice(&mc);
            unsafe { c::cref_acelp_reset(mc.as_mut_ptr()) };
            assert_eq!(mem_to_vec(&mem), mc);
        }
        let mut memc = mem_to_vec(&mem);
        let mut lsp_old = rand_lsp(&mut rng);
        // a sequence of frames with the state carried over
        for frame in 0..20 {
            let lsp_new = rand_lsp(&mut rng);
            let d = rand_acelp_data(&mut rng, ccfl, i_offset);
            let stab = (rng.next_u32() % 16385) as i16;
            let lost = if rng.next_u32() % 4 == 0 { rng.range(1, 3) } else { 0 };
            let last_lpc_lost = (rng.next_u32() % 2) as i32;
            let cnt = rng.range(0, 3);
            let mut synth_r = vec![0i32; 256];
            let mut pt_r = vec![0i32; 4];
            let mut pg_r = vec![0i32; 4];
            clpd_acelp_decode(&mut mem, i_offset, &lsp_old, &lsp_new, stab, &d, lost, last_lpc_lost, cnt, &mut synth_r, &mut pt_r, &mut pg_r, ccfl);
            let mut synth_c = vec![0i32; 256];
            let mut pt_c = vec![0i32; 4];
            let mut pg_c = vec![0i32; 4];
            let mut dc = data_to_vec(&d);
            unsafe {
                c::cref_acelp_decode(
                    memc.as_mut_ptr(),
                    i_offset,
                    lsp_old.as_ptr(),
                    lsp_new.as_ptr(),
                    stab,
                    dc.as_mut_ptr(),
                    lost,
                    last_lpc_lost,
                    cnt,
                    synth_c.as_mut_ptr(),
                    pt_c.as_mut_ptr(),
                    pg_c.as_mut_ptr(),
                    ccfl,
                )
            };
            assert_eq!(synth_r, synth_c, "synth frame {frame}");
            assert_eq!((pt_r, pg_r), (pt_c, pg_c), "pitch frame {frame}");
            assert_eq!(mem_to_vec(&mem), memc, "memory frame {frame}");
            lsp_old = lsp_new;

            // TCX time domain concealment on the same state now and then
            if rng.next_u32() % 3 == 0 {
                let pitch = [rng.range(1, 411), 256, rng.range(-5, 600)][(rng.next_u32() % 3) as usize] as i16;
                let lsp_new = rand_lsp(&mut rng);
                let n_lost = rng.range(0, 4);
                let mut sr_ = vec![0i32; 256];
                let mut sc_ = vec![0i32; 256];
                if pitch > 0 && pitch <= 411 {
                    clpd_tcx_td_conceal(&mut mem, &pitch, &lsp_old, &lsp_new, stab, n_lost, &mut sr_, ccfl, 0);
                    unsafe { c::cref_acelp_tcx_td_conceal(memc.as_mut_ptr(), pitch, lsp_old.as_ptr(), lsp_new.as_ptr(), stab, n_lost, sc_.as_mut_ptr(), ccfl, 0) };
                    assert_eq!(sr_, sc_, "TcxTDConceal synth (pitch {pitch})");
                    assert_eq!(mem_to_vec(&mem), memc, "TcxTDConceal memory");
                    lsp_old = lsp_new;
                }
            }
        }
    }
}

#[test]
fn pre_post_processing_zir_prepare_random() {
    let mut rng = Rng::new(0x12AE_0006);
    for it in 0..4_000 {
        let ccfl = if it % 2 == 0 { 1024 } else { 768 };
        let nb_subfr = ccfl / (4 * 64);
        let nb_sf_super = 4 * nb_subfr;
        let syn_sfd = nb_sf_super / 2 - 1;
        let synth_buf = rand_dbl_vec(&mut rng, 1883);
        let old_synth = rand_dbl_vec(&mut rng, 347);
        let old_t_pf: Vec<i32> = (0..7).map(|_| rng.range(0, 500)).collect();
        let old_gain_pf = rand_dbl_vec(&mut rng, 7);
        let sr = if it % 5 == 0 { rng.i32() } else { rng.range(6000, 24000) };
        let (mut sbr, mut sbc) = (synth_buf.clone(), synth_buf.clone());
        let (mut pr, mut pc) = (vec![0i32; 23], vec![0i32; 23]);
        let (mut gr, mut gc) = (vec![0i32; 23], vec![0i32; 23]);
        let (mut ior, mut ioc) = (0, 0);
        acelp_pre_processing(&mut sbr, &old_synth, &mut pr, &old_t_pf, &mut gr, &old_gain_pf, sr, &mut ior, ccfl, syn_sfd, nb_sf_super);
        let mut osc = old_synth.clone();
        let mut otc = old_t_pf.clone();
        let mut ogc = old_gain_pf.clone();
        unsafe { c::cref_acelp_pre_processing(sbc.as_mut_ptr(), osc.as_mut_ptr(), pc.as_mut_ptr(), otc.as_mut_ptr(), gc.as_mut_ptr(), ogc.as_mut_ptr(), sr, &mut ioc, ccfl, syn_sfd, nb_sf_super) };
        assert_eq!((sbr.clone(), pr.clone(), gr, ior), (sbc, pc, gc, ioc), "Acelp_PreProcessing");

        let (mut osr, mut osc) = (old_synth.clone(), old_synth.clone());
        let (mut otr, mut otc) = (old_t_pf.clone(), old_t_pf.clone());
        acelp_post_processing(&synth_buf, &mut osr, &pr, &mut otr, ccfl, syn_sfd, nb_sf_super);
        let mut sbc2 = synth_buf.clone();
        let mut pc2 = pr.clone();
        unsafe { c::cref_acelp_post_processing(sbc2.as_mut_ptr(), osc.as_mut_ptr(), pc2.as_mut_ptr(), otc.as_mut_ptr(), ccfl, syn_sfd, nb_sf_super) };
        assert_eq!((osr, otr), (osc, otc), "Acelp_PostProcessing");

        // CLpd_Acelp_Zir
        let mut m = rand_mem(&mut rng);
        let mut mc = mem_to_vec(&m);
        let lsp = rand_lsp(&mut rng);
        let mut a = [0i16; 16];
        let mut a_exp = 0;
        e_lpc_f_lsp_a_conversion(&lsp, &mut a, &mut a_exp);
        let len = [32, 48, 64, 96, 128][(rng.next_u32() % 5) as usize];
        let deem = (rng.next_u32() % 2) as i32;
        let mut zr = vec![0i32; 128];
        let mut zc = vec![0i32; 128];
        clpd_acelp_zir(&a, a_exp, &mut m, len, &mut zr, deem);
        unsafe { c::cref_acelp_zir(a.as_ptr(), a_exp, mc.as_mut_ptr(), len, zc.as_mut_ptr(), deem) };
        assert_eq!((zr, mem_to_vec(&m)), (zc, mc.clone()), "CLpd_Acelp_Zir");

        // CLpd_AcelpPrepareInternalMem
        let lsp2 = rand_lsp(&mut rng);
        let mut a2 = [0i16; 16];
        let mut a2_exp = 0;
        e_lpc_f_lsp_a_conversion(&lsp2, &mut a2, &mut a2_exp);
        let sb = rand_dbl_vec(&mut rng, 1883);
        let off = 347 + ccfl as usize / 2 + 256 * (rng.next_u32() % 4) as usize * ccfl as usize / 1024;
        let last = [0u8, 1, 2, 3, 4, 255][(rng.next_u32() % 6) as usize];
        let last_last = [0u8, 1, 2, 3, 255][(rng.next_u32() % 5) as usize];
        let clear = (rng.next_u32() % 4 == 0) as i32;
        let lpd_mode = [0u8, 4][(rng.next_u32() % 2) as usize];
        clpd_acelp_prepare_internal_mem(&sb, off, last, last_last, &a, a_exp, &a2, a2_exp, &mut m, ccfl, clear, lpd_mode);
        unsafe { c::cref_acelp_prepare_internal_mem(sb.as_ptr(), off as i32, last, last_last, a.as_ptr(), a_exp, a2.as_ptr(), a2_exp, mc.as_mut_ptr(), ccfl, clear, lpd_mode) };
        assert_eq!(mem_to_vec(&m), mc, "CLpd_AcelpPrepareInternalMem last {last} {last_last} clear {clear} mode {lpd_mode}");
    }
}

/// The file-static table `name` of the C++ source (a copy compiled into the shim `f`).
fn c_table<T: Copy>(f: unsafe extern "C" fn(*const i8, *mut i32) -> *const core::ffi::c_void, name: &str) -> Vec<T> {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut bytes = 0;
    let p = unsafe { f(cname.as_ptr(), &mut bytes) };
    assert!(!p.is_null(), "{name}");
    assert_eq!(bytes as usize % core::mem::size_of::<T>(), 0, "{name}");
    unsafe { core::slice::from_raw_parts(p as *const T, bytes as usize / core::mem::size_of::<T>()) }.to_vec()
}

/// POW_10_MEAN_ENERGY (`pow_10_mean_energy`) and TAB_COREMODE2NBITS (`tab_coremode2nbits`),
/// file-static in usacdec_acelp.cpp, element by element.
#[test]
fn acelp_tables() {
    assert_eq!(c_table::<i32>(c::cref_acelp_table, "pow_10_mean_energy"), POW_10_MEAN_ENERGY.to_vec());
    assert_eq!(c_table::<u8>(c::cref_acelp_table, "tab_coremode2nbits"), TAB_COREMODE2NBITS.to_vec());
}

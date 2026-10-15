// Differential tests of fdk_aac_rs_aacdec::usacdec_fac against the C++ reference. The shims of
// the transforms are in aacdec_usacdec_lpd.cpp (they need the channel state).

use crate::usacdec_acelp::{rand_lsp, rand_mem};
use crate::usacdec_lpd_state::*;
use crate::usacdec_lpd_util::{assert_vec_eq, rand_dbl_vec, Bs};
use fdk_aac_rs_aacdec::channelinfo::{CAacDecoderChannelInfo, CAacDecoderStaticChannelInfo, FacDataPtr};
use fdk_aac_rs_aacdec::usacdec_fac::*;
use fdk_aac_rs_aacdec::usacdec_lpc::e_lpc_f_lsp_a_conversion;
use fdk_aac_rs_cref::aacdec_usacdec_lpd as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::mdct::mdct_init;
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;

#[test]
fn get_memory_all_modes() {
    for gl in [128, 96] {
        for m in 0..256u32 {
            let mod_ = [(m & 3) as u8, ((m >> 2) & 3) as u8, ((m >> 4) & 3) as u8, ((m >> 6) & 3) as u8];
            let mut ci = CAacDecoderChannelInfo::default();
            ci.granule_length = gl;
            let (mut sr, mut sc) = (0, 0);
            // as many calls as CLpdChannelStream_Read() can make (5)
            for _ in 0..5 {
                let pr = clpd_fac_get_memory(&ci, &mod_, &mut sr);
                let pc = unsafe { c::cref_fac_get_memory(gl, mod_.as_ptr(), &mut sc) };
                let pr = match pr {
                    FacDataPtr::FacData0 => -2,
                    FacDataPtr::Spectral(o) => o as i32,
                    _ => -9,
                };
                assert_eq!((pr, sr), (pc, sc), "mod {mod_:?}");
                if sr > 8 {
                    break;
                }
            }
        }
    }
}

#[test]
fn read_random_bitstreams() {
    let mut rng = Rng::new(0x12FA_0001);
    for it in 0..60_000 {
        let mut b = Bs::random(&mut rng, 128);
        let length = [128, 96, 64, 48][(rng.next_u32() % 4) as usize];
        let use_gain = (rng.next_u32() % 2) as i32;
        let frame = rng.range(0, 3);
        let init = rand_dbl_vec(&mut rng, 128);
        let scale0: [i8; 4] = core::array::from_fn(|_| rng.next_u32() as i8);
        let (mut fr, mut fc) = (init.clone(), init);
        let (mut sr, mut sc) = (scale0, scale0);
        let er = clpd_fac_read(&mut b.rs, &mut fr, &mut sr, length, use_gain, frame);
        let (st, buf) = b.c();
        let ec = unsafe { c::cref_fac_read(st, buf, fc.as_mut_ptr(), sc.as_mut_ptr(), length, use_gain, frame) };
        assert_eq!((er, fr, sr), (ec, fc, sc), "it {it}");
        b.check("CLpd_FAC_Read");
    }
}

#[test]
fn gains_signal_syn_filt_zero_random() {
    let mut rng = Rng::new(0x12FA_0002);
    for it in 0..20_000 {
        let fac_length = [128, 96][(rng.next_u32() % 2) as usize];
        let data = rand_dbl_vec(&mut rng, 128);
        let gain = rng.i32();
        let alfd = rand_dbl_vec(&mut rng, 32);
        let m = rng.range(1, 3);
        let (mut dr, mut dc) = (data.clone(), data.clone());
        cfac_apply_gains(&mut dr, fac_length, gain, &alfd, m);
        unsafe { c::cref_fac_apply_gains(dc.as_mut_ptr(), fac_length, gain, alfd.as_ptr(), m) };
        assert_eq!(dr, dc, "CFac_ApplyGains");

        let fl = [128, 96, 64, 48, 32][(rng.next_u32() % 5) as usize];
        let lsp = rand_lsp(&mut rng);
        let mut a = [0i16; 16];
        let mut a_exp = 0;
        e_lpc_f_lsp_a_conversion(&lsp, &mut a, &mut a_exp);
        let fac = if it % 2 == 0 { rand_dbl_vec(&mut rng, 128) } else { (0..128).map(|_| rng.range(-200, 200) << rng.range(0, 20)).collect() };
        let scale = rng.range(-40, 40);
        let is_fd = (rng.next_u32() % 2) as i32;
        let (mut fr, mut fc) = (fac.clone(), fac.clone());
        let (mut or, mut oc) = (vec![0i32; 256], vec![0i32; 256]);
        cfac_calc_fac_signal(&mut or, &mut fr, scale, fl, &a, a_exp, 1, is_fd);
        unsafe { c::cref_fac_calc_fac_signal(oc.as_mut_ptr(), fc.as_mut_ptr(), scale, fl, a.as_ptr(), a_exp, 1, is_fd) };
        assert_eq!((or, fr), (oc, fc), "CFac_CalcFacSignal");

        let x = rand_dbl_vec(&mut rng, 256);
        let len = rng.range(0, 256);
        let (mut xr, mut xc) = (x.clone(), x);
        syn_filt_zero(&a, a_exp, len, &mut xr);
        unsafe { c::cref_fac_syn_filt_zero(a.as_ptr(), a_exp, len, xc.as_mut_ptr()) };
        assert_eq!(xr, xc, "Syn_filt_zero");
    }
}

/// A static channel info with an IMdct state left by the transform `prev` (TCX20/40/80 or FD
/// long/short of a frame of `lframe` samples) and random overlap and ACELP memory.
fn mdct_state(rng: &mut Rng, lframe: i32) -> CAacDecoderStaticChannelInfo {
    let mut s = CAacDecoderStaticChannelInfo::default();
    mdct_init(&mut s.i_mdct, 768);
    for x in s.i_mdct.overlap.iter_mut() {
        *x = (rng.next_u32() as i32) >> rng.range(1, 12);
    }
    s.acelp = rand_mem(rng);
    let m = &mut s.i_mdct;
    let ldiv = lframe / 4;
    let (tl, fr) = match rng.next_u32() % 5 {
        0 => (ldiv, ldiv),            // TCX20
        1 => (2 * ldiv, ldiv),        // TCX40
        2 => (4 * ldiv, ldiv),        // TCX80
        3 => (lframe, lframe / 8),    // FD long start
        _ => (lframe / 8, lframe / 8), // FD short
    };
    m.prev_tl = tl;
    m.prev_fr = fr;
    m.prev_nr = (tl - fr) >> 1;
    m.prev_wrs = fdk_get_window_slope(fr, 0);
    m.ov_offset = [0, lframe / 2, ldiv][(rng.next_u32() % 3) as usize];
    m.prev_alias_symmetry = 0;
    m.prev_prev_alias_symmetry = 0;
    s
}

#[test]
fn mdct2acelp_random_states() {
    let mut rng = Rng::new(0x12FA_0003);
    for it in 0..20_000 {
        let lframe = if it % 2 == 0 { 1024 } else { 768 };
        let s = mdct_state(&mut rng, lframe);
        let mut sci_v = sci_to_vec(&s);
        let mut sr = sci_from_vec(&sci_v);
        let lfac = lframe / 8;
        let fac_length = if rng.next_u32() % 4 == 0 { lfac / 2 } else { lfac };
        let has_fac = rng.next_u32() % 4 != 0;
        let fac = rand_dbl_vec(&mut rng, 128);
        let fac_scale = rng.range(-10, 40);
        let lsp = rand_lsp(&mut rng);
        let mut a = [0i16; 16];
        let mut a_exp = 0;
        e_lpc_f_lsp_a_conversion(&lsp, &mut a, &mut a_exp);
        let out = rand_dbl_vec(&mut rng, 1883);
        let off = 347 + [0, 64, 256, 512][(rng.next_u32() % 4) as usize] as usize;
        let nr_out = lframe - (off as i32 - 347);
        let is_fd = (rng.next_u32() % 2) as i32;
        if rng.next_u32() % 3 == 0 {
            // pFacZir of a preceding ACELP -> TCX20 transition: the excitation memory from
            // fac_length on (C keeps a pointer to it)
            let z = sr.acelp.old_exc_mem[lfac as usize..].to_vec();
            sr.i_mdct.p_fac_zir = Some(z);
            sci_v = sci_to_vec(&sr);
            sr = sci_from_vec(&sci_v);
        }
        let (mut outr, mut outc) = (out.clone(), out);
        let (mut fr, mut fc) = (fac.clone(), fac);
        let nr = clpd_fac_mdct2acelp(
            &mut sr.i_mdct,
            &mut outr,
            off,
            if has_fac { Some(&mut fr[..]) } else { None },
            fac_scale,
            &a,
            a_exp,
            nr_out,
            fac_length,
            is_fd,
            0,
        );
        let nc = unsafe {
            c::cref_fac_mdct2acelp(sci_v.as_mut_ptr(), outc.as_mut_ptr(), off as i32, has_fac as i32, fc.as_mut_ptr(), fac_scale, a.as_mut_ptr(), a_exp, nr_out, fac_length, is_fd, 0)
        };
        assert_eq!(nr, nc, "nrSamples");
        assert_vec_eq(&outr, &outc, "output");
        assert_eq!(fr, fc, "FAC data");
        assert_vec_eq(&sci_to_vec(&sr), &sci_v, "state");
    }
}

#[test]
fn acelp2mdct_random_states() {
    let mut rng = Rng::new(0x12FA_0004);
    for it in 0..20_000 {
        let lframe = if it % 2 == 0 { 1024 } else { 768 };
        let ldiv = lframe / 4;
        let mut s = mdct_state(&mut rng, lframe);
        // after ACELP (CLpd_FAC_Mdct2Acelp() leaves prev_fr = prev_nr = prev_tl = 0)
        s.i_mdct.prev_fr = 0;
        s.i_mdct.prev_nr = 0;
        s.i_mdct.prev_tl = 0;
        s.i_mdct.ov_offset = [0, lframe / 2, ldiv, lframe / 2 + ldiv][(rng.next_u32() % 4) as usize];
        let mut sci_v = sci_to_vec(&s);
        let mut sr = sci_from_vec(&sci_v);
        // the transition: TCX20/40/80 (fac_length = lFac) or FD long/short
        let (tl, fr, fac_length, n_spec, is_fd, out_base) = match rng.next_u32() % 5 {
            0 => (ldiv, ldiv, ldiv / 2, 1, 0, 347),
            1 => (2 * ldiv, ldiv, ldiv / 2, 1, 0, 347),
            2 => (4 * ldiv, ldiv, ldiv / 2, 1, 0, 347),
            3 => (lframe, lframe / 8, lframe / 8, 1, 1, 347),
            _ => (lframe / 8, lframe / 8, lframe / 16, 8, 1, 347),
        };
        let nr_pos = if is_fd != 0 { 0 } else { [0, ldiv, 2 * ldiv, 3 * ldiv][(rng.next_u32() % 4) as usize] };
        let off = out_base + nr_pos as usize;
        let no_out = lframe - nr_pos;
        let sh = rng.range(4, 30) as u32;
        let spec: Vec<i32> = (0..1024).map(|_| (rng.next_u32() as i32) >> sh).collect();
        let spec_scale: Vec<i16> = (0..8).map(|_| rng.range(-5, 25) as i16).collect();
        let fac = rand_dbl_vec(&mut rng, 128);
        let fac_scale = rng.range(-10, 40);
        let lsp = rand_lsp(&mut rng);
        let mut a = [0i16; 16];
        let mut a_exp = 0;
        e_lpc_f_lsp_a_conversion(&lsp, &mut a, &mut a_exp);
        let gain = if is_fd != 0 { 0 } else { (rng.next_u32() >> 1) as i32 };
        let lost = (rng.next_u32() % 4 == 0) as i32;
        let last_lpd = [0u8, 4][(rng.next_u32() % 2) as usize];
        let k = rng.range(0, 3);
        let shape = (rng.next_u32() % 2) as i32;
        let wrs = fdk_get_window_slope(fr, shape);
        let wrs_id = win_id(wrs);
        let out = rand_dbl_vec(&mut rng, 2048);
        let (mut outr, mut outc) = (out.clone(), out);
        let (mut sr_spec, mut sc_spec) = (spec.clone(), spec);
        let (mut fr_, mut fc_) = (fac.clone(), fac);
        let nr = clpd_fac_acelp2mdct(
            &mut sr.i_mdct,
            &mut outr,
            off,
            &mut sr_spec,
            &spec_scale,
            n_spec,
            &mut fr_[..fac_length as usize],
            fac_scale,
            fac_length,
            no_out,
            tl,
            wrs,
            fr,
            &a,
            a_exp,
            &mut sr.acelp,
            gain,
            lost,
            is_fd,
            last_lpd,
            k,
            0,
        );
        let nc = unsafe {
            c::cref_fac_acelp2mdct(
                sci_v.as_mut_ptr(),
                outc.as_mut_ptr(),
                off as i32,
                sc_spec.as_mut_ptr(),
                spec_scale.as_ptr(),
                n_spec,
                fc_.as_mut_ptr(),
                fac_scale,
                fac_length,
                no_out,
                tl,
                wrs_id,
                fr,
                a.as_mut_ptr(),
                a_exp,
                gain,
                lost,
                is_fd,
                last_lpd,
                k,
                0,
            )
        };
        assert_eq!(nr, nc, "nrSamples");
        assert_eq!(outr, outc, "output (tl {tl} fr {fr} fac {fac_length} nSpec {n_spec})");
        assert_eq!(sr_spec, sc_spec, "spectrum");
        assert_eq!(fr_, fc_, "FAC data");
        assert_eq!(sci_to_vec(&sr), sci_v, "state (tl {tl} fr {fr} fac {fac_length} nSpec {n_spec})");
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

/// GAIN_FAC (`gainFac`, file-static in usacdec_fac.cpp), element by element.
#[test]
fn gain_fac() {
    let f = fdk_aac_rs_cref::aacdec_usacdec_fac::cref_fac_table;
    assert_eq!(c_table::<i32>(f, "gainFac"), GAIN_FAC.to_vec());
}

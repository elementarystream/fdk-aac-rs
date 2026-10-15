// Differential tests of fdk_aac_rs_aacdec::usacdec_lpd against the C++ reference.

use crate::usacdec_acelp::rand_lsp;
use crate::usacdec_lpd_state::*;
use crate::usacdec_lpd_util::{rand_dbl_vec, Bs};
use fdk_aac_rs_aacdec::channelinfo::{
    CAacDecoderChannelInfo, CAacDecoderStaticChannelInfo, CWorkBufferCore1, SamplingRateInfo,
    AACDEC_RENDER_LPD, FD_LONG, FD_SHORT, LPD,
};
use fdk_aac_rs_aacdec::usacdec_lpc::e_lpc_f_lsp_a_conversion;
use fdk_aac_rs_aacdec::block::get_gain;
use fdk_aac_rs_aacdec::usacdec_lpd::*;
use fdk_aac_rs_arith::ac_arith_coder::CArcoData;
use fdk_aac_rs_cref::aacdec_usacdec_lpd as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::{AC_INDEP, AC_USAC};
use fdk_aac_rs_fdk::mdct::mdct_init;

#[test]
fn state_layout_matches_the_shim() {
    let sci = CAacDecoderStaticChannelInfo::default();
    let ci = CAacDecoderChannelInfo::default();
    assert_eq!(sci_to_vec(&sci).len() as i32, unsafe { c::cref_lpd_sci_ints() });
    assert_eq!(ci_to_vec(&ci).len() as i32, unsafe { c::cref_lpd_ci_ints() });
    for (i, &(l, s)) in WIN_CAND.iter().enumerate() {
        let id = unsafe { c::cref_lpd_win_id(l, s) };
        assert_eq!(id, win_id(fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope(l, s)), "{l} {s}");
        assert!(id >= 0 && id <= i as i32);
    }
}

/// A Rust channel and its C++ counterpart (as INT arrays).
pub struct Chan {
    pub sci: CAacDecoderStaticChannelInfo,
    pub ci: CAacDecoderChannelInfo,
    pub spec: Vec<i32>,
    pub wb1: Box<CWorkBufferCore1>,
    pub sci_v: Vec<i32>,
    pub ci_v: Vec<i32>,
    pub spec_c: Vec<i32>,
    pub wb1_c: Vec<i32>,
}

impl Chan {
    pub fn new(sci: CAacDecoderStaticChannelInfo, ci: CAacDecoderChannelInfo, spec: Vec<i32>, wb1: Vec<i32>) -> Chan {
        let sci_v = sci_to_vec(&sci);
        let ci_v = ci_to_vec(&ci);
        let mut w = Box::new(CWorkBufferCore1::default());
        w.buf.copy_from_slice(&wb1);
        // The Rust state goes through the serialization once, so both start alike.
        let sci = sci_from_vec(&sci_v);
        let ci = ci_from_vec(&ci_v);
        Chan { sci, ci, spec: spec.clone(), wb1: w, sci_v, ci_v, spec_c: spec, wb1_c: wb1 }
    }

    pub fn check(&self, what: &str) {
        let sv = sci_to_vec(&self.sci);
        if sv != self.sci_v {
            let i = sv.iter().zip(&self.sci_v).position(|(a, b)| a != b).unwrap();
            panic!("{what}: static channel info differs at {i}: {} != {}", sv[i], self.sci_v[i]);
        }
        let cv = ci_to_vec(&self.ci);
        if cv != self.ci_v {
            let i = cv.iter().zip(&self.ci_v).position(|(a, b)| a != b).unwrap();
            panic!("{what}: channel info differs at {i}: {} != {}", cv[i], self.ci_v[i]);
        }
        if self.spec != self.spec_c {
            let i = self.spec.iter().zip(&self.spec_c).position(|(a, b)| a != b).unwrap();
            panic!("{what}: spectrum differs at {i}: {} != {}", self.spec[i], self.spec_c[i]);
        }
        if self.wb1.buf[..] != self.wb1_c[..] {
            let i = self.wb1.buf.iter().zip(&self.wb1_c).position(|(a, b)| a != b).unwrap();
            panic!("{what}: work buffer differs at {i}: {} != {}", self.wb1.buf[i], self.wb1_c[i]);
        }
    }

    /// `CLpdChannelStream_Read()` on both; returns the error.
    pub fn read(&mut self, b: &mut Bs, sampling_rate: u32, flags: u32) -> i32 {
        let sri = SamplingRateInfo { sampling_rate, ..Default::default() };
        let er = clpd_channel_stream_read(&mut b.rs, &mut self.ci, &mut self.sci, &sri, flags, &mut self.spec);
        let (st, buf) = b.c();
        let ec = unsafe {
            c::cref_lpd_read(st, buf, self.sci_v.as_mut_ptr(), self.ci_v.as_mut_ptr(), self.spec_c.as_mut_ptr(), sampling_rate, flags)
        };
        assert_eq!(er, ec, "CLpdChannelStream_Read error");
        b.check("CLpdChannelStream_Read");
        self.check("CLpdChannelStream_Read");
        er
    }

    /// `CLpdChannelStream_Decode()` on both.
    pub fn decode(&mut self, flags: u32) {
        clpd_channel_stream_decode(&mut self.ci, &mut self.sci, flags, &mut self.spec);
        unsafe { c::cref_lpd_decode(self.sci_v.as_mut_ptr(), self.ci_v.as_mut_ptr(), self.spec_c.as_mut_ptr(), flags) };
        self.check("CLpdChannelStream_Decode");
    }

    /// `CLpd_RenderTimeSignal()` on both; returns the error and the output.
    pub fn render(&mut self, l_frame: i32, sampling_rate: u32, frame_ok: u32, headroom: i32, flags: u32, time_init: &[i32]) -> (i32, Vec<i32>) {
        let sri = SamplingRateInfo { sampling_rate, ..Default::default() };
        let mut tr = time_init.to_vec();
        let er = clpd_render_time_signal(
            &mut self.sci,
            &mut self.ci,
            &mut tr,
            l_frame,
            &sri,
            frame_ok,
            headroom,
            flags,
            0,
            &mut self.spec,
            &mut self.wb1,
        );
        let mut tc = time_init.to_vec();
        let ec = unsafe {
            c::cref_lpd_render(
                self.sci_v.as_mut_ptr(),
                self.ci_v.as_mut_ptr(),
                self.spec_c.as_mut_ptr(),
                self.wb1_c.as_mut_ptr(),
                tc.as_mut_ptr(),
                l_frame,
                sampling_rate,
                frame_ok,
                headroom,
                flags,
                0,
            )
        };
        assert_eq!(er, ec, "CLpd_RenderTimeSignal error");
        if tr != tc {
            let i = tr.iter().zip(&tc).position(|(a, b)| a != b).unwrap();
            panic!("CLpd_RenderTimeSignal output differs at {i}: {} != {}", tr[i], tc[i]);
        }
        self.check("CLpd_RenderTimeSignal");
        (er, tr)
    }
}

/// A channel as the decoder initializes it for USAC (overlap buffer, arithmetic decoder).
pub fn new_channel(rng: &mut Rng, granule_length: i32) -> Chan {
    let mut sci = CAacDecoderStaticChannelInfo::default();
    mdct_init(&mut sci.i_mdct, 768);
    sci.h_ar_co = Some(Box::new(CArcoData::default()));
    sci.concealment_info.prev_frame_ok = [1, 1];
    sci.last_core_mode = FD_LONG;
    let mut ci = CAacDecoderChannelInfo::default();
    ci.granule_length = granule_length;
    ci.render_mode = AACDEC_RENDER_LPD;
    ci.ics_info.window_shape = (rng.next_u32() % 2) as u8;
    let spec = vec![0i32; 1024];
    let wb1 = vec![0i32; 2048];
    Chan::new(sci, ci, spec, wb1)
}

/// The prevFrameOk bookkeeping of the concealment module (simplified, the same on both sides).
fn conceal_update(ch: &mut Chan, frame_ok: bool) {
    let ok = frame_ok as i32;
    let p = ch.sci.concealment_info.prev_frame_ok;
    ch.sci.concealment_info.prev_frame_ok = [ok, p[0]];
    let mut s = sci_from_vec(&ch.sci_v);
    let p = s.concealment_info.prev_frame_ok;
    s.concealment_info.prev_frame_ok = [ok, p[0]];
    // only the two values change; reserialize the C state with them
    let nv = sci_to_vec(&s);
    let n = nv.len();
    ch.sci_v[n - 2] = nv[n - 2];
    ch.sci_v[n - 1] = nv[n - 1];
}

#[test]
fn small_functions_random() {
    let mut rng = Rng::new(0x12D0_0001);
    // UsacRandomSign
    for _ in 0..100_000 {
        let s = rng.next_u32();
        let (mut sr, mut sc) = (s, s);
        let r = usac_random_sign(&mut sr);
        let cc = unsafe { c::cref_lpd_usac_random_sign(&mut sc) };
        assert_eq!((r, sr), (cc, sc));
    }
    // CLpd_ReadAndMapLpdModeToModArray: every lpd_mode
    for mode in 0..32u8 {
        let mut b = Bs::from_bytes(&[mode << 3, 0, 0, 0]);
        let init = [9u8, 8, 7, 6];
        let mut mr = init;
        let er = clpd_read_and_map_lpd_mode_to_mod_array(&mut mr, &mut b.rs, 0);
        let mut mc = init;
        let (st, buf) = b.c();
        let ec = unsafe { c::cref_lpd_read_mod(st, buf, mc.as_mut_ptr()) };
        assert_eq!((er, mr), (ec, mc), "lpd_mode {mode}");
        b.check("lpd_mode");
    }
    // get_gain, calcEnergy, find_mpitch
    for it in 0..20_000 {
        let x = rand_dbl_vec(&mut rng, 64);
        let y = rand_dbl_vec(&mut rng, 64);
        let n = if it % 3 == 0 { rng.range(1, 64) } else { 64 };
        assert_eq!(get_gain(&x, &y, n), unsafe { c::cref_lpd_get_gain(x.as_ptr(), y.as_ptr(), n) }, "get_gain");

        let lg = [192, 256, 384, 512, 768, 1024][(rng.next_u32() % 6) as usize];
        let r = rand_dbl_vec(&mut rng, 1024);
        let rs = rng.range(-40, 40) as i16;
        let (mut er, mut ec) = (0, 0);
        let mr = calc_energy(&r, rs, lg, &mut er);
        let mc = unsafe { c::cref_lpd_calc_energy(r.as_ptr(), rs, lg, &mut ec) };
        assert_eq!((mr, er), (mc, ec), "calcEnergy");

        let mut rc = r.clone();
        assert_eq!(find_mpitch(&r, lg), unsafe { c::cref_lpd_find_mpitch(rc.as_mut_ptr(), lg) }, "find_mpitch");
    }
}

#[test]
fn find_mpitch_never_returns_zero() {
    // Every peak bin of every transform length of the decoder: the TCX time domain
    // concealment needs a pitch other than 0.
    for lg in [192, 256, 384, 512, 768, 1024] {
        let mut i = 2;
        while i < (lg >> 4) {
            let mut x = vec![0i32; 1024];
            x[i as usize] = 1 << 20;
            let r = find_mpitch(&x, lg);
            let mut xc = x.clone();
            assert_eq!(r, unsafe { c::cref_lpd_find_mpitch(xc.as_mut_ptr(), lg) });
            assert!(r > 0, "lg {lg} bin {i}: pitch {r}");
            i += 2;
        }
    }
}

#[test]
fn bass_postfilter_and_filt_lp_random() {
    let mut rng = Rng::new(0x12D0_0002);
    for it in 0..3_000 {
        let headroom = [3, 5, 8, 11][(rng.next_u32() % 4) as usize];
        let syn = rand_dbl_vec(&mut rng, 64);
        let noise = rand_dbl_vec(&mut rng, 140);
        let mut or = vec![0i32; 64];
        let mut oc = vec![0i32; 64];
        let stop = if it % 2 == 0 { 64 } else { rng.range(0, 64) };
        filt_lp(&syn, &mut or, &noise, 12, &fdk_aac_rs_aacdec::usacdec_rom::FDK_DEC_FILT_LP, headroom, stop, 12);
        let mut nc = noise.clone();
        unsafe { c::cref_lpd_filt_lp(syn.as_ptr(), oc.as_mut_ptr(), nc.as_mut_ptr(), 12, headroom, stop, 12) };
        assert_eq!(or, oc, "filtLP");

        // bass_pf_1sf_delay with the parameters of CLpd_RenderTimeSignal() and
        // CBlock_FrequencyToTime()
        let l_frame = if it % 2 == 0 { 1024 } else { 768 };
        let (frame_length, lf, l_next) = match it % 3 {
            0 => (l_frame, l_frame, (l_frame / 128 - 1) * 64),
            1 => (l_frame, l_frame, (l_frame / 128 - 1) * 64 - l_frame / 8),
            _ => {
                let lpd_sfd = l_frame / 128;
                (l_frame, (lpd_sfd + 2) * 64 + 64, l_frame - (lpd_sfd + 4) * 64)
            }
        };
        let sh = rng.range(0, 24) as u32;
        let syn: Vec<i32> = (0..1883).map(|_| (rng.next_u32() as i32) >> sh).collect();
        let t_sf: Vec<i32> = (0..23).map(|_| if rng.next_u32() % 4 == 0 { 64 } else { rng.range(17, 411) }).collect();
        let gains: Vec<i32> = (0..23).map(|_| match rng.next_u32() % 4 { 0 => 0, 1 => rng.i32(), _ => rng.range(0, 1 << 15) }).collect();
        let mem = rand_dbl_vec(&mut rng, 76);
        let (mut mr, mut mc) = (mem.clone(), mem);
        let mut outr = vec![0i32; 1024];
        let mut outc = vec![0i32; 1024];
        bass_pf_1sf_delay(&syn, 411, &t_sf, &gains, frame_length, lf, l_next, &mut outr, headroom, &mut mr);
        let mut sc = syn.clone();
        let mut gc = gains.clone();
        unsafe { c::cref_lpd_bass_pf(sc.as_mut_ptr(), 411, t_sf.as_ptr(), gc.as_mut_ptr(), frame_length, lf, l_next, outc.as_mut_ptr(), headroom, mc.as_mut_ptr()) };
        assert_eq!((outr, mr), (outc, mc), "bass_pf_1sf_delay");
    }
}

#[test]
fn fdns_functions_random() {
    let mut rng = Rng::new(0x12D0_0003);
    for it in 0..6_000 {
        let gl = if it % 2 == 0 { 128 } else { 96 };
        let lg = [2 * gl, 4 * gl, 8 * gl][(rng.next_u32() % 3) as usize];
        let npts = gl / 2;
        // CLpd_AdaptLowFreqDeemph
        let sh = rng.range(2, 31) as u32;
        let x: Vec<i32> = (0..1024).map(|_| (rng.next_u32() as i32) >> sh).collect();
        let s = rng.range(2, 33);
        let (mut xr, mut xc) = (x.clone(), x.clone());
        let (mut gr, mut gc) = (vec![0i32; 32], vec![0i32; 32]);
        clpd_adapt_low_freq_deemph(&mut xr, lg, &mut gr, s);
        unsafe { c::cref_lpd_adapt_low_freq_deemph(xc.as_mut_ptr(), lg, gc.as_mut_ptr(), s) };
        assert_eq!((xr, gr), (xc, gc), "CLpd_AdaptLowFreqDeemph");

        // lpc2mdctAndNoiseShaping
        let lsp1 = rand_lsp(&mut rng);
        let lsp2 = rand_lsp(&mut rng);
        let (mut a1, mut a2) = ([0i16; 16], [0i16; 16]);
        let (mut e1, mut e2) = (0, 0);
        e_lpc_f_lsp_a_conversion(&lsp1, &mut a1, &mut e1);
        e_lpc_f_lsp_a_conversion(&lsp2, &mut a2, &mut e2);
        let sc0 = rng.range(-10, 40) as i16;
        let (mut rr, mut rc) = (x.clone(), x.clone());
        let (mut sr, mut scc) = (sc0, sc0);
        lpc2mdct_and_noise_shaping(&mut rr, &mut sr, lg, npts, &a1, e1, &a2, e2);
        unsafe { c::cref_lpd_lpc2mdct(rc.as_mut_ptr(), &mut scc, lg, npts, a1.as_ptr(), e1, a2.as_ptr(), e2) };
        assert_eq!((rr, sr), (rc, scc), "lpc2mdctAndNoiseShaping");
    }
}

/// lpc2mdctAndNoiseShaping with LP coefficient exponents that E_LPC_f_lsp_a_conversion() does not
/// produce: every exponent from -40 to 99 (including `A_exp % 32` in 30..=31, where the
/// reference builds' form of `0x80000000 >> fMax(3, A_exp + 2)` gives 0) and random 32-bit ones.
#[test]
fn lpc2mdct_random_exponents() {
    let mut rng = Rng::new(0x12D0_0034);
    for it in 0..40_000 {
        let gl = if it % 2 == 0 { 128 } else { 96 };
        let lg = [2 * gl, 4 * gl, 8 * gl][(rng.next_u32() % 3) as usize];
        let npts = gl / 2;
        let sh = rng.range(2, 31) as u32;
        let x: Vec<i32> = (0..1024).map(|_| (rng.next_u32() as i32) >> sh).collect();
        let (mut a1, mut a2) = ([0i16; 16], [0i16; 16]);
        let (mut e1, mut e2) = (0, 0);
        e_lpc_f_lsp_a_conversion(&rand_lsp(&mut rng), &mut a1, &mut e1);
        e_lpc_f_lsp_a_conversion(&rand_lsp(&mut rng), &mut a2, &mut e2);
        let e = if it % 4 == 3 { rng.i32() } else { -40 + (it / 4 % 140) as i32 };
        match it % 3 {
            0 => e1 = e,
            1 => e2 = e,
            _ => (e1, e2) = (e, rng.range(-40, 99)),
        }
        let sc0 = rng.range(-10, 40) as i16;
        let (mut rr, mut rc) = (x.clone(), x.clone());
        let (mut sr, mut scc) = (sc0, sc0);
        lpc2mdct_and_noise_shaping(&mut rr, &mut sr, lg, npts, &a1, e1, &a2, e2);
        unsafe { c::cref_lpd_lpc2mdct(rc.as_mut_ptr(), &mut scc, lg, npts, a1.as_ptr(), e1, a2.as_ptr(), e2) };
        assert!((&rr, sr) == (&rc, scc), "lpc2mdctAndNoiseShaping A1_exp {e1} A2_exp {e2}");
    }
}

/// A random TCX spectrum as CLpd_TCX_Read() leaves it (integers with headroom) and a channel
/// with plausible LPD parameters.
fn tcx_channel(rng: &mut Rng, gl: i32) -> Chan {
    let mut ch = new_channel(rng, gl);
    let sh = rng.range(8, 30) as u32;
    let spec: Vec<i32> = (0..1024).map(|i| if i % 3 == 0 || rng.next_u32() % 4 == 0 { 0 } else { (rng.next_u32() as i32) >> sh }).collect();
    let mut ci = ci_from_vec(&ch.ci_v);
    for k in 0..5 {
        ci.data.usac.lsp_coeff[k] = rand_lsp(rng);
    }
    for k in 0..4 {
        ci.p_dyn_data.specific_to.usac.tcx_noise_factor[k] = (rng.next_u32() % 8) as u8;
        ci.p_dyn_data.specific_to.usac.tcx_global_gain[k] = (rng.next_u32() % 128) as u8;
        ci.spec_scale[k] = rng.range(2, 33) as i16;
    }
    let mut sci = sci_from_vec(&ch.sci_v);
    sci.nf_random_seed = rng.next_u32();
    ch = Chan::new(sci, ci, spec, vec![0; 2048]);
    ch
}

#[test]
fn tcx_gain_and_fdns_decode_random() {
    let mut rng = Rng::new(0x12D0_000A);
    for it in 0..3_000 {
        let gl = if it % 2 == 0 { 128 } else { 96 };
        let mut ch = tcx_channel(&mut rng, gl);
        let frame = rng.range(0, 3) as usize;
        let lg = [2 * gl, 4 * gl, 8 * gl][(rng.next_u32() % 3) as usize];
        // calcTCXGain
        let rms_m = match it % 4 {
            0 => 0,
            1 => rng.i32(),
            _ => (rng.next_u32() >> rng.range(1, 30)) as i32,
        };
        let rms_e = rng.range(-40, 60);
        let spec = ch.spec.clone();
        calc_tcx_gain(&mut ch.ci, &spec, rms_m, rms_e, frame, lg);
        unsafe { c::cref_lpd_calc_tcx_gain(ch.ci_v.as_mut_ptr(), ch.spec_c.as_mut_ptr(), rms_m, rms_e, frame as i32, lg) };
        ch.check("calcTCXGain");

        // CLpd_FdnsDecode on the TCX spectrum of frame `frame` (as CLpd_TcxDecode() calls it)
        let off = fdk_aac_rs_aacdec::overlapadd::spec_tcx(frame as i32, gl, 0);
        let lg = lg.min(1024 - off as i32);
        let lsp1 = rand_lsp(&mut rng);
        let lsp2 = rand_lsp(&mut rng);
        let (mut a1, mut a2) = ([0i16; 16], [0i16; 16]);
        let (mut e1, mut e2) = (0, 0);
        e_lpc_f_lsp_a_conversion(&lsp1, &mut a1, &mut e1);
        e_lpc_f_lsp_a_conversion(&lsp2, &mut a2, &mut e2);
        let mut gains = ch.sci.last_alfd_gains;
        clpd_fdns_decode(&mut ch.ci, &mut ch.spec[off..], lg, frame, &a1, e1, &a2, e2, &mut gains, gl / 2);
        ch.sci.last_alfd_gains = gains;
        unsafe {
            c::cref_lpd_fdns_decode(
                ch.sci_v.as_mut_ptr(),
                ch.ci_v.as_mut_ptr(),
                ch.spec_c.as_mut_ptr(),
                off as i32,
                lg,
                frame as i32,
                a1.as_ptr(),
                e1,
                a2.as_ptr(),
                e2,
                gl / 2,
            )
        };
        ch.check("CLpd_FdnsDecode");
    }
}

#[test]
fn tcx_decode_random() {
    let mut rng = Rng::new(0x12D0_0004);
    for it in 0..3_000 {
        let gl = if it % 2 == 0 { 128 } else { 96 };
        let mut ch = tcx_channel(&mut rng, gl);
        let m = rng.range(1, 3);
        let frame = match m {
            1 => rng.range(0, 3),
            2 => 2 * rng.range(0, 1),
            _ => 0,
        };
        let last_mod = [0, 1, 2, 3, 255][(rng.next_u32() % 5) as usize];
        let ok = (rng.next_u32() % 4 != 0) as i32;
        clpd_tcx_decode(&mut ch.ci, &mut ch.sci, AC_USAC, m, last_mod, frame, ok, &mut ch.spec);
        unsafe { c::cref_lpd_tcx_decode(ch.sci_v.as_mut_ptr(), ch.ci_v.as_mut_ptr(), ch.spec_c.as_mut_ptr(), AC_USAC, m, last_mod, frame, ok) };
        ch.check(&format!("CLpd_TcxDecode mod {m} frame {frame} last {last_mod}"));
    }
}

#[test]
fn tcx_read_random_bitstreams() {
    let mut rng = Rng::new(0x12D0_0005);
    for it in 0..3_000 {
        let gl = if it % 2 == 0 { 128 } else { 96 };
        let mut ch = tcx_channel(&mut rng, gl);
        let m = rng.range(1, 3) as usize;
        let lg = [0, 2 * gl, 4 * gl, 8 * gl][m];
        let frame = if m == 3 { 0 } else { rng.range(0, 4 - (1 << (m - 1))) };
        let mut b = Bs::random(&mut rng, 512);
        let first = (rng.next_u32() % 2) as i32;
        let flags = if rng.next_u32() % 2 == 0 { AC_USAC | AC_INDEP } else { AC_USAC };
        let er = clpd_tcx_read(&mut b.rs, &mut ch.ci, &mut ch.sci, lg, first, frame, flags, &mut ch.spec);
        let (st, buf) = b.c();
        let ec = unsafe { c::cref_lpd_tcx_read(st, buf, ch.sci_v.as_mut_ptr(), ch.ci_v.as_mut_ptr(), ch.spec_c.as_mut_ptr(), lg, first, frame, flags) };
        assert_eq!(er, ec);
        b.check("CLpd_TCX_Read");
        ch.check("CLpd_TCX_Read");
    }
}

#[test]
fn reset_and_last_frame_ok() {
    let mut rng = Rng::new(0x12D0_0006);
    for it in 0..200 {
        let mut ch = tcx_channel(&mut rng, 128);
        let mut sci = sci_from_vec(&ch.sci_v);
        sci.old_synth.iter_mut().for_each(|x| *x = rng.i32());
        sci.concealment_info.prev_frame_ok = [rng.range(0, 1), rng.range(0, 1)];
        let ci = ci_from_vec(&ch.ci_v);
        ch = Chan::new(sci, ci, ch.spec.clone(), vec![0; 2048]);
        for before in 0..4 {
            let r = unsafe { c::cref_lpd_last_frame_ok(ch.sci_v.as_mut_ptr(), before) };
            assert_eq!(r, ch.sci.concealment_info.prev_frame_ok[(before & 1) as usize]);
        }
        let keep = (it % 2) as i32;
        clpd_reset(&mut ch.ci, &mut ch.sci, keep);
        unsafe { c::cref_lpd_reset(ch.sci_v.as_mut_ptr(), ch.ci_v.as_mut_ptr(), ch.spec_c.as_mut_ptr(), keep) };
        ch.check("CLpd_Reset");
    }
}

/// Decodes random bitstreams as LPD frames (with FD frames, frame loss and flushes in between),
/// comparing every call.
fn random_sequence(rng: &mut Rng, gl: i32, frames: usize, stats: &mut [usize; 4]) {
    let l_frame = gl * 8;
    let sampling_rate = [6000, 8000, 12800, 16000, 22050, 24000, 5000, 25000][(rng.next_u32() % 8) as usize];
    let headroom = [3, 5, 8, 11][(rng.next_u32() % 4) as usize];
    let mut ch = new_channel(rng, gl);
    // The decoder renders a lost frame as LPD only if the last good frame was LPD (the
    // concealment restores the render mode of the last good frame); the first good LPD frame
    // has reset the LPD state (CLpd_Reset() in CLpdChannelStream_Read()).
    let mut last_ok_lpd = false;
    for f in 0..frames {
        let what = rng.next_u32() % 16;
        let mut frame_ok = 1;
        let mut flags = AC_USAC;
        if f == 0 || what == 0 {
            flags |= AC_INDEP;
        }
        if what == 1 {
            // a lost frame (no bitstream)
            frame_ok = 0;
        } else if what == 2 {
            // an FD frame in between: only the core mode bookkeeping of the FD path
            let mut s = sci_from_vec(&ch.sci_v);
            s.last_core_mode = if rng.next_u32() % 2 == 0 { FD_LONG } else { FD_SHORT };
            ch.sci.last_core_mode = s.last_core_mode;
            let c_ = ci_from_vec(&ch.ci_v);
            let spec = ch.spec_c.clone();
            let wb1 = ch.wb1_c.clone();
            let mut sci = sci_from_vec(&sci_to_vec(&ch.sci));
            sci.last_core_mode = s.last_core_mode;
            ch = Chan::new(sci, c_, spec, wb1);
            last_ok_lpd = false;
            continue;
        } else {
            let mut b = Bs::random(rng, 1024);
            let err = ch.read(&mut b, sampling_rate, flags);
            if err != 0 {
                frame_ok = 0;
                stats[3] += 1;
            } else {
                stats[0] += 1;
                stats[1] += ch.ci.data.usac.r#mod.iter().filter(|&&m| m == 0).count();
                stats[2] += ch.ci.data.usac.r#mod.iter().filter(|&&m| m != 0).count();
                ch.decode(flags);
            }
        }
        if frame_ok == 0 && !last_ok_lpd {
            // concealed as an FD frame
            conceal_update(&mut ch, false);
            continue;
        }
        if frame_ok != 0 {
            last_ok_lpd = true;
        }
        let rflags = if what == 3 { 2 /* AACDEC_FLUSH */ } else { 0 };
        let time: Vec<i32> = (0..l_frame).map(|_| rng.i32()).collect();
        ch.render(l_frame, sampling_rate, frame_ok, headroom, rflags, &time);
        conceal_update(&mut ch, frame_ok != 0);
        let _ = LPD;
    }
}

#[test]
fn lpd_random_sequences_1024() {
    let mut rng = Rng::new(0x12D0_0007);
    let mut stats = [0usize; 4];
    for _ in 0..40 {
        random_sequence(&mut rng, 128, 40, &mut stats);
    }
    eprintln!("frames read {}, ACELP divisions {}, TCX divisions {}, errors {}", stats[0], stats[1], stats[2], stats[3]);
    assert!(stats[0] > 100 && stats[1] > 100 && stats[2] > 100);
}

#[test]
fn lpd_random_sequences_768() {
    let mut rng = Rng::new(0x12D0_0008);
    let mut stats = [0usize; 4];
    for _ in 0..40 {
        random_sequence(&mut rng, 96, 40, &mut stats);
    }
    eprintln!("frames read {}, ACELP divisions {}, TCX divisions {}, errors {}", stats[0], stats[1], stats[2], stats[3]);
    assert!(stats[0] > 100 && stats[1] > 100 && stats[2] > 100);
}

#[test]
fn render_random_wb1_and_spectrum() {
    // Rendering of concealed frames from arbitrary (consistent) states, with random work buffer
    // and spectrum contents.
    let mut rng = Rng::new(0x12D0_0009);
    for it in 0..300 {
        let gl = if it % 2 == 0 { 128 } else { 96 };
        let l_frame = gl * 8;
        let mut ch = tcx_channel(&mut rng, gl);
        let mut sci = sci_from_vec(&ch.sci_v);
        sci.last_core_mode = LPD;
        sci.last_lpd_mode = [0u8, 1, 2, 3, 4, 255][(rng.next_u32() % 6) as usize];
        sci.last_last_lpd_mode = [0u8, 1, 2, 3, 4][(rng.next_u32() % 5) as usize];
        sci.last_tcx_pitch = rng.range(32, 256) as i16;
        sci.last_tcx_gain = (rng.next_u32() >> 2) as i32;
        sci.last_tcx_gain_e = rng.range(-5, 20);
        sci.lpc4_lsf = crate::usacdec_lpd_util::rand_lsf(&mut rng);
        sci.lsf_adaptive_mean = crate::usacdec_lpd_util::rand_lsf(&mut rng);
        sci.old_stability = (rng.next_u32() % 16385) as i16;
        // CLpd_Reset() sets old_T0 to 64 before any LPD frame; decoded lags are at least PIT_MIN.
        sci.acelp.old_t0 = rng.range(16, 411) as u16;
        sci.acelp.old_t0_frac = (rng.next_u32() % 4) as u8;
        sci.i_mdct.ov_offset = l_frame / 2;
        sci.i_mdct.prev_tl = 2 * gl;
        sci.i_mdct.prev_fr = 2 * gl;
        sci.i_mdct.prev_nr = 0;
        sci.i_mdct.prev_wrs = fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope(2 * gl, 0);
        for x in sci.i_mdct.overlap.iter_mut() {
            *x = (rng.next_u32() as i32) >> 8;
        }
        for x in sci.acelp.old_exc_mem.iter_mut() {
            *x = (rng.next_u32() as i32) >> 12;
        }
        for x in sci.old_synth.iter_mut() {
            *x = (rng.next_u32() as i32) >> 8;
        }
        let ci = ci_from_vec(&ch.ci_v);
        let wb1 = rand_dbl_vec(&mut rng, 2048);
        ch = Chan::new(sci, ci, ch.spec_c.clone(), wb1);
        let time = vec![0i32; l_frame as usize];
        for _ in 0..3 {
            ch.render(l_frame, 16000, 0, 5, 0, &time);
        }
    }
}

/// The exponents of lpc2mdctAndNoiseShaping() per frequency bin, recomputed with the port's
/// primitives (for the coverage of `lpc2mdct_large_shifts`): `si1 - si2` (the shift of
/// `g1 >>=` or `g2 >>=`), `g_e` (the shift of `scaleValue(..., g_e)` or `scaleValue(..., -g_e)`)
/// and `shift` (of `scaleValueSaturate(tmp, shift)`).
fn lpc2mdct_exponents(npts: i32, a1: &[i16], e1: i32, a2: &[i16], e2: i32) -> Vec<(i32, i32, i32)> {
    use fdk_aac_rs_fdk::common_fix::*;
    use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm, inv_sqrt_norm2};
    use fdk_aac_rs_fdk::scale::scale_values;
    use fdk_aac_rs_fdk::tools_rom::{SINE_TABLE_384, SINE_TABLE_512};
    let n = npts as usize;
    let mut t = [vec![0i32; 4 * n], vec![0i32; 4 * n]];
    let (tab, step): (&[_], usize) = if npts == 64 { (&SINE_TABLE_512[..], 8) } else { (&SINE_TABLE_384[..], 8) };
    let mut s = [0i32; 2];
    for (j, (a, e)) in [(a1, e1), (a2, e2)].into_iter().enumerate() {
        let mut f = fl2fxconst_dbl(0.92);
        for i in 0..16 {
            let cs = tab[(i + 1) * step];
            let w = f_mult(a[i], f);
            t[j][2 + 2 * i] = f_mult(w, cs.re);
            t[j][3 + 2 * i] = f_mult(w, cs.im).wrapping_neg();
            f = f_mult(f, fl2fxconst_dbl(0.92));
        }
        let fix = f_max(1, e).wrapping_add(2);
        t[j][0] = 0x2000_0000u32.wrapping_shr(f_max(1, e) as u32) as i32;
        scale_values(&mut t[j][2..34], e.wrapping_sub(fix));
        s[j] = fix;
        fdk_aac_rs_fdk::fft::fft(2 * npts, &mut t[j], &mut s[j]);
    }
    if s[0] > s[1] {
        scale_values(&mut t[1][..2 * n], s[1].wrapping_sub(s[0]));
    } else if s[0] < s[1] {
        scale_values(&mut t[0][..2 * n], s[0].wrapping_sub(s[1]));
    }
    let s = s[0].max(s[1]);
    (0..n)
        .map(|k| {
            let g = |t: &[i32]| {
                let (mut re, mut im) = (t[2 * k], t[2 * k + 1]);
                let sh = f_max(f_min(f_norm(re), f_norm(im)) - 1, 0);
                re <<= sh;
                im <<= sh;
                let mut si = 0;
                let g = inv_sqrt_norm2(f_pow2(re).wrapping_add(f_pow2(im)), &mut si);
                (g, si + sh)
            };
            let ((mut g1, si1), (mut g2, si2)) = (g(&t[0]), g(&t[1]));
            let g_e = if si1 > si2 {
                g2 = g2.wrapping_shr((si1 - si2) as u32);
                si1 - s
            } else {
                g1 = g1.wrapping_shr((si2 - si1) as u32);
                si2 - s
            };
            let sum = (g1 >> 1).wrapping_add(g2 >> 1);
            let mut inv_e = 0;
            if sum != 0 {
                f_div_norm(fl2fxconst_dbl(0.5), sum, &mut inv_e);
                inv_e -= g_e;
            }
            let shift = if g_e < 0 { g_e + inv_e + 1 - 4 } else { 2 * g_e + inv_e + 1 - 4 };
            (si1 - si2, g_e, shift)
        })
        .collect()
}

/// lpc2mdctAndNoiseShaping() with shift amounts of 32 or more.
/// `scaleValueSaturate(tmp, shift)` is defined for every amount (it saturates or clears before
/// it would shift by 32 or more; the arm64 code at this call site selects with `csel` on the
/// headroom, as the C semantics require), the undefined operations are `g1 >>= si2 - si1` /
/// `g2 >>= si1 - si2` and `scaleValue(..., +-g_e)`, which all four reference builds compile as
/// scalar shifts by the amount modulo 32, as the port does.
///
/// Part 1, the LP filters that the decoder produces: LSF vectors as `reorder_lsf()` leaves them
/// (gaps of at least 50 Hz, between 50 and 6350 Hz) with the tightest clusters at every position,
/// random ones and convex combinations (concealment), through the decoder's LSF to LSP
/// conversion and `E_LPC_f_lsp_a_conversion()`. Their exponents stay far below 32 (printed).
/// Part 2, arbitrary coefficients and exponents (beyond the decoder): the operations with
/// amounts of 32 or more are reached and the port equals the reference.
#[test]
fn lpc2mdct_large_shifts() {
    use fdk_aac_rs_aacdec::usacdec_lpc::reorder_lsf;
    use fdk_aac_rs_fdk::lpc::fx_dbl2fx_lpc;
    use fdk_aac_rs_fdk::trig_fcts::fixp_cos;
    let mut rng = Rng::new(0x4832_385f);
    let lsf_to_lsp = |lsf: i16| {
        let c = fdk_aac_rs_fdk::common_fix::fl2fxconst_sgl((1 << 10) as f64 * std::f64::consts::PI / 6400.0);
        fx_dbl2fx_lpc(fixp_cos(fdk_aac_rs_fdk::common_fix::f_mult(lsf, c), 13 - 10))
    };
    let check = |rng: &mut Rng, a1: &[i16; 16], e1: i32, a2: &[i16; 16], e2: i32, npts: i32, what: &str| {
        let lg = npts * [2, 4, 8][(rng.next_u32() % 3) as usize];
        let x: Vec<i32> = (0..1024).map(|_| rng.i32() >> rng.range(2, 30)).collect();
        let sc0 = rng.range(-10, 40) as i16;
        let (mut rr, mut rc) = (x.clone(), x);
        let (mut sr, mut scc) = (sc0, sc0);
        lpc2mdct_and_noise_shaping(&mut rr, &mut sr, lg, npts, a1, e1, a2, e2);
        unsafe { c::cref_lpd_lpc2mdct(rc.as_mut_ptr(), &mut scc, lg, npts, a1.as_ptr(), e1, a2.as_ptr(), e2) };
        assert!((&rr, sr) == (&rc, scc), "{what}: A1_exp {e1} A2_exp {e2}");
    };
    // part 1
    let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
    let mut prev = [0i16; 16];
    for it in 0..20_000 {
        let mut lsf = [0i16; 16];
        match it % 4 {
            0 => {
                // a tight cluster of `m` LSFs (50 Hz apart) at a random position
                let m = rng.range(2, 16) as usize;
                let start = rng.range(0, 25600);
                let mut v: Vec<i32> = (0..16).map(|_| rng.range(0, 25600)).collect();
                v.sort();
                for i in 0..m {
                    v[i] = start + 200 * i as i32;
                }
                v.sort();
                for i in 0..16 {
                    lsf[i] = v[i].clamp(0, 32767) as i16;
                }
            }
            1 => {
                // all 16 as close as possible, anywhere
                let start = rng.range(0, 25600 - 200 * 15);
                for i in 0..16 {
                    lsf[i] = (start + 200 * i as i32 + rng.range(0, 3)) as i16;
                }
            }
            2 => {
                // random
                let mut v: Vec<i32> = (0..16).map(|_| rng.range(-1000, 27000)).collect();
                v.sort();
                for i in 0..16 {
                    lsf[i] = v[i] as i16;
                }
            }
            _ => {
                // a convex combination with the previous vector (as the concealment's fades)
                let w = rng.range(0, 32768) as i32;
                let mut v: Vec<i32> = (0..16).map(|_| rng.range(0, 25600)).collect();
                v.sort();
                for i in 0..16 {
                    lsf[i] = ((prev[i] as i32 * w + v[i] * (32768 - w)) >> 15) as i16;
                }
            }
        }
        if it % 4 != 3 {
            reorder_lsf(&mut lsf, 200, 16);
        }
        prev = lsf;
        let lsp = lsf.map(lsf_to_lsp);
        let (mut a, mut e) = ([0i16; 16], 0);
        e_lpc_f_lsp_a_conversion(&lsp, &mut a, &mut e);
        // against the flat filter and against a random decoder filter
        let lsp2 = rand_lsp(&mut rng);
        let (mut a2, mut e2) = ([0i16; 16], 0);
        e_lpc_f_lsp_a_conversion(&lsp2, &mut a2, &mut e2);
        let npts = if it % 2 == 0 { 64 } else { 48 };
        for (x1, y1, x2, y2) in [(&a, e, &a2, e2), (&a2, e2, &a, e)] {
            for (d, g, sh) in lpc2mdct_exponents(npts, x1, y1, x2, y2) {
                for (j, v) in [d, g, sh].into_iter().enumerate() {
                    lo[j] = lo[j].min(v);
                    hi[j] = hi[j].max(v);
                }
            }
            check(&mut rng, x1, y1, x2, y2, npts, &format!("decoder filters {it}"));
        }
    }
    eprintln!(
        "lpc2mdct_large_shifts: decoder filters: si1 - si2 in {}..={}, g_e in {}..={}, shift in {}..={}",
        lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]
    );
    assert!((0..3).all(|j| lo[j] > -32 && hi[j] < 32));
    // part 2
    let mut beyond = [0usize; 3];
    for it in 0..40_000 {
        let mut a = [[0i16; 16]; 2];
        let mut e = [0i32; 2];
        for j in 0..2 {
            match rng.range(0, 3) {
                0 => {
                    let (mut x, mut y) = ([0i16; 16], 0);
                    e_lpc_f_lsp_a_conversion(&rand_lsp(&mut rng), &mut x, &mut y);
                    a[j] = x;
                    e[j] = y;
                }
                1 => {
                    // 1 - z^-1 (weighted: a zero near z = 1), with a huge or tiny scale
                    a[j][0] = -17809;
                    e[j] = 1;
                }
                2 => {
                    for v in a[j].iter_mut() {
                        *v = rng.i16() >> rng.range(0, 15);
                    }
                    e[j] = rng.range(-2, 8);
                }
                _ => {
                    for v in a[j].iter_mut() {
                        *v = [i16::MIN, i16::MAX, 0, 1, -1][rng.range(0, 4) as usize];
                    }
                    e[j] = rng.range(0, 8);
                }
            }
            if rng.next_u32() % 2 == 0 {
                e[j] = rng.range(-40, 99);
            }
        }
        let npts = if it % 2 == 0 { 64 } else { 48 };
        for (d, g, sh) in lpc2mdct_exponents(npts, &a[0], e[0], &a[1], e[1]) {
            for (j, v) in [d, g, sh].into_iter().enumerate() {
                beyond[j] += (v.abs() >= 32) as usize;
                lo[j] = lo[j].min(v);
                hi[j] = hi[j].max(v);
            }
        }
        check(&mut rng, &a[0], e[0], &a[1], e[1], npts, &format!("arbitrary filters {it}"));
    }
    eprintln!(
        "lpc2mdct_large_shifts: all filters: si1 - si2 in {}..={}, g_e in {}..={}, shift in {}..={}; bins with \
         |si1 - si2| >= 32: {}, |g_e| >= 32: {}, |shift| >= 32: {}",
        lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], beyond[0], beyond[1], beyond[2]
    );
    // scaleValue() with amounts of 32 or more (|g_e|) is reached and compared; the amount of
    // scaleValueSaturate() stays within -3..=23 for any input (see above).
    assert!(beyond[1] > 1000, "{beyond:?}");
}

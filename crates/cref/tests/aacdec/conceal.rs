// Differential tests of fdk_aac_rs_aacdec::conceal against the C++ reference.
//
// The parameter functions on random and edge-case values; CConcealment_Store; sequences of good
// and bad frames through CConcealment_Apply, CBlock_FrequencyToTime and CConcealment_TDFading for
// every concealment method (including undefined method numbers, which conceal like muting),
// random fade/mute parameters and attenuation vectors, long and short blocks, every frame
// length, the IMDCT and the LPD render mode, with the block test context (block_ctx.rs); the
// complete state and the time output are compared after each step, including the LPC fields that
// CLpc_Conceal() writes for every bad frame of an IMDCT channel (data.usac.lsp_coeff, lpc4_lsf,
// lsf_adaptive_mean and the copy in concealmentInfo.lsf4).

use crate::block_ctx::*;
use fdk_aac_rs_aacdec::block::cblock_frequency_to_time;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_aacdec::conceal::*;
use fdk_aac_rs_cref::aacdec_conceal as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::MAXVAL_DBL;

#[test]
fn parameters() {
    let mut rng = Rng::new(0xc0c_0001);
    let mut p = Pair::new();
    for it in 0..20000 {
        let is_null = (rng.next_u32() % 8 == 0) as i32;
        p.r.usac = false;
        let n = p.r.dump().v.len();
        let v = random_image(&mut rng, n);
        p.load(&v);
        let what = format!("parameters {it}");
        match rng.next_u32() % 6 {
            0 => {
                unsafe { c::cref_conceal_init_common_data(p.c.0, is_null) };
                cconcealment_init_common_data(if is_null != 0 { None } else { Some(&mut p.r.params) });
            }
            1 => {
                let r = cconcealment_get_method(if is_null != 0 { None } else { Some(&p.r.params) });
                assert_eq!(r, unsafe { c::cref_conceal_get_method(p.c.0, is_null) }, "{what}");
                let r = cconcealment_get_delay(if is_null != 0 { None } else { Some(&p.r.params) });
                assert_eq!(r, unsafe { c::cref_conceal_get_delay(p.c.0, is_null) }, "{what}");
            }
            2 => {
                let ch = (rng.next_u32() % 2) as usize;
                let ci = &p.r.sci[ch].concealment_info;
                let r = cconcealment_get_state(if is_null != 0 { None } else { Some(ci) });
                assert_eq!(r, unsafe { c::cref_conceal_get_state(p.c.0, ch as i32, is_null) }, "{what}");
                let f = rng.i32();
                let r = cconcealment_get_last_frame_ok(if is_null != 0 { None } else { Some(ci) }, f);
                assert_eq!(r, unsafe { c::cref_conceal_get_last_frame_ok(p.c.0, ch as i32, is_null, f) }, "{what}");
            }
            3 => {
                let val = |rng: &mut Rng| match rng.next_u32() % 5 {
                    0 => AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,
                    1 => rng.i32(),
                    _ => rng.range(-3, 70),
                };
                let (m, fo, fi, mr) = (val(&mut rng), val(&mut rng), val(&mut rng), val(&mut rng));
                let cn = match rng.next_u32() % 4 {
                    0 => AACDEC_CONCEAL_PARAM_NOT_SPECIFIED,
                    1 => rng.i32(),
                    2 => MAXVAL_DBL,
                    _ => rng.range(-10, 0x200000),
                };
                let ce = unsafe { c::cref_conceal_set_params(p.c.0, is_null, m, fo, fi, mr, cn) };
                let re = cconcealment_set_params(if is_null != 0 { None } else { Some(&mut p.r.params) }, m, fo, fi, mr, cn);
                assert_eq!(re, ce, "{what}: SetParams({m}, {fo}, {fi}, {mr}, {cn})");
            }
            4 => {
                let vec = |rng: &mut Rng| -> Option<Vec<i16>> {
                    match rng.next_u32() % 4 {
                        0 => None,
                        1 => Some((0..32).map(|_| rng.range(-2, 260) as i16).collect()),
                        _ => Some((0..32).map(|_| rng.range(0, 255) as i16).collect()),
                    }
                };
                let (o, i) = (vec(&mut rng), vec(&mut rng));
                let ptr = |x: &Option<Vec<i16>>| x.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
                let ce = unsafe { c::cref_conceal_set_attenuation(p.c.0, is_null, ptr(&o), ptr(&i)) };
                let re = cconcealment_set_attenuation(
                    if is_null != 0 { None } else { Some(&mut p.r.params) },
                    o.as_deref(),
                    i.as_deref(),
                );
                assert_eq!(re, ce, "{what}: SetAttenuation");
            }
            _ => {
                let ch = (rng.next_u32() % 2) as usize;
                let rm = rng.range(0, 4);
                let spf = rng.range(100, 1024);
                unsafe { c::cref_conceal_init_channel_data(p.c.0, ch as i32, rm, spf) };
                cconcealment_init_channel_data(&mut p.r.sci[ch].concealment_info, rm, spf);
            }
        }
        p.check(&what);
    }
}

#[test]
fn store() {
    let mut rng = Rng::new(0xc0c_0002);
    let mut p = Pair::new();
    for it in 0..4000 {
        let usac = rng.next_u32() % 3 == 0;
        random_state(&mut p, &mut rng, usac, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        p.r.params.method = rng.range(-1, 4);
        if rng.next_u32() % 4 == 0 {
            p.r.ci[ch].render_mode = AACDEC_RENDER_LPD;
        }
        if rng.next_u32() % 8 == 0 {
            // a channel that never decoded an ics_info()
            p.r.ci[ch].ics_info.window_groups = 0;
        }
        p.sync();
        unsafe { c::cref_conceal_store(p.c.0, ch as i32) };
        let RCtx { ci, sci, params, spec, .. } = &mut p.r;
        cconcealment_store(&mut ci[ch], &mut sci[ch], params, &mut spec[ch * 1024..]);
        p.check(&format!("CConcealment_Store {it}"));
    }
}

/// One frame of a channel: CConcealment_Apply, the frequency to time transform (IMDCT render
/// mode) and the time domain fading.
fn frame(p: &mut Pair, rng: &mut Rng, ch: usize, spf: u32, frame_ok: i32, what: &str) {
    let headroom = 2;
    let flags = 0;
    let last_lpd = p.r.sci[ch].last_lpd_mode;
    p.sync();
    let ce = unsafe { c::cref_conceal_apply(p.c.0, ch as i32, spf as i32, last_lpd as i32, frame_ok, flags) };
    let re = {
        let sri = p.r.sri;
        let RCtx { ci, sci, params, spec, .. } = &mut p.r;
        cconcealment_apply(&mut ci[ch], &mut sci[ch], &sri, spf as i32, last_lpd, frame_ok, flags, params, &mut spec[ch * 1024..])
    };
    assert_eq!(re, ce, "{what}: CConcealment_Apply");
    p.check(&format!("{what}: CConcealment_Apply"));

    if p.r.ci[ch].render_mode == AACDEC_RENDER_IMDCT && p.r.sci[ch].last_core_mode != LPD {
        let fo = (frame_ok != 0 && ce != -1) as i32;
        unsafe { fdk_aac_rs_cref::aacdec_block::cref_block_frequency_to_time(p.c.0, ch as i32, spf as i32, fo, headroom, 0, ch as i32) };
        let RCtx { ci, sci, spec, wb1, out, .. } = &mut p.r;
        cblock_frequency_to_time(&mut sci[ch], &mut ci[ch], &mut out[ch], spf as i16, fo, &mut wb1.buf, headroom, 0, ch as i32, &mut spec[ch * 1024..]);
        p.check(&format!("{what}: CBlock_FrequencyToTime"));
    } else {
        // The random LPD states of these sequences are not states that the LPD decoder can
        // reach (pitch lags, LP coefficient exponents, ...), so random time data stands in for
        // CLpd_RenderTimeSignal() and the LPD branch of CBlock_FrequencyToTime(); both are
        // compared on the LPD streams of channel_real.rs and in block.rs / usacdec_lpd.rs.
        for x in p.r.out[ch].iter_mut() {
            *x = random_spec(rng) >> 4;
        }
        p.sync();
    }

    let ce = unsafe { c::cref_conceal_td_fading(p.c.0, ch as i32, spf as i32, headroom) };
    let re = {
        let RCtx { sci, params, out, .. } = &mut p.r;
        cconcealment_td_fading(spf as i32, &mut sci[ch], params, headroom, &mut out[ch])
    };
    assert_eq!(re, ce, "{what}: CConcealment_TDFading");
    p.check(&format!("{what}: CConcealment_TDFading"));
}

fn random_params(p: &mut Pair, rng: &mut Rng) {
    cconcealment_init_common_data(Some(&mut p.r.params));
    let pr = &mut p.r.params;
    pr.method = [0, 1, 2, 2, 1, 3, 5, -1][(rng.next_u32() % 8) as usize];
    pr.num_fade_out_frames = [0, 1, 3, 6, 31, rng.range(0, 31)][(rng.next_u32() % 6) as usize];
    pr.num_fade_in_frames = [0, 1, 5, 31, rng.range(0, 31)][(rng.next_u32() % 5) as usize];
    pr.num_mute_release_frames = [0, 0, 1, 3, 63, rng.range(0, 63)][(rng.next_u32() % 6) as usize];
    pr.comfort_noise_level = [0, 0x100000, MAXVAL_DBL, rng.range(0, 0x7FFFFFF)][(rng.next_u32() % 4) as usize];
    if rng.next_u32() % 3 == 0 {
        let o: Vec<i16> = (0..32).map(|_| rng.range(0, 255) as i16).collect();
        let i: Vec<i16> = (0..32).map(|_| rng.range(0, 255) as i16).collect();
        cconcealment_set_attenuation(Some(pr), Some(&o), Some(&i));
    }
}

/// Sequences of good and bad frames with every concealment method.
#[test]
fn frame_sequences() {
    let mut rng = Rng::new(0xc0c_0003);
    let mut p = Pair::new();
    for seq in 0..250 {
        let spf = random_state(&mut p, &mut rng, false, true, true);
        let ld = spf == 512 || spf == 480;
        random_params(&mut p, &mut rng);
        for ch in 0..2 {
            cconcealment_init_channel_data(&mut p.r.sci[ch].concealment_info, AACDEC_RENDER_IMDCT, spf as i32);
            p.r.sci[ch].last_core_mode = FD_LONG;
            p.r.sci[ch].last_lpd_mode = 255;
            p.r.ci[ch].render_mode = AACDEC_RENDER_IMDCT;
            p.r.ci[ch].curr_aliasing_symmetry = 0;
        }
        // bad frame patterns: single losses, bursts, long bursts, random
        let pattern = rng.next_u32() % 4;
        let nframes = 30;
        for f in 0..nframes {
            let ch = (rng.next_u32() % 2) as usize;
            let ok = match pattern {
                0 => rng.next_u32() % 5 != 0,
                1 => !(5..9).contains(&f),
                2 => !(3..20).contains(&f),
                _ => rng.next_u32() % 2 == 0,
            };
            if ok {
                // a decoded frame: new window, spectrum and scaling
                let sri = p.r.sri;
                random_ics(&mut rng, &mut p.r.ci[ch].ics_info, &sri, !ld);
                if ld {
                    p.r.ci[ch].ics_info.window_sequence = 0;
                    p.r.ci[ch].ics_info.window_shape = [0, 2][(rng.next_u32() % 2) as usize];
                }
                let sh = 4 + rng.next_u32() % 12;
                for i in 0..1024 {
                    p.r.spec[ch * 1024 + i] = random_spec(&mut rng) >> sh;
                }
                for i in 0..8 {
                    p.r.ci[ch].spec_scale[i] = rng.range(-2, 20) as i16;
                }
                p.r.ci[ch].render_mode = AACDEC_RENDER_IMDCT;
            } else {
                // a lost frame: the parser left garbage or nothing
                if rng.next_u32() % 2 == 0 {
                    for i in 0..1024 {
                        p.r.spec[ch * 1024 + i] = random_spec(&mut rng);
                    }
                }
            }
            let what = format!("seq {seq} frame {f} ch {ch} method {}", p.r.params.method);
            frame(&mut p, &mut rng, ch, spf, ok as i32, &what);
        }
    }
}

/// The LPD render mode (USAC): the concealment of the LPD branch and the fade out of the
/// subframes of the last LPD mode.
#[test]
fn frame_sequences_lpd() {
    let mut rng = Rng::new(0xc0c_0004);
    let mut p = Pair::new();
    for seq in 0..250 {
        let spf = loop {
            let spf = random_state(&mut p, &mut rng, true, false, true);
            if spf == 1024 || spf == 768 {
                break spf;
            }
        };
        random_params(&mut p, &mut rng);
        for ch in 0..2 {
            cconcealment_init_channel_data(&mut p.r.sci[ch].concealment_info, AACDEC_RENDER_LPD, spf as i32);
        }
        for f in 0..30 {
            let ch = (rng.next_u32() % 2) as usize;
            let ok = rng.next_u32() % 3 != 0;
            if ok {
                let lpd = rng.next_u32() % 2 == 0;
                p.r.ci[ch].render_mode = if lpd { AACDEC_RENDER_LPD } else { AACDEC_RENDER_IMDCT };
                for m in p.r.ci[ch].data.usac.r#mod.iter_mut() {
                    *m = (rng.next_u32() % 4) as u8;
                }
                p.r.sci[ch].last_lpd_mode = if lpd { (rng.next_u32() % 4) as u8 } else { 255 };
                p.r.sci[ch].last_core_mode = if lpd { LPD } else { FD_LONG };
                let sri = p.r.sri;
                random_ics(&mut rng, &mut p.r.ci[ch].ics_info, &sri, true);
                for i in 0..1024 {
                    p.r.spec[ch * 1024 + i] = random_spec(&mut rng) >> 8;
                }
            }
            frame(&mut p, &mut rng, ch, spf, ok as i32, &format!("lpd seq {seq} frame {f} ch {ch}"));
        }
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

/// FAC_MOD4_TABLE (`facMod4Table`, file-static in conceal.cpp), element by element.
#[test]
fn fac_mod4_table() {
    let f = fdk_aac_rs_cref::aacdec_conceal_tables::cref_conceal_table;
    assert_eq!(c_table::<i16>(f, "facMod4Table"), FAC_MOD4_TABLE.to_vec());
}

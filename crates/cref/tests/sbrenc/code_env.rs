// Differential tests of fdk_aac_rs_sbrenc::code_env against the C++ reference.
//
// The Huffman table setup of both amplitude resolutions (and an invalid one), and sequences of
// FDKsbrEnc_codeEnvelope() calls on random envelope and noise floor data of every band count
// (both frequency resolutions, coupling on both channels, headers, delta coding across frames
// on and off, all dF_edge factors), including values beyond the codebooks. The coded data, the
// direction vectors and the complete SBR_CODE_ENVELOPE are compared after every call. Realistic
// data is covered by the whole-encoder test (sbr_encoder.rs).

use fdk_aac_rs_cref::sbrenc_code_env as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrenc::bit_sbr::SbrEnvData;
use fdk_aac_rs_sbrenc::code_env::*;
use fdk_aac_rs_sbrenc::sbr_def::*;

use super::sbr_encoder_dump::{assert_dump_eq, code_env, env_data, Dump};

type H = *mut core::ffi::c_void;

fn c_dump(h: H) -> Vec<i32> {
    let mut buf = vec![0i32; 1024];
    let n = unsafe { c::cref_code_env_dump(h, buf.as_mut_ptr(), buf.len() as i32) };
    buf.truncate(n as usize);
    buf
}

fn r_dump(h: &SbrCodeEnvelope) -> Vec<i32> {
    let mut d = Dump::default();
    code_env(&mut d, h);
    d.0
}

#[test]
fn huffman_tables() {
    for amp_res in [SBR_AMP_RES_1_5, SBR_AMP_RES_3_0, 2, -1] {
        let n_sfb = [7, 14];
        let hc_env = unsafe { c::cref_code_env_new(n_sfb.as_ptr() as *mut i32, 1, 0x1234567, 0x234567) };
        let hc_noise = unsafe { c::cref_code_env_new(n_sfb.as_ptr() as *mut i32, 0, 0, 0) };
        let mut buf = vec![0i32; 4096];
        let mut n = 0;
        let rc = unsafe { c::cref_code_env_huffman(hc_env, hc_noise, amp_res, buf.as_mut_ptr(), 4096, &mut n) };
        buf.truncate(n as usize);

        let mut env = SbrCodeEnvelope::default();
        let mut noise = SbrCodeEnvelope::default();
        fdk_sbr_enc_init_sbr_code_envelope(&mut env, &n_sfb, 1, 0x1234567, 0x234567);
        fdk_sbr_enc_init_sbr_code_envelope(&mut noise, &n_sfb, 0, 0, 0);
        let mut ed = SbrEnvData::default();
        let rr = fdk_sbr_enc_init_sbr_huffman_tables(&mut ed, &mut env, &mut noise, amp_res);
        assert_eq!(rc, rr, "amp_res {amp_res}");
        let mut d = Dump::default();
        env_data(&mut d, &ed);
        assert_dump_eq(&buf, &d.0, "env data");
        assert_dump_eq(&c_dump(hc_env), &r_dump(&env), "henv");
        assert_dump_eq(&c_dump(hc_noise), &r_dump(&noise), "hnoise");
        unsafe {
            c::cref_code_env_free(hc_env);
            c::cref_code_env_free(hc_noise);
        }
    }
}

#[test]
fn code_envelope_sequences() {
    let mut rng = Rng::new(0x5b2_c0de);
    let mut calls = 0;
    let mut time_coded = 0;
    for case in 0..400 {
        // band counts as the frequency tables give them (nSfb[LO] = nSfb[HI] - nSfb[HI] / 2), and
        // the noise floor band counts (equal)
        let noise = case % 4 == 3;
        let n_hi = if noise { rng.range(1, 5) } else { rng.range(1, 48) };
        let n_lo = if noise { n_hi } else { n_hi - n_hi / 2 };
        let n_sfb = [n_lo, n_hi];
        let delta_t_across = rng.range(0, 1);
        let e1 = if rng.range(0, 1) == 0 { rng.i32() & 0x7fff_ffff } else { 0x2666_6666 };
        let incr = if rng.range(0, 1) == 0 { rng.i32() & 0x7fff_ffff } else { 0x0ccc_cccd };
        let amp_res = rng.range(0, 1);

        let hc_env = unsafe { c::cref_code_env_new(n_sfb.as_ptr() as *mut i32, delta_t_across, e1, incr) };
        let hc_other = unsafe { c::cref_code_env_new(n_sfb.as_ptr() as *mut i32, 0, 0, 0) };
        let mut buf = vec![0i32; 4096];
        let mut n = 0;
        unsafe { c::cref_code_env_huffman(hc_env, hc_other, amp_res, buf.as_mut_ptr(), 4096, &mut n) };

        let mut h = SbrCodeEnvelope::default();
        let mut other = SbrCodeEnvelope::default();
        fdk_sbr_enc_init_sbr_code_envelope(&mut h, &n_sfb, delta_t_across, e1, incr);
        fdk_sbr_enc_init_sbr_code_envelope(&mut other, &n_sfb, 0, 0, 0);
        let mut ed = SbrEnvData::default();
        fdk_sbr_enc_init_sbr_huffman_tables(&mut ed, &mut h, &mut other, amp_res);
        // the noise floor coder uses the second instance
        let (hc, h) = if noise { (hc_other, &mut other) } else { (hc_env, &mut h) };
        assert_dump_eq(&c_dump(hc), &r_dump(h), "init");

        let coupling = rng.range(0, 1);
        let channel = rng.range(0, 1);
        for _frame in 0..12 {
            let n_env = if noise { rng.range(1, 2) } else { rng.range(1, 5) };
            let mut freq_res = [0i32; MAX_ENVELOPES];
            for f in freq_res.iter_mut() {
                *f = if noise { 0 } else { rng.range(0, 1) };
            }
            let mut sfb = [0i8; MAX_NUM_ENVELOPE_VALUES];
            let style = rng.range(0, 3);
            let base = rng.range(0, 60);
            for v in sfb.iter_mut() {
                *v = match style {
                    0 => (base + rng.range(-3, 3)).clamp(0, 127) as i8,
                    1 => rng.range(0, 70) as i8,
                    2 => rng.range(-128, 127) as i8,
                    _ => (base + rng.range(-20, 20)) as i8,
                };
            }
            let fac = rng.range(0, 8);
            let header = (rng.range(0, 7) == 0) as i32;
            let mut sfb_c = sfb;
            let mut dir_c = [-1i32; MAX_ENVELOPES];
            unsafe {
                c::cref_code_env_set_fac(hc, fac);
                c::cref_code_env_code(
                    hc,
                    sfb_c.as_mut_ptr(),
                    freq_res.as_mut_ptr(),
                    dir_c.as_mut_ptr(),
                    coupling,
                    n_env,
                    channel,
                    header,
                );
            }
            h.d_f_edge_incr_fac = fac;
            let mut sfb_r = sfb;
            let mut dir_r = [-1i32; MAX_ENVELOPES];
            fdk_sbr_enc_code_envelope(&mut sfb_r, &freq_res, h, &mut dir_r, coupling, n_env, channel, header);
            assert_eq!(sfb_c, sfb_r, "case {case} coded data");
            assert_eq!(dir_c, dir_r, "case {case} directions");
            assert_dump_eq(&c_dump(hc), &r_dump(h), "state");
            calls += 1;
            time_coded += dir_r[..n_env as usize].iter().filter(|&&d| d == TIME).count();
        }
        unsafe {
            c::cref_code_env_free(hc_env);
            c::cref_code_env_free(hc_other);
        }
    }
    assert!(time_coded > 100, "time coding reached {time_coded} times in {calls} calls");
}

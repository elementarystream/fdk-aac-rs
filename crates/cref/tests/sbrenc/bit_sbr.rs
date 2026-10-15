// Differential tests of fdk_aac_rs_sbrenc::bit_sbr (with env_bit) against the C++ reference.
//
// Whole SBR payloads as FDKsbrEnc_EnvEncodeFrame() writes them (bitstream initialization,
// single or channel pair element, counted channel pair element, assembly with fill bits and
// CRC), from random envelope data within the codebooks of both amplitude resolutions (time and
// frequency coded envelopes and noise floors, coupling with level and balance channels,
// harmonics), random grids of every frame class (also low delay grids and FIXFIXonly), random
// headers (with and without the extra parts) and random PS extension data, with all syntax
// flags (low delay, scalable, CRC, DRM CRC). The payload bytes, the bit counts and the
// COMMON_DATA are compared. Realistic payloads are covered by the whole-encoder test.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::sbrenc_bit_sbr as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::crc::FdkCrcInfo;
use fdk_aac_rs_sbrenc::bit_sbr::*;
use fdk_aac_rs_sbrenc::cmondata::CommonData;
use fdk_aac_rs_sbrenc::code_env::*;
use fdk_aac_rs_sbrenc::env_bit::*;
use fdk_aac_rs_sbrenc::fram_gen::*;
use fdk_aac_rs_sbrenc::ps_bitenc::PsOut;
use fdk_aac_rs_sbrenc::ps_const::*;
use fdk_aac_rs_sbrenc::ps_main::*;
use fdk_aac_rs_sbrenc::sbr_def::*;
use fdk_aac_rs_sbrenc::sbr_encoder::*;

use super::sbr_encoder_dump::{assert_dump_eq, cmon_data, grid, ps_out, Dump};

/// The flat description of `env_from()` in the shim.
fn env_spec(rng: &mut Rng, amp_res: i32, ld_grid: i32, coupling: bool, balance: i32) -> Vec<i32> {
    let mut v = vec![amp_res, ld_grid];
    let n_env = rng.range(1, 5);
    v.push(n_env);
    let mut dom = [0i32; MAX_ENVELOPES];
    for d in dom.iter_mut() {
        *d = rng.range(0, 1);
    }
    let mut dom_noise = [0i32; MAX_ENVELOPES];
    for d in dom_noise.iter_mut() {
        *d = rng.range(0, 1);
    }
    v.extend_from_slice(&dom);
    v.extend_from_slice(&dom_noise);
    let n_noise = rng.range(0, 5);
    v.push(n_noise);
    for _ in 0..MAX_NUM_NOISE_VALUES {
        v.push(rng.range(0, 3));
    }
    let bal = coupling && balance != 0;
    let lav = match (amp_res, bal) {
        (SBR_AMP_RES_1_5, false) => CODE_BOOK_SCF_LAV10,
        (SBR_AMP_RES_1_5, true) => CODE_BOOK_SCF_LAV_BALANCE10,
        (_, false) => CODE_BOOK_SCF_LAV11,
        (_, true) => CODE_BOOK_SCF_LAV_BALANCE11,
    };
    for d in dom {
        for j in 0..MAX_FREQ_COEFFS {
            v.push(if j == 0 && d == FREQ { rng.range(0, 127) } else { rng.range(-lav, lav) });
        }
    }
    for _ in 0..MAX_ENVELOPES {
        v.push(rng.range(0, 48));
    }
    v.push(balance);
    let nlav = if bal { CODE_BOOK_SCF_LAV_BALANCE11 } else { CODE_BOOK_SCF_LAV11 };
    for _ in 0..MAX_FREQ_COEFFS {
        v.push(rng.range(-nlav, nlav));
    }
    v.push(rng.range(0, 1));
    v.push(rng.range(0, 48));
    for _ in 0..MAX_FREQ_COEFFS {
        v.push(rng.range(0, 1));
    }
    v.push(rng.range(0, 1));
    v
}

fn env_from(v: &[i32]) -> SbrEnvData {
    let mut e = SbrEnvData::default();
    let mut henv = SbrCodeEnvelope::default();
    let mut hnoise = SbrCodeEnvelope::default();
    let mut k = 0;
    let mut next = || {
        k += 1;
        v[k - 1]
    };
    fdk_sbr_enc_init_sbr_huffman_tables(&mut e, &mut henv, &mut hnoise, next());
    e.ld_grid = next() as u8;
    e.no_of_envelopes = next();
    for i in 0..MAX_ENVELOPES {
        e.domain_vec[i] = next();
    }
    for i in 0..MAX_ENVELOPES {
        e.domain_vec_noise[i] = next();
    }
    e.no_ofnoisebands = next();
    for i in 0..MAX_NUM_NOISE_VALUES {
        e.sbr_invf_mode_vec[i] = next();
    }
    for i in 0..MAX_ENVELOPES {
        for j in 0..MAX_FREQ_COEFFS {
            e.ienvelope[i][j] = next();
        }
    }
    for i in 0..MAX_ENVELOPES {
        e.no_scf_bands[i] = next();
    }
    e.balance = next();
    for i in 0..MAX_FREQ_COEFFS {
        e.sbr_noise_levels[i] = next() as i8;
    }
    e.add_harmonic_flag = next();
    e.no_harmonics = next();
    for i in 0..MAX_FREQ_COEFFS {
        e.add_harmonic[i] = next() as u8;
    }
    e.current_amp_res_ff = next();
    e
}

fn random_grid(rng: &mut Rng, ld: bool) -> SbrGrid {
    let mut g = SbrGrid {
        buffer_frame_start: rng.range(0, 4),
        number_time_slots: [8, 15, 16, 9][rng.range(0, 3) as usize],
        frame_class: if ld { rng.range(0, 4) } else { rng.range(0, 3) },
        bs_num_env: rng.range(1, 4),
        bs_abs_bord: rng.range(0, 20),
        n: rng.range(0, 3),
        p: rng.range(0, 4),
        bs_abs_bord_0: rng.range(0, 6),
        bs_abs_bord_1: rng.range(14, 22),
        bs_num_rel_0: rng.range(0, 2),
        bs_num_rel_1: rng.range(0, 2),
        ..Default::default()
    };
    for i in 0..MAX_NUM_REL {
        g.bs_rel_bord[i] = 2 * rng.range(1, 4);
        g.bs_rel_bord_0[i] = 2 * rng.range(1, 4);
        g.bs_rel_bord_1[i] = 2 * rng.range(1, 4);
    }
    for v in g.v_f.iter_mut() {
        *v = rng.range(0, 1);
    }
    for v in g.v_f_lr.iter_mut() {
        *v = rng.range(0, 1);
    }
    g
}

fn random_ps_out(rng: &mut Rng) -> PsOut {
    let mut o = PsOut {
        enable_ps_header: rng.range(0, 1),
        enable_iid: rng.range(0, 1),
        iid_mode: rng.range(0, 4),
        enable_icc: rng.range(0, 1),
        icc_mode: rng.range(0, 4),
        enable_ipd_opd: rng.range(0, 1),
        frame_class: rng.range(0, 1),
        n_envelopes: rng.range(0, 4),
        ..Default::default()
    };
    for e in 0..PS_MAX_ENVELOPES {
        o.frame_border[e] = rng.range(0, 31);
        o.delta_iid[e] = rng.range(0, 1);
        o.delta_icc[e] = rng.range(0, 1);
        for b in 0..PS_MAX_BANDS {
            o.iid[e][b] = rng.range(-7, 7);
            o.icc[e][b] = rng.range(0, 7);
            o.ipd[e][b] = rng.range(0, 7);
            o.opd[e][b] = rng.range(0, 7);
        }
    }
    o
}

#[test]
fn payloads() {
    let mut rng = Rng::new(0xb17_5b2);
    for case in 0..6000 {
        let elem = rng.range(1, 3);
        let flags = [0, SBR_SYNTAX_LOW_DELAY, SBR_SYNTAX_SCALABLE, SBR_SYNTAX_CRC, SBR_SYNTAX_CRC | SBR_SYNTAX_DRM_CRC]
            [rng.range(0, 4) as usize]
            | if rng.range(0, 7) == 0 { SBR_SYNTAX_CRC } else { 0 };
        let ld = flags & SBR_SYNTAX_LOW_DELAY != 0 || rng.range(0, 7) == 0;
        let coupling = elem != 1 && rng.range(0, 1) == 1;
        let amp_res = rng.range(0, 1);
        let mut hdr = [0i32; 17];
        for (i, v) in hdr.iter_mut().enumerate() {
            *v = match i {
                0 => amp_res,
                6 | 7 => rng.range(0, 1),
                15 => coupling as i32,
                _ => rng.range(0, 15),
            };
        }
        let header_active = rng.range(0, 1);
        let mut env_l = env_spec(&mut rng, amp_res, ld as i32, coupling, 0);
        let bal_r = if coupling { 1 } else { rng.range(0, 1) };
        let mut env_r = env_spec(&mut rng, amp_res, ld as i32, coupling, bal_r);
        let gl = random_grid(&mut rng, ld);
        let gr = random_grid(&mut rng, ld);
        let mut gd_l = Dump::default();
        grid(&mut gd_l, &gl);
        let mut gd_r = Dump::default();
        grid(&mut gd_r, &gr);
        let ps = if rng.range(0, 2) == 0 { Some(random_ps_out(&mut rng)) } else { None };
        let mut ps_flat = Dump::default();
        if let Some(o) = &ps {
            ps_out(&mut ps_flat, o);
        }

        // C
        let mut payload_c = vec![0u8; MAX_PAYLOAD_SIZE];
        let mut ret_c = [0i32; 3];
        let mut dump_c = vec![0i32; 256];
        let mut n = 0;
        unsafe {
            c::cref_bit_sbr_payload(
                elem,
                hdr.as_mut_ptr(),
                header_active,
                env_l.as_mut_ptr(),
                gd_l.0.as_mut_ptr(),
                if elem == 1 { core::ptr::null_mut() } else { env_r.as_mut_ptr() },
                gd_r.0.as_mut_ptr(),
                if ps.is_some() { ps_flat.0.as_mut_ptr() } else { core::ptr::null_mut() },
                flags,
                payload_c.as_mut_ptr(),
                ret_c.as_mut_ptr(),
                dump_c.as_mut_ptr(),
                256,
                &mut n,
            )
        };
        dump_c.truncate(n as usize);

        // port
        let h = SbrHeaderData {
            sbr_amp_res: hdr[0],
            sbr_start_frequency: hdr[1],
            sbr_stop_frequency: hdr[2],
            sbr_xover_band: hdr[3],
            sbr_noise_bands: hdr[4],
            sbr_data_extra: hdr[5],
            header_extra_1: hdr[6],
            header_extra_2: hdr[7],
            sbr_lc_stereo_mode: hdr[8],
            sbr_limiter_bands: hdr[9],
            sbr_limiter_gains: hdr[10],
            sbr_interpol_freq: hdr[11],
            sbr_smoothing_length: hdr[12],
            alter_scale: hdr[13],
            freq_scale: hdr[14],
            coupling: hdr[15],
            prev_coupling: hdr[16],
        };
        let b = SbrBitstreamData { header_active, ..Default::default() };
        let el = env_from(&env_l);
        let er = env_from(&env_r);
        let mut psh = ps.map(|o| {
            let mut p = ps_enc_create();
            p.ps_out[0] = o;
            p
        });
        let payload_mem = BitMem::new(MAX_PAYLOAD_SIZE);
        let mut cmon = CommonData::default();
        let mut crc = FdkCrcInfo::default();
        let ret_r;
        {
            fdk_init_bit_stream(&mut cmon.sbr_bitbuf, &payload_mem, MAX_PAYLOAD_SIZE as u32, 0, BS_WRITER);
            let crc_reg = fdk_sbr_enc_init_sbr_bitstream(
                &mut cmon,
                &payload_mem,
                0,
                MAX_PAYLOAD_SIZE as i32,
                &mut crc,
                flags,
            );
            let l = Some(EnvRef { data: &el, grid: &gl });
            let r = Some(EnvRef { data: &er, grid: &gr });
            let ps = psh.as_deref_mut().map(|p| &*p);
            let bits = match elem {
                1 => fdk_sbr_enc_write_env_single_channel_element(&h, ps, &b, l, &mut cmon, flags),
                2 => fdk_sbr_enc_write_env_channel_pair_element(&h, ps, &b, l, r, &mut cmon, flags),
                _ => fdk_sbr_enc_count_sbr_channel_pair_element(&h, ps, &b, l, r, &mut cmon, flags),
            };
            fdk_sbr_enc_assemble_sbr_bitstream(Some(&mut cmon), &mut crc, crc_reg, flags);
            ret_r = [bits, crc_reg, fdk_get_valid_bits(&mut cmon.sbr_bitbuf) as i32];
        }
        let payload_r = payload_mem.to_vec();
        let mut d = Dump::default();
        cmon_data(&mut d, &cmon, &payload_mem);
        assert_eq!(ret_c, ret_r, "case {case} elem {elem} flags {flags}");
        assert_dump_eq(&dump_c, &d.0, "common data");
        assert_eq!(payload_c, payload_r, "case {case} payload flags {flags} bits {ret_r:?}");
    }
}

/// The byte image of the members after `addHarmonic` (read by the missing harmonics detector).
#[test]
fn env_data_tail() {
    let mut rng = Rng::new(0x7a11);
    for _ in 0..200 {
        let amp_res = rng.range(0, 1);
        let (ld, coupling, balance) = (rng.range(0, 1), rng.range(0, 1) == 1, rng.range(0, 1));
        let mut spec = env_spec(&mut rng, amp_res, ld, coupling, balance);
        let mut tail_c = [0u8; 144];
        unsafe { c::cref_bit_sbr_env_tail(spec.as_mut_ptr(), tail_c.as_mut_ptr()) };
        assert_eq!(tail_c, env_from(&spec).add_harmonic_tail());
    }
}

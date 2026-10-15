// Differential tests of fdk_aac_rs_sacdec::sac_bitdec against the C++ reference: the parsers of
// the spatial specific configs (LD, LD with header, USAC MPS212, the default config), the frame
// parser and the decoding of the frame (mapIndexData, the smoothing data, the concealment) on
// random bitstreams and random parser states, and the static helpers.

use crate::sac_dec::*;
use crate::sac_dec_state::*;
use crate::sac_dec_util::*;
use fdk_aac_rs_cref::sacdec_sac_dec as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_sacdec::sac_bitdec::*;
use fdk_aac_rs_sacdec::sac_dec::*;
use fdk_aac_rs_sacdec::sac_dec_interface::*;

fn c_ssc(b: &[u8; SPATIAL_SPECIFIC_CONFIG_SIZE]) -> SpatialSpecificConfig {
    SpatialSpecificConfig::from_bytes(b)
}

fn random_ssc_bytes(rng: &mut Rng) -> [u8; SPATIAL_SPECIFIC_CONFIG_SIZE] {
    let mut b = [0u8; SPATIAL_SPECIFIC_CONFIG_SIZE];
    for v in b.iter_mut() {
        *v = rng.next_u32() as u8;
    }
    b
}

const AOTS: [i32; 6] = [AOT_ER_AAC_ELD, AOT_ER_AAC_LD, AOT_USAC, AOT_DRM_USAC, AOT_AAC_LC, AOT_NONE];

#[test]
fn sac_bitdec_parse_specific_config() {
    let mut rng = Rng::new(0xb1);
    for iter in 0..60000 {
        let mut data = random_bytes(&mut rng, 64);
        if iter % 2 == 0 {
            // tree config 7 at bits 12..16
            data[1] = (data[1] & 0xf0) | 7;
        }
        if iter % 4 == 0 {
            // no extensions / short ones
            for b in data[3..].iter_mut() {
                *b &= rng.next_u32() as u8;
            }
        }
        let size = 64;
        let valid = match iter % 5 {
            0 => rng.range(0, 64),
            _ => rng.range(0, 512),
        };
        let header_len = match iter % 4 {
            0 => 0,
            1 => rng.range(1, 8),
            2 => rng.range(0, 80),
            _ => rng.range(-2, 300),
        };
        let aot = AOTS[rng.next_u32() as usize % AOTS.len()];
        let init = random_ssc_bytes(&mut rng);

        let mut ssc = c_ssc(&init);
        let mut bs = Bs::new(&data, size, valid as u32);
        let r = spatial_dec_parse_specific_config(&mut bs.bs, &mut ssc, header_len, aot);
        let mut cb = init;
        let mut cbuf = data.clone();
        let mut cleft = 0;
        let cr = unsafe {
            c::cref_bitdec_parse_ssc(cbuf.as_mut_ptr(), size as i32, valid, header_len, aot, cb.as_mut_ptr(), &mut cleft)
        };
        assert_eq!(r, cr, "SpatialDecParseSpecificConfig ({iter})");
        assert_eq!(ssc.as_bytes()[..], cb[..], "config ({iter})");
        assert_eq!(bs.valid(), cleft, "bits ({iter})");

        // with the length header
        let upmix = [UPMIX_TYPE_NORMAL, UPMIX_TYPE_BYPASS][rng.next_u32() as usize % 2];
        let mut ssc = c_ssc(&init);
        let mut bs = Bs::new(&data, size, valid as u32);
        let r = spatial_dec_parse_specific_config_header(&mut bs.bs, &mut ssc, aot, upmix);
        let mut cb = init;
        let mut cbuf = data.clone();
        let cr = unsafe {
            c::cref_bitdec_parse_ssc_header(cbuf.as_mut_ptr(), size as i32, valid, aot, upmix, cb.as_mut_ptr(), &mut cleft)
        };
        assert_eq!(r, cr, "SpatialDecParseSpecificConfigHeader ({iter})");
        assert_eq!(ssc.as_bytes()[..], cb[..], "config with header ({iter})");
        assert_eq!(bs.valid(), cleft, "bits with header ({iter})");
    }
}

#[test]
fn sac_bitdec_parse_mps212_and_default_config() {
    let mut rng = Rng::new(0xb2);
    for iter in 0..60000 {
        let data = random_bytes(&mut rng, 16);
        let valid = rng.range(0, 128);
        let aot = AOTS[rng.next_u32() as usize % AOTS.len()];
        let sci = rng.range(-1, 4);
        let csfli = rng.range(0, 5);
        let fs = [44100, 48000, 96000, 0, -1][rng.next_u32() as usize % 5];
        let init = random_ssc_bytes(&mut rng);
        let mut ssc = c_ssc(&init);
        let mut bs = Bs::new(&data, 16, valid as u32);
        let r = spatial_dec_parse_mps212_config(&mut bs.bs, &mut ssc, fs, aot, sci, csfli);
        let mut cb = init;
        let mut cbuf = data.clone();
        let mut cleft = 0;
        let cr = unsafe {
            c::cref_bitdec_parse_mps212(cbuf.as_mut_ptr(), 16, valid, fs, aot, sci, csfli, cb.as_mut_ptr(), &mut cleft)
        };
        assert_eq!(r, cr, "SpatialDecParseMps212Config ({iter})");
        assert_eq!(ssc.as_bytes()[..], cb[..], "config ({iter})");
        assert_eq!(bs.valid(), cleft, "bits ({iter})");

        let mut ssc = c_ssc(&init);
        let n_ts = rng.range(1, 64);
        let ch = rng.range(1, 2);
        let blind = rng.range(0, 1);
        let r = fdk_spatial_dec_init_default_spatial_specific_config(&mut ssc, aot, ch, fs, n_ts, 0, blind);
        let mut cb = init;
        let cr = unsafe { c::cref_bitdec_default_ssc(cb.as_mut_ptr(), aot, fs, n_ts, 0, blind, ch) };
        assert_eq!(r, cr, "FDK_SpatialDecInitDefaultSpatialSpecificConfig ({iter})");
        assert_eq!(ssc.as_bytes()[..], cb[..], "default config ({iter})");

        // FDK_SpatialDecCompareSpatialSpecificConfigHeader: every byte
        let mut a = random_ssc_bytes(&mut rng);
        let b = if rng.next_u32() % 2 == 0 { a } else { random_ssc_bytes(&mut rng) };
        if rng.next_u32() % 4 == 0 {
            a = b;
            a[rng.next_u32() as usize % SPATIAL_SPECIFIC_CONFIG_SIZE] ^= 1 << (rng.next_u32() % 8);
        }
        assert_eq!(
            fdk_spatial_dec_compare_spatial_specific_config_header(&c_ssc(&a), &c_ssc(&b)),
            unsafe { c::cref_sacinst_compare_ssc(a.as_ptr(), b.as_ptr()) },
            "compare ({iter})"
        );
    }
}

#[test]
fn sac_bitdec_helpers() {
    use fdk_aac_rs_fdk::nlc_dec::{t_CLD, t_ICC, t_IPD};
    let mut rng = Rng::new(0xb3);
    for iter in 0..200000 {
        // getStrideMap, createMapping, mapFrequency over the band ranges of the library
        let stride_idx = rng.range(0, 3);
        let start = if iter % 8 == 0 { rng.range(0, 10) } else { 0 };
        let stop = rng.range(start + 1, 28);
        let mut a = [0i32; 29];
        let mut ca = [0i32; 29];
        let n = get_stride_map(stride_idx, start, stop, &mut a);
        let cn = unsafe { c::cref_bitdec_get_stride_map(stride_idx, start, stop, ca.as_mut_ptr()) };
        assert_eq!(n, cn, "getStrideMap ({iter})");
        assert_eq!(a[..=n as usize], ca[..=n as usize], "getStrideMap strides ({iter})");
        let stride = [1, 2, 5, 28][stride_idx as usize];
        let mut map = [0i32; 29];
        let mut cmap = [0i32; 29];
        create_mapping(&mut map, start, stop, stride);
        unsafe { c::cref_bitdec_create_mapping(cmap.as_mut_ptr(), start, stop, stride) };
        let ob = ((stop - start - 1) / stride + 1).max(1) as usize;
        assert_eq!(map[..=ob], cmap[..=ob], "createMapping ({iter})");
        let mut inp = [0i8; 28];
        for x in inp.iter_mut() {
            *x = rng.next_u32() as i8;
        }
        let mut out = [0i8; 28];
        for x in out.iter_mut() {
            *x = rng.next_u32() as i8;
        }
        let mut cout = out;
        map_frequency(&inp, &mut out, &map, n);
        unsafe { c::cref_bitdec_map_frequency(inp.as_ptr(), cout.as_mut_ptr(), cmap.as_mut_ptr(), n) };
        assert_eq!(out, cout, "mapFrequency ({iter})");

        // coarse2fine / fine2coarse
        let dt = [t_CLD, t_ICC, t_IPD][rng.next_u32() as usize % 3];
        let mut d = [0i8; 28];
        for x in d.iter_mut() {
            *x = if rng.next_u32() % 4 == 0 { rng.next_u32() as i8 } else { rng.range(-16, 16) as i8 };
        }
        let mut cd = d;
        let nb = stop - start;
        if iter % 2 == 0 {
            coarse2fine(&mut d, dt, start, nb);
            unsafe { c::cref_bitdec_coarse2fine(cd.as_mut_ptr(), dt, start, nb) };
        } else {
            fine2coarse(&mut d, dt, start, nb);
            unsafe { c::cref_bitdec_fine2coarse(cd.as_mut_ptr(), dt, start, nb) };
        }
        assert_eq!(d, cd, "coarse2fine/fine2coarse ({iter})");

        // deqIdx
        let v = if iter % 4 == 0 { rng.i32() } else { rng.range(-500, 500) };
        assert_eq!(deq_idx(v, dt), unsafe { c::cref_bitdec_deq_idx(v, dt) }, "deqIdx ({iter})");

        // factorFunct / factorCLD (not reachable from the library: decodeAndMapFrameOtt()
        // skips the CLD data for quantMode != 0)
        let db = if iter % 3 == 0 { rng.i32() } else { -((rng.next_u32() >> rng.range(1, 31)) as i32) };
        let qm = rng.range(-1, 3);
        assert_eq!(factor_funct(db, qm), unsafe { c::cref_bitdec_factor_funct(db, qm) }, "factorFunct ({iter})");
        let mut idx = rng.next_u32() as i8;
        let (mut d1, mut d2) = (rng.i32(), rng.i32());
        let mut io = [idx as i32, d1, d2];
        let mode = rng.range(0, 3) as i8;
        factor_cld(&mut idx, db, Some(&mut d1), Some(&mut d2), mode, qm);
        unsafe { c::cref_bitdec_factor_cld(io.as_mut_ptr(), db, mode as i32, qm) };
        assert_eq!([idx as i32, d1, d2], io, "factorCLD ({iter})");

        // nBitsParamSlot
        let ts = rng.range(1, 64);
        assert_eq!(n_bits_param_slot(ts), unsafe { c::cref_bitdec_n_bits_param_slot(ts) }, "nBitsParamSlot");
    }
}

/// Random parser state: the compact previous indices and the lossless states (as the parser
/// leaves them: valid quantization indices), loaded into both sides.
fn randomize_parser_state(p: &mut Pair, rng: &mut Rng) {
    let mut v = dump_self(&p.r.sd);
    let n = plain_len();
    let _ = n;
    // the previous compact indices (cmpOttCLDidxPrev ... cmpArbdmxGainIdxPrev) and the
    // decoded previous indices; their positions in the dump
    let mut s = (*p.r.sd).clone();
    for r in s.cmp_ott_cld_idx_prev.iter_mut().chain(s.ott_cld_idx_prev.iter_mut()) {
        for x in r.iter_mut() {
            *x = rng.range(-15, 15) as i8;
        }
    }
    for r in s.cmp_ott_icc_idx_prev.iter_mut().chain(s.ott_icc_idx_prev.iter_mut()) {
        for x in r.iter_mut() {
            *x = rng.range(0, 7) as i8;
        }
    }
    for r in s.cmp_ott_ipd_idx_prev.iter_mut().chain(s.ott_ipd_idx_prev.iter_mut()) {
        for x in r.iter_mut() {
            *x = rng.range(0, 15) as i8;
        }
    }
    for r in s.cmp_arbdmx_gain_idx_prev.iter_mut().chain(s.arbdmx_gain_idx_prev.iter_mut()) {
        for x in r.iter_mut() {
            *x = rng.range(-15, 15) as i8;
        }
    }
    for x in s.ott_icc_diffidx[0].iter_mut().flatten() {
        *x = rng.range(-2, 2) as i8;
    }
    s.conceal_info.conceal_state = rng.range(0, 5);
    s.conceal_info.cnt_state_frames = rng.range(0, 6) as u32;
    s.conceal_info.cnt_valid_frames = rng.range(0, 6) as u32;
    v.clear();
    v.extend(dump_self(&s));
    p.load(&v);
    let mut f = dump_frame(&p.r.frame, &p.r.ll);
    // lossless states (the last 2 * 3 * 6 + 1 values): 0 or 1
    let l = f.len();
    for x in f[l - 37..l - 1].iter_mut() {
        *x = rng.range(0, 1);
    }
    p.load_frame(&f);
}

#[test]
fn sac_bitdec_parse_and_decode_frame() {
    let mut rng = Rng::new(0xb4);
    let mut parsed = 0;
    for iter in 0..300 {
        let Some((ssc, qmf, frame)) = random_config(&mut rng) else { continue };
        let (mut p, r) = init_pair(&ssc, qmf, frame, INIT_ALL);
        if r != MPS_OK {
            continue;
        }
        for f in 0..30 {
            if f % 5 == 0 {
                randomize_parser_state(&mut p, &mut rng);
            }
            let mut data = random_bytes(&mut rng, 256);
            // shorter payloads parse more often
            if rng.next_u32() % 2 == 0 {
                for b in data.iter_mut() {
                    *b &= rng.next_u32() as u8 | 0x0f;
                }
            }
            let valid = rng.range(1, 2048);
            let indep = (rng.next_u32() % 3 == 0) as i32;
            let mut bs = Bs::new(&data, 256, valid as u32);
            let r = spatial_dec_parse_frame_data(&mut p.r.sd, &mut p.r.frame, &mut p.r.ll, &mut bs.bs, &p.r.ssc.clone(), UPMIXTYPE_NORMAL, indep);
            let mut cbuf = data.clone();
            let mut cleft = 0;
            let cr = unsafe { c::cref_sacinst_parse_frame(p.c.0, cbuf.as_mut_ptr(), 256, valid, UPMIXTYPE_NORMAL, indep, &mut cleft) };
            assert_eq!(r, cr, "SpatialDecParseFrameData ({iter}/{f})");
            assert_eq!(bs.valid(), cleft, "bits ({iter}/{f})");
            p.check(&format!("SpatialDecParseFrameData ({iter}/{f})"));
            if r == MPS_OK {
                parsed += 1;
            }
            if p.r.frame.num_parameter_sets > 0 {
                let r = spatial_dec_decode_frame(&mut p.r.sd, &p.r.ssc.clone(), &mut p.r.frame, &mut p.r.ll);
                let cr = unsafe { c::cref_sacinst_decode_frame(p.c.0) };
                assert_eq!(r, cr, "SpatialDecDecodeFrame ({iter}/{f})");
                p.check(&format!("SpatialDecDecodeFrame ({iter}/{f})"));
            }
        }
        // SpatialDecDecodeHeader on the configuration
        let r = spatial_dec_decode_header(&mut p.r.sd, &mut p.r.ssc);
        let cr = unsafe { c::cref_sacinst_decode_header(p.c.0) };
        assert_eq!(r, cr, "SpatialDecDecodeHeader ({iter})");
        p.check(&format!("SpatialDecDecodeHeader ({iter})"));
    }
    assert!(parsed > 500, "{parsed} frames parsed");
}

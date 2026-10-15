// Differential tests of fdk_aac_rs_sbrdec::env_dec against the C++ reference, and of
// calculateSbrEnvelope on the decoded data.
//
// Random channel elements are parsed (tests/sbrdec/env_extr.rs, compared there), then
// decodeSbrData runs on the parsed frames with random previous frame data (mono and stereo, all
// coupling modes, with and without frame errors); frames, previous frame data and header are
// compared. The decoded legacy SBR frames then go through calculateSbrEnvelope with random QMF
// data, scale factors and envelope calculator states carried over several frames.

use super::env_extr::{parse_element, ptr, random_flags, random_header};
use fdk_aac_rs_cref::sbrdec_env_calc::*;
use fdk_aac_rs_cref::sbrdec_env_extr::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::qmf::QmfScaleFactor;
use fdk_aac_rs_sbrdec::env_calc::*;
use fdk_aac_rs_sbrdec::env_dec::decode_sbr_data;
use fdk_aac_rs_sbrdec::env_extr::*;
use fdk_aac_rs_sbrdec::pvc_dec::PvcDynamicData;
use fdk_aac_rs_sbrdec::sbr_ram::pod_bytes;

#[test]
fn decode_and_calculate() {
    let mut rng = Rng::new(31);
    let mut decoded = 0;
    let mut calculated = 0;
    for it in 0..20000 {
        let (flags, overlap) = random_flags(&mut rng);
        let Some(mut h) = random_header(&mut rng, flags) else {
            continue;
        };
        let stereo = rng.next_u32() % 2 == 0;
        let (r, mut l, mut rt, prev) = parse_element(&mut rng, &mut h, flags, overlap, stereo, &format!("decode {it}"));
        // (the decoder only decodes successfully parsed frames: sbrDecoder_Parse restores the
        // previous frame data otherwise; partly parsed frames can have no defined result)
        if r == 0 {
            continue;
        }
        let trace = std::env::var_os("SBRDEC_TRACE").is_some();
        if trace {
            eprintln!("decode {it} r {r} stereo {stereo} flags {flags:#x}");
        }
        h.frame_error_flag = (rng.next_u32() % 8 == 0) as u8;
        h.sync_state = [SBR_ACTIVE, SBR_HEADER][rng.range(0, 1) as usize];
        let mut prev_r = super::env_extr::random_prev(&mut rng);
        let (mut hc, mut lc, mut rc, mut pc, mut prc) = (h, l, rt, prev, prev_r);
        let mut prev_l = prev;
        unsafe {
            cref_sbrdec_env_extr_decode(
                ptr(&mut hc),
                ptr(&mut lc),
                ptr(&mut pc),
                if stereo { ptr(&mut rc) } else { core::ptr::null_mut() },
                if stereo { ptr(&mut prc) } else { core::ptr::null_mut() },
            )
        };
        if stereo {
            decode_sbr_data(&mut h, &mut l, &mut prev_l, Some(&mut rt), Some(&mut prev_r));
        } else {
            decode_sbr_data(&mut h, &mut l, &mut prev_l, None, None);
        }
        let what = format!("decodeSbrData {it}");
        assert_eq!(pod_bytes(&h), pod_bytes(&hc), "{what}: header");
        assert_eq!(pod_bytes(&l), pod_bytes(&lc), "{what}: left");
        assert_eq!(pod_bytes(&prev_l), pod_bytes(&pc), "{what}: left previous");
        if stereo {
            assert_eq!(pod_bytes(&rt), pod_bytes(&rc), "{what}: right");
            assert_eq!(pod_bytes(&prev_r), pod_bytes(&prc), "{what}: right previous");
        }
        decoded += 1;

        // calculateSbrEnvelope on the decoded (legacy SBR) frame
        if h.bs_info.pvc_mode != 0 || r == 0 {
            continue;
        }
        let mut env = SbrCalculateEnvelope::ZERO;
        let mut hh = h;
        let rr = create_sbr_envelope_calc(&mut env, &mut hh, 1, flags);
        assert_eq!(rr, 0);
        for b in fdk_aac_rs_sbrdec::sbr_ram::pod_bytes_mut(&mut env).iter_mut().take(504) {
            if rng.next_u32() % 3 == 0 {
                *b = rng.next_u32() as u8;
            }
        }
        env.start_up = rng.range(0, 1);
        let mut env_c = env;
        let use_lp = flags & SBRDEC_SYNTAX_USAC == 0 && rng.next_u32() % 2 == 0;
        let cflags = flags & !(SBRDEC_USAC_ITES);
        // 128 slots: with random frames the envelope borders can exceed the slots of the
        // configuration
        let n_slots = 128;
        let class = rng.next_u32() % 4;
        let mut re: Vec<i32> = (0..n_slots * 64)
            .map(|_| match class {
                0 => 0,
                1 => rng.i32(),
                _ => (rng.next_u32() as i32) >> rng.range(4, 30),
            })
            .collect();
        let mut im: Vec<i32> = re.iter().map(|&x| x.rotate_left(7)).collect();
        let mut alias: Vec<i32> = (0..64).map(|_| (rng.next_u32() >> 1) as i32).collect();
        let (mut re2, mut im2, mut alias2) = (re.clone(), im.clone(), alias.clone());
        let mut scale = [rng.range(0, 20), rng.range(0, 20), rng.range(0, 20), rng.range(0, 20)];
        let mut sf = QmfScaleFactor { lb_scale: scale[0], ov_lb_scale: scale[1], hb_scale: scale[2], ov_hb_scale: scale[3] };
        let fe = (rng.next_u32() % 8 == 0) as i32;
        if trace {
            eprintln!(
                "calculate {it}: {:?} nsfb {:?} nnfb {} lo {} hi {} nlim {} lim {:?} nts {} ts {} interpol {} lp {use_lp}",
                l.frame_info,
                h.freq_band_data.n_sfb,
                h.freq_band_data.n_nfb,
                h.freq_band_data.low_subband,
                h.freq_band_data.high_subband,
                h.freq_band_data.no_limiter_bands,
                h.freq_band_data.limiter_band_table,
                h.number_time_slots,
                h.time_step,
                h.bs_data.interpol_freq
            );
        }
        let mut hc2 = h;
        let mut l2 = l;
        unsafe {
            cref_sbrdec_env_calc_calculate(
                scale.as_mut_ptr(),
                ptr(&mut env_c),
                ptr(&mut hc2),
                ptr(&mut l2),
                re.as_mut_ptr(),
                im.as_mut_ptr(),
                n_slots as i32,
                use_lp as i32,
                alias.as_mut_ptr(),
                cflags,
                fe,
            )
        };
        {
            let mut r: Vec<&mut [i32]> = re2.chunks_mut(64).collect();
            let mut i: Vec<&mut [i32]> = im2.chunks_mut(64).collect();
            let pvc = PvcDynamicData::default();
            calculate_sbr_envelope(&mut sf, &mut env, &h, &l, &pvc, &mut r, &mut i, use_lp, &mut alias2, cflags, fe);
        }
        let what = format!("calculateSbrEnvelope {it} lp {use_lp} flags {cflags:#x}");
        assert_eq!([sf.lb_scale, sf.ov_lb_scale, sf.hb_scale, sf.ov_hb_scale], scale, "{what}: scale");
        assert_eq!(pod_bytes(&env), pod_bytes(&env_c), "{what}: state");
        assert!(re2 == re, "{what}: real");
        if !use_lp {
            assert!(im2 == im, "{what}: imag");
        }
        calculated += 1;
    }
    assert!(decoded > 2000 && calculated > 500, "decoded {decoded}, calculated {calculated}");
}

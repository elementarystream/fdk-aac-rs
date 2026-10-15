// Differential tests of fdk_aac_rs_aacdec::channel against the C++ reference, on random states
// and bitstreams (channel_real.rs has the tests on real streams).
//
// CChannelElement_Read on random bitstreams for every audio object type and element type of the
// library, including the ER tools (VCB11, RVLC, HCR) and the USAC LPD channel stream and FAC data;
// CChannelElement_Decode on random valid element states (M/S, intensity stereo, PNS, TNS, complex
// prediction, single channels, RVLC) and on USAC elements read from random bitstreams (LPD
// channels); CChannel_CodebookTableInit.
//
// The HCR and RVLC state (`pComData->overlay`) is not part of the image of the context; it starts
// zeroed on both sides and is only changed by the compared calls.

use crate::block::{random_codebooks, random_tns};
use crate::block_ctx::*;
use crate::usacdec_acelp::rand_lsp;
use crate::usacdec_lpd_util::rand_lsf;
use fdk_aac_rs_aacdec::usacdec_lpc::e_lpc_f_lsp_a_conversion;
use fdk_aac_rs_aacdec::channel::*;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_cref::aacdec_channel as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::tools_rom::AC_EL_GA_CCE;

#[test]
fn codebook_table_init() {
    let mut rng = Rng::new(0xc4a_0001);
    let mut p = Pair::new();
    for it in 0..3000 {
        let usac = rng.next_u32() % 2 == 0;
        random_state(&mut p, &mut rng, usac, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        p.sync();
        unsafe { c::cref_channel_codebook_table_init(p.c.0, ch as i32) };
        c_channel_codebook_table_init(&mut p.r.ci[ch]);
        p.check(&format!("CChannel_CodebookTableInit {it}"));
    }
}

/// A random combination of the ER tools VCB11, RVLC and HCR.
fn er_tools(rng: &mut Rng) -> u32 {
    [0, AC_ER_VCB11, AC_ER_RVLC, AC_ER_HCR][(rng.next_u32() % 4) as usize]
        | [0, AC_ER_VCB11, AC_ER_RVLC, AC_ER_HCR][(rng.next_u32() % 4) as usize]
}

/// (aot, flags, epConfig) of the configurations the decoder supports.
fn random_aot(rng: &mut Rng, spf: u32) -> (i32, u32, i32) {
    let ld = spf == 512 || spf == 480;
    if ld {
        match rng.next_u32() % 2 {
            0 => (AOT_ER_AAC_LD, AC_ER | AC_LD | er_tools(rng), 0),
            _ => (AOT_ER_AAC_ELD, AC_ER | AC_ELD | er_tools(rng), 0),
        }
    } else if spf == 768 {
        (AOT_USAC, AC_USAC | if rng.next_u32() % 2 == 0 { AC_INDEP } else { 0 }, -1)
    } else {
        match rng.next_u32() % 6 {
            0 | 1 => (AOT_AAC_LC, 0, -1),
            2 | 3 => (AOT_ER_AAC_LC, AC_ER | er_tools(rng), 0),
            // USAC has frame lengths of 768 and 1024 only
            4 if spf == 1024 => (AOT_USAC, AC_USAC | if rng.next_u32() % 2 == 0 { AC_INDEP } else { 0 }, -1),
            4 => (AOT_AAC_LC, 0, -1),
            _ => (AOT_DRM_AAC, AC_DRM | AC_SCALABLE | AC_ER, 1),
        }
    }
}

#[test]
fn element_read() {
    let mut rng = Rng::new(0xc4a_0002);
    let mut p = Pair::new();
    // compared reads: all, with RVLC, with HCR, with an LPD channel, with FAC data of an FD channel
    let (mut compared, mut rvlc, mut hcr, mut lpd, mut fac) = (0, 0, 0, 0, 0);
    for it in 0..8000 {
        let spf = random_config(&mut p, &mut rng, true);
        let (aot, flags, ep) = random_aot(&mut rng, spf);
        let usac = aot == AOT_USAC;
        // the random state, now with the configuration that random_aot chose
        p.r.usac = usac;
        let n = p.r.dump().v.len();
        let v = random_image(&mut rng, n);
        p.r.load(&v);
        init_channels(&mut p, spf);
        let nch: u8 = 1 + (rng.next_u32() % 2) as u8;
        let mut el_flags = 0;
        let mut static_null = false;
        if usac {
            if nch == 2 && rng.next_u32() % 2 == 0 {
                el_flags |= AC_EL_USAC_CP_POSSIBLE;
            }
            if rng.next_u32() % 2 == 0 {
                el_flags |= AC_EL_USAC_NOISE;
            }
            if nch == 1 && rng.next_u32() % 4 == 0 {
                el_flags |= AC_EL_USAC_LFE | AC_EL_LFE;
            }
            for ch in 0..2 {
                // the decoder allocates hArCo for every USAC channel
                p.r.arco[ch].m_number_lines_prev = [0, spf as i16, spf as i16 / 8][(rng.next_u32() % 3) as usize];
                let a = p.r.arco[ch].clone();
                p.r.sci[ch].h_ar_co = Some(a);
                p.r.sci[ch].last_core_mode = [FD_LONG, FD_SHORT, LPD][(rng.next_u32() % 3) as usize];
                p.r.sci[ch].last_lpd_mode = [0, 1, 255][(rng.next_u32() % 3) as usize];
                p.r.ci[ch].data.usac.fac_data[0] = FacDataPtr::Null;
                p.r.ci[ch].render_mode = AACDEC_RENDER_IMDCT;
                p.r.sci[ch].lpc4_lsf = rand_lsf(&mut rng);
                p.r.sci[ch].lsf_adaptive_mean = rand_lsf(&mut rng);
            }
            p.r.sci[0].p_cpe_static_data = if el_flags & AC_EL_USAC_CP_POSSIBLE != 0 { Some(0) } else { None };
            p.r.cplx_present = el_flags & AC_EL_USAC_CP_POSSIBLE != 0;
        } else if aot == AOT_AAC_LC && nch == 1 && rng.next_u32() % 4 == 0 {
            // the CCE dummy channel of the decoder
            el_flags = AC_EL_GA_CCE;
            static_null = true;
        }
        let mut data = random_bytes(&mut rng, 1024);
        if usac && rng.next_u32() % 2 == 0 {
            // core_mode 1 (LPD) for the first channel
            data[0] |= 0x80;
        }
        p.set_bs(&data, 1024, 1024 * 8);
        p.sync();
        let ce = unsafe {
            c::cref_channel_element_read(p.c.0, aot, flags, el_flags, spf, nch as i32, ep, static_null as i32)
        };
        let re = {
            let sri = p.r.sri;
            let cpe_present = p.r.sci[0].p_cpe_static_data.is_some();
            let mut com_static = p.r.take_com_static();
            let RCtx { ci, sci, com, cpe, spec, bs, .. } = &mut p.r;
            let [c0, c1] = ci;
            let [s0, s1] = sci;
            let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
            let mut statics: [&mut CAacDecoderStaticChannelInfo; 2] = [&mut **s0, &mut **s1];
            let r = c_channel_element_read(
                bs,
                &mut chans[..nch as usize],
                if static_null { None } else { Some(&mut statics[..nch as usize]) },
                aot,
                &sri,
                flags,
                el_flags,
                spf,
                nch,
                ep as i8,
                None,
                spec,
                com,
                None,
                &mut com_static,
                if cpe_present { Some(&mut **cpe) } else { None },
            );
            p.r.put_com_static(com_static);
            r
        };
        compared += 1;
        rvlc += (flags & AC_ER_RVLC != 0) as u32;
        hcr += (flags & AC_ER_HCR != 0) as u32;
        if usac {
            let chans = &p.r.ci[..nch as usize];
            lpd += chans.iter().any(|c| c.render_mode == AACDEC_RENDER_LPD) as u32;
            fac += chans
                .iter()
                .any(|c| c.render_mode != AACDEC_RENDER_LPD && c.data.usac.fac_data[0] == FacDataPtr::FacData0)
                as u32;
        }
        let what = format!("CChannelElement_Read {it} aot {aot} flags {flags:#x} elFlags {el_flags:#x} nch {nch}");
        assert_eq!(re, ce, "{what}");
        p.check(&what);
        p.check_bs(&what);
    }
    eprintln!("CChannelElement_Read: {compared} compared, {rvlc} RVLC, {hcr} HCR, {lpd} LPD, {fac} FD with FAC data");
    assert!(rvlc > 500 && hcr > 500 && lpd > 200 && fac > 100, "coverage");
}

/// A random valid state of a decoded element.
fn element_state(p: &mut Pair, rng: &mut Rng) -> (u32, u32, i32, bool) {
    let usac = rng.next_u32() % 3 == 0;
    let spf = random_state(p, rng, usac, !usac, true);
    let el_channels = 1 + (rng.next_u32() % 2) as i32;
    let common = el_channels == 2 && rng.next_u32() % 4 != 0;
    if common {
        p.r.ci[1].ics_info = p.r.ci[0].ics_info;
        if usac && rng.next_u32() % 3 == 0 {
            p.r.ci[1].ics_info.max_sf_bands = rng.range(0, p.r.ci[1].ics_info.total_sf_bands as i32) as u8;
        }
    }
    let max = p.r.ci[0].ics_info.max_sf_bands.max(p.r.ci[1].ics_info.max_sf_bands);
    for ch in 0..2 {
        let pc = &mut p.r.ci[ch];
        pc.render_mode = AACDEC_RENDER_IMDCT;
        pc.p_dyn_data.raw_data_info.common_window = common as u8;
        pc.ics_info.max_sfb_ste = max;
        random_codebooks(rng, pc, false);
        random_tns(rng, pc);
        let nb = pc.ics_info.max_sf_bands as usize;
        let long = pc.ics_info.window_sequence != 2;
        for i in 0..128 {
            let transmitted = if long { i < nb } else { i % 16 < nb };
            pc.p_dyn_data.a_sfb_scale[i] = if transmitted { rng.range(-10, 30) as i16 } else { 0 };
            pc.p_dyn_data.a_scale_factor[i] = rng.range(-100, 155) as i16;
        }
        if usac {
            pc.data.usac.core_mode = 0;
            pc.p_dyn_data.specific_to.usac.tns_active = (rng.next_u32() % 2) as u8;
            pc.p_dyn_data.specific_to.usac.tns_on_lr = (rng.next_u32() % 2) as u8;
        } else {
            let pd = &mut pc.data.aac.pns_data;
            pd.pns_active = (rng.next_u32() % 3 == 0) as u8;
            for x in pd.pns_used.iter_mut() {
                *x = (rng.next_u32() % 5 == 0) as u8;
            }
        }
        let sh = 4 + rng.next_u32() % 12;
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = random_spec(rng) >> sh;
        }
        for x in p.r.sci[ch].concealment_info.spectral_coefficient.iter_mut() {
            *x = random_spec(rng) >> sh;
        }
        for x in p.r.sci[ch].concealment_info.spec_scale.iter_mut() {
            *x = rng.range(-5, 30) as i16;
        }
    }
    let j = &mut p.r.com.joint_stereo_data;
    j.ms_mask_present = (rng.next_u32() % if usac { 4 } else { 3 }) as u8;
    for x in j.ms_used.iter_mut() {
        *x = if rng.next_u32() % 3 == 0 { 0 } else { rng.next_u32() as u8 };
    }
    let mut el_flags = 0;
    let mut flags = 0;
    if usac {
        flags = AC_USAC;
        if el_channels == 2 && rng.next_u32() % 2 == 0 {
            el_flags |= AC_EL_USAC_CP_POSSIBLE;
            j.cplx_pred_flag = (rng.next_u32() % 2) as u8;
        } else {
            j.cplx_pred_flag = 0;
        }
        p.r.sci[0].p_cpe_static_data = if el_flags != 0 { Some(0) } else { None };
        p.r.cplx_present = el_flags != 0;
        let cp = &mut p.r.cplx;
        cp.pred_dir = (rng.next_u32() % 2) as u8;
        cp.complex_coef = (rng.next_u32() % 2) as u8;
        cp.use_prev_frame = if cp.complex_coef != 0 { (rng.next_u32() % 2) as u8 } else { 0 };
        for g in 0..8 {
            for b in 0..64 {
                cp.alpha_q_re[g][b] = rng.range(-30, 30) as i16;
                cp.alpha_q_im[g][b] = rng.range(-30, 30) as i16;
            }
        }
        let jp = &mut p.r.cpe.joint_stereo_persistent_data;
        jp.win_seq_prev = (rng.next_u32() % 4) as u8;
        jp.win_shape_prev = (rng.next_u32() % 2) as u8;
        jp.clear_spectral_coeffs = (rng.next_u32() % 4 == 0) as u8;
        p.r.ci[1].ics_info.window_shape = p.r.ci[0].ics_info.window_shape;
    } else {
        j.cplx_pred_flag = 0;
        if rng.next_u32() % 3 == 0 {
            // CRvlc_ElementCheck()
            flags = AC_ER | AC_ER_RVLC;
            for ch in 0..2 {
                let a = &mut p.r.ci[ch].p_dyn_data.specific_to.aac;
                a.rvlc_current_scale_factor_ok = (rng.next_u32() % 3 != 0) as i8;
                a.rvlc_intensity_used = (rng.next_u32() % 2) as i8;
            }
        }
    }
    (spf, flags, el_channels, usac)
}

#[test]
fn element_decode() {
    let mut rng = Rng::new(0xc4a_0003);
    let mut p = Pair::new();
    for it in 0..4000 {
        let (_spf, flags, el_channels, usac) = element_state(&mut p, &mut rng);
        let el_flags = if p.r.sci[0].p_cpe_static_data.is_some() { AC_EL_USAC_CP_POSSIBLE } else { 0 };
        p.sync();
        unsafe { c::cref_channel_element_decode(p.c.0, flags, el_flags, el_channels, usac as i32) };
        {
            let sri = p.r.sri;
            let cpe_present = p.r.sci[0].p_cpe_static_data.is_some();
            let mut com_static = p.r.take_com_static();
            let RCtx { ci, sci, com, cpe, spec, scratch, .. } = &mut p.r;
            let [c0, c1] = ci;
            let [s0, s1] = sci;
            let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
            let mut statics: [&mut CAacDecoderStaticChannelInfo; 2] = [&mut **s0, &mut **s1];
            let mut seed = statics[0].pns_current_seed;
            c_channel_element_decode(
                &mut chans[..el_channels as usize],
                &mut statics[..el_channels as usize],
                &sri,
                flags,
                el_flags,
                el_channels,
                spec,
                com,
                None,
                &mut com_static,
                if cpe_present { Some(&mut **cpe) } else { None },
                scratch,
                &mut seed,
            );
            statics[0].pns_current_seed = seed;
            p.r.put_com_static(com_static);
        }
        p.check(&format!("CChannelElement_Decode {it} usac {usac} flags {flags:#x} el_channels {el_channels}"));
    }
}

/// USAC elements with an LPD channel (`core_mode` 1) read from random bitstreams and, if the read
/// succeeded, decoded: CLpdChannelStream_Read() and CLpdChannelStream_Decode() inside
/// CChannelElement_Read() and CChannelElement_Decode(), on the states that the read produced.
#[test]
fn element_read_decode_lpd() {
    let mut rng = Rng::new(0xc4a_0004);
    let mut p = Pair::new();
    let (mut reads, mut read_errors, mut decodes, mut lpd_decoded) = (0, 0, 0, 0);
    for it in 0..4000 {
        let spf = loop {
            let spf = random_config(&mut p, &mut rng, false);
            if spf == 1024 || spf == 768 {
                break spf;
            }
        };
        p.r.usac = true;
        let n = p.r.dump().v.len();
        let v = random_image(&mut rng, n);
        p.r.load(&v);
        init_channels(&mut p, spf);
        let nch: u8 = 1 + (rng.next_u32() % 2) as u8;
        let mut el_flags = 0;
        if nch == 2 && rng.next_u32() % 2 == 0 {
            el_flags |= AC_EL_USAC_CP_POSSIBLE;
        }
        if rng.next_u32() % 2 == 0 {
            el_flags |= AC_EL_USAC_NOISE;
        }
        for ch in 0..2 {
            p.r.arco[ch].m_number_lines_prev = [0, spf as i16, spf as i16 / 8][(rng.next_u32() % 3) as usize];
            let a = p.r.arco[ch].clone();
            p.r.sci[ch].h_ar_co = Some(a);
            p.r.sci[ch].last_core_mode = [FD_LONG, FD_SHORT, LPD][(rng.next_u32() % 3) as usize];
            p.r.sci[ch].last_lpd_mode = [0, 1, 2, 3, 255][(rng.next_u32() % 5) as usize];
            p.r.sci[ch].last_lpc_lost = (rng.next_u32() % 4 == 0) as u8;
            p.r.sci[ch].lpc4_lsf = rand_lsf(&mut rng);
            p.r.sci[ch].lsf_adaptive_mean = rand_lsf(&mut rng);
            p.r.sci[ch].concealment_info.prev_frame_ok = [(rng.next_u32() % 4 != 0) as i32, (rng.next_u32() % 4 != 0) as i32];
            p.r.ci[ch].render_mode = AACDEC_RENDER_IMDCT;
            // LP coefficients of earlier frames (read without conversion when the last LPD mode
            // was TCX): as E_LPC_f_lsp_a_conversion() gives them, not with random exponents
            let RCtx { ci, sci, .. } = &mut p.r;
            let u = &mut ci[ch].data.usac;
            for (a, e) in u.lp_coeff.iter_mut().zip(u.lp_coeff_exp.iter_mut()) {
                e_lpc_f_lsp_a_conversion(&rand_lsp(&mut rng), a, e);
            }
            let s = &mut sci[ch];
            for (a, e) in s.lp_coeff_old.iter_mut().zip(s.lp_coeff_old_exp.iter_mut()) {
                e_lpc_f_lsp_a_conversion(&rand_lsp(&mut rng), a, e);
            }
        }
        p.r.sci[0].p_cpe_static_data = if el_flags & AC_EL_USAC_CP_POSSIBLE != 0 { Some(0) } else { None };
        p.r.cplx_present = el_flags & AC_EL_USAC_CP_POSSIBLE != 0;
        let flags = AC_USAC | if rng.next_u32() % 2 == 0 { AC_INDEP } else { 0 };
        let mut data = random_bytes(&mut rng, 2048);
        data[0] |= 0x80;
        if rng.next_u32() % 2 == 0 {
            data[0] |= 0x40;
        }
        p.set_bs(&data, 2048, 2048 * 8);
        p.sync();
        let what = format!("{it} spf {spf} flags {flags:#x} elFlags {el_flags:#x} nch {nch}");
        let ce = unsafe { c::cref_channel_element_read(p.c.0, AOT_USAC, flags, el_flags, spf, nch as i32, -1, 0) };
        let re = {
            let sri = p.r.sri;
            let cpe_present = p.r.sci[0].p_cpe_static_data.is_some();
            let mut com_static = p.r.take_com_static();
            let RCtx { ci, sci, com, cpe, spec, bs, .. } = &mut p.r;
            let [c0, c1] = ci;
            let [s0, s1] = sci;
            let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
            let mut statics: [&mut CAacDecoderStaticChannelInfo; 2] = [&mut **s0, &mut **s1];
            let r = c_channel_element_read(
                bs,
                &mut chans[..nch as usize],
                Some(&mut statics[..nch as usize]),
                AOT_USAC,
                &sri,
                flags,
                el_flags,
                spf,
                nch,
                -1,
                None,
                spec,
                com,
                None,
                &mut com_static,
                if cpe_present { Some(&mut **cpe) } else { None },
            );
            p.r.put_com_static(com_static);
            r
        };
        reads += 1;
        assert_eq!(re, ce, "CChannelElement_Read {what}");
        p.check(&format!("CChannelElement_Read {what}"));
        p.check_bs(&format!("CChannelElement_Read {what}"));
        if re != AAC_DEC_OK {
            read_errors += 1;
            continue;
        }
        let lpd = p.r.ci[..nch as usize].iter().filter(|c| c.render_mode == AACDEC_RENDER_LPD).count();
        unsafe { c::cref_channel_element_decode(p.c.0, flags, el_flags, nch as i32, 1) };
        {
            let sri = p.r.sri;
            let cpe_present = p.r.sci[0].p_cpe_static_data.is_some();
            let mut com_static = p.r.take_com_static();
            let RCtx { ci, sci, com, cpe, spec, scratch, .. } = &mut p.r;
            let [c0, c1] = ci;
            let [s0, s1] = sci;
            let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
            let mut statics: [&mut CAacDecoderStaticChannelInfo; 2] = [&mut **s0, &mut **s1];
            let mut seed = statics[0].pns_current_seed;
            c_channel_element_decode(
                &mut chans[..nch as usize],
                &mut statics[..nch as usize],
                &sri,
                flags,
                el_flags,
                nch as i32,
                spec,
                com,
                None,
                &mut com_static,
                if cpe_present { Some(&mut **cpe) } else { None },
                scratch,
                &mut seed,
            );
            statics[0].pns_current_seed = seed;
            p.r.put_com_static(com_static);
        }
        decodes += 1;
        lpd_decoded += lpd;
        p.check(&format!("CChannelElement_Decode {what}"));
    }
    eprintln!("LPD elements: {reads} reads ({read_errors} errors), {decodes} decodes with {lpd_decoded} LPD channels");
    assert!(lpd_decoded > 200, "coverage");
}

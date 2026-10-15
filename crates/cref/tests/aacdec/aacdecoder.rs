// Differential tests of fdk_aac_rs_aacdec::aacdecoder against the C++ reference: the helpers that
// can be tested in isolation. The decoder instance itself (CAacDecoder_Init, DecodeFrame, the
// extension payloads, the callbacks) is tested through the API in aacdecoder_lib.rs and
// aacdecoder_lib_stub.rs.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_aacdec::aacdecoder::*;
use fdk_aac_rs_aacdec::aacdecoder_lib::CStreamInfo;
use fdk_aac_rs_aacdec::channelinfo::CAacDecoderChannelInfo;
use fdk_aac_rs_cref::aacdec_aacdecoder as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_READER};
use fdk_aac_rs_tpdec::tpdec_lib::{transport_dec_open, CProgramConfig, CPROGRAMCONFIG_SIZE, TP_FLAG_MPEG4};

fn bs_over(buf: &mut [u8]) -> FdkBitStream {
    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(buf), buf.len() as u32, (buf.len() * 8) as u32, BS_READER);
    bs
}

#[test]
fn get_el_channels() {
    for t in -2..25 {
        for sci in 0..4 {
            assert_eq!(caac_decoder_get_el_channels(t, sci as u8), unsafe { c::cref_dec_get_el_channels(t, sci) });
        }
    }
}

#[test]
fn anc_data_parse_and_get() {
    let mut rng = Rng::new(11);
    for k in 0..5000 {
        let len = 256usize; // a power of two
        let buf: Vec<u8> = (0..len).map(|_| rng.next_u32() as u8).collect();
        let n = (rng.next_u32() % 10) as usize;
        let counts: Vec<i32> = (0..n).map(|_| (rng.next_u32() % 40) as i32 - 2).collect();
        let size = [0, 5, 20, 100, 300, -1][(rng.next_u32() % 6) as usize];
        let null = (k % 9 == 0) as i32;

        let mut anc_r = vec![0u8; 512];
        let mut offsets = [0i32; 8];
        let mut nr = 0;
        let mut rets = [0i32; 10];
        let mut bits = 0;
        let mut b = buf.clone();
        let rr = unsafe {
            c::cref_dec_anc(null, size, b.as_mut_ptr(), len as i32, counts.as_ptr() as *mut i32, n as i32, anc_r.as_mut_ptr(), offsets.as_mut_ptr(), &mut nr, rets.as_mut_ptr(), &mut bits)
        };

        let mut a = CAncData::default();
        let buffer = if null != 0 { AncBuffer::None } else { AncBuffer::Owned(vec![0u8; 512].into_boxed_slice()) };
        let rp = caac_decoder_anc_data_init(&mut a, buffer, size);
        assert_eq!(rr, rp);
        let mut pb = buf.clone();
        let mut bs = bs_over(&mut pb);
        for i in 0..n {
            let r = caac_decoder_anc_data_parse(&mut a, &mut bs, counts[i]);
            assert_eq!(rets[i], r, "parse {i}");
        }
        assert_eq!(offsets, a.offset);
        assert_eq!(nr, a.nr_elements);
        assert_eq!(bits, fdk_get_valid_bits(&mut bs) as i32);
        if let AncBuffer::Owned(o) = &a.buffer {
            assert_eq!(&anc_r[..], &o[..]);
        }

        let index = (rng.next_u32() % 10) as i32 - 1;
        let (mut off, mut sz) = (0, 0);
        let mut b = buf.clone();
        let mut anc2 = vec![0u8; 512];
        let r2 = unsafe {
            c::cref_dec_anc_get(b.as_mut_ptr(), len as i32, counts.as_ptr() as *mut i32, n as i32, anc2.as_mut_ptr(), size.max(0), index, &mut off, &mut sz)
        };
        let mut a2 = CAncData::default();
        caac_decoder_anc_data_init(&mut a2, AncBuffer::Owned(vec![0u8; 512].into_boxed_slice()), size.max(0));
        let mut pb = buf.clone();
        let mut bs = bs_over(&mut pb);
        for &cnt in &counts {
            caac_decoder_anc_data_parse(&mut a2, &mut bs, cnt);
        }
        let mut p = None;
        let mut s = 0;
        let r = caac_decoder_anc_data_get(&a2, index, &mut p, &mut s);
        assert_eq!((r2, off, sz), (r, p.map_or(-1, |x| x as i32), s));
    }
}

#[test]
fn cross_fade() {
    let mut rng = Rng::new(12);
    for k in 0..2000 {
        let nch = 1 + (rng.next_u32() % 8) as i32;
        let fs = [128, 256, 768, 1024][(rng.next_u32() % 4) as usize];
        let inter = (rng.next_u32() % 2) as i32;
        let td: Vec<i32> = (0..(nch * fs) as usize).map(|_| rng.i32()).collect();
        let flush: Vec<i32> = (0..8 * 128).map(|_| rng.i32()).collect();
        let apply = (k % 2) as i32;
        let mut tr = td.clone();
        let mut fr = flush.clone();
        unsafe { c::cref_dec_cross_fade(apply, tr.as_mut_ptr(), fr.as_mut_ptr(), nch, fs, inter) };
        let mut tp = td.clone();
        let mut fp: Vec<Option<Vec<i32>>> = (0..16).map(|i| if i < 8 { Some(flush[i * 128..(i + 1) * 128].to_vec()) } else { None }).collect();
        if apply != 0 {
            caac_decoder_apply_cross_fade(&mut tp, &fp, nch, fs, inter);
        } else {
            caac_decoder_prepare_cross_fade(&tp, &mut fp, nch, fs, inter);
        }
        assert_eq!(tr, tp);
        let fpf: Vec<i32> = fp.iter().take(8).flat_map(|f| f.clone().unwrap()).collect();
        assert_eq!(fr, fpf);
    }
}

fn pce_bytes(p: &CProgramConfig) -> Vec<u8> {
    p.as_bytes().to_vec()
}

fn pce_from(b: &[u8]) -> CProgramConfig {
    let mut p = CProgramConfig::zeroed();
    // SAFETY: CProgramConfig is #[repr(C)] with u8 fields only (no padding, any value valid).
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), &mut p as *mut CProgramConfig as *mut u8, CPROGRAMCONFIG_SIZE) };
    p
}

/// A random PCE bitstream (element_instance_tag ... comment), with a plausible structure.
fn random_pce(rng: &mut Rng) -> Vec<u8> {
    let mut bits: Vec<u8> = Vec::new();
    let mut put = |v: u32, n: u32| {
        for i in (0..n).rev() {
            bits.push(((v >> i) & 1) as u8);
        }
    };
    put(rng.next_u32() & 15, 4); // tag
    put(if rng.next_u32() % 4 == 0 { rng.next_u32() & 3 } else { 1 }, 2); // profile
    put(3 + rng.next_u32() % 9, 4); // sf index
    let nf = rng.next_u32() % 4;
    let ns = rng.next_u32() % 2;
    let nb = rng.next_u32() % 3;
    let nl = rng.next_u32() % 2;
    let na = rng.next_u32() % 2;
    let nc = rng.next_u32() % 2;
    put(nf, 4);
    put(ns, 4);
    put(nb, 4);
    put(nl, 2);
    put(na, 3);
    put(nc, 4);
    for _ in 0..3 {
        let pres = rng.next_u32() % 2;
        put(pres, 1);
        if pres != 0 {
            put(rng.next_u32(), 4 - (rng.next_u32() % 2));
        }
    }
    for _ in 0..nf + ns + nb {
        put(rng.next_u32() % 2, 1);
        put(rng.next_u32() & 15, 4);
    }
    for _ in 0..nl + na {
        put(rng.next_u32() & 15, 4);
    }
    for _ in 0..nc {
        put(rng.next_u32(), 5);
    }
    let mut out = vec![0u8; 64];
    for (i, b) in bits.iter().enumerate() {
        out[i / 8] |= b << (7 - i % 8);
    }
    for b in out[bits.len().div_ceil(8)..].iter_mut() {
        *b = rng.next_u32() as u8 & if rng.next_u32() % 2 == 0 { 0 } else { 0xff };
    }
    out
}

#[test]
fn program_config_element_read() {
    let mut rng = Rng::new(13);
    let mut tp = transport_dec_open(TT_MP4_RAW, TP_FLAG_MPEG4, 1).unwrap();
    for k in 0..20000 {
        let buf = if k % 5 == 0 { (0..64).map(|_| rng.next_u32() as u8).collect() } else { random_pce(&mut rng) };
        let start = if k % 3 == 0 {
            let mut p = CProgramConfig::zeroed();
            fdk_aac_rs_tpdec::tpdec_lib::cprogram_config_init(&mut p);
            p
        } else {
            // A valid PCE to compare with (from another random PCE).
            let mut b2 = random_pce(&mut rng);
            let mut p = CProgramConfig::zeroed();
            let mut bs = bs_over(&mut b2);
            fdk_aac_rs_tpdec::tpdec_lib::cprogram_config_read(&mut p, &mut bs, 0);
            p
        };
        let cc = (rng.next_u32() % 9) as i32;
        let mut pr = pce_bytes(&start);
        let mut b = buf.clone();
        let mut bits = 0;
        let rr = unsafe { c::cref_dec_pce_read(b.as_mut_ptr(), 64, pr.as_mut_ptr(), cc, 512, &mut bits) };
        let mut pp = start;
        let mut b = buf.clone();
        let mut bs = bs_over(&mut b);
        let rp = cprogram_config_element_read(&mut bs, Some(&mut tp), &mut pp, cc as u32, 512);
        assert_eq!(rr, rp, "pce read {k}");
        assert_eq!(bits, fdk_get_valid_bits(&mut bs) as i32);
        assert_eq!(pr, pce_bytes(&pp), "pce {k}");

        // validateElementInstanceTags with the PCE read and random tags and element lists.
        let pce = pce_from(&pr);
        let tags: [u8; 8] = core::array::from_fn(|_| (rng.next_u32() % 4) as u8);
        let nch = (rng.next_u32() % 9) as i32;
        let nel = (rng.next_u32() % 8) as i32;
        let els: Vec<i32> = (0..nel).map(|_| [ID_SCE, ID_CPE, ID_LFE, ID_END, ID_DSE][(rng.next_u32() % 5) as usize]).collect();
        let mut prc = pr.clone();
        let mut tg = tags;
        let vr = unsafe { c::cref_dec_validate_tags(prc.as_mut_ptr(), tg.as_mut_ptr(), nch, els.as_ptr() as *mut i32, nel) };
        let infos: [Option<Box<CAacDecoderChannelInfo>>; 8] = core::array::from_fn(|i| {
            let mut ci = Box::<CAacDecoderChannelInfo>::default();
            ci.element_instance_tag = tags[i];
            Some(ci)
        });
        let vp = validate_element_instance_tags(&pce, &infos, nch, &els, nel);
        assert_eq!(vr, vp, "validate {k}");
    }
}

#[test]
fn init_usac_sbr_qmf_configuration() {
    // CAacDecoder_Init of USAC configurations with SBR: the QMF domain request, including the
    // function-local table map_sbrRatio_2_nAnaBands of the C code (not reachable by
    // scripts/dump_table.py), for every sbrRatioIndex.
    for sri in 1..=3 {
        for (el, sci) in [(ID_USAC_SCE, 0), (ID_USAC_CPE, 0), (ID_USAC_CPE, 1), (ID_USAC_CPE, 3)] {
            let mut out = [0i32; 13];
            unsafe { c::cref_dec_init_usac_sbr(sri, el, sci, out.as_mut_ptr()) };

            let mut d = caac_decoder_open(TT_MP4_RAW).unwrap();
            fdk_aac_rs_pcmutils::pcmdmx_lib::pcm_dmx_open(Some(&mut d.h_pcm_utils));
            fdk_aac_rs_drc::drc_dec_lib::fdk_drc_dec_open(
                &mut d.h_uni_drc_decoder,
                fdk_aac_rs_drc::drc_dec_lib::DRC_DEC_ALL,
            );
            let mut asc = fdk_aac_rs_tpdec::tpdec_lib::CSAudioSpecificConfig::zeroed();
            asc.m_aot = AOT_USAC;
            asc.m_sampling_frequency = 48000;
            asc.m_sampling_frequency_index = 3;
            asc.m_samples_per_frame = 1024;
            asc.m_channel_configuration = if el == ID_USAC_CPE { 2 } else { 1 };
            asc.m_ep_config = -1;
            asc.m_sbr_present_flag = 1;
            asc.m_extension_sampling_frequency = 96000;
            asc.m_sc.m_usac_config.m_sbr_ratio_index = sri as u8;
            asc.m_sc.m_usac_config.m_usac_num_elements = 1;
            asc.m_sc.m_usac_config.m_n_usac_channels = if el == ID_USAC_CPE { 2 } else { 1 };
            asc.m_sc.m_usac_config.element[0].usac_element_type = el;
            asc.m_sc.m_usac_config.element[0].m_stereo_config_index = sci as u8;
            let mut changed = 0u8;
            let mut err = caac_decoder_init(&mut d, &asc, AC_CM_DET_CFG_CHANGE, &mut changed);
            err |= caac_decoder_init(&mut d, &asc, AC_CM_ALLOC_MEM, &mut changed) << 16;
            let g = &d.qmf_domain.global_conf;
            let p = [
                err,
                changed as i32,
                g.n_input_channels_requested as i32,
                g.n_output_channels_requested as i32,
                g.n_bands_analysis_requested as i32,
                g.n_bands_synthesis_requested as i32,
                g.n_qmf_time_slots_requested as i32,
                g.n_qmf_ov_time_slots_requested as i32,
                g.n_qmf_proc_bands_requested as i32,
                g.n_qmf_proc_channels_requested as i32,
                g.park_channel_requested as i32,
                g.qmf_domain_explicit_config as i32,
                d.aac_channels,
            ];
            assert_eq!(out, p, "sbrRatioIndex {sri}, element {el}, stereoConfigIndex {sci}");
            fdk_aac_rs_drc::drc_dec_lib::fdk_drc_dec_close(Some(&mut d.h_uni_drc_decoder));
            caac_decoder_close(d);
        }
    }
}

#[test]
fn stream_info_init() {
    let mut o = [0i32; 32];
    let n = unsafe { c::cref_dec_stream_info_init(o.as_mut_ptr()) } as usize;
    let mut si = CStreamInfo::default();
    cstream_info_init(&mut si);
    let p = [
        si.sample_rate,
        si.frame_size,
        si.num_channels,
        si.aac_sample_rate,
        si.profile,
        si.aot,
        si.channel_config,
        si.bit_rate,
        si.aac_samples_per_frame,
        si.aac_num_channels,
        si.ext_aot,
        si.ext_sampling_rate,
        si.output_delay as i32,
        si.flags as i32,
        si.ep_config as i32,
        si.drc_prog_ref_lev as i32,
        si.drc_pres_mode as i32,
        si.output_loudness,
    ];
    assert_eq!(&o[..n], &p[..]);
}

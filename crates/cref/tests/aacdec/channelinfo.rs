// Differential tests of fdk_aac_rs_aacdec::channelinfo against the C++ reference.
//
// getSamplingRateInfo, IcsRead and IcsReadMaxSfb on random and edge-case inputs, the inline
// functions of channelinfo.h, and layout-independent checks of the types: the constants, the
// error codes and the number of elements of every array field (compared with the C structures).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_aacdec::aac_rom::SFB_OFFSET_TABLES;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_cref::aacdec_aac_rom as crom;
use fdk_aac_rs_cref::aacdec_channelinfo as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_READER};
use std::collections::HashMap;

/// The Rust slice of a library sfb table pointer: the library's `sfbOffsetTables` and the
/// port's `SFB_OFFSET_TABLES` point to the same tables (checked by `aac_rom::sfb_offset_tables`).
fn sfb_map() -> HashMap<usize, &'static [i16]> {
    let mut m = HashMap::new();
    for i in 0..5 {
        for j in 0..16 {
            let (mut ln, mut sn) = (std::ptr::null(), std::ptr::null());
            let (mut ll, mut ls) = (std::ptr::null(), std::ptr::null());
            let (mut nl, mut ns) = (0, 0);
            unsafe {
                crom::cref_rom_sfb_info(i, j, &mut ln, &mut sn, &mut ll, &mut ls, &mut nl, &mut ns)
            };
            let r = &SFB_OFFSET_TABLES[i as usize][j as usize];
            m.insert(ll as usize, r.sfb_offset_long);
            m.insert(ls as usize, r.sfb_offset_short);
        }
    }
    m
}

fn same_table(map: &HashMap<usize, &'static [i16]>, c: *const i16, r: &'static [i16], what: &str) {
    if c.is_null() {
        assert!(r.is_empty(), "{what}: NULL");
    } else {
        let e = map.get(&(c as usize)).unwrap_or_else(|| panic!("{what}: unknown C table"));
        assert_eq!(e.as_ptr(), r.as_ptr(), "{what}: table");
        assert_eq!(e.len(), r.len(), "{what}: table length");
    }
}

fn ics_to(p: &CIcsInfo) -> [i32; 16] {
    let mut v = [0; 16];
    for i in 0..8 {
        v[i] = p.window_group_length[i] as i32;
    }
    v[8] = p.window_groups as i32;
    v[9] = p.valid as i32;
    v[10] = p.window_shape as i32;
    v[11] = p.window_sequence;
    v[12] = p.max_sf_bands as i32;
    v[13] = p.max_sfb_ste as i32;
    v[14] = p.scale_factor_grouping as i32;
    v[15] = p.total_sf_bands as i32;
    v
}

fn ics_from(v: &[i32; 16]) -> CIcsInfo {
    let mut p = CIcsInfo::default();
    for i in 0..8 {
        p.window_group_length[i] = v[i] as u8;
    }
    p.window_groups = v[8] as u8;
    p.valid = v[9] as u8;
    p.window_shape = v[10] as u8;
    p.window_sequence = v[11];
    p.max_sf_bands = v[12] as u8;
    p.max_sfb_ste = v[13] as u8;
    p.scale_factor_grouping = v[14] as u8;
    p.total_sf_bands = v[15] as u8;
    p
}

const SPF: [u32; 10] = [1024, 960, 768, 512, 480, 0, 256, 120, 2048, 1023];
const RATES: [u32; 16] =
    [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350, 0, 6000, 192000];

fn random_spf(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 8 {
        0 => rng.next_u32(),
        _ => SPF[(rng.next_u32() % SPF.len() as u32) as usize],
    }
}

fn random_sri(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 8 {
        0 => rng.next_u32(),
        1 => 15 + rng.next_u32() % 3,
        _ => rng.next_u32() % 16,
    }
}

fn random_rate(rng: &mut Rng) -> u32 {
    const BORDERS: [u32; 11] =
        [92017, 75132, 55426, 46009, 37566, 27713, 23004, 18783, 13856, 11502, 9391];
    match rng.next_u32() % 8 {
        0 => rng.next_u32(),
        1 => BORDERS[(rng.next_u32() % 11) as usize].wrapping_add(rng.next_u32() % 3).wrapping_sub(1),
        // around the wrap of samplingRate * 4 for 768 samples
        2 => (1u32 << 30).wrapping_add(rng.next_u32() % 200_000).wrapping_sub(100_000),
        3 => u32::MAX - rng.next_u32() % 4,
        4 => (BORDERS[(rng.next_u32() % 11) as usize] as u64 * 3 / 4) as u32 + rng.next_u32() % 3,
        _ => RATES[(rng.next_u32() % RATES.len() as u32) as usize],
    }
}

fn check_get_sampling_rate_info(map: &HashMap<usize, &'static [i16]>, spf: u32, sri: u32, sr: u32) {
    let mut f = [0i32; 4];
    let (mut lng, mut sht) = (std::ptr::null(), std::ptr::null());
    let ce = unsafe { c::cref_ci_get_sampling_rate_info(spf, sri, sr, f.as_mut_ptr(), &mut lng, &mut sht) };
    let mut t = SamplingRateInfo::default();
    let re = get_sampling_rate_info(&mut t, spf, sri, sr);
    let what = format!("getSamplingRateInfo({spf}, {sri}, {sr})");
    assert_eq!(re, ce, "{what}: return");
    assert_eq!(
        [
            t.number_of_scale_factor_bands_long as i32,
            t.number_of_scale_factor_bands_short as i32,
            t.sampling_rate_index as i32,
            t.sampling_rate as i32
        ],
        f,
        "{what}: fields"
    );
    same_table(map, lng, t.scale_factor_bands_long, &format!("{what}: long"));
    same_table(map, sht, t.scale_factor_bands_short, &format!("{what}: short"));
}

#[test]
fn get_sampling_rate_info_() {
    let map = sfb_map();
    for &spf in &SPF {
        for sri in 0..20 {
            for &sr in &RATES {
                check_get_sampling_rate_info(&map, spf, sri, sr);
            }
        }
    }
    let mut rng = Rng::new(0xa0c_c001);
    for _ in 0..20000 {
        let (spf, sri, sr) = (random_spf(&mut rng), random_sri(&mut rng), random_rate(&mut rng));
        check_get_sampling_rate_info(&map, spf, sri, sr);
    }
}

const FLAGS: [u32; 9] = [AC_ELD, AC_LD, AC_USAC, AC_RSVD50, AC_RSV603DA, AC_SCALABLE, AC_BSAC, AC_ER, 0x8000_0000];

fn random_flags(rng: &mut Rng) -> u32 {
    let mut f = 0;
    for &b in &FLAGS {
        if rng.next_u32() % 4 == 0 {
            f |= b;
        }
    }
    f
}

fn random_ics(rng: &mut Rng) -> [i32; 16] {
    let mut v = [0i32; 16];
    for (i, x) in v.iter_mut().enumerate() {
        *x = match i {
            11 => match rng.next_u32() % 4 {
                0 => rng.i32(),
                _ => (rng.next_u32() % 4) as i32,
            },
            _ => (rng.next_u32() % 256) as i32,
        };
    }
    v
}

fn check_ics_read(rng: &mut Rng, which: i32) {
    let nbytes = 16usize;
    let mut cbuf: Vec<u8> = (0..nbytes).map(|_| rng.next_u32() as u8).collect();
    let rbuf = cbuf.clone();
    let flags = random_flags(rng);
    let spf = if rng.next_u32() % 4 == 0 { random_spf(rng) } else { SPF[(rng.next_u32() % 5) as usize] };
    let (sri, sr) = (random_sri(rng), random_rate(rng));
    let ics = random_ics(rng);

    let mut cics = ics;
    let mut cleft = 0;
    let ce = unsafe {
        c::cref_ci_ics_read(which, cbuf.as_mut_ptr(), nbytes as i32, flags, spf, sri, sr, cics.as_mut_ptr(), &mut cleft)
    };

    let mut t = SamplingRateInfo::default();
    get_sampling_rate_info(&mut t, spf, sri, sr);
    let mut bs = FdkBitStream::default();
    // SAFETY: rbuf outlives bs.
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&rbuf), nbytes as u32, nbytes as u32 * 8, BS_READER);
    let mut p = ics_from(&ics);
    let re = if which == 0 { ics_read(&mut bs, &mut p, &t, flags) } else { ics_read_max_sfb(&mut bs, &mut p, &t) };
    let what = format!("{} flags {flags:#x} ({spf}, {sri}, {sr}) ics {ics:?}", if which == 0 { "IcsRead" } else { "IcsReadMaxSfb" });
    assert_eq!(re, ce, "{what}: return");
    assert_eq!(ics_to(&p), cics, "{what}: CIcsInfo");
    assert_eq!(fdk_get_valid_bits(&mut bs) as i32, cleft, "{what}: bits left");
}

#[test]
fn ics_read_() {
    let mut rng = Rng::new(0xa0c_c002);
    for _ in 0..30000 {
        check_ics_read(&mut rng, 0);
    }
}

#[test]
fn ics_read_max_sfb_() {
    let mut rng = Rng::new(0xa0c_c003);
    for _ in 0..10000 {
        check_ics_read(&mut rng, 1);
    }
}

#[test]
fn inline_functions() {
    let map = sfb_map();
    let mut rng = Rng::new(0xa0c_c004);
    for _ in 0..20000 {
        let (a, b) = (random_ics(&mut rng), random_ics(&mut rng));
        let spf = SPF[(rng.next_u32() % 5) as usize];
        let (sri, sr) = (random_sri(&mut rng), random_rate(&mut rng));
        let idx = (rng.next_u32() % 8) as i32;
        let tns_sri = (rng.next_u32() % 13) as i32;
        let mut out = [0i32; 16];
        let mut offsets = std::ptr::null();
        let n = unsafe {
            c::cref_ci_inline(a.as_ptr(), b.as_ptr(), spf, sri, sr, idx, tns_sri, out.as_mut_ptr(), &mut offsets)
        } as usize;
        let mut t = SamplingRateInfo::default();
        get_sampling_rate_info(&mut t, spf, sri, sr);
        let (pa, pb) = (ics_from(&a), ics_from(&b));
        let r = [
            is_valid(&pa) as i32,
            is_long_block(&pa) as i32,
            get_window_shape(&pa) as i32,
            get_window_sequence(&pa),
            get_number_of_scale_factor_bands(&pa, &t) as i32,
            get_windows_per_frame(&pa),
            get_window_groups(&pa) as i32,
            get_window_group_length(&pa, idx) as i32,
            get_scale_factor_bands_transmitted(&pa) as i32,
            get_scale_max_factor_bands_transmitted(&pa, &pb) as i32,
            get_scale_factor_bands_total(&pa) as i32,
            get_maximum_tns_bands(&pa, tns_sri) as i32,
            (get_window_group_length_table(&pa).as_ptr() == pa.window_group_length.as_ptr()) as i32,
        ];
        assert_eq!(&out[..n], &r[..], "{a:?} {b:?}");
        same_table(&map, offsets, get_scale_factor_band_offsets(&pa, &t), "GetScaleFactorBandOffsets");
    }
}

#[test]
fn array_lengths() {
    let mut out = [0i32; 128];
    let n = unsafe { c::cref_ci_array_lengths(out.as_mut_ptr()) } as usize;
    let s = CAacDecoderStaticChannelInfo::default();
    let d = CAacDecoderDynamicData::default();
    let mut w = CWorkBufferCore1::default();
    let cd = CAacDecoderCommonData::default();
    let ci = CAacDecoderChannelInfo::default();
    let u = &ci.data.usac;
    let cp = CCplxPredictionData::default();
    let js = CJointStereoData::default();
    let jp = CJointStereoPersistentData::default();
    let pd = CPulseData::default();
    let tns = CTnsData::default();
    let r = [
        CIcsInfo::default().window_group_length.len(),
        s.old_synth.len(),
        s.old_t_pf.len(),
        s.old_gain_pf.len(),
        s.mem_bpf.len(),
        s.lpc4_lsf.len(),
        s.lsf_adaptive_mean.len(),
        s.lp_coeff_old.len() * s.lp_coeff_old[0].len(),
        s.lp_coeff_old_exp.len(),
        s.last_alfd_gains.len(),
        d.a_scale_factor.len(),
        d.a_sfb_scale.len(),
        d.a_code_book.len(),
        d.band_is_noise.len(),
        d.specific_to.aac.a_num_line_in_sec4_hcr.len(),
        d.specific_to.aac.a_code_books4_hcr.len(),
        d.specific_to.usac.tcx_noise_factor.len(),
        d.specific_to.usac.tcx_global_gain.len(),
        w.drm_bs_buffer().len(),
        w.mdct_out_temp().len(),
        w.synth_buf().len(),
        w.work_buffer().len(),
        std::mem::size_of::<CWorkBufferCore1>(),
        cd.pns_random_seed.len(),
        cd.overlay.aac.a_rvlc_scf_esc.len(),
        cd.overlay.aac.a_rvlc_scf_fwd.len(),
        cd.overlay.aac.a_rvlc_scf_bwd.len(),
        2, // pAacDecoderDynamicData[2]: owned by the channels
        u.fac_data0.len(),
        u.fac_data_e.len(),
        u.fac_data.len(),
        u.r#mod.len(),
        u.lsp_coeff.len() * u.lsp_coeff[0].len(),
        u.lp_coeff.len() * u.lp_coeff[0].len(),
        u.lp_coeff_exp.len(),
        u.lsf_adaptive_mean_cand.len(),
        u.a_stability.len(),
        u.acelp.len(),
        u.tcx_gain.len(),
        u.tcx_gain_e.len(),
        ci.spec_scale.len(),
        cp.alpha_q_re.len() * cp.alpha_q_re[0].len(),
        cp.alpha_q_im.len() * cp.alpha_q_im[0].len(),
        cp.alpha_q_re.len(),
        js.ms_used.len(),
        js.store_dmx_re_prev.len(),
        jp.alpha_q_re_prev.len() * jp.alpha_q_re_prev[0].len(),
        jp.alpha_q_im_prev.len() * jp.alpha_q_im_prev[0].len(),
        2, // spectralCoeffs[2]: not stored
        2, // specScale[2]: not stored
        pd.pulse_offset.len(),
        pd.pulse_amp.len(),
        CFilter::default().coeff.len(),
        tns.filter.len() * tns.filter[0].len(),
        tns.filter.len(),
        tns.number_of_filters.len(),
        CPnsInterChannelData::default().correlated.len(),
        CPnsData::default().pns_used.len(),
    ];
    let r: Vec<i32> = r.iter().map(|&x| x as i32).collect();
    assert_eq!(&out[..n], &r[..]);
}

#[test]
fn constants() {
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_ci_constants(out.as_mut_ptr()) } as usize;
    let r = [
        WB_SECTION_SIZE,
        DRM_BS_BUFFER_SIZE,
        AACDEC_RENDER_INVALID,
        AACDEC_RENDER_IMDCT,
        AACDEC_RENDER_ELDFB,
        AACDEC_RENDER_LPD,
        AACDEC_RENDER_INTIMDCT,
        MAX_QUANTIZED_VALUE,
        FD_LONG,
        FD_SHORT,
        LPD,
        ZERO_HCB as i32,
        ESCBOOK as i32,
        NSPECBOOKS as i32,
        BOOKSCL as i32,
        NOISE_HCB as i32,
        INTENSITY_HCB2 as i32,
        INTENSITY_HCB as i32,
        LAST_HCB as i32,
        SFB_PER_PRED_BAND,
        SR_FNA_OUT,
        JointStereoMaximumGroups,
        JointStereoMaximumBands,
        N_MAX_LINES,
        TNS_MAX_WINDOWS,
        TNS_MAXIMUM_FILTERS,
        TNS_MAXIMUM_ORDER,
        NO_OFBANDS,
    ];
    assert_eq!(&out[..n], &r[..]);
}

const ERRORS: [AacDecoderError; 41] = [
    AAC_DEC_OK,
    AAC_DEC_OUT_OF_MEMORY,
    AAC_DEC_UNKNOWN,
    aac_dec_sync_error_start,
    AAC_DEC_TRANSPORT_SYNC_ERROR,
    AAC_DEC_NOT_ENOUGH_BITS,
    aac_dec_sync_error_end,
    aac_dec_init_error_start,
    AAC_DEC_INVALID_HANDLE,
    AAC_DEC_UNSUPPORTED_AOT,
    AAC_DEC_UNSUPPORTED_FORMAT,
    AAC_DEC_UNSUPPORTED_ER_FORMAT,
    AAC_DEC_UNSUPPORTED_EPCONFIG,
    AAC_DEC_UNSUPPORTED_MULTILAYER,
    AAC_DEC_UNSUPPORTED_CHANNELCONFIG,
    AAC_DEC_UNSUPPORTED_SAMPLINGRATE,
    AAC_DEC_INVALID_SBR_CONFIG,
    AAC_DEC_SET_PARAM_FAIL,
    AAC_DEC_NEED_TO_RESTART,
    AAC_DEC_OUTPUT_BUFFER_TOO_SMALL,
    aac_dec_init_error_end,
    aac_dec_decode_error_start,
    AAC_DEC_TRANSPORT_ERROR,
    AAC_DEC_PARSE_ERROR,
    AAC_DEC_UNSUPPORTED_EXTENSION_PAYLOAD,
    AAC_DEC_DECODE_FRAME_ERROR,
    AAC_DEC_CRC_ERROR,
    AAC_DEC_INVALID_CODE_BOOK,
    AAC_DEC_UNSUPPORTED_PREDICTION,
    AAC_DEC_UNSUPPORTED_CCE,
    AAC_DEC_UNSUPPORTED_LFE,
    AAC_DEC_UNSUPPORTED_GAIN_CONTROL_DATA,
    AAC_DEC_UNSUPPORTED_SBA,
    AAC_DEC_TNS_READ_ERROR,
    AAC_DEC_RVLC_ERROR,
    aac_dec_decode_error_end,
    aac_dec_anc_data_error_start,
    AAC_DEC_ANC_DATA_ERROR,
    AAC_DEC_TOO_SMALL_ANC_BUFFER,
    AAC_DEC_TOO_MANY_ANC_ELEMENTS,
    aac_dec_anc_data_error_end,
];

#[test]
fn error_codes() {
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_ci_error_codes(out.as_mut_ptr()) } as usize;
    assert_eq!(&out[..n], &ERRORS[..]);
    let mut rng = Rng::new(0xa0c_c005);
    let mut vals: Vec<i32> = ERRORS.to_vec();
    for e in ERRORS {
        vals.push(e - 1);
        vals.push(e + 1);
    }
    for _ in 0..1000 {
        vals.push(rng.i32());
    }
    for e in vals {
        assert_eq!(is_init_error(e), unsafe { c::cref_ci_is_init_error(e) }, "IS_INIT_ERROR({e:#x})");
        assert_eq!(is_decode_error(e), unsafe { c::cref_ci_is_decode_error(e) }, "IS_DECODE_ERROR({e:#x})");
        assert_eq!(is_output_valid(e), unsafe { c::cref_ci_is_output_valid(e) }, "IS_OUTPUT_VALID({e:#x})");
    }
}

/// The views of CWorkBufferCore1 share the memory of the union, like the C members.
#[test]
fn work_buffer_core1_views() {
    let mut w = CWorkBufferCore1::default();
    w.work_buffer()[0] = 0x0403_0201;
    assert_eq!(&w.drm_bs_buffer()[..4], &[1, 2, 3, 4]);
    w.drm_bs_buffer()[5] = 0x7f;
    assert_eq!(w.mdct_out_temp()[1], 0x7f00);
    assert_eq!(w.synth_buf()[1], 0x7f00);
}

/// The union decision of `CAacDecoderChannelInfo::data` (separate storage) relies on
/// `data.usac.lsp_coeff`, which `CLpc_Conceal()` reads for AAC channels too, lying behind the
/// whole `data.aac` member in C, so that AAC code never writes it.
#[test]
fn data_union_layout() {
    let (mut off, mut size) = (0, 0);
    unsafe { c::cref_ci_data_union_layout(&mut off, &mut size) };
    assert!(off >= size, "lsp_coeff at {off}, data.aac has {size} bytes");
}

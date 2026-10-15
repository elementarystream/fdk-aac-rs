// Differential tests of fdk_aac_rs_sbrenc::ps_main and ps_encode against the C++ reference.
//
// The parametric stereo encoder is set up as the SBR encoder sets it up (two QMF analysis
// filter banks of 64 bands, the 32 band PS synthesis filter bank and the dynamic RAM of the
// SBR encoder) with every stereo band count (and invalid ones), envelope count and IID
// quantization threshold, then runs FDKsbrEnc_PSEnc_ParametricStereoProcessing() on sequences
// of frames of the stereo corpus clips and of synthetic signals (noise, silence, full-scale
// square waves, identical, inverted and one-sided channels, level steps), with PS headers mixed
// in. The downmixed time signal, the QMF scale, the written and counted ps_data(), the complete
// PARAMETRIC_STEREO (PS_ENCODE, PS_OUT, hybrid filter states), the dynamic RAM and the QMF
// filter states are compared after every frame.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::sbrenc_ps_main as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_pcm::qmf_init_analysis_filter_bank;
use fdk_aac_rs_sbrenc::ps_const::*;
use fdk_aac_rs_sbrenc::ps_encode::HybridView;
use fdk_aac_rs_sbrenc::ps_main::*;
use fdk_aac_rs_sbrenc::sbrenc_ram::*;

use super::sbr_encoder_dump::{assert_dump_eq, parametric_stereo, Dump};
use super::sbr_encoder_pcm::Pcm;

type H = *mut core::ffi::c_void;

struct Port {
    ps: Box<ParametricStereo>,
    ana: [QmfFilterBank; 2],
    ana_states: [Vec<i16>; 2],
    syn: QmfFilterBank,
    syn_states: Vec<i32>,
    ram: Vec<i32>,
}

fn c_dump(h: H) -> Vec<i32> {
    let mut buf = vec![0i32; 64 * 1024];
    let n = unsafe { c::cref_ps_main_dump(h, buf.as_mut_ptr(), buf.len() as i32) };
    buf.truncate(n as usize);
    buf
}

fn r_dump(p: &Port) -> Vec<i32> {
    let mut d = Dump::default();
    parametric_stereo(&mut d, &p.ps);
    d.puts(&p.ram);
    for s in p.ana_states.iter() {
        d.0.extend(s.iter().map(|&x| x as i32));
    }
    d.puts(&p.syn_states);
    d.0
}

/// cfg: nStereoBands, maxEnvelopes, iidQuantErrorThreshold, noQmfSlots, noQmfBands.
fn new_port(cfg: &mut [i32; 5]) -> (Port, i32) {
    let mut ps = ps_enc_create();
    let mut ana = [QmfFilterBank::default(); 2];
    let mut ana_states = [vec![0i16; QMF_STATES_ANALYSIS_SIZE], vec![0i16; QMF_STATES_ANALYSIS_SIZE]];
    for ch in 0..2 {
        qmf_init_analysis_filter_bank(&mut ana[ch], Some(&mut ana_states[ch][..]), cfg[3], cfg[4], cfg[4], cfg[4], 0);
    }
    let mut syn = QmfFilterBank::default();
    let mut syn_states = vec![0i32; PS_QMF_STATES_SYNTHESIS_SIZE];
    qmf_init_synthesis_filter_bank(&mut syn, Some(&mut syn_states[..]), cfg[3], cfg[4] >> 1, cfg[4] >> 1, cfg[4] >> 1, 0);
    let mut c = PsencConfig {
        n_stereo_bands: cfg[0],
        max_envelopes: cfg[1],
        iid_quant_error_threshold: cfg[2],
        ..Default::default()
    };
    let ram = get_ram_sbr_dynamic_ram();
    let err = ps_enc_init(Some(&mut ps), Some(&mut c), cfg[3], cfg[4]);
    cfg[1] = c.max_envelopes;
    (Port { ps, ana, ana_states, syn, syn_states, ram }, err)
}

fn port_process(p: &mut Port, pcm: &mut [i16], buf_size: usize, send_header: i32) -> (i32, i32) {
    let [a0, a1] = &p.ana;
    let [s0, s1] = &mut p.ana_states;
    let mut qa = PsQmfAnalysis { bank: [a0, a1], states: [&mut s0[..], &mut s1[..]] };
    let mut qmf_scale = 0i8;
    let err = fdk_sbr_enc_ps_enc_parametric_stereo_processing(
        &mut p.ps,
        pcm,
        [0, buf_size],
        buf_size as u32,
        &mut qa,
        &mut p.ram,
        get_ram_sbr_env_r_buffer(0),
        get_ram_sbr_env_i_buffer(0),
        0,
        &p.syn,
        &mut p.syn_states,
        &mut qmf_scale,
        send_header,
    );
    (err, qmf_scale as i32)
}

fn port_write(p: &Port, count: bool) -> (i32, Vec<u8>, i32) {
    let mem = BitMem::new(1024);
    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(&mut bs, &mem, 1024, 0, BS_WRITER);
    let n = fdk_sbr_enc_ps_enc_write_ps_data(Some(&p.ps), if count { None } else { Some(&mut bs) });
    fdk_sync_cache(&mut bs);
    let v = fdk_get_valid_bits(&mut bs) as i32;
    (n, mem.to_vec(), v)
}

/// A frame of two channels (`buf_size` samples each) of signal `style`.
fn frame(rng: &mut Rng, clips: &[Pcm], style: i32, pos: usize, buf_size: usize) -> Vec<i16> {
    let mut v = vec![0i16; 2 * buf_size];
    match style {
        0..=3 => {
            let clip = &clips[style as usize % clips.len()];
            for ch in 0..2 {
                let f = clip.channel_frame(ch, pos, buf_size);
                v[ch * buf_size..(ch + 1) * buf_size].copy_from_slice(&f);
            }
        }
        4 => {
            let amp = rng.range(0, 15);
            for x in v.iter_mut() {
                *x = rng.i16() >> amp;
            }
        }
        5 => {}
        6 => {
            for (i, x) in v.iter_mut().enumerate() {
                *x = if (i / 7) % 2 == 0 { i16::MAX } else { i16::MIN };
            }
        }
        7..=9 => {
            let clip = &clips[0];
            let f = clip.channel_frame(0, pos, buf_size);
            v[..buf_size].copy_from_slice(&f);
            for i in 0..buf_size {
                v[buf_size + i] = match style {
                    7 => f[i],
                    8 => f[i].saturating_neg(),
                    _ => 0,
                };
            }
        }
        _ => {
            // level steps and a click
            let clip = &clips[1 % clips.len()];
            for ch in 0..2 {
                let f = clip.channel_frame(ch, pos, buf_size);
                for i in 0..buf_size {
                    let g = if i < buf_size / 2 { 1 } else { 8 };
                    v[ch * buf_size + i] = (f[i] as i32 / g) as i16;
                }
                v[ch * buf_size + buf_size / 3] = i16::MAX;
            }
        }
    }
    v
}

#[test]
fn parametric_stereo_processing() {
    let clips = Pcm::all_stereo();
    let mut rng = Rng::new(0x9_5ae1);
    let mut frames = 0;
    for case in 0..40 {
        let bands = [PSENC_STEREO_BANDS_10, PSENC_STEREO_BANDS_20, 0, 15][if case < 36 { case % 2 } else { case % 4 }];
        let mut cfg = [
            bands,
            rng.range(-1, 6),
            if rng.range(0, 1) == 0 { rng.i32() & 0x7fff_ffff } else { 0x1000_0000 >> rng.range(0, 8) },
            32,
            64,
        ];
        let mut cfg_c = cfg;
        let mut err_c = 0;
        let hc = unsafe { c::cref_ps_main_new(cfg_c.as_mut_ptr(), &mut err_c) };
        assert!(!hc.is_null());
        let (mut p, err_r) = new_port(&mut cfg);
        assert_eq!(err_c, err_r, "case {case} init");
        assert_eq!(cfg_c, cfg, "case {case} config");
        if err_r != PSENC_OK {
            unsafe { c::cref_ps_main_free(hc) };
            continue;
        }
        assert_dump_eq(&c_dump(hc), &r_dump(&p), "init");

        let buf_size = 2048;
        let mut pos = rng.range(0, 48000 * 4) as usize;
        let n_frames = 14;
        for fr in 0..n_frames {
            let style = if case % 5 == 0 { rng.range(0, 10) } else { (case % 4) as i32 };
            let pcm = frame(&mut rng, &clips, style, pos, buf_size);
            pos += buf_size;
            let send_header = (rng.range(0, 5) == 0) as i32;
            let mut pcm_c = pcm.clone();
            let mut scale_c = 0;
            let err_c = unsafe { c::cref_ps_main_process(hc, pcm_c.as_mut_ptr(), buf_size as i32, send_header, &mut scale_c) };
            let mut pcm_r = pcm.clone();
            let (err_r, scale_r) = port_process(&mut p, &mut pcm_r, buf_size, send_header);
            assert_eq!((err_c, scale_c), (err_r, scale_r), "case {case} frame {fr}");
            assert_eq!(pcm_c, pcm_r, "case {case} frame {fr} pcm");
            assert_dump_eq(&c_dump(hc), &r_dump(&p), &format!("case {case} frame {fr} state"));
            for count in [false, true] {
                let mut buf_c = vec![0u8; 1024];
                let mut valid_c = 0;
                let n_c = unsafe { c::cref_ps_main_write(hc, count as i32, buf_c.as_mut_ptr(), &mut valid_c) };
                let (n_r, buf_r, valid_r) = port_write(&p, count);
                assert_eq!((n_c, valid_c), (n_r, valid_r), "case {case} frame {fr} ps_data");
                assert_eq!(buf_c, buf_r, "case {case} frame {fr} ps_data bits");
            }
            frames += 1;
        }
        unsafe { c::cref_ps_main_free(hc) };
    }
    assert!(frames > 400);
}

/// A hybrid value for the MINVAL_DBL tests: often `MINVAL_DBL` (whose `fixp_abs()` the
/// reference builds take as not negative), otherwise edge cases and values of any magnitude.
fn hybrid_value(rng: &mut Rng) -> i32 {
    match rng.range(0, 5) {
        0 | 1 => i32::MIN,
        2 => [i32::MAX, -i32::MAX, 0, 1, -1, 0x4000_0000, -0x4000_0000, 0x0100_0000][rng.range(0, 7) as usize],
        3 => rng.i32() >> rng.range(0, 31),
        _ => rng.i32(),
    }
}

/// Writes `v` to `pHybridData[row][ch][ri][k]` of both encoders.
fn poke_hybrid(hc: H, p: &mut Port, row: usize, ch: usize, ri: usize, k: usize, v: i32) {
    unsafe { c::cref_ps_main_poke_hybrid(hc, row as i32, ch as i32, ri as i32, k as i32, v) };
    if row < HYBRID_READ_OFFSET {
        p.ps.static_hybrid_data[row][ch][ri][k] = v;
    } else {
        p.ram[hybrid_dyn_offset(row - HYBRID_READ_OFFSET, ch, ri) + k] = v;
    }
}

/// With `MINVAL_DBL` among the four hybrid values of a band, DownmixPSQmfData() does not scale
/// them (`sc` is 0), the sum of their squares can wrap to a negative `stereoScaleFactor`, and
/// with `tmpScaleFactor == 0` it then calls `schur_div(..., 0, 16)`, which has no defined result
/// in the x86 reference. This clears such bands of the current frame in both encoders before an
/// x86 downmix call (arm64 tests them).
fn avoid_x86_schur_div_by_zero(hc: H, p: &mut Port) {
    use fdk_aac_rs_fdk::common_fix::{f_mult, f_pow2_div2, fixp_abs};
    for n in 0..p.ps.no_qmf_slots as usize {
        for k in 0..71 {
            let at = |p: &Port, ch: usize, ri: usize| p.ram[hybrid_dyn_offset(n, ch, ri) + k];
            let v = [at(p, 0, 0), at(p, 0, 1), at(p, 1, 0), at(p, 1, 1)];
            let m = v.iter().map(|&x| fixp_abs(x) as u32).max().unwrap();
            let sc = (m.leading_zeros() as i32 - 2).max(0) as u32;
            let s = v.map(|x| x.wrapping_shl(sc));
            let stereo = s.iter().fold(0i32, |a, &x| a.wrapping_add(f_pow2_div2(x)));
            let tmp = fixp_abs(stereo.wrapping_add(f_mult(s[0], s[2])).wrapping_add(f_mult(s[1], s[3])));
            let clb = if tmp == 0 { 0 } else { (tmp as u32).leading_zeros() as i32 - 1 };
            if (stereo >> 1) < f_mult(i32::MAX, tmp) && tmp.wrapping_shl(clb as u32) == 0 {
                for (ch, ri) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    poke_hybrid(hc, p, HYBRID_READ_OFFSET + n, ch, ri, k, 0);
                }
            }
        }
    }
}

/// psFindBestScaling(), DownmixPSQmfData() and the PS parameter extraction with `MINVAL_DBL`
/// in the hybrid data. Both reference builds compile the `fixMax(tmp, fixp_abs(x))` chain of
/// psFindBestScaling() as one signed and three unsigned maxima (as findPeakLevels()), the
/// maximum of the four absolute values in DownmixPSQmfData() as an unsigned one, and
/// `CountLeadingBits(fixp_abs(x))` there without the sign fold. Hybrid data of `MINVAL_DBL` is
/// not known to come out of the hybrid analysis of 16-bit PCM, so the test writes it into the
/// hybrid buffers between the calls: into the current frame for the two file-static functions
/// (called through the shim's copy of ps_main.cpp), and into the static rows (the last ten
/// slots of the previous frame) for a whole frame of FDKsbrEnc_PSEnc_ParametricStereoProcessing().
#[test]
fn ps_hybrid_minval() {
    let clips = Pcm::all_stereo();
    let mut rng = Rng::new(0x6d69_6e76);
    let buf_size = 2048;
    let mut calls = [0usize; 3];
    for case in 0..30 {
        let bands = [PSENC_STEREO_BANDS_10, PSENC_STEREO_BANDS_20][case % 2];
        let mut cfg = [bands, [1, 2, 4][case % 3], 0x1000_0000 >> rng.range(0, 8), 32, 64];
        let mut cfg_c = cfg;
        let mut err_c = 0;
        let hc = unsafe { c::cref_ps_main_new(cfg_c.as_mut_ptr(), &mut err_c) };
        assert!(!hc.is_null());
        let (mut p, err_r) = new_port(&mut cfg);
        assert_eq!((err_c, cfg_c), (err_r, cfg), "case {case} init");
        assert_eq!(err_r, PSENC_OK);
        let mut pos = rng.range(0, 48000 * 4) as usize;
        for it in 0..24 {
            // a frame of PCM first (it sets up the hybrid data pointers and fills the hybrid data)
            if it == 0 || it % 3 == 2 {
                let style = rng.range(0, 10);
                let pcm = frame(&mut rng, &clips, style, pos, buf_size);
                pos += buf_size;
                let send_header = (rng.range(0, 3) == 0) as i32;
                let mut pcm_c = pcm.clone();
                let mut scale_c = 0;
                let err_c = unsafe {
                    c::cref_ps_main_process(hc, pcm_c.as_mut_ptr(), buf_size as i32, send_header, &mut scale_c)
                };
                let mut pcm_r = pcm.clone();
                let (err_r, scale_r) = port_process(&mut p, &mut pcm_r, buf_size, send_header);
                let what = format!("case {case} it {it} frame");
                assert_eq!((err_c, scale_c), (err_r, scale_r), "{what}");
                assert_eq!(pcm_c, pcm_r, "{what} pcm");
                assert_dump_eq(&c_dump(hc), &r_dump(&p), &format!("{what} state"));
                for count in [false, true] {
                    let mut buf_c = vec![0u8; 1024];
                    let mut valid_c = 0;
                    let n_c = unsafe { c::cref_ps_main_write(hc, count as i32, buf_c.as_mut_ptr(), &mut valid_c) };
                    let (n_r, buf_r, valid_r) = port_write(&p, count);
                    assert_eq!((n_c, valid_c, &buf_c), (n_r, valid_r, &buf_r), "{what} ps_data");
                }
                calls[2] += 1;
            }
            // MINVAL_DBL and others into the hybrid data: the static rows (used by the next
            // frame's parameter extraction) and the current frame
            let n_poke = [1, 4, 64, 400][rng.range(0, 3) as usize];
            for _ in 0..n_poke {
                let row = if rng.range(0, 2) == 0 {
                    rng.range(0, HYBRID_READ_OFFSET as i32 - 1) as usize
                } else {
                    HYBRID_READ_OFFSET + rng.range(0, 31) as usize
                };
                let (ch, ri) = (rng.range(0, 1) as usize, rng.range(0, 1) as usize);
                let k = rng.range(0, MAX_HYBRID_BANDS as i32 - 1) as usize;
                let v = hybrid_value(&mut rng);
                poke_hybrid(hc, &mut p, row, ch, ri, k, v);
            }
            let what = format!("case {case} it {it}");
            if it % 3 == 1 && cfg!(any(target_arch = "x86_64", target_arch = "x86")) {
                avoid_x86_schur_div_by_zero(hc, &mut p);
            }
            if it % 3 == 0 {
                unsafe { c::cref_ps_main_find_best_scaling(hc) };
                let enc = p.ps.h_ps_encode.as_deref().unwrap();
                let view = HybridView { stat: &p.ps.static_hybrid_data, dyn_ram: &p.ram, base: HYBRID_READ_OFFSET };
                let (mut dbs, mut mbv, mut dmx) = (p.ps.dyn_band_scale, p.ps.max_band_value, p.ps.dmx_scale);
                ps_find_best_scaling(enc, p.ps.no_qmf_slots, &view, &mut dbs, &mut mbv, &mut dmx);
                (p.ps.dyn_band_scale, p.ps.max_band_value, p.ps.dmx_scale) = (dbs, mbv, dmx);
                assert_dump_eq(&c_dump(hc), &r_dump(&p), &format!("{what} psFindBestScaling"));
                calls[0] += 1;
            } else if it % 3 == 1 {
                let scale = [p.ana[0].out_scalefactor, p.ana[1].out_scalefactor];
                let mut pcm_c = vec![0i16; 2 * buf_size];
                let mut qs_c = 0;
                let err_c = unsafe {
                    c::cref_ps_main_downmix(hc, pcm_c.as_mut_ptr(), buf_size as i32, scale[0], scale[1], &mut qs_c)
                };
                let mut pcm_r = vec![0i16; 2 * buf_size];
                let mut qs_r = 0i8;
                let n_slots = p.ps.no_qmf_slots;
                let err_r = downmix_ps_qmf_data(
                    &mut p.ps,
                    &p.syn,
                    &mut p.syn_states,
                    &mut p.ram,
                    get_ram_sbr_env_r_buffer(0),
                    get_ram_sbr_env_i_buffer(0),
                    &mut pcm_r,
                    0,
                    n_slots,
                    &scale,
                    &mut qs_r,
                );
                assert_eq!((err_c, qs_c), (err_r, qs_r as i32), "{what} DownmixPSQmfData");
                assert_eq!(pcm_c, pcm_r, "{what} DownmixPSQmfData pcm");
                assert_dump_eq(&c_dump(hc), &r_dump(&p), &format!("{what} DownmixPSQmfData state"));
                calls[1] += 1;
            }
        }
        unsafe { c::cref_ps_main_free(hc) };
    }
    assert!(calls.iter().all(|&n| n >= 200), "{calls:?}");
}

/// calculateICC() (ps_encode.cpp) with `MINVAL_DBL` cross powers: both reference builds compile
/// `CountLeadingBits(fixMax(fixp_abs(pwrCr), fixp_abs(pwrCi)))` as an unsigned maximum without
/// the sign fold, so the exponent is -1 for `MINVAL_DBL` and the following shifts take it modulo
/// 32. Called through the shim's copy of ps_encode.cpp.
#[test]
fn calculate_icc_minval() {
    use fdk_aac_rs_cref::sbrenc_ps_encode_static as cs;
    use fdk_aac_rs_sbrenc::ps_encode::calculate_icc;
    let mut rng = Rng::new(0x6963_6331);
    let mut minval_cases = 0;
    for iter in 0..20000 {
        type A = [[i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES];
        let mut pwr: [A; 4] = [[[0; PS_MAX_BANDS]; PS_MAX_ENVELOPES]; 4];
        for (j, a) in pwr.iter_mut().enumerate() {
            for v in a.iter_mut().flatten() {
                *v = if j < 2 {
                    // energies (not negative, as PSEncode() makes them)
                    match rng.range(0, 3) {
                        0 => 0,
                        1 => rng.i32() & 0x7fff_ffff,
                        _ => (rng.next_u32() >> rng.range(1, 31)) as i32,
                    }
                } else {
                    match rng.range(0, 3) {
                        0 => i32::MIN,
                        1 => rng.i32(),
                        _ => (rng.next_u32() as i32) >> rng.range(1, 31),
                    }
                };
            }
        }
        if pwr[2].iter().chain(pwr[3].iter()).flatten().any(|&v| v == i32::MIN) {
            minval_cases += 1;
        }
        let n_env = rng.range(1, PS_MAX_ENVELOPES as i32);
        let ps_bands = [PS_BANDS_COARSE, PS_BANDS_MID][rng.range(0, 1) as usize];
        let mut icc_c: A = [[0x5555; PS_MAX_BANDS]; PS_MAX_ENVELOPES];
        let mut pw = pwr;
        unsafe {
            cs::cref_ps_encode_calculateICC(
                pw[0].as_mut_ptr().cast(),
                pw[1].as_mut_ptr().cast(),
                pw[2].as_mut_ptr().cast(),
                pw[3].as_mut_ptr().cast(),
                icc_c.as_mut_ptr().cast(),
                n_env,
                ps_bands,
            )
        };
        let mut icc_r: A = [[0x5555; PS_MAX_BANDS]; PS_MAX_ENVELOPES];
        calculate_icc(&pwr[0], &pwr[1], &pwr[2], &pwr[3], &mut icc_r, n_env, ps_bands);
        assert_eq!(icc_c, icc_r, "iter {iter} n_env {n_env} bands {ps_bands} pwr {pwr:?}");
    }
    assert!(minval_cases > 10000);
}

/// The PS encoder's analysis of one channel as FDKsbrEnc_PSEnc_ParametricStereoProcessing() does
/// it (64-band QMF analysis as the SBR encoder sets it up, THREE_TO_TEN hybrid analysis): the 71
/// hybrid values (real, imaginary) of each of `slots` slots of `pcm`.
fn ps_hybrid_analysis(pcm: &[i16], slots: usize) -> Vec<([i32; 71], [i32; 71])> {
    use fdk_aac_rs_fdk::hybrid::*;
    use fdk_aac_rs_fdk::qmf_pcm::qmf_analysis_filtering_slot;
    let mut ana = QmfFilterBank::default();
    let mut states = vec![0i16; QMF_STATES_ANALYSIS_SIZE];
    qmf_init_analysis_filter_bank(&mut ana, Some(&mut states[..]), 32, 64, 64, 64, 0);
    let mut hyb = FdkAnaHybFilter::default();
    fdk_hybrid_analysis_open(
        &mut hyb,
        vec![0; HYB_ANA_STATES_LF_SIZE],
        (HYB_ANA_STATES_LF_SIZE * 4) as u32,
        vec![0; HYB_ANA_STATES_HF_SIZE],
        (HYB_ANA_STATES_HF_SIZE * 4) as u32,
    );
    assert_eq!(fdk_hybrid_analysis_init(&mut hyb, THREE_TO_TEN, 64, 64, 1), 0);
    let (mut qr, mut qi, mut work) = (vec![0i32; 64], vec![0i32; 64], vec![0i32; 2 * 64]);
    (0..slots)
        .map(|i| {
            qmf_analysis_filtering_slot::<i16>(&ana, &mut states, &mut qr, &mut qi, &pcm[i * 64..], 1, &mut work);
            let (mut hr, mut hi) = ([0i32; 71], [0i32; 71]);
            fdk_hybrid_analysis_apply(&mut hyb, &qr, &qi, &mut hr, &mut hi);
            (hr, hi)
        })
        .collect()
}

/// DownmixPSQmfData() divides by 0 (`schur_div(..., 0, 16)`, no defined result in the x86
/// reference) only if the sum of the squares of a band's four hybrid values wraps
/// (`stereoScaleFactor < 0`): values below 0.25 are first normalized to below 0.5 (sum below
/// 0.5), larger ones are used as they are, so it takes |L|^2 + |R|^2 >= 2.0, i.e. a complex
/// hybrid value of magnitude 1.0 (Q31) in one of the channels. Without the wrap the denominator
/// |L + R|^2 / 2 is 0 only if the condition `stereoScaleFactor / 2 < tmpScaleFactor` fails.
///
/// The hybrid analysis is linear (up to rounding): its impulse responses give, for every
/// hybrid band, the largest magnitude that 16-bit PCM can produce (the sum over the input
/// samples of the absolute complex response times 32768). It is far below 1.0. A PCM signal
/// with the signs of the largest real response (both channels equal, then opposite) is run
/// through the reference and the port: it reaches about the bound for the real part, and the
/// whole PS encoder state is compared.
#[test]
fn ps_hybrid_magnitude() {
    const SLOTS: usize = 40;
    const AMP: i32 = 16384;
    let n = SLOTS * 64;
    // response of slot SLOTS - 1 to an impulse at sample p, per unit input
    let mut resp = vec![vec![(0f64, 0f64); n]; 71];
    for p in 0..n {
        let mut pcm = vec![0i16; n + 64];
        pcm[p] = AMP as i16;
        let (hr, hi) = ps_hybrid_analysis(&pcm, SLOTS)[SLOTS - 1];
        for k in 0..71 {
            resp[k][p] = (hr[k] as f64 / AMP as f64, hi[k] as f64 / AMP as f64);
        }
    }
    let full = 32768.0 / 2f64.powi(31);
    let (mut bound, mut sup, mut bound_re, mut k_re) = (0f64, 0f64, 0f64, 0usize);
    for (k, r) in resp.iter().enumerate() {
        // sum of |z_p|: an upper bound of |sum z_p x_p| for |x_p| <= 1
        let b: f64 = r.iter().map(|&(x, y)| (x * x + y * y).sqrt()).sum::<f64>() * full;
        // the supremum itself: max over the phase t of sum |Re(e^-jt z_p)| (720 phases)
        for t in 0..720 {
            let (c, s) = ((t as f64).to_radians() / 2.0).sin_cos();
            sup = sup.max(r.iter().map(|&(x, y)| (x * s + y * c).abs()).sum::<f64>() * full);
        }
        let b_re: f64 = r.iter().map(|&(x, _)| x.abs()).sum::<f64>() * full;
        bound = bound.max(b);
        if b_re > bound_re {
            (bound_re, k_re) = (b_re, k);
        }
    }
    eprintln!(
        "ps_hybrid_magnitude: hybrid magnitude from 16-bit PCM <= {bound:.4} (Q31; supremum \
         {sup:.4}); real part <= {bound_re:.4} (band {k_re})"
    );
    assert!(bound < 1.0, "bound {bound}");

    // The signal with the signs of band k_re's real response, ending at sample 6144 + 640 (slot
    // 10 of the fourth frame), in both channels (equal or opposite).
    let buf_size = 2048;
    let end = 3 * buf_size + 11 * 64;
    let mut sig = vec![0i16; 4 * buf_size];
    for p in 0..n {
        let x = resp[k_re][p].0;
        sig[end - n + p] = if x > 0.0 { i16::MAX } else if x < 0.0 { i16::MIN } else { 0 };
    }
    let mut reached = 0f64;
    for opposite in [false, true] {
        let mut cfg = [PSENC_STEREO_BANDS_20, 1, 0x1000_0000, 32, 64];
        let mut cfg_c = cfg;
        let mut err_c = 0;
        let hc = unsafe { c::cref_ps_main_new(cfg_c.as_mut_ptr(), &mut err_c) };
        assert!(!hc.is_null());
        let (mut p, err_r) = new_port(&mut cfg);
        assert_eq!((err_c, err_r), (PSENC_OK, PSENC_OK));
        for fr in 0..4 {
            let mut pcm = vec![0i16; 2 * buf_size];
            for i in 0..buf_size {
                let x = sig[fr * buf_size + i];
                pcm[i] = x;
                pcm[buf_size + i] = if opposite { x.saturating_neg() } else { x };
            }
            let mut pcm_c = pcm.clone();
            let mut scale_c = 0;
            let err_c = unsafe { c::cref_ps_main_process(hc, pcm_c.as_mut_ptr(), buf_size as i32, 0, &mut scale_c) };
            let mut pcm_r = pcm.clone();
            let (err_r, scale_r) = port_process(&mut p, &mut pcm_r, buf_size, 0);
            let what = format!("opposite {opposite} frame {fr}");
            assert_eq!((err_c, scale_c), (err_r, scale_r), "{what}");
            assert_eq!(pcm_c, pcm_r, "{what} pcm");
            assert_dump_eq(&c_dump(hc), &r_dump(&p), &format!("{what} state"));
            for slot in 0..32 {
                for (ch, ri) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    for k in 0..71 {
                        let v = p.ram[hybrid_dyn_offset(slot, ch, ri) + k];
                        reached = reached.max((v as f64).abs() / 2f64.powi(31));
                    }
                }
            }
        }
        unsafe { c::cref_ps_main_free(hc) };
    }
    eprintln!("ps_hybrid_magnitude: largest hybrid value of the signal {reached:.4} (Q31)");
    assert!(reached > 0.95 * bound_re && reached <= bound_re * 1.001, "{reached} vs {bound_re}");
}

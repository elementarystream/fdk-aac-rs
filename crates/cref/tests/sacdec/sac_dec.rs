// Differential tests of fdk_aac_rs_sacdec::sac_dec against the C++ reference: FDK_SpatialDecOpen,
// FDK_SpatialDecInit and SpatialDecApplyFrame (with SpatialDecDecodeFrame, the slot processing
// of sac_process, the smoothing, GES, STP and TSD) on random configurations, frames and
// signals, comparing the complete decoder state, the frame, the configuration, the QMF domain
// and the output after every call.

use crate::sac_dec_state::*;
use crate::sac_dec_util::*;
use fdk_aac_rs_cref::sacdec_sac_dec as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::qmf::QMF_FLAG_MPSLDFB;
use fdk_aac_rs_fdk::syslib_channel_map_descr::{fdk_ch_map_descr_init, FdkChannelMapDescr};
use fdk_aac_rs_sacdec::sac_bitdec::*;
use fdk_aac_rs_sacdec::sac_dec::*;
use fdk_aac_rs_sacdec::sac_dec_interface::*;
use fdk_aac_rs_sacdec::sac_process::PcmMpsInput;

/// The user parameters as the shim takes them.
pub fn user_params(p: &SpatialdecParam) -> [i32; 8] {
    [
        p.output_mode as i32,
        p.blind_enable as i32,
        p.bypass_mode as i32,
        p.conceal_method as i32,
        p.conceal_num_keep_frames as i32,
        p.conceal_fade_out_slope_length as i32,
        p.conceal_fade_in_slope_length as i32,
        p.conceal_num_release_frames as i32,
    ]
}

pub fn default_user_params() -> SpatialdecParam {
    SpatialdecParam {
        output_mode: 0,
        blind_enable: 0,
        bypass_mode: 0,
        conceal_method: 1,
        conceal_num_keep_frames: 10,
        conceal_fade_out_slope_length: 5,
        conceal_fade_in_slope_length: 5,
        conceal_num_release_frames: 3,
    }
}

/// A random configuration: LD (ELD) or USAC MPS212, parsed from random bits by the C++
/// parser (both sides get the same bytes). Returns the configuration, the QMF bands and the
/// frame size of the core.
pub fn random_config(rng: &mut Rng) -> Option<(SpatialSpecificConfig, i32, i32)> {
    let mut ssc = SpatialSpecificConfig::default();
    let mut data = random_bytes(rng, 32);
    if rng.next_u32() % 2 == 0 {
        // LD: samplingFreqIndex 3 (48 kHz) or 8 (16 kHz), nTimeSlots-1, freqRes, tree 7,
        // quantMode, arbDmx, fixedGain, tempShape, decorr
        let fsi: u32 = [3, 4, 5, 6, 7, 8][rng.next_u32() as usize % 6];
        let ts: u32 = [7, 15, 31, 14][rng.next_u32() as usize % 4];
        let fr = rng.range(1, 7) as u32;
        let quant = if rng.next_u32() % 4 == 0 { rng.range(0, 2) as u32 } else { 0 };
        let arb = (rng.next_u32() % 6 == 0) as u32;
        let gain = rng.range(0, 7) as u32;
        let tsc = rng.range(0, 2) as u32;
        let dec = rng.range(0, 2) as u32;
        let mut bits: u64 = 0;
        let mut n = 0;
        let mut put = |v: u32, w: u32| {
            bits = (bits << w) | v as u64;
            n += w;
        };
        put(fsi, 4);
        put(ts, 5);
        put(fr, 3);
        put(7, 4);
        put(quant, 2);
        put(arb, 1);
        put(gain, 3);
        put(tsc, 2);
        put(dec, 2);
        put(rng.next_u32() & 1, 1); // envQuantMode or padding
        let bits = bits << (64 - n);
        data[..8].copy_from_slice(&bits.to_be_bytes());
        // No extension config: bytes after the alignment are random (sacExtType/len)
        for b in data[4..].iter_mut() {
            *b = 0;
        }
        let mut bs = Bs::new(&data, 32, 32 * 8);
        let r = spatial_dec_parse_specific_config(&mut bs.bs, &mut ssc, 0, AOT_ER_AAC_ELD);
        if r != MPS_OK {
            return None;
        }
        let fs = ssc.sampling_freq;
        let qmf = if fs < 27713 { 32 } else { 64 };
        Some((ssc, qmf, ssc.n_time_slots * qmf))
    } else {
        let sci = [1, 2, 3][rng.next_u32() as usize % 3];
        let csfli = if sci == 3 { [2, 3, 4][rng.next_u32() as usize % 3] } else { [3, 4][rng.next_u32() as usize % 2] };
        // freqRes (3 bits, not 0), fixedGain, tempShape, decorr (not 3), highRate, phase,
        // otbPresent, ...
        data[0] = (data[0] & 0x1f) | ((rng.range(1, 7) as u8) << 5);
        if (data[0] >> 1) & 3 == 3 {
            data[0] &= !2;
        }
        let mut bs = Bs::new(&data, 32, 32 * 8);
        let r = spatial_dec_parse_mps212_config(&mut bs.bs, &mut ssc, 44100, AOT_USAC, sci, csfli);
        if r != MPS_OK {
            return None;
        }
        let qmf = if sci == 3 { [24, 32, 16][csfli as usize - 2] } else { 64 };
        Some((ssc, qmf, ssc.n_time_slots * qmf))
    }
}

/// The QMF domain configuration of the time interface (as the AAC decoder requests it).
pub fn time_interface_gc(ssc: &SpatialSpecificConfig, qmf: i32, frame: i32) -> [i32; 21] {
    let mut g = [0i32; 21];
    g[2] = 1 + (ssc.stereo_config_index > 1) as i32; // nInputChannels_requested
    g[4] = 2; // nOutputChannels_requested
    g[8] = if ssc.core_codec == AOT_ER_AAC_ELD { QMF_FLAG_MPSLDFB as i32 } else { 0 };
    g[10] = qmf; // nBandsAnalysis_requested
    g[12] = if ssc.stereo_config_index == 3 { 64 } else { qmf }; // nBandsSynthesis_requested
    g[14] = frame / qmf; // nQmfTimeSlots_requested
    g[18] = 64; // nQmfProcBands_requested
    g[20] = 1; // nQmfProcChannels_requested
    g
}

/// Opens a pair, configures the QMF domain and initializes both decoders with `ssc`.
pub fn init_pair(ssc: &SpatialSpecificConfig, qmf: i32, frame: i32, flags: u32) -> (Pair, i32) {
    let mut p = Pair::new();
    p.set_ssc(ssc);
    p.configure_qd(&time_interface_gc(ssc, qmf, frame));
    p.check("open");
    let up = default_user_params();
    let r = fdk_spatial_dec_init(&mut p.r.sd, &mut p.r.qd, &mut p.r.frame, &mut p.r.ssc, qmf, UPMIX_TYPE_NORMAL, &up, flags);
    let cr = unsafe { c::cref_sacinst_init(p.c.0, qmf, UPMIX_TYPE_NORMAL, user_params(&up).as_ptr(), flags) };
    assert_eq!(r, cr, "FDK_SpatialDecInit");
    p.check("FDK_SpatialDecInit");
    (p, r)
}

pub const INIT_ALL: u32 = MASK_MPEGS_INIT_ALL_STATES | MASK_MPEGS_INIT_ALL_PARAMS | MPEGS_INIT_CONFIG;

#[test]
fn sac_dec_open_init() {
    let mut rng = Rng::new(0xd1);
    let mut ok = 0;
    for iter in 0..400 {
        let Some((ssc, qmf, frame)) = random_config(&mut rng) else { continue };
        let flags = if iter % 3 == 0 { rng.next_u32() & (MASK_MPEGS_INIT_ALL_STATES | MASK_MPEGS_INIT_ALL_PARAMS | 0x10) } else { INIT_ALL };
        let (mut p, r) = init_pair(&ssc, qmf, frame, flags);
        if r == MPS_OK {
            ok += 1;
        }
        // A second init with other flags and another band count.
        let qmf2 = [16, 24, 32, 64, 128, 0][rng.next_u32() as usize % 6];
        let flags2 = rng.next_u32();
        let up = default_user_params();
        let r = fdk_spatial_dec_init(&mut p.r.sd, &mut p.r.qd, &mut p.r.frame, &mut p.r.ssc, qmf2, UPMIX_TYPE_NORMAL, &up, flags2);
        let cr = unsafe { c::cref_sacinst_init(p.c.0, qmf2, UPMIX_TYPE_NORMAL, user_params(&up).as_ptr(), flags2) };
        assert_eq!(r, cr, "FDK_SpatialDecInit (2)");
        p.check("FDK_SpatialDecInit (2)");
    }
    assert!(ok > 100, "{ok} successful initializations");
}

/// The default channel map descriptor (passThrough 0 or 1).
pub fn map_descr(pass_through: u32) -> FdkChannelMapDescr {
    let mut d = FdkChannelMapDescr::default();
    fdk_ch_map_descr_init(&mut d, None, 0, pass_through);
    d
}

/// Runs SpatialDecApplyFrame on both sides with the time interface (or the QMF interface for
/// stereoConfigIndex 3: the work buffers of the QMF domain), compares everything.
fn apply_frame(p: &mut Pair, rng: &mut Rng, frame_size: i32, with_residual: bool, what: &str) {
    let n_out = 2 * frame_size * if p.r.ssc.stereo_config_index == 3 { 64 / p.r.sd.qmf_bands.max(1) } else { 1 } as i32;
    let n_in = (frame_size * 2) as usize;
    let input: Vec<i32> = match rng.next_u32() % 4 {
        0 => (0..n_in).map(|_| rng.i32()).collect(),
        1 => vec![0; n_in],
        _ => (0..n_in).map(|_| (rng.next_u32() as i32) >> rng.range(8, 20)).collect(),
    };
    let mut out = vec![0x1234i32; (n_out as usize).max(n_in) + 64];
    // USAC with residual: the input in the output buffer (as mpegSurroundDecoder_Apply() places
    // it), which the slots are written to while the input is read
    let in_place = with_residual && rng.next_u32() % 2 == 0;
    if in_place {
        out[..n_in].copy_from_slice(&input);
    }
    let mut cout = out.clone();
    let mut flags = if rng.next_u32() % 8 == 0 { MPEGS_CONCEAL } else { 0 } | if rng.next_u32() % 16 == 0 { MPEGS_BYPASSMODE } else { 0 };
    let mut cflags = flags;
    let n_samples = if rng.next_u32() % 16 == 0 { rng.range(0, 3 * frame_size) as u32 } else { frame_size as u32 };
    let n_in_ch = if with_residual { 2 } else if rng.next_u32() % 32 == 0 { 2 } else { 1 };
    let pass = (rng.next_u32() % 4 == 0) as u32;
    let md = map_descr(pass);
    let mode = INPUTMODE_TIME;
    let r = spatial_dec_apply_frame(
        &mut p.r.sd,
        &mut p.r.qd,
        &mut p.r.ssc,
        &mut p.r.frame,
        &mut p.r.ll,
        mode,
        if in_place { PcmMpsInput::Output(0) } else { PcmMpsInput::Buffer(&input) },
        &mut out,
        n_samples,
        &mut flags,
        n_in_ch,
        &md,
    );
    let cr = unsafe {
        c::cref_sacinst_apply_frame(
            p.c.0,
            mode,
            input.as_ptr(),
            if in_place { 0 } else { -1 },
            cout.as_mut_ptr(),
            n_samples,
            &mut cflags,
            n_in_ch,
            pass as i32,
        )
    };
    assert_eq!(r, cr, "{what}: SpatialDecApplyFrame result");
    assert_eq!(flags, cflags, "{what}: control flags");
    if out != cout {
        let i = out.iter().zip(&cout).position(|(a, b)| a != b).unwrap();
        panic!("{what}: output differs at {i}: {} vs {}", out[i], cout[i]);
    }
    p.check(what);
}

/// Parses a random frame on both sides (the parser state is compared by check()).
fn parse_random_frame(p: &mut Pair, rng: &mut Rng, what: &str) {
    let data = random_bytes(rng, 128);
    let valid = rng.range(1, 1024);
    let mut bs = Bs::new(&data, 128, valid as u32);
    let indep = (rng.next_u32() % 4 == 0) as i32;
    let r = spatial_dec_parse_frame_data(&mut p.r.sd, &mut p.r.frame, &mut p.r.ll, &mut bs.bs, &p.r.ssc.clone(), UPMIXTYPE_NORMAL, indep);
    let mut cbuf = data.clone();
    let mut cleft = 0;
    let cr = unsafe { c::cref_sacinst_parse_frame(p.c.0, cbuf.as_mut_ptr(), 128, valid, UPMIXTYPE_NORMAL, indep, &mut cleft) };
    assert_eq!(r, cr, "{what}: SpatialDecParseFrameData");
    assert_eq!(bs.valid(), cleft, "{what}: bits");
    if r == MPS_OK {
        p.r.frame.new_bs_data = 1;
        let mut v = p.c.dump_frame();
        v[1] = 1; // newBsData
        unsafe { c::cref_sacinst_load_frame(p.c.0, v.as_ptr()) };
    }
    p.check(what);
}

#[test]
fn sac_dec_apply_frame_random() {
    let mut rng = Rng::new(0xd2);
    let mut frames = 0;
    for iter in 0..150 {
        let Some((ssc, qmf, frame)) = random_config(&mut rng) else { continue };
        if ssc.stereo_config_index == 3 {
            continue; // QMF interface to SBR (covered by the end-to-end tests)
        }
        let (mut p, r) = init_pair(&ssc, qmf, frame, INIT_ALL);
        if r != MPS_OK {
            continue;
        }
        // sacInDataHeadroom is set by mpegSurroundDecoder_Apply() before each frame.
        let mut v = dump_self(&p.r.sd);
        let hr = plain_len() - 1;
        v[hr] = [1, 2, 8, 3][rng.next_u32() as usize % 4];
        p.load(&v);
        for f in 0..12 {
            if rng.next_u32() % 3 != 0 {
                parse_random_frame(&mut p, &mut rng, &format!("parse {iter}/{f}"));
            }
            apply_frame(&mut p, &mut rng, frame, ssc.stereo_config_index > 1, &format!("apply {iter}/{f}"));
            frames += 1;
        }
    }
    assert!(frames > 300, "{frames} frames");
}

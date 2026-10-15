// Differential tests of fdk_aac_rs_aacenc::aacenc (the functions of aacenc.cpp) against the
// C++ reference.
//
// * FDKaacEnc_CalcBitsPerFrame, FDKaacEnc_CalcBitrate, FDKaacEnc_LimitBitrate (without and with
//   transport encoders of every format), FDKaacEnc_GetVBRBitrate,
//   FDKaacEnc_AdjustVBRBitrateMode, FDKaacEnc_InitCheckAncillary and
//   FDKaacEnc_AacInitDefaultConfig on random and edge inputs; the table configTabVBR.
// * FDKaacEnc_Open, FDKaacEnc_Initialize, FDKaacEnc_EncodeFrame, FDKaacEnc_Close and
//   FDKaacEnc_GetBitReservoirState are compared through the API (aacenc_lib.rs: every output
//   byte and AACENC_OutArgs::bitResState after every call) and stage by stage in
//   qc_main_chain.rs.

use crate::bitenc::Tp;
use fdk_aac_rs_aacenc::aacenc::*;
use fdk_aac_rs_cref::aacenc_aacenc as c;
use fdk_aac_rs_cref::aacenc_bitenc as cb;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

const RATES: [i32; 12] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
const FRAMES: [i32; 8] = [1024, 960, 512, 480, 256, 240, 128, 120];
const MODES: [ChannelMode; 12] = [1, 2, 3, 4, 5, 6, 7, 11, 12, 14, 33, 34];

/// A positive value of varying magnitude (no 0: a division by 0 has no defined result).
fn pos(rng: &mut Rng) -> i32 {
    match rng.range(0, 3) {
        0 => rng.range(1, 100),
        1 => rng.range(1, 100000),
        2 => rng.range(1, 2_000_000),
        _ => (rng.next_u32() >> 1).max(1) as i32 & 0x3fff_ffff | 1,
    }
}

#[test]
fn calc_bits_per_frame_and_bitrate() {
    let mut rng = Rng::new(5);
    for k in 0..200_000 {
        let fl = if k % 3 == 0 { pos(&mut rng) } else { FRAMES[rng.range(0, 7) as usize] };
        let sr = if k % 5 == 0 { pos(&mut rng) } else { RATES[rng.range(0, 11) as usize] };
        let br = if k % 7 == 0 { rng.i32() } else { rng.range(-1, 600000) };
        // SAFETY: shim calls.
        let c1 = unsafe { c::cref_aacenc_calc_bits_per_frame(br, fl, sr) };
        assert_eq!(fdk_aac_enc_calc_bits_per_frame(br, fl, sr), c1, "CalcBitsPerFrame({br}, {fl}, {sr})");
        let c2 = unsafe { c::cref_aacenc_calc_bitrate(br, fl, sr) };
        assert_eq!(fdk_aac_enc_calc_bitrate(br, fl, sr), c2, "CalcBitrate({br}, {fl}, {sr})");
    }
}

fn limit_case(rng: &mut Rng, tp: Option<&mut Tp>, what: &str) {
    let aot = [2, 5, 29, 23, 39, 129, 132][rng.range(0, 6) as usize];
    let sr = RATES[rng.range(0, 11) as usize];
    let fl = FRAMES[rng.range(0, 7) as usize];
    let nch = rng.range(1, 8);
    let nche = rng.range(1, nch);
    let br = match rng.range(0, 3) {
        0 => rng.range(-1, 1000),
        1 => rng.range(0, 1_000_000),
        2 => rng.i32() & 0x3fff_ffff,
        _ => rng.range(8000, 300000),
    };
    let avg_bits = rng.range(-1, 10000);
    let mode = rng.range(-1, 7);
    let nsub = rng.range(1, 4);
    let mut ac = -12345;
    let mut ar = -12345;
    let use_avg = rng.range(0, 3) != 0;
    let (hc, hr) = match tp {
        // SAFETY: shim call.
        Some(t) => (unsafe { cb::cref_bitenc_tp_handle(t.c) }, Some(&mut *t.r)),
        None => (core::ptr::null_mut(), None),
    };
    // SAFETY: shim call.
    let rc = unsafe {
        c::cref_aacenc_limit_bitrate(
            hc,
            aot,
            sr,
            fl,
            nch,
            nche,
            br,
            avg_bits,
            if use_avg { &mut ac } else { core::ptr::null_mut() },
            mode,
            nsub,
        )
    };
    let rr = fdk_aac_enc_limit_bitrate(
        hr,
        aot,
        sr,
        fl,
        nch,
        nche,
        br,
        avg_bits,
        if use_avg { Some(&mut ar) } else { None },
        mode,
        nsub,
    );
    assert_eq!((rr, ar), (rc, ac), "{what}: LimitBitrate(aot {aot}, {sr}, {fl}, {nch}, {nche}, {br}, ..., {nsub})");
}

#[test]
fn limit_bitrate() {
    let mut rng = Rng::new(6);
    for k in 0..20000 {
        limit_case(&mut rng, None, &format!("no transport, case {k}"));
    }
    // with transport encoders (their static bits depend on the format and the frame size)
    let cfgs: [(TransportType, [i32; 8]); 6] = [
        (TT_MP4_RAW, [2, 2, 44100, 1024, 2, 0, 0, 128000]),
        (TT_MP4_ADIF, [2, 2, 44100, 1024, 2, 0, 0, 128000]),
        (TT_MP4_ADTS, [2, 2, 44100, 1024, 2, 0, 10, 128000]),
        (TT_MP4_ADTS, [2, 6, 48000, 1024, 6, CC_PROTECTION as i32, 10, 320000]),
        (TT_MP4_LOAS, [2, 2, 48000, 1024, 2, CC_MPEG_ID as i32, 10, 96000]),
        (TT_MP4_LATM_MCP1, [2, 1, 32000, 1024, 1, CC_MPEG_ID as i32, 0, 48000]),
    ];
    for (fmt, cfg) in cfgs {
        let mut tp = Tp::open(fmt, cfg);
        for k in 0..3000 {
            limit_case(&mut rng, Some(&mut tp), &format!("transport {fmt} {cfg:?} case {k}"));
        }
    }
}

#[test]
fn vbr_bitrate_and_mode() {
    let mut rng = Rng::new(7);
    for mode in -1..=8 {
        for &cm in MODES.iter() {
            // SAFETY: shim call (only modes of the channel mode table: the reference has no
            // defined result for others).
            let rc = unsafe { c::cref_aacenc_get_vbr_bitrate(mode, cm) };
            assert_eq!(fdk_aac_enc_get_vbr_bitrate(mode, cm), rc, "GetVBRBitrate({mode}, {cm})");
            for _ in 0..200 {
                let br = match rng.range(0, 2) {
                    0 => -1,
                    1 => rng.range(0, 1_000_000),
                    _ => rng.i32(),
                };
                let rc = unsafe { c::cref_aacenc_adjust_vbr_bitrate_mode(mode, br, cm) };
                assert_eq!(
                    fdk_aac_enc_adjust_vbr_bitrate_mode(mode, br, cm),
                    rc,
                    "AdjustVBRBitrateMode({mode}, {br}, {cm})"
                );
            }
        }
    }
}

#[test]
fn init_check_ancillary() {
    let mut rng = Rng::new(8);
    for k in 0..50000 {
        let br = if k % 4 == 0 { rng.i32() } else { rng.range(0, 400000) };
        let anc = match rng.range(0, 4) {
            0 => -1,
            1 => 0,
            2 => rng.i32(),
            3 => rng.range(-5, 30000),
            _ => rng.range(0, 20000),
        };
        let fl = FRAMES[rng.range(0, 7) as usize];
        let sr = RATES[rng.range(0, 11) as usize];
        let (mut bc, mut brr) = (-7, -7);
        // SAFETY: shim call.
        let ec = unsafe { c::cref_aacenc_init_check_ancillary(br, fl, anc, &mut bc, sr) };
        let er = fdk_aac_enc_init_check_ancillary(br, fl, anc, &mut brr, sr);
        assert_eq!((er, brr), (ec, bc), "InitCheckAncillary({br}, {fl}, {anc}, .., {sr})");
    }
}

#[test]
fn aac_init_default_config() {
    // SAFETY: shim call.
    assert_eq!(unsafe { c::cref_aacenc_config_size() } as usize, core::mem::size_of::<AacencConfig>());
    let mut cc = AacencConfig::default();
    // SAFETY: AacencConfig has the C layout (size checked above).
    unsafe { c::cref_aacenc_init_default_config(&mut cc as *mut AacencConfig as *mut core::ffi::c_void) };
    let mut rc = AacencConfig { sample_rate: 99, bit_rate: 77, ..Default::default() };
    fdk_aac_enc_aac_init_default_config(&mut rc);
    assert_eq!(rc, cc);
}

#[test]
fn config_tab_vbr() {
    let mut v = [0i32; 64];
    // SAFETY: shim writes 3 values per entry.
    let n = unsafe { c::cref_aacenc_config_tab_vbr(v.as_mut_ptr()) } as usize;
    assert_eq!(n, 6);
    for i in 0..n {
        // GetVBRBitrate returns chanBitrate * nChannelsEff for VBR modes: entries 1..=5 through
        // mono (1 effective channel) and stereo (2)
        if (1..=5).contains(&v[3 * i]) {
            assert_eq!(fdk_aac_enc_get_vbr_bitrate(v[3 * i], MODE_1), v[3 * i + 1], "entry {i}");
            assert_eq!(fdk_aac_enc_get_vbr_bitrate(v[3 * i], MODE_2), 2 * v[3 * i + 2], "entry {i}");
        }
    }
    // the table itself
    let t = fdk_aac_rs_aacenc::aacenc::config_tab_vbr();
    for i in 0..n {
        assert_eq!(t[i], (v[3 * i], [v[3 * i + 1], v[3 * i + 2]]), "configTabVBR[{i}]");
    }
}

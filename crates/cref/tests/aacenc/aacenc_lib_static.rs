// Differential tests of the file-static functions and tables of fdk_aac_rs_aacenc::aacenc_lib
// (shim aacenc_aacenc_lib_static.cpp): eldSbrAutoConfigTab, eldSbrConfigurator,
// getSbrSignalingMode, GetCoreChannelMode, isPsActive, isSbrActive, FDKaacEnc_GetCBRBitrate,
// aacEncoder_LimitBitrate, aacEncDistributeSbrBits and FDKaacEnc_MapConfig. The other internal
// functions (FDKaacEnc_AdjustEncSettings, aacEncInit, the callbacks) are compared through the
// API (aacenc_lib.rs).

use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc::*;
use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_cref::aacenc_aacenc_lib_static as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_sbrenc::sbr_encoder::SbrElementInfo;

const RATES: [i32; 12] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
const MODES: [ChannelMode; 12] = [1, 2, 3, 4, 5, 6, 7, 11, 12, 14, 33, 34];
const AOTS: [AudioObjectType; 8] = [2, 5, 29, 23, 39, 129, 132, 0];

#[test]
fn eld_table_and_configurator() {
    let mut v = [0u32; 5 * 40];
    // SAFETY: shim writes 5 values per entry.
    let n = unsafe { c::cref_aacenc_lib_static_eld_tab(v.as_mut_ptr()) } as usize;
    let t = eld_sbr_auto_config_tab();
    assert_eq!(t.len(), n);
    for (i, e) in t.iter().enumerate() {
        assert_eq!(
            [e.n_channels, e.sampling_rate, e.bitrate_range, e.sbr_mode as u32, e.ch_mode as u32],
            v[5 * i..5 * i + 5],
            "eldSbrAutoConfigTab[{i}]"
        );
    }
    let mut rng = Rng::new(41);
    for _ in 0..20000 {
        let nch = rng.range(0, 3) as u32;
        let sr = if rng.range(0, 2) == 0 { rng.next_u32() } else { RATES[rng.range(0, 11) as usize] as u32 };
        let br = if rng.range(0, 2) == 0 { rng.next_u32() } else { rng.range(0, 200000) as u32 };
        // SAFETY: shim call.
        let ec = unsafe { c::cref_aacenc_lib_static_eld_configurator(nch, sr, br) };
        let er = eld_sbr_configurator(nch, sr, br).map_or(-1, |p| (p as *const EldSbrConfigurator as usize - t.as_ptr() as usize) as i32 / core::mem::size_of::<EldSbrConfigurator>() as i32);
        assert_eq!(er, ec, "eldSbrConfigurator({nch}, {sr}, {br})");
    }
}

#[test]
fn signaling_and_modes() {
    for &aot in AOTS.iter().chain([1, 17, 42].iter()) {
        for tt in [-1, 0, 1, 2, 6, 7, 10, 12] {
            for sig in [0i32, 1, 2, 3, 0xff, 0x80] {
                for sbr in 0..=3u32 {
                    // SAFETY: shim call.
                    let ec = unsafe { c::cref_aacenc_lib_static_signaling(aot, tt, sig, sbr) };
                    assert_eq!(get_sbr_signaling_mode(aot, tt, sig as u8, sbr), ec, "getSbrSignalingMode({aot}, {tt}, {sig}, {sbr})");
                }
            }
        }
        for mode in [-1, 0, 1, 2, 3, 6, 7, 11, 128, 129] {
            // SAFETY: shim call.
            let ec = unsafe { c::cref_aacenc_lib_static_core_mode(mode, aot) };
            assert_eq!(get_core_channel_mode(mode, aot) | (is_ps_active(aot) << 16), ec, "GetCoreChannelMode({mode}, {aot})");
        }
    }
}

#[test]
fn cbr_bitrate() {
    let mut rng = Rng::new(42);
    for _ in 0..20000 {
        let mut cfg = AacencConfig::default();
        cfg.audio_object_type = AOTS[rng.range(0, 7) as usize];
        cfg.channel_mode = MODES[rng.range(0, 11) as usize];
        cfg.sample_rate = if rng.range(0, 3) == 0 { rng.i32() } else { RATES[rng.range(0, 11) as usize] };
        cfg.syntax_flags = [0u32, AC_SBR_PRESENT, AC_ER | AC_ELD, AC_SBR_PRESENT | AC_ELD][rng.range(0, 3) as usize];
        let ratio = rng.range(0, 3);
        let mut sa = -1;
        // SAFETY: AacencConfig has the C layout.
        let ec = unsafe { c::cref_aacenc_lib_static_cbr(&cfg as *const AacencConfig as *const c_void, ratio, &mut sa) };
        assert_eq!((fdk_aac_enc_get_cbr_bitrate(&cfg, ratio), is_sbr_active(&cfg)), (ec, sa), "{cfg:?} ratio {ratio}");
    }
}

#[test]
fn limit_bitrate_and_distribution() {
    let mut rng = Rng::new(43);
    for k in 0..40000 {
        let aot = AOTS[rng.range(0, 6) as usize];
        let sr = RATES[rng.range(0, 11) as usize];
        let fl = [1024, 960, 512, 480, 256, 240, 128, 120][rng.range(0, 7) as usize];
        let mode = MODES[rng.range(0, 11) as usize];
        let nch = rng.range(1, 8);
        let br = match rng.range(0, 2) {
            0 => rng.range(0, 600000),
            1 => rng.range(-10, 50000),
            _ => rng.i32() & 0x0fff_ffff,
        };
        let nsub = rng.range(1, 4);
        let sbr_active = rng.range(0, 1);
        // the core sample rate is samplingRate >> (ratio - 1): the ratio is 1 or 2 with SBR
        let ratio = if sbr_active != 0 { rng.range(1, 2) } else { rng.range(0, 2) };
        let flags = [0u32, AC_LD_MPS, AC_SBR_PRESENT][rng.range(0, 2) as usize];
        // SBR limits only stereo/mono elements (sbrEncoder_LimitBitRate asserts 1 or 2
        // channels); PS needs one element
        // SAFETY: shim call.
        let ec = unsafe { c::cref_aacenc_lib_static_limit(sr, fl, nch, mode, br, nsub, sbr_active, ratio, flags, aot) };
        let er = aac_encoder_limit_bitrate(None, sr, fl, nch, mode, br, nsub, sbr_active, ratio, flags, aot);
        assert_eq!(er, ec, "case {k}: aacEncoder_LimitBitrate({sr}, {fl}, {nch}, {mode}, {br}, {nsub}, {sbr_active}, {ratio}, {flags:#x}, {aot})");

        let order = rng.range(0, 2);
        let mut out = [0i32; 64];
        // SAFETY: shim writes 8 values per element.
        let n = unsafe { c::cref_aacenc_lib_static_distribute(mode, order, br, out.as_mut_ptr()) } as usize;
        let mut cm = fdk_aac_rs_aacenc::qc_data::ChannelMapping::default();
        fdk_aac_rs_aacenc::channel_map::fdk_aac_enc_init_channel_mapping(mode, order, &mut cm);
        let mut el = [SbrElementInfo::default(); 8];
        aac_enc_distribute_sbr_bits(&cm, &mut el, br);
        assert_eq!(cm.n_elements as usize, n);
        for i in 0..n {
            let e = &el[i];
            assert_eq!(
                [
                    e.channel_index[0] as i32,
                    e.channel_index[1] as i32,
                    e.el_type,
                    e.bit_rate,
                    e.instance_tag,
                    e.n_channels_in_el as i32,
                    e.f_parametric_stereo as i32,
                    e.f_dual_mono as i32
                ],
                out[8 * i..8 * i + 8],
                "aacEncDistributeSbrBits({mode}, {order}, {br}) element {i}"
            );
        }
    }
}

#[test]
fn map_config() {
    let mut rng = Rng::new(44);
    for k in 0..30000 {
        let mut cfg = AacencConfig::default();
        cfg.audio_object_type = AOTS[rng.range(0, 7) as usize];
        cfg.channel_mode = MODES[rng.range(0, 11) as usize];
        cfg.sample_rate = RATES[rng.range(0, 11) as usize];
        cfg.framelength = [1024, 512, 480, 256][rng.range(0, 3) as usize];
        cfg.bit_rate = rng.range(0, 500000);
        cfg.n_channels = rng.range(1, 8);
        cfg.n_sub_frames = rng.range(1, 3);
        cfg.ep_config = [-1i8, 0, 1][rng.range(0, 2) as usize];
        cfg.downscale_factor = [1u32, 2, 4, 0][rng.range(0, 3) as usize];
        cfg.syntax_flags = rng.next_u32() & (AC_SBR_PRESENT | AC_LD_MPS | AC_ER_VCB11 | AC_ER_HCR | AC_ER_RVLC | AC_ER | AC_ELD);
        let u = [
            RATES[rng.range(0, 11) as usize] as u32,
            rng.range(1, 4) as u32,
            rng.range(0, 1) as u32,
            [0xFFu32, 0, 1, 10, 255][rng.range(0, 4) as usize],
            [0u32, 1, 2, 6, 7, 10, 0xFFFF_FFFF][rng.range(0, 6) as usize],
            rng.range(0, 15) as u32,
        ];
        let sig = rng.range(-1, 2);
        let mut out = [0i32; 18];
        // SAFETY: AacencConfig has the C layout.
        unsafe { c::cref_aacenc_lib_static_map_config(u.as_ptr(), sig, &cfg as *const AacencConfig as *const c_void, out.as_mut_ptr()) };
        let ext = UserParam {
            user_samplerate: u[0],
            user_tp_nsub_frames: u[1] as u8,
            user_tp_protection: u[2] as u8,
            user_tp_header_period: u[3] as u8,
            user_tp_type: u[4] as i32,
            user_pce_additions: u[5],
            ..Default::default()
        };
        let mut cc = CoderConfig::default();
        cc.aot = 77;
        fdk_aac_enc_map_config(&mut cc, &ext, sig, &cfg);
        let r = [
            cc.aot,
            cc.ext_aot,
            cc.channel_mode,
            cc.channel_config_zero as i32,
            cc.sampling_rate,
            cc.ext_sampling_rate,
            cc.downscale_sampling_rate,
            cc.bit_rate,
            cc.samples_per_frame,
            cc.no_channels,
            cc.bits_frame,
            cc.n_sub_frames,
            cc.flags as i32,
            cc.matrix_mixdown_a as i32,
            cc.header_period as i32,
            cc.sbr_signaling,
            cc.sbr_present as i32,
            cc.ps_present as i32,
        ];
        assert_eq!(r, out, "case {k}: FDKaacEnc_MapConfig {cfg:?} {u:?} {sig}");
    }
}

// Differential tests of fdk_aac_rs_aacenc::channel_map against the C++ reference.

use fdk_aac_rs_aacenc::aacenc::{CH_ORDER_MPEG, CH_ORDER_WAV, CH_ORDER_WG4};
use fdk_aac_rs_aacenc::channel_map::*;
use fdk_aac_rs_aacenc::qc_data::{ChannelMapping, ElementBits, QcState};
use fdk_aac_rs_cref::aacenc_channel_map as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

/// Every mode of the table, the special values and a range of invalid ones.
fn all_modes() -> Vec<ChannelMode> {
    let mut m: Vec<ChannelMode> = (-3..=40).collect();
    m.extend([MODE_212, 127, 129, 255, 256, i32::MAX, i32::MIN]);
    m
}

const ORDERS: [i32; 5] = [CH_ORDER_MPEG, CH_ORDER_WAV, CH_ORDER_WG4, 3, -1];

const VALID_MODES: [ChannelMode; 12] = [
    MODE_1,
    MODE_2,
    MODE_1_2,
    MODE_1_2_1,
    MODE_1_2_2,
    MODE_1_2_2_1,
    MODE_1_2_2_2_1,
    MODE_6_1,
    MODE_7_1_BACK,
    MODE_7_1_TOP_FRONT,
    MODE_7_1_REAR_SURROUND,
    MODE_7_1_FRONT_CENTER,
];

fn c_mapping(mode: i32, co: i32, prefill: &ChannelMapping) -> (i32, ChannelMapping) {
    let mut cm = *prefill;
    let err = unsafe { c::cref_channel_map_InitChannelMapping(mode, co, &mut cm as *mut _ as *mut _) };
    (err, cm)
}

#[test]
fn get_mono_stereo_mode_and_configuration() {
    for mode in all_modes() {
        let want = unsafe { c::cref_channel_map_GetMonoStereoMode(mode) };
        assert_eq!(fdk_aac_enc_get_mono_stereo_mode(mode), want, "mode {mode}");

        let mut out = [0i32; 4];
        let idx = unsafe { c::cref_channel_map_GetChannelModeConfiguration(mode, out.as_mut_ptr()) };
        match fdk_aac_enc_get_channel_mode_configuration(mode) {
            None => assert_eq!(idx, -1, "mode {mode}"),
            Some(cfg) => {
                let rust_idx = (cfg as *const ChannelModeConfigTab as usize
                    - CHANNEL_MODE_CONFIG.as_ptr() as usize)
                    / core::mem::size_of::<ChannelModeConfigTab>();
                assert_eq!(rust_idx as i32, idx, "mode {mode}");
                assert_eq!(
                    [cfg.enc_mode, cfg.n_channels, cfg.n_channels_eff, cfg.n_elements],
                    out,
                    "mode {mode}"
                );
            }
        }
    }
}

#[test]
fn channel_mode_config_table() {
    // Every entry of the table is reached through its mode (the modes are unique).
    for (i, e) in CHANNEL_MODE_CONFIG.iter().enumerate() {
        let mut out = [0i32; 4];
        let idx = unsafe { c::cref_channel_map_GetChannelModeConfiguration(e.enc_mode, out.as_mut_ptr()) };
        assert_eq!(idx, i as i32);
        assert_eq!([e.enc_mode, e.n_channels, e.n_channels_eff, e.n_elements], out);
    }
}

#[test]
fn determine_encoder_mode() {
    // Only the modes with a defined reference result are compared: MODE_UNKNOWN and the modes
    // of the table.
    let mut modes = vec![MODE_UNKNOWN];
    modes.extend(VALID_MODES);
    for mode in modes {
        for n in -2..=12 {
            let mut cm = mode;
            let want = unsafe { c::cref_channel_map_DetermineEncoderMode(&mut cm, n) };
            let mut rm = mode;
            let got = fdk_aac_enc_determine_encoder_mode(&mut rm, n);
            assert_eq!((got, rm), (want, cm), "mode {mode} n {n}");
        }
    }
    // Port only: an invalid mode is rejected (deviation, see the function documentation).
    for mode in [-1, 8, 9, 10, 13, 15, 32, MODE_212] {
        let mut m = mode;
        assert_ne!(fdk_aac_enc_determine_encoder_mode(&mut m, 2), 0);
        assert_eq!(m, mode);
    }
}

#[test]
fn init_channel_mapping() {
    let mut rng = Rng::new(0x636d_6170);
    for mode in all_modes() {
        for co in ORDERS {
            // Prefill with garbage: C clears the whole structure.
            let mut prefill = ChannelMapping::default();
            prefill.n_channels = rng.i32();
            prefill.el_info[7].channel_index[1] = rng.i32();
            let (want_err, want) = c_mapping(mode, co, &prefill);
            let mut got = prefill;
            let got_err = fdk_aac_enc_init_channel_mapping(mode, co, &mut got);
            assert_eq!(got_err, want_err, "mode {mode} co {co}");
            assert_eq!(got, want, "mode {mode} co {co}");
        }
    }
}

fn random_element_bits(rng: &mut Rng) -> [ElementBits; 8] {
    let mut eb = [ElementBits::default(); 8];
    for e in eb.iter_mut() {
        *e = ElementBits {
            ch_bitrate_el: rng.i32(),
            max_bits_el: rng.i32(),
            bit_res_level_el: rng.i32(),
            max_bit_res_bits_el: rng.i32(),
            relative_bits_el: rng.i32(),
        };
    }
    eb
}

fn check_element_bits(rng: &mut Rng, cm: &ChannelMapping, bitrate: i32, avg: i32, max_ch: i32) {
    let eb0 = random_element_bits(rng);
    let mut want_eb = eb0;
    let mut cmc = *cm;
    let want = unsafe {
        c::cref_channel_map_InitElementBits(
            want_eb.as_mut_ptr() as *mut _,
            &mut cmc as *mut _ as *mut _,
            bitrate,
            avg,
            max_ch,
        )
    };
    let mut qc = QcState { element_bits: eb0.to_vec(), ..Default::default() };
    let got = fdk_aac_enc_init_element_bits(&mut qc, cm, bitrate, avg, max_ch);
    assert_eq!(got, want, "mode {} br {bitrate} avg {avg} max {max_ch}", cm.enc_mode);
    assert_eq!(qc.element_bits, want_eb.to_vec(), "mode {} br {bitrate} avg {avg} max {max_ch}", cm.enc_mode);

    // A shorter vector: the entries it has get the same values, nothing panics.
    let n = (rng.next_u32() % 9) as usize;
    let mut qc = QcState { element_bits: eb0[..n].to_vec(), ..Default::default() };
    let got = fdk_aac_enc_init_element_bits(&mut qc, cm, bitrate, avg, max_ch);
    assert_eq!(got, want);
    assert_eq!(qc.element_bits, want_eb[..n].to_vec());
}

#[test]
fn init_element_bits() {
    let mut rng = Rng::new(0x656c_6269);
    let sample_rates = [8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
    let frame_lengths = [120, 128, 240, 256, 480, 512, 960, 1024];
    for mode in all_modes() {
        for co in [CH_ORDER_MPEG, CH_ORDER_WAV, CH_ORDER_WG4] {
            let mut cm = ChannelMapping::default();
            fdk_aac_enc_init_channel_mapping(mode, co, &mut cm);
            // Realistic configurations.
            for &br in &[0, 1, 8000, 16000, 32000, 64000, 96000, 128000, 192000, 256000, 320000, 512000, 800000, 6_000_000] {
                for _ in 0..4 {
                    let sr = sample_rates[(rng.next_u32() as usize) % sample_rates.len()];
                    let fl = frame_lengths[(rng.next_u32() as usize) % frame_lengths.len()];
                    let avg = ((br as i64 * fl as i64) / sr as i64) as i32;
                    let max_ch = [6144, 6144 * 2, 768, 6144 / 2][(rng.next_u32() % 4) as usize];
                    check_element_bits(&mut rng, &cm, br, avg, max_ch);
                }
            }
            // Random and edge-case parameters (including the shift by 32 for bitrateTot == -1).
            for _ in 0..200 {
                let br = rng.i32();
                let avg = rng.i32();
                let max_ch = rng.i32();
                check_element_bits(&mut rng, &cm, br, avg, max_ch);
            }
            for br in [-1, -2, i32::MIN, i32::MAX, 0, 1] {
                for v in [0, 1, -1, i32::MAX, i32::MIN, 6144] {
                    check_element_bits(&mut rng, &cm, br, v, 6144);
                    check_element_bits(&mut rng, &cm, br, 6144, v);
                    check_element_bits(&mut rng, &cm, br, v, v);
                }
            }
        }
    }
    // Mappings with random content (relative bits, modes).
    for _ in 0..2000 {
        let mut cm = ChannelMapping::default();
        let mode = VALID_MODES[(rng.next_u32() % 12) as usize];
        fdk_aac_enc_init_channel_mapping(mode, CH_ORDER_MPEG, &mut cm);
        for e in cm.el_info.iter_mut() {
            e.relative_bits = rng.i32();
        }
        if rng.next_u32() % 8 == 0 {
            cm.enc_mode = rng.i32();
        }
        let (br, avg, max_ch) = (rng.i32(), rng.i32(), rng.i32());
        check_element_bits(&mut rng, &cm, br, avg, max_ch);
    }
}

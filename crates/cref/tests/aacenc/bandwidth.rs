// Differential tests of fdk_aac_rs_aacenc::bandwidth against the C++ reference.

use fdk_aac_rs_aacenc::aacenc::CH_ORDER_MPEG;
use fdk_aac_rs_aacenc::bandwidth::*;
use fdk_aac_rs_aacenc::channel_map::fdk_aac_enc_init_channel_mapping;
use fdk_aac_rs_aacenc::qc_data::ChannelMapping;
use fdk_aac_rs_cref::aacenc_bandwidth as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

const SAMPLE_RATES: [i32; 16] =
    [0, 7350, 8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000, 192000, 44099];
const FRAME_LENGTHS: [i32; 12] = [0, 64, 120, 128, 240, 256, 480, 512, 768, 960, 1024, 2048];

#[test]
fn tables() {
    let rust: [Vec<i32>; 7] = [
        BAND_WIDTH_TABLE.iter().flat_map(|e| [e.chan_bit_rate, e.band_width_mono, e.band_width2_and_more_chan]).collect(),
        BAND_WIDTH_TABLE_LD_22050.iter().flat_map(|e| [e.chan_bit_rate, e.band_width_mono, e.band_width2_and_more_chan]).collect(),
        BAND_WIDTH_TABLE_LD_24000.iter().flat_map(|e| [e.chan_bit_rate, e.band_width_mono, e.band_width2_and_more_chan]).collect(),
        BAND_WIDTH_TABLE_LD_32000.iter().flat_map(|e| [e.chan_bit_rate, e.band_width_mono, e.band_width2_and_more_chan]).collect(),
        BAND_WIDTH_TABLE_LD_44100.iter().flat_map(|e| [e.chan_bit_rate, e.band_width_mono, e.band_width2_and_more_chan]).collect(),
        BAND_WIDTH_TABLE_LD_48000.iter().flat_map(|e| [e.chan_bit_rate, e.band_width_mono, e.band_width2_and_more_chan]).collect(),
        BAND_WIDTH_TABLE_VBR.iter().flat_map(|e| [e.bitrate_mode, e.band_width_mono, e.band_width2_and_more_chan]).collect(),
    ];
    for (i, r) in rust.iter().enumerate() {
        let mut out = [0i32; 64];
        let n = unsafe { c::cref_bandwidth_table(i as i32, out.as_mut_ptr()) } as usize;
        assert_eq!(&out[..n], &r[..], "table {i}");
    }
}

#[test]
fn get_bandwidth_entry_matches() {
    let mut rng = Rng::new(0x6277_656e);
    for &fl in &FRAME_LENGTHS {
        for &sr in &SAMPLE_RATES {
            let mut rates: Vec<i32> = (-1000..700_000).step_by(997).collect();
            rates.extend([i32::MIN, i32::MAX, -1, 0, 1]);
            // Around every table boundary.
            for t in [&BAND_WIDTH_TABLE[..], &BAND_WIDTH_TABLE_LD_22050, &BAND_WIDTH_TABLE_LD_24000,
                      &BAND_WIDTH_TABLE_LD_32000, &BAND_WIDTH_TABLE_LD_44100, &BAND_WIDTH_TABLE_LD_48000] {
                for e in t {
                    rates.extend([e.chan_bit_rate - 1, e.chan_bit_rate, e.chan_bit_rate + 1]);
                }
            }
            for _ in 0..200 {
                rates.push(rng.i32());
            }
            for &br in &rates {
                for entry in [0, 1, 2, -1] {
                    let want = unsafe { c::cref_bandwidth_GetBandwidthEntry(fl, sr, br, entry) };
                    assert_eq!(get_bandwidth_entry(fl, sr, br, entry), want, "fl {fl} sr {sr} br {br} entry {entry}");
                }
            }
        }
    }
}

fn check(prop: i32, bitrate: i32, br_mode: i32, sr: i32, fl: i32, cm: &ChannelMapping, mode: ChannelMode) {
    let mut cmc = *cm;
    let mut want_bw = 0x5a5a_5a5a;
    let want = unsafe {
        c::cref_bandwidth_DetermineBandWidth(prop, bitrate, br_mode, sr, fl, &mut cmc as *mut _ as *mut _, mode, &mut want_bw)
    };
    let mut got_bw = 0x5a5a_5a5a;
    let got = fdk_aac_enc_determine_band_width(prop, bitrate, br_mode, sr, fl, cm, mode, &mut got_bw);
    assert_eq!(
        (got, got_bw),
        (want, want_bw),
        "prop {prop} br {bitrate} mode {br_mode} sr {sr} fl {fl} cm.nChannelsEff {} encMode {mode}",
        cm.n_channels_eff
    );
}

#[test]
fn determine_band_width() {
    let mut rng = Rng::new(0x6277_6474);
    let mut modes: Vec<ChannelMode> = (-1..=16).collect();
    modes.extend([MODE_7_1_REAR_SURROUND, MODE_7_1_FRONT_CENTER, MODE_212]);
    let bitrates = [0, 6000, 8000, 12000, 16000, 24000, 32000, 48000, 64000, 96000, 128000, 160000, 192000, 256000, 320000, 512000, 800000, 2_000_000];
    for &mode in &modes {
        let mut cm = ChannelMapping::default();
        fdk_aac_enc_init_channel_mapping(mode, CH_ORDER_MPEG, &mut cm);
        if cm.n_channels_eff == 0 {
            // Invalid modes: a mapping of another mode (no defined x86 reference result
            // otherwise).
            fdk_aac_enc_init_channel_mapping(MODE_2, CH_ORDER_MPEG, &mut cm);
        }
        for br_mode in -2..=9 {
            for &sr in &SAMPLE_RATES {
                for &fl in &FRAME_LENGTHS {
                    for &br in &bitrates {
                        for prop in [0, 3000, 15000, 20000, 25000] {
                            check(prop, br, br_mode, sr, fl, &cm, mode);
                        }
                    }
                }
            }
        }
    }
    // Random parameters.
    for _ in 0..200_000 {
        let mode = modes[(rng.next_u32() as usize) % modes.len()];
        let mut cm = ChannelMapping::default();
        let cm_mode = if rng.next_u32() % 4 == 0 { modes[(rng.next_u32() as usize) % modes.len()] } else { mode };
        fdk_aac_enc_init_channel_mapping(cm_mode, CH_ORDER_MPEG, &mut cm);
        if cm.n_channels_eff == 0 {
            cm.n_channels_eff = rng.range(1, 8);
        }
        if rng.next_u32() % 16 == 0 {
            cm.n_channels_eff = rng.i32();
            // x86 division by zero: no defined reference result; see the function documentation.
            if cm.n_channels_eff == 0 || cm.n_channels_eff == -1 {
                cm.n_channels_eff = 3;
            }
        }
        let prop = if rng.next_u32() % 2 == 0 { 0 } else { rng.i32() };
        let br = if rng.next_u32() % 2 == 0 { rng.range(0, 1_000_000) } else { rng.i32() };
        let br_mode = rng.range(-2, 9);
        let sr = if rng.next_u32() % 4 == 0 { rng.i32() } else { SAMPLE_RATES[(rng.next_u32() as usize) % SAMPLE_RATES.len()] };
        let fl = FRAME_LENGTHS[(rng.next_u32() as usize) % FRAME_LENGTHS.len()];
        check(prop, br, br_mode, sr, fl, &cm, mode);
    }
    // arm64 only: the divisions without a defined result on x86 (sdiv gives 0 and INT_MIN).
    if matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::Aarch64) {
        let mut cm = ChannelMapping::default();
        for eff in [0, -1] {
            cm.n_channels_eff = eff;
            for br in [i32::MIN, -5, 0, 64000] {
                for fl in [512, 1024] {
                    check(0, br, 0, 48000, fl, &cm, MODE_2);
                }
            }
        }
    }
}

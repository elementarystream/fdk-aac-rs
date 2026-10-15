// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/bandwidth.h, libAACenc/src/bandwidth.cpp.

//! Port of `bandwidth.h` and `bandwidth.cpp`: bandwidth expert.

use crate::aacenc::*;
use crate::qc_data::ChannelMapping;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::{f_min, f_mult, FixpDbl};
use fdk_aac_rs_fdk::fixpoint_math::f_div_norm;
use fdk_aac_rs_fdk::scale::scale_value;

/// `BANDWIDTH_TAB`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BandwidthTab {
    pub chan_bit_rate: i32,
    pub band_width_mono: i32,
    pub band_width2_and_more_chan: i32,
}

/// `BANDWIDTH_TAB_VBR`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BandwidthTabVbr {
    pub bitrate_mode: AacencBitrateMode,
    pub band_width_mono: i32,
    pub band_width2_and_more_chan: i32,
}

const fn bw(
    chan_bit_rate: i32,
    band_width_mono: i32,
    band_width2_and_more_chan: i32,
) -> BandwidthTab {
    BandwidthTab { chan_bit_rate, band_width_mono, band_width2_and_more_chan }
}

const fn bwv(
    bitrate_mode: AacencBitrateMode,
    band_width_mono: i32,
    band_width2_and_more_chan: i32,
) -> BandwidthTabVbr {
    BandwidthTabVbr { bitrate_mode, band_width_mono, band_width2_and_more_chan }
}

/// `bandWidthTable`
///
/// Dumped with `scripts/dump_table.py libAACenc/src/bandwidth.cpp bandWidthTable INT --per-line 3`.
pub static BAND_WIDTH_TABLE: [BandwidthTab; 9] = [
    bw(0, 3700, 5000),
    bw(12000, 5000, 6400),
    bw(20000, 6900, 9640),
    bw(28000, 9600, 13050),
    bw(40000, 12060, 14260),
    bw(56000, 13950, 15500),
    bw(72000, 14200, 16120),
    bw(96000, 17000, 17000),
    bw(576001, 17000, 17000),
];

/// `bandWidthTable_LD_22050`
///
/// Dumped with `scripts/dump_table.py libAACenc/src/bandwidth.cpp bandWidthTable_LD_22050 INT --per-line 3`.
pub static BAND_WIDTH_TABLE_LD_22050: [BandwidthTab; 8] = [
    bw(8000, 2000, 2400),
    bw(12000, 2500, 2700),
    bw(16000, 3300, 3100),
    bw(24000, 6250, 7200),
    bw(32000, 9200, 10500),
    bw(40000, 16000, 16000),
    bw(48000, 16000, 16000),
    bw(282241, 16000, 16000),
];

/// `bandWidthTable_LD_24000`
///
/// Dumped with `scripts/dump_table.py libAACenc/src/bandwidth.cpp bandWidthTable_LD_24000 INT --per-line 3`.
pub static BAND_WIDTH_TABLE_LD_24000: [BandwidthTab; 9] = [
    bw(8000, 2000, 2000),
    bw(12000, 2000, 2300),
    bw(16000, 2200, 2500),
    bw(24000, 5650, 7200),
    bw(32000, 11600, 12000),
    bw(40000, 12000, 16000),
    bw(48000, 16000, 16000),
    bw(64000, 16000, 16000),
    bw(307201, 16000, 16000),
];

/// `bandWidthTable_LD_32000`
///
/// Dumped with `scripts/dump_table.py libAACenc/src/bandwidth.cpp bandWidthTable_LD_32000 INT --per-line 3`.
pub static BAND_WIDTH_TABLE_LD_32000: [BandwidthTab; 9] = [
    bw(8000, 2000, 2000),
    bw(12000, 2000, 2000),
    bw(24000, 4250, 7200),
    bw(32000, 8400, 9000),
    bw(40000, 9400, 11300),
    bw(48000, 11900, 14700),
    bw(64000, 14800, 16000),
    bw(76000, 16000, 16000),
    bw(409601, 16000, 16000),
];

/// `bandWidthTable_LD_44100`
///
/// Dumped with `scripts/dump_table.py libAACenc/src/bandwidth.cpp bandWidthTable_LD_44100 INT --per-line 3`.
pub static BAND_WIDTH_TABLE_LD_44100: [BandwidthTab; 11] = [
    bw(8000, 2000, 2000),
    bw(24000, 2000, 2000),
    bw(32000, 4400, 5700),
    bw(40000, 7400, 8800),
    bw(48000, 9000, 10700),
    bw(56000, 11000, 12900),
    bw(64000, 14400, 15500),
    bw(80000, 16000, 16200),
    bw(96000, 16500, 16000),
    bw(128000, 16000, 16000),
    bw(564481, 16000, 16000),
];

/// `bandWidthTable_LD_48000`
///
/// Dumped with `scripts/dump_table.py libAACenc/src/bandwidth.cpp bandWidthTable_LD_48000 INT --per-line 3`.
pub static BAND_WIDTH_TABLE_LD_48000: [BandwidthTab; 11] = [
    bw(8000, 2000, 2000),
    bw(24000, 2000, 2000),
    bw(32000, 4400, 5700),
    bw(40000, 7400, 8800),
    bw(48000, 9000, 10700),
    bw(56000, 11000, 12800),
    bw(64000, 14300, 15400),
    bw(80000, 16000, 16200),
    bw(96000, 16500, 16000),
    bw(128000, 16000, 16000),
    bw(614401, 16000, 16000),
];

/// `bandWidthTableVBR`
///
/// Dumped with `scripts/dump_table.py libAACenc/src/bandwidth.cpp bandWidthTableVBR INT --per-line 3`.
pub static BAND_WIDTH_TABLE_VBR: [BandwidthTabVbr; 8] = [
    bwv(0, 0, 0),
    bwv(1, 13000, 13000),
    bwv(2, 13000, 13000),
    bwv(3, 15750, 15750),
    bwv(4, 16500, 16500),
    bwv(5, 19293, 19293),
    bwv(7, 0, 0),
    bwv(6, 0, 0),
];

/// `GetBandwidthEntry(const INT frameLength, const INT sampleRate, const INT chanBitRate,
/// const INT entryNo)` (file-static): the bandwidth for the channel bitrate from the table of
/// the frame length (and sample rate), -1 if there is none.
pub fn get_bandwidth_entry(
    frame_length: i32,
    sample_rate: i32,
    chan_bit_rate: i32,
    entry_no: i32,
) -> i32 {
    let mut bandwidth = -1;

    let p_bw_tab: Option<&[BandwidthTab]> = match frame_length {
        960 | 1024 => Some(&BAND_WIDTH_TABLE),
        120 | 128 | 240 | 256 | 480 | 512 => match sample_rate {
            8000 | 11025 | 12000 | 16000 | 22050 => Some(&BAND_WIDTH_TABLE_LD_22050),
            24000 => Some(&BAND_WIDTH_TABLE_LD_24000),
            32000 => Some(&BAND_WIDTH_TABLE_LD_32000),
            44100 => Some(&BAND_WIDTH_TABLE_LD_44100),
            48000 | 64000 | 88200 | 96000 => Some(&BAND_WIDTH_TABLE_LD_48000),
            _ => None,
        },
        _ => None,
    };

    if let Some(p_bw_tab) = p_bw_tab {
        let bw_tab_size = p_bw_tab.len();
        for i in 0..bw_tab_size - 1 {
            if chan_bit_rate >= p_bw_tab[i].chan_bit_rate
                && chan_bit_rate < p_bw_tab[i + 1].chan_bit_rate
            {
                let pick = |e: &BandwidthTab| {
                    if entry_no == 0 {
                        e.band_width_mono
                    } else {
                        e.band_width2_and_more_chan
                    }
                };
                match frame_length {
                    960 | 1024 => {
                        bandwidth = pick(&p_bw_tab[i]);
                    }
                    120 | 128 | 240 | 256 | 480 | 512 => {
                        let mut q_res = 0;
                        let start_bw = pick(&p_bw_tab[i]);
                        let end_bw = pick(&p_bw_tab[i + 1]);
                        let start_br = p_bw_tab[i].chan_bit_rate;
                        let end_br = p_bw_tab[i + 1].chan_bit_rate;

                        let bw_fac_fix: FixpDbl =
                            f_div_norm(chan_bit_rate - start_br, end_br - start_br, &mut q_res);
                        bandwidth =
                            scale_value(f_mult(bw_fac_fix, end_bw - start_bw), q_res) + start_bw;
                    }
                    _ => bandwidth = -1,
                }
                break;
            } // within bitrate range
        }
    } // pBwTab!=NULL

    bandwidth
}

/// `FDKaacEnc_DetermineBandWidth(const INT proposedBandWidth, const INT bitrate,
/// const AACENC_BITRATE_MODE bitrateMode, const INT sampleRate, const INT frameLength,
/// const CHANNEL_MAPPING *const cm, const CHANNEL_MODE encoderMode, INT *const bandWidth)`
///
/// `bitrate / cm->nChannelsEff` is computed as the arm64 build does (`sdiv`: 0 for a divisor
/// of 0, `INT_MIN` for `INT_MIN / -1`) on every variant.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_determine_band_width(
    proposed_band_width: i32,
    bitrate: i32,
    bitrate_mode: AacencBitrateMode,
    sample_rate: i32,
    frame_length: i32,
    cm: &ChannelMapping,
    encoder_mode: ChannelMode,
    band_width: &mut i32,
) -> AacEncoderError {
    let mut error_status = AAC_ENC_OK;
    // sdiv (arm64): x / 0 == 0, INT_MIN / -1 == INT_MIN.
    let chan_bit_rate =
        if cm.n_channels_eff == 0 { 0 } else { bitrate.wrapping_div(cm.n_channels_eff) };

    match bitrate_mode {
        AACENC_BR_MODE_VBR_1 | AACENC_BR_MODE_VBR_2 | AACENC_BR_MODE_VBR_3
        | AACENC_BR_MODE_VBR_4 | AACENC_BR_MODE_VBR_5 => {
            if proposed_band_width != 0 {
                // use given bw
                *band_width = proposed_band_width;
            } else {
                // take bw from table
                match encoder_mode {
                    MODE_1 => {
                        *band_width = BAND_WIDTH_TABLE_VBR[bitrate_mode as usize].band_width_mono;
                    }
                    MODE_2
                    | MODE_1_2
                    | MODE_1_2_1
                    | MODE_1_2_2
                    | MODE_1_2_2_1
                    | MODE_6_1
                    | MODE_1_2_2_2_1
                    | MODE_7_1_REAR_SURROUND
                    | MODE_7_1_FRONT_CENTER
                    | MODE_7_1_BACK
                    | MODE_7_1_TOP_FRONT => {
                        *band_width =
                            BAND_WIDTH_TABLE_VBR[bitrate_mode as usize].band_width2_and_more_chan;
                    }
                    _ => return AAC_ENC_UNSUPPORTED_CHANNELCONFIG,
                }
            }
        }
        AACENC_BR_MODE_CBR | AACENC_BR_MODE_SFR | AACENC_BR_MODE_FF => {
            // bandwidth limiting
            if proposed_band_width != 0 {
                *band_width = f_min(proposed_band_width, f_min(20000, sample_rate >> 1));
            } else {
                // search reasonable bandwidth
                let entry_no = match encoder_mode {
                    MODE_1 => 0, // mono: use mono bandwidth settings
                    // stereo, sce + cpe, sce + cpe + sce, sce + cpe + cpe,
                    // (5.1) sce + cpe + cpe + lfe, ...
                    MODE_2
                    | MODE_1_2
                    | MODE_1_2_1
                    | MODE_1_2_2
                    | MODE_1_2_2_1
                    | MODE_6_1
                    | MODE_1_2_2_2_1
                    | MODE_7_1_REAR_SURROUND
                    | MODE_7_1_FRONT_CENTER
                    | MODE_7_1_BACK
                    | MODE_7_1_TOP_FRONT => {
                        1 // use stereo bandwidth settings
                    }
                    _ => return AAC_ENC_UNSUPPORTED_CHANNELCONFIG,
                };

                *band_width =
                    get_bandwidth_entry(frame_length, sample_rate, chan_bit_rate, entry_no);

                if *band_width == -1 {
                    match frame_length {
                        120 | 128 | 240 | 256 => *band_width = 16000,
                        _ => error_status = AAC_ENC_INVALID_CHANNEL_BITRATE,
                    }
                }
            }
        }
        _ => {
            *band_width = 0;
            return AAC_ENC_UNSUPPORTED_BITRATE_MODE;
        }
    }

    *band_width = f_min(*band_width, sample_rate / 2);

    error_status
}

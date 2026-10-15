// Differential tests of fdk_aac_rs_aacenc::aacenc_lib: SBR and PS reconfiguration.
//
// The SBR encoder's state (mh_det, the frequency scale, the envelope arrays) depends on the
// history of the configuration. Here the SBR encoders
// (HE-AAC, HE-AAC v2, ELD with SBR, downsampled SBR) are reconfigured mid-stream: bitrate steps
// up and down (other SBR tunings: start/stop frequencies, number of bands), channel mode and
// SBR ratio changes, with and without AACENC_INIT_STATES, on music and synthetic signals. Every
// output byte and the state exposed by the API are compared after every call (Pair of
// aacenc_lib.rs).
//
// Values without a defined reference value after sbrEncoder_Init are 0 in the port; these runs
// include every SBR configuration, so a dependence of the bitstream on them would show as a
// difference.

use crate::aacenc_lib::{signal, EncCall, Pair};
use crate::adj_thr_enc::Signal;
use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_cref::testutil::Rng;

#[test]
fn sbr_reconfiguration() {
    let mut rng = Rng::new(36);
    let mut frames = 0;
    let mut bytes = 0;
    // (aot, sbr mode, channel mode, rates)
    let cases: [(u32, Option<u32>, u32, &[i32]); 6] = [
        (5, None, 2, &[22050, 32000, 44100, 48000]),
        (5, None, 1, &[16000, 24000, 44100, 48000]),
        (29, None, 2, &[22050, 32000, 44100, 48000]),
        (39, Some(1), 2, &[32000, 44100, 48000]),
        (39, Some(1), 1, &[16000, 24000, 48000]),
        (132, None, 2, &[32000, 44100]),
    ];
    for (k, &(aot, sbr, mode, rates)) in cases.iter().enumerate() {
        for &rate in rates {
            for seq in 0..3 {
                // No defined x86 reference result (x86 division by zero: the port gives the
                // arm64 result). Not run on x86.
                if seq == 2 && aot == 39 && cfg!(any(target_arch = "x86_64", target_arch = "x86")) {
                    continue;
                }
                let what = format!("sbr aot {aot} mode {mode} rate {rate} sequence {seq}");
                let ch = if mode == 1 { 1 } else { 2 };
                let Some(mut p) = Pair::open(0, 2, &what) else { continue };
                p.set(AACENC_AOT, aot);
                if let Some(s) = sbr {
                    p.set(AACENC_SBR_MODE, s);
                }
                p.set(AACENC_SAMPLERATE, rate as u32);
                p.set(AACENC_CHANNELMODE, mode);
                if p.set(AACENC_BITRATE, 16000) != AACENC_OK || !p.ch_set {
                    continue;
                }
                if aot == 39 && rng.range(0, 1) == 0 {
                    p.set(AACENC_SBR_RATIO, 2);
                }
                p.init();
                let fl = p.info("frame length").frame_length.max(1) as usize;
                let sig = [Signal::Music(0), Signal::Music(1), Signal::Sweep, Signal::Clicks][(k + seq) % 4];
                let pcm = signal(sig, rate, ch, fl * 40);
                // bitrate steps (up and down), occasionally with a state reset or other
                // parameters
                let steps: Vec<u32> = match seq {
                    0 => vec![16000, 24000, 32000, 48000, 64000, 96000, 128000, 160000],
                    1 => vec![160000, 96000, 48000, 24000, 12000, 64000, 20000, 128000],
                    _ => (0..8).map(|_| rng.range(8000, 200000) as u32).collect(),
                };
                for f in 0..40 {
                    if f % 5 == 0 && f / 5 < steps.len() {
                        p.set(AACENC_BITRATE, steps[f / 5]);
                        match rng.range(0, 6) {
                            0 => {
                                p.set(
                                    AACENC_CONTROL_STATE,
                                    AACENC_INIT_CONFIG | AACENC_INIT_STATES | AACENC_INIT_TRANSPORT,
                                );
                            }
                            1 => {
                                p.set(AACENC_BITRATEMODE, rng.range(0, 5) as u32);
                            }
                            2 if aot == 39 => {
                                p.set(AACENC_SBR_RATIO, rng.range(1, 2) as u32);
                            }
                            3 => {
                                p.set(AACENC_HEADER_PERIOD, rng.range(0, 12) as u32);
                            }
                            _ => {}
                        }
                    }
                    let (e, o) = p.encode(&EncCall::pcm(pcm[f * fl * ch..(f + 1) * fl * ch].to_vec(), 20480));
                    frames += 1;
                    bytes += o.num_out_bytes.max(0) as usize;
                    if e != AACENC_OK && e != AACENC_ENCODE_ERROR && e != AACENC_INVALID_CONFIG {
                        break;
                    }
                }
            }
        }
    }
    eprintln!("sbr_reconfiguration: {frames} encode calls, {bytes} output bytes");
    assert!(bytes > 0);
}

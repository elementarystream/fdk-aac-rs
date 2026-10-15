// Differential tests of fdk_aac_rs_sacenc::sacenc_onsetdetect against the C++ reference.
//
// Initializations with the configurations of the encoder (every frame length in time slots,
// the lower bounds of 1725 Hz at every sampling rate, 32 or 64 bands) and invalid ones
// (including the re-initialization quirk); sequences of frames of hybrid data with level jumps
// (transients) and silence, random previous transient positions and data scales, with the
// detected positions and the whole state (including the energy history) compared after every
// call.

use fdk_aac_rs_cref::sacenc_sacenc_onsetdetect as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::FixpDpk;
use fdk_aac_rs_sacenc::sacenc_onsetdetect as r;
use fdk_aac_rs_sacenc::sacenc_onsetdetect::{OnsetDetect, OnsetDetectConfig};

use super::sacenc_vectorfunctions::flat;

const N_HIST: usize = 32;

fn check(h: *mut core::ffi::c_void, o: &OnsetDetect, what: &str) {
    let mut v = [0i32; 6];
    let mut hist = [0i32; N_HIST];
    let mut sc = [0i8; N_HIST];
    unsafe { c::cref_sacenc_onset_get(h, v.as_mut_ptr(), hist.as_mut_ptr(), sc.as_mut_ptr(), N_HIST as i32) };
    let rv = [
        o.max_time_slots,
        o.min_transient_distance,
        o.avg_energy_distance,
        o.lower_bound_onset_detection,
        o.upper_bound_onset_detection,
        o.avg_energy_distance_scale as i32,
    ];
    assert_eq!(v, rv, "{what}: fields");
    assert_eq!(&hist[..], &o.p_energy_hist_fdk[..], "{what}: energy history");
    assert_eq!(&sc[..], &o.p_energy_hist_scale[..], "{what}: scales");
}

/// Hybrid data of a frame: `nts` slots of 64 bands, with a level per slot (jumps, decays,
/// silence).
pub fn hybrid_frame(rng: &mut Rng, nts: usize) -> Vec<Vec<FixpDpk>> {
    let mut level = rng.range(0, 30) as u32;
    let kind = rng.range(0, 3);
    (0..nts)
        .map(|ts| {
            match kind {
                0 => {
                    if rng.range(0, 7) == 0 {
                        level = rng.range(0, 30) as u32;
                    }
                }
                1 => level = (level + 1).min(31),
                2 => {
                    if ts == nts / 2 {
                        level = level.saturating_sub(12);
                    }
                }
                _ => level = 31,
            }
            (0..64)
                .map(|_| {
                    if level >= 31 {
                        FixpDpk::new(0, 0)
                    } else {
                        FixpDpk::new((rng.next_u32() as i32) >> (level + 1), (rng.next_u32() as i32) >> (level + 1))
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn sequences() {
    let mut rng = Rng::new(1);
    for seq in 0..400 {
        let h = unsafe { c::cref_sacenc_onset_open(16) };
        let mut o = None;
        r::fdk_sacenc_onset_detect_open(Some(&mut o), 16);
        let mut o = o.unwrap();
        check(h, &o, "open");

        let mut nts = rng.range(1, 16);
        let nhyb = [32, 64][rng.range(0, 1) as usize];
        let lower = if rng.range(0, 4) == 0 { rng.range(-2, 20) } else { [5, 6, 7, 8, 9, 10, 11, 13][rng.range(0, 7) as usize] };
        let upper = if rng.range(0, 6) == 0 { rng.range(-2, 70) } else { nhyb };
        let mut cfg = OnsetDetectConfig { max_time_slots: nts, lower_bound_onset_detection: lower, upper_bound_onset_detection: upper };
        let flags = if rng.range(0, 5) == 0 { 0 } else { 1 };
        let ec = unsafe { c::cref_sacenc_onset_init(h, nts, lower, upper, flags) };
        let er = r::fdk_sacenc_onset_detect_init(&mut o, &cfg, flags);
        assert_eq!(ec, er, "init seq {seq}");
        check(h, &o, &format!("init seq {seq}"));

        if rng.range(0, 3) == 0 {
            // A second initialization (may be refused, see the module documentation).
            nts = rng.range(1, 16);
            cfg.max_time_slots = nts;
            cfg.lower_bound_onset_detection = rng.range(0, 20);
            let ec = unsafe { c::cref_sacenc_onset_init(h, nts, cfg.lower_bound_onset_detection, upper, 1) };
            let er = r::fdk_sacenc_onset_detect_init(&mut o, &cfg, 1);
            assert_eq!(ec, er, "re-init seq {seq}");
            check(h, &o, &format!("re-init seq {seq}"));
            nts = o.max_time_slots;
        }

        let mut prev = -1;
        for fr in 0..25 {
            let n = if rng.range(0, 10) == 0 { rng.range(-1, 17) } else { nts };
            let rows = n.max(1) as usize;
            let data = hybrid_frame(&mut rng, rows);
            let scale = rng.range(0, 20);
            let (mut pc, mut pr) = ([7i32; 1], [7i32; 1]);
            let ec = unsafe {
                c::cref_sacenc_onset_apply(h, n, nhyb, flat(&data).as_ptr(), rows as i32, 64, scale, prev, pc.as_mut_ptr())
            };
            let er = r::fdk_sacenc_onset_detect_apply(&mut o, n, nhyb, &data, scale, prev, &mut pr);
            assert_eq!((ec, pc), (er, pr), "apply seq {seq} frame {fr}");
            check(h, &o, &format!("apply seq {seq} frame {fr}"));
            prev = if rng.range(0, 3) == 0 { rng.range(-1, 2 * nts) } else { pr[0] };

            let u = if rng.range(0, 10) == 0 { rng.range(0, 17) } else { nts };
            let ec = unsafe { c::cref_sacenc_onset_update(h, u) };
            let er = r::fdk_sacenc_onset_detect_update(&mut o, u);
            assert_eq!(ec, er, "update seq {seq} frame {fr}");
            check(h, &o, &format!("update seq {seq} frame {fr}"));
        }
        assert_eq!(unsafe { c::cref_sacenc_onset_close(h) }, 0);
    }
    assert_eq!(unsafe { c::cref_sacenc_onset_open_null() }, r::fdk_sacenc_onset_detect_open(None, 16));
}

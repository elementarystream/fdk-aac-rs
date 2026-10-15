// Differential tests of fdk_aac_rs_aacenc::chaosmeasure against the C++ reference.
//
// The chaos measure (public function and its static kernel) is compared on random spectra with
// extreme values and on MDCT spectra of real and synthetic PCM for every configuration; the
// output buffer is filled with the same values before each call, and compared completely.

use crate::band_nrg::{configs, frames, normalize};
use fdk_aac_rs_aacenc::chaosmeasure::*;
use fdk_aac_rs_cref::aacenc_chaosmeasure as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;

fn check(spec: &[i32], n: i32, fill: &[i32], what: &str) {
    let mut s = spec.to_vec();
    s.resize(1024, 0);
    let (mut a, mut b, mut d) = (fill.to_vec(), fill.to_vec(), fill.to_vec());
    fdk_aac_enc_calculate_chaos_measure(&s, n, &mut a);
    unsafe { c::cref_chaosmeasure_CalculateChaosMeasure(s.as_mut_ptr(), n, b.as_mut_ptr()) };
    assert_eq!(a, b, "{what}: CalculateChaosMeasure");
    let mut e = fill.to_vec();
    fdk_aac_enc_fdk_aac_enc_calculate_chaos_measure_peak_fast(&s, n, &mut e);
    unsafe { c::cref_chaosmeasure_CalculateChaosMeasurePeakFast(s.as_mut_ptr(), n, d.as_mut_ptr()) };
    assert_eq!(e, d, "{what}: CalculateChaosMeasurePeakFast");
}

#[test]
fn chaos_measure_random() {
    let mut r = Rng::new(61);
    for it in 0..3000 {
        let n = match r.next_u32() % 4 {
            0 => r.range(3, 8),
            _ => r.range(3, 1024),
        };
        let shift = r.next_u32() % 32;
        let spec: Vec<i32> = (0..1024)
            .map(|_| match r.next_u32() % 8 {
                0 => [0, MINVAL_DBL, MAXVAL_DBL, 1, -1, MINVAL_DBL + 1][(r.next_u32() % 6) as usize],
                1 => 0,
                2 => r.i32(),
                _ => (r.next_u32() as i32) >> shift,
            })
            .collect();
        let fill: Vec<i32> = (0..1024).map(|_| r.i32()).collect();
        check(&spec, n, &fill, &format!("iteration {it}"));
    }
}

#[test]
fn chaos_measure_realistic() {
    let mut count = 0;
    let fill = vec![0x1234_5678; 1024];
    for conf in configs() {
        for mut frame in frames(&conf, 6) {
            if frame.short {
                continue;
            }
            let n = conf.long.sfb_offset[conf.long.sfb_active as usize];
            let what = conf.name.to_string();
            for ch in 0..2 {
                check(&frame.spectrum[ch], n, &fill, &what);
            }
            normalize(&mut frame, &conf);
            for ch in 0..2 {
                check(&frame.spectrum[ch], n, &fill, &what);
                count += 1;
            }
        }
    }
    assert!(count > 200);
}

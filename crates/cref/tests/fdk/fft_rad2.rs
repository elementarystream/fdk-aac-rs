// Differential tests of fdk_aac_rs_fdk::fft_rad2 against the C++ reference.

use crate::fft::{cases, signal};
use fdk_aac_rs_cref::fdk_fft_rad2::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::fft_rad2::*;
use fdk_aac_rs_fdk::tools_rom::{SINE_TABLE_1024, SINE_TABLE_512};

/// `dit_fft` with the parameters of `fft()` (ldn 6 to 9, SineTable512) and with the other sine
/// table and table sizes, which change the twiddle step.
#[test]
fn dit_fft_lengths_and_tables() {
    let mut r = Rng::new(0xd17f);
    // (ldn, table, trigDataSize): table 0 = SineTable512, 1 = SineTable1024
    let configs: &[(i32, i32, i32)] = &[
        (3, 0, 512),
        (4, 0, 512),
        (5, 0, 512),
        (6, 0, 512),
        (7, 0, 512),
        (8, 0, 512),
        (9, 0, 512),
        (6, 1, 1024),
        (9, 1, 1024),
        (10, 1, 1024),
        (8, 0, 256),
        (7, 1, 512),
    ];
    for &(ldn, table, size) in configs {
        let n = 2usize << ldn;
        let trig: &[_] = if table == 0 { &SINE_TABLE_512 } else { &SINE_TABLE_1024 };
        for k in 0..cases(n) {
            let x = signal(&mut r, k, n);
            let (mut a, mut b) = (x.clone(), x.clone());
            dit_fft(&mut a, ldn, trig, size);
            unsafe { cref_fft_rad2_dit_fft(b.as_mut_ptr(), ldn, table, size) };
            assert_eq!(a, b, "dit_fft(ldn {ldn}, table {table}, size {size}) case {k}");
        }
    }
}

/// The two implementations perform the same operations for ldn >= 3; with the arithmetic of the
/// selected variant they give the same values.
#[test]
fn dit_fft_arm_and_generic_agree() {
    let mut r = Rng::new(0xd180);
    for ldn in 3..=10 {
        let n = 2usize << ldn;
        for k in 0..cases(n) / 4 {
            let x = signal(&mut r, k, n);
            let (mut a, mut b) = (x.clone(), x.clone());
            dit_fft_arm(&mut a, ldn, &SINE_TABLE_1024, 1024);
            dit_fft_generic(&mut b, ldn, &SINE_TABLE_1024, 1024);
            assert_eq!(a, b, "ldn {ldn} case {k}");
        }
    }
}

/// The NEON implementation (used by `dit_fft` of the `aarch64` variant for SineTable512) against
/// the straight port of the ARM code, for every length it supports, on the edge cases and on
/// random input (10x the cases of the C comparison, as no C call is needed).
#[cfg(target_arch = "aarch64")]
#[test]
fn dit_fft_neon_equals_straight_port() {
    if fdk_aac_rs_fdk::ARCH != fdk_aac_rs_fdk::Arch::Aarch64 {
        return;
    }
    let mut r = Rng::new(0xd181);
    for ldn in 4..=10 {
        let n = 2usize << ldn;
        for k in 0..10 * cases(n) {
            let x = signal(&mut r, k, n);
            let (mut a, mut b) = (x.clone(), x.clone());
            dit_fft_neon(&mut a, ldn);
            dit_fft_arm(&mut b, ldn, &SINE_TABLE_512, 512);
            assert_eq!(a, b, "ldn {ldn} case {k}");
        }
    }
}

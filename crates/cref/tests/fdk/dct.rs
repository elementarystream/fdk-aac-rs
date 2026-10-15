// Differential tests of fdk_aac_rs_fdk::dct against the C++ reference.

use crate::fft::{signal, FFT_LENGTHS};
use fdk_aac_rs_cref::fdk_dct::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::archdef::{FixpStp, FixpWtp};
use fdk_aac_rs_fdk::dct::*;
use fdk_aac_rs_fdk::tools_rom::{
    SINE_TABLE_1024, SINE_TABLE_384, SINE_TABLE_480, SINE_TABLE_80, WINDOW_SLOPES,
};

/// The identifiers of `cref_dct_dct_getTables` for the port's tables.
fn table_ids(length: i32) -> [i32; 3] {
    let (twiddle, sin_twiddle, sin_step) = dct_get_tables(length);
    let window_id = |w: Option<&'static [FixpWtp]>| -> i32 {
        let Some(w) = w else { return -1 };
        for (r, row) in WINDOW_SLOPES[0].iter().enumerate() {
            for (k, e) in row.iter().enumerate() {
                if e.is_some_and(|e| e.as_ptr() == w.as_ptr()) {
                    return (r * 9 + k) as i32;
                }
            }
        }
        -2
    };
    let sine_id = |t: Option<&'static [FixpStp]>| -> i32 {
        let Some(t) = t else { return 0 };
        let p = t.as_ptr();
        if p == SINE_TABLE_1024.as_ptr() {
            1
        } else if p == SINE_TABLE_480.as_ptr() {
            2
        } else if p == SINE_TABLE_384.as_ptr() {
            3
        } else if p == SINE_TABLE_80.as_ptr() {
            4
        } else {
            -1
        }
    };
    [window_id(twiddle), sine_id(sin_twiddle), sin_step]
}

#[test]
fn dct_getTables() {
    for length in 0..2048 {
        let c = {
            let mut out = [0i32; 3];
            unsafe { cref_dct_dct_getTables(length, out.as_mut_ptr()) };
            out
        };
        // FDK_ASSERT(*sin_step > 0) is a debug_assert! in the port.
        if cfg!(debug_assertions) && c[2] <= 0 {
            continue;
        }
        assert_eq!(table_ids(length), c, "dct_getTables({length})");
    }
}

/// The transform lengths `L = 2 * M` with an FFT of length `M` and the tables of `L`
/// (`twiddle` only for the type IV transforms).
fn lengths(need_twiddle: bool) -> Vec<i32> {
    FFT_LENGTHS
        .iter()
        .map(|&m| 2 * m)
        .filter(|&l| {
            let (twiddle, sin_twiddle, _) = dct_get_tables(l);
            sin_twiddle.is_some() && (!need_twiddle || twiddle.is_some())
        })
        .collect()
}

fn cases(l: usize) -> usize {
    if l <= 128 {
        1500
    } else if l <= 512 {
        400
    } else {
        150
    }
}

type Rust2 = fn(&mut [i32], &mut [i32], i32, &mut i32);
type C2 = unsafe extern "C" fn(*mut i32, *mut i32, i32, *mut i32);

/// dct_II, dct_III, dst_III: output, exponent and the work buffer.
fn check_with_tmp(name: &str, seed: u64, rust: Rust2, c: C2) {
    let ls: Vec<i32> = lengths(false).into_iter().filter(|l| l % 4 == 0).collect();
    assert!(ls.len() >= 20, "{name}: {ls:?}");
    let mut r = Rng::new(seed);
    for &l in &ls {
        let n = l as usize;
        for k in 0..cases(n) {
            let x = signal(&mut r, k, n);
            let garbage: Vec<i32> = (0..n).map(|_| r.i32()).collect();
            let e0 = r.range(-20, 20);
            let (mut a, mut b) = (x.clone(), x.clone());
            let (mut ta, mut tb) = (garbage.clone(), garbage.clone());
            let (mut ea, mut eb) = (e0, e0);
            rust(&mut a, &mut ta, l, &mut ea);
            unsafe { c(b.as_mut_ptr(), tb.as_mut_ptr(), l, &mut eb) };
            assert_eq!(a, b, "{name}(L = {l}) case {k}");
            assert_eq!(ea, eb, "{name}(L = {l}) exponent case {k}");
            assert_eq!(ta, tb, "{name}(L = {l}) tmp case {k}");
        }
    }
}

#[test]
fn dct_II() {
    check_with_tmp("dct_II", 0xdc72, dct_ii, cref_dct_dct_II);
}

#[test]
fn dct_III() {
    check_with_tmp("dct_III", 0xdc73, dct_iii, cref_dct_dct_III);
}

#[test]
fn dst_III() {
    check_with_tmp("dst_III", 0xd573, dst_iii, cref_dct_dst_III);
}

type Rust4 = fn(&mut [i32], i32, &mut i32);
type C4 = unsafe extern "C" fn(*mut i32, i32, *mut i32);

fn check_iv(name: &str, seed: u64, rust: Rust4, c: C4) {
    let ls = lengths(true);
    assert!(ls.len() >= 20, "{name}: {ls:?}");
    let mut r = Rng::new(seed);
    for &l in &ls {
        let n = l as usize;
        for k in 0..cases(n) {
            let x = signal(&mut r, k, n);
            let e0 = r.range(-20, 20);
            let (mut a, mut b) = (x.clone(), x.clone());
            let (mut ea, mut eb) = (e0, e0);
            rust(&mut a, l, &mut ea);
            unsafe { c(b.as_mut_ptr(), l, &mut eb) };
            assert_eq!(a, b, "{name}(L = {l}) case {k}");
            assert_eq!(ea, eb, "{name}(L = {l}) exponent case {k}");
        }
    }
}

#[test]
fn dct_IV() {
    check_iv("dct_IV", 0xdc74, dct_iv, cref_dct_dct_IV);
}

#[test]
fn dst_IV() {
    check_iv("dst_IV", 0xd574, dst_iv, cref_dct_dst_IV);
}

/// The optimized dct_IV / dst_IV (per-pair twiddling, NEON blocks on aarch64) against the
/// straight ports, for every length with tables, on the edge cases and random input.
fn check_iv_against_ref(name: &str, seed: u64, fast: Rust4, straight: Rust4) {
    let mut r = Rng::new(seed);
    for &l in &lengths(true) {
        let n = l as usize;
        for k in 0..4 * cases(n) {
            let x = signal(&mut r, k, n);
            let e0 = r.range(-20, 20);
            let (mut a, mut b) = (x.clone(), x.clone());
            let (mut ea, mut eb) = (e0, e0);
            fast(&mut a, l, &mut ea);
            straight(&mut b, l, &mut eb);
            assert_eq!(a, b, "{name}(L = {l}) case {k}");
            assert_eq!(ea, eb, "{name}(L = {l}) exponent case {k}");
        }
    }
}

/// The optimized dct_II / dct_III / dst_III against the straight ports (output, exponent and
/// work buffer), for every length with tables, on the edge cases and random input.
fn check_with_tmp_against_ref(name: &str, seed: u64, fast: Rust2, straight: Rust2) {
    let ls: Vec<i32> = lengths(false).into_iter().filter(|l| l % 4 == 0).collect();
    let mut r = Rng::new(seed);
    for &l in &ls {
        let n = l as usize;
        for k in 0..4 * cases(n) {
            let x = signal(&mut r, k, n);
            let garbage: Vec<i32> = (0..n).map(|_| r.i32()).collect();
            let e0 = r.range(-20, 20);
            let (mut a, mut b) = (x.clone(), x.clone());
            let (mut ta, mut tb) = (garbage.clone(), garbage.clone());
            let (mut ea, mut eb) = (e0, e0);
            fast(&mut a, &mut ta, l, &mut ea);
            straight(&mut b, &mut tb, l, &mut eb);
            assert_eq!(a, b, "{name}(L = {l}) case {k}");
            assert_eq!(ea, eb, "{name}(L = {l}) exponent case {k}");
            assert_eq!(ta, tb, "{name}(L = {l}) tmp case {k}");
        }
    }
}

#[test]
fn dct_II_III_equal_straight_port() {
    check_with_tmp_against_ref("dct_II", 0xdc76, dct_ii, dct_ii_ref);
    check_with_tmp_against_ref("dct_III", 0xdc77, dct_iii, dct_iii_ref);
    check_with_tmp_against_ref("dst_III", 0xd577, dst_iii, dst_iii_ref);
}

#[test]
fn dct_IV_equals_straight_port() {
    check_iv_against_ref("dct_IV", 0xdc75, dct_iv, dct_iv_ref);
}

#[test]
fn dst_IV_equals_straight_port() {
    check_iv_against_ref("dst_IV", 0xd575, dst_iv, dst_iv_ref);
}

/// The lengths the library uses are among the tested ones.
#[test]
fn library_lengths_are_tested() {
    let iv = lengths(true);
    for l in [
        16, 24, 32, 48, 64, 96, 120, 128, 192, 240, 256, 384, 480, 512, 768, 960, 1024, 20, 40, 160,
    ] {
        assert!(iv.contains(&l), "type IV length {l}");
    }
    let ii = lengths(false);
    for l in [16, 24, 32, 40, 48, 64, 96, 128, 192, 256, 384, 512, 768, 960, 1024] {
        assert!(ii.contains(&l), "type II/III length {l}");
    }
}

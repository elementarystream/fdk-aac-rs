// Differential tests of fdk_aac_rs_fdk::fft against the C++ reference.
//
// Every kernel is reached through the library's fft() for its length, so the code compiled into
// the reference library is compared. The input vectors (`signal`) are shared with the tests of
// fft_rad2, dct and mdct.

use fdk_aac_rs_cref::fdk_fft::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::fft::*;

/// Every length `fft()` supports.
pub const FFT_LENGTHS: &[i32] = &[
    2, 3, 4, 5, 6, 8, 10, 12, 15, 16, 20, 24, 32, 48, 60, 64, 80, 96, 120, 128, 192, 240, 256, 384,
    480, 512,
];

/// Input vector number `k` of `n` values: the edge cases (zeros, impulses, constant and
/// alternating full scale, `MINVAL`/`MAXVAL`), then random values scaled by 0 to 31 bits of
/// headroom, mostly 1 to 4 bits.
pub fn signal(r: &mut Rng, k: usize, n: usize) -> Vec<i32> {
    match k {
        0 => vec![0; n],
        1 => {
            let mut v = vec![0; n];
            v[0] = i32::MAX;
            v
        }
        2 => {
            let mut v = vec![0; n];
            v[r.range(0, n as i32 - 1) as usize] = i32::MIN;
            v
        }
        3 => vec![i32::MAX; n],
        4 => vec![i32::MIN; n],
        5 => (0..n).map(|i| if i % 2 == 0 { i32::MAX } else { i32::MIN }).collect(),
        6 => (0..n).map(|i| if (i / 2) % 2 == 0 { i32::MIN } else { i32::MAX }).collect(),
        7 => (0..n).map(|_| if r.next_u32() & 1 == 0 { i32::MIN } else { i32::MAX }).collect(),
        8 => (0..n).map(|i| if i % 3 == 0 { 0x4000_0000 } else { -0x4000_0000 }).collect(),
        _ => {
            let shift = match r.next_u32() % 8 {
                0 => 0,
                1 => r.range(5, 31) as u32,
                _ => r.range(1, 4) as u32,
            };
            (0..n).map(|_| (r.next_u32() as i32) >> shift).collect()
        }
    }
}

/// Number of input vectors per length.
pub fn cases(n: usize) -> usize {
    if n <= 64 {
        3000
    } else if n <= 256 {
        800
    } else {
        300
    }
}

fn check_fft(length: i32, seed: u64) {
    let mut r = Rng::new(seed);
    let n = 2 * length as usize;
    for k in 0..cases(n) {
        let x = signal(&mut r, k, n);
        let sf0 = r.range(-40, 40);
        let (mut a, mut b) = (x.clone(), x.clone());
        let (mut sa, mut sb) = (sf0, sf0);
        fft(length, &mut a, &mut sa);
        unsafe { cref_fft_fft(length, b.as_mut_ptr(), &mut sb) };
        assert_eq!(a, b, "fft({length}) case {k}, input {x:?}");
        assert_eq!(sa, sb, "fft({length}) scalefactor case {k}");
    }
}

#[test]
fn fft_every_length() {
    for (i, &len) in FFT_LENGTHS.iter().enumerate() {
        check_fft(len, 0xff70_0000 + i as u64);
    }
}

/// The NEON `fft_16` / `fft_32` (aarch64 variant) against the straight ports, on the edge
/// cases and many random inputs.
#[test]
fn fft_16_32_equal_straight_port() {
    type Kernel = fn(&mut [i32]);
    let kernels: &[(usize, Kernel, Kernel)] =
        &[(16, fft_16, fft_16_ref), (32, fft_32, fft_32_ref)];
    for (i, &(len, fast, straight)) in kernels.iter().enumerate() {
        let mut r = Rng::new(0xff72_0000 + i as u64);
        let n = 2 * len;
        for k in 0..40 * cases(n) {
            let x = signal(&mut r, k, n);
            let (mut a, mut b) = (x.clone(), x.clone());
            fast(&mut a);
            straight(&mut b);
            assert_eq!(a, b, "fft_{len} case {k}, input {x:?}");
        }
    }
}

/// The optimized `fft()` (lane-parallel `fftN2`, NEON `dit_fft`) against the straight port, for
/// every length, on the edge cases and random input.
#[test]
fn fft_equals_straight_port() {
    for (i, &len) in FFT_LENGTHS.iter().enumerate() {
        let mut r = Rng::new(0xff71_0000 + i as u64);
        let n = 2 * len as usize;
        for k in 0..4 * cases(n) {
            let x = signal(&mut r, k, n);
            let (mut a, mut b) = (x.clone(), x.clone());
            let (mut sa, mut sb) = (3, 3);
            fft(len, &mut a, &mut sa);
            fft_ref(len, &mut b, &mut sb);
            assert_eq!(a, b, "fft({len}) case {k}");
            assert_eq!(sa, sb, "fft({len}) scalefactor case {k}");
        }
    }
}

/// The kernels called directly give the same values as `fft()` of the reference.
#[test]
fn fft_kernels() {
    type Kernel = fn(&mut [i32]);
    let kernels: &[(i32, Kernel)] = &[
        (2, fft2),
        (3, fft3),
        (4, fft_4),
        (5, fft5),
        (6, fft6),
        (8, fft_8),
        (10, fft10),
        (12, fft12),
        (15, fft15),
        (16, fft_16),
        (20, fft20),
        (24, fft24),
        (32, fft_32),
        (48, fft48),
        (60, fft60),
        (80, fft80),
        (96, fft96),
        (120, fft120),
        (192, fft192),
        (240, fft240),
        (384, fft384),
        (480, fft480),
    ];
    let mut r = Rng::new(0xff71);
    for &(len, kernel) in kernels {
        let n = 2 * len as usize;
        for k in 0..cases(n) / 4 {
            let x = signal(&mut r, k, n);
            let (mut a, mut b) = (x.clone(), x.clone());
            let mut sb = 0;
            kernel(&mut a);
            unsafe { cref_fft_fft(len, b.as_mut_ptr(), &mut sb) };
            assert_eq!(a, b, "kernel {len} case {k}");
        }
    }
}

/// `fft_4` and `fft_8` of fft.h, as compiled into the shim (fft_8 is also inlined into
/// FDK_hybrid.cpp).
#[test]
fn fft_4_fft_8_inline() {
    let mut r = Rng::new(0xff72);
    for k in 0..20_000 {
        let x = signal(&mut r, k % 1000, 8);
        let (mut a, mut b) = (x.clone(), x.clone());
        fft_4(&mut a);
        unsafe { cref_fft_fft_4(b.as_mut_ptr()) };
        assert_eq!(a, b, "fft_4 case {k}");

        let x = signal(&mut r, k % 1000, 16);
        let (mut a, mut b) = (x.clone(), x.clone());
        fft_8(&mut a);
        unsafe { cref_fft_fft_8(b.as_mut_ptr()) };
        assert_eq!(a, b, "fft_8 case {k}");
    }
}

/// Unsupported lengths leave the data and the exponent unchanged (`FDK_ASSERT(0)` is compiled
/// out in the reference; the port's `debug_assert!` fires in the checked profile).
#[test]
fn fft_unsupported_length() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut r = Rng::new(0xff73);
    for &len in &[0, 1, 7, 9, 11, 13, 14, 17, 18, 30, 36, 40, 1024] {
        let x = signal(&mut r, 20, 2 * 1024);
        let (mut a, mut b) = (x.clone(), x.clone());
        let (mut sa, mut sb) = (3, 3);
        fft(len, &mut a, &mut sa);
        unsafe { cref_fft_fft(len, b.as_mut_ptr(), &mut sb) };
        assert_eq!(a, b, "fft({len})");
        assert_eq!((sa, sb), (3, 3), "fft({len}) scalefactor");
    }
}

/// FFT16_W16 (`fft16_w16`, file-static in fft.cpp; a copy compiled into the shim), element by
/// element.
#[test]
fn fft16_w16_table() {
    let cname = std::ffi::CString::new("fft16_w16").unwrap();
    let mut bytes = 0;
    let p = unsafe { cref_fft_table(cname.as_ptr(), &mut bytes) } as *const i16;
    assert_eq!(bytes as usize, 2 * 2 * FFT16_W16.len());
    let c = unsafe { core::slice::from_raw_parts(p, FFT16_W16.len() * 2) };
    for (i, w) in FFT16_W16.iter().enumerate() {
        assert_eq!((w.re, w.im), (c[2 * i], c[2 * i + 1]), "fft16_w16[{i}]");
    }
}

/// FFT32_W32 (`fft32_w32`, file-static in fft.cpp; a copy compiled into the shim), element by
/// element.
#[test]
fn fft32_w32_table() {
    let cname = std::ffi::CString::new("fft32_w32").unwrap();
    let mut bytes = 0;
    let p = unsafe { cref_fft_table(cname.as_ptr(), &mut bytes) } as *const i16;
    assert_eq!(bytes as usize, 2 * 2 * FFT32_W32.len());
    let c = unsafe { core::slice::from_raw_parts(p, FFT32_W32.len() * 2) };
    for (i, w) in FFT32_W32.iter().enumerate() {
        assert_eq!((w.re, w.im), (c[2 * i], c[2 * i + 1]), "fft32_w32[{i}]");
    }
}

// Differential tests of fdk_aac_rs_fdk::generic_stds against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_generic_stds::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::generic_stds::*;

#[test]
fn constants() {
    let rust: [i64; 18] = [
        SECT_DATA_L1 as i64,
        SECT_DATA_L2 as i64,
        SECT_DATA_L1_A as i64,
        SECT_DATA_L1_B as i64,
        SECT_CONSTDATA_L1 as i64,
        SECT_DATA_EXTERN as i64,
        SECT_CONSTDATA_EXTERN as i64,
        FDK_MAX_OVERLAYS as i64,
        FDK_INT_MAX as i64,
        FDK_INT_MIN as i64,
        FDK_EXITCODE_OK as i64,
        FDK_EXITCODE_USAGE as i64,
        FDK_EXITCODE_DATAERROR as i64,
        FDK_EXITCODE_NOINPUT as i64,
        FDK_EXITCODE_UNAVAILABLE as i64,
        FDK_EXITCODE_SOFTWARE as i64,
        FDK_EXITCODE_CANTCREATE as i64,
        FDK_EXITCODE_IOERROR as i64,
    ];
    for (i, &v) in rust.iter().enumerate() {
        assert_eq!(v, unsafe { cref_generic_stds_const(i as i32) }, "constant {i}");
    }
    assert_eq!(M_PI.to_bits(), unsafe { cref_M_PI() }.to_bits());
}

#[test]
fn FDKmemcpy_FDKmemmove_FDKmemset_FDKmemclear() {
    let mut r = Rng::new(901);
    for _ in 0..2000 {
        let n = r.range(0, 40) as usize;
        let src: Vec<u8> = (0..n).map(|_| r.next_u32() as u8).collect();
        let k = r.range(0, n as i32) as usize;
        let (mut a, mut b) = (vec![0xAAu8; n], vec![0xAAu8; n]);
        fdk_memcpy(&mut a, &src, k);
        unsafe { cref_FDKmemcpy(b.as_mut_ptr(), src.as_ptr(), k as u32) };
        assert_eq!(a, b);
        let (mut a, mut b) = (vec![0x55u8; n], vec![0x55u8; n]);
        fdk_memmove(&mut a, &src, k);
        unsafe { cref_FDKmemmove(b.as_mut_ptr(), src.as_ptr(), k as u32) };
        assert_eq!(a, b);
        let v = r.i32();
        let (mut a, mut b) = (src.clone(), src.clone());
        fdk_memset(&mut a, v, k);
        unsafe { cref_FDKmemset(b.as_mut_ptr(), v, k as u32) };
        assert_eq!(a, b);
        let (mut a, mut b) = (src.clone(), src.clone());
        fdk_memclear(&mut a, k);
        unsafe { cref_FDKmemclear(b.as_mut_ptr(), k as u32) };
        assert_eq!(a, b);
        // memclear of wider elements clears the same bytes.
        let mut w: Vec<i32> = (0..n).map(|_| r.i32()).collect();
        let mut wb: Vec<u8> = w.iter().flat_map(|x| x.to_ne_bytes()).collect();
        fdk_memclear(&mut w, k);
        unsafe { cref_FDKmemclear(wb.as_mut_ptr(), (4 * k) as u32) };
        let wa: Vec<u8> = w.iter().flat_map(|x| x.to_ne_bytes()).collect();
        assert_eq!(wa, wb);
    }
}

#[test]
fn FDKmemcmp_FDKstrcmp_FDKstrlen() {
    let mut r = Rng::new(902);
    for _ in 0..20000 {
        let n = r.range(0, 12) as usize;
        // Few different characters, so that equal prefixes are frequent.
        let s1: Vec<u8> = (0..n).map(|_| b"ab\x00\xff"[(r.next_u32() % 4) as usize]).collect();
        let mut s2 = s1.clone();
        if n > 0 && r.next_u32() % 2 == 0 {
            let i = r.range(0, n as i32 - 1) as usize;
            s2[i] = b"ab\x00\xff"[(r.next_u32() % 4) as usize];
        }
        let k = r.range(0, n as i32) as usize;
        let c = unsafe { cref_FDKmemcmp(s1.as_ptr(), s2.as_ptr(), k as u32) };
        assert_eq!(fdk_memcmp(&s1, &s2, k).signum(), c.signum(), "{s1:?} {s2:?} {k}");
        // NUL-terminated copies for the string functions.
        let mut z1 = s1.clone();
        z1.push(0);
        let mut z2 = s2.clone();
        z2.push(0);
        let c = unsafe { cref_FDKstrcmp(z1.as_ptr() as *const i8, z2.as_ptr() as *const i8) };
        assert_eq!(fdk_strcmp(&z1, &z2).signum(), c.signum(), "{z1:?} {z2:?}");
        let c = unsafe {
            cref_FDKstrncmp(z1.as_ptr() as *const i8, z2.as_ptr() as *const i8, k as u32)
        };
        assert_eq!(fdk_strncmp(&z1, &z2, k).signum(), c.signum());
        assert_eq!(fdk_strlen(&z1), unsafe { cref_FDKstrlen(z1.as_ptr() as *const i8) });
        assert_eq!(fdk_strlen(&s1[..0]), 0);
    }
}

#[test]
fn endian_min_max() {
    assert_eq!(is_little_endian(), unsafe { cref_IS_LITTLE_ENDIAN() });
    each_dd(903, N, |a, b| {
        assert_eq!(to_little_endian(a as u32), unsafe { cref_TO_LITTLE_ENDIAN(a as u32) });
        assert_eq!(fdk_max(a, b), unsafe { cref_FDKmax(a, b) });
        assert_eq!(fdk_min(a, b), unsafe { cref_FDKmin(a, b) });
    });
}

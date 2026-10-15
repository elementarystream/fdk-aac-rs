// Differential tests of fdk_aac_rs_sacenc::sacenc_huff_tab against the C++ reference.
//
// The three tables are compared word by word with the tables of the library, the structure
// layout with offsetof/sizeof, and HUFF_VALUE/HUFF_LENGTH on random words.

use fdk_aac_rs_cref::sacenc_sacenc_huff_tab as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacenc::sacenc_huff_tab::*;

fn words<T>(t: &T) -> Vec<u32> {
    let n = core::mem::size_of::<T>() / 4;
    // SAFETY: the tables consist of `u32` words only.
    unsafe { core::slice::from_raw_parts(t as *const T as *const u32, n) }.to_vec()
}

#[test]
fn tables() {
    let mut buf = vec![0u32; 2048];
    let n = unsafe { c::cref_sacenc_huff_tab_cld(buf.as_mut_ptr()) } as usize;
    assert_eq!(&buf[..n], &words(&FDK_SACENC_HUFF_CLD_TAB)[..]);
    let n = unsafe { c::cref_sacenc_huff_tab_icc(buf.as_mut_ptr()) } as usize;
    assert_eq!(&buf[..n], &words(&FDK_SACENC_HUFF_ICC_TAB)[..]);
    let n = unsafe { c::cref_sacenc_huff_tab_part0(buf.as_mut_ptr()) } as usize;
    assert_eq!(&buf[..n], &words(&FDK_SACENC_HUFF_PART0_TAB)[..]);
    // The structures are the dumps HUFF_CLD_TAB_RAW, HUFF_ICC_TAB_RAW and HUFF_PART0_TAB_RAW,
    // reinterpreted; the dumps themselves, word by word:
    let n = unsafe { c::cref_sacenc_huff_tab_cld(buf.as_mut_ptr()) } as usize;
    assert_eq!(&buf[..n], &HUFF_CLD_TAB_RAW[..]);
    let n = unsafe { c::cref_sacenc_huff_tab_icc(buf.as_mut_ptr()) } as usize;
    assert_eq!(&buf[..n], &HUFF_ICC_TAB_RAW[..]);
    let n = unsafe { c::cref_sacenc_huff_tab_part0(buf.as_mut_ptr()) } as usize;
    assert_eq!(&buf[..n], &HUFF_PART0_TAB_RAW[..]);
}

#[test]
fn layout() {
    use core::mem::{offset_of, size_of};
    let mut v = [0i32; 14];
    let n = unsafe { c::cref_sacenc_huff_tab_offsets(v.as_mut_ptr()) } as usize;
    let r = [
        offset_of!(HuffCldTable, h2d) / 4,
        offset_of!(HuffCldTab2d, lav5) / 4,
        offset_of!(HuffCldTab2d, lav7) / 4,
        offset_of!(HuffCldTab2d, lav9) / 4,
        size_of::<HuffCldTab2d>() / 4,
        offset_of!(HuffIccTable, h2d) / 4,
        offset_of!(HuffIccTab2d, lav3) / 4,
        offset_of!(HuffIccTab2d, lav5) / 4,
        offset_of!(HuffIccTab2d, lav7) / 4,
        size_of::<HuffIccTab2d>() / 4,
        offset_of!(HuffPt0Table, icc) / 4,
        size_of::<HuffResTable>() / 4,
        size_of::<Lav6_2d>() / 4,
        size_of::<Lav12_2d>() / 4,
    ];
    assert_eq!(v[..n].iter().map(|&x| x as usize).collect::<Vec<_>>(), r);
}

#[test]
fn value_and_length() {
    let mut rng = Rng::new(1);
    for _ in 0..10000 {
        let p = rng.next_u32();
        let e = HuffEntry { packed: p };
        assert_eq!(unsafe { c::cref_sacenc_huff_tab_value(p) }, huff_value(e));
        assert_eq!(unsafe { c::cref_sacenc_huff_tab_length(p) }, huff_length(e));
    }
}

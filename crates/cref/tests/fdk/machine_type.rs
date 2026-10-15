// Differential tests of fdk_aac_rs_fdk::machine_type against the C++ reference.

use fdk_aac_rs_cref::fdk_machine_type::*;
use fdk_aac_rs_fdk::machine_type::*;

#[test]
fn constants() {
    let rust: [i64; 15] = [
        SHORT_BITS as i64,
        CHAR_BITS as i64,
        core::mem::size_of::<IntPcm>() as i64,
        MAXVAL_PCM as i64,
        MINVAL_PCM as i64,
        WAV_BITS as i64,
        SAMPLE_BITS as i64,
        SAMPLE_MAX as i64,
        SAMPLE_MIN as i64,
        ALIGNMENT_DEFAULT as i64,
        ALIGNMENT_EXTRES as i64,
        4,
        4,
        2,
        8,
    ];
    for (i, &v) in rust.iter().enumerate() {
        assert_eq!(v, unsafe { cref_machine_type_const(i as i32) }, "constant {i}");
    }
}

#[test]
fn ALIGN_SIZE() {
    for a in 0..10_000u32 {
        assert_eq!(align_size(a as usize) as u32, unsafe { cref_ALIGN_SIZE(a) });
        assert_eq!(algn_size_extres(a as usize) as u32, unsafe { cref_ALGN_SIZE_EXTRES(a) });
    }
}

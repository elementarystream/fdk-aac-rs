// Differential tests of fdk_aac_rs_sacenc::sacenc_const against the C++ reference.

use fdk_aac_rs_cref::sacenc_sacenc_const as c;
use fdk_aac_rs_sacenc::sacenc_const::*;

#[test]
fn constants() {
    let mut v = [0i32; 5];
    let mut eps = 0f32;
    unsafe { c::cref_sacenc_const_values(v.as_mut_ptr(), &mut eps) };
    assert_eq!(
        v,
        [
            NUM_QMF_BANDS as i32,
            MAX_QMF_BANDS as i32,
            SACENC_MAX_NUM_BOXES as i32,
            SACENC_MAX_INPUT_CHANNELS as i32,
            SACENC_MAX_OUTPUT_CHANNELS as i32
        ]
    );
    assert_eq!(eps.to_bits(), SACENC_FLOAT_EPSILON.to_bits());
}

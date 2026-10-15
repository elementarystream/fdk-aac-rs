// Differential tests of fdk_aac_rs_sbrenc::sbrenc_ram against the C++ reference: the layout of
// the dynamic RAM (sizes and the offsets of the buffer getters). The use of the RAM is compared
// in the PS and whole-encoder tests (ps_main.rs, sbr_encoder.rs), which dump all of it.

use fdk_aac_rs_cref::sbrenc_sbrenc_ram as c;
use fdk_aac_rs_sbrenc::sbrenc_ram::*;

#[test]
fn layout() {
    let mut out = [0i32; 11];
    unsafe { c::cref_sbrenc_ram_layout(out.as_mut_ptr()) };
    let mut r = vec![
        SBR_ENC_DYN_RAM_DBL as i32,
        OFFSET_NRG_DBL as i32,
        ENV_R_BUFF_DBL as i32,
        ENV_I_BUFF_DBL as i32,
        Y_2_BUF_DBL as i32,
    ];
    for n in 0..2 {
        r.push(get_ram_sbr_env_r_buffer(n) as i32);
        r.push(get_ram_sbr_env_i_buffer(n) as i32);
        r.push(get_ram_sbr_env_y_buffer(n) as i32);
    }
    assert_eq!(out.to_vec(), r);
    assert_eq!(get_ram_sbr_dynamic_ram().len(), SBR_ENC_DYN_RAM_DBL);
}

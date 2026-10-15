// Differential tests of fdk_aac_rs_sacdec::sac_dec_ssc_struct against the C++ reference: the
// C layout of SPATIAL_SPECIFIC_CONFIG (compared byte by byte by FDKmemcmp).

use core::mem::{offset_of, size_of};
use fdk_aac_rs_cref::sacdec_sac_dec_ssc_struct as c;
use fdk_aac_rs_sacdec::sac_dec_ssc_struct::*;

#[test]
fn sac_dec_ssc_struct_layout() {
    type S = SpatialSpecificConfig;
    let rs = [
        offset_of!(S, syntax_flags),
        offset_of!(S, sampling_freq),
        offset_of!(S, n_time_slots),
        offset_of!(S, freq_res),
        offset_of!(S, tree_config),
        offset_of!(S, quant_mode),
        offset_of!(S, b_arbitrary_downmix),
        offset_of!(S, b_residual_coding),
        offset_of!(S, bs_fixed_gain_dmx),
        offset_of!(S, temp_shape_config),
        offset_of!(S, decorr_config),
        offset_of!(S, n_input_channels),
        offset_of!(S, n_output_channels),
        offset_of!(S, n_ott_boxes),
        offset_of!(S, ott_config),
        offset_of!(S, n_ttt_boxes),
        offset_of!(S, residual_config),
        offset_of!(S, sac_ext_cnt),
        offset_of!(S, sac_ext_type),
        offset_of!(S, env_quant_mode),
        offset_of!(S, core_codec),
        offset_of!(S, stereo_config_index),
        offset_of!(S, core_sbr_frame_length_index),
        offset_of!(S, bs_high_rate_mode),
        offset_of!(S, bs_decorr_type),
        offset_of!(S, bs_pseudo_lr),
        offset_of!(S, bs_phase_coding),
        offset_of!(S, bs_ott_bands_phase_present),
        offset_of!(S, bs_ott_bands_phase),
        offset_of!(S, ott_cld_default),
        offset_of!(S, num_ott_bands_ipd),
        offset_of!(S, bitstream_ott_bands),
        offset_of!(S, num_ott_bands),
        size_of::<S>(),
        size_of::<SpatialdecFreqRes>(),
        size_of::<fdk_aac_rs_fdk::audio::AudioObjectType>(),
    ];
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_ssc_layout(out.as_mut_ptr()) } as usize;
    let rs: Vec<i32> = rs.iter().map(|&v| v as i32).collect();
    assert_eq!(&out[..n], &rs[..]);
    // The only padding byte precedes bsOttBandsPhase.
    assert_eq!(offset_of!(S, _pad0), offset_of!(S, bs_ott_bands_phase_present) + 1);
    assert_eq!(offset_of!(S, _pad0) + 1, offset_of!(S, bs_ott_bands_phase));
}

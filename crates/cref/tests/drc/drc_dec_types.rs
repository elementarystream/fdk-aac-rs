// Differential tests of fdk_aac_rs_drc::drc_dec_types against the C++ reference.
//
// The structures of the port have the memory layout of the C structures (the reader compares
// them with FDKmemcmp, and the tests compare whole structures byte by byte): every size and every
// field offset is compared with the C++ compiler's.

use core::mem::{offset_of, size_of};
use fdk_aac_rs_cref::drc_drc_dec_types as c;
use fdk_aac_rs_drc::drc_dec_types::*;

/// Sizes and offsets in the order of `cref_drc_types_layout` (shim/drc_drc_dec_types.cpp).
fn rust_layout() -> Vec<(&'static str, usize)> {
    macro_rules! s {
        ($t:ty) => {
            (stringify!($t), size_of::<$t>())
        };
    }
    macro_rules! o {
        ($t:ty, $f:ident) => {
            (concat!(stringify!($t), ".", stringify!($f)), offset_of!($t, $f))
        };
    }
    vec![
        s!(GainNode), o!(GainNode, gain_db), o!(GainNode, time),
        s!(UniDrcGainExtension), o!(UniDrcGainExtension, uni_drc_gain_ext_type),
        o!(UniDrcGainExtension, ext_bit_size),
        s!(UniDrcGain), o!(UniDrcGain, n_nodes), o!(UniDrcGain, gain_node),
        o!(UniDrcGain, uni_drc_gain_ext_present), o!(UniDrcGain, uni_drc_gain_extension),
        o!(UniDrcGain, status),
        s!(DuckingModification), o!(DuckingModification, ducking_scaling_present),
        o!(DuckingModification, ducking_scaling),
        s!(GainModification), o!(GainModification, target_characteristic_left_present),
        o!(GainModification, target_characteristic_left_index),
        o!(GainModification, target_characteristic_right_present),
        o!(GainModification, target_characteristic_right_index),
        o!(GainModification, gain_scaling_present), o!(GainModification, attenuation_scaling),
        o!(GainModification, amplification_scaling), o!(GainModification, gain_offset_present),
        o!(GainModification, gain_offset),
        s!(BandBorder), s!(CustomIndex),
        s!(DrcCharacteristic), o!(DrcCharacteristic, present), o!(DrcCharacteristic, is_cicp),
        o!(DrcCharacteristic, cicp_index), o!(DrcCharacteristic, cicp_index),
        s!(GainSet), o!(GainSet, gain_coding_profile), o!(GainSet, gain_interpolation_type),
        o!(GainSet, full_frame), o!(GainSet, time_alignment), o!(GainSet, time_delta_min_present),
        o!(GainSet, time_delta_min), o!(GainSet, band_count), o!(GainSet, drc_band_type),
        o!(GainSet, gain_sequence_index), o!(GainSet, drc_characteristic),
        o!(GainSet, band_border),
        s!(CustomDrcCharSigmoid), o!(CustomDrcCharSigmoid, gain),
        o!(CustomDrcCharSigmoid, io_ratio), o!(CustomDrcCharSigmoid, exp),
        o!(CustomDrcCharSigmoid, flip_sign),
        s!(CustomDrcCharNodes), o!(CustomDrcCharNodes, characteristic_node_count),
        o!(CustomDrcCharNodes, node_level), o!(CustomDrcCharNodes, node_gain),
        s!(CustomDrcChar),
        s!(DrcCoefficientsUniDrc), o!(DrcCoefficientsUniDrc, drc_location),
        o!(DrcCoefficientsUniDrc, drc_frame_size_present), o!(DrcCoefficientsUniDrc, drc_frame_size),
        o!(DrcCoefficientsUniDrc, characteristic_left_count),
        o!(DrcCoefficientsUniDrc, characteristic_left_format),
        o!(DrcCoefficientsUniDrc, custom_characteristic_left),
        o!(DrcCoefficientsUniDrc, characteristic_right_count),
        o!(DrcCoefficientsUniDrc, characteristic_right_format),
        o!(DrcCoefficientsUniDrc, custom_characteristic_right),
        o!(DrcCoefficientsUniDrc, gain_sequence_count), o!(DrcCoefficientsUniDrc, gain_set_count),
        o!(DrcCoefficientsUniDrc, gain_set),
        o!(DrcCoefficientsUniDrc, gain_set_index_for_gain_sequence),
        s!(DrcInstructionsUniDrc), o!(DrcInstructionsUniDrc, drc_set_id),
        o!(DrcInstructionsUniDrc, drc_set_complexity_level), o!(DrcInstructionsUniDrc, drc_location),
        o!(DrcInstructionsUniDrc, drc_apply_to_downmix), o!(DrcInstructionsUniDrc, downmix_id_count),
        o!(DrcInstructionsUniDrc, downmix_id), o!(DrcInstructionsUniDrc, drc_set_effect),
        o!(DrcInstructionsUniDrc, limiter_peak_target_present),
        o!(DrcInstructionsUniDrc, limiter_peak_target),
        o!(DrcInstructionsUniDrc, drc_set_target_loudness_present),
        o!(DrcInstructionsUniDrc, drc_set_target_loudness_value_upper),
        o!(DrcInstructionsUniDrc, drc_set_target_loudness_value_lower),
        o!(DrcInstructionsUniDrc, depends_on_drc_set_present),
        o!(DrcInstructionsUniDrc, depends_on_drc_set), o!(DrcInstructionsUniDrc, depends_on_drc_set),
        o!(DrcInstructionsUniDrc, requires_eq), o!(DrcInstructionsUniDrc, gain_ducking),
        o!(DrcInstructionsUniDrc, gain_ducking), o!(DrcInstructionsUniDrc, gain_set_index),
        o!(DrcInstructionsUniDrc, drc_channel_count), o!(DrcInstructionsUniDrc, n_drc_channel_groups),
        o!(DrcInstructionsUniDrc, gain_set_index_for_channel_group),
        s!(ChannelLayout), o!(ChannelLayout, base_channel_count),
        o!(ChannelLayout, layout_signaling_present), o!(ChannelLayout, defined_layout),
        o!(ChannelLayout, speaker_position),
        s!(DownmixInstructions), o!(DownmixInstructions, downmix_id),
        o!(DownmixInstructions, target_channel_count), o!(DownmixInstructions, target_layout),
        o!(DownmixInstructions, downmix_coefficients_present),
        o!(DownmixInstructions, bs_downmix_offset), o!(DownmixInstructions, downmix_coefficient),
        s!(UniDrcConfigExtension), o!(UniDrcConfigExtension, uni_drc_config_ext_type),
        o!(UniDrcConfigExtension, ext_bit_size),
        s!(UniDrcConfig), o!(UniDrcConfig, sample_rate_present), o!(UniDrcConfig, sample_rate),
        o!(UniDrcConfig, downmix_instructions_count_v0),
        o!(UniDrcConfig, downmix_instructions_count_v1),
        o!(UniDrcConfig, downmix_instructions_count),
        o!(UniDrcConfig, drc_coefficients_uni_drc_count_v0),
        o!(UniDrcConfig, drc_coefficients_uni_drc_count_v1),
        o!(UniDrcConfig, drc_coefficients_uni_drc_count),
        o!(UniDrcConfig, drc_instructions_uni_drc_count_v0),
        o!(UniDrcConfig, drc_instructions_uni_drc_count_v1),
        o!(UniDrcConfig, drc_instructions_uni_drc_count), o!(UniDrcConfig, channel_layout),
        o!(UniDrcConfig, downmix_instructions), o!(UniDrcConfig, drc_coefficients_uni_drc),
        o!(UniDrcConfig, drc_instructions_uni_drc), o!(UniDrcConfig, uni_drc_config_ext_present),
        o!(UniDrcConfig, uni_drc_config_ext),
        o!(UniDrcConfig, drc_instructions_count_incl_virtual), o!(UniDrcConfig, diff),
        s!(LoudnessMeasurement), o!(LoudnessMeasurement, method_definition),
        o!(LoudnessMeasurement, method_value), o!(LoudnessMeasurement, measurement_system),
        o!(LoudnessMeasurement, reliability),
        s!(LoudnessInfo), o!(LoudnessInfo, drc_set_id), o!(LoudnessInfo, eq_set_id),
        o!(LoudnessInfo, downmix_id), o!(LoudnessInfo, sample_peak_level_present),
        o!(LoudnessInfo, sample_peak_level), o!(LoudnessInfo, true_peak_level_present),
        o!(LoudnessInfo, true_peak_level), o!(LoudnessInfo, true_peak_level_measurement_system),
        o!(LoudnessInfo, true_peak_level_reliability), o!(LoudnessInfo, measurement_count),
        o!(LoudnessInfo, loudness_measurement),
        s!(LoudnessInfoSetExtension), o!(LoudnessInfoSetExtension, loudness_info_set_ext_type),
        o!(LoudnessInfoSetExtension, ext_bit_size),
        s!(LoudnessInfoSet), o!(LoudnessInfoSet, loudness_info_album_count_v0),
        o!(LoudnessInfoSet, loudness_info_album_count_v1),
        o!(LoudnessInfoSet, loudness_info_album_count), o!(LoudnessInfoSet, loudness_info_count_v0),
        o!(LoudnessInfoSet, loudness_info_count_v1), o!(LoudnessInfoSet, loudness_info_count),
        o!(LoudnessInfoSet, loudness_info_album), o!(LoudnessInfoSet, loudness_info),
        o!(LoudnessInfoSet, loudness_info_set_ext_present), o!(LoudnessInfoSet, loudness_info_set_ext),
        o!(LoudnessInfoSet, diff),
    ]
}

#[test]
fn layout_matches_c() {
    let rs = rust_layout();
    let mut cv = vec![0i32; 512];
    let n = unsafe { c::cref_drc_types_layout(cv.as_mut_ptr()) } as usize;
    assert_eq!(n, rs.len(), "number of layout values");
    for (i, (name, v)) in rs.iter().enumerate() {
        assert_eq!(*v as i32, cv[i], "{name}");
    }
}

#[test]
fn band_border_union() {
    let mut b = BandBorder { start_sub_band_index: 0x3A5 };
    assert_eq!(b.crossover_freq_index(), 0xA5);
    b.set_crossover_freq_index(0x07);
    assert_eq!(b.start_sub_band_index, 0x307);
}

#[test]
fn ducking_gain_modification_union() {
    // duckingModificationForChannel[c] overlays the bytes 4c..4c+4 of
    // gainModificationForChannelGroup.
    let mut inst = DrcInstructionsUniDrc::ZERO;
    inst.ducking_modification_for_channel_mut()[1].ducking_scaling = 0x1234;
    inst.ducking_modification_for_channel_mut()[1].ducking_scaling_present = 1;
    let gm = inst.gain_modification_for_channel_group()[0][0];
    assert_eq!(gm.gain_scaling_present, 1);
    assert_eq!(gm.attenuation_scaling, 0x1234);
    let b = pod_bytes(&inst);
    assert_eq!(&b[26 + 4..26 + 8], &[1, 0, 0x34, 0x12]);
}

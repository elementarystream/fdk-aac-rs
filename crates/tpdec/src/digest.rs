// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the state digest of the transport decoder (see fdk_aac_rs_fdk::digest).

//! [`StateDigest`] of the transport decoder state (not in C; see `fdk_aac_rs_fdk::digest`).
//! [`TransportDec`](crate::tpdec_lib::TransportDec) itself is digested in its module (it has a
//! private field).

use fdk_aac_rs_fdk::digest::{Digester, StateDigest};
use fdk_aac_rs_fdk::impl_state_digest;

use crate::tp_data::*;
use crate::tpdec_adif::CAdifHeader;
use crate::tpdec_adts::{StructAdts, StructAdtsBs};
use crate::tpdec_drm::StructDrm;
use crate::tpdec_latm::{CLatmDemux, LatmLayerInfo};
use crate::tpdec_lib::TransportDecParser;

impl_state_digest!(CCtrlCFGChange {
    flush_cnt,
    flush_status,
    build_up_cnt,
    build_up_status,
    cfg_changed,
    content_changed,
    force_cfg_change,
});
impl_state_digest!(TransportDecParser { adts, adif, latm, drm });
impl_state_digest!(StructDrm { crc_info, crc_read_value });
impl_state_digest!(LatmLayerInfo {
    m_frame_length_type,
    m_buffer_fullness,
    m_stream_id,
    m_frame_length_in_bits,
});
impl_state_digest!(CLatmDemux {
    m_linfo,
    m_tara_buffer_fullness,
    m_other_data_length,
    m_audio_mux_length_bytes,
    m_use_same_stream_mux,
    m_audio_mux_version,
    m_audio_mux_version_a,
    m_all_streams_same_time_framing,
    m_no_sub_frames,
    m_num_program,
    m_num_layer,
    m_other_data_present,
    m_crc_check_present,
    buffer_fullness_achieved,
    usac_explicit_cfg_changed,
    apply_asc,
    new_cfg_has_audio_pre_roll,
});
impl_state_digest!(CAdifHeader {
    num_program_config_elements,
    bit_rate,
    copyright_id_present,
    original_copy,
    home,
    bitstream_type,
});
/// The ADTS parser: `crcReadValue` is read only while the header in `bs` has CRC protection
/// (`protection_absent == 0`), and `adtsRead_DecodeHeader()` stores such a header in `bs` only
/// after it wrote `crcReadValue`: without protection it contributes nothing.
impl StateDigest for StructAdts {
    fn digest(&self, d: &mut Digester) {
        let StructAdts {
            bs,
            decoder_can_do_mpeg4,
            buffer_fullnes_start_flag,
            crc_info,
            crc_read_value,
            raw_data_block_dist,
        } = self;
        d.field("bs", bs);
        d.field("decoder_can_do_mpeg4", decoder_can_do_mpeg4);
        d.field("buffer_fullnes_start_flag", buffer_fullnes_start_flag);
        d.field("crc_info", crc_info);
        if bs.protection_absent == 0 {
            d.field("crc_read_value", crc_read_value);
        }
        d.field("raw_data_block_dist", raw_data_block_dist);
    }
}
impl_state_digest!(StructAdtsBs {
    mpeg_id,
    layer,
    protection_absent,
    profile,
    sample_freq_index,
    private_bit,
    channel_config,
    original,
    home,
    copyright_id,
    copyright_start,
    frame_length,
    adts_fullness,
    num_raw_blocks,
    num_pce_bits,
});
impl_state_digest!(CProgramConfig {
    element_instance_tag,
    profile,
    sampling_frequency_index,
    num_front_channel_elements,
    num_side_channel_elements,
    num_back_channel_elements,
    num_lfe_channel_elements,
    num_assoc_data_elements,
    num_valid_cc_elements,
    mono_mixdown_present,
    mono_mixdown_element_number,
    stereo_mixdown_present,
    stereo_mixdown_element_number,
    matrix_mixdown_index_present,
    matrix_mixdown_index,
    pseudo_surround_enable,
    front_element_is_cpe,
    front_element_tag_select,
    front_element_height_info,
    side_element_is_cpe,
    side_element_tag_select,
    side_element_height_info,
    back_element_is_cpe,
    back_element_tag_select,
    back_element_height_info,
    lfe_element_tag_select,
    assoc_data_element_tag_select,
    cc_element_is_ind_sw,
    valid_cc_element_tag_select,
    comment_field_bytes,
    comment,
    is_valid,
    num_channels,
    num_effective_channels,
    el_counter,
});
impl_state_digest!(CSAudioSpecificConfig {
    m_sc,
    m_progr_config_element,
    m_aot,
    m_sampling_frequency,
    m_samples_per_frame,
    m_direct_mapping,
    m_extension_audio_object_type,
    m_extension_sampling_frequency,
    m_channel_configuration,
    m_ep_config,
    m_vcb11_flag,
    m_rvlc_flag,
    m_hcr_flag,
    m_sbr_present_flag,
    m_ps_present_flag,
    m_sampling_frequency_index,
    m_extension_sampling_frequency_index,
    m_extension_channel_configuration,
    config_mode,
    aac_config_changed,
    sbr_config_changed,
    sac_config_changed,
    config,
    config_bits,
});
impl_state_digest!(CSAudioSpecificConfigSc {
    m_ga_specific_config,
    m_eld_specific_config,
    m_usac_config,
});
impl_state_digest!(CSUsacConfig {
    m_frame_length_flag,
    m_core_sbr_frame_length_index,
    m_sbr_ratio_index,
    m_n_usac_channels,
    m_channel_configuration_index,
    m_usac_num_elements,
    element,
    num_audio_channels,
    m_usac_config_extension_present,
    element_length_present,
    usac_config,
    usac_config_bits,
});
impl_state_digest!(CSUsacElementConfig {
    usac_element_type,
    m_noise_filling,
    m_harmonic_sbr,
    m_inter_tes,
    m_pvc,
    m_stereo_config_index,
    ext_element,
});
impl_state_digest!(CSUsacExtElementConfig {
    usac_ext_element_type,
    usac_ext_element_config_length,
    usac_ext_element_default_length,
    usac_ext_element_payload_frag,
    usac_ext_element_has_audio_pre_roll,
});
impl_state_digest!(CSEldSpecificConfig {
    m_frame_length_flag,
    m_sbr_present_flag,
    m_use_ld_qmf_time_align,
    m_sbr_sampling_rate,
    m_sbr_crc_flag,
    m_downscaled_sampling_frequency,
});
impl_state_digest!(CSGaSpecificConfig {
    m_frame_length_flag,
    m_depends_on_core_coder,
    m_core_coder_delay,
    m_extension_flag,
    m_extension_flag3,
    m_layer,
    m_num_of_sub_frame,
    m_layer_length,
});

/// The callbacks: the functions by address (the same functions in every decoder of a process);
/// of the user data pointers only whether they are set (they point to the decoder that owns
/// the transport decoder, a different object in every decoder).
impl StateDigest for CSTpCallBacks {
    fn digest(&self, d: &mut Digester) {
        let CSTpCallBacks {
            cb_update_config,
            cb_update_config_data,
            cb_free_mem,
            cb_free_mem_data,
            cb_ctrl_cfg_change,
            cb_ctrl_cfg_change_data,
            cb_ssc,
            cb_ssc_data,
            cb_sbr,
            cb_sbr_data,
            cb_usac,
            cb_usac_data,
            cb_uni_drc,
            cb_uni_drc_data,
        } = self;
        let f = |p: Option<usize>| p.unwrap_or(0) as u64;
        d.u64(f(cb_update_config.map(|p| p as usize)));
        d.u64(f(cb_free_mem.map(|p| p as usize)));
        d.u64(f(cb_ctrl_cfg_change.map(|p| p as usize)));
        d.u64(f(cb_ssc.map(|p| p as usize)));
        d.u64(f(cb_sbr.map(|p| p as usize)));
        d.u64(f(cb_usac.map(|p| p as usize)));
        d.u64(f(cb_uni_drc.map(|p| p as usize)));
        for p in [
            cb_update_config_data,
            cb_free_mem_data,
            cb_ctrl_cfg_change_data,
            cb_ssc_data,
            cb_sbr_data,
            cb_usac_data,
            cb_uni_drc_data,
        ] {
            d.bytes(&[p.is_null() as u8]);
        }
    }
}

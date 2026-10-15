// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the state digest of the SBR and PS decoder (see fdk_aac_rs_fdk::digest).

//! [`StateDigest`](fdk_aac_rs_fdk::digest::StateDigest) of the SBR and PS decoder (not in C; see
//! `fdk_aac_rs_fdk::digest`). Every field is fed, including the noise and harmonic phase
//! indices of the envelope adjustment (`phase_index`, `harm_index`), which advance with every
//! frame and make the states of two decoders that started at different frames differ unless the
//! advance since their starts happens to be equal: the chunked decoding falls back to the
//! sequential decoding for such streams.

use fdk_aac_rs_fdk::impl_state_digest;

use crate::env_calc::SbrCalculateEnvelope;
use crate::env_extr::{
    FrameInfo, FreqBandData, SbrFrameData, SbrHeaderData, SbrHeaderDataBs, SbrHeaderDataBsInfo,
    SbrPrevFrameData,
};
use crate::hbe::HbeTransposer;
use crate::lpp_tran::{PatchParam, SbrLppTrans, TransposerSettings, WhiteningFactors};
use crate::psdec::{MpegPsBsData, PsDec};
use crate::pvc_dec::PvcStaticData;
use crate::sbr_dec::{SbrChannel, SbrDec};
use crate::sbrdec_drc::SbrdecDrcChannel;
use crate::sbrdecoder::{SbrDecoderElement, SbrDecoderInstance};

impl_state_digest!(SbrDecoderInstance {
    p_sbr_element,
    sbr_header,
    h_parametric_stereo_dec,
    core_codec,
    num_sbr_elements,
    num_sbr_channels,
    sample_rate_in,
    sample_rate_out,
    codec_frame_size,
    syn_downsample_fac,
    downscale_factor,
    num_delay_frames,
    harmonic_sbr,
    num_flushed_frames,
    flags,
    sbr_in_data_headroom,
} skip {
    // Scratch of the per-channel parallelism: the overlap buffer is moved in only for the
    // duration of a call, the work buffer sections and the output copy are written before they
    // are read (verified by poisoning them before every split).
    par_scratch,
});
impl_state_digest!(PsDec {
    no_sub_samples,
    no_channels,
    proc_frame_based,
    b_ps_data_avail,
    ps_decoded_prv,
    bs_last_slot,
    bs_read_slot,
    process_slot,
    bs_data,
    a_iid_prev_frame_index,
    a_icc_prev_frame_index,
    b_prev_frame_fine_iid_q,
    prev_freq_res_iid,
    prev_freq_res_icc,
    last_usb,
    hybrid_analysis,
    hybrid_synthesis,
    ap_decor,
    h11r_prev,
    h12r_prev,
    h21r_prev,
    h22r_prev,
});
impl_state_digest!(MpegPsBsData {
    b_ps_header_valid,
    b_enable_iid,
    b_enable_icc,
    b_enable_ext,
    mode_iid,
    mode_icc,
    freq_res_iid,
    freq_res_icc,
    b_fine_iid_q,
    b_frame_class,
    no_env,
    a_env_start_stop,
    ab_iid_dt_flag,
    ab_icc_dt_flag,
    aa_iid_index,
    aa_icc_index,
});
impl_state_digest!(SbrHeaderData {
    sync_state,
    status,
    frame_error_flag,
    number_time_slots,
    number_of_analysis_bands,
    time_step,
    _pad0,
    sbr_proc_smpl_rate,
    bs_data,
    bs_dflt,
    bs_info,
    _pad1,
    freq_band_data,
    pvc_id_prev,
    _pad2,
});
impl_state_digest!(FreqBandData {
    n_sfb,
    n_nfb,
    num_master,
    low_subband,
    high_subband,
    ov_high_subband,
    limiter_band_table,
    no_limiter_bands,
    n_invf_bands,
    _pad0,
    _freq_band_table,
    freq_band_table_lo,
    freq_band_table_hi,
    freq_band_table_noise,
    v_k_master,
    _pad1,
});
impl_state_digest!(SbrHeaderDataBsInfo { amp_resolution, xover_band, sbr_preprocessing, pvc_mode });
impl_state_digest!(SbrHeaderDataBs {
    start_freq,
    stop_freq,
    freq_scale,
    alter_scale,
    noise_bands,
    limiter_bands,
    limiter_gains,
    interpol_freq,
    smoothing_length,
});
impl_state_digest!(SbrDecoderElement {
    p_sbr_channel,
    transposer_settings,
    element_id,
    n_channels,
    frame_error_flag,
    use_frame_slot,
    use_header_slot,
});
impl_state_digest!(TransposerSettings {
    n_cols,
    no_of_patches,
    lb_start_patching,
    lb_stop_patching,
    bw_borders,
    patch_param,
    wh_factors,
    overlap,
    _pad0,
});
impl_state_digest!(WhiteningFactors { off, transition_level, low_level, mid_level, high_level });
impl_state_digest!(PatchParam {
    source_start_band,
    source_stop_band,
    guard_start_band,
    target_start_band,
    target_band_offs,
    num_bands_in_patch,
});
impl_state_digest!(SbrChannel { frame_data, prev_frame_data, sbr_dec });
impl_state_digest!(SbrDec {
    sbr_calculate_envelope,
    lpp_trans,
    pvc_static_data,
    scale_ov,
    scale_lb,
    scale_hbe,
    prev_frame_l_sbr,
    prev_frame_hbe_sbr,
    codec_frame_size,
    h_hbe,
    qmf_domain_in_ch,
    qmf_domain_out_ch,
    sbr_drc_channel,
    hbe_slots_real,
    hbe_slots_imag,
    codec_qmf_buffer_real,
    codec_qmf_buffer_imag,
    hbe_no_channels,
    saved_states,
    apply_sbr_proc_old,
});
impl_state_digest!(SbrdecDrcChannel {
    prev_fact_mag,
    prev_fact_exp,
    curr_fact_mag,
    next_fact_mag,
    curr_fact_exp,
    next_fact_exp,
    num_bands_curr,
    num_bands_next,
    band_top_curr,
    band_top_next,
    drc_interpolation_scheme_curr,
    drc_interpolation_scheme_next,
    enable,
    win_sequence_curr,
    win_sequence_next,
});
impl_state_digest!(HbeTransposer {
    ana_qmf_states,
    syn_qmf_states,
    x_over_qmf,
    max_stretch,
    time_domain_win_len,
    qmf_in_buf_size,
    qmf_out_buf_size,
    no_cols,
    no_channels,
    start_band,
    stop_band,
    b_sbr41,
    in_buf_f,
    qmf_in_buf_real_f,
    qmf_in_buf_imag_f,
    qmf_buffer_codec_temp_slot_f,
    hbe_analysisc_qmf,
    hbe_synthesis_qmf,
    synthesis_qmf_pre_mod_f,
    qmf_hbe_buf_real_f,
    qmf_hbe_buf_imag_f,
    b_x_products,
    kstart,
    synth_size,
    highband_exp,
    target_exp,
});
impl_state_digest!(PvcStaticData { kx_last, pvc_mode_last, esg_slot_index, pvc_border0, esg });
impl_state_digest!(SbrLppTrans { _p_settings, bw_vector_old, lpc });
impl_state_digest!(SbrCalculateEnvelope {
    filt_buffer,
    filt_buffer_noise,
    filt_buffer_e,
    filt_buffer_noise_e,
    _pad0,
    start_up,
    phase_index,
    prev_tran_env,
    harm_flags_prev,
    harm_index,
    _pad1,
    sbr_patching_mode,
    prev_sbr_noise_floor_level,
    prev_n_nfb,
    prev_n_sfb,
    prev_lo_subband,
    prev_hi_subband,
    prev_ov_high_subband,
    _prev_freq_band_table,
    prev_freq_band_table_lo,
    prev_freq_band_table_hi,
    prev_freq_band_table_noise,
    sinusoidal_position_prev,
    _pad2,
    harm_flags_prev_active,
});
impl_state_digest!(SbrPrevFrameData {
    sfb_nrg_prev,
    prev_noise_level,
    _pad0,
    coupling,
    sbr_invf_mode,
    amp_res,
    stop_pos,
    frame_error_flag,
    prev_sbr_pitch_in_bins,
    prev_frame_info,
    _pad1,
});
impl_state_digest!(FrameInfo {
    frame_class,
    n_envelopes,
    borders,
    freq_res,
    tran_env,
    n_noise_envelopes,
    borders_noise,
    pvc_borders,
    noise_position,
    var_length,
});
impl_state_digest!(SbrFrameData {
    n_scale_factors,
    frame_info,
    domain_vec,
    domain_vec_noise,
    _pad0,
    sbr_invf_mode,
    coupling,
    amp_resolution_current_frame,
    add_harmonics,
    i_envelope,
    sbr_noise_floor_level,
    i_tes_active,
    inter_temp_shape_mode,
    pvc_id,
    ns,
    sinusoidal_position,
    sbr_patching_mode,
    sbr_oversampling_flag,
    sbr_pitch_in_bins,
    _pad1,
});

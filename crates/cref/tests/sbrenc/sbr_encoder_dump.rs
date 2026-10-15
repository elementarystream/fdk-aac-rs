// State dumps of the port's SBR encoder structures: the same sequence of values as
// shim/sbrenc_sbr_encoder_dump.cpp writes for the C structures (every member in declaration
// order; pointers to tables as their contents or an identifier, pointers into owned memory as
// the memory they point to).

#![allow(dead_code)]

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::FdkBitStream;
use fdk_aac_rs_fdk::hybrid::FdkAnaHybFilter;
use fdk_aac_rs_sbrenc::bit_sbr::*;
use fdk_aac_rs_sbrenc::cmondata::CommonData;
use fdk_aac_rs_sbrenc::code_env::SbrCodeEnvelope;
use fdk_aac_rs_sbrenc::fram_gen::*;
use fdk_aac_rs_sbrenc::invf_est::*;
use fdk_aac_rs_sbrenc::mh_det::*;
use fdk_aac_rs_sbrenc::nf_est::*;
use fdk_aac_rs_sbrenc::ps_bitenc::PsOut;
use fdk_aac_rs_sbrenc::ps_encode::{PsData, PsEncode};
use fdk_aac_rs_sbrenc::ps_main::ParametricStereo;
use fdk_aac_rs_sbrenc::ton_corr::*;
use fdk_aac_rs_sbrenc::tran_det::*;

/// A state dump.
#[derive(Default)]
pub struct Dump(pub Vec<i32>);

impl Dump {
    pub fn put(&mut self, v: i32) {
        self.0.push(v);
    }
    pub fn puts(&mut self, v: &[i32]) {
        self.0.extend_from_slice(v);
    }
    pub fn putu8(&mut self, v: &[u8]) {
        self.0.extend(v.iter().map(|&x| x as i32));
    }
    pub fn puts8(&mut self, v: &[i8]) {
        self.0.extend(v.iter().map(|&x| x as i32));
    }
    /// A pointer to a table of INTs: 0, or 1 and the first n values.
    pub fn putp(&mut self, p: Option<&[i32]>, n: usize) {
        match p {
            None => self.put(0),
            Some(p) => {
                self.put(1);
                self.puts(&p[..n]);
            }
        }
    }
}

/// Compares two dumps; on a difference, reports the first differing index with context.
pub fn assert_dump_eq(c: &[i32], r: &[i32], what: &str) {
    if c != r {
        let i = c.iter().zip(r.iter()).position(|(a, b)| a != b).unwrap_or(c.len().min(r.len()));
        let lo = i.saturating_sub(4);
        panic!(
            "{what}: dumps differ at {i} (len C {} Rust {}): C {:?} Rust {:?}",
            c.len(),
            r.len(),
            &c[lo..(i + 8).min(c.len())],
            &r[lo..(i + 8).min(r.len())]
        );
    }
}

pub fn grid(d: &mut Dump, g: &SbrGrid) {
    d.put(g.buffer_frame_start);
    d.put(g.number_time_slots);
    d.put(g.frame_class);
    d.put(g.bs_num_env);
    d.put(g.bs_abs_bord);
    d.put(g.n);
    d.put(g.p);
    d.puts(&g.bs_rel_bord);
    d.puts(&g.v_f);
    d.put(g.bs_abs_bord_0);
    d.put(g.bs_abs_bord_1);
    d.put(g.bs_num_rel_0);
    d.put(g.bs_num_rel_1);
    d.puts(&g.bs_rel_bord_0);
    d.puts(&g.bs_rel_bord_1);
    d.puts(&g.v_f_lr);
}

pub fn frame_info(d: &mut Dump, f: &SbrFrameInfo) {
    d.put(f.n_envelopes);
    d.puts(&f.borders);
    d.puts(&f.freq_res);
    d.put(f.short_env);
    d.put(f.n_noise_envelopes);
    d.puts(&f.borders_noise);
}

pub fn env_frame(d: &mut Dump, h: &SbrEnvelopeFrame) {
    d.put(h.frame_middle_slot);
    d.put(h.static_framing);
    d.put(h.num_env_static);
    d.put(h.freq_res_fixfix[0]);
    d.put(h.freq_res_fixfix[1]);
    d.put(h.f_res_trans_is_low as i32);
    d.putp(h.v_tuning_segm, 6);
    d.putp(h.v_tuning_freq, 3);
    d.put(h.dmin);
    d.put(h.dmax);
    d.put(h.allow_spread);
    d.put(h.frame_class_old);
    d.put(h.spread_flag);
    d.puts(&h.v_bord);
    d.put(h.length_v_bord);
    d.puts(&h.v_freq);
    d.put(h.length_v_freq);
    d.puts(&h.v_bord_follow);
    d.put(h.length_v_bord_follow);
    d.put(h.i_tran_follow);
    d.put(h.i_fill_follow);
    d.puts(&h.v_freq_follow);
    d.put(h.length_v_freq_follow);
    grid(d, &h.sbr_grid);
    frame_info(d, &h.sbr_frame_info);
}

pub fn tran_det(d: &mut Dump, h: &SbrTransientDetector) {
    d.puts(&h.transients);
    d.puts(&h.thresholds);
    d.put(h.tran_thr);
    d.put(h.split_thr_m);
    d.put(h.split_thr_e);
    d.put(h.prev_low_band_energy);
    d.put(h.prev_high_band_energy);
    d.put(h.tran_fc);
    d.put(h.no_cols);
    d.put(h.no_rows);
    d.put(h.mode);
    d.put(h.frame_shift);
    d.put(h.tran_off);
}

pub fn fast_tran_det(d: &mut Dump, h: &FastTranDetector) {
    d.puts(&h.transient_candidates);
    d.put(h.n_time_slots);
    d.put(h.lookahead);
    d.put(h.start_band);
    d.put(h.stop_band);
    d.puts(&h.d_bf_m);
    d.puts(&h.d_bf_e);
    d.puts(&h.energy_time_slots);
    d.puts(&h.energy_time_slots_scale);
    d.puts(&h.delta_energy);
    d.puts(&h.delta_energy_scale);
    d.puts(&h.lowpass_energy);
    d.puts(&h.lowpass_energy_scale);
}

pub fn inv_filt(d: &mut Dump, h: &SbrInvFiltEst) {
    d.put(h.number_of_strongest);
    d.puts(&h.prev_region_sbr);
    d.puts(&h.prev_region_orig);
    d.puts(&h.freq_band_table_inv_filt);
    d.put(h.no_detector_bands);
    d.put(h.no_detector_bands_max);
    match h.detector_params {
        None => d.put(0),
        Some(p) => {
            d.put(1);
            d.puts(p.quant_steps_sbr);
            d.puts(p.quant_steps_orig);
            d.puts(p.nrg_borders);
            d.put(p.num_regions_sbr);
            d.put(p.num_regions_orig);
            d.put(p.num_regions_nrg);
            for r in p.region_space.iter() {
                d.puts(r);
            }
            for r in p.region_space_transient.iter() {
                d.puts(r);
            }
            d.puts(&p.energy_comp_factor);
        }
    }
    d.puts(&h.prev_invf_mode);
    for v in h.detector_values.iter() {
        d.puts(&v.orig_quota_mean);
        d.puts(&v.sbr_quota_mean);
        d.puts(&v.orig_quota_mean_strongest);
        d.puts(&v.sbr_quota_mean_strongest);
        d.put(v.orig_quota_mean_filt);
        d.put(v.sbr_quota_mean_filt);
        d.put(v.orig_quota_mean_strongest_filt);
        d.put(v.sbr_quota_mean_strongest_filt);
        d.put(v.orig_quota_max);
        d.put(v.sbr_quota_max);
        d.put(v.avg_nrg);
    }
    d.put(h.nrg_avg);
    d.puts(&h.wm_qmf);
}

pub fn nf_est(d: &mut Dump, h: &SbrNoiseFloorEstimate) {
    for r in h.prev_noise_levels.iter() {
        d.puts(r);
    }
    d.puts(&h.noise_floor_offset);
    d.putp(h.smooth_filter.map(|f| &f[..]), 4);
    d.put(h.ana_max_level);
    d.put(h.weight_fac);
    d.puts(&h.freq_band_table_qmf);
    d.put(h.no_noise_bands);
    d.put(h.noise_bands);
    d.put(h.time_slots);
    d.put(h.diff_thres);
}

/// Only the first `nSfb` values of the tonality difference and flatness rows (see mh_det.rs).
pub fn mh_det(d: &mut Dump, h: &SbrMissingHarmonicsDetector) {
    d.put(h.qmf_no_channels);
    d.put(h.n_sfb);
    d.put(h.sample_freq);
    d.put(h.previous_transient_flag);
    d.put(h.previous_transient_frame);
    d.put(h.previous_transient_pos);
    d.put(h.no_vec_per_frame);
    d.put(h.transient_pos_offset);
    d.put(h.move_);
    d.put(h.tot_no_est);
    d.put(h.no_est_per_frame);
    d.put(h.time_slots);
    d.putu8(&h.guide_scfb);
    d.putu8(&h.prev_envelope_compensation);
    d.putu8(&h.detection_vectors);
    let n = h.n_sfb.clamp(0, 48) as usize;
    for r in h.tonality_diff.iter() {
        d.puts(&r[..n]);
    }
    for r in h.sfm_orig.iter() {
        d.puts(&r[..n]);
    }
    for r in h.sfm_sbr.iter() {
        d.puts(&r[..n]);
    }
    match h.mh_params {
        None => d.put(0),
        Some(p) => {
            d.put(1);
            d.put(p.delta_time);
            let t = &p.thres_holds;
            d.puts(&[
                t.thres_hold_diff,
                t.thres_hold_diff_guide,
                t.thres_hold_tone,
                t.inv_thres_hold_tone,
                t.thres_hold_tone_guide,
                t.sfm_thres_sbr,
                t.sfm_thres_orig,
                t.decay_guide_orig,
                t.decay_guide_diff,
                t.deriv_thres_max_ld_64,
                t.deriv_thres_below_ld_64,
                t.deriv_thres_above_ld_64,
            ]);
            d.put(p.max_comp);
        }
    }
    d.puts(&h.guide_vector_diff);
    d.puts(&h.guide_vector_orig);
    d.putu8(&h.guide_vector_detected);
}

pub fn ton_corr(d: &mut Dump, h: &SbrTonCorrEst) {
    d.put(h.switch_inverse_filt);
    d.put(h.no_qmf_channels);
    d.put(h.buffer_length);
    d.put(h.step_size);
    d.put(h.number_of_estimates);
    d.put(h.number_of_estimates_per_frame as i32);
    d.puts(&h.lpc_length);
    d.put(h.next_sample);
    d.put(h.move_);
    d.put(h.frame_start_index);
    d.put(h.start_index_matrix);
    d.put(h.frame_start_index_invf_est);
    d.put(h.prev_transient_flag);
    d.put(h.transient_next_frame);
    d.put(h.transient_pos_offset);
    d.puts(&h.sign_matrix);
    d.puts(&h.quota_matrix);
    d.puts(&h.nrg_vector);
    d.puts(&h.nrg_vector_freq);
    d.puts8(&h.index_vector);
    for p in h.patch_param.iter() {
        d.puts(&[
            p.source_start_band,
            p.source_stop_band,
            p.guard_start_band,
            p.target_start_band,
            p.target_band_offs,
            p.num_bands_in_patch,
        ]);
    }
    d.put(h.guard);
    d.put(h.shift_start_sb);
    d.put(h.no_of_patches);
    mh_det(d, &h.sbr_missing_harmonics_detector);
    nf_est(d, &h.sbr_noise_floor_estimate);
    inv_filt(d, &h.sbr_inv_filt);
}

/// A Huffman table of sbrenc_rom: 0 for `None`, 1 + its index in the list of the C dump, -1 if
/// unknown.
pub fn tabid<T>(p: Option<&[T]>) -> i32 {
    use fdk_aac_rs_sbrenc::sbrenc_rom::*;
    let Some(p) = p else {
        return 0;
    };
    let t: [*const u8; 20] = [
        V_HUFF_ENVELOPE_LEVEL_C_10T.as_ptr() as _,
        V_HUFF_ENVELOPE_LEVEL_L_10T.as_ptr() as _,
        V_HUFF_ENVELOPE_LEVEL_C_10F.as_ptr() as _,
        V_HUFF_ENVELOPE_LEVEL_L_10F.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_C_10T.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_L_10T.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_C_10F.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_L_10F.as_ptr() as _,
        V_HUFF_ENVELOPE_LEVEL_C_11T.as_ptr() as _,
        V_HUFF_ENVELOPE_LEVEL_L_11T.as_ptr() as _,
        V_HUFF_ENVELOPE_LEVEL_C_11F.as_ptr() as _,
        V_HUFF_ENVELOPE_LEVEL_L_11F.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_C_11T.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_L_11T.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_C_11F.as_ptr() as _,
        BOOK_SBR_ENV_BALANCE_L_11F.as_ptr() as _,
        V_HUFF_NOISE_LEVEL_C_11T.as_ptr() as _,
        V_HUFF_NOISE_LEVEL_L_11T.as_ptr() as _,
        BOOK_SBR_NOISE_BALANCE_C_11T.as_ptr() as _,
        BOOK_SBR_NOISE_BALANCE_L_11T.as_ptr() as _,
    ];
    let q = p.as_ptr() as *const u8;
    t.iter().position(|&x| x == q).map(|i| i as i32 + 1).unwrap_or(-1)
}

pub fn code_env(d: &mut Dump, h: &SbrCodeEnvelope) {
    d.put(h.offset);
    d.put(h.up_date);
    d.puts(&h.n_sfb);
    d.puts8(&h.sfb_nrg_prev);
    d.put(h.delta_t_across_frames);
    d.put(h.d_f_edge_1st_env);
    d.put(h.d_f_edge_incr);
    d.put(h.d_f_edge_incr_fac);
    d.put(h.code_book_scf_lav_time);
    d.put(h.code_book_scf_lav_freq);
    d.put(h.code_book_scf_lav_level_time);
    d.put(h.code_book_scf_lav_level_freq);
    d.put(h.code_book_scf_lav_balance_time);
    d.put(h.code_book_scf_lav_balance_freq);
    d.put(h.start_bits);
    d.put(h.start_bits_balance);
    d.put(tabid(h.hufftable_time_l));
    d.put(tabid(h.hufftable_freq_l));
    d.put(tabid(h.hufftable_level_time_l));
    d.put(tabid(h.hufftable_balance_time_l));
    d.put(tabid(h.hufftable_level_freq_l));
    d.put(tabid(h.hufftable_balance_freq_l));
}

/// `hSbrBSGrid` is dumped by the caller.
pub fn env_data(d: &mut Dump, e: &SbrEnvData) {
    d.put(e.sbr_xpos_ctrl);
    d.puts(&e.freq_res_fixfix);
    d.put(e.f_res_trans_is_low as i32);
    d.put(e.sbr_invf_mode);
    d.puts(&e.sbr_invf_mode_vec);
    d.put(e.sbr_xpos_mode);
    for r in e.ienvelope.iter() {
        d.puts(r);
    }
    d.put(e.code_book_scf_lav_balance);
    d.put(e.code_book_scf_lav);
    d.put(tabid(e.hufftable_time_c));
    d.put(tabid(e.hufftable_freq_c));
    d.put(tabid(e.hufftable_time_l));
    d.put(tabid(e.hufftable_freq_l));
    d.put(tabid(e.hufftable_level_time_c));
    d.put(tabid(e.hufftable_balance_time_c));
    d.put(tabid(e.hufftable_level_freq_c));
    d.put(tabid(e.hufftable_balance_freq_c));
    d.put(tabid(e.hufftable_level_time_l));
    d.put(tabid(e.hufftable_balance_time_l));
    d.put(tabid(e.hufftable_level_freq_l));
    d.put(tabid(e.hufftable_balance_freq_l));
    d.put(tabid(e.hufftable_noise_time_l));
    d.put(tabid(e.hufftable_noise_time_c));
    d.put(tabid(e.hufftable_noise_freq_l));
    d.put(tabid(e.hufftable_noise_freq_c));
    d.put(tabid(e.hufftable_noise_level_time_l));
    d.put(tabid(e.hufftable_noise_level_time_c));
    d.put(tabid(e.hufftable_noise_balance_time_l));
    d.put(tabid(e.hufftable_noise_balance_time_c));
    d.put(tabid(e.hufftable_noise_level_freq_l));
    d.put(tabid(e.hufftable_noise_level_freq_c));
    d.put(tabid(e.hufftable_noise_balance_freq_l));
    d.put(tabid(e.hufftable_noise_balance_freq_c));
    d.put(e.no_harmonics);
    d.put(e.add_harmonic_flag);
    d.putu8(&e.add_harmonic);
    d.put(e.si_sbr_start_env_bits_balance);
    d.put(e.si_sbr_start_env_bits);
    d.put(e.si_sbr_start_noise_bits_balance);
    d.put(e.si_sbr_start_noise_bits);
    d.put(e.no_of_envelopes);
    d.puts(&e.no_scf_bands);
    d.puts(&e.domain_vec);
    d.puts(&e.domain_vec_noise);
    d.puts8(&e.sbr_noise_levels);
    d.put(e.no_ofnoisebands);
    d.put(e.balance);
    d.put(e.init_sbr_amp_res);
    d.put(e.current_amp_res_ff);
    d.puts(&e.ton_hf);
    d.put(e.global_tonality);
    d.put(e.extended_data);
    d.put(e.extension_size);
    d.put(e.extension_id);
    d.putu8(&e.extended_data_buffer);
    d.put(e.ld_grid as i32);
}

pub fn header_data(d: &mut Dump, h: &SbrHeaderData) {
    d.puts(&[
        h.sbr_amp_res,
        h.sbr_start_frequency,
        h.sbr_stop_frequency,
        h.sbr_xover_band,
        h.sbr_noise_bands,
        h.sbr_data_extra,
        h.header_extra_1,
        h.header_extra_2,
        h.sbr_lc_stereo_mode,
        h.sbr_limiter_bands,
        h.sbr_limiter_gains,
        h.sbr_interpol_freq,
        h.sbr_smoothing_length,
        h.alter_scale,
        h.freq_scale,
        h.coupling,
        h.prev_coupling,
    ]);
}

pub fn bitstream_data(d: &mut Dump, h: &SbrBitstreamData) {
    d.puts(&[
        h.total_bits,
        h.payload_bits,
        h.fill_bits,
        h.header_active,
        h.header_active_delay,
        h.nr_send_header_data,
        h.count_send_header_data,
        h.right_border_fix,
    ]);
}

/// A bitstream: its state, the buffer as an offset in `base` (in bytes).
pub fn bitstream(d: &mut Dump, b: &FdkBitStream, base: &BitMem) {
    d.put(b.cache_word as i32);
    d.put(b.bits_in_cache as i32);
    d.put(b.h_bit_buf.valid_bits as i32);
    d.put(b.h_bit_buf.read_offset as i32);
    d.put(b.h_bit_buf.write_offset as i32);
    d.put(b.h_bit_buf.bit_ndx as i32);
    let m = b.h_bit_buf.mem();
    d.put(if m.is_empty() { -1 } else if m.ptr_eq(base) { b.h_bit_buf.base() as i32 } else { i32::MIN });
    d.put(b.h_bit_buf.buf_size() as i32);
    d.put(b.h_bit_buf.buf_bits() as i32);
    d.put(b.config_cache as i32);
}

pub fn cmon_data(d: &mut Dump, h: &CommonData, base: &BitMem) {
    d.put(h.sbr_hdr_bits);
    d.put(h.sbr_data_bits);
    d.put(h.sbr_fill_bits);
    bitstream(d, &h.sbr_bitbuf, base);
    bitstream(d, &h.tmp_write_bitbuf, base);
    d.put(h.x_over_freq);
    d.put(h.dyn_bw_enabled);
    d.put(h.sbr_num_channels);
    d.put(h.dyn_x_over_freq_enc);
}

pub fn ps_data(d: &mut Dump, p: &PsData) {
    d.puts(&[p.iid_enable, p.iid_enable_last, p.iid_quant_mode, p.iid_quant_mode_last]);
    d.puts(&p.iid_diff_mode);
    for r in p.iid_idx.iter() {
        d.puts(r);
    }
    d.puts(&p.iid_idx_last);
    d.puts(&[p.icc_enable, p.icc_enable_last, p.icc_quant_mode, p.icc_quant_mode_last]);
    d.puts(&p.icc_diff_mode);
    for r in p.icc_idx.iter() {
        d.puts(r);
    }
    d.puts(&p.icc_idx_last);
    d.puts(&[p.n_envelopes_last, p.header_cnt, p.iid_time_cnt, p.icc_time_cnt, p.no_env_cnt]);
}

pub fn ps_encode(d: &mut Dump, h: &PsEncode) {
    ps_data(d, &h.ps_data);
    d.put(h.ps_enc_mode);
    d.put(h.n_qmf_iid_groups);
    d.put(h.n_sub_qmf_iid_groups);
    d.puts(&h.iid_group_borders);
    d.puts(&h.subband2parameter_index);
    d.putu8(&h.iid_group_width_ld);
    d.put(h.iid_quant_error_threshold);
    d.putu8(&h.ps_band_nrg_scale);
}

pub fn ps_out(d: &mut Dump, o: &PsOut) {
    d.puts(&[
        o.enable_ps_header,
        o.enable_iid,
        o.iid_mode,
        o.enable_icc,
        o.icc_mode,
        o.enable_ipd_opd,
        o.frame_class,
        o.n_envelopes,
    ]);
    d.puts(&o.frame_border);
    for (delta, v, last) in [
        (&o.delta_iid, &o.iid, &o.iid_last),
        (&o.delta_icc, &o.icc, &o.icc_last),
        (&o.delta_ipd, &o.ipd, &o.ipd_last),
        (&o.delta_opd, &o.opd, &o.opd_last),
    ] {
        d.puts(delta);
        for r in v.iter() {
            d.puts(r);
        }
        d.puts(last);
    }
}

fn ana_hyb(d: &mut Dump, h: &FdkAnaHybFilter) {
    for v in h.buffer_lf_real.iter().chain(h.buffer_lf_imag.iter()) {
        d.put(*v as i32);
    }
    for v in h.buffer_hf_real.iter().chain(h.buffer_hf_imag.iter()) {
        d.put(*v as i32);
    }
    d.put(h.buffer_lf_pos);
    d.put(h.buffer_hf_pos);
    d.put(h.nr_bands);
    d.put(h.cplx_bands);
    d.put(h.hf_mode as i32);
    d.puts(&h.p_lf_memory);
    d.puts(&h.p_hf_memory);
    d.put(h.lf_memory_size as i32);
    d.put(h.hf_memory_size as i32);
    d.put(h.p_setup.is_some() as i32);
}

/// The hybrid data in the dynamic RAM is part of the dump of the RAM.
pub fn parametric_stereo(d: &mut Dump, h: &ParametricStereo) {
    d.put(h.h_ps_encode.is_some() as i32);
    if let Some(e) = h.h_ps_encode.as_deref() {
        ps_encode(d, e);
    }
    ps_out(d, &h.ps_out[0]);
    ps_out(d, &h.ps_out[1]);
    for a in h.static_hybrid_data.iter() {
        for b in a.iter() {
            for c in b.iter() {
                d.puts(c);
            }
        }
    }
    for a in h.qmf_delay_lines.iter() {
        for b in a.iter() {
            d.puts(b);
        }
    }
    d.put(h.qmf_delay_scale);
    d.put(h.ps_delay);
    d.put(h.max_envelopes as i32);
    d.putu8(&h.dyn_band_scale);
    d.puts(&h.max_band_value);
    d.put(h.dmx_scale as i32);
    d.put(h.init_ps);
    d.put(h.no_qmf_slots);
    d.put(h.no_qmf_bands);
    for f in h.fdk_hyb_ana_filter.iter() {
        ana_hyb(d, f);
    }
    d.put(h.fdk_hyb_syn_filter.nr_bands);
    d.put(h.fdk_hyb_syn_filter.cplx_bands);
    d.put(h.fdk_hyb_syn_filter.p_setup.is_some() as i32);
}

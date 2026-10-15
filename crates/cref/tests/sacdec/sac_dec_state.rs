// The complete state of the port's spatial decoder, bitstream frame and QMF domain in the
// layout of the dumps of shim/sacdec_sac_dec.cpp, the loading of the plain members from such a
// dump, and a pair of test instances (C++ and port) side by side.

#![allow(dead_code)]

use fdk_aac_rs_cref::sacdec_sac_dec as c;
use fdk_aac_rs_fdk::decorrelate::DecorrDec;
use fdk_aac_rs_fdk::hybrid::{FdkAnaHybFilter, FdkHybridSetup};
use fdk_aac_rs_fdk::qmf::QmfFilterBank;
use fdk_aac_rs_fdk::qmf_domain::*;
use fdk_aac_rs_fdk::tools_rom::*;
use fdk_aac_rs_sacdec::sac_bitdec::{kernels_table_index, spatial_dec_create_bs_frame};
use fdk_aac_rs_sacdec::sac_dec::*;
use fdk_aac_rs_sacdec::sac_dec_interface::*;
use fdk_aac_rs_sacdec::sac_rom::{BP_GF__FDK, BP__FDK};

/// A writer/reader of the plain members (the order of `io_self()` of the shim).
trait Io {
    fn v<T: Num>(&mut self, x: &mut T);
    /// Marks the start of the member `_name` (used to name the position of a difference).
    fn m(&mut self, _name: &'static str) {}
    fn a<T: Num>(&mut self, x: &mut [T]) {
        for e in x.iter_mut() {
            self.v(e);
        }
    }
}

trait Num: Copy {
    fn to_i32(self) -> i32;
    fn from_i32(v: i32) -> Self;
}
macro_rules! num {
    ($($t:ty),*) => {$(
        impl Num for $t {
            fn to_i32(self) -> i32 { self as i32 }
            fn from_i32(v: i32) -> Self { v as $t }
        }
    )*};
}
num!(i8, u8, i16, i32, u32);

struct Wr(Vec<i32>);
impl Io for Wr {
    fn v<T: Num>(&mut self, x: &mut T) {
        self.0.push(x.to_i32());
    }
}
struct Rd<'a>(&'a [i32], usize);
impl Io for Rd<'_> {
    fn v<T: Num>(&mut self, x: &mut T) {
        *x = T::from_i32(self.0[self.1]);
        self.1 += 1;
    }
}

fn flat3<const A: usize, const B: usize, const C: usize, T: Num>(
    io: &mut impl Io,
    x: &mut [[[T; C]; B]; A],
) {
    for a in x.iter_mut() {
        for b in a.iter_mut() {
            io.a(b);
        }
    }
}
fn flat2<const A: usize, const B: usize, T: Num>(io: &mut impl Io, x: &mut [[T; B]; A]) {
    for a in x.iter_mut() {
        io.a(a);
    }
}

fn io_self(s: &mut SpatialDec, io: &mut impl Io) {
    io.m("s.err_int");
    io.v(&mut s.err_int);
    io.m("s.static_dec_scale");
    io.v(&mut s.static_dec_scale);
    io.m("s.sampling_freq");
    io.v(&mut s.sampling_freq);
    io.m("s.decoder_level");
    io.v(&mut s.decoder_level);
    io.m("s.decoder_mode");
    io.v(&mut s.decoder_mode);
    io.m("s.binaural_mode");
    io.v(&mut s.binaural_mode);
    let p = &mut s.create_params;
    io.m("p.max_num_input_channels");
    io.v(&mut p.max_num_input_channels);
    io.m("p.max_num_output_channels");
    io.v(&mut p.max_num_output_channels);
    io.m("p.max_num_qmf_bands");
    io.v(&mut p.max_num_qmf_bands);
    io.m("p.max_num_hybrid_bands");
    io.v(&mut p.max_num_hybrid_bands);
    io.m("p.max_num_x_channels");
    io.v(&mut p.max_num_x_channels);
    io.m("p.max_num_v_channels");
    io.v(&mut p.max_num_v_channels);
    io.m("p.max_num_decor_channels");
    io.v(&mut p.max_num_decor_channels);
    io.m("p.max_num_cmplx_qmf_bands");
    io.v(&mut p.max_num_cmplx_qmf_bands);
    io.m("p.max_num_cmplx_hyb_bands");
    io.v(&mut p.max_num_cmplx_hyb_bands);
    io.m("p.max_num_res_channels");
    io.v(&mut p.max_num_res_channels);
    io.m("p.b_proc_residual");
    io.v(&mut p.b_proc_residual);
    io.m("p.max_num_residual_channels");
    io.v(&mut p.max_num_residual_channels);
    io.m("p.max_num_ott_boxes");
    io.v(&mut p.max_num_ott_boxes);
    io.m("p.max_num_params");
    io.v(&mut p.max_num_params);
    io.m("s.num_complex_processing_bands");
    io.v(&mut s.num_complex_processing_bands);
    io.m("s.tree_config");
    io.v(&mut s.tree_config);
    io.m("s.num_input_channels");
    io.v(&mut s.num_input_channels);
    io.m("s.num_output_channels");
    io.v(&mut s.num_output_channels);
    io.m("s.num_ott_boxes");
    io.v(&mut s.num_ott_boxes);
    io.m("s.num_m2rows");
    io.v(&mut s.num_m2rows);
    io.m("s.num_output_channels_at");
    io.v(&mut s.num_output_channels_at);
    io.m("s.quant_mode");
    io.v(&mut s.quant_mode);
    io.m("s.arbitrary_downmix");
    io.v(&mut s.arbitrary_downmix);
    io.m("s.residual_coding");
    io.v(&mut s.residual_coding);
    io.m("s.nr_residual_frame");
    io.v(&mut s.nr_residual_frame);
    io.m("s.nr_arb_downmix_residual_frame");
    io.v(&mut s.nr_arb_downmix_residual_frame);
    io.m("s.temp_shape_config");
    io.v(&mut s.temp_shape_config);
    io.m("s.decorr_type");
    io.v(&mut s.decorr_type);
    io.m("s.decorr_config");
    io.v(&mut s.decorr_config);
    io.m("s.env_quant_mode");
    io.v(&mut s.env_quant_mode);
    io.m("s.clip_protect_gain__fdk");
    io.v(&mut s.clip_protect_gain__fdk);
    io.m("s.clip_protect_gain_sf__fdk");
    io.v(&mut s.clip_protect_gain_sf__fdk);
    io.m("s.num_direkt_signals");
    io.v(&mut s.num_direkt_signals);
    io.m("s.w_start_residual_idx");
    io.v(&mut s.w_start_residual_idx);
    io.m("s.num_decor_signals");
    io.v(&mut s.num_decor_signals);
    io.m("s.num_v_channels");
    io.v(&mut s.num_v_channels);
    io.m("s.num_x_channels");
    io.v(&mut s.num_x_channels);
    io.m("s.time_slots");
    io.v(&mut s.time_slots);
    io.m("s.cur_time_slot");
    io.v(&mut s.cur_time_slot);
    io.m("s.prev_time_slot");
    io.v(&mut s.prev_time_slot);
    io.m("s.cur_ps");
    io.v(&mut s.cur_ps);
    io.m("s.frame_length");
    io.v(&mut s.frame_length);
    io.m("s.upmix_type");
    io.v(&mut s.upmix_type);
    io.m("s.partially_complex");
    io.v(&mut s.partially_complex);
    io.m("s.use_fd_reverb");
    io.v(&mut s.use_fd_reverb);
    io.m("s.b_share_delay_with_sbr");
    io.v(&mut s.b_share_delay_with_sbr);
    io.m("s.tp_hyb_band_border");
    io.v(&mut s.tp_hyb_band_border);
    io.m("s.qmf_bands");
    io.v(&mut s.qmf_bands);
    io.m("s.hybrid_bands");
    io.v(&mut s.hybrid_bands);
    io.m("s.tsd_ts");
    io.v(&mut s.tsd_ts);
    io.m("s.param2hyb");
    io.a(&mut s.param2hyb);
    io.m("s.kernels_width");
    io.a(&mut s.kernels_width);
    io.m("s.residual_sampling_freq");
    io.v(&mut s.residual_sampling_freq);
    io.m("s.residual_present");
    io.a(&mut s.residual_present);
    io.m("s.residual_bands");
    io.a(&mut s.residual_bands);
    io.m("s.residual_qmf_bands");
    io.a(&mut s.residual_qmf_bands);
    io.m("s.arbdmx_frames_per_spatial_frame");
    io.v(&mut s.arbdmx_frames_per_spatial_frame);
    io.m("s.arbdmx_upd_qmf");
    io.v(&mut s.arbdmx_upd_qmf);
    io.m("s.num_parameter_bands");
    io.v(&mut s.num_parameter_bands);
    io.m("s.bitstream_parameter_bands");
    io.v(&mut s.bitstream_parameter_bands);
    io.m("s.num_ott_bands");
    io.a(&mut s.num_ott_bands);
    io.m("s.extend_frame");
    io.v(&mut s.extend_frame);
    io.m("s.num_parameter_sets_prev");
    io.v(&mut s.num_parameter_sets_prev);
    io.m("s.smg_time");
    io.a(&mut s.smg_time);
    io.m("s.smg_data");
    flat2(io, &mut s.smg_data);
    io.m("s.ott_cld_idx_prev");
    flat2(io, &mut s.ott_cld_idx_prev);
    io.m("s.ott_icc_idx_prev");
    flat2(io, &mut s.ott_icc_idx_prev);
    io.m("s.arbdmx_gain_idx_prev");
    flat2(io, &mut s.arbdmx_gain_idx_prev);
    io.m("s.ott_ipd_idx_prev");
    flat2(io, &mut s.ott_ipd_idx_prev);
    io.m("s.out_idx_data");
    flat3(io, &mut s.out_idx_data);
    io.m("s.cmp_ott_cld_idx_prev");
    flat2(io, &mut s.cmp_ott_cld_idx_prev);
    io.m("s.cmp_ott_icc_idx_prev");
    flat2(io, &mut s.cmp_ott_icc_idx_prev);
    io.m("s.ott_icc_diffidx");
    flat3(io, &mut s.ott_icc_diffidx);
    io.m("s.cmp_ott_ipd_idx_prev");
    flat2(io, &mut s.cmp_ott_ipd_idx_prev);
    io.m("s.cmp_arbdmx_gain_idx_prev");
    flat2(io, &mut s.cmp_arbdmx_gain_idx_prev);
    io.m("s.ott_cld__fdk");
    flat3(io, &mut s.ott_cld__fdk);
    io.m("s.ott_icc__fdk");
    flat3(io, &mut s.ott_icc__fdk);
    io.m("s.arbdmx_gain__fdk");
    flat3(io, &mut s.arbdmx_gain__fdk);
    io.m("s.arbdmx_alpha__fdk");
    io.a(&mut s.arbdmx_alpha__fdk);
    io.m("s.arbdmx_alpha_prev__fdk");
    io.a(&mut s.arbdmx_alpha_prev__fdk);
    io.m("s.stereo_config_index");
    io.v(&mut s.stereo_config_index);
    io.m("s.high_rate_mode");
    io.v(&mut s.high_rate_mode);
    io.m("s.phase_coding");
    io.v(&mut s.phase_coding);
    io.m("s.ott_ipd__fdk");
    flat3(io, &mut s.ott_ipd__fdk);
    io.m("s.phase_left__fdk");
    io.a(&mut s.phase_left__fdk);
    io.m("s.phase_right__fdk");
    io.a(&mut s.phase_right__fdk);
    io.m("s.phase_prev_left__fdk");
    io.a(&mut s.phase_prev_left__fdk);
    io.m("s.phase_prev_right__fdk");
    io.a(&mut s.phase_prev_right__fdk);
    io.m("s.num_ott_bands_ipd");
    io.v(&mut s.num_ott_bands_ipd);
    io.m("s.m2_real__fdk");
    flat3(io, &mut s.m2_real__fdk);
    io.m("s.m2_imag__fdk");
    flat3(io, &mut s.m2_imag__fdk);
    io.m("s.m2_real_prev__fdk");
    flat3(io, &mut s.m2_real_prev__fdk);
    io.m("s.m2_imag_prev__fdk");
    flat3(io, &mut s.m2_imag_prev__fdk);
    io.m("s.pc_filterdelay");
    io.v(&mut s.pc_filterdelay);
    io.m("s.qmf_input_delay_buf_pos");
    io.v(&mut s.qmf_input_delay_buf_pos);
    io.m("s.qmf_input_real__fdk");
    flat2(io, &mut s.qmf_input_real__fdk);
    io.m("s.qmf_input_imag__fdk");
    flat2(io, &mut s.qmf_input_imag__fdk);
    io.m("s.hyb_input_real__fdk");
    flat2(io, &mut s.hyb_input_real__fdk);
    io.m("s.hyb_input_imag__fdk");
    flat2(io, &mut s.hyb_input_imag__fdk);
    io.m("s.bin_gain");
    io.v(&mut s.bin_gain);
    io.m("s.reverb_gain");
    io.v(&mut s.reverb_gain);
    io.m("s.bin_center_gain");
    io.v(&mut s.bin_center_gain);
    io.m("s.reverb_center_gain");
    io.v(&mut s.reverb_center_gain);
    io.m("s.qmf_residual_real__fdk");
    flat3(io, &mut s.qmf_residual_real__fdk);
    io.m("s.qmf_residual_imag__fdk");
    flat3(io, &mut s.qmf_residual_imag__fdk);
    io.m("s.hyb_residual_real__fdk");
    flat2(io, &mut s.hyb_residual_real__fdk);
    io.m("s.hyb_residual_imag__fdk");
    flat2(io, &mut s.hyb_residual_imag__fdk);
    io.m("s.w_real__fdk");
    flat2(io, &mut s.w_real__fdk);
    io.m("s.w_imag__fdk");
    flat2(io, &mut s.w_imag__fdk);
    io.m("s.hyb_output_real_dry__fdk");
    flat2(io, &mut s.hyb_output_real_dry__fdk);
    io.m("s.hyb_output_imag_dry__fdk");
    flat2(io, &mut s.hyb_output_imag_dry__fdk);
    io.m("s.hyb_output_real_wet__fdk");
    flat2(io, &mut s.hyb_output_real_wet__fdk);
    io.m("s.hyb_output_imag_wet__fdk");
    flat2(io, &mut s.hyb_output_imag_wet__fdk);
    let sm = &mut s.smooth_state;
    io.m("sm.prev_param_slot");
    io.v(&mut sm.prev_param_slot);
    io.m("sm.prev_smg_time");
    io.v(&mut sm.prev_smg_time);
    io.m("sm.prev_smg_data");
    io.a(&mut sm.prev_smg_data);
    io.m("sm.opd_left_state__fdk");
    io.a(&mut sm.opd_left_state__fdk);
    io.m("sm.opd_right_state__fdk");
    io.a(&mut sm.opd_right_state__fdk);
    let r = &mut s.reshape_bb_env_state;
    io.m("r.alpha__fdk");
    io.v(&mut r.alpha__fdk);
    io.m("r.beta__fdk");
    io.v(&mut r.beta__fdk);
    io.m("r.part_nrg_prev__fdk");
    flat2(io, &mut r.part_nrg_prev__fdk);
    io.m("r.norm_nrg_prev__fdk");
    io.a(&mut r.norm_nrg_prev__fdk);
    io.m("r.frame_nrg_prev__fdk");
    io.a(&mut r.frame_nrg_prev__fdk);
    io.m("r.part_nrg_prev_sf");
    io.a(&mut r.part_nrg_prev_sf);
    io.m("r.part_nrg_prev2_sf");
    io.a(&mut r.part_nrg_prev2_sf);
    io.m("r.norm_nrg_prev_sf");
    io.a(&mut r.norm_nrg_prev_sf);
    io.m("r.frame_nrg_prev_sf");
    io.a(&mut r.frame_nrg_prev_sf);
    io.m("s.row2channel_dmx_ges");
    io.a(&mut s.row2channel_dmx_ges);
    let h = &mut s.h_stp_dec;
    io.m("h.run_dry_ener");
    io.a(&mut h.run_dry_ener);
    io.m("h.run_wet_ener");
    io.a(&mut h.run_wet_ener);
    io.m("h.old_dry_ener_ld64");
    io.a(&mut h.old_dry_ener_ld64);
    io.m("h.old_wet_ener_ld64");
    io.a(&mut h.old_wet_ener_ld64);
    io.m("h.prev_tp_scale");
    io.a(&mut h.prev_tp_scale);
    io.m("h.update_old_ener");
    io.v(&mut h.update_old_ener);
    io.m("s.b_overwrite_m1m2prev");
    io.v(&mut s.b_overwrite_m1m2prev);
    let ci = &mut s.conceal_info;
    io.m("ci.conceal_params.method");
    io.v(&mut ci.conceal_params.method);
    io.m("ci.conceal_params.num_keep_frames");
    io.v(&mut ci.conceal_params.num_keep_frames);
    io.m("ci.conceal_params.num_fade_out_frames");
    io.v(&mut ci.conceal_params.num_fade_out_frames);
    io.m("ci.conceal_params.num_fade_in_frames");
    io.v(&mut ci.conceal_params.num_fade_in_frames);
    io.m("ci.conceal_params.num_release_frames");
    io.v(&mut ci.conceal_params.num_release_frames);
    io.m("ci.conceal_state");
    io.v(&mut ci.conceal_state);
    io.m("ci.cnt_state_frames");
    io.v(&mut ci.cnt_state_frames);
    io.m("ci.cnt_valid_frames");
    io.v(&mut ci.cnt_valid_frames);
    io.m("s.sac_in_data_headroom");
    io.v(&mut s.sac_in_data_headroom);
}

/// Names the value at the position `i` of a dump of the spatial decoder.
pub fn member_at(i: usize) -> String {
    struct Loc(usize, Vec<(usize, &'static str)>);
    impl Io for Loc {
        fn v<T: Num>(&mut self, _x: &mut T) {
            self.0 += 1;
        }
        fn m(&mut self, name: &'static str) {
            self.1.push((self.0, name));
        }
    }
    let mut s = SpatialDec::default();
    let mut l = Loc(0, Vec::new());
    io_self(&mut s, &mut l);
    match l.1.iter().rev().find(|(p, _)| *p <= i) {
        Some((p, n)) if i < l.0 => format!("{n}[{}]", i - p),
        _ => format!("dump tail [{}]", i - l.0),
    }
}

/// The number of values of the plain members.
pub fn plain_len() -> usize {
    let mut s = SpatialDec::default();
    let mut w = Wr(Vec::new());
    io_self(&mut s, &mut w);
    w.0.len()
}

fn setup_id(p: Option<&'static FdkHybridSetup>) -> i32 {
    p.map_or(-1, |p| {
        p.nr_qmf_bands as i32 * 100 + p.n_hyb_bands[0] as i32 * 10 + p.n_hyb_bands[1] as i32
    })
}

fn dump_ana_hyb(h: &FdkAnaHybFilter, o: &mut Vec<i32>) {
    // The offsets of the C++ NULL pointers are dumped as 0 (the port has no NULL offsets).
    o.extend(h.buffer_lf_real.iter().map(|&v| v as i32));
    o.extend(h.buffer_lf_imag.iter().map(|&v| v as i32));
    o.extend(h.buffer_hf_real.iter().map(|&v| v as i32));
    o.extend(h.buffer_hf_imag.iter().map(|&v| v as i32));
    o.extend([h.buffer_lf_pos, h.buffer_hf_pos, h.nr_bands, h.cplx_bands, h.hf_mode as i32]);
    o.extend([h.lf_memory_size as i32, h.hf_memory_size as i32, setup_id(h.p_setup)]);
    o.extend(&h.p_lf_memory[..(h.lf_memory_size as usize / 4).min(h.p_lf_memory.len())]);
    o.extend(&h.p_hf_memory[..(h.hf_memory_size as usize / 4).min(h.p_hf_memory.len())]);
}

const ABSENT: i32 = -100;

/// As `dump()` of the decorrelator tests, then the buffer.
fn dump_decorr(d: &DecorrDec, o: &mut Vec<i32>) {
    o.extend([
        d.l_state_buffer_cplx,
        if d.state_buffer_cplx.is_some() { 0 } else { -1 },
        d.l_delay_buffer_cplx,
        d.delay_buffer_cplx.map_or(-1, |v| v as i32),
    ]);
    let four = |o: &mut Vec<i32>, v: Option<[i32; 4]>| o.extend(v.unwrap_or([ABSENT; 4]));
    four(o, d.rev_filt_type.copied());
    four(o, d.rev_band_offset.map(|t| t.map(|v| v as i32)));
    four(o, d.rev_delay.map(|t| t.map(|v| v as i32)));
    four(o, d.rev_filter_order.map(|t| t.map(|v| v as i32)));
    o.extend(d.reverb_band_delay_buffer_index);
    o.extend(d.state_buffer_offset.map(|v| v as i32));
    o.push(d.numbins);
    o.push(d.partially_complex);
    let k = &d.ducker;
    o.extend([k.hybrid_bands, k.parameter_bands, k.partially_complex, k.ducker_type]);
    o.push(match (k.qs_next, k.map_proc_bands2_hyb_bands) {
        (Some(q), Some(m)) => (q.as_ptr() as isize - m.as_ptr() as isize) as i32,
        _ => -1,
    });
    for i in 0..21 {
        o.push(k.map_proc_bands2_hyb_bands.map_or(ABSENT, |t| t[i] as i32));
    }
    for i in 0..65 {
        o.push(k.map_hyb_bands2_proc_bands.map_or(ABSENT, |t| t[i] as i32));
    }
    o.extend(k.smooth_dir_rev_nrg);
    o.extend(k.peak_decay);
    o.extend(k.peak_diff);
    o.extend([k.max_val_direct_data, k.max_val_reverb_data]);
    o.extend([
        k.scale_direct_nrg as i32,
        k.scale_reverb_nrg as i32,
        k.scale_smooth_dir_rev_nrg as i32,
        k.headroom_smooth_dir_rev_nrg as i32,
    ]);
    for f in &d.filter {
        o.push(f.state_cplx.map_or(-1, |v| v as i32));
        o.push(f.delay_buffer_cplx.map_or(-1, |v| v as i32));
        for j in 0..3 {
            o.push(f.numerator_real.map_or(ABSENT, |t| t[j] as i32));
        }
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[0].re as i32));
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[0].im as i32));
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[3].re as i32));
        o.push(f.coeffs_packed.map_or(ABSENT, |t| t[3].im as i32));
        o.push(f.denominator_real.is_some() as i32);
    }
    if let Some(b) = &d.state_buffer_cplx {
        o.extend(&b[..]);
    }
}

/// The complete state of a spatial decoder (`dump_self()` of the shim).
pub fn dump_self(s: &SpatialDec) -> Vec<i32> {
    let mut c = s.clone();
    let mut w = Wr(Vec::new());
    io_self(&mut c, &mut w);
    let mut o = w.0;
    o.push(kernels_table_index(s.kernels));
    o.push(s.p_config_current as i32);
    o.push(s.p_activ_m2_param_bands as i32);
    o.push(s.h_stp_dec.bp as i32);
    o.push(s.h_stp_dec.bp_gf as i32);
    let _ = (&BP__FDK, &BP_GF__FDK);
    for h in &s.hybrid_analysis {
        dump_ana_hyb(h, &mut o);
    }
    for h in &s.hybrid_synthesis {
        o.extend([h.nr_bands, h.cplx_bands, setup_id(h.p_setup)]);
    }
    dump_decorr(&s.ap_decor[0], &mut o);
    o
}

/// Loads the plain members from a dump (`cref_sacinst_load()`).
pub fn load_self(s: &mut SpatialDec, v: &[i32]) -> usize {
    let mut r = Rd(v, 0);
    io_self(s, &mut r);
    r.1
}

fn io_ll(l: &mut LosslessData, io: &mut impl Io) {
    io.a(&mut l.bs_xxx_data_mode);
    io.a(&mut l.bs_quant_coarse_xxx);
    io.a(&mut l.bs_freq_res_stride_xxx);
    io.a(&mut l.nocmp_quant_coarse_xxx);
}

fn io_frame(f: &mut SpatialBsFrame, ll: &mut BsLlState, io: &mut impl Io) {
    io.v(&mut f.bs_independency_flag);
    io.v(&mut f.new_bs_data);
    io.v(&mut f.num_parameter_sets);
    io.a(&mut f.param_slot);
    for x in f.cmp_ott_cld_idx.iter_mut() {
        flat2(io, x);
    }
    for x in f.cmp_ott_icc_idx.iter_mut() {
        flat2(io, x);
    }
    io.a(&mut f.bs_smooth_mode);
    io.a(&mut f.bs_smooth_time);
    io.a(&mut f.bs_freq_res_stride_smg);
    flat2(io, &mut f.bs_smg_data);
    for x in f.cmp_arbdmx_gain_idx.iter_mut() {
        flat2(io, x);
    }
    for l in f.cld_lossless_data.iter_mut() {
        io_ll(l, io);
    }
    for l in f.icc_lossless_data.iter_mut() {
        io_ll(l, io);
    }
    for l in f.ipd_lossless_data.iter_mut() {
        io_ll(l, io);
    }
    for x in f.cmp_ott_ipd_idx.iter_mut() {
        flat2(io, x);
    }
    io.v(&mut f.phase_mode);
    io.v(&mut f.opd_smoothing_mode);
    io.a(&mut f.temp_shape_enable_channel_ges);
    flat2(io, &mut f.bs_env_shape_data);
    io.a(&mut f.temp_shape_enable_channel_stp);
    io.v(&mut f.tsd_data[0].bs_tsd_enable);
    io.v(&mut f.tsd_data[0].num_slots);
    io.a(&mut f.tsd_data[0].bs_tsd_tr_phase_data);
    for i in 0..MAX_NUM_PARAMETERS {
        for s in [&mut ll.cld_lossless_state[i], &mut ll.icc_lossless_state[i], &mut ll.ipd_lossless_state[i]]
        {
            io.v(&mut s.bs_quant_coarse_xxx_prev);
            io.v(&mut s.bs_quant_coarse_xxx_prev_parse);
        }
    }
}

/// The frame and the lossless states (`cref_sacinst_dump_frame()`); the last value is the
/// check of the state links of the C++ structure (always 1 in the port).
pub fn dump_frame(f: &SpatialBsFrame, ll: &BsLlState) -> Vec<i32> {
    let (mut f, mut ll) = (f.clone(), *ll);
    let mut w = Wr(Vec::new());
    io_frame(&mut f, &mut ll, &mut w);
    w.0.push(1);
    w.0
}

pub fn load_frame(f: &mut SpatialBsFrame, ll: &mut BsLlState, v: &[i32]) -> usize {
    let mut r = Rd(v, 0);
    io_frame(f, ll, &mut r);
    r.1
}

// --- QMF domain ----------------------------------------------------------------------------

/// The QMF tables in the order of `qmf_tables` of the shim (the order of the qmf tests).
pub fn qmf_tables() -> [&'static [i16]; 34] {
    [
        &QMF_PFILT120,
        &QMF_PFILT200,
        &QMF_PFILT240,
        &QMF_PFILT400,
        &QMF_PFILT640,
        &QMF_CLDFB_640,
        &QMF_CLDFB_320,
        &QMF_CLDFB_160,
        &QMF_CLDFB_80,
        &QMF_MPSLDFB_320,
        &QMF_MPSLDFB_640,
        &QMF_PHASESHIFT_COS16,
        &QMF_PHASESHIFT_SIN16,
        &QMF_PHASESHIFT_COS24,
        &QMF_PHASESHIFT_SIN24,
        &QMF_PHASESHIFT_COS32,
        &QMF_PHASESHIFT_SIN32,
        &QMF_PHASESHIFT_COS_DOWNSAMP32,
        &QMF_PHASESHIFT_SIN_DOWNSAMP32,
        &QMF_PHASESHIFT_COS40,
        &QMF_PHASESHIFT_SIN40,
        &QMF_PHASESHIFT_COS64,
        &QMF_PHASESHIFT_SIN64,
        &QMF_PHASESHIFT_COS32_CLDFB_ANA,
        &QMF_PHASESHIFT_COS32_CLDFB_SYN,
        &QMF_PHASESHIFT_SIN32_CLDFB,
        &QMF_PHASESHIFT_COS16_CLDFB_ANA,
        &QMF_PHASESHIFT_COS16_CLDFB_SYN,
        &QMF_PHASESHIFT_SIN16_CLDFB,
        &QMF_PHASESHIFT_COS8_CLDFB_ANA,
        &QMF_PHASESHIFT_COS8_CLDFB_SYN,
        &QMF_PHASESHIFT_SIN8_CLDFB,
        &QMF_PHASESHIFT_COS64_CLDFB,
        &QMF_PHASESHIFT_SIN64_CLDFB,
    ]
}

fn table_id(t: &[i16]) -> i32 {
    if t.is_empty() {
        return 0;
    }
    match qmf_tables().iter().position(|x| x.as_ptr() == t.as_ptr() && x.len() == t.len()) {
        Some(i) => i as i32 + 1,
        None => -1,
    }
}

fn table_of(id: i32) -> &'static [i16] {
    if id <= 0 {
        &[]
    } else {
        qmf_tables()[id as usize - 1]
    }
}

fn dump_fb(f: &QmfFilterBank, o: &mut Vec<i32>) {
    o.extend([
        table_id(f.p_filter),
        f.filter_states as i32,
        f.filter_size,
        table_id(f.t_cos),
        table_id(f.t_sin),
        f.filter_scale,
        f.no_channels,
        f.no_col,
        f.lsb,
        f.usb,
        f.syn_scalefactor,
        f.out_scalefactor,
        f.out_gain_m,
        f.out_gain_e,
        f.flags as i32,
        f.p_stride as i32,
    ]);
}

fn load_fb(f: &mut QmfFilterBank, v: &[i32]) {
    f.p_filter = table_of(v[0]);
    f.filter_states = v[1] != 0;
    f.filter_size = v[2];
    f.t_cos = table_of(v[3]);
    f.t_sin = table_of(v[4]);
    f.filter_scale = v[5];
    f.no_channels = v[6];
    f.no_col = v[7];
    f.lsb = v[8];
    f.usb = v[9];
    f.syn_scalefactor = v[10];
    f.out_scalefactor = v[11];
    f.out_gain_m = v[12];
    f.out_gain_e = v[13];
    f.flags = v[14] as u32;
    f.p_stride = v[15] as u8;
}

pub fn gc_fields(g: &FdkQmfDomainGc) -> [i32; 21] {
    [
        g.qmf_domain_explicit_config as i32,
        g.n_input_channels as i32,
        g.n_input_channels_requested as i32,
        g.n_output_channels as i32,
        g.n_output_channels_requested as i32,
        g.park_channel as i32,
        g.park_channel_requested as i32,
        g.flags as i32,
        g.flags_requested as i32,
        g.n_bands_analysis as i32,
        g.n_bands_analysis_requested as i32,
        g.n_bands_synthesis as i32,
        g.n_bands_synthesis_requested as i32,
        g.n_qmf_time_slots as i32,
        g.n_qmf_time_slots_requested as i32,
        g.n_qmf_ov_time_slots as i32,
        g.n_qmf_ov_time_slots_requested as i32,
        g.n_qmf_proc_bands as i32,
        g.n_qmf_proc_bands_requested as i32,
        g.n_qmf_proc_channels as i32,
        g.n_qmf_proc_channels_requested as i32,
    ]
}

pub fn set_gc_fields(g: &mut FdkQmfDomainGc, v: &[i32]) {
    g.qmf_domain_explicit_config = v[0] as u8;
    g.n_input_channels = v[1] as u8;
    g.n_input_channels_requested = v[2] as u8;
    g.n_output_channels = v[3] as u8;
    g.n_output_channels_requested = v[4] as u8;
    g.park_channel = v[5] as u8;
    g.park_channel_requested = v[6] as u8;
    g.flags = v[7] as u32;
    g.flags_requested = v[8] as u32;
    g.n_bands_analysis = v[9] as u8;
    g.n_bands_analysis_requested = v[10] as u8;
    g.n_bands_synthesis = v[11] as u16;
    g.n_bands_synthesis_requested = v[12] as u16;
    g.n_qmf_time_slots = v[13] as u8;
    g.n_qmf_time_slots_requested = v[14] as u8;
    g.n_qmf_ov_time_slots = v[15] as u8;
    g.n_qmf_ov_time_slots_requested = v[16] as u8;
    g.n_qmf_proc_bands = v[17] as u8;
    g.n_qmf_proc_bands_requested = v[18] as u8;
    g.n_qmf_proc_channels = v[19] as u8;
    g.n_qmf_proc_channels_requested = v[20] as u8;
}

fn ptr3(p: QmfSlotPtr) -> [i32; 3] {
    match p {
        QmfSlotPtr::Null => [0, 0, 0],
        QmfSlotPtr::Work { sect, offset } => [1, sect as i32, offset as i32],
        QmfSlotPtr::Overlap { ch, offset } => [2, ch as i32, offset as i32],
    }
}

fn ptr_of(v: &[i32]) -> QmfSlotPtr {
    match v[0] {
        1 => QmfSlotPtr::Work { sect: v[1] as u8, offset: v[2] as u32 },
        2 => QmfSlotPtr::Overlap { ch: v[1] as u8, offset: v[2] as u32 },
        _ => QmfSlotPtr::Null,
    }
}

/// The complete QMF domain (`dump_qd()` of the shim).
pub fn dump_qd(qd: &FdkQmfDomain) -> Vec<i32> {
    let g = &qd.global_conf;
    let mut o: Vec<i32> = gc_fields(g).to_vec();
    for k in 0..QMF_MAX_WB_SECTIONS {
        o.push(qd.mem.work_buffer[k].is_some() as i32);
    }
    let n_ana = 10 * g.n_bands_analysis as usize;
    let n_ov = g.n_qmf_ov_time_slots as usize * g.n_qmf_proc_bands as usize * CMPLX_MOD as usize;
    let n_slots = g.n_qmf_time_slots as usize + g.n_qmf_ov_time_slots as usize;
    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        let d = &qd.qmf_domain_in[ch];
        dump_fb(&d.fb, &mut o);
        o.extend([d.scaling.lb_scale, d.scaling.ov_lb_scale, d.scaling.hb_scale, d.scaling.ov_hb_scale]);
        o.extend([
            d.work_buf_n_time_slots as i32,
            d.work_buf_n_bands as i32,
            d.work_buffer_offset as i32,
            d.work_buffer_sect_size as i32,
            d.p_work_buffer as i32,
            d.p_global_conf as i32,
        ]);
        o.push(d.p_ana_qmf_states.is_some() as i32);
        if let Some(s) = &d.p_ana_qmf_states {
            o.extend(&s[..n_ana]);
        }
        o.push(qd.mem.overlap_buffer[ch].is_some() as i32);
        if let Some(s) = &qd.mem.overlap_buffer[ch] {
            o.extend(&s[..n_ov]);
        }
        let both = d.h_qmf_slots_real.is_some() && d.h_qmf_slots_imag.is_some();
        o.push(both as i32);
        if both {
            let (r, i) = (d.h_qmf_slots_real.as_deref().unwrap(), d.h_qmf_slots_imag.as_deref().unwrap());
            for ts in 0..n_slots {
                o.extend(ptr3(r[ts]));
                o.extend(ptr3(i[ts]));
            }
        }
    }
    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        let d = &qd.qmf_domain_out[ch];
        dump_fb(&d.fb, &mut o);
        o.push(d.p_syn_qmf_states.is_some() as i32);
        if let Some(s) = &d.p_syn_qmf_states {
            o.extend(&s[..SYN_QMF_STATES_SIZE]);
        }
    }
    for k in 0..QMF_MAX_WB_SECTIONS {
        if let Some(b) = &qd.mem.work_buffer[k] {
            o.extend(&b[..]);
        }
    }
    o
}

/// Builds the port's QMF domain from a C++ dump (`dump_qd()`): every field and buffer the dump
/// contains (the buffers are allocated with the port's sizes; values beyond the dumped extent
/// are kept).
pub fn load_qd_full(qd: &mut FdkQmfDomain, v: &[i32]) {
    let mut p = 0usize;
    let mut take = |n: usize| {
        let s = &v[p..p + n];
        p += n;
        s
    };
    set_gc_fields(&mut qd.global_conf, take(21));
    let wb: Vec<bool> = take(QMF_MAX_WB_SECTIONS).iter().map(|&x| x != 0).collect();
    let g = qd.global_conf;
    let n_ana = 10 * g.n_bands_analysis as usize;
    let n_ov = g.n_qmf_ov_time_slots as usize * g.n_qmf_proc_bands as usize * CMPLX_MOD as usize;
    let n_slots = g.n_qmf_time_slots as usize + g.n_qmf_ov_time_slots as usize;
    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        let d = &mut qd.qmf_domain_in[ch];
        load_fb(&mut d.fb, take(16));
        let s = take(4);
        d.scaling.lb_scale = s[0];
        d.scaling.ov_lb_scale = s[1];
        d.scaling.hb_scale = s[2];
        d.scaling.ov_hb_scale = s[3];
        let s = take(6);
        d.work_buf_n_time_slots = s[0] as u8;
        d.work_buf_n_bands = s[1] as u8;
        d.work_buffer_offset = s[2] as u16;
        d.work_buffer_sect_size = s[3] as u16;
        d.p_work_buffer = s[4] != 0;
        d.p_global_conf = s[5] != 0;
        if take(1)[0] != 0 {
            let src = take(n_ana);
            let b = d.p_ana_qmf_states.get_or_insert_with(|| vec![0; ANA_QMF_STATES_SIZE].into());
            b[..n_ana].copy_from_slice(src);
        } else {
            d.p_ana_qmf_states = None;
        }
        if take(1)[0] != 0 {
            let src = take(n_ov);
            let b = qd.mem.overlap_buffer[ch]
                .get_or_insert_with(|| vec![0; QMF_OVERLAP_BUFFER_SIZE].into());
            b[..n_ov].copy_from_slice(src);
        } else {
            qd.mem.overlap_buffer[ch] = None;
        }
        let d = &mut qd.qmf_domain_in[ch];
        if take(1)[0] != 0 {
            let r = d.h_qmf_slots_real.get_or_insert_with(|| vec![QmfSlotPtr::Null; QMF_SLOTS_SIZE].into());
            let mut re = Vec::new();
            let mut im = Vec::new();
            for _ in 0..n_slots {
                re.push(ptr_of(take(3)));
                im.push(ptr_of(take(3)));
            }
            r[..n_slots].copy_from_slice(&re);
            let i = d.h_qmf_slots_imag.get_or_insert_with(|| vec![QmfSlotPtr::Null; QMF_SLOTS_SIZE].into());
            i[..n_slots].copy_from_slice(&im);
        } else {
            d.h_qmf_slots_real = None;
            d.h_qmf_slots_imag = None;
        }
    }
    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        let d = &mut qd.qmf_domain_out[ch];
        load_fb(&mut d.fb, take(16));
        if take(1)[0] != 0 {
            let src = take(SYN_QMF_STATES_SIZE);
            let b = d.p_syn_qmf_states.get_or_insert_with(|| vec![0; SYN_QMF_STATES_SIZE].into());
            b.copy_from_slice(src);
        } else {
            d.p_syn_qmf_states = None;
        }
    }
    for k in 0..QMF_MAX_WB_SECTIONS {
        if wb[k] {
            let src = take(QMF_WB_SECTION_SIZE as usize);
            let b = qd.mem.work_buffer[k].get_or_insert_with(|| vec![0; QMF_WB_SECTION_SIZE as usize].into());
            b.copy_from_slice(src);
        } else {
            qd.mem.work_buffer[k] = None;
        }
    }
}

// --- test instance pair -------------------------------------------------------------------

/// A C++ test instance (`SacInst` of the shim).
pub struct CInst(pub *mut core::ffi::c_void);

impl Drop for CInst {
    fn drop(&mut self) {
        unsafe { c::cref_sacinst_free(self.0) };
    }
}

impl CInst {
    pub fn dump(&self) -> Vec<i32> {
        let mut out = vec![0i32; 1 << 16];
        let n = unsafe { c::cref_sacinst_dump(self.0, out.as_mut_ptr()) };
        out.truncate(n as usize);
        out
    }
    pub fn dump_frame(&self) -> Vec<i32> {
        let mut out = vec![0i32; 1 << 15];
        let n = unsafe { c::cref_sacinst_dump_frame(self.0, out.as_mut_ptr()) };
        out.truncate(n as usize);
        out
    }
    pub fn dump_qd(&self) -> Vec<i32> {
        let mut out = vec![0i32; 1 << 17];
        let n = unsafe { c::cref_sacinst_dump_qd(self.0, out.as_mut_ptr()) };
        out.truncate(n as usize);
        out
    }
    pub fn ssc(&self) -> SpatialSpecificConfig {
        let mut b = [0u8; SPATIAL_SPECIFIC_CONFIG_SIZE];
        unsafe { c::cref_sacinst_get_ssc(self.0, b.as_mut_ptr()) };
        SpatialSpecificConfig::from_bytes(&b)
    }
}

/// The port's test instance.
pub struct RInst {
    pub sd: Box<SpatialDec>,
    pub ssc: SpatialSpecificConfig,
    pub frame: SpatialBsFrame,
    pub ll: BsLlState,
    pub qd: FdkQmfDomain,
}

/// A pair of test instances.
pub struct Pair {
    pub r: RInst,
    pub c: CInst,
}

impl Pair {
    /// Opens both decoders (`FDK_SpatialDecOpen`) with the decoder level 0.
    pub fn new() -> Pair {
        let cfg = SpatialDecConfig {
            decoder_level: DECODER_LEVEL_0,
            decoder_mode: EXT_HQ_ONLY,
            max_num_output_channels: OUTPUT_CHANNELS_DEFAULT,
            binaural_mode: BINAURAL_NONE,
        };
        let mut sd = fdk_spatial_dec_open(&cfg, 0).expect("FDK_SpatialDecOpen");
        sd.p_config_current = true;
        let mut frame = SpatialBsFrame::default();
        spatial_dec_create_bs_frame(&mut frame);
        let c = CInst(unsafe { c::cref_sacinst_new(0, EXT_HQ_ONLY, OUTPUT_CHANNELS_DEFAULT) });
        assert_eq!(unsafe { c::cref_sacinst_is_open(c.0) }, 1);
        Pair {
            r: RInst {
                sd,
                ssc: SpatialSpecificConfig::default(),
                frame,
                ll: BsLlState::default(),
                qd: FdkQmfDomain::default(),
            },
            c,
        }
    }

    /// Sets the configuration (`pConfigCurrent`) on both sides.
    pub fn set_ssc(&mut self, ssc: &SpatialSpecificConfig) {
        self.r.ssc = *ssc;
        unsafe { c::cref_sacinst_set_ssc(self.c.0, ssc.as_bytes().as_ptr()) };
    }

    /// Requests the QMF domain configuration (21 values of the gc) and configures both domains.
    pub fn configure_qd(&mut self, gc: &[i32; 21]) {
        set_gc_fields(&mut self.r.qd.global_conf, gc);
        let r = fdk_qmf_domain_configure(&mut self.r.qd);
        let cr = unsafe { c::cref_sacinst_configure_qd(self.c.0, gc.as_ptr()) };
        assert_eq!(r, cr, "FDK_QmfDomain_Configure");
    }

    /// Loads the plain members of the spatial decoder from `v` on both sides.
    pub fn load(&mut self, v: &[i32]) {
        load_self(&mut self.r.sd, v);
        unsafe { c::cref_sacinst_load(self.c.0, v.as_ptr()) };
    }

    pub fn load_frame(&mut self, v: &[i32]) {
        load_frame(&mut self.r.frame, &mut self.r.ll, v);
        unsafe { c::cref_sacinst_load_frame(self.c.0, v.as_ptr()) };
    }

    /// Compares the complete states.
    pub fn check(&self, what: &str) {
        let (rs, cs) = (dump_self(&self.r.sd), self.c.dump());
        if rs != cs {
            let i = rs.iter().zip(&cs).position(|(a, b)| a != b).unwrap_or(rs.len().min(cs.len()));
            panic!(
                "{what}: spatialDec differs at {i} ({}, plain members: {}): port {:?} C {:?} (lengths {} {})",
                member_at(i),
                plain_len(),
                &rs[i.saturating_sub(2)..(i + 6).min(rs.len())],
                &cs[i.saturating_sub(2)..(i + 6).min(cs.len())],
                rs.len(),
                cs.len()
            );
        }
        let (rf, cf) = (dump_frame(&self.r.frame, &self.r.ll), self.c.dump_frame());
        if rf != cf {
            let i = rf.iter().zip(&cf).position(|(a, b)| a != b).unwrap_or(0);
            panic!("{what}: frame differs at {i}: port {:?} C {:?}", &rf[i..(i + 6).min(rf.len())], &cf[i..(i + 6).min(cf.len())]);
        }
        assert_eq!(self.r.ssc.as_bytes()[..], self.c.ssc().as_bytes()[..], "{what}: config");
        let (rq, cq) = (dump_qd(&self.r.qd), self.c.dump_qd());
        if rq != cq {
            let i = rq.iter().zip(&cq).position(|(a, b)| a != b).unwrap_or(rq.len().min(cq.len()));
            panic!(
                "{what}: QMF domain differs at {i}: port {:?} C {:?} (lengths {} {})",
                &rq[i.saturating_sub(2)..(i + 6).min(rq.len())],
                &cq[i.saturating_sub(2)..(i + 6).min(cq.len())],
                rq.len(),
                cq.len()
            );
        }
    }

    /// Changes the plain members of the spatial decoder with `f` (on a copy of the port's
    /// state) and loads the result into both sides.
    pub fn set_state(&mut self, f: impl FnOnce(&mut SpatialDec)) {
        let mut s = (*self.r.sd).clone();
        f(&mut s);
        let v = dump_self(&s);
        self.load(&v);
    }

    /// Changes the frame and the lossless states with `f` and loads the result into both
    /// sides.
    pub fn set_frame(&mut self, f: impl FnOnce(&mut SpatialBsFrame, &mut BsLlState)) {
        let (mut fr, mut ll) = (self.r.frame.clone(), self.r.ll);
        f(&mut fr, &mut ll);
        let v = dump_frame(&fr, &ll);
        self.load_frame(&v);
    }
}

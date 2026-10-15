// State dump of the port's whole SBR encoder: the same sequence of values as
// sbrdump_sbr_encoder() in shim/sbrenc_sbr_encoder_dump.cpp writes for SBR_ENCODER.

#![allow(dead_code)]

use fdk_aac_rs_sbrenc::sbr_encoder::MAX_PAYLOAD_SIZE;
use fdk_aac_rs_fdk::qmf::QmfFilterBank;
use fdk_aac_rs_sbrenc::env_est::*;
use fdk_aac_rs_sbrenc::resampler::Downsampler;
use fdk_aac_rs_sbrenc::sbr::*;
use fdk_aac_rs_sbrenc::sbr_encoder::SbrConfigData;

use super::sbr_encoder_dump::*;

fn qmf(d: &mut Dump, q: &QmfFilterBank) {
    d.put(!q.p_filter.is_empty() as i32);
    if !q.p_filter.is_empty() {
        d.put(q.p_filter[0] as i32);
        d.put(q.p_filter[1] as i32);
    }
    d.put(q.filter_states as i32);
    d.put(q.filter_size);
    d.put(!q.t_cos.is_empty() as i32);
    if !q.t_cos.is_empty() {
        d.put(q.t_cos[1] as i32);
    }
    d.put(!q.t_sin.is_empty() as i32);
    if !q.t_sin.is_empty() {
        d.put(q.t_sin[1] as i32);
    }
    d.put(q.filter_scale);
    d.put(q.no_channels);
    d.put(q.no_col);
    d.put(q.lsb);
    d.put(q.usb);
    d.put(q.syn_scalefactor);
    d.put(q.out_scalefactor);
    d.put(q.out_gain_m);
    d.put(q.out_gain_e);
    d.put(q.flags as i32);
    d.put(q.p_stride as i32);
}

fn downsampler(d: &mut Dump, ds: &Downsampler) {
    for s in ds.down_filter.states.iter() {
        d.puts(s);
    }
    d.put(ds.down_filter.coeffa.is_some() as i32);
    if let Some(c) = ds.down_filter.coeffa {
        d.put(c[0] as i32);
    }
    d.put(ds.down_filter.gain);
    d.put(ds.down_filter.wc);
    d.put(ds.down_filter.no_coeffs);
    d.put(ds.down_filter.ptr);
    d.put(ds.ratio);
    d.put(ds.delay);
    d.put(ds.pending);
}

fn extract(d: &mut Dump, x: &SbrExtractEnvelope) {
    for i in 0..32 {
        d.put((x.r_buffer + 64 * i) as i32);
    }
    for i in 0..32 {
        d.put((x.i_buffer + 64 * i) as i32);
    }
    d.put(!x.p_y_buffer.is_empty() as i32);
    d.puts(&x.p_y_buffer);
    for i in 0..32 {
        if i < Y_BUFFER_STATIC_ROWS {
            d.put(0);
            d.put((64 * i) as i32);
        } else {
            d.put(1);
            d.put((x.y_buffer_dyn + 64 * (i - Y_BUFFER_STATIC_ROWS)) as i32);
        }
    }
    d.puts(&x.y_buffer_scale);
    d.putu8(&x.envelope_compensation);
    d.putu8(&x.pre_transient_info);
    d.put(x.y_buffer_write_offset);
    d.put(x.y_buffer_sz_shift);
    d.put(x.r_buffer_read_offset);
    d.put(x.no_cols);
    d.put(x.no_rows);
    d.put(x.start_index);
    d.put(x.time_slots);
    d.put(x.time_step);
}

fn env_channel(d: &mut Dump, h: &EnvChannel, grid_owner: i32) {
    fast_tran_det(d, &h.sbr_fast_transient_detector);
    tran_det(d, &h.sbr_transient_detector);
    code_env(d, &h.sbr_code_envelope);
    code_env(d, &h.sbr_code_noise_floor);
    extract(d, &h.sbr_extract_envelope);
    env_frame(d, &h.sbr_env_frame);
    ton_corr(d, &h.ton_corr);
    env_data(d, &h.enc_env_data);
    d.put(grid_owner);
    d.put(h.qmf_scale);
    d.put(h.f_level_protect as i32);
}

fn config_data(d: &mut Dump, c: &SbrConfigData) {
    d.put(c.sbr_syntax_flags as i32);
    d.put(c.n_channels);
    d.puts(&c.n_sfb);
    d.put(c.num_master);
    d.put(c.sample_freq);
    d.put(c.frame_size);
    d.put(c.x_over_freq);
    d.put(c.dyn_x_over_freq);
    d.put(c.no_qmf_bands);
    d.put(c.no_qmf_slots);
    d.put(1);
    d.putu8(&c.freq_band_table_lo);
    d.put(1);
    d.putu8(&c.freq_band_table_hi);
    d.put(1);
    d.putu8(&c.v_k_master);
    d.put(c.stereo_mode);
    d.put(c.no_env_channels);
    d.put(c.use_wave_coding);
    d.put(c.use_parametric_coding);
    d.put(c.xpos_ctrl_switch);
    d.put(c.switch_transposers);
    d.put(c.init_amp_res_ff as i32);
    d.put(c.threshold_amp_res_ff_m);
    d.put(c.threshold_amp_res_ff_e as i32);
}

/// `sbrdump_sbr_encoder()`
pub fn sbr_encoder(d: &mut Dump, e: &SbrEncoder) {
    // hSbrBSGrid of every channel: the global index of the channel that owns the grid
    let mut owner = [-1i32; MAX_SBR_ELEMENTS];
    for s in e.sbr_element.iter().flatten() {
        for ch in 0..2 {
            if let Some(gi) = s.sbr_channel[ch] {
                if let Some(c) = e.p_sbr_channel[gi].as_deref() {
                    owner[gi] = match c.h_env_channel.enc_env_data.h_sbr_bs_grid {
                        None => -1,
                        Some(k) => s.sbr_channel.get(k).copied().flatten().map(|x| x as i32).unwrap_or(-2),
                    };
                }
            }
        }
    }
    for el in e.sbr_element.iter() {
        d.put(el.is_some() as i32);
        let Some(s) = el.as_deref() else {
            continue;
        };
        for ch in 0..2 {
            d.put(s.sbr_channel[ch].map(|x| x as i32).unwrap_or(-1));
        }
        for ch in 0..2 {
            d.put(s.h_qmf_analysis[ch].map(|x| x as i32).unwrap_or(-1));
        }
        config_data(d, &s.sbr_config_data);
        header_data(d, &s.sbr_header_data);
        bitstream_data(d, &s.sbr_bitstream_data);
        cmon_data(d, &s.cmon_data, &s.payload_delay_line);
        d.puts(&s.dyn_x_over_freq_delay);
        d.put(s.el_info.el_type);
        d.put(s.el_info.bit_rate);
        d.put(s.el_info.instance_tag);
        d.put(s.el_info.f_parametric_stereo as i32);
        d.put(s.el_info.f_dual_mono as i32);
        d.put(s.el_info.n_channels_in_el as i32);
        d.putu8(&s.el_info.channel_index);
        for row in s.payload_delay_line.to_vec().chunks(MAX_PAYLOAD_SIZE) {
            d.putu8(row);
        }
        for v in s.payload_delay_line_size.iter() {
            d.put(*v as i32);
        }
    }
    for (k, c) in e.p_sbr_channel.iter().enumerate() {
        d.put(c.is_some() as i32);
        let Some(c) = c.as_deref() else {
            continue;
        };
        env_channel(d, &c.h_env_channel, owner[k]);
        downsampler(d, &c.down_sampler);
    }
    for q in 0..MAX_SBR_ELEMENTS {
        qmf(d, &e.qmf_analysis[q]);
        if e.qmf_analysis[q].filter_states {
            d.0.extend(e.qmf_analysis_states[q].iter().map(|&x| x as i32));
        }
    }
    downsampler(d, &e.lfe_down_sampler);
    d.puts(&[
        e.lfe_ch_idx,
        e.no_elements,
        e.n_channels,
        e.frame_size,
        e.buffer_offset,
        e.downsampled_offset,
        e.downmix_size,
        e.down_sample_factor,
        e.downsampling_method,
        e.n_bitstr_delay,
        e.sbr_dec_delay,
        e.estimate_bitrate,
        e.input_data_delay,
    ]);
    d.put(!e.dynamic_ram.is_empty() as i32);
    d.puts(&e.dynamic_ram);
    d.put(e.h_parametric_stereo.is_some() as i32);
    if let Some(p) = e.h_parametric_stereo.as_deref() {
        parametric_stereo(d, p);
    }
    qmf(d, &e.qmf_synthesis_ps);
    if e.qmf_synthesis_ps.filter_states {
        d.puts(&e.qmf_synthesis_ps_states);
    }
    d.put(e.max_elements);
    d.put(e.max_channels);
    d.put(e.support_ps);
}

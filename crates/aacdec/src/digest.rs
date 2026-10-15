// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the state digest of the decoder (see fdk_aac_rs_fdk::digest).

//! The digest of the complete decoder state (not in C; see `fdk_aac_rs_fdk::digest`), for the
//! chunked parallel decoding of whole files (`fdk_aac_rs::chunked`).
//!
//! [`AacDecoderInstance::state_digest`] feeds every field of the decoder instance and of
//! everything it owns (transport decoder with its input buffer, channel memory, concealment,
//! SBR/PS, MPEG Surround, DRC, MPEG-D DRC, QMF domain, PCM downmix, limiter, ancillary data),
//! with these exceptions, each of which cannot influence the future output:
//!
//! | Not fed | Why |
//! |---|---|
//! | `blockNumber` | only incremented, never read |
//! | `streamInfo.numTotalBytes`, `numBadBytes`, `numTotalAccessUnits`, `numBadAccessUnits` | only accumulated (or reset) and reported; the chunked decoding reports them as the sequential decoder does, by adding the difference at the boundary |
//! | `TDNoiseSeed`, `TDNoiseStates` of the concealment | read only for the comfort noise of the time domain fading, which is applied only while a channel is concealed or fades in: a hazard ([`HazardProbe`]). The seed advances in every frame, so it would never be equal |
//! | the per-frame channel memory (`pAacDecoderChannelInfo[]` with `pDynData`, `pComData`, `workBufferCore2`), except for USAC, RSVD50, RSV603DA, DRM and ER-RVLC (whose decoding reads persistent data there) | the data of the current frame of every channel, written by the parsing of the frame before it is read. Only members that are written with the same value in every frame of a configuration (or at its initialization) are read before they are written, and earlier values otherwise only in frames with errors (concealment) or a re-initialization, which are hazards. This is the property that the frame pipeline relies on for its double-buffered channel memory, checked there by poisoning the other set (`crate::pipeline::state`); for the chunked decoding by poisoning the speculative runs (`FDK_AAC_RS_CHUNK_POISON`, [`AacDecoderInstance::poison_unfed_state`]) |
//! | the overlap buffer of the AAC filter bank beyond its live part | overwritten before it is read (`fdk_aac_rs_fdk::digest::digest_mdct_imlt`; not for ELD, USAC, RSVD50 and RSV603DA, which use the buffer otherwise) |
//! | the frame pipeline (`pipe`, `d_gen`) | not in C; a decoder with a pipeline is never compared (only whether it has one is fed) |
//! | the per-channel parallelism (`chan_par`: thread pool, poisoning mode and seed) | not in C; no decoding state (the per-task scratch is written before it is read, checked by poisoning it) |
//!
//! Some state is fed in a normalized form, independent of where data sits in a ring buffer
//! (all accesses are relative to the ring positions): the input buffer of the transport decoder
//! (`fdk_aac_rs_fdk::digest`), and the limiter windows (`fdk_aac_rs_pcmutils`), whose layout
//! matters only when the attack time or the channel count change without a reset, a hazard.
//! Inactive CRC regions contribute only that they are inactive (their positions are written
//! when they are started).
//!
//! Everything else is fed as it is, scratch memory included (which can only make two states
//! differ that would decode identically).

use fdk_aac_rs_fdk::audio::{
    AC_DRM, AC_ELD, AC_ER_RVLC, AC_RSV603DA, AC_RSVD50, AC_SBR_PRESENT, AC_USAC,
};
use fdk_aac_rs_fdk::digest::{digest_mdct_imlt, Digest, Digester, StateDigest};
use fdk_aac_rs_fdk::impl_state_digest;

/// The configurations whose per-frame channel memory is fed (see the module documentation).
const FRAME_MEMORY_FLAGS: u32 = AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_DRM | AC_ER_RVLC;
/// The configurations that use the overlap buffer of the IMDCT otherwise than the AAC filter
/// bank (the ELD filter bank, the LPD and FAC of USAC).
const NOT_IMLT_FLAGS: u32 = AC_ELD | AC_USAC | AC_RSVD50 | AC_RSV603DA;

use crate::aacdec_drc_types::{CDrcChannelData, CDrcInfo, CDrcParams};
use crate::aacdec_hcr_types::{
    CErHcrInfo, HcrInputOutput, HcrNonPcwSideinfo, HcrSectionInfo, HcrSegmentInfo,
};
use crate::aacdecoder::{AacDecoderInstance, AncBuffer, CAncData, CUsacCoreExtensions, SbrParams};
use crate::aacdecoder_lib::CStreamInfo;
use crate::channelinfo::*;
use crate::conceal_types::{CConcealParams, CConcealmentInfo, ConcealState_Ok};
use crate::delay::FdkSignalDelay;
use crate::rvlc_info::CErRvlcInfo;
use crate::usacdec_acelp::{CAcelpChannelData, CAcelpStaticMem};

impl StateDigest for AacDecoderInstance {
    fn digest(&self, d: &mut Digester) {
        let AacDecoderInstance {
            aac_channels,
            asc_channels,
            block_number: _,
            nr_of_layers,
            output_interleaved,
            aac_out_data_headroom,
            h_input,
            sampling_rate_info,
            frame_ok,
            flags,
            el_flags,
            elements,
            el_tags,
            ch_mapping,
            map_descr,
            ch_map_index,
            sbr_data_len,
            pce,
            stream_info,
            p_aac_decoder_channel_info,
            p_aac_decoder_static_channel_info,
            p_com_data,
            p_com_static_data,
            com_data,
            com_static_data,
            p_work_buffer_core1,
            work_buffer_core1,
            work_buffer_core2,
            p_time_data2,
            time_data2_size,
            cpe_static_data,
            conceal_common_data,
            conceal_method_user,
            usac_core_ext,
            num_usac_elements,
            usac_stereo_config_index,
            p_usac_config,
            nb_div,
            use_ld_qmf_time_align,
            aac_channels_prev,
            channel_type_prev,
            channel_indices_prev,
            downscale_factor,
            downscale_factor_in_bs,
            h_sbr_decoder,
            sbr_enabled,
            sbr_enabled_prev,
            ps_possible,
            sbr_params,
            p_drm_bs_buffer,
            drm_bs_buffer_size,
            qmf_domain,
            qmf_mode_curr,
            qmf_mode_user,
            h_drc_info,
            metadata_expiry,
            p_mpeg_surround_decoder,
            mps_enable_user,
            mps_enable_curr,
            mps_applicable,
            mps_output_mode,
            mps_out_channels_last,
            mps_frame_size_last,
            anc_data,
            h_pcm_utils,
            h_limiter,
            limiter_enable_user,
            limiter_enable_curr,
            ext_gain,
            ext_gain_delay,
            h_uni_drc_decoder,
            multiband_drc_present,
            num_time_slots,
            loudness_info_set_position,
            default_target_loudness,
            p_time_data_flush,
            flush_status,
            flush_cnt,
            build_up_status,
            build_up_cnt,
            has_audio_pre_roll,
            preroll_au_length,
            access_unit,
            apply_crossfade,
            usac_residual_delay,
            pipe,
            d_gen: _,
            chan_par: _,
        } = self;
        d.field("aac_channels", aac_channels);
        d.field("asc_channels", asc_channels);
        d.field("nr_of_layers", nr_of_layers);
        d.field("output_interleaved", output_interleaved);
        d.field("aac_out_data_headroom", aac_out_data_headroom);
        d.field("h_input", h_input);
        d.field("sampling_rate_info", sampling_rate_info);
        d.field("frame_ok", frame_ok);
        d.field("flags", flags);
        d.field("el_flags", el_flags);
        d.field("elements", elements);
        d.field("el_tags", el_tags);
        d.field("ch_mapping", ch_mapping);
        d.field("map_descr", map_descr);
        d.field("ch_map_index", ch_map_index);
        d.field("sbr_data_len", sbr_data_len);
        d.field("pce", pce);
        d.field("stream_info", stream_info);
        let frame_memory = flags[0] & FRAME_MEMORY_FLAGS != 0;
        if frame_memory {
            d.field("p_aac_decoder_channel_info", p_aac_decoder_channel_info);
        } else {
            d.field_with("p_aac_decoder_channel_info", |d| {
                for c in p_aac_decoder_channel_info {
                    c.is_some().digest(d);
                }
            });
        }
        let imlt = flags[0] & NOT_IMLT_FLAGS == 0;
        if !negative_control() {
            d.field_with("p_aac_decoder_static_channel_info", |d| {
                for (i, s) in p_aac_decoder_static_channel_info.iter().enumerate() {
                    d.element(i, &s.as_deref().map(|s| StaticInfo(s, imlt)));
                }
            });
        }
        d.field("p_com_data", p_com_data);
        d.field("p_com_static_data", p_com_static_data);
        if frame_memory {
            d.field("com_data", com_data);
        } else {
            d.field("com_data", &com_data.len());
        }
        d.field("com_static_data", com_static_data);
        d.field("p_work_buffer_core1", p_work_buffer_core1);
        d.field("work_buffer_core1", work_buffer_core1);
        if frame_memory {
            d.field("work_buffer_core2", work_buffer_core2);
        } else {
            d.field("work_buffer_core2", &work_buffer_core2.as_ref().map(|w| w.len()));
        }
        d.field("p_time_data2", p_time_data2);
        d.field("time_data2_size", time_data2_size);
        d.field("cpe_static_data", cpe_static_data);
        d.field("conceal_common_data", conceal_common_data);
        d.field("conceal_method_user", conceal_method_user);
        d.field("usac_core_ext", usac_core_ext);
        d.field("num_usac_elements", num_usac_elements);
        d.field("usac_stereo_config_index", usac_stereo_config_index);
        d.field("p_usac_config", p_usac_config);
        d.field("nb_div", nb_div);
        d.field("use_ld_qmf_time_align", use_ld_qmf_time_align);
        d.field("aac_channels_prev", aac_channels_prev);
        d.field("channel_type_prev", channel_type_prev);
        d.field("channel_indices_prev", channel_indices_prev);
        d.field("downscale_factor", downscale_factor);
        d.field("downscale_factor_in_bs", downscale_factor_in_bs);
        d.field("h_sbr_decoder", h_sbr_decoder);
        d.field("sbr_enabled", sbr_enabled);
        d.field("sbr_enabled_prev", sbr_enabled_prev);
        d.field("ps_possible", ps_possible);
        d.field("sbr_params", sbr_params);
        d.field("p_drm_bs_buffer", p_drm_bs_buffer);
        d.field("drm_bs_buffer_size", drm_bs_buffer_size);
        d.field("qmf_domain", qmf_domain);
        d.field("qmf_mode_curr", qmf_mode_curr);
        d.field("qmf_mode_user", qmf_mode_user);
        d.field("h_drc_info", h_drc_info);
        d.field("metadata_expiry", metadata_expiry);
        d.field("p_mpeg_surround_decoder", p_mpeg_surround_decoder);
        d.field("mps_enable_user", mps_enable_user);
        d.field("mps_enable_curr", mps_enable_curr);
        d.field("mps_applicable", mps_applicable);
        d.field("mps_output_mode", mps_output_mode);
        d.field("mps_out_channels_last", mps_out_channels_last);
        d.field("mps_frame_size_last", mps_frame_size_last);
        d.field("anc_data", anc_data);
        d.field("h_pcm_utils", h_pcm_utils);
        d.field("h_limiter", h_limiter);
        d.field("limiter_enable_user", limiter_enable_user);
        d.field("limiter_enable_curr", limiter_enable_curr);
        d.field("ext_gain", ext_gain);
        d.field("ext_gain_delay", ext_gain_delay);
        d.field("h_uni_drc_decoder", h_uni_drc_decoder);
        d.field("multiband_drc_present", multiband_drc_present);
        d.field("num_time_slots", num_time_slots);
        d.field("loudness_info_set_position", loudness_info_set_position);
        d.field("default_target_loudness", default_target_loudness);
        d.field("p_time_data_flush", p_time_data_flush);
        d.field("flush_status", flush_status);
        d.field("flush_cnt", flush_cnt);
        d.field("build_up_status", build_up_status);
        d.field("build_up_cnt", build_up_cnt);
        d.field("has_audio_pre_roll", has_audio_pre_roll);
        d.field("preroll_au_length", preroll_au_length);
        d.field("access_unit", access_unit);
        d.field("apply_crossfade", apply_crossfade);
        d.field("usac_residual_delay", usac_residual_delay);
        d.field("pipe", &pipe.is_some());
    }
}

/// Negative control of the digest (feature `test-hooks`, environment variable
/// `FDK_AAC_RS_DIGEST_NEGATIVE`): the persistent channel memory (IMDCT overlap, concealment, PNS
/// seeds, ...) is left out of the digest, so that the chunked decoding takes boundaries where
/// the decoders differ.
fn negative_control() -> bool {
    if !cfg!(feature = "test-hooks") {
        return false;
    }
    static NEG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NEG.get_or_init(|| std::env::var_os("FDK_AAC_RS_DIGEST_NEGATIVE").is_some())
}

/// Depth of the field traces of [`AacDecoderInstance::state_trace`].
pub const TRACE_DEPTH: usize = 12;

impl AacDecoderInstance {
    /// Not in C: the digest of the complete decoder state (see the [module documentation](self)).
    /// Two decoders with equal digests return the same output for the same calls, up to the
    /// first hazard ([`HazardProbe`]).
    pub fn state_digest(&self) -> Digest {
        let mut d = Digester::new();
        self.digest(&mut d);
        d.finish()
    }

    /// Not in C: [`AacDecoderInstance::state_digest`] with the digests of all fields up to
    /// `depth` levels, for `fdk_aac_rs_fdk::digest::diff_traces` (the digest differs from the one
    /// of `state_digest()`).
    pub fn state_trace(&self, depth: usize) -> (Digest, Vec<(String, Digest)>) {
        let mut d = Digester::traced(depth);
        self.digest(&mut d);
        d.finish_traced()
    }

    /// Not in C: the state that decides whether a decoder call is a hazard of the state digest
    /// (see [`HazardProbe::hazard`]).
    pub fn hazard_probe(&self) -> HazardProbe {
        let concealing = self.p_aac_decoder_static_channel_info.iter().flatten().any(|s| {
            s.concealment_info.conceal_state != ConcealState_Ok
                || s.concealment_info.conceal_state_old != ConcealState_Ok
        });
        HazardProbe {
            concealing,
            limiter: self.h_limiter.as_deref().map(|l| l.digest_layout()),
            gen: self.d_gen,
        }
    }

    /// Not in C: whether the states of two decoders of this stream that started at different
    /// access units rarely become equal: SBR (the noise and harmonic phase indices run on) and
    /// USAC (the noise filling seeds, the LPD state). The chunked decoding gives such a stream up
    /// sooner; the digest still decides every boundary.
    pub fn converges_rarely(&self) -> bool {
        self.flags[0] & (AC_SBR_PRESENT | AC_USAC) != 0
    }

    /// Not in C, for tests of the digest (`FDK_AAC_RS_CHUNK_POISON`): fills the state that is
    /// not fed to the digest with a pattern derived from `seed` (see the module documentation):
    /// the comfort noise state of the concealment, the per-frame channel memory (where it is not
    /// fed), the overlap buffer of the AAC filter bank beyond its live part, the positions of
    /// inactive CRC regions, the CRC value of an ADTS header without CRC, the fetch position of
    /// the input buffer and the frame counter. The
    /// stream statistics are not changed (they are reported). A decoder whose output changes
    /// after this in a call that is not a hazard has state that the digest misses.
    pub fn poison_unfed_state(&mut self, seed: u64) {
        let mut x = seed | 1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        if self.flags[0] & FRAME_MEMORY_FLAGS == 0 {
            crate::pipeline::state::poison_channel_memory(self, next());
        }
        let imlt = self.flags[0] & NOT_IMLT_FLAGS == 0;
        for s in self.p_aac_decoder_static_channel_info.iter_mut().flatten() {
            let c = &mut s.concealment_info;
            c.td_noise_seed = next() as u32;
            for v in c.td_noise_states.iter_mut() {
                *v = next() as i32;
            }
            let m = &mut s.i_mdct;
            let n = m.overlap.len();
            if imlt && m.ov_size as usize == n && m.ov_offset >= 0 && m.prev_tl > 0 {
                let a = m.ov_offset as usize;
                let b = (m.prev_tl / 2) as usize;
                if a <= n && b <= n && a <= n - b {
                    for v in &mut m.overlap[a..n - b] {
                        *v = next() as i32;
                    }
                }
            }
        }
        self.block_number = next() as i32;
        if let Some(tp) = self.h_input.as_deref_mut() {
            for crc in [&mut tp.parser.adts.crc_info, &mut tp.parser.drm.crc_info] {
                for r in crc.crc_reg_data.iter_mut().filter(|r| r.is_active == 0) {
                    r.max_bits = next() as i32;
                    r.bit_buf_cnt_bits = next() as i32;
                    r.valid_bits = next() as i32;
                }
            }
            if tp.parser.adts.bs.protection_absent != 0 {
                tp.parser.adts.crc_read_value = next() as u16;
            }
            let bb = &mut tp.bit_stream[0].h_bit_buf;
            bb.write_offset = (next() as u32) & bb.buf_size().wrapping_sub(1);
        }
    }
}

/// Not in C: what makes a decoder call a hazard of the state digest, i.e. a call whose output
/// may depend on state that is not part of the digest (or only up to a symmetry). Two decoders
/// with equal digests decode identically up to the first call that is a hazard; the conditions
/// themselves are part of the digest, so both decoders reach it at the same call.
///
/// - The call returns an error other than `AAC_DEC_NOT_ENOUGH_BITS` (also a decode error with
///   concealed output): the decoding of a defective frame may read per-frame channel memory of
///   an earlier frame, which is not fed.
/// - A channel is concealed, or fades in after concealment, before or after the call (state
///   other than ok, now or in the previous frame): the time domain fading adds comfort noise
///   from `TDNoiseSeed` and `TDNoiseStates`, which are not fed. (A superset: the noise is
///   added only with a nonzero comfort noise level, and only in the frames where the time
///   domain fading runs; the concealment state changes once per frame, at most.)
/// - The channel memory is (re)initialized during the call (`CAacDecoder_Init()` with a
///   configuration change, `CAacDecoder_DeInit()`): memory is then reinterpreted, for example
///   the whole overlap buffer of the filter bank, of which only the live part is fed.
/// - The attack time, the channel count or the buffer sizes of the limiter change during the
///   call: the ring buffers are then read in their physical layout, which is not fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HazardProbe {
    concealing: bool,
    limiter: Option<(u32, u32, usize, usize)>,
    gen: u32,
}

impl HazardProbe {
    /// Whether a call with the probes `before` and `after` it that returned `ret` is a hazard.
    pub fn hazard(before: &HazardProbe, after: &HazardProbe, ret: i32) -> bool {
        use crate::aacdecoder_lib::{AAC_DEC_NOT_ENOUGH_BITS, AAC_DEC_OK};
        (ret != AAC_DEC_OK && ret != AAC_DEC_NOT_ENOUGH_BITS)
            || before.concealing
            || after.concealing
            || before.limiter != after.limiter
            || before.gen != after.gen
    }
}

/// `streamInfo`: the statistics `numTotalBytes`, `numBadBytes`, `numTotalAccessUnits` and
/// `numBadAccessUnits` are not fed (see the module documentation).
impl StateDigest for CStreamInfo {
    fn digest(&self, d: &mut Digester) {
        let CStreamInfo {
            sample_rate,
            frame_size,
            num_channels,
            p_channel_type,
            p_channel_indices,
            aac_sample_rate,
            profile,
            aot,
            channel_config,
            bit_rate,
            aac_samples_per_frame,
            aac_num_channels,
            ext_aot,
            ext_sampling_rate,
            output_delay,
            flags,
            ep_config,
            num_lost_access_units,
            num_total_bytes: _,
            num_bad_bytes: _,
            num_total_access_units: _,
            num_bad_access_units: _,
            drc_prog_ref_lev,
            drc_pres_mode,
            output_loudness,
        } = self;
        d.field("sample_rate", sample_rate);
        d.field("frame_size", frame_size);
        d.field("num_channels", num_channels);
        d.field("p_channel_type", p_channel_type);
        d.field("p_channel_indices", p_channel_indices);
        d.field("aac_sample_rate", aac_sample_rate);
        d.field("profile", profile);
        d.field("aot", aot);
        d.field("channel_config", channel_config);
        d.field("bit_rate", bit_rate);
        d.field("aac_samples_per_frame", aac_samples_per_frame);
        d.field("aac_num_channels", aac_num_channels);
        d.field("ext_aot", ext_aot);
        d.field("ext_sampling_rate", ext_sampling_rate);
        d.field("output_delay", output_delay);
        d.field("flags", flags);
        d.field("ep_config", ep_config);
        d.field("num_lost_access_units", num_lost_access_units);
        d.field("drc_prog_ref_lev", drc_prog_ref_lev);
        d.field("drc_pres_mode", drc_pres_mode);
        d.field("output_loudness", output_loudness);
    }
}

/// The ancillary data: the buffer contents (written by the decoder) and its bookkeeping.
impl StateDigest for CAncData {
    fn digest(&self, d: &mut Digester) {
        let CAncData { buffer, buffer_size, offset, nr_elements } = self;
        d.field_with("buffer", |d| match buffer {
            AncBuffer::None => d.u32(0),
            AncBuffer::Owned(b) => {
                d.u32(1);
                b.digest(d);
            }
            AncBuffer::Raw(r) => {
                d.u32(2);
                let n = (*buffer_size).max(0) as usize;
                if n > 0 {
                    r.as_slice()[..n.min(r.as_slice().len())].digest(d);
                }
            }
        });
        d.field("buffer_size", buffer_size);
        d.field("offset", offset);
        d.field("nr_elements", nr_elements);
    }
}

impl StateDigest for FacDataPtr {
    fn digest(&self, d: &mut Digester) {
        match *self {
            FacDataPtr::Null => d.u32(0),
            FacDataPtr::FacData0 => d.u32(1),
            FacDataPtr::Spectral(o) => {
                d.u32(2);
                d.u64(o as u64);
            }
            FacDataPtr::Scratch => d.u32(3),
        }
    }
}

impl_state_digest!(FdkSignalDelay { delay_line, delay, num_channels });
impl_state_digest!(SbrParams { bs_delay });
impl_state_digest!(CUsacCoreExtensions {
    usac_ext_element_type,
    usac_ext_element_default_length,
    usac_ext_element_payload_frag,
});
impl_state_digest!(CDrcInfo {
    params,
    enable,
    digital_norm,
    update,
    num_out_channels,
    prev_aac_num_channels,
    num_payloads,
    num_threads,
    prog_ref_level,
    prog_ref_level_present,
    prl_expiry_count,
    pres_mode,
    dvb_anc_data_available,
    dvb_anc_data_position,
    drc_payload_position,
    apply_ext_gain,
    additional_gain_prev,
    additional_gain_filter_state,
    additional_gain_filter_state1,
});
impl_state_digest!(CDrcParams {
    usr_cut,
    usr_boost,
    usr_apply_heavy_compression,
    cut,
    boost,
    target_ref_level,
    apply_heavy_compression,
    expiry_frame,
    bs_delay_enable,
    default_presentation_mode,
    encoder_target_level,
});
impl_state_digest!(CDrcChannelData {
    expiry_count,
    num_bands,
    band_top,
    drc_interpolation_scheme,
    drc_value,
    drc_data_type,
});
impl_state_digest!(CConcealParams {
    fade_out_factor,
    fade_in_factor,
    method,
    num_fade_out_frames,
    num_fade_in_frames,
    num_mute_release_frames,
    comfort_noise_level,
});
impl_state_digest!(CConcealmentInfo {
    spectral_coefficient,
    spec_scale,
    i_random_phase,
    prev_frame_ok,
    cnt_valid_frames,
    cnt_fade_frames,
    win_grp_offset,
    att_grp_offset,
    last_render_mode,
    window_shape,
    window_sequence,
    last_win_grp_len,
    conceal_state,
    conceal_state_old,
    fade_old,
    last_fading_type,
    a_rvlc_previous_scale_factor,
    a_rvlc_previous_codebook,
    rvlc_previous_scale_factor_ok,
    rvlc_previous_block_type,
    lsf4,
    last_tcx_gain,
    last_tcx_gain_e,
    td_noise_coef,
    td_noise_att,
} skip {
    // the comfort noise of the time domain fading (a hazard, see HazardProbe)
    td_noise_seed,
    td_noise_states,
});
impl_state_digest!(CpePersistentData { joint_stereo_persistent_data });
impl_state_digest!(CJointStereoPersistentData {
    clear_spectral_coeffs,
    alpha_q_re_prev,
    alpha_q_im_prev,
    win_seq_prev,
    win_shape_prev,
    win_groups_prev,
});
impl_state_digest!(CWorkBufferCore1 { buf });
impl_state_digest!(CAacDecoderCommonStaticData { cplx_prediction_data });
impl_state_digest!(CCplxPredictionData {
    pred_dir,
    igf_pred_dir,
    complex_coef,
    use_prev_frame,
    alpha_q_re,
    alpha_q_im,
});
impl_state_digest!(CAacDecoderCommonData {
    pns_inter_channel_data,
    pns_random_seed,
    joint_stereo_data,
    overlay,
});
impl_state_digest!(CAacDecoderCommonDataOverlay { aac });
impl_state_digest!(CAacDecoderCommonDataOverlayAac {
    er_hcr_info,
    er_rvlc_info,
    a_rvlc_scf_esc,
    a_rvlc_scf_fwd,
    a_rvlc_scf_bwd,
});
impl_state_digest!(CErRvlcInfo {
    sf_concealment,
    rev_global_gain,
    length_of_rvlc_sf,
    dpcm_noise_nrg,
    sf_escapes_present,
    length_of_rvlc_escapes,
    dpcm_noise_last_position,
    dpcm_is_last_position,
    length_of_rvlc_sf_fwd,
    length_of_rvlc_sf_bwd,
    p_rvl_bit_cnt_rvl,
    p_bitstr_indx_rvl_rvl,
    num_window_groups,
    max_sfb_transmitted,
    first_noise_group,
    first_noise_band,
    direction,
    bs_anchor,
    bitstream_index_rvl_fwd,
    bitstream_index_rvl_bwd,
    bitstream_index_esc,
    p_huff_tree_rvl_codewds,
    p_huff_tree_rvlc_escape,
    num_decoded_escape_words_fwd,
    num_decoded_escape_words_bwd,
    num_decoded_escape_words_esc,
    noise_used,
    intensity_used,
    sf_used,
    first_scf,
    last_scf,
    first_nrg,
    last_nrg,
    first_is,
    last_is,
    error_log_rvlc,
    conceal_min,
    conceal_max,
    conceal_min_esc,
    conceal_max_esc,
});
impl_state_digest!(CErHcrInfo { dec_in_out, segment_info, section_info, non_pcw_sideinfo });
impl_state_digest!(HcrNonPcwSideinfo {
    i_node,
    i_result_pointer,
    p_escape_sequence_info,
    codeword_offset,
    p_state,
    p_codebook,
    p_cnt_sign,
    p_sta,
});
impl_state_digest!(HcrSectionInfo {
    num_codeword,
    num_sorted_section,
    p_num_codeword_in_section,
    p_num_sorted_codeword_in_section,
    p_num_extended_sorted_codeword_in_section,
    num_extended_sorted_codeword_in_section_idx,
    p_num_extended_sorted_sections_in_sets,
    num_extended_sorted_sections_in_sets_idx,
    p_reorder_offset,
    p_sorted_codebook,
    p_extended_sorted_codebook,
    extended_sorted_codebook_idx,
    p_max_len_of_cb_in_ext_srt_sec,
    max_len_of_cb_in_ext_srt_sec_idx,
    p_codebook_switch,
});
impl_state_digest!(HcrSegmentInfo {
    num_segment,
    p_segment_bitfield,
    p_codeword_bitfield,
    segment_offset,
    p_left_start_of_segment,
    p_right_start_of_segment,
    p_remaining_bits_in_segment,
    read_direction,
    num_word_for_bitfield,
    p_num_bit_valid_in_last_word,
});
impl_state_digest!(HcrInputOutput {
    error_log,
    quantized_spectral_coefficients_idx,
    length_of_reordered_spectral_data,
    num_section,
    bitstream_anchor,
    length_of_longest_codeword,
});
impl_state_digest!(CJointStereoData {
    ms_mask_present,
    ms_used,
    igf_ms_mask_present,
    cplx_pred_flag,
    igf_cplx_pred_flag,
    store_dmx_re_prev,
    store_dmx_re_prev_e,
});
impl_state_digest!(CPnsInterChannelData { correlated });
impl StateDigest for CAacDecoderStaticChannelInfo {
    fn digest(&self, d: &mut Digester) {
        self.digest_with(d, false);
    }
}

/// The persistent channel memory, with the IMDCT state of the AAC filter bank or not.
struct StaticInfo<'a>(&'a CAacDecoderStaticChannelInfo, bool);

impl StateDigest for StaticInfo<'_> {
    fn digest(&self, d: &mut Digester) {
        self.0.digest_with(d, self.1);
    }
}

impl CAacDecoderStaticChannelInfo {
    /// The digest of the persistent channel memory; with `imlt`, the IMDCT state is fed with
    /// only the live part of its overlap buffer (`fdk_aac_rs_fdk::digest::digest_mdct_imlt`; for
    /// the AAC filter bank, see [`AacDecoderInstance::state_digest`]).
    fn digest_with(&self, d: &mut Digester, imlt: bool) {
        let CAacDecoderStaticChannelInfo {
            i_mdct,
            h_ar_co,
            pns_current_seed,
            old_synth,
            old_t_pf,
            old_gain_pf,
            mem_bpf,
            old_bpf_control_info,
            last_core_mode,
            last_lpd_mode,
            last_last_lpd_mode,
            last_lpc_lost,
            lpc4_lsf,
            lsf_adaptive_mean,
            lp_coeff_old,
            lp_coeff_old_exp,
            old_stability,
            num_lost_lpd_frames,
            last_tcx_gain,
            last_tcx_gain_e,
            last_alfd_gains,
            last_tcx_pitch,
            last_tcx_noise_factor,
            acelp,
            nf_random_seed,
            drc_data,
            concealment_info,
            p_cpe_static_data,
        } = self;
        if imlt {
            d.field_with("i_mdct", |d| digest_mdct_imlt(i_mdct, d));
        } else {
            d.field("i_mdct", i_mdct);
        }
        d.field("h_ar_co", h_ar_co);
        d.field("pns_current_seed", pns_current_seed);
        d.field("old_synth", old_synth);
        d.field("old_t_pf", old_t_pf);
        d.field("old_gain_pf", old_gain_pf);
        d.field("mem_bpf", mem_bpf);
        d.field("old_bpf_control_info", old_bpf_control_info);
        d.field("last_core_mode", last_core_mode);
        d.field("last_lpd_mode", last_lpd_mode);
        d.field("last_last_lpd_mode", last_last_lpd_mode);
        d.field("last_lpc_lost", last_lpc_lost);
        d.field("lpc4_lsf", lpc4_lsf);
        d.field("lsf_adaptive_mean", lsf_adaptive_mean);
        d.field("lp_coeff_old", lp_coeff_old);
        d.field("lp_coeff_old_exp", lp_coeff_old_exp);
        d.field("old_stability", old_stability);
        d.field("num_lost_lpd_frames", num_lost_lpd_frames);
        d.field("last_tcx_gain", last_tcx_gain);
        d.field("last_tcx_gain_e", last_tcx_gain_e);
        d.field("last_alfd_gains", last_alfd_gains);
        d.field("last_tcx_pitch", last_tcx_pitch);
        d.field("last_tcx_noise_factor", last_tcx_noise_factor);
        d.field("acelp", acelp);
        d.field("nf_random_seed", nf_random_seed);
        d.field("drc_data", drc_data);
        d.field("concealment_info", concealment_info);
        d.field("p_cpe_static_data", p_cpe_static_data);
    }
}
impl_state_digest!(CAcelpStaticMem {
    old_exc_mem,
    old_syn_mem,
    a,
    a_exp,
    gc_threshold,
    de_emph_mem,
    past_gpit,
    past_gcode,
    old_t0,
    old_t0_frac,
    deemph_mem_wsyn,
    wsyn_rms,
    seed_ace,
});
impl_state_digest!(CAacDecoderChannelInfo {
    data,
    spec_scale,
    ics_info,
    granule_length,
    element_instance_tag,
    render_mode,
    p_dyn_data,
    curr_aliasing_symmetry,
});
impl_state_digest!(CAacDecoderDynamicData {
    a_scale_factor,
    a_sfb_scale,
    a_code_book,
    band_is_noise,
    tns_data,
    raw_data_info,
    specific_to,
});
impl_state_digest!(CAacDecoderDynamicDataSpecificTo { aac, usac });
impl_state_digest!(CAacDecoderDynamicDataUsac {
    fd_noise_level_and_offset,
    tns_active,
    tns_on_lr,
    tcx_noise_factor,
    tcx_global_gain,
});
impl_state_digest!(CAacDecoderDynamicDataAac {
    pulse_data,
    a_num_line_in_sec4_hcr,
    a_code_books4_hcr,
    len_of_reordered_spectral_data,
    len_of_longest_codeword,
    number_section,
    rvlc_current_scale_factor_ok,
    rvlc_intensity_used,
});
impl_state_digest!(CPulseData {
    pulse_data_present,
    number_pulse,
    pulse_start_band,
    pulse_offset,
    pulse_amp,
});
impl_state_digest!(CRawDataInfo { common_window, global_gain });
impl_state_digest!(CTnsData { filter, number_of_filters, data_present, active, gain_ld });
impl_state_digest!(CFilter { coeff, start_band, stop_band, direction, resolution, order });
impl_state_digest!(CIcsInfo {
    window_group_length,
    window_groups,
    valid,
    window_shape,
    window_sequence,
    max_sf_bands,
    max_sfb_ste,
    scale_factor_grouping,
    total_sf_bands,
});
impl_state_digest!(CAacDecoderChannelInfoData { usac, aac });
impl_state_digest!(CAacDecoderChannelInfoAac { pns_data });
impl_state_digest!(CPnsData { pns_used, current_energy, pns_active, current_seed });
impl_state_digest!(CAacDecoderChannelInfoUsac {
    fac_data0,
    fac_data_e,
    fac_data,
    core_mode,
    core_mode_last,
    lpd_mode_last,
    r#mod,
    bpf_control_info,
    lsp_coeff,
    lp_coeff,
    lp_coeff_exp,
    lsf_adaptive_mean_cand,
    a_stability,
    acelp,
    tcx_gain,
    tcx_gain_e,
});
impl_state_digest!(CAcelpChannelData {
    acelp_core_mode,
    mean_energy,
    t0,
    t0_frac,
    ltp_filtering_flag,
    icb_index,
    gains,
});
impl_state_digest!(SamplingRateInfo {
    scale_factor_bands_long,
    scale_factor_bands_short,
    number_of_scale_factor_bands_long,
    number_of_scale_factor_bands_short,
    sampling_rate_index,
    sampling_rate,
});

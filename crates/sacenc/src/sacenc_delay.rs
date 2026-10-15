// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_delay.h, libSACenc/src/sacenc_delay.cpp.

//! Port of `sacenc_delay.h` and `sacenc_delay.cpp`: the delay management of the encoder.
//!
//! The code is identical in the `aarch64` and `x86` builds. The delays are `INT` sums of
//! caller values (the core coder delay, the time alignment) and wrap like the reference. x86
//! division by zero: the arm64 result (quotient 0) on both variants.

use crate::sacenc_lib::*;

/// `MAX_DELAY_INPUT`
pub const MAX_DELAY_INPUT: i32 = 1024;
/// `MAX_DELAY_OUTPUT`
pub const MAX_DELAY_OUTPUT: i32 = 4096;
/// `MAX_DELAY_SURROUND_ANALYSIS`: should be equal to or larger than the dual rate SBR resampler
/// filter length.
pub const MAX_DELAY_SURROUND_ANALYSIS: i32 = 5;
/// `MAX_BITSTREAM_DELAY`
pub const MAX_BITSTREAM_DELAY: i32 = 1;

/// `struct DELAY_CONFIG`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DelayConfig {
    // Routing Config Switches
    pub b_dmx_align: i32,
    pub b_time_dom_dmx: i32,
    pub b_minimize_delay: i32,
    pub b_sac_time_alignment_dynamic_out: i32,

    // Needed Input Variables
    pub n_qmf_len: i32,
    pub n_frame_len: i32,
    pub n_surround_delay: i32,
    pub n_arb_dmx_delay: i32,
    pub n_limiter_delay: i32,
    pub n_core_coder_delay: i32,
    pub n_sac_stream_mux_delay: i32,
    /// Overwritten, if bSacTimeAlignmentDynamicOut
    pub n_sac_time_alignment: i32,
}

/// `struct DELAY`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delay {
    pub config: DelayConfig,

    // Variable Delaybuffers -> Delays
    pub n_dmx_align_buffer: i32,
    pub n_surround_analysis_buffer: i32,
    pub n_arb_dmx_analysis_buffer: i32,
    pub n_output_audio_buffer: i32,
    pub n_bitstream_frame_buffer: i32,
    pub n_output_audio_qmf_frame_buffer: i32,
    pub n_discard_out_frames: i32,

    // Variable Delaybuffers Computation Variables
    pub n_bitstream_frame_buffer_size: i32,

    // Output: Infos
    /// Delay of the downmixed signal after the space encoder
    pub n_info_dmx_delay: i32,
    /// Delay of the whole en-/decoder including CoreCoder
    pub n_info_codec_delay: i32,
    /// Delay of the Mpeg Surround decoder
    pub n_info_decoder_delay: i32,
}

/// `HANDLE_DELAY`
pub type HandleDelay = Option<Box<Delay>>;

/// `a / b` in `INT` as arm64 computes it (`sdiv`: 0 for `b == 0`, wrapping for
/// `INT_MIN / -1`).
#[inline]
fn div(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `fdk_sacenc_delay_Open(phDelay)`
pub fn fdk_sacenc_delay_open(ph_delay: Option<&mut HandleDelay>) -> FdkSacencError {
    match ph_delay {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = Some(Box::default());
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_delay_Close(phDelay)`
pub fn fdk_sacenc_delay_close(ph_delay: Option<&mut HandleDelay>) -> FdkSacencError {
    match ph_delay {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = None;
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_delay_Init(hDelay, nQmfLen, nFrameLen, nCoreCoderDelay, nSacStreamMuxDelay)`
pub fn fdk_sacenc_delay_init(
    h_delay: &mut Delay,
    n_qmf_len: i32,
    n_frame_len: i32,
    n_core_coder_delay: i32,
    n_sac_stream_mux_delay: i32,
) -> FdkSacencError {
    // Fill structure before calculation
    h_delay.config = DelayConfig::default();

    h_delay.config.n_qmf_len = n_qmf_len;
    h_delay.config.n_frame_len = n_frame_len;
    h_delay.config.n_core_coder_delay = n_core_coder_delay;
    h_delay.config.n_sac_stream_mux_delay = n_sac_stream_mux_delay;

    SACENC_OK
}

/// `fdk_sacenc_delay_SubCalulateBufferDelays(hDel)`: calculates the delays of the buffers.
pub fn fdk_sacenc_delay_sub_calulate_buffer_delays(h_del: &mut Delay) -> FdkSacencError {
    if h_del.config.b_sac_time_alignment_dynamic_out > 0 {
        h_del.config.n_sac_time_alignment = 0;
    }

    let cfg = h_del.config;
    let q = cfg.n_qmf_len;
    let fl = cfg.n_frame_len;

    // Only Ld-QMF Delay, no hybrid
    let n_encoder_an_delay = (2i32.wrapping_mul(q)).wrapping_add(q / 2);
    let n_encoder_syn_delay = q.wrapping_add(q / 2);
    let n_decoder_an_delay = (2i32.wrapping_mul(q)).wrapping_add(q / 2);
    let n_decoder_syn_delay = q.wrapping_add(q / 2);
    // WindowLookahead is just half a frame
    let n_encoder_win_delay = fl / 2;

    let n_residual_coder_frame_delay: i32 = 0;
    let n_arb_dmx_residual_coder_frame_delay: i32 = 0;

    // Calculate variable Buffer-Delays
    if cfg.b_time_dom_dmx == 0 {
        // ArbitraryDmx and TdDmx off
        h_del.n_surround_analysis_buffer = 0;
        h_del.n_arb_dmx_analysis_buffer = 0;
        let mut temp_delay = n_encoder_syn_delay
            .wrapping_add(cfg.n_limiter_delay)
            .wrapping_add(cfg.n_core_coder_delay)
            .wrapping_add(cfg.n_sac_time_alignment)
            .wrapping_add(n_decoder_an_delay);
        temp_delay = (n_residual_coder_frame_delay.wrapping_mul(fl))
            .wrapping_add(cfg.n_sac_stream_mux_delay)
            .wrapping_sub(temp_delay);

        if temp_delay > 0 {
            h_del.n_bitstream_frame_buffer = 0;
            h_del.n_output_audio_buffer = temp_delay;
        } else {
            temp_delay = temp_delay.wrapping_neg();
            h_del.n_bitstream_frame_buffer = div(temp_delay.wrapping_add(fl).wrapping_sub(1), fl);
            h_del.n_output_audio_buffer =
                (h_del.n_bitstream_frame_buffer.wrapping_mul(fl)).wrapping_sub(temp_delay);
        }

        h_del.n_output_audio_qmf_frame_buffer =
            div(h_del.n_output_audio_buffer.wrapping_add(q / 2).wrapping_sub(1), q);

        if cfg.b_dmx_align > 0 {
            let temp_delay = n_encoder_win_delay
                .wrapping_add(n_encoder_an_delay)
                .wrapping_add(n_encoder_syn_delay)
                .wrapping_add(h_del.n_output_audio_buffer)
                .wrapping_add(cfg.n_limiter_delay)
                .wrapping_add(cfg.n_core_coder_delay);
            h_del.n_discard_out_frames = div(temp_delay.wrapping_add(fl).wrapping_sub(1), fl);
            h_del.n_dmx_align_buffer =
                h_del.n_discard_out_frames.wrapping_mul(fl).wrapping_sub(temp_delay);
        } else {
            h_del.n_discard_out_frames = 0;
            h_del.n_dmx_align_buffer = 0;
        }

        // Output: Info-Variables
        h_del.n_info_dmx_delay = h_del
            .n_surround_analysis_buffer
            .wrapping_add(n_encoder_an_delay)
            .wrapping_add(n_encoder_win_delay)
            .wrapping_add(n_encoder_syn_delay)
            .wrapping_add(h_del.n_output_audio_buffer)
            .wrapping_add(cfg.n_limiter_delay);
        h_del.n_info_codec_delay = h_del
            .n_info_dmx_delay
            .wrapping_add(cfg.n_core_coder_delay)
            .wrapping_add(cfg.n_sac_time_alignment)
            .wrapping_add(n_decoder_an_delay)
            .wrapping_add(n_decoder_syn_delay);
    } else {
        // ArbitraryDmx or TdDmx on
        let mut temp_delay1 = cfg.n_arb_dmx_delay.wrapping_sub(cfg.n_surround_delay);

        if temp_delay1 >= 0 {
            h_del.n_surround_analysis_buffer = temp_delay1;
            h_del.n_arb_dmx_analysis_buffer = 0;
        } else {
            h_del.n_surround_analysis_buffer = 0;
            h_del.n_arb_dmx_analysis_buffer = temp_delay1.wrapping_neg();
        }

        // Surround Path
        temp_delay1 = n_encoder_win_delay
            .wrapping_add(cfg.n_surround_delay)
            .wrapping_add(h_del.n_surround_analysis_buffer)
            .wrapping_add(n_encoder_an_delay);
        // ArbDmx Compare Path
        let temp_delay2 = n_encoder_win_delay
            .wrapping_add(cfg.n_arb_dmx_delay)
            .wrapping_add(h_del.n_arb_dmx_analysis_buffer)
            .wrapping_add(n_encoder_an_delay);
        // ArbDmx Passthrough
        let temp_delay3 = cfg
            .n_arb_dmx_delay
            .wrapping_add(cfg.n_limiter_delay)
            .wrapping_add(cfg.n_core_coder_delay)
            .wrapping_add(cfg.n_sac_time_alignment)
            .wrapping_add(n_decoder_an_delay);

        let mut temp_delay12 =
            n_residual_coder_frame_delay.max(n_arb_dmx_residual_coder_frame_delay).wrapping_mul(fl);
        temp_delay12 = temp_delay12.wrapping_add(cfg.n_sac_stream_mux_delay);

        if temp_delay1 > temp_delay2 {
            temp_delay12 = temp_delay12.wrapping_add(temp_delay1);
        } else {
            temp_delay12 = temp_delay12.wrapping_add(temp_delay2);
        }

        if temp_delay3 > temp_delay12 {
            let d = temp_delay3.wrapping_sub(temp_delay12);
            if cfg.b_minimize_delay > 0 {
                h_del.n_bitstream_frame_buffer = div(d, fl); // floor
                h_del.n_output_audio_buffer = 0;
                let rest = d.wrapping_sub(h_del.n_bitstream_frame_buffer.wrapping_mul(fl));
                h_del.n_surround_analysis_buffer =
                    h_del.n_surround_analysis_buffer.wrapping_add(rest);
                h_del.n_arb_dmx_analysis_buffer =
                    h_del.n_arb_dmx_analysis_buffer.wrapping_add(rest);
            } else {
                h_del.n_bitstream_frame_buffer = div(d.wrapping_add(fl).wrapping_sub(1), fl);
                h_del.n_output_audio_buffer = h_del
                    .n_bitstream_frame_buffer
                    .wrapping_mul(fl)
                    .wrapping_add(temp_delay12)
                    .wrapping_sub(temp_delay3);
            }
        } else {
            h_del.n_bitstream_frame_buffer = 0;
            h_del.n_output_audio_buffer = temp_delay12.wrapping_sub(temp_delay3);
        }

        if cfg.b_dmx_align > 0 {
            let temp_delay = cfg
                .n_arb_dmx_delay
                .wrapping_add(h_del.n_output_audio_buffer)
                .wrapping_add(cfg.n_limiter_delay)
                .wrapping_add(cfg.n_core_coder_delay);
            h_del.n_discard_out_frames = div(temp_delay.wrapping_add(fl).wrapping_sub(1), fl);
            h_del.n_dmx_align_buffer =
                h_del.n_discard_out_frames.wrapping_mul(fl).wrapping_sub(temp_delay);
        } else {
            h_del.n_discard_out_frames = 0;
            h_del.n_dmx_align_buffer = 0;
        }

        // Output: Info-Variables
        h_del.n_info_dmx_delay = cfg
            .n_arb_dmx_delay
            .wrapping_add(h_del.n_output_audio_buffer)
            .wrapping_add(cfg.n_limiter_delay);
        h_del.n_info_codec_delay = h_del
            .n_info_dmx_delay
            .wrapping_add(cfg.n_core_coder_delay)
            .wrapping_add(cfg.n_sac_time_alignment)
            .wrapping_add(n_decoder_an_delay)
            .wrapping_add(n_decoder_syn_delay);
        h_del.n_info_decoder_delay = n_decoder_an_delay.wrapping_add(n_decoder_syn_delay);
    }

    // Additonal Variables needed for Computation Issues
    h_del.n_bitstream_frame_buffer_size = h_del.n_bitstream_frame_buffer.wrapping_add(1);

    SACENC_OK
}

/// `assignParameterInRange(startRange, stopRange, value, ptr)` (static in C)
fn assign_parameter_in_range(
    start_range: i32,
    stop_range: i32,
    value: i32,
    ptr: &mut i32,
) -> FdkSacencError {
    if (start_range <= value) && (value <= stop_range) {
        *ptr = value;
        SACENC_OK
    } else {
        SACENC_INVALID_CONFIG
    }
}

/// `fdk_sacenc_delay_SetDmxAlign(hDelay, bDmxAlignIn)`
pub fn fdk_sacenc_delay_set_dmx_align(h_delay: &mut Delay, b_dmx_align_in: i32) -> FdkSacencError {
    assign_parameter_in_range(0, 1, b_dmx_align_in, &mut h_delay.config.b_dmx_align)
}

/// `fdk_sacenc_delay_SetTimeDomDmx(hDelay, bTimeDomDmxIn)`
pub fn fdk_sacenc_delay_set_time_dom_dmx(
    h_delay: &mut Delay,
    b_time_dom_dmx_in: i32,
) -> FdkSacencError {
    assign_parameter_in_range(0, 1, b_time_dom_dmx_in, &mut h_delay.config.b_time_dom_dmx)
}

/// `fdk_sacenc_delay_SetSacTimeAlignmentDynamicOut(hDelay, bSacTimeAlignmentDynamicOutIn)`
pub fn fdk_sacenc_delay_set_sac_time_alignment_dynamic_out(
    h_delay: &mut Delay,
    b_sac_time_alignment_dynamic_out_in: i32,
) -> FdkSacencError {
    assign_parameter_in_range(
        0,
        1,
        b_sac_time_alignment_dynamic_out_in,
        &mut h_delay.config.b_sac_time_alignment_dynamic_out,
    )
}

/// `fdk_sacenc_delay_SetNSacTimeAlignment(hDelay, nSacTimeAlignmentIn)`
pub fn fdk_sacenc_delay_set_n_sac_time_alignment(
    h_delay: &mut Delay,
    n_sac_time_alignment_in: i32,
) -> FdkSacencError {
    assign_parameter_in_range(
        -32768,
        32767,
        n_sac_time_alignment_in,
        &mut h_delay.config.n_sac_time_alignment,
    )
}

/// `fdk_sacenc_delay_SetMinimizeDelay(hDelay, bMinimizeDelay)`
pub fn fdk_sacenc_delay_set_minimize_delay(
    h_delay: &mut Delay,
    b_minimize_delay: i32,
) -> FdkSacencError {
    assign_parameter_in_range(0, 1, b_minimize_delay, &mut h_delay.config.b_minimize_delay)
}

/// `fdk_sacenc_delay_GetOutputAudioBufferDelay(hDelay)`
pub fn fdk_sacenc_delay_get_output_audio_buffer_delay(h_delay: &Delay) -> i32 {
    h_delay.n_output_audio_buffer
}

/// `fdk_sacenc_delay_GetSurroundAnalysisBufferDelay(hDelay)`
pub fn fdk_sacenc_delay_get_surround_analysis_buffer_delay(h_delay: &Delay) -> i32 {
    h_delay.n_surround_analysis_buffer
}

/// `fdk_sacenc_delay_GetArbDmxAnalysisBufferDelay(hDelay)`
pub fn fdk_sacenc_delay_get_arb_dmx_analysis_buffer_delay(h_delay: &Delay) -> i32 {
    h_delay.n_arb_dmx_analysis_buffer
}

/// `fdk_sacenc_delay_GetBitstreamFrameBufferSize(hDelay)`
pub fn fdk_sacenc_delay_get_bitstream_frame_buffer_size(h_delay: &Delay) -> i32 {
    h_delay.n_bitstream_frame_buffer_size
}

/// `fdk_sacenc_delay_GetDmxAlignBufferDelay(hDelay)`
pub fn fdk_sacenc_delay_get_dmx_align_buffer_delay(h_delay: &Delay) -> i32 {
    h_delay.n_dmx_align_buffer
}

/// `fdk_sacenc_delay_GetDiscardOutFrames(hDelay)`
pub fn fdk_sacenc_delay_get_discard_out_frames(h_delay: &Delay) -> i32 {
    h_delay.n_discard_out_frames
}

/// `fdk_sacenc_delay_GetInfoDmxDelay(hDelay)`
pub fn fdk_sacenc_delay_get_info_dmx_delay(h_delay: &Delay) -> i32 {
    h_delay.n_info_dmx_delay
}

/// `fdk_sacenc_delay_GetInfoCodecDelay(hDelay)`
pub fn fdk_sacenc_delay_get_info_codec_delay(h_delay: &Delay) -> i32 {
    h_delay.n_info_codec_delay
}

/// `fdk_sacenc_delay_GetInfoDecoderDelay(hDelay)`
pub fn fdk_sacenc_delay_get_info_decoder_delay(h_delay: &Delay) -> i32 {
    h_delay.n_info_decoder_delay
}

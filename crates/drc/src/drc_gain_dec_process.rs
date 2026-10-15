// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcGainDec_process.h, libDRCdec/src/drcGainDec_process.cpp.

//! Port of `drcGainDec_process.h`: interpolation of the linear gain nodes and their application
//! to time domain or subband domain signals.
//!
//! `FUNCTION_interpolateDrcGain_func1` is not defined in either reference build (checked in the
//! preprocessed sources), so the generic loop is ported.
//!
//! The audio buffers are slices; the C code addresses them with pointer arithmetic. Positions
//! outside of the slices are skipped.

use fdk_aac_rs_fdk::clz::{fixnorm_d, fixnormz_s};
use fdk_aac_rs_fdk::common_fix::DFRACT_BITS;
use fdk_aac_rs_fdk::common_fix::{cnt_leading_zeros, f_mult_div2, fl2fxconst_dbl, FixpDbl};
use fdk_aac_rs_fdk::scale::saturate_left_shift;

use crate::drc_dec_gain_decoder::*;
use crate::drc_dec_types::*;
use crate::drc_decoder::*;

/// `E_TGAINSTEP`
const E_TGAINSTEP: i32 = 12;

/// The target of `_processNodeSegments`/`_interpolateDrcGain`: an audio channel (a slice) or the
/// subband gains, and the index of the C pointer `buffer` in it (which may lie outside of it).
struct Buf<'a, 'b> {
    data: BufData<'a, 'b>,
    base: i64,
}

enum BufData<'a, 'b> {
    Slice(&'a mut [FixpDbl]),
    Subband(&'a mut SubbandGains<'b>),
}

impl Buf<'_, '_> {
    #[inline]
    fn get_mut(&mut self, i: i64) -> Option<&mut FixpDbl> {
        let k = self.base + i;
        match &mut self.data {
            BufData::Slice(s) => usize::try_from(k).ok().and_then(|k| s.get_mut(k)),
            BufData::Subband(g) => g.get_mut(k),
        }
    }
}

/// `_prepareLnbIndex(pActiveDrc, channelOffset, drcChannelOffset, numChannelsProcessed,
/// lnbPointer)`
///
/// A negative `channelOffset` has no defined reference result; the port returns `DE_NOT_OK`
/// (the library passes 0).
fn prepare_lnb_index(
    p_active_drc: &mut ActiveDrc,
    p_inst: &DrcInstructionsUniDrc,
    channel_offset: i32,
    drc_channel_offset: i32,
    num_channels_processed: i32,
    lnb_pointer: i32,
) -> DrcError {
    /* channelOffset: start index of physical channels
    numChannelsProcessed: number of processed channels, physical channels and DRC channels
    channelOffset + drcChannelOffset: start index of DRC channels, i.e. the channel order
    referenced in pInst.sequenceIndex */

    /* sanity checks */
    if channel_offset.wrapping_add(num_channels_processed) > 8 {
        return DE_NOT_OK;
    }

    if channel_offset.wrapping_add(drc_channel_offset).wrapping_add(num_channels_processed) > 8 {
        return DE_NOT_OK;
    }

    if channel_offset.wrapping_add(drc_channel_offset) < 0 {
        return DE_NOT_OK;
    }

    if channel_offset < 0 {
        return DE_NOT_OK;
    }

    /* prepare lnbIndexForChannel, a map of indices from each channel to its corresponding
     * linearNodeBuffer instance */
    for c in channel_offset..channel_offset + num_channels_processed {
        if p_inst.drc_set_id > 0 {
            let mut drc_channel = c + drc_channel_offset;
            /* fallback for configuration with more physical channels than DRC channels: reuse
            DRC gain of first channel. This is necessary for HE-AAC mono with stereo output */
            if drc_channel >= p_inst.drc_channel_count as i32 {
                drc_channel = 0;
            }
            let g = p_active_drc.channel_group_for_channel[drc_channel as usize] as i32;
            if (g >= 0) && p_active_drc.channel_group_is_parametric_drc[g as usize & 7] == 0 {
                p_active_drc.lnb_index_for_channel[c as usize][lnb_pointer as usize] = p_active_drc
                    .active_drc_offset
                    + p_active_drc.gain_element_for_group[g as usize & 7] as i32;
            }
        }
    }

    DE_OK
}

/// `_interpolateDrcGain(gainInterpolationType, timePrev, tGainStep, start, stop, stepsize,
/// gainLeft, gainRight, slopeLeft, slopeRight, buffer)`
///
/// The 32 bit products of the C code (`a * stepsize`, `a * n`, `LONG` = `INT` on LP64) can
/// overflow; both builds wrap.
fn interpolate_drc_gain(
    _gain_interpolation_type: GainInterpolationType,
    time_prev: i16,   /* time0 */
    t_gain_step: i16, /* time1 - time0 */
    start: i16,
    stop: i16,
    stepsize: i16,
    gain_left: FixpDbl,
    gain_right: FixpDbl,
    _slope_left: FixpDbl,
    _slope_right: FixpDbl,
    buffer: &mut Buf,
) -> DrcError {
    if t_gain_step < 0 {
        return DE_NOT_OK;
    }
    if t_gain_step == 0 {
        return DE_OK;
    }

    /* get start index offset and buffer index for downsampled interpolation */
    /* start_modulo = (start+timePrev)%stepsize; */
    /* stepsize is a power of 2 */
    let start_modulo = (start as i32 + time_prev as i32) & (stepsize as i32 - 1);
    let start_offset = if start_modulo != 0 { stepsize as i32 - start_modulo } else { 0 };
    /* n_buf = (start + timePrev + start_offset)/stepsize; */
    let n_buf = (start as i32 + time_prev as i32 + start_offset)
        .wrapping_shr((15 - fixnormz_s(stepsize)) as u32);

    {
        /* gainInterpolationType == GIT_LINEAR */
        /* runs = ceil((stop - start - start_offset)/stepsize). This works for stepsize = 2^N
         * only. */
        let runs = (stop as i32 - start as i32 - start_offset + stepsize as i32 - 1)
            .wrapping_shr((30 - fixnorm_d(stepsize as i32)) as u32);
        let mut n_min =
            (cnt_leading_zeros(gain_right).min(cnt_leading_zeros(gain_left)) - 1).min(8);
        let mut a: i32 = gain_right
            .wrapping_shl(n_min as u32)
            .wrapping_sub(gain_left.wrapping_shl(n_min as u32))
            .wrapping_div(t_gain_step as i32);
        let a_step = a.wrapping_mul(stepsize as i32);
        let n = start as i32 + start_offset;
        a = a.wrapping_mul(n).wrapping_add(gain_left.wrapping_shl(n_min as u32));
        buffer.base += n_buf as i64;
        a = a.wrapping_sub(a_step);
        n_min = 8 - n_min;
        for i in 0..runs.max(0) as i64 {
            a = a.wrapping_add(a_step);
            if let Some(s) = buffer.get_mut(i) {
                *s = f_mult_div2(*s, a).wrapping_shl(n_min as u32);
            }
        }
        buffer.base -= n_buf as i64;
    }
    DE_OK
}

/// `_processNodeSegments(frameSize, gainInterpolationType, nNodes, pNodeLin, offset, stepsize,
/// nodePrevious, channelGain, buffer)`
fn process_node_segments(
    frame_size: i32,
    gain_interpolation_type: GainInterpolationType,
    n_nodes: i32,
    lnb: &LinearNodeBuffer,
    row: usize,
    offset: i32,
    stepsize: i16,
    node_previous: NodeLin, /* the last node of the previous frame */
    channel_gain: FixpDbl,
    buffer: &mut Buf,
) -> DrcError {
    let mut err = DE_OK;
    let mut gain_lin: FixpDbl;
    let mut gain_lin_prev: FixpDbl;
    let mut slope_lin: FixpDbl = 0;
    let mut slope_lin_prev: FixpDbl;

    let mut time_prev = (node_previous.time as i32).wrapping_add(offset) as i16;
    gain_lin_prev = node_previous.gain_lin;
    for n in 0..n_nodes.clamp(0, 16) {
        let node = lnb.node(row, n);
        let time = (node.time as i32).wrapping_add(offset) as i16;
        let duration = (time as i32 - time_prev as i32) as i16;
        gain_lin = node.gain_lin;
        if channel_gain != fl2fxconst_dbl(1.0 / 256.0) {
            gain_lin = saturate_left_shift(f_mult_div2(gain_lin, channel_gain), 9, DFRACT_BITS);
        }

        if (time_prev as i32 >= (frame_size - 1)) || (time < 0) {
            /* This segment (between previous and current node) lies outside of this audio
             * frame */
            time_prev = time;
            gain_lin_prev = gain_lin;
            slope_lin_prev = slope_lin;
            let _ = slope_lin_prev;
            continue;
        }

        /* start and stop are the boundaries of the region of this segment that lie within this
        audio frame. Their values are relative to the beginning of this segment. stop is the
        first sample that isn't processed any more. */
        let start = (-(time_prev as i32)).max(1) as i16;
        let stop =
            (time.min(frame_size.wrapping_sub(1) as i16) as i32 - time_prev as i32 + 1) as i16;

        err = interpolate_drc_gain(
            gain_interpolation_type,
            time_prev,
            duration,
            start,
            stop,
            stepsize,
            gain_lin_prev,
            gain_lin,
            0,
            slope_lin,
            buffer,
        );
        if err != 0 {
            return err;
        }

        time_prev = time;
        gain_lin_prev = gain_lin;
    }
    let _ = (&mut slope_lin, E_TGAINSTEP);
    err
}

/// `processDrcTime(hGainDec, activeDrcIndex, delaySamples, channelOffset, drcChannelOffset,
/// numChannelsProcessed, timeDataChannelOffset, deinterleavedAudio)`: process DRC on
/// time-domain signal
pub fn process_drc_time(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    active_drc_index: usize,
    delay_samples: i32,
    channel_offset: i32,
    drc_channel_offset: i32,
    num_channels_processed: i32,
    time_data_channel_offset: i32,
    deinterleaved_audio: &mut [FixpDbl],
) -> DrcError {
    let lnb_pointer = h_gain_dec.drc_gain_buffers.lnb_pointer;
    let mut offset = 0;

    if h_gain_dec.delay_mode == DM_REGULAR_DELAY {
        offset = h_gain_dec.frame_size;
    }

    if delay_samples.wrapping_add(offset)
        > (NUM_LNB_FRAMES as i32 - 2).wrapping_mul(h_gain_dec.frame_size)
    {
        /* if delaySamples is too big, NUM_LNB_FRAMES should be increased */
        return DE_NOT_OK;
    }

    let Some(inst_idx) = h_gain_dec.active_drc[active_drc_index].inst() else {
        return DE_NOT_OK;
    };
    let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[inst_idx];
    let err = prepare_lnb_index(
        &mut h_gain_dec.active_drc[active_drc_index],
        p_inst,
        channel_offset,
        drc_channel_offset,
        num_channels_processed,
        lnb_pointer,
    );
    if err != 0 {
        return err;
    }

    let frame_size = h_gain_dec.frame_size;
    let is_channel_gain_drc = active_drc_index as i32 == h_gain_dec.channel_gain_active_drc_index;
    let p_active_drc = h_gain_dec.active_drc[active_drc_index];
    let p_drc_gain_buffers = &mut h_gain_dec.drc_gain_buffers;

    /* apply channelOffset */
    let mut buf = Buf {
        data: BufData::Slice(deinterleaved_audio),
        base: channel_offset as i64 * time_data_channel_offset as i64,
    };

    /* signal processing loop */
    // channelOffset >= 0 and channelOffset + numChannelsProcessed <= 8 (_prepareLnbIndex).
    for c in channel_offset as usize..(channel_offset + num_channels_processed) as usize {
        if is_channel_gain_drc {
            p_drc_gain_buffers.channel_gain[c][lnb_pointer as usize] = h_gain_dec.channel_gain[c];
        }

        let b = 0;
        {
            /* get pointer to oldest linearNodes */
            let mut lnb_ix = lnb_pointer + 1 - NUM_LNB_FRAMES as i32;
            while lnb_ix < 0 {
                lnb_ix += NUM_LNB_FRAMES as i32;
            }

            let mut channel_gain = if is_channel_gain_drc {
                p_drc_gain_buffers.channel_gain[c][lnb_ix as usize]
            } else {
                fl2fxconst_dbl(1.0 / 256.0)
            };

            /* Loop over all node buffers in linearNodeBuffer. All nodes which are not relevant
            for the current frame are sorted out inside _processNodeSegments. */
            for i in 0..NUM_LNB_FRAMES as i32 - 1 {
                /* Prepare previous node */
                let idx = p_active_drc.lnb_index_for_channel[c][lnb_ix as usize];
                let p_lnb_previous = if idx >= 0 {
                    p_drc_gain_buffers.lnb(idx + b)
                } else {
                    &p_drc_gain_buffers.dummy_lnb
                };
                let mut node_previous = p_lnb_previous
                    .node(lnb_ix as usize, p_lnb_previous.n_nodes[lnb_ix as usize] - 1);
                node_previous.time = (node_previous.time as i32).wrapping_sub(frame_size) as i16;
                if channel_gain != fl2fxconst_dbl(1.0 / 256.0) {
                    node_previous.gain_lin = saturate_left_shift(
                        f_mult_div2(
                            node_previous.gain_lin,
                            p_drc_gain_buffers.channel_gain[c][lnb_ix as usize],
                        ),
                        9,
                        DFRACT_BITS,
                    );
                }

                /* Prepare current linearNodeBuffer instance */
                lnb_ix += 1;
                if lnb_ix >= NUM_LNB_FRAMES as i32 {
                    lnb_ix = 0;
                }

                /* if lnbIndexForChannel changes over time, use the old indices for smooth
                 * transitions */
                let idx = p_active_drc.lnb_index_for_channel[c][lnb_ix as usize];
                /* lnbIndexForChannel = -1 means "no DRC processing", due to
                drcInstructionsIndex < 0, drcSetId < 0 or channel group < 0 */
                let p_lnb = if idx >= 0 {
                    p_drc_gain_buffers.lnb(idx + b)
                } else {
                    &p_drc_gain_buffers.dummy_lnb
                };

                if is_channel_gain_drc {
                    channel_gain = p_drc_gain_buffers.channel_gain[c][lnb_ix as usize];
                }

                /* number of frames of offset with respect to lnbPointer */
                let lnb_pointer_diff = i - (NUM_LNB_FRAMES as i32 - 2);

                let err = process_node_segments(
                    frame_size,
                    p_lnb.gain_interpolation_type,
                    p_lnb.n_nodes[lnb_ix as usize],
                    p_lnb,
                    lnb_ix as usize,
                    lnb_pointer_diff
                        .wrapping_mul(frame_size)
                        .wrapping_add(delay_samples)
                        .wrapping_add(offset),
                    1,
                    node_previous,
                    channel_gain,
                    &mut buf,
                );
                if err != 0 {
                    return err;
                }
            }
            buf.base += time_data_channel_offset as i64; /* proceed to next channel */
        }
    }
    DE_OK
}

/// `subbandGains[k][m]` with the flat addressing of C: `subbandGains` (12 rows of 16) is followed
/// by `dummySubbandGains` (16) in `DRC_GAIN_DECODER`, which the port reproduces. Positions past
/// that are skipped.
struct SubbandGains<'a> {
    rows: &'a mut [[FixpDbl; 16]; 12],
    dummy: &'a mut [FixpDbl; 16],
}

impl SubbandGains<'_> {
    fn get_mut(&mut self, flat: i64) -> Option<&mut FixpDbl> {
        if (0..192).contains(&flat) {
            Some(&mut self.rows[flat as usize / 16][flat as usize % 16])
        } else if (192..208).contains(&flat) {
            Some(&mut self.dummy[flat as usize - 192])
        } else {
            None
        }
    }

    fn get(&self, flat: i64) -> Option<FixpDbl> {
        if (0..192).contains(&flat) {
            Some(self.rows[flat as usize / 16][flat as usize % 16])
        } else if (192..208).contains(&flat) {
            Some(self.dummy[flat as usize - 192])
        } else {
            None
        }
    }
}

/// `processDrcSubband(hGainDec, activeDrcIndex, delaySamples, channelOffset, drcChannelOffset,
/// numChannelsProcessed, processSingleTimeslot, deinterleavedAudioReal,
/// deinterleavedAudioImag)`: process DRC on subband-domain signal
///
/// The QMF modes are compiled out in the reference (`(1024 / 256) >= (4096 / 64)` is false):
/// `SDM_QMF64` and `SDM_QMF71` return `DE_NOT_OK`.
pub fn process_drc_subband(
    h_gain_dec: &mut DrcGainDecoder,
    h_uni_drc_config: &UniDrcConfig,
    active_drc_index: usize,
    delay_samples: i32,
    channel_offset: i32,
    drc_channel_offset: i32,
    num_channels_processed: i32,
    process_single_timeslot: i32,
    deinterleaved_audio_real: &mut [&mut [FixpDbl]],
    deinterleaved_audio_imag: &mut [&mut [FixpDbl]],
) -> DrcError {
    let subband_domain_mode = h_gain_dec.subband_domain_supported;
    let mut signal_index = 0usize;
    let n_decoder_subbands;
    let l: i16; /* L: downsampling factor */
    let mut offset = 0;

    if h_gain_dec.delay_mode == DM_REGULAR_DELAY {
        offset = h_gain_dec.frame_size;
    }

    if delay_samples.wrapping_add(offset)
        > (NUM_LNB_FRAMES as i32 - 2).wrapping_mul(h_gain_dec.frame_size)
    {
        /* if delaySamples is too big, NUM_LNB_FRAMES should be increased */
        return DE_NOT_OK;
    }

    match subband_domain_mode {
        SDM_QMF64 | SDM_QMF71 => {
            /* QMF domain processing is not supported. */
            return DE_NOT_OK;
        }
        SDM_STFT256 => {
            n_decoder_subbands = SUBBAND_NUM_BANDS_STFT256;
            l = SUBBAND_DOWNSAMPLING_FACTOR_STFT256 as i16;
            /* analysisDelay = SUBBAND_ANALYSIS_DELAY_STFT256; */
        }
        _ => return DE_NOT_OK,
    }

    /* frameSizeSb = hGainDec->frameSize/L; */
    /* L is a power of 2 */
    let frame_size_sb = h_gain_dec.frame_size.wrapping_shr((15 - fixnormz_s(l)) as u32); /* timeslots per frame */

    let (m_start, m_stop) =
        if (process_single_timeslot < 0) || (process_single_timeslot >= frame_size_sb) {
            (0, frame_size_sb)
        } else {
            (process_single_timeslot, process_single_timeslot + 1)
        };

    let Some(inst_idx) = h_gain_dec.active_drc[active_drc_index].inst() else {
        return DE_NOT_OK;
    };
    let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[inst_idx];
    let lnb_pointer = h_gain_dec.drc_gain_buffers.lnb_pointer;
    let err = prepare_lnb_index(
        &mut h_gain_dec.active_drc[active_drc_index],
        p_inst,
        channel_offset,
        drc_channel_offset,
        num_channels_processed,
        lnb_pointer,
    );
    if err != 0 {
        return err;
    }

    let frame_size = h_gain_dec.frame_size;
    let is_channel_gain_drc = active_drc_index as i32 == h_gain_dec.channel_gain_active_drc_index;
    let p_active_drc = h_gain_dec.active_drc[active_drc_index];
    let active_drc_offset = p_active_drc.active_drc_offset;
    let mut subband_gains = SubbandGains {
        rows: &mut h_gain_dec.subband_gains,
        dummy: &mut h_gain_dec.dummy_subband_gains,
    };
    let p_linear_node_buffer = &h_gain_dec.drc_gain_buffers;

    if p_active_drc.subband_gains_ready == 0 {
        /* only for the first time per frame that processDrcSubband is called */
        /* write subbandGains */
        for g in 0..(p_inst.n_drc_channel_groups as usize).min(8) {
            let b = 0i32;
            {
                let p_lnb = p_linear_node_buffer
                    .lnb(active_drc_offset + p_active_drc.gain_element_for_group[g] as i32 + b);
                let row_base = (active_drc_offset as i64 + g as i64) * 16;

                for m in 0..frame_size_sb.max(0) as i64 {
                    if let Some(s) =
                        subband_gains.get_mut(row_base + b as i64 * frame_size_sb as i64 + m)
                    {
                        *s = fl2fxconst_dbl(1.0 / 128.0);
                    }
                }

                let mut lnb_ix = lnb_pointer - (NUM_LNB_FRAMES as i32 - 1);
                while lnb_ix < 0 {
                    lnb_ix += NUM_LNB_FRAMES as i32;
                }

                /* Loop over all node buffers in linearNodeBuffer. All nodes which are not
                relevant for the current frame are sorted out inside _processNodeSegments. */
                for i in 0..NUM_LNB_FRAMES as i32 - 1 {
                    /* Prepare previous node */
                    let mut node_previous =
                        p_lnb.node(lnb_ix as usize, p_lnb.n_nodes[lnb_ix as usize] - 1);
                    node_previous.time =
                        (node_previous.time as i32).wrapping_sub(frame_size) as i16;

                    lnb_ix += 1;
                    if lnb_ix >= NUM_LNB_FRAMES as i32 {
                        lnb_ix = 0;
                    }

                    /* number of frames of offset with respect to lnbPointer */
                    let lnb_pointer_diff = i - (NUM_LNB_FRAMES as i32 - 2);

                    let mut buf = Buf {
                        data: BufData::Subband(&mut subband_gains),
                        base: row_base + b as i64 * frame_size_sb as i64,
                    };
                    let err = process_node_segments(
                        frame_size,
                        p_lnb.gain_interpolation_type,
                        p_lnb.n_nodes[lnb_ix as usize],
                        p_lnb,
                        lnb_ix as usize,
                        lnb_pointer_diff
                            .wrapping_mul(frame_size)
                            .wrapping_add(delay_samples)
                            .wrapping_add(offset)
                            .wrapping_sub((l as i32 - 1) / 2),
                        l,
                        node_previous,
                        fl2fxconst_dbl(1.0 / 256.0),
                        &mut buf,
                    );
                    if err != 0 {
                        return err;
                    }
                }
            }
        }
        h_gain_dec.active_drc[active_drc_index].subband_gains_ready = 1;
    }

    // channelOffset >= 0 and channelOffset + numChannelsProcessed <= 8 (_prepareLnbIndex).
    for c in channel_offset as usize..(channel_offset + num_channels_processed) as usize {
        let g: i32 = if p_inst.drc_set_id > 0 {
            p_active_drc.channel_group_for_channel[(c as i32 + drc_channel_offset) as usize] as i32
        } else {
            -1
        };

        let mut real_pos = 0usize;
        let mut imag_pos = 0usize;

        let row: Option<i64> =
            if (g >= 0) && p_active_drc.channel_group_is_parametric_drc[g as usize & 7] == 0 {
                Some((active_drc_offset as i64 + g as i64) * 16)
            } else {
                None
            };

        for m in m_start..m_stop {
            let mut n_min: i32 = 8;
            {
                /* single-band DRC */
                let mut gain_sb = match row {
                    Some(r) => subband_gains.get(r + m as i64).unwrap_or(0),
                    None => h_gain_dec_dummy(&subband_gains, m),
                };
                if is_channel_gain_drc {
                    gain_sb = saturate_left_shift(
                        f_mult_div2(gain_sb, h_gain_dec.channel_gain[c]),
                        9,
                        DFRACT_BITS,
                    );
                }
                /* normalize gainSb for keeping signal precision */
                n_min = (cnt_leading_zeros(gain_sb) - 1).min(n_min);
                gain_sb = gain_sb.wrapping_shl(n_min as u32);
                n_min = 8 - n_min;
                if subband_domain_mode == SDM_STFT256 {
                    /* For STFT filterbank, real and imaginary parts are interleaved. */
                    if let Some(audio_real) = deinterleaved_audio_real.get_mut(signal_index) {
                        for _ in 0..n_decoder_subbands * 2 {
                            if let Some(s) = audio_real.get_mut(real_pos) {
                                *s = f_mult_div2(*s, gain_sb).wrapping_shl(n_min as u32);
                            }
                            real_pos += 1;
                        }
                    }
                } else {
                    for _ in 0..n_decoder_subbands {
                        if let Some(s) = deinterleaved_audio_real
                            .get_mut(signal_index)
                            .and_then(|a| a.get_mut(real_pos))
                        {
                            *s = f_mult_div2(*s, gain_sb).wrapping_shl(n_min as u32);
                        }
                        real_pos += 1;
                        if let Some(s) = deinterleaved_audio_imag
                            .get_mut(signal_index)
                            .and_then(|a| a.get_mut(imag_pos))
                        {
                            *s = f_mult_div2(*s, gain_sb).wrapping_shl(n_min as u32);
                        }
                        imag_pos += 1;
                    }
                }
            }
        }
        signal_index += 1;
    }
    DE_OK
}

/// `dummySubbandGains[m]` (with the flat addressing of [`SubbandGains`] past its 16 entries).
fn h_gain_dec_dummy(g: &SubbandGains, m: i32) -> FixpDbl {
    g.get(192 + m as i64).unwrap_or(0)
}

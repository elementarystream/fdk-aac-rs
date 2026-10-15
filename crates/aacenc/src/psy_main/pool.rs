// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not a port: the multi-threaded variant of FDKaacEnc_psyMain() (libAACenc/src/psy_main.cpp).

//! The psychoacoustic model with the opt-in multi-threading ([`fdk_aac_enc_psy_main_pool`]).
//!
//! The code is that of [`fdk_aac_enc_psy_main`], split in two parts:
//!
//! - [`psy_main_start`]: the checks, the block switching of the element and the window
//!   parameters (everything that can fail after it changed state, see [`psy_main_finish`]);
//! - [`psy_main_finish`]: the rest. For a channel pair with a pool, the per-channel loops between
//!   two coupled steps run as one task per channel (phases A to F below); the coupled steps run
//!   on the calling thread in their place.
//!
//! Why this is the sequential result: in the loops that are distributed, iteration `ch` reads and
//! writes only the state of channel `ch` (`psyStatic[ch]`, `psyData[ch]`, `tnsData[ch]`,
//! `pnsData[ch]`, `psyOutChannel[ch]`, the `QC_OUT_CHANNEL` of `ch`, `sfbTonality[ch]`) and the
//! configuration, which is only read. Consecutive loops over the channels that are not
//! separated by a coupled step are fused per channel (this only reorders independent work). The
//! values the sequential code combines across the channels (`zeroSpec`, `minSpecShift`, `maxNrg`,
//! `tnsSpecShift`) are combined after the tasks with the same operations (`||`, `min`, `max` do
//! not depend on the order). Phase A computes `sfbMaxScaleSpec` also when the spectrum of both
//! channels turns out to be zero: the zero branch then overwrites exactly the values it wrote.
//!
//! Errors: the only error after a state change in phase A is a failing transform. The block types
//! and frame lengths that make it fail are checked before the tasks start (the sequential loop
//! runs then, returning at the first failing channel as C does), except for the short-block
//! check of `FDKaacEnc_Transform_Real()` (different exponents of the eight windows), which cannot
//! fail: `mdct_block()` starts every window at exponent 2 and `dct_iv()` adds an amount that
//! depends only on the transform length, which is the same for the eight windows.

use super::*;
use crate::par::ThreadPool;

/// The window parameters of a frame of an element (the result of [`psy_main_start`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PsyFrame {
    n_time_samples: i32,
    block_switching_offset: i32,
    is_short_window: [i32; 2],
    this_conf: [usize; 2],
    window_length: [i32; 2],
    n_windows: [i32; 2],
    max_sfb: [i32; 2],
}

/// The first part of [`fdk_aac_enc_psy_main`] (see the [module documentation](self)): the checks,
/// the block switching and its synchronization, and the window parameters. Returns the error of
/// `FDKaacEnc_psyMain()` if it fails here.
pub(crate) fn psy_main_start(
    channels: i32,
    psy_static: &mut [PsyStatic],
    psy_conf: &[PsyConfiguration; 2],
    p_input: &[IntPcm],
    input_buf_size: u32,
    ch_idx: &[i32],
) -> Result<PsyFrame, AacEncoderError> {
    let common_window: i32 = 1;
    let n_ch = channels as usize;

    let mut this_conf = [0usize; 2];
    let mut window_length = [0i32; 2];
    let mut n_windows = [0i32; 2];
    let mut max_sfb = [0i32; 2];
    let mut is_short_window = [0i32; 2];

    // number of incoming time samples to be processed
    let n_time_samples = psy_conf[0].granule_length;

    let block_switching_offset: i32 = match psy_conf[0].filterbank {
        FB_LC => n_time_samples + (9 * n_time_samples / (2 * TRANS_FAC)),
        FB_LD | FB_ELD => n_time_samples,
        _ => return Err(AAC_ENC_UNSUPPORTED_FILTERBANK),
    };

    // (see fdk_aac_enc_psy_main)
    {
        let long = &psy_conf[0];
        let short = &psy_conf[1];
        if !(0..=n_time_samples).contains(&long.lowpass_line)
            || !(0..=n_time_samples).contains(&long.lowpass_line_lfe)
            || (long.filterbank == FB_LC
                && !(0..=n_time_samples / TRANS_FAC).contains(&short.lowpass_line))
        {
            return Err(AAC_ENC_UNKNOWN);
        }
    }

    let input_offset = |ch: usize, off: i32| -> usize {
        (ch_idx[ch] as i64 * input_buf_size as i64 + off as i64) as usize
    };

    // block switching
    if psy_conf[0].filterbank != FB_ELD {
        let mut p_time_signal = [[0 as IntPcm; 1024]; 2];
        let n = n_time_samples as usize;
        for ch in 0..n_ch {
            // copy input data and use for block switching
            let src = input_offset(ch, 0);
            p_time_signal[ch][..n].copy_from_slice(&p_input[src..src + n]);
        }
        if n_ch == 2 {
            // both channels at once (the same results as one call per channel)
            let (l, r) = pair(psy_static);
            fdk_aac_enc_block_switching_pair(
                [&mut l.block_switching_control, &mut r.block_switching_control],
                n_time_samples,
                [l.is_lfe, r.is_lfe],
                [&p_time_signal[0], &p_time_signal[1]],
            );
        }
        for ch in 0..n_ch {
            let st = &mut psy_static[ch];
            if n_ch != 2 {
                fdk_aac_enc_block_switching(
                    &mut st.block_switching_control,
                    n_time_samples,
                    st.is_lfe,
                    &p_time_signal[ch],
                );
            }

            // fill up internal input buffer, to 2xframelength samples
            let bso = block_switching_offset as usize;
            let len = (2 * n_time_samples - block_switching_offset) as usize;
            st.psy_input_buffer[bso..bso + len].copy_from_slice(&p_time_signal[ch][..len]);
        }

        // synch left and right block type
        let err = if channels > 1 {
            let (l, r) = pair(psy_static);
            fdk_aac_enc_sync_block_switching(
                &mut l.block_switching_control,
                Some(&mut r.block_switching_control),
                channels,
                common_window,
            )
        } else {
            fdk_aac_enc_sync_block_switching(
                &mut psy_static[0].block_switching_control,
                None,
                channels,
                common_window,
            )
        };

        if err != 0 {
            return Err(AAC_ENC_UNSUPPORTED_AOT); // mixed up LC and LD
        }
    } else {
        for ch in 0..n_ch {
            // copy input data and use for block switching
            let src = input_offset(ch, 0);
            let n = n_time_samples as usize;
            let bso = block_switching_offset as usize;
            psy_static[ch].psy_input_buffer[bso..bso + n].copy_from_slice(&p_input[src..src + n]);
        }
    }

    for ch in 0..n_ch {
        is_short_window[ch] =
            (psy_static[ch].block_switching_control.last_window_sequence == SHORT_WINDOW) as i32;
    }
    if (0..n_ch).any(|ch| is_short_window[ch] != 0) && psy_conf[1].granule_length != n_time_samples
    {
        STALE_SHORT_BLOCKS.with(|c| c.set(c.get() + 1));
    }
    // (see fdk_aac_enc_psy_main: a pair with one short and one long block is refused)
    if n_ch == 2 && is_short_window[0] != is_short_window[1] {
        MIXED_BLOCK_PAIRS.with(|c| c.set(c.get() + 1));
        return Err(AAC_ENC_UNKNOWN);
    }

    // set parameters according to window length
    for ch in 0..n_ch {
        if is_short_window[ch] != 0 {
            this_conf[ch] = 1;
            window_length[ch] = psy_conf[0].granule_length / TRANS_FAC;
            n_windows[ch] = TRANS_FAC;
            max_sfb[ch] = MAX_SFB_SHORT as i32;
        } else {
            this_conf[ch] = 0;
            window_length[ch] = psy_conf[0].granule_length;
            n_windows[ch] = 1;
            max_sfb[ch] = MAX_GROUPED_SFB as i32;
        }
    }

    Ok(PsyFrame {
        n_time_samples,
        block_switching_offset,
        is_short_window,
        this_conf,
        window_length,
        n_windows,
        max_sfb,
    })
}

/// The state of one channel in [`psy_main_finish`], with the values its tasks return.
struct Ch<'a> {
    ch: usize,
    st: &'a mut PsyStatic,
    pd: &'a mut PsyData,
    tns: &'a mut TnsData,
    pns: &'a mut PnsData,
    poc: &'a mut PsyOutChannel,
    qoc: &'a mut QcOutChannel,
    sfb_tonality: [FixpSgl; MAX_SFB_LONG],
    /// phase A: the transform failed
    failed: bool,
    /// phase A: a spectral line below the lowpass line is not 0
    nonzero: bool,
    /// phase A: the minimum of `sfbMaxScaleSpec`
    min_spec_shift: i32,
    /// phase B: the maximum band energy
    max_nrg: FixpDbl,
    /// phase D: `nrgScaling[ch]`
    nrg_scaling: i32,
    /// phase D: the maximum `nrgScaling` of the long block with TNS
    tns_spec_shift: i32,
    /// phase F: `maxSfbPerGroup[ch]`
    max_sfb_per_group: i32,
}

/// Runs `f` for every channel: as one task per channel on `pool` if given, otherwise in channel
/// order.
fn run<F: Fn(&mut Ch<'_>) + Sync>(pool: Option<&ThreadPool>, chs: &mut [Ch<'_>], f: F) {
    match pool {
        Some(pool) if chs.len() > 1 => crate::par::for_each_mut(Some(pool), chs, |_, c| f(c)),
        _ => chs.iter_mut().for_each(f),
    }
}

/// Whether the transform of the block type `block_type` with `frame_length` lines cannot fail
/// on the filter bank `filterbank` (see the [module documentation](self)).
fn transform_cannot_fail(filterbank: i32, block_type: i32, frame_length: i32) -> bool {
    if filterbank == FB_ELD {
        block_type == LONG_WINDOW && matches!(frame_length, 512 | 480 | 256 | 240 | 128 | 120)
    } else {
        matches!(block_type, LONG_WINDOW | STOP_WINDOW | START_WINDOW | SHORT_WINDOW)
    }
}

/// Whether [`psy_main_finish`] cannot fail after [`psy_main_start`] returned `frame` for the
/// statics `psy_static` (its only error is a failing transform, see the
/// [module documentation](self)).
pub(crate) fn psy_main_finish_cannot_fail(
    frame: &PsyFrame,
    psy_static: &[PsyStatic],
    psy_conf: &[PsyConfiguration; 2],
) -> bool {
    psy_static.iter().enumerate().all(|(ch, st)| {
        ch < 2
            && transform_cannot_fail(
                psy_conf[frame.this_conf[ch]].filterbank,
                st.block_switching_control.last_window_sequence,
                frame.n_time_samples,
            )
    })
}

/// Fills `d` with pseudo-random values (feature `verify-poison`: shows that the output does not
/// depend on what `PSY_DYNAMIC` holds before [`psy_main_finish`], which the parallel
/// psychoacoustics of the elements relies on; see `crate::aacenc`).
#[cfg(feature = "verify-poison")]
pub(crate) fn poison_psy_dynamic(d: &mut PsyDynamic, seed: u32) {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(0x9e37_79b9) | 1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x as i32
    };
    for pd in d.psy_data.iter_mut() {
        for a in [
            &mut pd.sfb_threshold[..],
            &mut pd.sfb_energy[..],
            &mut pd.sfb_energy_ld_data[..],
            &mut pd.sfb_max_scale_spec[..],
            &mut pd.sfb_energy_ms[..],
            &mut pd.sfb_energy_ms_ld_data[..],
            &mut pd.sfb_spread_energy[..],
            &mut pd.grouped_sfb_offset[..],
        ] {
            a.iter_mut().for_each(|v| *v = next());
        }
        pd.mdct_scale = next();
        pd.sfb_active = next();
        pd.lowpass_line = next();
    }
    for t in d.tns_data.iter_mut() {
        t.num_of_subblocks = next();
        t.data_raw.0.iter_mut().for_each(|v| *v = next());
        t.tns_max_scale_spec = next();
        t.filters_merged = next();
    }
    for p in d.pns_data.iter_mut() {
        p.noise_fuzzy_measure.iter_mut().for_each(|v| *v = next() as FixpSgl);
        p.noise_energy_correlation.iter_mut().for_each(|v| *v = next());
        p.pns_flag.iter_mut().for_each(|v| *v = next());
    }
}

/// [`fdk_aac_enc_psy_main`] with the opt-in multi-threading: with a pool, the per-channel parts of
/// a channel pair run as one task per channel (why this is exact: see the private module
/// `psy_main::pool`). Without a pool this is the straight port.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_psy_main_pool(
    channels: i32,
    psy_static: &mut [PsyStatic],
    psy_dynamic: &mut PsyDynamic,
    psy_conf: &mut [PsyConfiguration; 2],
    psy_out_element: &mut PsyOutElement,
    psy_out_channel: &mut [PsyOutChannel],
    qc_out_channel: &mut [QcOutChannel],
    p_input: &[IntPcm],
    input_buf_size: u32,
    ch_idx: &[i32],
    total_channels: i32,
    pool: Option<&ThreadPool>,
) -> AacEncoderError {
    if pool.is_none() || channels != 2 {
        return fdk_aac_enc_psy_main(
            channels,
            psy_static,
            psy_dynamic,
            psy_conf,
            psy_out_element,
            psy_out_channel,
            qc_out_channel,
            p_input,
            input_buf_size,
            ch_idx,
            total_channels,
        );
    }
    let frame =
        match psy_main_start(channels, psy_static, psy_conf, p_input, input_buf_size, ch_idx) {
            Ok(frame) => frame,
            Err(e) => return e,
        };
    psy_main_finish(
        &frame,
        channels,
        psy_static,
        psy_dynamic,
        psy_conf,
        psy_out_element,
        psy_out_channel,
        qc_out_channel,
        p_input,
        input_buf_size,
        ch_idx,
        pool,
    )
}

/// The second part of [`fdk_aac_enc_psy_main`] after [`psy_main_start`] returned `frame` (see the
/// [module documentation](self)); `channels` is 1 or 2.
#[allow(clippy::too_many_arguments)]
pub(crate) fn psy_main_finish(
    frame: &PsyFrame,
    channels: i32,
    psy_static: &mut [PsyStatic],
    psy_dynamic: &mut PsyDynamic,
    psy_conf: &[PsyConfiguration; 2],
    psy_out_element: &mut PsyOutElement,
    psy_out_channel: &mut [PsyOutChannel],
    qc_out_channel: &mut [QcOutChannel],
    p_input: &[IntPcm],
    input_buf_size: u32,
    ch_idx: &[i32],
    pool: Option<&ThreadPool>,
) -> AacEncoderError {
    let common_window: i32 = 1;
    let n_ch = channels as usize;
    let PsyFrame {
        n_time_samples,
        block_switching_offset,
        is_short_window,
        this_conf,
        window_length,
        n_windows,
        max_sfb,
    } = *frame;

    let input_offset = |ch: usize, off: i32| -> usize {
        (ch_idx[ch] as i64 * input_buf_size as i64 + off as i64) as usize
    };

    let PsyDynamic { psy_data, tns_data, pns_data } = psy_dynamic;
    // (FDKaacEnc_psyMain() resets tnsData[1] also for a single channel, see phase C)
    let [td0, td1] = tns_data;
    let mut tns_iter = [td0, td1].into_iter();
    let mut chs: Vec<Ch<'_>> = Vec::with_capacity(2);
    {
        for (ch, ((((st, pd), pns), poc), qoc)) in psy_static
            .iter_mut()
            .zip(psy_data.iter_mut())
            .zip(pns_data.iter_mut())
            .zip(psy_out_channel.iter_mut())
            .zip(qc_out_channel.iter_mut())
            .take(n_ch)
            .enumerate()
        {
            let Some(tns) = tns_iter.next() else { break };
            chs.push(Ch {
                ch,
                st,
                pd,
                tns,
                pns,
                poc,
                qoc,
                sfb_tonality: [0; MAX_SFB_LONG],
                failed: false,
                nonzero: false,
                min_spec_shift: MAX_SHIFT_DBL,
                max_nrg: 0,
                nrg_scaling: 0,
                tns_spec_shift: 0,
                max_sfb_per_group: 0,
            });
        }
    }
    let spare_tns = tns_iter.next();
    if chs.len() != n_ch {
        // (not reached: the element has its statics and output channels)
        return AAC_ENC_UNKNOWN;
    }

    // Phase A: transform and get mdctScaling for all channels and windows, the zero spectrum
    // check, and (parallel only) sfbMaxScaleSpec.
    let phase_a1 = |c: &mut Ch<'_>| {
        let ch = c.ch;
        let conf = &psy_conf[this_conf[ch]];
        let (pd, st) = (&mut *c.pd, &mut *c.st);
        let spec = &mut c.qoc.mdct_spectrum;
        let mut mdct_spectrum_e: i32 = 0;

        // update number of active bands
        if st.is_lfe != 0 {
            pd.sfb_active = conf.sfb_active_lfe;
            pd.lowpass_line = conf.lowpass_line_lfe;
        } else {
            pd.sfb_active = conf.sfb_active;
            pd.lowpass_line = conf.lowpass_line;
        }

        let bsc = &mut st.block_switching_control;
        let failed = if conf.filterbank == FB_ELD {
            fdk_aac_enc_transform_real_eld(
                &st.psy_input_buffer,
                spec,
                bsc.last_window_sequence,
                bsc.window_shape,
                &mut bsc.last_window_shape,
                n_time_samples,
                &mut mdct_spectrum_e,
                conf.filterbank,
                &mut st.overlap_add_buffer,
            ) != 0
        } else {
            fdk_aac_enc_transform_real(
                &st.psy_input_buffer,
                spec,
                bsc.last_window_sequence,
                bsc.window_shape,
                &mut bsc.last_window_shape,
                &mut st.mdct_pers,
                n_time_samples,
                &mut mdct_spectrum_e,
                conf.filterbank,
            ) != 0
        };
        if failed {
            c.failed = true;
            return;
        }
        if let Some(hook) = SPECTRUM_HOOK.with(|h| h.get()) {
            hook(&mut spec[..n_time_samples.max(0) as usize]);
        }

        let mut zero_spec = true;
        for w in 0..n_windows[ch] {
            let w_offset = w * window_length[ch];

            // Low pass / highest sfb
            let start = (pd.lowpass_line + w_offset) as usize;
            if pd.lowpass_line > window_length[ch] {
                negative_lowpass_clear();
            }
            let len = (window_length[ch] - pd.lowpass_line).max(0) as usize;
            spec[start..start + len].fill(0);

            if (psy_conf[0].filterbank != FB_LC) && (pd.lowpass_line >= FADE_OUT_LEN as i32) {
                // Do blending to reduce gibbs artifacts
                for i in 0..FADE_OUT_LEN {
                    let k = (pd.lowpass_line + w_offset - FADE_OUT_LEN as i32 + i as i32) as usize;
                    spec[k] = f_mult(spec[k], FADE_OUT_FACTOR[i]);
                }
            }

            // Check for zero spectrum. These loops will usually terminate very, very early.
            let mut line = 0;
            while (line < pd.lowpass_line) && zero_spec {
                if spec[(line + w_offset) as usize] != 0 {
                    zero_spec = false;
                    break;
                }
                line += 1;
            }
        } // w loop
        c.nonzero = !zero_spec;

        pd.mdct_scale = mdct_spectrum_e;

        // rotate internal time samples
        let nts = n_time_samples as usize;
        st.psy_input_buffer.copy_within(nts..2 * nts, 0);

        // ... and get remaining samples from input buffer
        let src = input_offset(ch, 2 * n_time_samples - block_switching_offset);
        let len = (block_switching_offset - n_time_samples) as usize;
        st.psy_input_buffer[nts..nts + len].copy_from_slice(&p_input[src..src + len]);
    };
    let phase_a2 = |c: &mut Ch<'_>| {
        let ch = c.ch;
        let conf = &psy_conf[this_conf[ch]];
        let pd = &mut *c.pd;
        let mut min_spec_shift = MAX_SHIFT_DBL;
        for w in 0..n_windows[ch] {
            let w_offset = (w * window_length[ch]) as usize;
            let m = (w * max_sfb[ch]) as usize;
            fdk_aac_enc_calc_sfb_max_scale_spec(
                &c.qoc.mdct_spectrum[w_offset..],
                &conf.sfb_offset,
                &mut pd.sfb_max_scale_spec[m..],
                pd.sfb_active,
            );

            for sfb in 0..pd.sfb_active as usize {
                min_spec_shift = fix_min(min_spec_shift, pd.sfb_max_scale_spec[m + sfb]);
            }
        }
        c.min_spec_shift = min_spec_shift;
    };
    let parallel_a = chs.iter().all(|c| {
        transform_cannot_fail(
            psy_conf[this_conf[c.ch]].filterbank,
            c.st.block_switching_control.last_window_sequence,
            n_time_samples,
        )
    });
    let zero_spec = if parallel_a && pool.is_some() {
        run(pool, &mut chs, |c| {
            phase_a1(c);
            if !c.failed {
                phase_a2(c);
            }
        });
        if chs.iter().any(|c| c.failed) {
            // (not reached, see the module documentation)
            return AAC_ENC_UNSUPPORTED_FILTERBANK;
        }
        !chs.iter().any(|c| c.nonzero)
    } else {
        // the sequential order: return at the first failing transform
        for c in chs.iter_mut() {
            phase_a1(c);
            if c.failed {
                return AAC_ENC_UNSUPPORTED_FILTERBANK;
            }
        }
        let zero_spec = !chs.iter().any(|c| c.nonzero);
        if !zero_spec {
            chs.iter_mut().for_each(phase_a2);
        }
        zero_spec
    };

    // Do some rescaling to get maximum possible accuracy for energies
    let mut final_shift: i32 = 0;
    let mut ld_shift: FixpDbl = 0;
    if !zero_spec {
        let min_spec_shift = chs.iter().fold(MAX_SHIFT_DBL, |m, c| fix_min(m, c.min_spec_shift));
        let mut nrg_shift: i32 = MAX_SHIFT_DBL;

        // Phase B: calc possible energy leftshift for each sfb
        run(pool, &mut chs, |c| {
            let ch = c.ch;
            let conf = &psy_conf[this_conf[ch]];
            let pd = &mut *c.pd;
            let mut max_nrg: FixpDbl = 0;
            for w in 0..n_windows[ch] {
                let w_offset = (w * window_length[ch]) as usize;
                let m = (w * max_sfb[ch]) as usize;
                let curr_nrg = fdk_aac_enc_check_band_energy_optim(
                    &c.qoc.mdct_spectrum[w_offset..],
                    &pd.sfb_max_scale_spec[m..],
                    &conf.sfb_offset,
                    pd.sfb_active,
                    &mut pd.sfb_energy[m..],
                    &mut pd.sfb_energy_ld_data[m..],
                    min_spec_shift - 4,
                );
                max_nrg = fix_max(max_nrg, curr_nrg);
            }
            c.max_nrg = max_nrg;
        });
        let max_nrg = chs.iter().fold(0, |m, c| fix_max(m, c.max_nrg));

        if max_nrg != 0 {
            nrg_shift = (count_leading_bits(max_nrg) >> 1) + (min_spec_shift - 4);
        }

        // For short windows 1 additional bit headroom is necessary to prevent overflows when
        // summing up energies in FDKaacEnc_groupShortData()
        if is_short_window[0] != 0 {
            nrg_shift -= 1;
        }

        // both spectrum and energies mustn't overflow
        final_shift = fix_min(min_spec_shift, nrg_shift);

        // do not shift more than 3 bits more to the left than signal without blockfloating
        // point would be to avoid overflow of scaled PCM quantization thresholds
        if final_shift > chs[0].pd.mdct_scale + 3 {
            final_shift = chs[0].pd.mdct_scale + 3;
        }

        fdk_assert!(final_shift >= 0); // right shift is not allowed
        if final_shift < 0 {
            negative_final_shift();
        }

        // FIXP_DBL ldShift = finalShift * FL2FXCONST_DBL(2.0 / 64);
        ld_shift = final_shift.wrapping_mul(fl2fxconst_dbl(2.0 / 64.0));
    }

    let lfe0 = (channels >= 1) && (chs[0].st.is_lfe != 0);
    let tns_enabled = psy_conf[0].tns_conf.tns_active != 0 || psy_conf[1].tns_conf.tns_active != 0;

    // Phase C: the rescaling (or the clearing of a zero spectrum), tonality and TNS detection
    run(pool, &mut chs, |c| {
        let ch = c.ch;
        let pd = &mut *c.pd;
        if !zero_spec {
            // correct sfbEnergy and sfbEnergyLdData with new finalShift
            let max_sfb_ch = max_sfb[ch] as usize;
            let mut w_max_sfb_ch: usize = 0;
            for _w in 0..n_windows[ch] {
                for sfb in 0..pd.sfb_active as usize {
                    let k = w_max_sfb_ch + sfb;
                    let mut scale = fix_max(0, pd.sfb_max_scale_spec[k] - 4);
                    scale = fix_min((scale - final_shift) << 1, DFRACT_BITS - 1);
                    if scale >= 0 {
                        pd.sfb_energy[k] >>= scale;
                    } else {
                        // (see fdk_aac_enc_psy_main)
                        pd.sfb_energy[k] = pd.sfb_energy[k].wrapping_shl((-scale) as u32);
                    }
                    pd.sfb_threshold[k] = f_mult(pd.sfb_energy[k], C_RATIO);
                    pd.sfb_energy_ld_data[k] = pd.sfb_energy_ld_data[k].wrapping_add(ld_shift);
                }
                w_max_sfb_ch += max_sfb_ch;
            }

            if final_shift != 0 {
                let w_len = window_length[ch];
                let lowpass_line = pd.lowpass_line;
                let mut w_offset: i32 = 0;
                let mdct_spectrum = &mut c.qoc.mdct_spectrum;
                for w in 0..n_windows[ch] {
                    let spectrum = &mut mdct_spectrum[w_offset as usize..];
                    shift_spectrum_left(spectrum, lowpass_line, final_shift);
                    w_offset += w_len;

                    // update sfbMaxScaleSpec
                    let m = (w * max_sfb[ch]) as usize;
                    for sfb in 0..pd.sfb_active as usize {
                        pd.sfb_max_scale_spec[m + sfb] -= final_shift;
                    }
                }
                // update mdctScale
                pd.mdct_scale -= final_shift;
            }
        } else {
            // all spectral lines are zero
            pd.mdct_scale = 0;
            // clear sfbMaxScaleSpec
            for w in 0..n_windows[ch] {
                let m = (w * max_sfb[ch]) as usize;
                for sfb in 0..pd.sfb_active as usize {
                    pd.sfb_max_scale_spec[m + sfb] = 0;
                    pd.sfb_energy[m + sfb] = 0;
                    pd.sfb_energy_ld_data[m + sfb] = fl2fxconst_dbl(-1.0);
                    pd.sfb_threshold[m + sfb] = 0;
                }
            }
        }

        // Advance psychoacoustics: Tonality and TNS
        if !lfe0 {
            let conf = &psy_conf[this_conf[ch]];
            if is_short_window[ch] == 0 {
                // tonality
                fdk_aac_enc_calculate_full_tonality(
                    &c.qoc.mdct_spectrum,
                    &pd.sfb_max_scale_spec,
                    &pd.sfb_energy_ld_data,
                    &mut c.sfb_tonality,
                    pd.sfb_active,
                    &conf.sfb_offset,
                    conf.pns_conf.use_pns,
                );
            }
            if tns_enabled {
                for w in 0..n_windows[ch] {
                    let w_offset = (w * window_length[ch]) as usize;
                    // TNS
                    fdk_aac_enc_tns_detect(
                        &mut *c.tns,
                        &conf.tns_conf,
                        &mut c.poc.tns_info,
                        conf.sfb_cnt,
                        &c.qoc.mdct_spectrum[w_offset..],
                        w,
                        c.st.block_switching_control.last_window_sequence,
                    );
                }
            }
        }
    });

    if lfe0 {
        let raw = &mut chs[0].tns.data_raw;
        *raw.tns_active_mut(TnsDataRaw::LONG_SUB_BLOCK_INFO, HIFILT) = 0;
        *raw.tns_active_mut(TnsDataRaw::LONG_SUB_BLOCK_INFO, LOFILT) = 0;
    } else if tns_enabled {
        let mut tns_active = [0i32; TRANS_FAC as usize];

        if channels == 2 {
            let (c0, c1) = pair(&mut chs[..]);
            fdk_aac_enc_tns_sync(
                &mut *c1.tns,
                &*c0.tns,
                &mut c1.poc.tns_info,
                &c0.poc.tns_info,
                c1.st.block_switching_control.last_window_sequence,
                c0.st.block_switching_control.last_window_sequence,
                &psy_conf[this_conf[1]].tns_conf,
            );
        }

        if channels >= 1 {
            // all checks for TNS do only work for common windows (which is always set)
            fdk_assert!(1 == common_window);
            let last = n_ch - 1;
            for w in 0..n_windows[0] as usize {
                let o = if is_short_window[0] != 0 {
                    TnsDataRaw::short_sub_block_info(w)
                } else {
                    TnsDataRaw::LONG_SUB_BLOCK_INFO
                };
                let (r0, rl) = (&chs[0].tns.data_raw, &chs[last].tns.data_raw);
                tns_active[w] = (r0.tns_active(o, HIFILT) != 0
                    || r0.tns_active(o, LOFILT) != 0
                    || rl.tns_active(o, HIFILT) != 0
                    || rl.tns_active(o, LOFILT) != 0) as i32;
            }
        }

        // Phase D: the scaling before TNS, the TNS filtering, and the band energies after it
        run(pool, &mut chs, |c| {
            let ch = c.ch;
            let conf = &psy_conf[this_conf[ch]];
            let pd = &mut *c.pd;
            if tns_active[0] != 0 && is_short_window[ch] == 0 {
                // Scale down spectrum if tns is active in one of the two channels with same
                // lastWindowSequence
                let shift = 1;
                let spec = &mut c.qoc.mdct_spectrum;
                for sfb in 0..conf.lowpass_line.max(0) as usize {
                    spec[sfb] >>= shift;
                }

                // update thresholds
                for sfb in 0..pd.sfb_active as usize {
                    pd.sfb_threshold[sfb] >>= 2 * shift;
                }

                pd.mdct_scale += shift; // update mdctScale
            }

            let last_ws = c.st.block_switching_control.last_window_sequence;
            for w in 0..n_windows[ch] {
                let w_offset = (w * window_length[ch]) as usize;
                fdk_aac_enc_tns_encode(
                    &c.poc.tns_info,
                    &*c.tns,
                    conf.sfb_cnt,
                    &conf.tns_conf,
                    // hThisPsyConf[ch]->lowpassLine: filter stops before that line !
                    conf.sfb_offset[pd.sfb_active as usize],
                    &mut c.qoc.mdct_spectrum[w_offset..],
                    w,
                    last_ws,
                );

                if tns_active[w as usize] != 0 {
                    let m = (w * max_sfb[ch]) as usize;
                    fdk_aac_enc_calc_sfb_max_scale_spec(
                        &c.qoc.mdct_spectrum[w_offset..],
                        &conf.sfb_offset,
                        &mut pd.sfb_max_scale_spec[m..],
                        pd.sfb_active,
                    );
                }
            }

            let mut tns_spec_shift = 0;
            for w in 0..n_windows[ch] {
                if tns_active[w as usize] != 0 {
                    if is_short_window[ch] != 0 {
                        let m = (w * max_sfb[ch]) as usize;
                        fdk_aac_enc_calc_band_energy_optim_short(
                            &c.qoc.mdct_spectrum[(w * window_length[ch]) as usize..],
                            &pd.sfb_max_scale_spec[m..],
                            &conf.sfb_offset,
                            pd.sfb_active,
                            &mut pd.sfb_energy[m..],
                        );
                    } else {
                        // with tns, energy calculation can overflow; -> scaling
                        c.nrg_scaling = fdk_aac_enc_calc_band_energy_optim_long(
                            &c.qoc.mdct_spectrum,
                            &pd.sfb_max_scale_spec,
                            &conf.sfb_offset,
                            pd.sfb_active,
                            &mut pd.sfb_energy,
                            &mut pd.sfb_energy_ld_data,
                        );
                        // nrgScaling is set only if nrg would have an overflow
                        tns_spec_shift = fix_max(tns_spec_shift, c.nrg_scaling);
                    }
                }
            }
            c.tns_spec_shift = tns_spec_shift;
        });
    } else {
        // In case of disable TNS, reset its dynamic data. Some of its elements is required in
        // PNS detection below.
        for c in chs.iter_mut() {
            *c.tns = TnsData::default();
        }
        if let Some(t) = spare_tns {
            *t = TnsData::default();
        }
    }
    let tns_scaling = !lfe0 && tns_enabled;
    let tns_spec_shift = chs.iter().fold(0, |m, c| fix_max(m, c.tns_spec_shift));

    // Phase E: the scaling after TNS, and the thresholds
    run(pool, &mut chs, |c| {
        let ch = c.ch;
        let conf = &psy_conf[this_conf[ch]];
        let (pd, st) = (&mut *c.pd, &mut *c.st);

        // adapt scaling to prevent nrg overflow, only for long blocks
        if tns_scaling && (tns_spec_shift != 0) && is_short_window[ch] == 0 {
            // scale down spectrum, nrg's and thresholds, if there was an overflow in sfbNrg
            // calculation after tns
            let spec = &mut c.qoc.mdct_spectrum;
            for line in 0..conf.lowpass_line.max(0) as usize {
                spec[line] >>= tns_spec_shift;
            }
            let scale = (tns_spec_shift - c.nrg_scaling) << 1;
            for sfb in 0..pd.sfb_active as usize {
                pd.sfb_energy_ld_data[sfb] = pd.sfb_energy_ld_data[sfb]
                    .wrapping_sub(scale.wrapping_mul(fl2fxconst_dbl(1.0 / LD_DATA_SCALING as f64)));
                pd.sfb_energy[sfb] >>= scale;
                pd.sfb_threshold[sfb] >>= tns_spec_shift << 1;
            }
            // update mdctScale; not necessary to update sfbMaxScaleSpec
            pd.mdct_scale += tns_spec_shift;
        }

        // Advance thresholds
        let clip_energy: FixpDbl;
        let mut energy_shift = pd.mdct_scale * 2;
        let clip_nrg_shift = energy_shift - THR_SHIFTBITS;
        let headroom: i32 = if is_short_window[ch] != 0 { 6 } else { 0 };

        if clip_nrg_shift >= 0 {
            // (see fdk_aac_enc_psy_main)
            clip_energy = conf.clip_energy.wrapping_shr(clip_nrg_shift as u32);
        } else if clip_nrg_shift >= -headroom {
            clip_energy = conf.clip_energy.wrapping_shl((-clip_nrg_shift) as u32);
        } else {
            clip_energy = MAXVAL_DBL;
        }

        for w in 0..n_windows[ch] {
            let m = (w * max_sfb[ch]) as usize;
            // limit threshold to avoid clipping
            for i in 0..pd.sfb_active as usize {
                pd.sfb_threshold[m + i] = fix_min(pd.sfb_threshold[m + i], clip_energy);
            }

            // spreading
            fdk_aac_enc_spreading_max(
                pd.sfb_active,
                &conf.sfb_mask_low_factor,
                &conf.sfb_mask_high_factor,
                &mut pd.sfb_threshold[m..],
            );

            // PCM quantization threshold
            energy_shift += PCM_QUANT_THR_SCALE;
            if energy_shift >= 0 {
                energy_shift = fix_min(DFRACT_BITS - 1, energy_shift);
                for i in 0..pd.sfb_active as usize {
                    pd.sfb_threshold[m + i] = fix_max(
                        pd.sfb_threshold[m + i] >> THR_SHIFTBITS,
                        conf.sfb_pcm_quant_threshold[i] >> energy_shift,
                    );
                }
            } else {
                energy_shift = fix_min(DFRACT_BITS - 1, -energy_shift);
                for i in 0..pd.sfb_active as usize {
                    pd.sfb_threshold[m + i] = fix_max(
                        pd.sfb_threshold[m + i] >> THR_SHIFTBITS,
                        conf.sfb_pcm_quant_threshold[i].wrapping_shl(energy_shift as u32),
                    );
                }
            }

            if st.is_lfe == 0 {
                // preecho control
                if st.block_switching_control.last_window_sequence == STOP_WINDOW {
                    // prevent FDKaacEnc_PreEchoControl from comparing stop thresholds with short
                    // thresholds
                    for i in 0..pd.sfb_active as usize {
                        st.sfb_thresholdnm1[i] = MAXVAL_DBL;
                    }

                    st.mdct_scalenm1 = 0;
                    st.calc_pre_echo = 0;
                }

                fdk_aac_enc_pre_echo_control(
                    &mut st.sfb_thresholdnm1,
                    st.calc_pre_echo,
                    pd.sfb_active,
                    conf.max_allowed_increase_factor,
                    conf.min_remaining_threshold_factor,
                    &mut pd.sfb_threshold[m..],
                    pd.mdct_scale,
                    &mut st.mdct_scalenm1,
                );

                st.calc_pre_echo = 1;

                if st.block_switching_control.last_window_sequence == START_WINDOW {
                    // prevent FDKaacEnc_PreEchoControl in next frame to compare start thresholds
                    // with short thresholds
                    for i in 0..pd.sfb_active as usize {
                        st.sfb_thresholdnm1[i] = MAXVAL_DBL;
                    }

                    st.mdct_scalenm1 = 0;
                    st.calc_pre_echo = 0;
                }
            }

            // spread energy to avoid hole detection
            let n = pd.sfb_active as usize;
            let (energy, spread) = (&pd.sfb_energy, &mut pd.sfb_spread_energy);
            spread[m..m + n].copy_from_slice(&energy[m..m + n]);

            fdk_aac_enc_spreading_max(
                pd.sfb_active,
                &conf.sfb_mask_low_factor_spr_en,
                &conf.sfb_mask_high_factor_spr_en,
                &mut pd.sfb_spread_energy[m..],
            );
        }
    });

    // Calc bandwise energies for mid and side channel. Do it only if 2 channels exist
    if channels == 2 {
        let conf1 = &psy_conf[this_conf[1]];
        let (c0, c1) = pair(&mut chs[..]);
        let last_ws1 = c1.st.block_switching_control.last_window_sequence;
        let (pd0, pd1) = (&mut *c0.pd, &mut *c1.pd);
        for w in 0..n_windows[1] {
            let w_offset = (w * window_length[1]) as usize;
            let m0 = (w * max_sfb[0]) as usize;
            let m1 = (w * max_sfb[1]) as usize;
            fdk_aac_enc_calc_band_nrg_ms_opt(
                &c0.qoc.mdct_spectrum[w_offset..],
                &c1.qoc.mdct_spectrum[w_offset..],
                &pd0.sfb_max_scale_spec[m0..],
                &pd1.sfb_max_scale_spec[m1..],
                &conf1.sfb_offset,
                pd0.sfb_active,
                &mut pd0.sfb_energy_ms[m0..],
                &mut pd1.sfb_energy_ms[m1..],
                (last_ws1 != SHORT_WINDOW) as i32,
                &mut pd0.sfb_energy_ms_ld_data,
                &mut pd1.sfb_energy_ms_ld_data,
            );
        }
    }

    // Phase F: group short data (maxSfb[ch] for short blocks is determined here), the intensity
    // parameter initialization and the PNS decision
    run(pool, &mut chs, |c| {
        let ch = c.ch;
        let (pd, poc, qoc) = (&mut *c.pd, &mut *c.poc, &mut *c.qoc);
        if is_short_window[ch] != 0 {
            let conf_short = &psy_conf[1];
            let bsc = &c.st.block_switching_control;
            let no_sfb = bsc.no_of_groups * conf_short.sfb_cnt;
            // At this point, energies and thresholds are copied/regrouped from the ".Short" to
            // the ".Long" arrays
            fdk_aac_enc_group_short_data(
                &mut qoc.mdct_spectrum,
                &mut pd.sfb_threshold,
                &mut pd.sfb_energy,
                &mut pd.sfb_energy_ms,
                &mut pd.sfb_spread_energy,
                conf_short.sfb_cnt,
                pd.sfb_active,
                &conf_short.sfb_offset,
                &conf_short.sfb_min_snr_ld_data,
                &mut pd.grouped_sfb_offset,
                &mut c.max_sfb_per_group,
                &mut qoc.sfb_min_snr_ld_data,
                bsc.no_of_groups,
                &bsc.group_len,
                psy_conf[1].granule_length,
            );

            let n = pd.sfb_active as usize;
            let step = conf_short.sfb_cnt.max(1) as usize;

            // calculate ldData arrays (short values are in .Long-arrays after
            // FDKaacEnc_groupShortData)
            let mut sfb_grp: usize = 0;
            while (sfb_grp as i32) < no_sfb {
                ld_data_vector(
                    &pd.sfb_energy[sfb_grp..sfb_grp + n],
                    &mut qoc.sfb_energy_ld_data[sfb_grp..],
                );
                sfb_grp += step;
            }

            // calc sfbThrld and set Values smaller 2^-31 to 2^-33
            sfb_grp = 0;
            while (sfb_grp as i32) < no_sfb {
                ld_data_vector(
                    &pd.sfb_threshold[sfb_grp..sfb_grp + n],
                    &mut qoc.sfb_threshold_ld_data[sfb_grp..],
                );
                for sfb in 0..n {
                    qoc.sfb_threshold_ld_data[sfb_grp + sfb] = fix_max(
                        qoc.sfb_threshold_ld_data[sfb_grp + sfb],
                        fl2fxconst_dbl(-0.515625),
                    );
                }
                sfb_grp += step;
            }

            if channels == 2 {
                sfb_grp = 0;
                while (sfb_grp as i32) < no_sfb {
                    let src = pd.sfb_energy_ms;
                    ld_data_vector(
                        &src[sfb_grp..sfb_grp + n],
                        &mut pd.sfb_energy_ms_ld_data[sfb_grp..],
                    );
                    sfb_grp += step;
                }
            }

            poc.sfb_offsets = pd.grouped_sfb_offset;
        } else {
            let conf_long = &psy_conf[0];
            // maxSfb[ch] for long blocks
            let mut sfb = pd.sfb_active - 1;
            while sfb >= 0 {
                let mut line = conf_long.sfb_offset[(sfb + 1) as usize] - 1;
                while line >= conf_long.sfb_offset[sfb as usize] {
                    if qoc.mdct_spectrum[line as usize] != 0 {
                        break;
                    }
                    line -= 1;
                }
                if line > conf_long.sfb_offset[sfb as usize] {
                    break;
                }
                sfb -= 1;
            }
            c.max_sfb_per_group = sfb + 1;
            c.max_sfb_per_group = fix_max(fix_min(5, pd.sfb_active), c.max_sfb_per_group);

            let n = pd.sfb_active as usize;
            // sfbNrgLdData is calculated in FDKaacEnc_advancePsychLong, copy in psyOut structure
            qoc.sfb_energy_ld_data[..n].copy_from_slice(&pd.sfb_energy_ld_data[..n]);

            // (see fdk_aac_enc_psy_main: 61 values)
            psy_conf_sfb_offsets_61(conf_long, &mut poc.sfb_offsets);

            // sfbMinSnrLdData modified in adjust threshold, copy necessary
            qoc.sfb_min_snr_ld_data[..n].copy_from_slice(&conf_long.sfb_min_snr_ld_data[..n]);

            // calc sfbThrld and set Values smaller 2^-31 to 2^-33
            ld_data_vector(&pd.sfb_threshold[..n], &mut qoc.sfb_threshold_ld_data);
            for i in 0..n {
                qoc.sfb_threshold_ld_data[i] =
                    fix_max(qoc.sfb_threshold_ld_data[i], fl2fxconst_dbl(-0.515625));
            }
        }

        // Intensity parameter intialization.
        poc.is_book = [0; MAX_GROUPED_SFB];
        poc.is_scale = [0; MAX_GROUPED_SFB];

        let win = if is_short_window[ch] != 0 { 1 } else { 0 };
        if c.st.is_lfe == 0 {
            let tns_order = poc.tns_info.order[0][0];
            let raw = &c.tns.data_raw;
            let tns_prediction_gain = raw.prediction_gain(TnsDataRaw::LONG_SUB_BLOCK_INFO, HIFILT);
            let tns_active = raw.tns_active(TnsDataRaw::LONG_SUB_BLOCK_INFO, HIFILT);
            let sfb_offset = psy_conf[win].sfb_offset;
            let tonality_reads = crate::noisedet::noise_detect_tonality_reads();
            // PNS Decision
            fdk_aac_enc_pns_detect(
                &psy_conf[0].pns_conf,
                &mut *c.pns,
                c.st.block_switching_control.last_window_sequence,
                pd.sfb_active,
                c.max_sfb_per_group, // count of Sfb which are not zero.
                &qoc.sfb_threshold_ld_data,
                &sfb_offset,
                &qoc.mdct_spectrum,
                &pd.sfb_max_scale_spec,
                &c.sfb_tonality,
                tns_order,
                tns_prediction_gain,
                tns_active,
                &qoc.sfb_energy_ld_data,
                &mut poc.noise_nrg,
            );
            // (see fdk_aac_enc_psy_main: a tonality that was not computed)
            if lfe0 && crate::noisedet::noise_detect_tonality_reads() != tonality_reads {
                super::add_psy_main_lfe_flag_pairs(1);
            }
        } // !isLFE
    });

    let mut max_sfb_per_group = [0i32; 2];
    for c in &chs {
        max_sfb_per_group[c.ch] = c.max_sfb_per_group;
    }

    // stereo Processing
    if channels == 2 {
        let (c0, c1) = pair(&mut chs[..]);
        psy_out_element.tools_info.ms_digest = MS_NONE;
        psy_out_element.common_window = common_window;
        if psy_out_element.common_window != 0 {
            let m = fix_max(max_sfb_per_group[0], max_sfb_per_group[1]);
            max_sfb_per_group[0] = m;
            max_sfb_per_group[1] = m;
        }
        let allow_is = (psy_conf[0].allow_is != 0 && psy_out_element.common_window != 0) as i32;
        let short = c0.st.block_switching_control.last_window_sequence == SHORT_WINDOW;
        let (is_sfb_cnt, is_sfb_per_group) = if !short {
            (psy_conf[0].sfb_cnt, psy_conf[0].sfb_cnt)
        } else {
            (c0.st.block_switching_control.no_of_groups * psy_conf[1].sfb_cnt, psy_conf[1].sfb_cnt)
        };
        // sfbOffset of IntensityStereoProcessing: psyConf[0].sfbOffset for long blocks,
        // psyData[0]->groupedSfbOffset for short blocks.
        let is_sfb_offset: [i32; MAX_GROUPED_SFB + 1];
        let is_sfb_offset: &[i32] = if !short {
            &psy_conf[0].sfb_offset
        } else {
            is_sfb_offset = c0.pd.grouped_sfb_offset;
            &is_sfb_offset
        };

        if !short {
            // PNS preprocessing depending on ms processing: PNS not in Short Window!
            fdk_aac_enc_pre_process_pns_channel_pair(
                c0.pd.sfb_active,
                &c0.pd.sfb_energy,
                &c1.pd.sfb_energy,
                &c0.qoc.sfb_energy_ld_data,
                &c1.qoc.sfb_energy_ld_data,
                &c0.pd.sfb_energy_ms,
                &psy_conf[0].pns_conf,
                &mut *c0.pns,
                &mut *c1.pns,
            );
        }

        fdk_aac_enc_intensity_stereo_processing(
            &c0.pd.sfb_energy,
            &mut c1.pd.sfb_energy,
            &mut c0.qoc.mdct_spectrum,
            &mut c1.qoc.mdct_spectrum,
            &c0.pd.sfb_threshold,
            &mut c1.pd.sfb_threshold,
            &mut c1.qoc.sfb_threshold_ld_data,
            &c0.pd.sfb_spread_energy,
            &mut c1.pd.sfb_spread_energy,
            &c0.qoc.sfb_energy_ld_data,
            &mut c1.qoc.sfb_energy_ld_data,
            &mut psy_out_element.tools_info.ms_digest,
            &mut psy_out_element.tools_info.ms_mask,
            is_sfb_cnt,
            is_sfb_per_group,
            max_sfb_per_group[0],
            is_sfb_offset,
            allow_is,
            &mut c1.poc.is_book,
            &mut c1.poc.is_scale,
            [Some(&mut *c0.pns), Some(&mut *c1.pns)],
        );
    }
    // (the rest uses the arrays of both channels)
    drop(chs);

    if channels == 2 {
        // For short blocks it's OK to pass the ".Long" arrays here. They contain grouped short
        // data since FDKaacEnc_groupShortData()
        let short = psy_static[0].block_switching_control.last_window_sequence == SHORT_WINDOW;
        let is_sfb_cnt = if !short {
            psy_conf[0].sfb_cnt
        } else {
            psy_static[0].block_switching_control.no_of_groups * psy_conf[1].sfb_cnt
        };
        let sfb_offsets0 = psy_out_channel[0].sfb_offsets;
        let is_book1 = psy_out_channel[1].is_book;
        let (allow_ms, ms_sfb_cnt, ms_sfb_per_group) = if !short {
            (psy_conf[0].allow_ms, psy_data[0].sfb_active, psy_data[0].sfb_active)
        } else {
            (psy_conf[1].allow_ms, is_sfb_cnt, psy_conf[1].sfb_cnt)
        };
        fdk_aac_enc_ms_stereo_processing(
            &mut psy_data[..2],
            &mut qc_out_channel[..2],
            Some(&is_book1),
            &mut psy_out_element.tools_info.ms_digest,
            &mut psy_out_element.tools_info.ms_mask,
            allow_ms,
            ms_sfb_cnt,
            ms_sfb_per_group,
            max_sfb_per_group[0],
            &sfb_offsets0,
        );

        if !short {
            // PNS postprocessing
            let (pns0, pns1) = pair(&mut pns_data[..]);
            fdk_aac_enc_post_process_pns_channel_pair(
                psy_data[0].sfb_active,
                &psy_conf[0].pns_conf,
                pns0,
                pns1,
                &mut psy_out_element.tools_info.ms_mask,
                &mut psy_out_element.tools_info.ms_digest,
            );
        }
    } // (channels == 2)

    // PNS Coding
    for ch in 0..n_ch {
        let pd = &psy_data[ch];
        if psy_static[ch].is_lfe != 0 {
            // no PNS coding
            for sfb in 0..pd.sfb_active as usize {
                psy_out_channel[ch].noise_nrg[sfb] = NO_NOISE_PNS;
            }
        } else {
            let conf = &psy_conf[this_conf[ch]];
            fdk_aac_enc_code_pns_channel(
                pd.sfb_active,
                &conf.pns_conf,
                &pns_data[ch].pns_flag,
                &pd.sfb_energy_ld_data,
                // this is the energy that will be written to the bitstream
                &mut psy_out_channel[ch].noise_nrg,
                &mut qc_out_channel[ch].sfb_threshold_ld_data,
            );
        }
    }

    // build output
    for ch in 0..n_ch {
        let pd = &psy_data[ch];
        let poc = &mut psy_out_channel[ch];
        let bsc = &psy_static[ch].block_switching_control;
        poc.max_sfb_per_group = max_sfb_per_group[ch];
        poc.mdct_scale = pd.mdct_scale;
        if is_short_window[ch] == 0 {
            poc.sfb_cnt = psy_conf[0].sfb_active;
            poc.sfb_per_group = psy_conf[0].sfb_active;
            poc.last_window_sequence = bsc.last_window_sequence;
            poc.window_shape = bsc.window_shape;
        } else {
            let sfb_cnt = bsc.no_of_groups * psy_conf[1].sfb_cnt;

            poc.sfb_cnt = sfb_cnt;
            poc.sfb_per_group = psy_conf[1].sfb_cnt;
            poc.last_window_sequence = SHORT_WINDOW;
            poc.window_shape = SINE_WINDOW;
        }
        // generate grouping mask
        let mut mask: i32 = 0;
        for grp in 0..bsc.no_of_groups as usize {
            mask <<= 1;
            for _j in 1..bsc.group_len[grp] {
                mask = (mask << 1) | 1;
            }
        }
        poc.grouping_mask = mask;

        // build interface
        poc.group_len.copy_from_slice(&bsc.group_len[..MAX_NO_OF_GROUPS]);
        let qoc = &mut qc_out_channel[ch];
        qoc.sfb_energy.copy_from_slice(&pd.sfb_energy[..MAX_GROUPED_SFB]);
        qoc.sfb_spread_energy.copy_from_slice(&pd.sfb_spread_energy[..MAX_GROUPED_SFB]);
    }

    AAC_ENC_OK
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/psbitdec.h, libSBRdec/src/psbitdec.cpp.

//! Port of `psbitdec.h` and `psbitdec.cpp`: the bitstream parser and the parameter decoding of
//! the parametric stereo decoder.

use crate::huff_dec::Huffman;
use crate::psdec::*;
use crate::sbr_rom::*;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_read_bits, FdkBitStream};

/// `decode_huff_cw(h, hBitBuf, length)`: decodes one code word (`length` is always `NULL` in
/// the library).
pub fn decode_huff_cw(h: Huffman, h_bit_buf: &mut FdkBitStream) -> i8 {
    let mut index: i8 = 0;

    while index >= 0 {
        let bit = fdk_read_bits(h_bit_buf, 1);
        index = h[index as usize][bit as usize];
    }
    // Add offset (the result is an SCHAR)
    (index as i32 + 64) as i8
}

/// `limitMinMax(i, min, max)`
pub fn limit_min_max(i: i8, min: i8, max: i8) -> i8 {
    if i < min {
        min
    } else if i > max {
        max
    } else {
        i
    }
}

/// `deltaDecodeArray()`: decodes delta values in place and updates the data buffers according to
/// the quantization classes. `a_index` has `nr_elements * stride` entries.
#[allow(clippy::too_many_arguments)]
pub fn delta_decode_array(
    enable: i8,
    a_index: &mut [i8],
    a_prev_frame_index: &[i8],
    dt_df: i8,
    nr_elements: u8,
    stride: u8,
    min_idx: i8,
    max_idx: i8,
) {
    let n = nr_elements as usize;

    // Delta decode
    if enable == 1 {
        if dt_df == 0 {
            // Delta coded in freq
            a_index[0] = limit_min_max(a_index[0], min_idx, max_idx);
            for i in 1..n {
                // SCHAR addition (int, truncated back to SCHAR)
                a_index[i] = (a_index[i - 1] as i32 + a_index[i] as i32) as i8;
                a_index[i] = limit_min_max(a_index[i], min_idx, max_idx);
            }
        } else {
            // Delta time
            for i in 0..n {
                a_index[i] =
                    (a_prev_frame_index[i * stride as usize] as i32 + a_index[i] as i32) as i8;
                a_index[i] = limit_min_max(a_index[i], min_idx, max_idx);
            }
        }
    } else {
        // No data is sent, set index to zero
        for v in a_index[..n].iter_mut() {
            *v = 0;
        }
    }
    if stride == 2 {
        // (int i = nrElements * stride - 1: no iteration for nrElements == 0)
        let mut i = n as i32 * stride as i32 - 1;
        while i > 0 {
            a_index[i as usize] = a_index[(i >> 1) as usize];
            i -= 1;
        }
    }
}

/// `map34IndexTo20()`: maps the ICC/IID parameters of 34 stereo bands to 20 stereo bands.
pub fn map34_index_to20(a_index: &mut [i8; NO_HI_RES_BINS], no_bins: u8) {
    // SCHAR arithmetic in int, divisions truncate toward zero
    let a = |a_index: &[i8; NO_HI_RES_BINS], i: usize| a_index[i] as i32;
    a_index[0] = ((2 * a(a_index, 0) + a(a_index, 1)) / 3) as i8;
    a_index[1] = ((a(a_index, 1) + 2 * a(a_index, 2)) / 3) as i8;
    a_index[2] = ((2 * a(a_index, 3) + a(a_index, 4)) / 3) as i8;
    a_index[3] = ((a(a_index, 4) + 2 * a(a_index, 5)) / 3) as i8;
    a_index[4] = ((a(a_index, 6) + a(a_index, 7)) / 2) as i8;
    a_index[5] = ((a(a_index, 8) + a(a_index, 9)) / 2) as i8;
    a_index[6] = a_index[10];
    a_index[7] = a_index[11];
    a_index[8] = ((a(a_index, 12) + a(a_index, 13)) / 2) as i8;
    a_index[9] = ((a(a_index, 14) + a(a_index, 15)) / 2) as i8;
    a_index[10] = a_index[16];
    // For IPD/OPD it stops here

    if no_bins as usize == NO_HI_RES_BINS {
        a_index[11] = a_index[17];
        a_index[12] = a_index[18];
        a_index[13] = a_index[19];
        a_index[14] = ((a(a_index, 20) + a(a_index, 21)) / 2) as i8;
        a_index[15] = ((a(a_index, 22) + a(a_index, 23)) / 2) as i8;
        a_index[16] = ((a(a_index, 24) + a(a_index, 25)) / 2) as i8;
        a_index[17] = ((a(a_index, 26) + a(a_index, 27)) / 2) as i8;
        a_index[18] =
            ((a(a_index, 28) + a(a_index, 29) + a(a_index, 30) + a(a_index, 31)) / 4) as i8;
        a_index[19] = ((a(a_index, 32) + a(a_index, 33)) / 2) as i8;
    }
}

/// `DecodePs(h_ps_d, frameError, pScratch)`: decodes the delta coded IID and ICC indices of the
/// current slot (`processSlot`). Returns the PS processing flag (1: apply PS).
pub fn decode_ps(h_ps_d: &mut PsDec, frame_error: u8, p_coef: &mut PsDecCoefficients) -> i32 {
    let ps = h_ps_d;
    let slot = ps.process_slot as usize & 1;
    let b_ps_header_valid = ps.bs_data[slot].b_ps_header_valid;
    let b_ps_data_avail = if ps.b_ps_data_avail[slot] == PPT_MPEG { 1 } else { 0 };

    // Decide whether to process or to conceal PS data or not.
    #[allow(clippy::nonminimal_bool)] // (the condition of the C code)
    let skip = (ps.ps_decoded_prv != 0 && frame_error == 0 && b_ps_data_avail == 0)
        || (ps.ps_decoded_prv == 0
            && (frame_error != 0 || b_ps_data_avail == 0 || b_ps_header_valid == 0));
    if skip {
        // Don't apply PS processing. Declare current PS header and bitstream data invalid.
        ps.bs_data[slot].b_ps_header_valid = 0;
        ps.b_ps_data_avail[slot] = PPT_NONE;
        return 0;
    }

    if frame_error != 0 || b_ps_header_valid == 0 {
        // no new PS data available (e.g. frame loss) => keep latest data constant (i.e. FIX
        // with noEnv=0)
        ps.bs_data[slot].no_env = 0;
    }

    // Decode bitstream payload or prepare parameter for concealment:
    let mut env = 0usize;
    while env < ps.bs_data[slot].no_env as usize && env < MAX_NO_PS_ENV {
        let bs = &mut ps.bs_data[slot];
        let no_iid_steps =
            if bs.b_fine_iid_q != 0 { NO_IID_STEPS_FINE } else { NO_IID_STEPS } as i8;

        let (prev_iid, prev_icc): ([i8; NO_HI_RES_IID_BINS], [i8; NO_HI_RES_ICC_BINS]) = if env == 0
        {
            (ps.a_iid_prev_frame_index, ps.a_icc_prev_frame_index)
        } else {
            (bs.aa_iid_index[env - 1], bs.aa_icc_index[env - 1])
        };

        let freq_res_iid = bs.freq_res_iid as usize;
        let freq_res_icc = bs.freq_res_icc as usize;
        // freqResIid/freqResIcc are 0..=2 (modes > 5 are rejected by ReadPsData()).
        let n_iid = FDK_SBR_DECODER_A_NO_IID_BINS.get(freq_res_iid).copied().unwrap_or(0);
        let n_icc = FDK_SBR_DECODER_A_NO_ICC_BINS.get(freq_res_icc).copied().unwrap_or(0);
        delta_decode_array(
            bs.b_enable_iid as i8,
            &mut bs.aa_iid_index[env],
            &prev_iid,
            bs.ab_iid_dt_flag[env],
            n_iid,
            if freq_res_iid != 0 { 1 } else { 2 },
            -no_iid_steps,
            no_iid_steps,
        );

        delta_decode_array(
            bs.b_enable_icc as i8,
            &mut bs.aa_icc_index[env],
            &prev_icc,
            bs.ab_icc_dt_flag[env],
            n_icc,
            if freq_res_icc != 0 { 1 } else { 2 },
            0,
            (NO_ICC_STEPS - 1) as i8,
        );
        env += 1;
    }

    // handling of FIX noEnv=0
    let bs = &mut ps.bs_data[slot];
    if bs.no_env == 0 {
        // set noEnv=1, keep last parameters or force 0 if not enabled
        bs.no_env = 1;

        if bs.b_enable_iid != 0 {
            bs.b_fine_iid_q = ps.b_prev_frame_fine_iid_q;
            bs.freq_res_iid = ps.prev_freq_res_iid;
            bs.aa_iid_index[0] = ps.a_iid_prev_frame_index;
        } else {
            bs.aa_iid_index[0] = [0; NO_HI_RES_IID_BINS];
        }

        if bs.b_enable_icc != 0 {
            bs.freq_res_icc = ps.prev_freq_res_icc;
            bs.aa_icc_index[0] = ps.a_icc_prev_frame_index;
        } else {
            bs.aa_icc_index[0] = [0; NO_HI_RES_ICC_BINS];
        }
    }

    // noEnv is 1..=5 here (read as 0, 1, 2, 4 or 1..=4 from the bitstream).
    let last = (bs.no_env as usize - 1).min(MAX_NO_PS_ENV - 1);

    // Update previous frame Iid quantization
    ps.b_prev_frame_fine_iid_q = bs.b_fine_iid_q;

    // Update previous frequency resolution for IID
    ps.prev_freq_res_iid = bs.freq_res_iid;

    // Update previous frequency resolution for ICC
    ps.prev_freq_res_icc = bs.freq_res_icc;

    // Update previous frame index buffers
    ps.a_iid_prev_frame_index = bs.aa_iid_index[last];
    ps.a_icc_prev_frame_index = bs.aa_icc_index[last];

    // PS data from bitstream (if avail) was decoded now
    ps.b_ps_data_avail[slot] = PPT_NONE;

    let no_sub_samples = ps.no_sub_samples as i32;
    let bs = &mut ps.bs_data[slot];

    // handling of env borders for FIX & VAR
    if bs.b_frame_class == 0 {
        // FIX_BORDERS NoEnv=0,1,2,4
        bs.a_env_start_stop[0] = 0;
        for env in 1..bs.no_env as usize {
            bs.a_env_start_stop[env] = ((env as i32 * no_sub_samples) / bs.no_env as i32) as u8;
        }
        bs.a_env_start_stop[bs.no_env as usize] = no_sub_samples as u8;
        // 1024 (32 slots) env borders:  0, 8, 16, 24, 32
        //  960 (30 slots) env borders:  0, 7, 15, 22, 30
    } else {
        // VAR_BORDERS NoEnv=1,2,3,4
        bs.a_env_start_stop[0] = 0;

        // handle case aEnvStartStop[noEnv]<noSubSample for VAR_BORDERS by duplicating last PS
        // parameters and incrementing noEnv
        let ne = bs.no_env as usize;
        if (bs.a_env_start_stop[ne] as i32) < no_sub_samples {
            bs.aa_iid_index[ne] = bs.aa_iid_index[ne - 1];
            bs.aa_icc_index[ne] = bs.aa_icc_index[ne - 1];
            bs.no_env += 1;
            bs.a_env_start_stop[bs.no_env as usize] = no_sub_samples as u8;
        }

        // enforce strictly monotonic increasing borders
        for env in 1..bs.no_env as usize {
            // UCHAR thr = (UCHAR)noSubSamples - (noEnv - env)
            let mut thr = (no_sub_samples as u8 as i32 - (bs.no_env as i32 - env as i32)) as u8;
            if bs.a_env_start_stop[env] > thr {
                bs.a_env_start_stop[env] = thr;
            } else {
                thr = bs.a_env_start_stop[env - 1].wrapping_add(1);
                if bs.a_env_start_stop[env] < thr {
                    bs.a_env_start_stop[env] = thr;
                }
            }
        }
    }

    // copy data prior to possible 20<->34 in-place mapping
    for env in 0..bs.no_env as usize {
        p_coef.aa_iid_index_mapped[env] = bs.aa_iid_index[env];
        p_coef.aa_icc_index_mapped[env] = bs.aa_icc_index[env];
    }

    // MPEG baseline PS: if ICC/IID parameters for 34 stereo bands are decoded they have to be
    // mapped to 20 stereo bands. Additionaly the IPD/OPD parameters won't be used.
    for env in 0..bs.no_env as usize {
        if bs.freq_res_iid == 2 {
            map34_index_to20(&mut p_coef.aa_iid_index_mapped[env], NO_HI_RES_IID_BINS as u8);
        }
        if bs.freq_res_icc == 2 {
            map34_index_to20(&mut p_coef.aa_icc_index_mapped[env], NO_HI_RES_ICC_BINS as u8);
        }
    }

    1
}

/// `ReadPsData(h_ps_d, hBitBuf, nBitsLeft)`: reads the parametric stereo data of the current
/// read slot (`bsReadSlot`) from the bitstream. Returns the number of bits read. `h_ps_d` is
/// `None` for `NULL` (returns 0).
pub fn read_ps_data(
    h_ps_d: Option<&mut PsDec>,
    h_bit_buf: &mut FdkBitStream,
    n_bits_left: i32,
) -> u32 {
    let Some(ps) = h_ps_d else {
        return 0;
    };
    let mut n_bits_left = n_bits_left;

    let read = ps.bs_read_slot as usize & 1;
    let last = ps.bs_last_slot as usize & 1;

    if ps.bs_read_slot != ps.bs_last_slot {
        // Copy last header data
        ps.bs_data[read] = ps.bs_data[last];
    }
    let bs = &mut ps.bs_data[read];

    let startbits = fdk_get_valid_bits(h_bit_buf) as i32;

    let b_enable_header = fdk_read_bits(h_bit_buf, 1) as i8;

    // Read header
    if b_enable_header != 0 {
        bs.b_ps_header_valid = 1;
        bs.b_enable_iid = fdk_read_bits(h_bit_buf, 1) as u8;
        if bs.b_enable_iid != 0 {
            bs.mode_iid = fdk_read_bits(h_bit_buf, 3) as u8;
        }

        bs.b_enable_icc = fdk_read_bits(h_bit_buf, 1) as u8;
        if bs.b_enable_icc != 0 {
            bs.mode_icc = fdk_read_bits(h_bit_buf, 3) as u8;
        }

        bs.b_enable_ext = fdk_read_bits(h_bit_buf, 1) as u8;
    }

    bs.b_frame_class = fdk_read_bits(h_bit_buf, 1) as u8;
    if bs.b_frame_class == 0 {
        // FIX_BORDERS NoEnv=0,1,2,4
        bs.no_env = FDK_SBR_DECODER_A_FIX_NO_ENV_DECODE[fdk_read_bits(h_bit_buf, 2) as usize];
        // all additional handling of env borders is now in DecodePs()
    } else {
        // VAR_BORDERS NoEnv=1,2,3,4
        bs.no_env = 1 + fdk_read_bits(h_bit_buf, 2) as u8;
        for env in 1..bs.no_env as usize + 1 {
            bs.a_env_start_stop[env] = (fdk_read_bits(h_bit_buf, 5) as u8).wrapping_add(1);
        }
        // all additional handling of env borders is now in DecodePs()
    }

    // verify that IID & ICC modes (quant grid, freq res) are supported
    if bs.mode_iid > 5 || bs.mode_icc > 5 {
        // no useful PS data could be read from bitstream
        ps.b_ps_data_avail[read] = PPT_NONE;
        // discard all remaining bits
        n_bits_left -= startbits - fdk_get_valid_bits(h_bit_buf) as i32;
        while n_bits_left > 0 {
            let i = n_bits_left.min(8);
            fdk_read_bits(h_bit_buf, i as u32);
            n_bits_left -= i;
        }
        return (startbits - fdk_get_valid_bits(h_bit_buf) as i32) as u32;
    }

    if bs.mode_iid > 2 {
        bs.freq_res_iid = bs.mode_iid - 3;
        bs.b_fine_iid_q = 1;
    } else {
        bs.freq_res_iid = bs.mode_iid;
        bs.b_fine_iid_q = 0;
    }

    if bs.mode_icc > 2 {
        bs.freq_res_icc = bs.mode_icc - 3;
    } else {
        bs.freq_res_icc = bs.mode_icc;
    }

    // Extract IID data
    if bs.b_enable_iid != 0 {
        for env in 0..bs.no_env as usize {
            let dt_flag = fdk_read_bits(h_bit_buf, 1) as i8;
            let current_table: Huffman = if dt_flag == 0 {
                if bs.b_fine_iid_q != 0 {
                    &A_BOOK_PS_IID_FINE_FREQ_DECODE
                } else {
                    &A_BOOK_PS_IID_FREQ_DECODE
                }
            } else if bs.b_fine_iid_q != 0 {
                &A_BOOK_PS_IID_FINE_TIME_DECODE
            } else {
                &A_BOOK_PS_IID_TIME_DECODE
            };

            for gr in 0..FDK_SBR_DECODER_A_NO_IID_BINS[bs.freq_res_iid as usize] as usize {
                bs.aa_iid_index[env][gr] = decode_huff_cw(current_table, h_bit_buf);
            }
            bs.ab_iid_dt_flag[env] = dt_flag;
        }
    }

    // Extract ICC data
    if bs.b_enable_icc != 0 {
        for env in 0..bs.no_env as usize {
            let dt_flag = fdk_read_bits(h_bit_buf, 1) as i8;
            let current_table: Huffman =
                if dt_flag == 0 { &A_BOOK_PS_ICC_FREQ_DECODE } else { &A_BOOK_PS_ICC_TIME_DECODE };

            for gr in 0..FDK_SBR_DECODER_A_NO_ICC_BINS[bs.freq_res_icc as usize] as usize {
                bs.aa_icc_index[env][gr] = decode_huff_cw(current_table, h_bit_buf);
            }
            bs.ab_icc_dt_flag[env] = dt_flag;
        }
    }

    if bs.b_enable_ext != 0 {
        // Decoders that support only the baseline version of the PS tool are allowed to ignore
        // the IPD/OPD data, but according header data has to be parsed. ISO/IEC 14496-3
        // Subpart 8 Annex 4
        let mut cnt = fdk_read_bits(h_bit_buf, PS_EXTENSION_SIZE_BITS) as i32;
        if cnt == (1 << PS_EXTENSION_SIZE_BITS) - 1 {
            cnt += fdk_read_bits(h_bit_buf, PS_EXTENSION_ESC_COUNT_BITS) as i32;
        }
        while cnt > 0 {
            fdk_read_bits(h_bit_buf, 8);
            cnt -= 1;
        }
    }

    // new PS data was read from bitstream
    ps.b_ps_data_avail[read] = PPT_MPEG;

    (startbits - fdk_get_valid_bits(h_bit_buf) as i32) as u32
}

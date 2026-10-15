// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPDec/src/tpdec_latm.h, libMpegTPDec/src/tpdec_latm.cpp.

//! Port of `tpdec_latm.h`: the LATM demultiplexer (AudioMuxElement, StreamMuxConfig,
//! PayloadLengthInfo).
//!
//! The code of this file is identical in the `aarch64` and `x86` reference builds (apart from
//! `FDK_ASSERT`).
//!
//! `pAsc` is the `asc` array of the transport decoder in C (`TPDEC_TRACKINDEX(prog, lay)` is
//! always 0, since one program with one layer is supported); the port passes that array.

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_for, fdk_read_bits, fdk_sync_cache, FdkBitStream,
};

use crate::tp_data::*;
use crate::tpdec_asc::audio_specific_config_parse;
use crate::tpdec_lib::*;

/// `MIN_LATM_HEADERLENGTH`
pub const MIN_LATM_HEADERLENGTH: i32 = 9;
/// `MIN_LOAS_HEADERLENGTH`: both in bits.
pub const MIN_LOAS_HEADERLENGTH: i32 = MIN_LATM_HEADERLENGTH + 24;
/// `MIN_TP_BUF_SIZE_LOAS`
pub const MIN_TP_BUF_SIZE_LOAS: i32 = 8194;

/// `LATM_MAX_PROG`
pub const LATM_MAX_PROG: usize = 1;
/// `LATM_MAX_LAYER`
pub const LATM_MAX_LAYER: usize = 1;
/// `LATM_MAX_VAR_CHUNKS`
pub const LATM_MAX_VAR_CHUNKS: usize = 16;
/// `LATM_MAX_ID`
pub const LATM_MAX_ID: usize = 16;

/// `TPDEC_TRACKINDEX(p, l)`
#[inline]
const fn tpdec_trackindex(p: u32, l: u32) -> usize {
    (p + l) as usize
}

/// `LATM_LAYER_INFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatmLayerInfo {
    pub m_frame_length_type: u32,
    pub m_buffer_fullness: u32,
    pub m_stream_id: u32,
    pub m_frame_length_in_bits: u32,
}

/// `CLatmDemux`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CLatmDemux {
    pub m_linfo: [[LatmLayerInfo; LATM_MAX_LAYER]; LATM_MAX_PROG],
    pub m_tara_buffer_fullness: u32,
    pub m_other_data_length: u32,
    /// Length of LOAS payload
    pub m_audio_mux_length_bytes: u32,
    pub m_use_same_stream_mux: u8,
    pub m_audio_mux_version: u8,
    pub m_audio_mux_version_a: u8,
    pub m_all_streams_same_time_framing: u8,
    pub m_no_sub_frames: u8,
    pub m_num_program: u8,
    pub m_num_layer: [u8; LATM_MAX_PROG],
    pub m_other_data_present: u8,
    pub m_crc_check_present: u8,
    pub buffer_fullness_achieved: i8,
    /// explicit config in case of USAC and LOAS/LATM must be compared to IPF cfg
    pub usac_explicit_cfg_changed: u8,
    /// apply ASC immediate without flushing
    pub apply_asc: u8,
    /// the new (dummy parsed) config has an AudioPreRoll
    pub new_cfg_has_audio_pre_roll: u8,
}

/// `CLatmDemux_GetValue(bs)`
fn clatm_demux_get_value(bs: &mut FdkBitStream) -> u32 {
    let mut value: i32 = 0;

    let bytes_for_value = fdk_read_bits(bs, 2) as u8;

    for _ in 0..=bytes_for_value as u32 {
        value = value.wrapping_shl(8);
        let tmp = fdk_read_bits(bs, 8) as u8;
        value = value.wrapping_add(tmp as i32);
    }

    value as u32
}

/// `CLatmDemux_ReadAudioMuxElement(bs, pLatmDemux, m_muxConfigPresent, pTpDecCallbacks, pAsc,
/// pfConfigFound)`
fn clatm_demux_read_audio_mux_element(
    bs: &mut FdkBitStream,
    p_latm_demux: &mut CLatmDemux,
    m_mux_config_present: i32,
    cb: &mut dyn TpDecCallbacks,
    p_asc: &mut [CSAudioSpecificConfig],
    pf_config_found: &mut i32,
) -> TransportDecError {
    let mut error_status;

    'bail: {
        if m_mux_config_present != 0 {
            p_latm_demux.m_use_same_stream_mux = fdk_read_bits(bs, 1) as u8;

            if p_latm_demux.m_use_same_stream_mux == 0 {
                let config_changed: u8 = 0;
                let mut config_mode: u8 = 0;

                if p_latm_demux.apply_asc == 0 {
                    let bs_anchor_dummy_parse = bs.clone();
                    p_latm_demux.new_cfg_has_audio_pre_roll = 0;
                    // do dummy-parsing of ASC to determine if there is an audioPreRoll
                    config_mode |= AC_CM_DET_CFG_CHANGE;
                    error_status = clatm_demux_read_stream_mux_config(
                        bs,
                        p_latm_demux,
                        cb,
                        p_asc,
                        pf_config_found,
                        config_mode,
                        config_changed,
                    );
                    if error_status != TRANSPORTDEC_OK {
                        break 'bail;
                    }

                    // Allow flushing only when audioPreroll functionality is enabled in current
                    // and new config otherwise the new config can be applied immediately.
                    if p_asc[0].m_sc.m_usac_config.element[0]
                        .ext_element
                        .usac_ext_element_has_audio_pre_roll
                        != 0
                        && p_latm_demux.new_cfg_has_audio_pre_roll != 0
                    {
                        p_latm_demux.new_cfg_has_audio_pre_roll = 0;
                        // with audioPreRoll we must flush before applying new cfg
                        p_latm_demux.apply_asc = 0;
                    } else {
                        *bs = bs_anchor_dummy_parse.clone();
                        p_latm_demux.apply_asc = 1; // apply new config immediate
                    }
                }

                if p_latm_demux.apply_asc != 0 {
                    let mut config_changed: u8 = config_changed;
                    let mut bs_anchor = bs.clone();
                    for i in 0..2 {
                        config_mode = 0;

                        if i == 0 {
                            config_mode |= AC_CM_DET_CFG_CHANGE;
                            bs_anchor = bs.clone();
                        } else {
                            config_mode |= AC_CM_ALLOC_MEM;
                            *bs = bs_anchor.clone();
                        }

                        error_status = clatm_demux_read_stream_mux_config(
                            bs,
                            p_latm_demux,
                            cb,
                            p_asc,
                            pf_config_found,
                            config_mode,
                            config_changed,
                        );
                        if error_status != TRANSPORTDEC_OK {
                            break 'bail;
                        }

                        if i == 0
                            && (p_asc[0].aac_config_changed != 0
                                || p_asc[0].sbr_config_changed != 0
                                || p_asc[0].sac_config_changed != 0)
                        {
                            config_changed = 1;
                            let err_c = cb.free_mem(&p_asc[0]);
                            if err_c != 0 {
                                error_status = TRANSPORTDEC_PARSE_ERROR;
                                break 'bail;
                            }
                        }
                    }
                }
            }
        }

        // If there was no configuration read, its not possible to parse PayloadLengthInfo
        // below.
        if *pf_config_found == 0 {
            error_status = TRANSPORTDEC_SYNC_ERROR;
            break 'bail;
        }

        if p_latm_demux.m_audio_mux_version_a == 0 {
            // Do only once per call, because parsing and decoding is done in-line.
            error_status = clatm_demux_read_payload_length_info(bs, p_latm_demux);
            if error_status != TRANSPORTDEC_OK {
                *pf_config_found = 0;
                break 'bail;
            }
        } else {
            // audioMuxVersionA > 0 is reserved for future extensions
            error_status = TRANSPORTDEC_UNSUPPORTED_FORMAT;
            *pf_config_found = 0;
            break 'bail;
        }
    }

    // bail:
    if error_status != TRANSPORTDEC_OK {
        p_latm_demux.apply_asc = 1;
    }

    error_status
}

/// `CLatmDemux_Read(bs, pLatmDemux, tt, pTpDecCallbacks, pAsc, pfConfigFound,
/// ignoreBufferFullness)`: reads an AudioMuxElement.
pub fn clatm_demux_read(
    bs: &mut FdkBitStream,
    p_latm_demux: &mut CLatmDemux,
    tt: TransportType,
    cb: &mut dyn TpDecCallbacks,
    p_asc: &mut [CSAudioSpecificConfig],
    pf_config_found: &mut i32,
    ignore_buffer_fullness: i32,
) -> TransportDecError {
    let audio_mux_length_bytes_last: u32 = 0;

    let cnt_bits = fdk_get_valid_bits(bs);

    if (cnt_bits as i32) < MIN_LATM_HEADERLENGTH {
        return TRANSPORTDEC_NOT_ENOUGH_BITS;
    }

    let error_status = clatm_demux_read_audio_mux_element(
        bs,
        p_latm_demux,
        (tt != TT_MP4_LATM_MCP0) as i32,
        cb,
        p_asc,
        pf_config_found,
    );
    if error_status != TRANSPORTDEC_OK {
        return error_status;
    }

    if ignore_buffer_fullness == 0 {
        // UINT arithmetic: m_channelConfiguration (SCHAR) converts to unsigned.
        let cmp_buffer_fullness: u32 =
            24u32.wrapping_add(audio_mux_length_bytes_last.wrapping_mul(8)).wrapping_add(
                p_latm_demux.m_linfo[0][0]
                    .m_buffer_fullness
                    .wrapping_mul(
                        p_asc[tpdec_trackindex(0, 0)].m_channel_configuration as i32 as u32,
                    )
                    .wrapping_mul(32),
            );

        // evaluate buffer fullness

        if p_latm_demux.m_linfo[0][0].m_buffer_fullness != 0xFF
            && p_latm_demux.buffer_fullness_achieved == 0
        {
            if cnt_bits < cmp_buffer_fullness {
                // condition for start of decoding is not fulfilled

                // the current frame will not be decoded
                return TRANSPORTDEC_NOT_ENOUGH_BITS;
            } else {
                p_latm_demux.buffer_fullness_achieved = 1;
            }
        }
    }

    error_status
}

/// `CLatmDemux_ReadStreamMuxConfig(bs, pLatmDemux, pTpDecCallbacks, pAsc, pfConfigFound,
/// configMode, configChanged)`: reads a StreamMuxConfig.
pub fn clatm_demux_read_stream_mux_config(
    bs: &mut FdkBitStream,
    p_latm_demux: &mut CLatmDemux,
    cb: &mut dyn TpDecCallbacks,
    p_asc: &mut [CSAudioSpecificConfig],
    pf_config_found: &mut i32,
    config_mode: u8,
    config_changed: u8,
) -> TransportDecError {
    // the actual config is needed for flushing, after that new config can be parsed
    // (uninitialized in C; AudioSpecificConfig_Parse() initializes it before use)
    let mut asc_dummy = CSAudioSpecificConfig::zeroed();
    p_latm_demux.usac_explicit_cfg_changed = 0;
    let mut error_status = TRANSPORTDEC_OK;
    let mut update_config = [0u8; 1];

    'bail: {
        p_latm_demux.m_audio_mux_version = fdk_read_bits(bs, 1) as u8;

        if p_latm_demux.m_audio_mux_version == 0 {
            p_latm_demux.m_audio_mux_version_a = 0;
        } else {
            p_latm_demux.m_audio_mux_version_a = fdk_read_bits(bs, 1) as u8;
        }

        if p_latm_demux.m_audio_mux_version_a == 0 {
            if p_latm_demux.m_audio_mux_version == 1 {
                p_latm_demux.m_tara_buffer_fullness = clatm_demux_get_value(bs);
            }
            p_latm_demux.m_all_streams_same_time_framing = fdk_read_bits(bs, 1) as u8;
            p_latm_demux.m_no_sub_frames = (fdk_read_bits(bs, 6) + 1) as u8;
            p_latm_demux.m_num_program = (fdk_read_bits(bs, 4) + 1) as u8;

            if p_latm_demux.m_num_program as usize > LATM_MAX_PROG {
                error_status = TRANSPORTDEC_UNSUPPORTED_FORMAT;
                break 'bail;
            }

            let mut id_cnt: u32 = 0;
            for prog in 0..p_latm_demux.m_num_program as u32 {
                p_latm_demux.m_num_layer[prog as usize] = (fdk_read_bits(bs, 3) + 1) as u8;
                if p_latm_demux.m_num_layer[prog as usize] as usize > LATM_MAX_LAYER {
                    error_status = TRANSPORTDEC_UNSUPPORTED_FORMAT;
                    break 'bail;
                }

                for lay in 0..p_latm_demux.m_num_layer[prog as usize] as u32 {
                    let use_same_config: u32;
                    let track = tpdec_trackindex(prog, lay);
                    {
                        let p_linfo = &mut p_latm_demux.m_linfo[prog as usize][lay as usize];
                        p_linfo.m_stream_id = id_cnt;
                        id_cnt += 1;
                        p_linfo.m_frame_length_in_bits = 0;
                    }

                    if prog == 0 && lay == 0 {
                        use_same_config = 0;
                    } else {
                        use_same_config = fdk_read_bits(bs, 1);
                    }

                    if use_same_config != 0 {
                        if lay > 0 {
                            // Unreachable (one layer): FDKmemcpy(&pAsc[track], &pAsc[track - 1])
                            if track < p_asc.len() {
                                p_asc[track] = p_asc[track - 1];
                            }
                        } else {
                            error_status = TRANSPORTDEC_PARSE_ERROR;
                            break 'bail;
                        }
                    } else {
                        let mut usac_config_length_prev: u32 = 0;
                        // (uninitialized in C; only read up to usacConfigLengthPrev bytes,
                        // which are written)
                        let mut usac_config_prev = [0u8; TP_USAC_MAX_CONFIG_LEN];

                        if p_latm_demux.apply_asc == 0 && p_asc[track].m_aot == AOT_USAC {
                            // store previous USAC config length
                            usac_config_length_prev =
                                (p_asc[track].m_sc.m_usac_config.usac_config_bits as u32 + 7) >> 3;
                            if usac_config_length_prev > TP_USAC_MAX_CONFIG_LEN as u32 {
                                error_status = TRANSPORTDEC_PARSE_ERROR;
                                break 'bail;
                            }
                            usac_config_prev = [0u8; TP_USAC_MAX_CONFIG_LEN];
                            // store previous USAC config
                            let n = usac_config_length_prev as usize;
                            usac_config_prev[..n]
                                .copy_from_slice(&p_asc[track].m_sc.m_usac_config.usac_config[..n]);
                        }
                        if p_latm_demux.m_audio_mux_version == 1 {
                            let asc_len = clatm_demux_get_value(bs) as i32;
                            // The ascLen could be wrong, so check if validBits<=bufBits
                            if asc_len < 0 || asc_len > fdk_get_valid_bits(bs) as i32 {
                                error_status = TRANSPORTDEC_PARSE_ERROR;
                                break 'bail;
                            }
                            fdk_sync_cache(bs);
                            let mut tmp_bs = bs.clone();
                            tmp_bs.h_bit_buf.valid_bits = asc_len as u32;

                            // Read ASC
                            let target = if p_latm_demux.apply_asc != 0 {
                                &mut p_asc[track]
                            } else {
                                &mut asc_dummy
                            };
                            error_status = audio_specific_config_parse(
                                target,
                                &mut tmp_bs,
                                1,
                                cb,
                                config_mode,
                                config_changed,
                                AOT_NULL_OBJECT,
                            );
                            if error_status != TRANSPORTDEC_OK {
                                break 'bail;
                            }

                            // The field p_linfo->m_ascLen could be wrong, so check if
                            if 0 > fdk_get_valid_bits(&mut tmp_bs) as i32 {
                                error_status = TRANSPORTDEC_PARSE_ERROR;
                                break 'bail;
                            }
                            fdk_push_for(bs, asc_len as u32); // position bitstream after ASC
                        } else {
                            // Read ASC
                            let target = if p_latm_demux.apply_asc != 0 {
                                &mut p_asc[track]
                            } else {
                                &mut asc_dummy
                            };
                            error_status = audio_specific_config_parse(
                                target,
                                bs,
                                0,
                                cb,
                                config_mode,
                                config_changed,
                                AOT_NULL_OBJECT,
                            );
                            if error_status != TRANSPORTDEC_OK {
                                break 'bail;
                            }
                        }
                        if p_latm_demux.apply_asc == 0 {
                            update_config[track] = 0;
                        } else {
                            update_config[track] = 1;
                        }

                        if p_latm_demux.apply_asc == 0 && asc_dummy.m_aot == AOT_USAC {
                            // flush in case SMC has changed
                            let usac_config_length =
                                (asc_dummy.m_sc.m_usac_config.usac_config_bits as u32 + 7) >> 3;
                            if usac_config_length > TP_USAC_MAX_CONFIG_LEN as u32 {
                                error_status = TRANSPORTDEC_PARSE_ERROR;
                                break 'bail;
                            }
                            let n = usac_config_length as usize;
                            let np = usac_config_length_prev as usize;
                            if usac_config_length != usac_config_length_prev
                                || usac_config_prev[..np]
                                    != asc_dummy.m_sc.m_usac_config.usac_config[..np]
                            {
                                let dst = &mut p_asc[track].m_sc.m_usac_config;
                                dst.usac_config = [0; TP_USAC_MAX_CONFIG_LEN];
                                // store new USAC config
                                dst.usac_config[..n].copy_from_slice(
                                    &asc_dummy.m_sc.m_usac_config.usac_config[..n],
                                );
                                dst.usac_config_bits =
                                    asc_dummy.m_sc.m_usac_config.usac_config_bits;
                                p_latm_demux.usac_explicit_cfg_changed = 1;
                            }

                            if asc_dummy.m_sc.m_usac_config.m_usac_num_elements != 0
                                && asc_dummy.m_sc.m_usac_config.element[0]
                                    .ext_element
                                    .usac_ext_element_has_audio_pre_roll
                                    != 0
                            {
                                // if dummy parsed cfg has audioPreRoll we first flush before
                                // applying new cfg
                                p_latm_demux.new_cfg_has_audio_pre_roll = 1;
                            }
                        }
                    }

                    let p_linfo = &mut p_latm_demux.m_linfo[prog as usize][lay as usize];
                    p_linfo.m_frame_length_type = fdk_read_bits(bs, 3);
                    match p_linfo.m_frame_length_type {
                        0 => {
                            p_linfo.m_buffer_fullness = fdk_read_bits(bs, 8);

                            if p_latm_demux.m_all_streams_same_time_framing == 0
                                && lay > 0
                                && (p_asc[track].m_aot == AOT_AAC_SCAL
                                    || p_asc[track].m_aot == AOT_ER_AAC_SCAL)
                                && (p_asc[track - 1].m_aot == AOT_CELP
                                    || p_asc[track - 1].m_aot == AOT_ER_CELP)
                            {
                                // The layer maybe ignored later so read it anyway:
                                // coreFrameOffset =
                                fdk_read_bits(bs, 6);
                            }
                        }
                        1 => {
                            p_linfo.m_frame_length_in_bits = fdk_read_bits(bs, 9);
                        }
                        // 3, 4, 5: CELP; 6, 7: HVXC
                        _ => {
                            error_status = TRANSPORTDEC_PARSE_ERROR;
                            break 'bail;
                        }
                    }
                } // layer loop
            } // prog loop

            p_latm_demux.m_other_data_present = fdk_read_bits(bs, 1) as u8;
            p_latm_demux.m_other_data_length = 0;

            if p_latm_demux.m_other_data_present != 0 {
                if p_latm_demux.m_audio_mux_version == 1 {
                    p_latm_demux.m_other_data_length = clatm_demux_get_value(bs);
                } else {
                    loop {
                        p_latm_demux.m_other_data_length <<= 8; // *= 256
                        let other_data_len_esc = fdk_read_bits(bs, 1);
                        p_latm_demux.m_other_data_length =
                            p_latm_demux.m_other_data_length.wrapping_add(fdk_read_bits(bs, 8));
                        if other_data_len_esc == 0 {
                            break;
                        }
                    }
                }
                if p_latm_demux.m_audio_mux_length_bytes < (p_latm_demux.m_other_data_length >> 3) {
                    error_status = TRANSPORTDEC_PARSE_ERROR;
                    break 'bail;
                }
            }

            p_latm_demux.m_crc_check_present = fdk_read_bits(bs, 1) as u8;

            if p_latm_demux.m_crc_check_present != 0 {
                fdk_read_bits(bs, 8);
            }
        } else {
            // audioMuxVersionA > 0 is reserved for future extensions
            error_status = TRANSPORTDEC_UNSUPPORTED_FORMAT;
        }

        // Configure source decoder:
        if error_status == TRANSPORTDEC_OK {
            for prog in 0..p_latm_demux.m_num_program as u32 {
                for lay in 0..p_latm_demux.m_num_layer[prog as usize] as u32 {
                    let track = tpdec_trackindex(prog, lay);
                    if update_config[track] != 0 {
                        // configChanged points into the ASC (see TpDecCallbacks::update_config)
                        let mut cc = p_asc[track].aac_config_changed;
                        let mode = p_asc[track].config_mode;
                        let cb_error = cb.update_config(&p_asc[track], mode, &mut cc);
                        p_asc[track].aac_config_changed = cc;
                        if cb_error == TRANSPORTDEC_NEED_TO_RESTART {
                            *pf_config_found = 0;
                            error_status = TRANSPORTDEC_NEED_TO_RESTART;
                            break 'bail;
                        }
                        if cb_error != 0 {
                            *pf_config_found = 0;
                            if lay == 0 {
                                error_status = TRANSPORTDEC_SYNC_ERROR;
                                break 'bail;
                            }
                        } else {
                            *pf_config_found = 1;
                        }
                    } else {
                        *pf_config_found = 1;
                    }
                }
            }
        }
    }

    // bail:
    if error_status != TRANSPORTDEC_OK {
        let apply_asc = p_latm_demux.apply_asc;
        *p_latm_demux = CLatmDemux::default(); // reset structure
        p_latm_demux.apply_asc = apply_asc;
    } else {
        // no error and config parsing is finished
        if config_mode == AC_CM_ALLOC_MEM {
            p_latm_demux.apply_asc = 0;
        }
    }

    error_status
}

/// `CLatmDemux_ReadAuChunkLengthInfo(bs)`: the chunk length in bits, -1 if the escape sequence
/// runs past the valid bytes.
fn clatm_demux_read_au_chunk_length_info(bs: &mut FdkBitStream) -> i32 {
    let mut len = 0i32;
    let mut tmp = 255i32;
    let mut valid_bytes = (fdk_get_valid_bits(bs) as i32) >> 3;

    while tmp == 255 && {
        let v = valid_bytes;
        valid_bytes -= 1;
        v > 0
    } {
        tmp = fdk_read_bits(bs, 8) as i32;
        len += tmp;
    }

    if tmp == 255 {
        -1
    } else {
        len << 3
    }
}

/// `CLatmDemux_ReadPayloadLengthInfo(bs, pLatmDemux)`
pub fn clatm_demux_read_payload_length_info(
    bs: &mut FdkBitStream,
    p_latm_demux: &mut CLatmDemux,
) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;
    let mut total_payload_bits: i32 = 0;

    if p_latm_demux.m_all_streams_same_time_framing == 1 {
        fdk_aac_rs_fdk::fdk_assert!(p_latm_demux.m_num_program as usize <= LATM_MAX_PROG);
        for prog in 0..p_latm_demux.m_num_program as usize {
            fdk_aac_rs_fdk::fdk_assert!(p_latm_demux
                .m_num_layer
                .get(prog)
                .is_none_or(|&n| n as usize <= LATM_MAX_LAYER));
            // m_numProgram and m_numLayer are at most 1 after a successful StreamMuxConfig and
            // 0 after a reset; the port bounds the loops by the arrays.
            let num_layer = match p_latm_demux.m_num_layer.get(prog) {
                Some(&n) => n as usize,
                None => 0,
            };
            for lay in 0..num_layer {
                let Some(p_linfo) = p_latm_demux.m_linfo.get_mut(prog).and_then(|l| l.get_mut(lay))
                else {
                    break;
                };

                match p_linfo.m_frame_length_type {
                    0 => {
                        let au_chunk_length_info = clatm_demux_read_au_chunk_length_info(bs);
                        if au_chunk_length_info >= 0 {
                            p_linfo.m_frame_length_in_bits = au_chunk_length_info as u32;
                            total_payload_bits = total_payload_bits
                                .wrapping_add(p_linfo.m_frame_length_in_bits as i32);
                        } else {
                            return TRANSPORTDEC_PARSE_ERROR;
                        }
                    }
                    // 3, 5, 7 and others
                    _ => return TRANSPORTDEC_PARSE_ERROR, // AAC_DEC_LATM_INVALIDFRAMELENGTHTYPE;
                }
            }
        }
    } else {
        error_status = TRANSPORTDEC_PARSE_ERROR; // AAC_DEC_LATM_TIMEFRAMING;
    }
    if p_latm_demux.m_audio_mux_length_bytes > 0
        && total_payload_bits > (p_latm_demux.m_audio_mux_length_bytes as i32).wrapping_mul(8)
    {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    error_status
}

/// `CLatmDemux_GetFrameLengthInBits(pLatmDemux, prog, layer)`
pub fn clatm_demux_get_frame_length_in_bits(
    p_latm_demux: &CLatmDemux,
    prog: u32,
    layer: u32,
) -> u32 {
    let mut n_frame_len_bits = 0;
    if prog < p_latm_demux.m_num_program as u32 {
        if let (Some(&nl), Some(li)) =
            (p_latm_demux.m_num_layer.get(prog as usize), p_latm_demux.m_linfo.get(prog as usize))
        {
            if layer < nl as u32 {
                if let Some(l) = li.get(layer as usize) {
                    n_frame_len_bits = l.m_frame_length_in_bits;
                }
            }
        }
    }
    n_frame_len_bits
}

/// `CLatmDemux_GetOtherDataPresentFlag(pLatmDemux)`
pub fn clatm_demux_get_other_data_present_flag(p_latm_demux: &CLatmDemux) -> u32 {
    if p_latm_demux.m_other_data_present != 0 {
        1
    } else {
        0
    }
}

/// `CLatmDemux_GetOtherDataLength(pLatmDemux)`
pub fn clatm_demux_get_other_data_length(p_latm_demux: &CLatmDemux) -> u32 {
    p_latm_demux.m_other_data_length
}

/// `CLatmDemux_GetNrOfSubFrames(pLatmDemux)`
pub fn clatm_demux_get_nr_of_sub_frames(p_latm_demux: &CLatmDemux) -> u32 {
    p_latm_demux.m_no_sub_frames as u32
}

/// `CLatmDemux_GetNrOfLayers(pLatmDemux, prog)`
pub fn clatm_demux_get_nr_of_layers(p_latm_demux: &CLatmDemux, prog: u32) -> u32 {
    let mut num_layer = 0;
    if prog < p_latm_demux.m_num_program as u32 {
        if let Some(&n) = p_latm_demux.m_num_layer.get(prog as usize) {
            num_layer = n as u32;
        }
    }
    num_layer
}

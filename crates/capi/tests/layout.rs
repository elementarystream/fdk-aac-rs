// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! Layout of the API structures: compiles `tests/layout.c` against the installed headers
//! (`include/fdk-aac/`) with the C compiler (`$CC`, default `cc`) for the target architecture,
//! runs it, and compares every `sizeof`, `_Alignof` and `offsetof` with the Rust definitions
//! (PORTING.md, "C ABI"), for the encoder and the decoder structures.

use core::mem::{align_of, offset_of, size_of};
use fdk_aac::dec::*;
use fdk_aac::enc::*;
use fdk_aac::lib_info::LIB_INFO;
use std::path::Path;
use std::process::Command;

macro_rules! size {
    ($v:ident, $c:literal, $t:ty) => {
        $v.push((concat!("sizeof(", $c, ")").to_string(), size_of::<$t>()));
        $v.push((concat!("alignof(", $c, ")").to_string(), align_of::<$t>()));
    };
}

macro_rules! off {
    ($v:ident, $c:literal, $cf:literal, $t:ty, $($f:tt)+) => {
        $v.push((concat!("offsetof(", $c, ", ", $cf, ")").to_string(), offset_of!($t, $($f)+)));
    };
}

fn expected() -> Vec<(String, usize)> {
    let mut v = Vec::new();
    size!(v, "AACENC_BufDesc", AACENC_BufDesc);
    off!(v, "AACENC_BufDesc", "numBufs", AACENC_BufDesc, num_bufs);
    off!(v, "AACENC_BufDesc", "bufs", AACENC_BufDesc, bufs);
    off!(v, "AACENC_BufDesc", "bufferIdentifiers", AACENC_BufDesc, buffer_identifiers);
    off!(v, "AACENC_BufDesc", "bufSizes", AACENC_BufDesc, buf_sizes);
    off!(v, "AACENC_BufDesc", "bufElSizes", AACENC_BufDesc, buf_el_sizes);

    size!(v, "AACENC_InArgs", AACENC_InArgs);
    off!(v, "AACENC_InArgs", "numInSamples", AACENC_InArgs, num_in_samples);
    off!(v, "AACENC_InArgs", "numAncBytes", AACENC_InArgs, num_anc_bytes);

    size!(v, "AACENC_OutArgs", AACENC_OutArgs);
    off!(v, "AACENC_OutArgs", "numOutBytes", AACENC_OutArgs, num_out_bytes);
    off!(v, "AACENC_OutArgs", "numInSamples", AACENC_OutArgs, num_in_samples);
    off!(v, "AACENC_OutArgs", "numAncBytes", AACENC_OutArgs, num_anc_bytes);
    off!(v, "AACENC_OutArgs", "bitResState", AACENC_OutArgs, bit_res_state);

    size!(v, "AACENC_InfoStruct", AACENC_InfoStruct);
    off!(v, "AACENC_InfoStruct", "maxOutBufBytes", AACENC_InfoStruct, max_out_buf_bytes);
    off!(v, "AACENC_InfoStruct", "maxAncBytes", AACENC_InfoStruct, max_anc_bytes);
    off!(v, "AACENC_InfoStruct", "inBufFillLevel", AACENC_InfoStruct, in_buf_fill_level);
    off!(v, "AACENC_InfoStruct", "inputChannels", AACENC_InfoStruct, input_channels);
    off!(v, "AACENC_InfoStruct", "frameLength", AACENC_InfoStruct, frame_length);
    off!(v, "AACENC_InfoStruct", "nDelay", AACENC_InfoStruct, n_delay);
    off!(v, "AACENC_InfoStruct", "nDelayCore", AACENC_InfoStruct, n_delay_core);
    off!(v, "AACENC_InfoStruct", "confBuf", AACENC_InfoStruct, conf_buf);
    off!(v, "AACENC_InfoStruct", "confSize", AACENC_InfoStruct, conf_size);

    size!(v, "AACENC_MetaData", AACENC_MetaData);
    off!(v, "AACENC_MetaData", "drc_profile", AACENC_MetaData, drc_profile);
    off!(v, "AACENC_MetaData", "comp_profile", AACENC_MetaData, comp_profile);
    off!(v, "AACENC_MetaData", "drc_TargetRefLevel", AACENC_MetaData, drc_target_ref_level);
    off!(v, "AACENC_MetaData", "comp_TargetRefLevel", AACENC_MetaData, comp_target_ref_level);
    off!(v, "AACENC_MetaData", "prog_ref_level_present", AACENC_MetaData, prog_ref_level_present);
    off!(v, "AACENC_MetaData", "prog_ref_level", AACENC_MetaData, prog_ref_level);
    off!(v, "AACENC_MetaData", "PCE_mixdown_idx_present", AACENC_MetaData, pce_mixdown_idx_present);
    off!(v, "AACENC_MetaData", "ETSI_DmxLvl_present", AACENC_MetaData, etsi_dmx_lvl_present);
    off!(v, "AACENC_MetaData", "centerMixLevel", AACENC_MetaData, center_mix_level);
    off!(v, "AACENC_MetaData", "surroundMixLevel", AACENC_MetaData, surround_mix_level);
    off!(v, "AACENC_MetaData", "dolbySurroundMode", AACENC_MetaData, dolby_surround_mode);
    off!(v, "AACENC_MetaData", "drcPresentationMode", AACENC_MetaData, drc_presentation_mode);
    off!(v, "AACENC_MetaData", "ExtMetaData", AACENC_MetaData, ext_meta_data);
    let ext = offset_of!(AACENC_MetaData, ext_meta_data);
    for (c, r) in [
        ("extAncDataEnable", offset_of!(AACENC_ExtMetaData, ext_anc_data_enable)),
        ("extDownmixLevelEnable", offset_of!(AACENC_ExtMetaData, ext_downmix_level_enable)),
        ("extDownmixLevel_A", offset_of!(AACENC_ExtMetaData, ext_downmix_level_a)),
        ("extDownmixLevel_B", offset_of!(AACENC_ExtMetaData, ext_downmix_level_b)),
        ("dmxGainEnable", offset_of!(AACENC_ExtMetaData, dmx_gain_enable)),
        ("dmxGain5", offset_of!(AACENC_ExtMetaData, dmx_gain5)),
        ("dmxGain2", offset_of!(AACENC_ExtMetaData, dmx_gain2)),
        ("lfeDmxEnable", offset_of!(AACENC_ExtMetaData, lfe_dmx_enable)),
        ("lfeDmxLevel", offset_of!(AACENC_ExtMetaData, lfe_dmx_level)),
    ] {
        v.push((format!("offsetof(AACENC_MetaData, ExtMetaData.{c})"), ext + r));
    }
    v.push(("sizeof(AACENC_MetaData.ExtMetaData)".to_string(), size_of::<AACENC_ExtMetaData>()));

    size!(v, "LIB_INFO", LIB_INFO);
    off!(v, "LIB_INFO", "title", LIB_INFO, title);
    off!(v, "LIB_INFO", "build_date", LIB_INFO, build_date);
    off!(v, "LIB_INFO", "build_time", LIB_INFO, build_time);
    off!(v, "LIB_INFO", "module_id", LIB_INFO, module_id);
    off!(v, "LIB_INFO", "version", LIB_INFO, version);
    off!(v, "LIB_INFO", "flags", LIB_INFO, flags);
    off!(v, "LIB_INFO", "versionStr", LIB_INFO, versionStr);

    size!(v, "HANDLE_AACENCODER", HANDLE_AACENCODER);
    size!(v, "AACENC_ERROR", AACENC_ERROR);
    size!(v, "AACENC_PARAM", AACENC_PARAM);
    size!(v, "FDK_MODULE_ID", core::ffi::c_int);
    v.push(("FDK_MODULE_LAST".to_string(), fdk_aac_rs_fdk::audio::FDK_MODULE_LAST as usize));

    size!(v, "CStreamInfo", CStreamInfo);
    off!(v, "CStreamInfo", "sampleRate", CStreamInfo, sampleRate);
    off!(v, "CStreamInfo", "frameSize", CStreamInfo, frameSize);
    off!(v, "CStreamInfo", "numChannels", CStreamInfo, numChannels);
    off!(v, "CStreamInfo", "pChannelType", CStreamInfo, pChannelType);
    off!(v, "CStreamInfo", "pChannelIndices", CStreamInfo, pChannelIndices);
    off!(v, "CStreamInfo", "aacSampleRate", CStreamInfo, aacSampleRate);
    off!(v, "CStreamInfo", "profile", CStreamInfo, profile);
    off!(v, "CStreamInfo", "aot", CStreamInfo, aot);
    off!(v, "CStreamInfo", "channelConfig", CStreamInfo, channelConfig);
    off!(v, "CStreamInfo", "bitRate", CStreamInfo, bitRate);
    off!(v, "CStreamInfo", "aacSamplesPerFrame", CStreamInfo, aacSamplesPerFrame);
    off!(v, "CStreamInfo", "aacNumChannels", CStreamInfo, aacNumChannels);
    off!(v, "CStreamInfo", "extAot", CStreamInfo, extAot);
    off!(v, "CStreamInfo", "extSamplingRate", CStreamInfo, extSamplingRate);
    off!(v, "CStreamInfo", "outputDelay", CStreamInfo, outputDelay);
    off!(v, "CStreamInfo", "flags", CStreamInfo, flags);
    off!(v, "CStreamInfo", "epConfig", CStreamInfo, epConfig);
    off!(v, "CStreamInfo", "numLostAccessUnits", CStreamInfo, numLostAccessUnits);
    off!(v, "CStreamInfo", "numTotalBytes", CStreamInfo, numTotalBytes);
    off!(v, "CStreamInfo", "numBadBytes", CStreamInfo, numBadBytes);
    off!(v, "CStreamInfo", "numTotalAccessUnits", CStreamInfo, numTotalAccessUnits);
    off!(v, "CStreamInfo", "numBadAccessUnits", CStreamInfo, numBadAccessUnits);
    off!(v, "CStreamInfo", "drcProgRefLev", CStreamInfo, drcProgRefLev);
    off!(v, "CStreamInfo", "drcPresMode", CStreamInfo, drcPresMode);
    off!(v, "CStreamInfo", "outputLoudness", CStreamInfo, outputLoudness);

    size!(v, "HANDLE_AACDECODER", HANDLE_AACDECODER);
    size!(v, "AAC_DECODER_ERROR", AAC_DECODER_ERROR);
    size!(v, "AACDEC_PARAM", AACDEC_PARAM);
    size!(v, "TRANSPORT_TYPE", TRANSPORT_TYPE);
    size!(v, "AUDIO_OBJECT_TYPE", AUDIO_OBJECT_TYPE);
    size!(v, "AUDIO_CHANNEL_TYPE", AUDIO_CHANNEL_TYPE);
    size!(v, "INT_PCM", INT_PCM);
    v
}

#[test]
fn layout_matches_the_c_headers() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "i686"
    } else {
        "x86_64"
    };
    let exe = tmp.join(format!("layout-{arch}"));
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let mut cmd = Command::new(cc);
    if cfg!(target_vendor = "apple") {
        cmd.args(["-arch", arch]);
    } else if cfg!(target_arch = "x86") {
        // the C program must have the layout of the 32-bit target
        cmd.arg("-m32");
    }
    let status = cmd
        .arg("-std=c11")
        .arg("-I")
        .arg(dir.join("include/fdk-aac"))
        .arg(dir.join("tests/layout.c"))
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("run the C compiler");
    assert!(status.success(), "compiling tests/layout.c failed");
    let out = Command::new(&exe).output().expect("run the layout program");
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).expect("utf-8");
    let c: Vec<(String, usize)> = text
        .lines()
        .map(|l| {
            let (k, v) = l.rsplit_once(' ').expect("name value");
            (k.to_string(), v.parse().expect("number"))
        })
        .collect();
    let rust = expected();
    assert_eq!(c.len(), rust.len(), "number of items");
    for (a, b) in c.iter().zip(rust.iter()) {
        assert_eq!(a, b, "C (left) vs Rust (right)");
    }
    println!("{} layout items match ({arch})", c.len());
}

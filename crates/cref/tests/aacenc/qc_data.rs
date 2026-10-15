// Differential tests of fdk_aac_rs_aacenc::qc_data against the C++ reference.
//
// The port declares the plain-data structs of the encoder with the memory layout of C, so that
// the tests can compare complete C states with the port's states (by copying the C struct into
// the Rust type). This test checks the sizes and some field offsets against the reference.

use fdk_aac_rs_aacenc::aacenc::AacencConfig;
use fdk_aac_rs_aacenc::aacenc_pns::{PnsConfig, PnsData};
use fdk_aac_rs_aacenc::aacenc_tns::*;
use fdk_aac_rs_aacenc::adj_thr_data::*;
use fdk_aac_rs_aacenc::block_switch::BlockSwitchingControl;
use fdk_aac_rs_aacenc::channel_map::ChannelModeConfigTab;
use fdk_aac_rs_aacenc::dyn_bits::{SectionData, SectionInfo};
use fdk_aac_rs_aacenc::interface::{PsyOutChannel, ToolsInfo};
use fdk_aac_rs_aacenc::line_pe::{PeChannelData, PeData};
use fdk_aac_rs_aacenc::pnsparam::NoiseParams;
use fdk_aac_rs_aacenc::psy_configuration::*;
use fdk_aac_rs_aacenc::psy_data::PsyData;
use fdk_aac_rs_aacenc::qc_data::*;
use fdk_aac_rs_cref::aacenc_qc_data as c;
use std::collections::HashMap;
use std::ffi::CStr;
use std::mem::{offset_of, size_of};

fn c_layouts() -> HashMap<String, usize> {
    let n = unsafe { c::cref_qc_data_layout_count() };
    (0..n)
        .map(|i| {
            let mut v = 0;
            let name = unsafe { CStr::from_ptr(c::cref_qc_data_layout(i, &mut v)) };
            (name.to_str().unwrap().to_string(), v as usize)
        })
        .collect()
}

#[test]
fn layouts_match() {
    let c = c_layouts();
    let check = |name: &str, rust: usize| {
        assert_eq!(c[name], rust, "{name}");
    };
    check("BLOCK_SWITCHING_CONTROL", size_of::<BlockSwitchingControl>());
    check("TNS_PARAMETER_TABULATED", size_of::<TnsParameterTabulated>());
    check("TNS_CONFIG", size_of::<TnsConfig>());
    check("TNS_SUBBLOCK_INFO", size_of::<TnsSubblockInfo>());
    check("TNS_DATA_RAW", size_of::<TnsDataRaw>());
    check("TNS_DATA", size_of::<TnsData>());
    check("TNS_INFO", size_of::<TnsInfo>());
    check("NOISEPARAMS", size_of::<NoiseParams>());
    check("NOISEPARAMS.gapFillThr", offset_of!(NoiseParams, gap_fill_thr));
    check("PNS_CONFIG", size_of::<PnsConfig>());
    check("PNS_DATA", size_of::<PnsData>());
    // The size of a C pointer (4 bytes on 32-bit x86).
    let ptr = size_of::<*const u8>();
    // PSY_DATA starts with the pointer mdctSpectrum, which the port does not have.
    check("PSY_DATA.sfbThreshold", ptr);
    check("PSY_DATA", ptr + size_of::<PsyData>());
    check("TOOLSINFO", size_of::<ToolsInfo>());
    // PSY_OUT_CHANNEL ends with the six pointers into QC_OUT_CHANNEL.
    check("PSY_OUT_CHANNEL.mdctSpectrum", size_of::<PsyOutChannel>());
    check("PSY_OUT_CHANNEL", size_of::<PsyOutChannel>() + 6 * ptr);
    check("SECTION_INFO", size_of::<SectionInfo>());
    check("SECTION_DATA", size_of::<SectionData>());
    check("PE_CHANNEL_DATA", size_of::<PeChannelData>());
    check("PE_DATA", size_of::<PeData>());
    check("BRES_PARAM", size_of::<BresParam>());
    check("AH_PARAM", size_of::<AhParam>());
    check("MINSNR_ADAPT_PARAM", size_of::<MinsnrAdaptParam>());
    check("ATS_ELEMENT", size_of::<AtsElement>());
    check("AACENC_CONFIG", size_of::<AacencConfig>());
    check("AACENC_CONFIG.epConfig", offset_of!(AacencConfig, ep_config));
    check("AACENC_CONFIG.anc_Rate", offset_of!(AacencConfig, anc_rate));
    check("AACENC_CONFIG.downscaleFactor", offset_of!(AacencConfig, downscale_factor));
    check("ELEMENT_INFO", size_of::<ElementInfo>());
    check("CHANNEL_MAPPING", size_of::<ChannelMapping>());
    check("PADDING", size_of::<Padding>());
    check("QC_OUT_CHANNEL", size_of::<QcOutChannel>());
    check("QC_OUT_CHANNEL.sectionData", offset_of!(QcOutChannel, section_data));
    check("ELEMENT_BITS", size_of::<ElementBits>());
    check("PSY_CONFIGURATION", size_of::<PsyConfiguration>());
    check(
        "PSY_CONFIGURATION.minRemainingThresholdFactor",
        offset_of!(PsyConfiguration, min_remaining_threshold_factor),
    );
    check("PSY_CONFIGURATION.tnsConf", offset_of!(PsyConfiguration, tns_conf));
    check("PSY_CONFIGURATION.pnsConf", offset_of!(PsyConfiguration, pns_conf));
    check("SFB_PARAM_LONG", size_of::<SfbParamLong>());
    check("SFB_PARAM_SHORT", size_of::<SfbParamShort>());
    check("CHANNEL_MODE_CONFIG_TAB", size_of::<ChannelModeConfigTab>());
}

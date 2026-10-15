// Differential tests of fdk_aac_rs_fdk::audio against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_audio::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

/// Every constant of the port, in the order of `cref_audio_const`.
const CONSTANTS: &[(&str, i64)] = &[
    ("FF_UNKNOWN", FF_UNKNOWN as i64),
    ("FF_RAW", FF_RAW as i64),
    ("FF_MP4_3GPP", FF_MP4_3GPP as i64),
    ("FF_MP4_MP4F", FF_MP4_MP4F as i64),
    ("FF_RAWPACKETS", FF_RAWPACKETS as i64),
    ("TT_UNKNOWN", TT_UNKNOWN as i64),
    ("TT_MP4_RAW", TT_MP4_RAW as i64),
    ("TT_MP4_ADIF", TT_MP4_ADIF as i64),
    ("TT_MP4_ADTS", TT_MP4_ADTS as i64),
    ("TT_MP4_LATM_MCP1", TT_MP4_LATM_MCP1 as i64),
    ("TT_MP4_LATM_MCP0", TT_MP4_LATM_MCP0 as i64),
    ("TT_MP4_LOAS", TT_MP4_LOAS as i64),
    ("TT_DRM", TT_DRM as i64),
    ("AOT_NONE", AOT_NONE as i64),
    ("AOT_NULL_OBJECT", AOT_NULL_OBJECT as i64),
    ("AOT_AAC_MAIN", AOT_AAC_MAIN as i64),
    ("AOT_AAC_LC", AOT_AAC_LC as i64),
    ("AOT_AAC_SSR", AOT_AAC_SSR as i64),
    ("AOT_AAC_LTP", AOT_AAC_LTP as i64),
    ("AOT_SBR", AOT_SBR as i64),
    ("AOT_AAC_SCAL", AOT_AAC_SCAL as i64),
    ("AOT_TWIN_VQ", AOT_TWIN_VQ as i64),
    ("AOT_CELP", AOT_CELP as i64),
    ("AOT_HVXC", AOT_HVXC as i64),
    ("AOT_RSVD_10", AOT_RSVD_10 as i64),
    ("AOT_RSVD_11", AOT_RSVD_11 as i64),
    ("AOT_TTSI", AOT_TTSI as i64),
    ("AOT_MAIN_SYNTH", AOT_MAIN_SYNTH as i64),
    ("AOT_WAV_TAB_SYNTH", AOT_WAV_TAB_SYNTH as i64),
    ("AOT_GEN_MIDI", AOT_GEN_MIDI as i64),
    ("AOT_ALG_SYNTH_AUD_FX", AOT_ALG_SYNTH_AUD_FX as i64),
    ("AOT_ER_AAC_LC", AOT_ER_AAC_LC as i64),
    ("AOT_RSVD_18", AOT_RSVD_18 as i64),
    ("AOT_ER_AAC_LTP", AOT_ER_AAC_LTP as i64),
    ("AOT_ER_AAC_SCAL", AOT_ER_AAC_SCAL as i64),
    ("AOT_ER_TWIN_VQ", AOT_ER_TWIN_VQ as i64),
    ("AOT_ER_BSAC", AOT_ER_BSAC as i64),
    ("AOT_ER_AAC_LD", AOT_ER_AAC_LD as i64),
    ("AOT_ER_CELP", AOT_ER_CELP as i64),
    ("AOT_ER_HVXC", AOT_ER_HVXC as i64),
    ("AOT_ER_HILN", AOT_ER_HILN as i64),
    ("AOT_ER_PARA", AOT_ER_PARA as i64),
    ("AOT_RSVD_28", AOT_RSVD_28 as i64),
    ("AOT_PS", AOT_PS as i64),
    ("AOT_MPEGS", AOT_MPEGS as i64),
    ("AOT_ESCAPE", AOT_ESCAPE as i64),
    ("AOT_MP3ONMP4_L1", AOT_MP3ONMP4_L1 as i64),
    ("AOT_MP3ONMP4_L2", AOT_MP3ONMP4_L2 as i64),
    ("AOT_MP3ONMP4_L3", AOT_MP3ONMP4_L3 as i64),
    ("AOT_RSVD_35", AOT_RSVD_35 as i64),
    ("AOT_RSVD_36", AOT_RSVD_36 as i64),
    ("AOT_AAC_SLS", AOT_AAC_SLS as i64),
    ("AOT_SLS", AOT_SLS as i64),
    ("AOT_ER_AAC_ELD", AOT_ER_AAC_ELD as i64),
    ("AOT_USAC", AOT_USAC as i64),
    ("AOT_SAOC", AOT_SAOC as i64),
    ("AOT_LD_MPEGS", AOT_LD_MPEGS as i64),
    ("AOT_MP2_AAC_LC", AOT_MP2_AAC_LC as i64),
    ("AOT_MP2_SBR", AOT_MP2_SBR as i64),
    ("AOT_DRM_AAC", AOT_DRM_AAC as i64),
    ("AOT_DRM_SBR", AOT_DRM_SBR as i64),
    ("AOT_DRM_MPEG_PS", AOT_DRM_MPEG_PS as i64),
    ("AOT_DRM_SURROUND", AOT_DRM_SURROUND as i64),
    ("AOT_DRM_USAC", AOT_DRM_USAC as i64),
    ("MODE_INVALID", MODE_INVALID as i64),
    ("MODE_UNKNOWN", MODE_UNKNOWN as i64),
    ("MODE_1", MODE_1 as i64),
    ("MODE_2", MODE_2 as i64),
    ("MODE_1_2", MODE_1_2 as i64),
    ("MODE_1_2_1", MODE_1_2_1 as i64),
    ("MODE_1_2_2", MODE_1_2_2 as i64),
    ("MODE_1_2_2_1", MODE_1_2_2_1 as i64),
    ("MODE_1_2_2_2_1", MODE_1_2_2_2_1 as i64),
    ("MODE_6_1", MODE_6_1 as i64),
    ("MODE_7_1_BACK", MODE_7_1_BACK as i64),
    ("MODE_7_1_TOP_FRONT", MODE_7_1_TOP_FRONT as i64),
    ("MODE_7_1_REAR_SURROUND", MODE_7_1_REAR_SURROUND as i64),
    ("MODE_7_1_FRONT_CENTER", MODE_7_1_FRONT_CENTER as i64),
    ("MODE_212", MODE_212 as i64),
    ("ACT_NONE", ACT_NONE as i64),
    ("ACT_FRONT", ACT_FRONT as i64),
    ("ACT_SIDE", ACT_SIDE as i64),
    ("ACT_BACK", ACT_BACK as i64),
    ("ACT_LFE", ACT_LFE as i64),
    ("ACT_TOP", ACT_TOP as i64),
    ("ACT_FRONT_TOP", ACT_FRONT_TOP as i64),
    ("ACT_SIDE_TOP", ACT_SIDE_TOP as i64),
    ("ACT_BACK_TOP", ACT_BACK_TOP as i64),
    ("ACT_BOTTOM", ACT_BOTTOM as i64),
    ("ACT_FRONT_BOTTOM", ACT_FRONT_BOTTOM as i64),
    ("ACT_SIDE_BOTTOM", ACT_SIDE_BOTTOM as i64),
    ("ACT_BACK_BOTTOM", ACT_BACK_BOTTOM as i64),
    ("SIG_UNKNOWN", SIG_UNKNOWN as i64),
    ("SIG_IMPLICIT", SIG_IMPLICIT as i64),
    ("SIG_EXPLICIT_BW_COMPATIBLE", SIG_EXPLICIT_BW_COMPATIBLE as i64),
    ("SIG_EXPLICIT_HIERARCHICAL", SIG_EXPLICIT_HIERARCHICAL as i64),
    ("ID_NONE", ID_NONE as i64),
    ("ID_SCE", ID_SCE as i64),
    ("ID_CPE", ID_CPE as i64),
    ("ID_CCE", ID_CCE as i64),
    ("ID_LFE", ID_LFE as i64),
    ("ID_DSE", ID_DSE as i64),
    ("ID_PCE", ID_PCE as i64),
    ("ID_FIL", ID_FIL as i64),
    ("ID_END", ID_END as i64),
    ("ID_EXT", ID_EXT as i64),
    ("ID_SCAL", ID_SCAL as i64),
    ("ID_USAC_SCE", ID_USAC_SCE as i64),
    ("ID_USAC_CPE", ID_USAC_CPE as i64),
    ("ID_USAC_LFE", ID_USAC_LFE as i64),
    ("ID_USAC_EXT", ID_USAC_EXT as i64),
    ("ID_USAC_END", ID_USAC_END as i64),
    ("ID_LAST", ID_LAST as i64),
    ("ID_CONFIG_EXT_FILL", ID_CONFIG_EXT_FILL as i64),
    ("ID_CONFIG_EXT_DOWNMIX", ID_CONFIG_EXT_DOWNMIX as i64),
    ("ID_CONFIG_EXT_LOUDNESS_INFO", ID_CONFIG_EXT_LOUDNESS_INFO as i64),
    ("ID_CONFIG_EXT_AUDIOSCENE_INFO", ID_CONFIG_EXT_AUDIOSCENE_INFO as i64),
    ("ID_CONFIG_EXT_HOA_MATRIX", ID_CONFIG_EXT_HOA_MATRIX as i64),
    ("ID_CONFIG_EXT_SIG_GROUP_INFO", ID_CONFIG_EXT_SIG_GROUP_INFO as i64),
    ("EXT_FIL", EXT_FIL as i64),
    ("EXT_FILL_DATA", EXT_FILL_DATA as i64),
    ("EXT_DATA_ELEMENT", EXT_DATA_ELEMENT as i64),
    ("EXT_DATA_LENGTH", EXT_DATA_LENGTH as i64),
    ("EXT_UNI_DRC", EXT_UNI_DRC as i64),
    ("EXT_LDSAC_DATA", EXT_LDSAC_DATA as i64),
    ("EXT_SAOC_DATA", EXT_SAOC_DATA as i64),
    ("EXT_DYNAMIC_RANGE", EXT_DYNAMIC_RANGE as i64),
    ("EXT_SAC_DATA", EXT_SAC_DATA as i64),
    ("EXT_SBR_DATA", EXT_SBR_DATA as i64),
    ("EXT_SBR_DATA_CRC", EXT_SBR_DATA_CRC as i64),
    ("ID_EXT_ELE_FILL", ID_EXT_ELE_FILL as i64),
    ("ID_EXT_ELE_MPEGS", ID_EXT_ELE_MPEGS as i64),
    ("ID_EXT_ELE_SAOC", ID_EXT_ELE_SAOC as i64),
    ("ID_EXT_ELE_AUDIOPREROLL", ID_EXT_ELE_AUDIOPREROLL as i64),
    ("ID_EXT_ELE_UNI_DRC", ID_EXT_ELE_UNI_DRC as i64),
    ("ID_EXT_ELE_OBJ_METADATA", ID_EXT_ELE_OBJ_METADATA as i64),
    ("ID_EXT_ELE_SAOC_3D", ID_EXT_ELE_SAOC_3D as i64),
    ("ID_EXT_ELE_HOA", ID_EXT_ELE_HOA as i64),
    ("ID_EXT_ELE_FMT_CNVRTR", ID_EXT_ELE_FMT_CNVRTR as i64),
    ("ID_EXT_ELE_MCT", ID_EXT_ELE_MCT as i64),
    ("ID_EXT_ELE_ENHANCED_OBJ_METADATA", ID_EXT_ELE_ENHANCED_OBJ_METADATA as i64),
    ("ID_EXT_ELE_VR_METADATA", ID_EXT_ELE_VR_METADATA as i64),
    ("ID_EXT_ELE_UNKNOWN", ID_EXT_ELE_UNKNOWN as i64),
    ("TC_NOTHING", TC_NOTHING as i64),
    ("TC_RAW_ADTS", TC_RAW_ADTS as i64),
    ("TC_RAW_LATM_MCP1", TC_RAW_LATM_MCP1 as i64),
    ("TC_RAW_SDC", TC_RAW_SDC as i64),
    ("FDK_NONE", FDK_NONE as i64),
    ("FDK_TOOLS", FDK_TOOLS as i64),
    ("FDK_SYSLIB", FDK_SYSLIB as i64),
    ("FDK_AACDEC", FDK_AACDEC as i64),
    ("FDK_AACENC", FDK_AACENC as i64),
    ("FDK_SBRDEC", FDK_SBRDEC as i64),
    ("FDK_SBRENC", FDK_SBRENC as i64),
    ("FDK_TPDEC", FDK_TPDEC as i64),
    ("FDK_TPENC", FDK_TPENC as i64),
    ("FDK_MPSDEC", FDK_MPSDEC as i64),
    ("FDK_MPEGFILEREAD", FDK_MPEGFILEREAD as i64),
    ("FDK_MPEGFILEWRITE", FDK_MPEGFILEWRITE as i64),
    ("FDK_PCMDMX", FDK_PCMDMX as i64),
    ("FDK_MPSENC", FDK_MPSENC as i64),
    ("FDK_TDLIMIT", FDK_TDLIMIT as i64),
    ("FDK_UNIDRCDEC", FDK_UNIDRCDEC as i64),
    ("FDK_MODULE_LAST", FDK_MODULE_LAST as i64),
    ("AC_ER_VCB11", AC_ER_VCB11 as i64),
    ("AC_ER_RVLC", AC_ER_RVLC as i64),
    ("AC_ER_HCR", AC_ER_HCR as i64),
    ("AC_SCALABLE", AC_SCALABLE as i64),
    ("AC_ELD", AC_ELD as i64),
    ("AC_LD", AC_LD as i64),
    ("AC_ER", AC_ER as i64),
    ("AC_BSAC", AC_BSAC as i64),
    ("AC_USAC", AC_USAC as i64),
    ("AC_RSV603DA", AC_RSV603DA as i64),
    ("AC_HDAAC", AC_HDAAC as i64),
    ("AC_RSVD50", AC_RSVD50 as i64),
    ("AC_SBR_PRESENT", AC_SBR_PRESENT as i64),
    ("AC_SBRCRC", AC_SBRCRC as i64),
    ("AC_PS_PRESENT", AC_PS_PRESENT as i64),
    ("AC_MPS_PRESENT", AC_MPS_PRESENT as i64),
    ("AC_DRM", AC_DRM as i64),
    ("AC_INDEP", AC_INDEP as i64),
    ("AC_MPEGD_RES", AC_MPEGD_RES as i64),
    ("AC_SAOC_PRESENT", AC_SAOC_PRESENT as i64),
    ("AC_DAB", AC_DAB as i64),
    ("AC_ELD_DOWNSCALE", AC_ELD_DOWNSCALE as i64),
    ("AC_LD_MPS", AC_LD_MPS as i64),
    ("AC_DRC_PRESENT", AC_DRC_PRESENT as i64),
    ("AC_USAC_SCFGI3", AC_USAC_SCFGI3 as i64),
    ("AC_EL_USAC_TW", AC_EL_USAC_TW as i64),
    ("AC_EL_USAC_NOISE", AC_EL_USAC_NOISE as i64),
    ("AC_EL_USAC_ITES", AC_EL_USAC_ITES as i64),
    ("AC_EL_USAC_PVC", AC_EL_USAC_PVC as i64),
    ("AC_EL_USAC_MPS212", AC_EL_USAC_MPS212 as i64),
    ("AC_EL_USAC_LFE", AC_EL_USAC_LFE as i64),
    ("AC_EL_USAC_CP_POSSIBLE", AC_EL_USAC_CP_POSSIBLE as i64),
    ("AC_EL_ENHANCED_NOISE", AC_EL_ENHANCED_NOISE as i64),
    ("AC_EL_IGF_AFTER_TNS", AC_EL_IGF_AFTER_TNS as i64),
    ("AC_EL_IGF_INDEP_TILING", AC_EL_IGF_INDEP_TILING as i64),
    ("AC_EL_IGF_USE_ENF", AC_EL_IGF_USE_ENF as i64),
    ("AC_EL_FULLBANDLPD", AC_EL_FULLBANDLPD as i64),
    ("AC_EL_LPDSTEREOIDX", AC_EL_LPDSTEREOIDX as i64),
    ("AC_EL_LFE", AC_EL_LFE as i64),
    ("AC_CM_DET_CFG_CHANGE", AC_CM_DET_CFG_CHANGE as i64),
    ("AC_CM_ALLOC_MEM", AC_CM_ALLOC_MEM as i64),
    ("CC_MPEG_ID", CC_MPEG_ID as i64),
    ("CC_IS_BASELAYER", CC_IS_BASELAYER as i64),
    ("CC_PROTECTION", CC_PROTECTION as i64),
    ("CC_SBR", CC_SBR as i64),
    ("CC_SBRCRC", CC_SBRCRC as i64),
    ("CC_SAC", CC_SAC as i64),
    ("CC_RVLC", CC_RVLC as i64),
    ("CC_VCB11", CC_VCB11 as i64),
    ("CC_HCR", CC_HCR as i64),
    ("CC_PSEUDO_SURROUND", CC_PSEUDO_SURROUND as i64),
    ("CC_USAC_NOISE", CC_USAC_NOISE as i64),
    ("CC_USAC_TW", CC_USAC_TW as i64),
    ("CC_USAC_HBE", CC_USAC_HBE as i64),
    ("CAPF_AAC_LC", CAPF_AAC_LC as i64),
    ("CAPF_ER_AAC_LD", CAPF_ER_AAC_LD as i64),
    ("CAPF_ER_AAC_SCAL", CAPF_ER_AAC_SCAL as i64),
    ("CAPF_ER_AAC_LC", CAPF_ER_AAC_LC as i64),
    ("CAPF_AAC_480", CAPF_AAC_480 as i64),
    ("CAPF_AAC_512", CAPF_AAC_512 as i64),
    ("CAPF_AAC_960", CAPF_AAC_960 as i64),
    ("CAPF_AAC_1024", CAPF_AAC_1024 as i64),
    ("CAPF_AAC_HCR", CAPF_AAC_HCR as i64),
    ("CAPF_AAC_VCB11", CAPF_AAC_VCB11 as i64),
    ("CAPF_AAC_RVLC", CAPF_AAC_RVLC as i64),
    ("CAPF_AAC_MPEG4", CAPF_AAC_MPEG4 as i64),
    ("CAPF_AAC_DRC", CAPF_AAC_DRC as i64),
    ("CAPF_AAC_CONCEALMENT", CAPF_AAC_CONCEALMENT as i64),
    ("CAPF_AAC_DRM_BSFORMAT", CAPF_AAC_DRM_BSFORMAT as i64),
    ("CAPF_ER_AAC_ELD", CAPF_ER_AAC_ELD as i64),
    ("CAPF_ER_AAC_BSAC", CAPF_ER_AAC_BSAC as i64),
    ("CAPF_AAC_ELD_DOWNSCALE", CAPF_AAC_ELD_DOWNSCALE as i64),
    ("CAPF_AAC_USAC_LP", CAPF_AAC_USAC_LP as i64),
    ("CAPF_AAC_USAC", CAPF_AAC_USAC as i64),
    ("CAPF_ER_AAC_ELDV2", CAPF_ER_AAC_ELDV2 as i64),
    ("CAPF_AAC_UNIDRC", CAPF_AAC_UNIDRC as i64),
    ("CAPF_ADTS", CAPF_ADTS as i64),
    ("CAPF_ADIF", CAPF_ADIF as i64),
    ("CAPF_LATM", CAPF_LATM as i64),
    ("CAPF_LOAS", CAPF_LOAS as i64),
    ("CAPF_RAWPACKETS", CAPF_RAWPACKETS as i64),
    ("CAPF_DRM", CAPF_DRM as i64),
    ("CAPF_RSVD50", CAPF_RSVD50 as i64),
    ("CAPF_SBR_LP", CAPF_SBR_LP as i64),
    ("CAPF_SBR_HQ", CAPF_SBR_HQ as i64),
    ("CAPF_SBR_DRM_BS", CAPF_SBR_DRM_BS as i64),
    ("CAPF_SBR_CONCEALMENT", CAPF_SBR_CONCEALMENT as i64),
    ("CAPF_SBR_DRC", CAPF_SBR_DRC as i64),
    ("CAPF_SBR_PS_MPEG", CAPF_SBR_PS_MPEG as i64),
    ("CAPF_SBR_PS_DRM", CAPF_SBR_PS_DRM as i64),
    ("CAPF_SBR_ELD_DOWNSCALE", CAPF_SBR_ELD_DOWNSCALE as i64),
    ("CAPF_SBR_HBEHQ", CAPF_SBR_HBEHQ as i64),
    ("CAPF_DMX_BLIND", CAPF_DMX_BLIND as i64),
    ("CAPF_DMX_PCE", CAPF_DMX_PCE as i64),
    ("CAPF_DMX_ARIB", CAPF_DMX_ARIB as i64),
    ("CAPF_DMX_DVB", CAPF_DMX_DVB as i64),
    ("CAPF_DMX_CH_EXP", CAPF_DMX_CH_EXP as i64),
    ("CAPF_DMX_6_CH", CAPF_DMX_6_CH as i64),
    ("CAPF_DMX_8_CH", CAPF_DMX_8_CH as i64),
    ("CAPF_DMX_24_CH", CAPF_DMX_24_CH as i64),
    ("CAPF_LIMITER", CAPF_LIMITER as i64),
    ("CAPF_MPS_STD", CAPF_MPS_STD as i64),
    ("CAPF_MPS_LD", CAPF_MPS_LD as i64),
    ("CAPF_MPS_USAC", CAPF_MPS_USAC as i64),
    ("CAPF_MPS_HQ", CAPF_MPS_HQ as i64),
    ("CAPF_MPS_LP", CAPF_MPS_LP as i64),
    ("CAPF_MPS_BLIND", CAPF_MPS_BLIND as i64),
    ("CAPF_MPS_BINAURAL", CAPF_MPS_BINAURAL as i64),
    ("CAPF_MPS_2CH_OUT", CAPF_MPS_2CH_OUT as i64),
    ("CAPF_MPS_6CH_OUT", CAPF_MPS_6CH_OUT as i64),
    ("CAPF_MPS_8CH_OUT", CAPF_MPS_8CH_OUT as i64),
    ("CAPF_MPS_1CH_IN", CAPF_MPS_1CH_IN as i64),
    ("CAPF_MPS_2CH_IN", CAPF_MPS_2CH_IN as i64),
    ("CAPF_MPS_6CH_IN", CAPF_MPS_6CH_IN as i64),
    ("FDK_BUF_TYPE_MASK_IO", FDK_BUF_TYPE_MASK_IO as i64),
    ("FDK_BUF_TYPE_MASK_DESCR", FDK_BUF_TYPE_MASK_DESCR as i64),
    ("FDK_BUF_TYPE_MASK_ID", FDK_BUF_TYPE_MASK_ID as i64),
    ("FDK_BUF_TYPE_INPUT", FDK_BUF_TYPE_INPUT as i64),
    ("FDK_BUF_TYPE_OUTPUT", FDK_BUF_TYPE_OUTPUT as i64),
    ("FDK_BUF_TYPE_PCM_DATA", FDK_BUF_TYPE_PCM_DATA as i64),
    ("FDK_BUF_TYPE_ANC_DATA", FDK_BUF_TYPE_ANC_DATA as i64),
    ("FDK_BUF_TYPE_BS_DATA", FDK_BUF_TYPE_BS_DATA as i64),
    ("USAC_ID_BIT", USAC_ID_BIT as i64),
    ("EXT_ID_BITS", EXT_ID_BITS as i64),
];

#[test]
fn constants() {
    assert_eq!(CONSTANTS.len() as i32, unsafe { cref_audio_const_count() });
    for (i, &(name, v)) in CONSTANTS.iter().enumerate() {
        assert_eq!(v, unsafe { cref_audio_const(i as i32) }, "{name}");
    }
    assert_eq!(64, unsafe { cref_sizeof_CODER_CONFIG_rawConfig() });
    assert_eq!(CoderConfig::default().raw_config.len(), 64);
}

#[test]
fn macros() {
    for x in -300..300 {
        assert_eq!(tt_is_packet(x) as i32, unsafe { cref_TT_IS_PACKET(x) }, "TT_IS_PACKET({x})");
        assert_eq!(can_do_ps(x) as i32, unsafe { cref_CAN_DO_PS(x) }, "CAN_DO_PS({x})");
        assert_eq!(is_usac(x) as i32, unsafe { cref_IS_USAC(x) }, "IS_USAC({x})");
        assert_eq!(is_lowdelay(x) as i32, unsafe { cref_IS_LOWDELAY(x) }, "IS_LOWDELAY({x})");
        assert_eq!(is_channel_element(x) as i32, unsafe { cref_IS_CHANNEL_ELEMENT(x) });
        assert_eq!(is_mp4_channel_element(x) as i32, unsafe { cref_IS_MP4_CHANNEL_ELEMENT(x) });
        assert_eq!(is_usac_channel_element(x) as i32, unsafe { cref_IS_USAC_CHANNEL_ELEMENT(x) });
    }
}

fn version_string_of(v: i32) -> (Vec<u8>, Vec<u8>) {
    let mut info = LibInfo { version: v, ..Default::default() };
    lib_version_string(&mut info);
    let mut s = [0i8; 32];
    unsafe { cref_LIB_VERSION_STRING(v, s.as_mut_ptr()) };
    let s: Vec<u8> = s.iter().map(|&c| c as u8).collect();
    let n = s.iter().position(|&c| c == 0).unwrap();
    (info.version_str[..=n].to_vec(), s[..=n].to_vec())
}

#[test]
fn LIB_VERSION_LIB_VERSION_STRING() {
    let mut r = Rng::new(1101);
    for i in 0..N {
        let (l0, l1, l2) = if i < 1000 {
            ((i % 10) as i32, (i / 10 % 10) as i32, (i / 100) as i32)
        } else {
            (r.range(0, 255), r.range(0, 255), r.range(0, 255))
        };
        let v = lib_version(l0, l1, l2);
        assert_eq!(v, unsafe { cref_LIB_VERSION(l0, l1, l2) }, "LIB_VERSION({l0}, {l1}, {l2})");
        let (a, b) = version_string_of(v);
        assert_eq!(a, b, "{l0}.{l1}.{l2}");
    }
    // Arbitrary versions (the levels are masked).
    each_d(1102, N, |v| {
        let (a, b) = version_string_of(v);
        assert_eq!(a, b, "{v:#x}");
    });
}

#[test]
fn FDKlibInfo() {
    let mut r = Rng::new(1103);
    let modules =
        [FDK_NONE, FDK_TOOLS, FDK_SYSLIB, FDK_AACDEC, FDK_SBRDEC, FDK_TPDEC, FDK_PCMDMX, FDK_UNIDRCDEC];
    let last = FDK_MODULE_LAST as usize;
    for _ in 0..20000 {
        // A table filled from the start, with random free entries.
        let used = r.range(0, last as i32) as usize;
        let ids: Vec<i32> = (0..last)
            .map(|i| {
                if i < used || r.next_u32() % 8 == 0 {
                    modules[(r.next_u32() % 8) as usize]
                } else {
                    FDK_NONE
                }
            })
            .collect();
        let flags: Vec<u32> = (0..last).map(|_| r.next_u32()).collect();
        let module_id = modules[(r.next_u32() % 8) as usize];
        let (mut caps, mut lookup) = (0u32, 0i32);
        let mut after = vec![0i32; last];
        unsafe {
            cref_libinfo(
                ids.as_ptr(),
                flags.as_ptr(),
                module_id,
                &mut caps,
                &mut lookup,
                after.as_mut_ptr(),
            )
        };
        let mut info: Vec<LibInfo> = (0..last)
            .map(|i| LibInfo { module_id: ids[i], flags: flags[i], ..Default::default() })
            .collect();
        assert_eq!(fdk_lib_info_get_capabilities(&info, module_id), caps);
        assert_eq!(fdk_lib_info_lookup(&info, module_id), lookup, "{ids:?} {module_id}");
        fdk_init_lib_info(&mut info);
        assert_eq!(info.iter().map(|e| e.module_id).collect::<Vec<_>>(), after);
    }
}

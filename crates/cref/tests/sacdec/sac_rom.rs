// Differential tests of fdk_aac_rs_sacdec::sac_rom against the C++ reference: every table of
// sac_rom.cpp and the file-static tables of sac_calcM1andM2.cpp and sac_tsd.cpp, the macro
// constants and the dequantization helpers.

use fdk_aac_rs_cref::sacdec_sac_rom as c;
use fdk_aac_rs_sacdec::sac_calc_m1_and_m2 as calc;
use fdk_aac_rs_sacdec::sac_rom::*;
use fdk_aac_rs_sacdec::sac_stp as stp;
use fdk_aac_rs_sacdec::sac_tsd as tsd;

fn flat<T: Copy + Into<i64>>(v: &[T]) -> Vec<i32> {
    v.iter().map(|&x| x.into() as i32).collect()
}

fn rust_tables() -> Vec<(&'static str, Vec<i32>)> {
    let mut t: Vec<(&'static str, Vec<i32>)> = vec![
        ("dequantCPC__FDK", flat(&DEQUANT_CPC__FDK)),
        ("dequantICC__FDK", flat(&DEQUANT_ICC__FDK)),
        ("dequantCLD__FDK", flat(&DEQUANT_CLD__FDK)),
        ("dequantIPD__FDK", flat(&DEQUANT_IPD__FDK)),
        (
            "dequantIPD_CLD_ICC_splitAngle__FDK",
            DEQUANT_IPD_CLD_ICC_SPLIT_ANGLE__FDK.iter().flatten().flatten().copied().collect(),
        ),
        ("dequantCLD_c_l", flat(&DEQUANT_CLD_C_L)),
        ("H11_nc", H11_NC.iter().flatten().copied().collect()),
        ("H12_nc", H12_NC.iter().flatten().copied().collect()),
        ("dequantCLD_c1", flat(&DEQUANT_CLD_C1)),
        ("BP__FDK", flat(&BP__FDK)),
        ("BP_GF__FDK", flat(&BP_GF__FDK)),
        ("samplingFreqTable", flat(&SAMPLING_FREQ_TABLE)),
        ("freqResTable", flat(&FREQ_RES_TABLE)),
        ("freqResTable_LD", flat(&FREQ_RES_TABLE_LD)),
        ("tempShapeChanTable", TEMP_SHAPE_CHAN_TABLE.iter().flatten().map(|&v| v as i32).collect()),
        ("kernels_4_to_71", flat(&KERNELS_4_TO_71)),
        ("kernels_5_to_71", flat(&KERNELS_5_TO_71)),
        ("kernels_7_to_71", flat(&KERNELS_7_TO_71)),
        ("kernels_10_to_71", flat(&KERNELS_10_TO_71)),
        ("kernels_14_to_71", flat(&KERNELS_14_TO_71)),
        ("kernels_20_to_71", flat(&KERNELS_20_TO_71)),
        ("kernels_28_to_71", flat(&KERNELS_28_TO_71)),
        ("kernels_4_to_64", flat(&KERNELS_4_TO_64)),
        ("kernels_5_to_64", flat(&KERNELS_5_TO_64)),
        ("kernels_7_to_64", flat(&KERNELS_7_TO_64)),
        ("kernels_9_to_64", flat(&KERNELS_9_TO_64)),
        ("kernels_12_to_64", flat(&KERNELS_12_TO_64)),
        ("kernels_15_to_64", flat(&KERNELS_15_TO_64)),
        ("kernels_23_to_64", flat(&KERNELS_23_TO_64)),
        ("mapping_15_to_23", flat(&MAPPING_15_TO_23)),
        ("mapping_12_to_23", flat(&MAPPING_12_TO_23)),
        ("mapping_9_to_23", flat(&MAPPING_9_TO_23)),
        ("mapping_7_to_23", flat(&MAPPING_7_TO_23)),
        ("mapping_5_to_23", flat(&MAPPING_5_TO_23)),
        ("mapping_4_to_23", flat(&MAPPING_4_TO_23)),
        ("clipGainTable__FDK", flat(&CLIP_GAIN_TABLE__FDK)),
        ("clipGainSFTable__FDK", flat(&CLIP_GAIN_SF_TABLE__FDK)),
        ("pbStrideTable", flat(&PB_STRIDE_TABLE)),
        ("smgTimeTable", flat(&SMG_TIME_TABLE)),
        ("envShapeDataTable__FDK", ENV_SHAPE_DATA_TABLE__FDK.iter().flatten().copied().collect()),
        ("row2channelSTP", ROW2CHANNEL_STP.iter().flatten().map(|&v| v as i32).collect()),
        ("row2channelGES", ROW2CHANNEL_GES.iter().flatten().map(|&v| v as i32).collect()),
        ("row2residual", ROW2RESIDUAL.iter().flatten().map(|&v| v as i32).collect()),
    ];
    let mut tp = Vec::new();
    for e in &TREE_PROPERTY_TABLE {
        tp.extend([
            e.num_input_channels as i32,
            e.num_output_channels as i32,
            e.num_ott_boxes as i32,
            e.num_ttt_boxes as i32,
        ]);
        tp.extend(e.ott_mode_lfe.iter().map(|&v| v as i32));
    }
    t.push(("treePropertyTable", tp));
    t.push(("sinIpd_tab", flat(&calc::SIN_IPD_TAB)));
    t.push(("sqrt_one_minus_ICC2", flat(&calc::SQRT_ONE_MINUS_ICC2)));
    t.push(("sqrt_CLD_e", flat(&calc::SQRT_CLD_E)));
    t.push(("sqrt_CLD_m", flat(&calc::SQRT_CLD_M)));
    t.push(("CLD_m", flat(&calc::CLD_M)));
    t.push(("nBitsTsdCW_32slots", flat(&tsd::N_BITS_TSD_CW_32SLOTS)));
    t.push(("nBitsTsdCW_64slots", flat(&tsd::N_BITS_TSD_CW_64SLOTS)));
    t.push(("phiTsd", tsd::PHI_TSD.iter().flatten().copied().collect()));
    t
}

#[test]
fn sac_rom_tables() {
    let tables = rust_tables();
    let mut out = vec![0i32; 4096];
    for (id, (name, rs)) in tables.iter().enumerate() {
        let n = unsafe { c::cref_sacrom_table(id as i32, out.as_mut_ptr()) };
        assert!(n >= 0, "{name}: no C table");
        assert_eq!(&out[..n as usize], &rs[..], "{name}");
    }
    assert_eq!(unsafe { c::cref_sacrom_table(tables.len() as i32, out.as_mut_ptr()) }, -1);
}

#[test]
fn sac_rom_constants() {
    let rs = [
        PI__IPD,
        PIX2__IPD,
        stp::SF_SCALE_LD64,
        stp::STP_LPF_COEFF1__FDK,
        stp::ONE_MINUS_STP_LPF_COEFF1__FDK,
        stp::STP_LPF_COEFF2__FDK,
        stp::ONE_MINUS_STP_LPF_COEFF2__FDK,
        stp::STP_SCALE_LIMIT__FDK,
        stp::ONE_DIV_STP_SCALE_LIMIT__FDK,
        stp::ABS_THR__FDK,
        stp::ABS_THR2__FDK,
        stp::STP_SCALE_LIMIT_HI,
        stp::STP_SCALE_LIMIT_LO,
        stp::STP_SCALE_LIMIT_HI_LD64,
        stp::STP_SCALE_LIMIT_LO_LD64,
    ];
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_sacrom_consts(out.as_mut_ptr()) } as usize;
    assert_eq!(&out[..n], &rs[..]);
}

#[test]
fn sac_rom_dequant_cld() {
    // The library passes the indices 0..=30.
    for i in 0..31 {
        let mut o = [0i32; 2];
        unsafe { c::cref_sacrom_get_cld_values(i, o.as_mut_ptr()) };
        assert_eq!(spatial_dequant_get_cld_values(i), (o[0], o[1]), "SpatialDequantGetCLDValues({i})");
        assert_eq!(
            spatial_dequant_get_cld2_values(i),
            unsafe { c::cref_sacrom_get_cld2_values(i) },
            "SpatialDequantGetCLD2Values({i})"
        );
    }
    for q in [16, 24, 32, 64] {
        assert_eq!(sac_get_hybrid_subbands(q), q - 3 + 10);
    }
}

/// The function-local table `chanelIdx` of `mapChannel()` (sac_process.cpp), element by
/// element through the function of a copy of the file.
#[test]
fn sac_rom_function_local_chanel_idx() {
    use fdk_aac_rs_sacdec::sac_process::CHANEL_IDX;
    assert_eq!(CHANEL_IDX.len(), 1);
    for (ch, &v) in CHANEL_IDX[0].iter().enumerate() {
        assert_eq!(v as i32, unsafe { c::cref_sacrom_map_channel(ch as u32) }, "chanelIdx[0][{ch}]");
    }
}

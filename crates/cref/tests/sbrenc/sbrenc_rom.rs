// Differential tests of fdk_aac_rs_sbrenc::sbrenc_rom against the C++ reference.
//
// Every Huffman table and every row of the SBR and PS tuning tables is compared with the
// tables of the reference library.

use fdk_aac_rs_cref::sbrenc_sbrenc_rom as c;
use fdk_aac_rs_sbrenc::sbrenc_rom::*;

#[test]
fn huffman_tables() {
    let tabs: [&[i32]; 20] = [
        &V_HUFF_ENVELOPE_LEVEL_C_10T,
        &[],
        &V_HUFF_ENVELOPE_LEVEL_C_10F,
        &[],
        &BOOK_SBR_ENV_BALANCE_C_10T,
        &[],
        &BOOK_SBR_ENV_BALANCE_C_10F,
        &[],
        &V_HUFF_ENVELOPE_LEVEL_C_11T,
        &[],
        &V_HUFF_ENVELOPE_LEVEL_C_11F,
        &[],
        &BOOK_SBR_ENV_BALANCE_C_11T,
        &[],
        &BOOK_SBR_ENV_BALANCE_C_11F,
        &[],
        &V_HUFF_NOISE_LEVEL_C_11T,
        &[],
        &BOOK_SBR_NOISE_BALANCE_C_11T,
        &[],
    ];
    let lens: [&[u8]; 20] = [
        &[],
        &V_HUFF_ENVELOPE_LEVEL_L_10T,
        &[],
        &V_HUFF_ENVELOPE_LEVEL_L_10F,
        &[],
        &BOOK_SBR_ENV_BALANCE_L_10T,
        &[],
        &BOOK_SBR_ENV_BALANCE_L_10F,
        &[],
        &V_HUFF_ENVELOPE_LEVEL_L_11T,
        &[],
        &V_HUFF_ENVELOPE_LEVEL_L_11F,
        &[],
        &BOOK_SBR_ENV_BALANCE_L_11T,
        &[],
        &BOOK_SBR_ENV_BALANCE_L_11F,
        &[],
        &V_HUFF_NOISE_LEVEL_L_11T,
        &[],
        &BOOK_SBR_NOISE_BALANCE_L_11T,
    ];
    let mut out = vec![0i32; 256];
    for id in 0..20 {
        let n = unsafe { c::cref_sbrenc_rom_huff(id as i32, out.as_mut_ptr()) };
        let v: Vec<i32> = if id % 2 == 0 { tabs[id].to_vec() } else { lens[id].iter().map(|&x| x as i32).collect() };
        assert_eq!(&out[..n as usize], &v[..], "table {id}");
    }
    assert_eq!(unsafe { c::cref_sbrenc_rom_huff(20, out.as_mut_ptr()) }, -1);
}

#[test]
fn tuning_tables() {
    let n = unsafe { c::cref_sbrenc_rom_tuning_size() };
    assert_eq!(n, SBR_TUNING_TABLE_SIZE);
    assert_eq!(n as usize, SBR_TUNING_TABLE.len());
    for (i, t) in SBR_TUNING_TABLE.iter().enumerate() {
        let mut v = [0i32; 14];
        unsafe { c::cref_sbrenc_rom_tuning_row(i as i32, v.as_mut_ptr()) };
        let r = [
            t.core_coder,
            t.bitrate_from as i32,
            t.bitrate_to as i32,
            t.sample_rate as i32,
            t.num_channels as i32,
            t.start_freq as i32,
            t.start_freq_speech as i32,
            t.stop_freq as i32,
            t.stop_freq_speech as i32,
            t.num_noise_bands as i32,
            t.noise_floor_offset as i32,
            t.noise_max_level as i32,
            t.stereo_mode,
            t.freq_scale as i32,
        ];
        assert_eq!(v, r, "sbrTuningTable[{i}]");
    }
    for (i, t) in PS_TUNING_TABLE.iter().enumerate() {
        let mut v = [0i32; 5];
        unsafe { c::cref_sbrenc_rom_ps_tuning_row(i as i32, v.as_mut_ptr()) };
        let r = [
            t.bitrate_from as i32,
            t.bitrate_to as i32,
            t.n_stereo_bands,
            t.n_envelopes,
            t.iid_quant_error_threshold,
        ];
        assert_eq!(v, r, "psTuningTable[{i}]");
    }
}

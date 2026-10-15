// Differential tests of fdk_aac_rs_pcmutils::pcmdmx_lib against the C++ reference.
//
// - the enum values, the tables and the default metadata;
// - the file-static helpers (getSpeakerPos, getSpeakerDistance, getChMode4Plain, getChannelMode,
//   getChannelDescription, dmxSetChannel/dmxAddChannel, getMixFactors) through a copy of
//   pcmdmx_lib.cpp, on exhaustive and random inputs;
// - the API (pcmDmx_Open/Close/Reset/SetParam/GetParam/SetMatrixMixdownFromPce/Parse/
//   ReadDvbAncData/ApplyFrame/GetLibInfo) of the library itself, on random call sequences,
//   comparing return values, outputs, the channel descriptions and the complete instance state.
//
// pcmDmx_ApplyFrame runs on every input channel configuration of the MPEG channel
// configurations, PCE-like combinations and random channel descriptions (in random channel
// orders), against every output channel count, dual channel mode, profile, pseudo surround mode,
// metadata delay and expiry setting, with random metadata, both channel maps and pass-through.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::pcmutils_pcmdmx_lib as c;
use fdk_aac_rs_cref::pcmutils_pcmdmx_lib_static as cs;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, FdkBitStream};
use fdk_aac_rs_fdk::syslib_channel_map_descr::{fdk_ch_map_descr_init, ChannelMapInfo, FdkChannelMapDescr, FDK_MAP_INFO_TAB_LEN_WG4, FDK_MAP_INFO_TAB_WG4};
use fdk_aac_rs_pcmutils::pcmdmx_lib::*;
use std::ffi::c_void;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn meta_vals(m: &DmxBsMetaData) -> [i32; 11] {
    [
        m.type_flags as i32,
        m.c_lev_idx as i32,
        m.s_lev_idx as i32,
        m.dmix_idx_a as i32,
        m.dmix_idx_b as i32,
        m.dmix_idx_lfe as i32,
        m.dmx_gain_idx2 as i32,
        m.dmx_gain_idx5 as i32,
        m.matrix_mixdown_idx as i32,
        m.pseudo_surround as i32,
        m.expiry_count as i32,
    ]
}

fn params_vals(p: &PcmDmxUserParams) -> [i32; 7] {
    [
        p.dmx_profile,
        p.expiry_frame as i32,
        p.dual_channel_mode,
        p.pseudo_surr_mode,
        p.num_out_channels_min as i32,
        p.num_out_channels_max as i32,
        p.frame_delay as i32,
    ]
}

fn state(d: &PcmDmxInstance) -> [i32; 30] {
    let mut s = [0i32; 30];
    s[..11].copy_from_slice(&meta_vals(&d.bs_meta_data[0]));
    s[11..22].copy_from_slice(&meta_vals(&d.bs_meta_data[1]));
    s[22..29].copy_from_slice(&params_vals(&d.user_params));
    s[29] = d.apply_processing as i32;
    s
}

/// Random metadata with every field in the range of its bitstream field.
fn random_meta(rng: &mut Rng) -> DmxBsMetaData {
    if rng.next_u32() % 5 == 0 {
        return DFLT_META_DATA;
    }
    DmxBsMetaData {
        type_flags: rng.next_u32() & 0x7F,
        c_lev_idx: (rng.next_u32() % 8) as u8,
        s_lev_idx: (rng.next_u32() % 8) as u8,
        dmix_idx_a: (rng.next_u32() % 8) as u8,
        dmix_idx_b: (rng.next_u32() % 8) as u8,
        dmix_idx_lfe: (rng.next_u32() % 16) as u8,
        dmx_gain_idx2: if rng.next_u32() % 2 == 0 { 0 } else { (rng.next_u32() % 128) as u8 },
        dmx_gain_idx5: if rng.next_u32() % 2 == 0 { 0 } else { (rng.next_u32() % 128) as u8 },
        matrix_mixdown_idx: (rng.next_u32() % 4) as u8,
        pseudo_surround: rng.range(-1, 1) as i8,
        expiry_count: rng.next_u32() % 8,
    }
}

fn random_params(rng: &mut Rng) -> PcmDmxUserParams {
    let outs = [-1i16, 1, 2, 6, 8];
    PcmDmxUserParams {
        dmx_profile: rng.range(0, 3),
        expiry_frame: rng.next_u32() % 4,
        dual_channel_mode: rng.range(0, 3),
        pseudo_surr_mode: rng.range(-1, 1),
        num_out_channels_min: outs[(rng.next_u32() % 5) as usize],
        num_out_channels_max: outs[(rng.next_u32() % 5) as usize],
        frame_delay: (rng.next_u32() % 2) as u8,
    }
}

/// A channel map descriptor on both sides: kind 0 = default table, 1 = WG4 table, 2 = a random
/// custom table (valid or not) with map values below 8.
struct MapDescr {
    kind: i32,
    pass: i32,
    maps: Vec<u8>,
    nch: Vec<u8>,
    rs: FdkChannelMapDescr,
}

impl MapDescr {
    fn new(rng: &mut Rng) -> MapDescr {
        let kind = match rng.next_u32() % 8 {
            0 => 1,
            1 => 2,
            _ => 0,
        };
        let pass = (rng.next_u32() % 6 == 0) as i32;
        let mut maps = vec![0u8; 15 * 24];
        let mut nch = vec![0u8; 15];
        let mut rs = FdkChannelMapDescr::default();
        match kind {
            0 => fdk_ch_map_descr_init(&mut rs, None, 0, pass as u32),
            1 => fdk_ch_map_descr_init(&mut rs, Some(&FDK_MAP_INFO_TAB_WG4), FDK_MAP_INFO_TAB_LEN_WG4, pass as u32),
            _ => {
                let valid = rng.next_u32() % 4 != 0;
                // Channels of the channel configurations 0 to 14. The maps of the
                // configurations the module uses for its output must stay within their channel
                // count (the reference has no defined result otherwise).
                const CFG_CH: [usize; 15] = [8, 1, 2, 3, 4, 5, 6, 8, 8, 8, 8, 7, 8, 8, 8];
                for i in 0..15 {
                    let n = 1 + (rng.next_u32() as usize) % CFG_CH[i];
                    nch[i] = n as u8;
                    let m = &mut maps[24 * i..24 * i + 24];
                    for (j, v) in m.iter_mut().enumerate().take(n) {
                        *v = j as u8;
                    }
                    for j in (1..n).rev() {
                        let k = (rng.next_u32() as usize) % (j + 1);
                        m.swap(j, k);
                    }
                    if !valid && rng.next_u32() % 3 == 0 {
                        m[(rng.next_u32() as usize) % n] = (rng.next_u32() % 8) as u8;
                    }
                }
                let tab: Vec<ChannelMapInfo> = (0..15)
                    .map(|i| ChannelMapInfo {
                        channel_map: Box::leak(maps[24 * i..24 * i + 24].to_vec().into_boxed_slice()),
                        num_channels: nch[i],
                    })
                    .collect();
                rs.map_info_tab = Some(Box::leak(tab.into_boxed_slice()));
                rs.map_info_tab_len = 15;
                rs.f_pass_through = pass as u32;
            }
        }
        MapDescr { kind, pass, maps, nch, rs }
    }
}

struct Dmx {
    rs: Option<HandlePcmDownmix>,
    c: *mut c_void,
}

impl Dmx {
    fn open() -> Dmx {
        let mut rs = None;
        let e = pcm_dmx_open(Some(&mut rs));
        let mut ce = 0;
        let c = unsafe { c::cref_pcmdmx_open(0, &mut ce) };
        assert_eq!(e, ce, "pcmDmx_Open");
        assert!(rs.is_some() && !c.is_null());
        let mut d = Dmx { rs, c };
        d.check("pcmDmx_Open");
        d
    }

    fn rs(&mut self) -> &mut PcmDmxInstance {
        self.rs.as_mut().unwrap()
    }

    fn check(&mut self, what: &str) {
        let mut s = [0i32; 30];
        unsafe { cs::cref_pcmdmx_static_state(self.c, s.as_mut_ptr()) };
        assert_eq!(state(self.rs()), s, "instance state after {what}");
    }

    fn set_meta(&mut self, m: [DmxBsMetaData; 2]) {
        self.rs().bs_meta_data = m;
        let mut v = [0i32; 22];
        v[..11].copy_from_slice(&meta_vals(&m[0]));
        v[11..].copy_from_slice(&meta_vals(&m[1]));
        unsafe { cs::cref_pcmdmx_static_set_meta(self.c, v.as_ptr()) };
        self.check("set_meta");
    }

    fn set_param(&mut self, param: i32, value: i32, what: &str) {
        let e = pcm_dmx_set_param(self.rs.as_deref_mut(), param, value);
        assert_eq!(e, unsafe { c::cref_pcmdmx_set_param(self.c, param, value) }, "pcmDmx_SetParam({param:#x}, {value}) {what}");
        self.check(&format!("pcmDmx_SetParam({param:#x}, {value}) {what}"));
    }
}

impl Drop for Dmx {
    fn drop(&mut self) {
        let e = pcm_dmx_close(Some(&mut self.rs));
        assert!(self.rs.is_none());
        assert_eq!(e, unsafe { c::cref_pcmdmx_close(self.c, 0) }, "pcmDmx_Close");
    }
}

// ---------------------------------------------------------------------------------------------
// Constants and tables
// ---------------------------------------------------------------------------------------------

#[test]
fn pcmdmx_consts() {
    let mut cc = [0i32; 50];
    unsafe { c::cref_pcmdmx_consts(cc.as_mut_ptr()) };
    let mut r = vec![
        PCMDMX_OK,
        PCMDMX_UNSUPPORTED,
        pcm_dmx_fatal_error_start,
        PCMDMX_OUT_OF_MEMORY,
        pcm_dmx_fatal_error_end,
        PCMDMX_INVALID_HANDLE,
        PCMDMX_INVALID_ARGUMENT,
        PCMDMX_INVALID_CH_CONFIG,
        PCMDMX_INVALID_MODE,
        PCMDMX_UNKNOWN_PARAM,
        PCMDMX_UNABLE_TO_SET_PARAM,
        PCMDMX_CORRUPT_ANC_DATA,
        PCMDMX_OUTPUT_BUFFER_TOO_SMALL,
        DMX_PROFILE_SETTING,
        DMX_BS_DATA_EXPIRY_FRAME,
        DMX_BS_DATA_DELAY,
        MIN_NUMBER_OF_OUTPUT_CHANNELS,
        MAX_NUMBER_OF_OUTPUT_CHANNELS,
        DMX_DUAL_CHANNEL_MODE,
        DMX_PSEUDO_SURROUND_MODE,
        DMX_PRFL_STANDARD,
        DMX_PRFL_MATRIX_MIX,
        DMX_PRFL_FORCE_MATRIX_MIX,
        DMX_PRFL_ARIB_JAPAN,
        NEVER_DO_PS_DMX,
        AUTO_PS_DMX,
        FORCE_PS_DMX,
        STEREO_MODE,
        CH1_MODE,
        CH2_MODE,
        MIXED_MODE,
        PCMDMX_RESET_PARAMS as i32,
        PCMDMX_RESET_BS_DATA as i32,
        PCMDMX_RESET_FULL as i32,
    ];
    for e in 0..16 {
        r.push(pcmdmx_is_fatal_error(e));
    }
    assert_eq!(r.as_slice(), &cc[..r.len()]);
}

#[test]
fn pcmdmx_tables() {
    let mut sgl = [0i16; 40];
    let mut spkr = [0i8; 24];
    let mut modes = [0i32; 9];
    let mut dflt = [0i32; 11];
    unsafe { cs::cref_pcmdmx_static_tables(sgl.as_mut_ptr(), spkr.as_mut_ptr(), modes.as_mut_ptr(), dflt.as_mut_ptr()) };
    assert_eq!(AB_MIX_LVL_VALUE_TAB.as_slice(), &sgl[0..8], "abMixLvlValueTab");
    assert_eq!(LFE_MIX_LVL_VALUE_TAB.as_slice(), &sgl[8..24], "lfeMixLvlValueTab");
    assert_eq!(MPEG_MIX_DOWN_IDX2_COEF.as_slice(), &sgl[24..28], "mpegMixDownIdx2Coef");
    assert_eq!(MPEG_MIX_DOWN_IDX2_PRE_FACT.concat().as_slice(), &sgl[28..40], "mpegMixDownIdx2PreFact");
    assert_eq!(SPKR_SLOT_POS, spkr, "spkrSlotPos");
    assert_eq!(OUT_CH_MODE_TABLE, modes, "outChModeTable");
    assert_eq!(meta_vals(&DFLT_META_DATA), dflt, "dfltMetaData");
}

// ---------------------------------------------------------------------------------------------
// File-static helpers
// ---------------------------------------------------------------------------------------------

/// Every channel type that the decoder uses.
const TYPES: [i32; 13] = [
    ACT_NONE,
    ACT_FRONT,
    ACT_SIDE,
    ACT_BACK,
    ACT_LFE,
    ACT_TOP,
    ACT_FRONT_TOP,
    ACT_SIDE_TOP,
    ACT_BACK_TOP,
    ACT_BOTTOM,
    ACT_FRONT_BOTTOM,
    ACT_SIDE_BOTTOM,
    ACT_BACK_BOTTOM,
];

#[test]
fn get_speaker_pos_() {
    let mut rng = Rng::new(701);
    let mut types: Vec<i32> = (0..0x40).collect();
    for _ in 0..64 {
        types.push(rng.i32());
    }
    for &t in &types {
        for idx in 0..=255u8 {
            for num in 0..16u8 {
                let r = get_speaker_pos(t, idx, num);
                let mut cc = [0i8; 3];
                unsafe { cs::cref_pcmdmx_static_get_speaker_pos(t, idx, num, cc.as_mut_ptr()) };
                assert_eq!([r.x, r.y, r.z], cc, "getSpeakerPos({t:#x}, {idx}, {num})");
            }
        }
    }
}

#[test]
fn get_speaker_distance_() {
    let mut rng = Rng::new(702);
    for _ in 0..100_000 {
        let a = [rng.next_u32() as i8, rng.next_u32() as i8, rng.next_u32() as i8];
        let b = [rng.next_u32() as i8, rng.next_u32() as i8, rng.next_u32() as i8];
        let r = get_speaker_distance(
            PcmDmxSpeakerPosition { x: a[0], y: a[1], z: a[2] },
            PcmDmxSpeakerPosition { x: b[0], y: b[1], z: b[2] },
        );
        assert_eq!(r, unsafe { cs::cref_pcmdmx_static_get_speaker_distance(a.as_ptr(), b.as_ptr()) }, "{a:?} {b:?}");
    }
}

/// Every channel mode constant of pcmdmx_lib.cpp.
const MODES: [i32; 36] = [
    CH_MODE_UNDEFINED,
    CH_MODE_1_0_0_0,
    CH_MODE_2_0_0_0,
    CH_MODE_3_0_0_0,
    CH_MODE_2_0_1_0,
    CH_MODE_2_0_0_1,
    CH_MODE_3_0_1_0,
    CH_MODE_2_0_2_0,
    CH_MODE_2_0_1_1,
    CH_MODE_4_0_0_0,
    CH_MODE_3_0_2_0,
    CH_MODE_2_0_2_1,
    CH_MODE_3_0_1_1,
    CH_MODE_3_2_0_0,
    CH_MODE_5_0_0_0,
    CH_MODE_3_0_2_1,
    CH_MODE_3_2_0_1,
    CH_MODE_3_2_1_0,
    CH_MODE_5_0_1_0,
    CH_MODE_6_0_0_0,
    CH_MODE_2_2_2_1,
    CH_MODE_3_0_3_1,
    CH_MODE_3_2_1_1,
    CH_MODE_3_2_2_0,
    CH_MODE_3_0_2_2,
    CH_MODE_5_0_2_0,
    CH_MODE_5_0_1_1,
    CH_MODE_7_0_0_0,
    CH_MODE_3_2_2_1,
    CH_MODE_3_0_4_1,
    CH_MODE_5_0_2_1,
    CH_MODE_5_2_1_0,
    CH_MODE_3_2_1_2,
    CH_MODE_2_2_2_2,
    CH_MODE_3_0_3_2,
    CH_MODE_8_0_0_0,
];

#[test]
fn get_ch_mode4_plain_() {
    let mut rng = Rng::new(703);
    for &m in MODES.iter() {
        for plain in -1..4 {
            for cfg in [0, 1, 6, 7, 11, 12, 14, rng.i32()] {
                let r = get_ch_mode4_plain(plain, m, cfg);
                assert_eq!(r, unsafe { cs::cref_pcmdmx_static_get_ch_mode4_plain(plain, m, cfg) });
            }
        }
    }
}

/// The (type, index) pairs of the MPEG channel configurations and of random PCE-like
/// configurations with consecutive indices per type, or random channel descriptions.
fn random_channels(rng: &mut Rng) -> Vec<(i32, u8)> {
    let f = ACT_FRONT;
    let b = ACT_BACK;
    let l = ACT_LFE;
    let ft = ACT_FRONT_TOP;
    let mut v: Vec<(i32, u8)> = match rng.next_u32() % 4 {
        0 | 1 => match rng.next_u32() % 10 {
            0 => vec![(f, 0)],
            1 => vec![(f, 0), (f, 1)],
            2 => vec![(f, 0), (f, 1), (f, 2)],
            3 => vec![(f, 0), (f, 1), (f, 2), (b, 0)],
            4 => vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1)],
            5 => vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (l, 0)],
            6 => vec![(f, 0), (f, 1), (f, 2), (f, 3), (f, 4), (b, 0), (b, 1), (l, 0)],
            7 => vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (b, 2), (l, 0)],
            8 => vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (b, 2), (b, 3), (l, 0)],
            _ => vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (l, 0), (ft, 0), (ft, 1)],
        },
        2 => {
            // PCE-like: random channel counts per type, consecutive indices.
            let n = 1 + rng.next_u32() as usize % 8;
            let mut counts = [0u8; 13];
            let mut v = Vec::new();
            for _ in 0..n {
                let t = match rng.next_u32() % 8 {
                    0..=2 => 1,
                    3 => 2,
                    4 | 5 => 3,
                    6 => 4,
                    _ => (rng.next_u32() % 13) as usize,
                };
                v.push((TYPES[t], counts[t]));
                counts[t] += 1;
            }
            v
        }
        _ => {
            // Random channel descriptions.
            let n = 1 + rng.next_u32() as usize % 8;
            (0..n)
                .map(|_| {
                    let t = TYPES[(rng.next_u32() % 13) as usize];
                    let i = match rng.next_u32() % 3 {
                        0 => rng.next_u32() as u8,
                        _ => (rng.next_u32() % 5) as u8,
                    };
                    (t, i)
                })
                .collect()
        }
    };
    // random channel order (any output channel mapping)
    if rng.next_u32() % 2 == 0 {
        for j in (1..v.len()).rev() {
            let k = (rng.next_u32() as usize) % (j + 1);
            v.swap(j, k);
        }
    }
    v
}

fn channel_arrays(rng: &mut Rng, ch: &[(i32, u8)]) -> ([i32; 8], [u8; 8]) {
    let mut t = [0i32; 8];
    let mut i = [0u8; 8];
    for k in 0..8 {
        if let Some(&(ty, ix)) = ch.get(k) {
            t[k] = ty;
            i[k] = ix;
        } else {
            t[k] = TYPES[(rng.next_u32() % 13) as usize];
            i[k] = rng.next_u32() as u8;
        }
    }
    (t, i)
}

#[test]
fn get_channel_mode_() {
    let mut rng = Rng::new(704);
    for it in 0..100_000 {
        let ch = random_channels(&mut rng);
        let (t, mut idx) = channel_arrays(&mut rng, &ch);
        let mut cidx = idx;
        let mut off = [0u8; 8];
        let mut coff = [0x33u8; 8];
        let mut mode = 0;
        let mut cmode = 0;
        let n = ch.len() as u32;
        let e = get_channel_mode(n, &t, &mut idx, &mut off, &mut mode);
        let ce = unsafe { cs::cref_pcmdmx_static_get_channel_mode(n, t.as_ptr(), cidx.as_mut_ptr(), coff.as_mut_ptr(), &mut cmode) };
        let what = format!("iteration {it}: {ch:?}");
        assert_eq!(e, ce, "getChannelMode result {what}");
        assert_eq!(idx, cidx, "channelIndices {what}");
        assert_eq!(off, coff, "offsetTable {what}");
        assert_eq!(mode, cmode, "chMode {what}");
    }
}

#[test]
fn get_channel_description_() {
    let mut rng = Rng::new(705);
    let mut modes: Vec<i32> = OUT_CH_MODE_TABLE[1..].to_vec();
    modes.push(CH_MODE_5_0_2_1);
    for it in 0..20_000 {
        let m = modes[(rng.next_u32() as usize) % modes.len()];
        let md = MapDescr::new(&mut rng);
        let mut t = [0x55i32; 8];
        let mut idx = [0x55u8; 8];
        let mut off = [0x55u8; 8];
        let mut ct = t;
        let mut cidx = idx;
        let mut coff = off;
        get_channel_description(m, &md.rs, &mut t, &mut idx, &mut off);
        unsafe {
            cs::cref_pcmdmx_static_get_channel_description(
                m,
                md.kind,
                md.pass,
                md.maps.as_ptr(),
                md.nch.as_ptr(),
                15,
                ct.as_mut_ptr(),
                cidx.as_mut_ptr(),
                coff.as_mut_ptr(),
            )
        };
        let what = format!("iteration {it}: mode {m:#x}, map kind {}, pass {}", md.kind, md.pass);
        assert_eq!(t, ct, "channelType {what}");
        assert_eq!(idx, cidx, "channelIndices {what}");
        assert_eq!(off, coff, "offsetTable {what}");
    }
}

#[test]
fn dmx_set_add_channel() {
    let mut rng = Rng::new(706);
    for it in 0..100_000 {
        let mut f = [[0i16; 8]; 8];
        let mut s = [[0i32; 8]; 8];
        for o in 0..8 {
            for i in 0..8 {
                if rng.next_u32() % 3 != 0 {
                    f[o][i] = rng.i16();
                    s[o][i] = rng.range(0, 5);
                }
            }
        }
        let mut cf: Vec<i16> = f.concat();
        let mut csc: Vec<i32> = s.concat();
        let op = (rng.next_u32() % 2) as i32;
        let dst = (rng.next_u32() % 8) as usize;
        let src = (rng.next_u32() % 8) as usize;
        let factor = rng.i16();
        let scale = rng.range(0, 3);
        if op == 0 {
            dmx_set_channel(&mut f, &mut s, dst, src, factor, scale);
        } else {
            dmx_add_channel(&mut f, &mut s, dst, src, factor, scale);
        }
        unsafe { cs::cref_pcmdmx_static_dmx_channel_op(op, cf.as_mut_ptr(), csc.as_mut_ptr(), dst as u32, src as u32, factor, scale) };
        assert_eq!(f.concat(), cf, "factors, iteration {it} op {op}");
        assert_eq!(s.concat(), csc, "scales, iteration {it} op {op}");
    }
}

#[test]
fn get_mix_factors_() {
    let mut rng = Rng::new(707);
    let outs = [CH_MODE_1_0_0_0, CH_MODE_2_0_0_0, CH_MODE_3_0_2_1, CH_MODE_3_0_4_1, CH_MODE_5_0_2_1];
    let cfgs = [1, 2, 3, 4, 5, 6, 7, 11, 12, 14];
    for it in 0..200_000 {
        let (is_cfg, in_mode) =
            if rng.next_u32() % 4 == 0 { (1u8, cfgs[(rng.next_u32() % 10) as usize]) } else { (0u8, MODES[1 + (rng.next_u32() % 35) as usize]) };
        let out_mode = outs[(rng.next_u32() % 5) as usize];
        let p = random_params(&mut rng);
        let m = random_meta(&mut rng);
        let mut f = [[0i16; 8]; 8];
        let mut sc = 0;
        let e = get_mix_factors(is_cfg, in_mode, out_mode, &p, &m, &mut f, &mut sc);
        let mut cf = [0i16; 64];
        let mut csc = 0;
        let ce = unsafe {
            cs::cref_pcmdmx_static_get_mix_factors(is_cfg, in_mode, out_mode, params_vals(&p).as_ptr(), meta_vals(&m).as_ptr(), cf.as_mut_ptr(), &mut csc)
        };
        let what = format!("iteration {it}: in {in_mode:#x} (cfg {is_cfg}), out {out_mode:#x}, {p:?}, {m:?}");
        assert_eq!(e, ce, "getMixFactors result {what}");
        assert_eq!(f.concat().as_slice(), cf.as_slice(), "mixFactors {what}");
        assert_eq!(sc, csc, "outScale {what}");
    }
}

// ---------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------

fn random_param_id(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 9 {
        0 => DMX_PROFILE_SETTING,
        1 => DMX_BS_DATA_EXPIRY_FRAME,
        2 => DMX_BS_DATA_DELAY,
        3 => MIN_NUMBER_OF_OUTPUT_CHANNELS,
        4 => MAX_NUMBER_OF_OUTPUT_CHANNELS,
        5 => DMX_DUAL_CHANNEL_MODE,
        6 => DMX_PSEUDO_SURROUND_MODE,
        7 => rng.range(-2, 0x40),
        _ => rng.i32(),
    }
}

fn random_param_value(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 4 {
        0 => rng.i32(),
        1 => [-1, 0, 1, 2, 6, 8][(rng.next_u32() % 6) as usize],
        _ => rng.range(-3, 10),
    }
}

#[test]
fn pcm_dmx_params() {
    let mut rng = Rng::new(708);
    for seed in 0..300 {
        let mut d = Dmx::open();
        for step in 0..100 {
            let what = format!("seed {seed} step {step}");
            match rng.next_u32() % 8 {
                0 => {
                    let flags = rng.next_u32() % 5;
                    let e = pcm_dmx_reset(d.rs.as_deref_mut(), flags);
                    assert_eq!(e, unsafe { c::cref_pcmdmx_reset(d.c, flags) }, "pcmDmx_Reset {what}");
                    d.check(&format!("pcmDmx_Reset({flags}) {what}"));
                }
                1 => {
                    let (present, idx, ps) = (rng.range(-1, 2), rng.i32(), rng.range(-1, 2));
                    let e = pcm_dmx_set_matrix_mixdown_from_pce(d.rs.as_deref_mut(), present, idx, ps);
                    assert_eq!(e, unsafe { c::cref_pcmdmx_set_matrix_mixdown_from_pce(d.c, present, idx, ps) });
                    d.check(&format!("pcmDmx_SetMatrixMixdownFromPce {what}"));
                }
                2 => {
                    let param = random_param_id(&mut rng);
                    let mut v = 0x1234;
                    let mut cv = 0x1234;
                    let e = pcm_dmx_get_param(d.rs.as_deref(), param, Some(&mut v));
                    let ce = unsafe { c::cref_pcmdmx_get_param(d.c, param, &mut cv, 0) };
                    assert_eq!(e, ce, "pcmDmx_GetParam({param:#x}) {what}");
                    assert_eq!(v, cv, "pcmDmx_GetParam({param:#x}) value {what}");
                }
                _ => {
                    let param = random_param_id(&mut rng);
                    let value = random_param_value(&mut rng);
                    d.set_param(param, value, &what);
                }
            }
        }
    }
}

#[test]
fn pcm_dmx_null_arguments() {
    let n = core::ptr::null_mut();
    unsafe {
        let mut ce = 0;
        assert!(c::cref_pcmdmx_open(1, &mut ce).is_null());
        assert_eq!(pcm_dmx_open(None), ce, "pcmDmx_Open(NULL)");
        assert_eq!(pcm_dmx_close(None), c::cref_pcmdmx_close(n, 1), "pcmDmx_Close(NULL)");
        assert_eq!(pcm_dmx_reset(None, 3), c::cref_pcmdmx_reset(n, 3));
        assert_eq!(pcm_dmx_set_matrix_mixdown_from_pce(None, 1, 1, 1), c::cref_pcmdmx_set_matrix_mixdown_from_pce(n, 1, 1, 1));
        let mut v = 0;
        for param in [0, 1, 0x10, 0x11, 0x20, 0x21, 0x30, 0x31, 0x40] {
            for value in [-2, -1, 0, 1, 2, 3, 6, 8] {
                assert_eq!(pcm_dmx_set_param(None, param, value), c::cref_pcmdmx_set_param(n, param, value), "SetParam {param} {value}");
            }
            assert_eq!(pcm_dmx_get_param(None, param, Some(&mut v)), c::cref_pcmdmx_get_param(n, param, &mut v, 0));
        }
        let d = Dmx::open();
        assert_eq!(pcm_dmx_get_param(d.rs.as_deref(), 1, None), c::cref_pcmdmx_get_param(d.c, 1, &mut v, 1));
        let mut buf = [0u8; 16];
        assert_eq!(pcm_dmx_read_dvb_anc_data(None, Some(&buf), 3, 0), c::cref_pcmdmx_read_dvb_anc_data(n, buf.as_mut_ptr(), 3, 0));
        assert_eq!(pcm_dmx_parse(None, None, 24, 0), c::cref_pcmdmx_parse(n, [0u32; 9].as_mut_ptr(), buf.as_mut_ptr(), 24, 0, 1));
    }
}

// ---------------------------------------------------------------------------------------------
// Metadata parser
// ---------------------------------------------------------------------------------------------

/// A random bitstream over `1 << size_log2` bytes, starting at a random position; the DVB sync
/// byte is placed at the position where pcmDmx_Parse expects it in most cases.
struct Bs {
    rs: FdkBitStream,
    rbuf: BitMem,
    cst: [u32; 9],
    cbuf: Vec<u8>,
}

fn bs_state(bs: &FdkBitStream) -> [u32; 9] {
    let b = &bs.h_bit_buf;
    [bs.cache_word, bs.bits_in_cache, bs.config_cache, b.valid_bits, b.read_offset, b.write_offset, b.bit_ndx, b.buf_size(), b.buf_bits()]
}

fn random_bs(rng: &mut Rng, is_mpeg2: bool) -> Bs {
    let size_log2 = 2 + rng.next_u32() % 7;
    let size = 1u32 << size_log2;
    let mut mem: Vec<u8> = (0..size).map(|_| rng.next_u32() as u8).collect();
    // DVB-like payloads: fields with mostly realistic flags.
    let byte_pos = rng.next_u32() % size;
    let sync_pos = (byte_pos + if is_mpeg2 { 2 } else { 0 }) % size;
    if rng.next_u32() % 8 != 0 {
        mem[sync_pos as usize] = 0xBC;
    }
    let mut p = Bs { rs: FdkBitStream::default(), rbuf: BitMem::from_slice(&mem), cst: [0; 9], cbuf: mem };
    fdk_init_bit_stream(&mut p.rs, &p.rbuf, size, 0, 0);
    let bits = size * 8;
    let b = &mut p.rs.h_bit_buf;
    b.bit_ndx = if rng.next_u32() % 4 == 0 { rng.next_u32() % bits } else { byte_pos * 8 };
    b.read_offset = rng.next_u32() % size;
    b.write_offset = rng.next_u32() % size;
    b.valid_bits = match rng.next_u32() % 4 {
        0 => rng.next_u32() % (bits + 1),
        1 => rng.next_u32() % 160,
        _ => 24 + rng.next_u32() % 120,
    };
    if rng.next_u32() % 4 == 0 {
        p.rs.cache_word = rng.next_u32();
        p.rs.bits_in_cache = rng.next_u32() % 32;
    }
    p.cst = bs_state(&p.rs);
    p
}

#[test]
fn pcm_dmx_parse_() {
    let mut rng = Rng::new(709);
    let mut ok = 0;
    for seed in 0..200 {
        let mut d = Dmx::open();
        for step in 0..100 {
            let is_mpeg2 = (rng.next_u32() % 3 == 0) as i32;
            let mut bs = random_bs(&mut rng, is_mpeg2 != 0);
            let anc_bits = match rng.next_u32() % 4 {
                0 => rng.next_u32() % 200,
                1 => rng.next_u32(),
                _ => bs.rs.h_bit_buf.valid_bits.wrapping_add(bs.rs.bits_in_cache).wrapping_sub(rng.next_u32() % 64),
            };
            let what = format!("seed {seed} step {step}, mpeg2 {is_mpeg2}, ancDataBits {anc_bits}");
            let e = pcm_dmx_parse(d.rs.as_deref_mut(), Some(&mut bs.rs), anc_bits, is_mpeg2);
            let (st, buf) = (bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr());
            let ce = unsafe { c::cref_pcmdmx_parse(d.c, st, buf, anc_bits, is_mpeg2, 0) };
            assert_eq!(e, ce, "pcmDmx_Parse result {what}");
            if e == PCMDMX_OK {
                ok += 1;
            }
            assert_eq!(bs_state(&bs.rs), bs.cst, "bitstream state after pcmDmx_Parse {what}");
            assert_eq!(bs.rbuf.to_vec(), bs.cbuf);
            d.check(&format!("pcmDmx_Parse {what}"));
            if rng.next_u32() % 10 == 0 {
                let flags = rng.next_u32() % 4;
                pcm_dmx_reset(d.rs.as_deref_mut(), flags);
                unsafe { c::cref_pcmdmx_reset(d.c, flags) };
            }
        }
    }
    assert!(ok > 2000, "{ok} successful parses");
}

#[test]
fn pcm_dmx_read_dvb_anc_data_() {
    let mut rng = Rng::new(710);
    for seed in 0..200 {
        let mut d = Dmx::open();
        for step in 0..100 {
            let is_mpeg2 = (rng.next_u32() % 3 == 0) as i32;
            let mut buf: Vec<u8> = (0..16).map(|_| rng.next_u32() as u8).collect();
            if rng.next_u32() % 8 != 0 {
                buf[if is_mpeg2 != 0 { 2 } else { 0 }] = 0xBC;
            }
            let bytes = match rng.next_u32() % 4 {
                0 => rng.next_u32() % 40,
                1 => rng.next_u32(),
                _ => 3 + rng.next_u32() % 14,
            };
            let what = format!("seed {seed} step {step}, mpeg2 {is_mpeg2}, bytes {bytes}");
            let e = pcm_dmx_read_dvb_anc_data(d.rs.as_deref_mut(), Some(&buf), bytes, is_mpeg2);
            let ce = unsafe { c::cref_pcmdmx_read_dvb_anc_data(d.c, buf.as_mut_ptr(), bytes, is_mpeg2) };
            assert_eq!(e, ce, "pcmDmx_ReadDvbAncData result {what}");
            d.check(&format!("pcmDmx_ReadDvbAncData {what}"));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// pcmDmx_ApplyFrame
// ---------------------------------------------------------------------------------------------

struct Frame {
    n_ch: Vec<(i32, u8)>,
    frame_size: u32,
    interleaved: i32,
    pcm_buf_size: i32,
    use_out_scale: bool,
    null_mask: i32,
    pcm: Vec<i32>,
    t: [i32; 8],
    idx: [u8; 8],
}

fn random_frame(rng: &mut Rng) -> Frame {
    let n_ch = random_channels(rng);
    let frame_size = match rng.next_u32() % 16 {
        0 => [120, 128, 240, 256, 480, 512, 960, 1024][(rng.next_u32() % 8) as usize],
        1 => 0,
        _ => 1 + rng.next_u32() % 48,
    };
    // Room for channel offsets up to 255 as well.
    let len = 257 * frame_size.max(1) as usize + 16;
    let pcm: Vec<i32> = match rng.next_u32() % 4 {
        0 => (0..len).map(|_| rng.next_u32() as i32).collect(),
        1 => (0..len).map(|_| rng.i32()).collect(),
        2 => (0..len).map(|_| (rng.next_u32() as i32) >> (rng.next_u32() % 16)).collect(),
        _ => (0..len).map(|i| if (i / 7) % 2 == 0 { i32::MAX } else { i32::MIN }).collect(),
    };
    let pcm_buf_size = match rng.next_u32() % 8 {
        0 => rng.range(0, 9) * frame_size as i32 - rng.range(0, 2),
        1 => rng.i32(),
        _ => len as i32,
    };
    let (t, idx) = channel_arrays(rng, &n_ch);
    let null_mask = if rng.next_u32() % 50 == 0 { 1 << (rng.next_u32() % 5) } else { 0 };
    Frame { n_ch, frame_size, interleaved: (rng.next_u32() % 2) as i32, pcm_buf_size, use_out_scale: rng.next_u32() % 3 != 0, null_mask, pcm, t, idx }
}

/// pcmDmx_ApplyFrame on both sides; compares everything. Returns the result.
fn apply_frame(d: &mut Dmx, fr: &Frame, md: &MapDescr, n_channels: i32, what: &str) -> i32 {
    let mut pcm = fr.pcm.clone();
    let mut cpcm = fr.pcm.clone();
    let mut n = n_channels;
    let mut cn = n_channels;
    let (mut t, mut idx) = (fr.t, fr.idx);
    let (mut ct, mut cidx) = (fr.t, fr.idx);
    let mut sc = 0x77;
    let mut csc = 0x77;
    let m = fr.null_mask;
    let e = pcm_dmx_apply_frame(
        d.rs.as_deref_mut(),
        if m & 16 != 0 { None } else { Some(&mut pcm) },
        fr.pcm_buf_size,
        fr.frame_size,
        if m & 1 != 0 { None } else { Some(&mut n) },
        fr.interleaved,
        if m & 2 != 0 { None } else { Some(&mut t) },
        if m & 4 != 0 { None } else { Some(&mut idx) },
        if m & 8 != 0 { None } else { Some(&md.rs) },
        if fr.use_out_scale { Some(&mut sc) } else { None },
    );
    let ce = unsafe {
        c::cref_pcmdmx_apply_frame(
            d.c,
            cpcm.as_mut_ptr(),
            fr.pcm_buf_size,
            fr.frame_size,
            &mut cn,
            fr.interleaved,
            ct.as_mut_ptr(),
            cidx.as_mut_ptr(),
            md.kind,
            md.pass,
            md.maps.as_ptr(),
            md.nch.as_ptr(),
            15,
            fr.use_out_scale as i32,
            &mut csc,
            m,
        )
    };
    assert_eq!(e, ce, "pcmDmx_ApplyFrame result {what}");
    assert_eq!(n, cn, "nChannels {what}");
    assert_eq!(t, ct, "channelType {what}");
    assert_eq!(idx, cidx, "channelIndices {what}");
    assert_eq!(sc, csc, "pDmxOutScale {what}");
    if pcm != cpcm {
        let i = pcm.iter().zip(&cpcm).position(|(a, b)| a != b).unwrap();
        panic!("PCM output differs at {i}: {} vs {} ({what})", pcm[i], cpcm[i]);
    }
    d.check(&format!("pcmDmx_ApplyFrame {what}"));
    e
}

fn random_settings(d: &mut Dmx, rng: &mut Rng, what: &str) {
    let outs = [-1, 0, 1, 2, 6, 8];
    for _ in 0..rng.next_u32() % 4 {
        d.set_param(MIN_NUMBER_OF_OUTPUT_CHANNELS, outs[(rng.next_u32() % 6) as usize], what);
    }
    for _ in 0..rng.next_u32() % 4 {
        d.set_param(MAX_NUMBER_OF_OUTPUT_CHANNELS, outs[(rng.next_u32() % 6) as usize], what);
    }
    if rng.next_u32() % 2 == 0 {
        d.set_param(DMX_DUAL_CHANNEL_MODE, rng.range(0, 3), what);
    }
    if rng.next_u32() % 2 == 0 {
        d.set_param(DMX_PROFILE_SETTING, rng.range(0, 3), what);
    }
    if rng.next_u32() % 2 == 0 {
        d.set_param(DMX_PSEUDO_SURROUND_MODE, rng.range(-1, 1), what);
    }
    if rng.next_u32() % 4 == 0 {
        d.set_param(DMX_BS_DATA_DELAY, rng.range(0, 1), what);
    }
    if rng.next_u32() % 4 == 0 {
        d.set_param(DMX_BS_DATA_EXPIRY_FRAME, rng.range(-1, 4), what);
    }
}

#[test]
fn pcm_dmx_apply_frame_random() {
    let mut rng = Rng::new(711);
    let mut results = [0u32; 16];
    let mut changed = 0;
    for seed in 0..1500 {
        let mut d = Dmx::open();
        let md = MapDescr::new(&mut rng);
        for step in 0..12 {
            let what = format!("seed {seed} step {step}");
            if step == 0 || rng.next_u32() % 3 == 0 {
                random_settings(&mut d, &mut rng, &what);
            }
            if rng.next_u32() % 2 == 0 {
                let m = [random_meta(&mut rng), random_meta(&mut rng)];
                d.set_meta(m);
            }
            let fr = random_frame(&mut rng);
            let n_channels = match rng.next_u32() % 40 {
                0 => rng.range(-2, 12),
                _ => fr.n_ch.len() as i32,
            };
            let what = format!("{what}: channels {:?}, nChannels {n_channels}, frameSize {}, interleaved {}, map kind {} pass {}", fr.n_ch, fr.frame_size, fr.interleaved, md.kind, md.pass);
            if n_channels < 0 {
                continue; // no defined result in the reference
            }
            let e = apply_frame(&mut d, &fr, &md, n_channels, &what);
            results[e as usize & 15] += 1;
            if n_channels != fr.n_ch.len() as i32 || e == PCMDMX_OK {
                changed += 1;
            }
        }
    }
    // Every outcome is exercised.
    for e in [PCMDMX_OK, PCMDMX_INVALID_HANDLE, PCMDMX_INVALID_ARGUMENT, PCMDMX_INVALID_CH_CONFIG, PCMDMX_INVALID_MODE, PCMDMX_OUTPUT_BUFFER_TOO_SMALL] {
        assert!(results[e as usize] > 20, "result {e}: {results:?}");
    }
    assert!(changed > 1000);
}

/// Every MPEG channel configuration against every output channel count and mode.
#[test]
fn pcm_dmx_apply_frame_configurations() {
    let mut rng = Rng::new(712);
    let f = ACT_FRONT;
    let b = ACT_BACK;
    let l = ACT_LFE;
    let ft = ACT_FRONT_TOP;
    let s = ACT_SIDE;
    let cfgs: Vec<Vec<(i32, u8)>> = vec![
        vec![(f, 0)],
        vec![(f, 0), (f, 1)],
        vec![(f, 0), (f, 1), (f, 2)],
        vec![(f, 0), (f, 1), (f, 2), (b, 0)],
        vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1)],
        vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (l, 0)],
        vec![(f, 0), (f, 1), (f, 2), (f, 3), (f, 4), (b, 0), (b, 1), (l, 0)],
        vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (b, 2), (l, 0)],
        vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (b, 2), (b, 3), (l, 0)],
        vec![(f, 0), (f, 1), (f, 2), (b, 0), (b, 1), (l, 0), (ft, 0), (ft, 1)],
        vec![(f, 0), (f, 1), (b, 0)],
        vec![(f, 0), (f, 1), (b, 0), (b, 1)],
        vec![(f, 0), (f, 1), (f, 2), (s, 0), (s, 1), (b, 0), (l, 0)],
        vec![(f, 0), (f, 1), (f, 2), (s, 0), (s, 1), (b, 0), (b, 1), (l, 0)],
        vec![(f, 0), (f, 1), (f, 2), (f, 3), (f, 4), (b, 0), (l, 0)],
        vec![(f, 0), (b, 0)],
    ];
    let outs = [-1, 0, 1, 2, 6, 8];
    let mut count = 0;
    for (ci, cfg) in cfgs.iter().enumerate() {
        for &min in &outs {
            for &max in &outs {
                for dual in 0..4 {
                    for profile in 0..4 {
                        for ps in -1..=1 {
                            let mut d = Dmx::open();
                            let md = MapDescr::new(&mut rng);
                            let what = format!("cfg {ci} min {min} max {max} dual {dual} profile {profile} ps {ps}");
                            d.set_param(MIN_NUMBER_OF_OUTPUT_CHANNELS, min, &what);
                            d.set_param(MAX_NUMBER_OF_OUTPUT_CHANNELS, max, &what);
                            d.set_param(DMX_DUAL_CHANNEL_MODE, dual, &what);
                            d.set_param(DMX_PROFILE_SETTING, profile, &what);
                            d.set_param(DMX_PSEUDO_SURROUND_MODE, ps, &what);
                            let m = [random_meta(&mut rng), random_meta(&mut rng)];
                            d.set_meta(m);
                            let mut fr = random_frame(&mut rng);
                            let mut order = cfg.clone();
                            if rng.next_u32() % 2 == 0 {
                                for j in (1..order.len()).rev() {
                                    let k = (rng.next_u32() as usize) % (j + 1);
                                    order.swap(j, k);
                                }
                            }
                            fr.n_ch = order;
                            let (t, idx) = channel_arrays(&mut rng, &fr.n_ch);
                            fr.t = t;
                            fr.idx = idx;
                            fr.null_mask = 0;
                            if fr.frame_size == 0 {
                                fr.frame_size = 32;
                                let len = 257 * 32 + 16;
                                fr.pcm = (0..len).map(|_| rng.i32()).collect();
                            }
                            fr.pcm_buf_size = fr.pcm.len() as i32;
                            let n = fr.n_ch.len() as i32;
                            let what = format!("{what}: {:?}, frameSize {}, interleaved {}", fr.n_ch, fr.frame_size, fr.interleaved);
                            apply_frame(&mut d, &fr, &md, n, &what);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 16 * 36 * 4 * 4 * 3);
}

// ---------------------------------------------------------------------------------------------
// Library info
// ---------------------------------------------------------------------------------------------

const N: usize = FDK_MODULE_LAST as usize;

fn check_lib_info(ids_in: &[i32; N], null_info: bool) {
    let mut ids = [0i32; N];
    let mut version = [0i32; N];
    let mut flags = [0u32; N];
    let mut vstr = [0i8; 32 * N];
    let mut title = [0i8; 32 * N];
    let e = unsafe {
        c::cref_pcmdmx_get_lib_info(
            null_info as i32,
            ids_in.as_ptr(),
            ids.as_mut_ptr(),
            version.as_mut_ptr(),
            flags.as_mut_ptr(),
            vstr.as_mut_ptr(),
            title.as_mut_ptr(),
        )
    };
    let mut info = [LibInfo::default(); N];
    for (i, e) in info.iter_mut().enumerate() {
        e.module_id = ids_in[i];
        e.version_str = [b'x'; 32];
    }
    let r = pcm_dmx_get_lib_info(if null_info { None } else { Some(&mut info) });
    assert_eq!(r, e, "pcmDmx_GetLibInfo result for {ids_in:?}");
    for i in 0..N {
        let what = format!("entry {i} for {ids_in:?}");
        assert_eq!(info[i].module_id, ids[i], "{what}: module_id");
        assert_eq!(info[i].version, version[i], "{what}: version");
        assert_eq!(info[i].flags, flags[i], "{what}: flags");
        let cv: Vec<u8> = vstr[32 * i..32 * i + 32].iter().map(|&b| b as u8).collect();
        assert_eq!(info[i].version_str.as_slice(), cv.as_slice(), "{what}: versionStr");
        let ct: Vec<u8> = title[32 * i..32 * i + 32].iter().map(|&b| b as u8).take_while(|&b| b != 0).collect();
        assert_eq!(info[i].title.as_bytes(), ct.as_slice(), "{what}: title");
    }
}

#[test]
fn pcm_dmx_get_lib_info_() {
    assert_eq!(FDK_MODULE_LAST, unsafe { c::cref_pcmdmx_module_last() });
    check_lib_info(&[0; N], true);
    check_lib_info(&[0; N], false);
    check_lib_info(&[3; N], false);
    let mut ids = [0; N];
    ids[0] = FDK_PCMDMX;
    ids[1] = FDK_TOOLS;
    check_lib_info(&ids, false);
    let mut rng = Rng::new(713);
    for _ in 0..1000 {
        let mut ids = [0i32; N];
        let n = (rng.next_u32() as usize) % (N + 1);
        for id in ids.iter_mut().take(n) {
            *id = 1 + (rng.next_u32() % 40) as i32;
        }
        if rng.next_u32() % 2 == 0 {
            for id in ids.iter_mut().skip(n + 1) {
                *id = (rng.next_u32() % 40) as i32;
            }
        }
        check_lib_info(&ids, false);
    }
}

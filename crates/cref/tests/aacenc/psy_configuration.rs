// Differential tests of fdk_aac_rs_aacenc::psy_configuration against the C++ reference.
//
// FDKaacEnc_InitPsyConfiguration is run for every sample rate, granule length / filter bank and
// block type of the encoder with many bit rates, bandwidths and IS/MS flags; the complete
// PSY_CONFIGURATION (copied from C into the port's #[repr(C)] struct) is compared. The helpers
// are also used by the TNS and PNS tests to build their configurations.

use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc::*;
use fdk_aac_rs_aacenc::psy_configuration::tables::*;
use fdk_aac_rs_aacenc::psy_configuration::*;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_cref::aacenc_psy_configuration as c;
use fdk_aac_rs_cref::testutil::Rng;

/// The sample rates the encoder accepts.
pub const RATES: [i32; 12] =
    [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];

/// Granule lengths and filter banks of the encoder (LC, LD, ELD with the downscaled lengths).
///
/// 960 is not among them: it has no defined x86 reference result. It is tested on aarch64 only
/// (`init_psy_configuration_960`).
pub const GRANULES: [(i32, FbType); 9] = [
    (1024, FB_LC),
    (512, FB_LD),
    (480, FB_LD),
    (512, FB_ELD),
    (480, FB_ELD),
    (256, FB_ELD),
    (240, FB_ELD),
    (128, FB_ELD),
    (120, FB_ELD),
];

/// Bit rates per channel.
pub const BITRATES: [i32; 21] = [
    6000, 8000, 12000, 16000, 20000, 24000, 28000, 32000, 40000, 48000, 56000, 64000, 80000,
    96000, 128000, 160000, 192000, 256000, 320000, 400000, 576000,
];

/// A PSY_CONFIGURATION with all fields zero.
pub fn zeroed_psy_conf() -> PsyConfiguration {
    // SAFETY: PsyConfiguration consists of integers only.
    unsafe { core::mem::zeroed() }
}

/// `FDKaacEnc_InitPsyConfiguration` of the reference.
pub fn c_init_psy(
    bitrate: i32,
    samplerate: i32,
    bandwidth: i32,
    blocktype: i32,
    granule_length: i32,
    use_is: i32,
    use_ms: i32,
    filterbank: FbType,
) -> (i32, PsyConfiguration) {
    let mut p = zeroed_psy_conf();
    let e = unsafe {
        c::cref_psy_configuration_InitPsyConfiguration(
            bitrate,
            samplerate,
            bandwidth,
            blocktype,
            granule_length,
            use_is,
            use_ms,
            &mut p as *mut PsyConfiguration as *mut c_void,
            filterbank,
        )
    };
    (e, p)
}

/// `FDKaacEnc_InitPsyConfiguration` of the port.
pub fn r_init_psy(
    bitrate: i32,
    samplerate: i32,
    bandwidth: i32,
    blocktype: i32,
    granule_length: i32,
    use_is: i32,
    use_ms: i32,
    filterbank: FbType,
) -> (i32, PsyConfiguration) {
    let mut p = zeroed_psy_conf();
    let e = fdk_aac_enc_init_psy_configuration(
        bitrate,
        samplerate,
        bandwidth,
        blocktype,
        granule_length,
        use_is,
        use_ms,
        &mut p,
        filterbank,
    );
    (e, p)
}

/// Runs both inits, checks that they agree and returns the configuration.
pub fn init_psy_checked(
    bitrate: i32,
    samplerate: i32,
    bandwidth: i32,
    blocktype: i32,
    granule_length: i32,
    use_is: i32,
    use_ms: i32,
    filterbank: FbType,
) -> (i32, PsyConfiguration) {
    let a = (bitrate, samplerate, bandwidth, blocktype, granule_length, use_is, use_ms, filterbank);
    let rc = c_init_psy(a.0, a.1, a.2, a.3, a.4, a.5, a.6, a.7);
    let rr = r_init_psy(a.0, a.1, a.2, a.3, a.4, a.5, a.6, a.7);
    assert_eq!(rr.0, rc.0, "{a:?}: return value");
    assert_eq!(rr.1, rc.1, "{a:?}: PSY_CONFIGURATION");
    rr
}

/// Block types of a granule length (short blocks only for long frames).
pub fn block_types(granule_length: i32) -> &'static [i32] {
    if granule_length > 512 {
        &[LONG_WINDOW, START_WINDOW, SHORT_WINDOW, STOP_WINDOW]
    } else {
        &[LONG_WINDOW]
    }
}

#[test]
fn init_psy_configuration_grid() {
    let mut r = Rng::new(0x9c0f_1001);
    let mut n = 0;
    for &sr in &RATES {
        for &(gl, fb) in &GRANULES {
            for &bt in block_types(gl) {
                for &br in &BITRATES {
                    for k in 0..3 {
                        let bw = match k {
                            0 => sr / 2,
                            1 => r.range(1000, sr / 2),
                            _ => r.range(500, 24000),
                        };
                        let (is, ms) = (r.range(0, 1), r.range(0, 1));
                        let (e, _) = init_psy_checked(br, sr, bw, bt, gl, is, ms, fb);
                        assert_eq!(e, AAC_ENC_OK);
                        n += 1;
                    }
                }
            }
        }
    }
    assert!(n > 5000);
}

/// Frame length 960 (see GRANULES): aarch64 only.
#[cfg(target_arch = "aarch64")]
#[test]
fn init_psy_configuration_960() {
    for &sr in &RATES {
        for &bt in block_types(960) {
            for &br in &BITRATES {
                init_psy_checked(br, sr, sr / 2, bt, 960, 1, 1, FB_LC);
            }
        }
    }
}

#[test]
fn init_psy_configuration_edge() {
    let mut r = Rng::new(0x9c0f_1002);
    // Unsupported frame lengths and sample rates (for the 1024/960 table; inputs without a
    // defined reference result are left out, see fdk_aac_enc_init_sfb_table).
    for &(gl, sr) in &[(1000, 48000), (0, 48000), (1024, 7350), (1024, 44000), (1024, 0)] {
        let (e, _) = init_psy_checked(64000, sr, 16000, LONG_WINDOW, gl, 1, 1, FB_LC);
        assert_ne!(e, AAC_ENC_OK);
    }
    // Invalid block types: no sfb table, all bands empty; no defined x86 reference result, so
    // aarch64 only.
    if cfg!(target_arch = "aarch64") {
        for bt in [4, 5, -1, 100] {
            for &(gl, fb) in &GRANULES {
                init_psy_checked(64000, 48000, 16000, bt, gl, 1, 1, fb);
            }
        }
    }
    // Extreme bit rates and bandwidths.
    for _ in 0..3000 {
        let sr = RATES[r.range(0, 11) as usize];
        let (gl, fb) = GRANULES[r.range(0, 8) as usize];
        let bts = block_types(gl);
        let bt = bts[r.range(0, bts.len() as i32 - 1) as usize];
        let br = match r.range(0, 3) {
            0 => r.range(1, 1000),
            1 => r.range(1, 2_000_000),
            2 => r.range(0, 10_000_000),
            _ => BITRATES[r.range(0, 20) as usize],
        };
        let bw = match r.range(0, 3) {
            0 => r.range(1, 100),
            1 => r.range(1, 200_000),
            2 => sr,
            _ => r.range(sr / 4, sr),
        };
        init_psy_checked(br, sr, bw, bt, gl, r.range(0, 1), r.range(0, 1), fb);
    }
}

#[test]
fn init_sfb_table() {
    for &sr in RATES.iter().chain(&[16000 * 2, 22050 * 4, 48000 * 4, 96000 * 4, 7350]) {
        for gl in [1024, 960, 512, 480, 256, 0] {
            // (initSfbTable alone has no division)
            for bt in [LONG_WINDOW, START_WINDOW, SHORT_WINDOW, STOP_WINDOW, 4, 5] {
                let ld = gl == 512 || gl == 480;
                // Inputs without a defined reference result are skipped.
                let ld_known = [16000 * 2, 22050 * 4, 48000 * 4, 96000 * 4].contains(&sr)
                    || RATES.contains(&sr);
                if ld && (bt == SHORT_WINDOW || !ld_known) {
                    continue;
                }
                {
                    let cnt0 = 0;
                    let (mut oc, mut or) = ([0x55i32; MAX_SFB + 1], [0x55i32; MAX_SFB + 1]);
                    let (mut cc, mut cr) = (cnt0, cnt0);
                    let ec = unsafe {
                        c::cref_psy_configuration_initSfbTable(
                            sr,
                            bt,
                            gl,
                            oc.as_mut_ptr(),
                            &mut cc,
                        )
                    };
                    let er = fdk_aac_enc_init_sfb_table(sr, bt, gl, &mut or, &mut cr);
                    let w = format!("sr {sr} gl {gl} bt {bt} cnt0 {cnt0}");
                    assert_eq!(er, ec, "{w}: return value");
                    assert_eq!(cr, cc, "{w}: sfbCnt");
                    assert_eq!(or, oc, "{w}: sfbOffset");
                }
            }
        }
    }
}

#[test]
fn tables_match() {
    fn bytes<T>(t: &T) -> &[u8] {
        // SAFETY: plain integer structs without padding.
        unsafe { core::slice::from_raw_parts(t as *const T as *const u8, size_of::<T>()) }
    }
    let longs = [
        &P_22050_LONG_512,
        &P_32000_LONG_512,
        &P_44100_LONG_512,
        &P_22050_LONG_480,
        &P_32000_LONG_480,
        &P_44100_LONG_480,
    ];
    for (i, t) in longs.iter().enumerate() {
        let p = unsafe { c::cref_psy_configuration_param_long(i as i32) } as *const u8;
        let cb = unsafe { core::slice::from_raw_parts(p, size_of::<SfbParamLong>()) };
        assert_eq!(bytes(*t), cb, "SFB_PARAM_LONG {i}");
    }
    for (t, tab) in [&SFB_INFO_TAB[..], &SFB_INFO_TAB_LD_512[..], &SFB_INFO_TAB_LD_480[..]]
        .iter()
        .enumerate()
    {
        assert_eq!(tab.len() as i32, unsafe { c::cref_psy_configuration_info_count(t as i32) });
        for (i, e) in tab.iter().enumerate() {
            let (mut pl, mut ps) = (core::ptr::null(), core::ptr::null());
            let rate =
                unsafe { c::cref_psy_configuration_info(t as i32, i as i32, &mut pl, &mut ps) };
            assert_eq!(e.sample_rate, rate, "table {t} entry {i}");
            let cl = unsafe {
                core::slice::from_raw_parts(pl as *const u8, size_of::<SfbParamLong>())
            };
            assert_eq!(bytes(e.param_long), cl, "table {t} entry {i}: paramLong");
            match e.param_short {
                None => assert!(ps.is_null(), "table {t} entry {i}: paramShort"),
                Some(s) => {
                    assert!(!ps.is_null());
                    let cs = unsafe {
                        core::slice::from_raw_parts(ps as *const u8, size_of::<SfbParamShort>())
                    };
                    assert_eq!(bytes(s), cs, "table {t} entry {i}: paramShort");
                }
            }
        }
    }
}

#[test]
fn barc_line_value() {
    let mut r = Rng::new(0x9c0f_1003);
    for lines in [1024, 960, 512, 480, 256, 128, 120, 0] {
        for &sr in &RATES {
            for f in [1, 2, 4] {
                for line in 0..=1024 {
                    let fs = sr * f;
                    let c = unsafe { c::cref_psy_configuration_BarcLineValue(lines, line, fs) };
                    let rr = fdk_aac_enc_barc_line_value(lines, line, fs);
                    assert_eq!(rr, c, "lines {lines} line {line} fs {fs}");
                }
            }
        }
    }
    for _ in 0..100_000 {
        let lines = [1024, 512, 480, 128, r.range(0, 2000)][r.range(0, 4) as usize];
        let (line, fs) = match r.range(0, 1) {
            0 => (r.range(-2000, 5000), r.range(-400_000, 400_000)),
            _ => (r.i32(), r.i32()),
        };
        let c = unsafe { c::cref_psy_configuration_BarcLineValue(lines, line, fs) };
        let rr = fdk_aac_enc_barc_line_value(lines, line, fs);
        assert_eq!(rr, c, "lines {lines} line {line} fs {fs}");
    }
}

#[test]
fn get_mask_factor() {
    let mut r = Rng::new(0x9c0f_1004);
    for i in 0..200_000 {
        let (db, dbe, ten, tene) = if i % 2 == 0 {
            // as in FDKaacEnc_initSpreading
            (r.i32() >> r.range(0, 31), 23, 0x50000000, 27)
        } else {
            (r.i32(), r.range(-40, 70), r.i32(), r.range(-40, 70))
        };
        let c = unsafe { c::cref_psy_configuration_getMaskFactor(db, dbe, ten, tene) };
        let rr = fdk_aac_rs_aacenc::psy_configuration::get_mask_factor(db, dbe, ten, tene);
        assert_eq!(rr, c, "dbVal {db:#x} e {dbe} ten {ten:#x} e {tene}");
    }
}

/// The sub-functions of the init on the offsets of all configurations, with other bit rates
/// and random barc values and offsets.
#[test]
fn init_sub_functions() {
    let mut r = Rng::new(0x9c0f_1005);
    let mut offsets: Vec<(Vec<i32>, i32, i32, i32)> = Vec::new(); // offsets, sfbCnt, rate, bt
    for &sr in &RATES {
        for &(gl, fb) in &GRANULES {
            for &bt in block_types(gl) {
                let (_, p) = init_psy_checked(64000, sr, sr / 2, bt, gl, 0, 0, fb);
                offsets.push((p.sfb_offset.to_vec(), p.sfb_cnt, sr, bt));
            }
        }
    }
    for _ in 0..30 {
        // random increasing offsets
        let cnt = r.range(0, MAX_SFB as i32);
        let mut o = vec![0i32; MAX_SFB + 1];
        for i in 1..=cnt as usize {
            o[i] = o[i - 1] + r.range(1, 96);
        }
        offsets.push((o, cnt, RATES[r.range(0, 11) as usize], r.range(0, 3)));
    }
    for (o, cnt, sr, bt) in &offsets {
        let (cnt, sr, bt) = (*cnt, *sr, *bt);
        let lines = o[cnt as usize];
        let mut oc = o.clone();
        // initBarcValues
        let (mut bc, mut br) = ([0x11i32; MAX_SFB], [0x11i32; MAX_SFB]);
        for fs in [sr, sr * 2, sr * 4] {
            unsafe {
                c::cref_psy_configuration_initBarcValues(cnt, oc.as_mut_ptr(), lines, fs, bc.as_mut_ptr())
            };
            fdk_aac_enc_init_barc_values(cnt, o, lines, fs, &mut br);
            assert_eq!(br, bc, "initBarcValues cnt {cnt} fs {fs}");
        }
        // InitMinPCMResolution
        let (mut qc, mut qr) = ([0x22i32; MAX_SFB], [0x22i32; MAX_SFB]);
        unsafe {
            c::cref_psy_configuration_InitMinPCMResolution(cnt, oc.as_mut_ptr(), qc.as_mut_ptr())
        };
        fdk_aac_enc_init_min_pcm_resolution(cnt, o, &mut qr);
        assert_eq!(qr, qc, "InitMinPCMResolution");
        // initSpreading, with the barc values and random ones
        for k in 0..3 {
            let mut barc = bc;
            if k > 0 {
                for v in barc.iter_mut() {
                    *v = if k == 1 { r.range(0, 0x30000000) } else { r.i32() };
                }
                if k == 1 {
                    barc.sort();
                }
            }
            let bitrate = [8000, 20000, 20001, 64000][r.range(0, 3) as usize];
            let mut c4 = [[0x33i32; MAX_SFB]; 4];
            let mut r4 = c4;
            let [c0, c1, c2, c3] = &mut c4;
            unsafe {
                c::cref_psy_configuration_initSpreading(
                    cnt,
                    barc.as_mut_ptr(),
                    c0.as_mut_ptr(),
                    c1.as_mut_ptr(),
                    c2.as_mut_ptr(),
                    c3.as_mut_ptr(),
                    bitrate,
                    bt,
                )
            };
            let [r0, r1, r2, r3] = &mut r4;
            fdk_aac_enc_init_spreading(cnt, &barc, r0, r1, r2, r3, bitrate, bt);
            assert_eq!(r4, c4, "initSpreading cnt {cnt} k {k}");
        }
        // initMinSnr
        if cnt > 0 {
            for _ in 0..20 {
                let active = r.range(1, cnt);
                let bitrate = match r.range(0, 2) {
                    0 => BITRATES[r.range(0, 20) as usize],
                    1 => r.range(1000, 600_000),
                    _ => r.range(1, 0x7fff_ffff),
                };
                let fs = [sr, sr * 2, sr * 4][r.range(0, 2) as usize];
                // numLines values with a defined x86 reference result only.
                let lines_m = [1024, 512, 480, 128][r.range(0, 3) as usize];
                let (mut sc, mut sr_) = ([0x44i32; MAX_SFB], [0x44i32; MAX_SFB]);
                unsafe {
                    c::cref_psy_configuration_initMinSnr(
                        bitrate,
                        fs,
                        lines_m,
                        oc.as_ptr(),
                        active,
                        bt,
                        sc.as_mut_ptr(),
                    )
                };
                fdk_aac_enc_init_min_snr(bitrate, fs, lines_m, o, active, bt, &mut sr_);
                assert_eq!(sr_, sc, "initMinSnr br {bitrate} fs {fs} lines {lines_m} active {active}");
            }
        }
        oc.clear();
    }
}

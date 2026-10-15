// Differential tests of fdk_aac_rs_fdk::syslib_channel_map_descr against the C++ reference.

use fdk_aac_rs_cref::fdk_syslib_channel_map_descr::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::syslib_channel_map_descr::*;

#[test]
fn default_and_wg4_tables() {
    assert_eq!(FDK_MAP_INFO_TAB_LEN_WG4, unsafe { cref_chmap_wg4_len() });
    for pass in [0u32, 1, 7] {
        let mut dflt = FdkChannelMapDescr::default();
        fdk_ch_map_descr_init(&mut dflt, None, 0, pass);
        let mut wg4 = FdkChannelMapDescr::default();
        fdk_ch_map_descr_init(&mut wg4, Some(&FDK_MAP_INFO_TAB_WG4), FDK_MAP_INFO_TAB_LEN_WG4, pass);
        assert_eq!(fdk_ch_map_descr_is_valid(Some(&dflt)), 1);
        assert_eq!(fdk_ch_map_descr_is_valid(Some(&wg4)), 1);
        for map_idx in 0..20u32 {
            for ch in 0..=255u8 {
                assert_eq!(
                    fdk_ch_map_descr_get_map_value(&dflt, ch, map_idx),
                    unsafe { cref_chmap_default_get(pass, ch, map_idx) },
                    "default, pass {pass}, ch {ch}, map {map_idx}"
                );
                assert_eq!(
                    fdk_ch_map_descr_get_map_value(&wg4, ch, map_idx),
                    unsafe { cref_chmap_wg4_get(pass, ch, map_idx) },
                    "Wg4, pass {pass}, ch {ch}, map {map_idx}"
                );
            }
        }
        for map_idx in [u32::MAX, 0x8000_0000] {
            assert_eq!(fdk_ch_map_descr_get_map_value(&dflt, 3, map_idx), unsafe {
                cref_chmap_default_get(pass, 3, map_idx)
            });
        }
    }
}

/// Random channel maps (often permutations, sometimes with duplicates or values out of range,
/// including values of 32 and more; maps of 32 or more channels take the general branch, which
/// compares the values without shifting).
fn random_map(r: &mut Rng) -> &'static [u8] {
    let n = match r.next_u32() % 4 {
        0 => r.range(0, 4),
        1 => r.range(28, 40),
        _ => r.range(0, 24),
    } as usize;
    let mut m: Vec<u8> = (0..n as u8).collect();
    for i in (1..n).rev() {
        m.swap(i, r.range(0, i as i32) as usize);
    }
    match r.next_u32() % 4 {
        0 if n > 0 => {
            let i = r.range(0, n as i32 - 1) as usize;
            m[i] = if r.next_u32() % 2 == 0 { r.next_u32() as u8 % 70 } else { r.next_u32() as u8 };
        }
        1 if n > 1 => {
            m[0] = m[n - 1];
        }
        _ => {}
    }
    Box::leak(m.into_boxed_slice())
}

/// Whether the C++ reference of this test binary was compiled by Apple clang for x86-64
/// (`--target x86_64-apple-darwin`, ref-x86_64-mac). At `1 << v` with `v >= 32` it vectorizes
/// the first `numChannels & 0x18` values as a `float` 2^v, while the port's `x86` variant follows
/// ref-x86_64 (Debian clang 19: scalar `bts`, amount modulo 32).
const APPLE_CLANG_X86: bool = cfg!(all(target_arch = "x86_64", target_vendor = "apple"));

/// The map with its first `num_ch < 32` values folded into 0..31 (keeping them out of range or
/// duplicate) when the reference is Apple clang x86 (see `APPLE_CLANG_X86`); otherwise unchanged.
fn fold_for_apple_clang_x86(map: &'static [u8], num_ch: u8) -> &'static [u8] {
    let n = num_ch as usize;
    if APPLE_CLANG_X86 && n < 32 && map.iter().take(n).any(|&v| v >= 32) {
        let m: Vec<u8> =
            map.iter().enumerate().map(|(i, &v)| if i < n { v & 31 } else { v }).collect();
        Box::leak(m.into_boxed_slice())
    } else {
        map
    }
}

/// `FDK_chMapDescr_isValid` and `FDK_chMapDescr_init` (which inlines the check) with one map
/// value of 32 or more at every position of the identity map of 1 to 31 channels, alone and with
/// the value's residue modulo 32 missing or present elsewhere: the undefined shifts in the
/// vectorized part (ref-arm64: the first `n & 0x18` values) and in the scalar code.
///
/// Called from `isValid_init_setPassThrough`: `cref_chmap_init_custom` keeps the map in a static
/// that two tests in parallel threads would share.
fn check_map_values_of_32_and_more() {
    if APPLE_CLANG_X86 {
        eprintln!("skipped: the reference is ref-x86_64-mac (Apple clang)");
        return;
    }
    let mut checked = 0;
    for n in 1..32u8 {
        for pos in 0..n as usize {
            for v in [31u8, 32, 33, 34, 36, 40, 47, 48, 63, 64, 65, 95, 96, 127, 128, 129, 159, 160, 200, 254, 255] {
                for other in [None, Some((v % 32) as usize), Some(pos)] {
                    let mut m: Vec<u8> = (0..n).collect();
                    m[pos] = v;
                    if let Some(o) = other {
                        // put the residue at another position (if there is one)
                        let o = if o == pos { (pos + 1) % n as usize } else { o };
                        if o != pos && o < n as usize {
                            m[o] = v % 32;
                        }
                    }
                    let map: &'static [u8] = Box::leak(m.into_boxed_slice());
                    let info: &'static [ChannelMapInfo] =
                        Box::leak(Box::new([ChannelMapInfo { channel_map: map, num_channels: n }]));
                    let d = FdkChannelMapDescr {
                        map_info_tab: Some(info),
                        map_info_tab_len: 1,
                        f_pass_through: 0,
                    };
                    assert_eq!(
                        fdk_ch_map_descr_is_valid(Some(&d)),
                        unsafe { cref_chmap_is_valid_single(map.as_ptr(), n) },
                        "isValid {map:?}"
                    );
                    let mut d = FdkChannelMapDescr::default();
                    fdk_ch_map_descr_init(&mut d, Some(info), 1, 0);
                    let default_chosen = d.map_info_tab.map(|t| t.as_ptr()) != Some(info.as_ptr());
                    let (mut c_value, mut c_err) = (0u8, 0i32);
                    let c_default = unsafe {
                        cref_chmap_init_custom(map.as_ptr(), n, 1, 0, 0, 0, 0, &mut c_value, &mut c_err)
                    };
                    assert_eq!(default_chosen as i32, c_default, "init {map:?}");
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 10000, "{checked}");
}

#[test]
fn isValid_init_setPassThrough() {
    let mut r = Rng::new(1201);
    assert_eq!(fdk_ch_map_descr_is_valid(None), unsafe { cref_chmap_is_valid_null() });
    for pass in [0u32, 1] {
        assert_eq!(
            fdk_ch_map_descr_set_pass_through(&mut FdkChannelMapDescr::default(), pass),
            unsafe { cref_chmap_set_pass_through_empty(pass) }
        );
    }
    for _ in 0..20000 {
        let map = random_map(&mut r);
        // numChannels is at most the length of the map.
        let num_ch = if r.next_u32() % 4 == 0 && !map.is_empty() {
            r.range(0, map.len() as i32) as u8
        } else {
            map.len() as u8
        };
        // Below 32 channels, fdk_chMapDescr_isValidMap computes `1 << pChannelMap[i]` for the
        // first numChannels entries: values of 32 or more are undefined shifts, which the port
        // reproduces as ref-arm64 and ref-x86_64 (Debian clang 19) compile them. The
        // library only validates its own tables, so these maps are unreachable.
        let map = fold_for_apple_clang_x86(map, num_ch);
        let info: &'static [ChannelMapInfo] =
            Box::leak(Box::new([ChannelMapInfo { channel_map: map, num_channels: num_ch }]));
        let d = FdkChannelMapDescr { map_info_tab: Some(info), map_info_tab_len: 1, f_pass_through: 0 };
        assert_eq!(
            fdk_ch_map_descr_is_valid(Some(&d)),
            unsafe { cref_chmap_is_valid_single(map.as_ptr(), num_ch) },
            "{map:?} {num_ch}"
        );

        let len = r.range(0, 1) as u32;
        let (pass, pass2) = (r.range(0, 2) as u32, r.range(0, 2) as u32);
        let ch = r.range(0, 45) as u8;
        let map_idx = r.range(0, 2) as u32;
        let mut d = FdkChannelMapDescr::default();
        fdk_ch_map_descr_init(&mut d, Some(info), len, pass);
        let err = fdk_ch_map_descr_set_pass_through(&mut d, pass2);
        let value = fdk_ch_map_descr_get_map_value(&d, ch, map_idx);
        let default_chosen = d.map_info_tab.map(|t| t.as_ptr()) != Some(info.as_ptr());
        let (mut c_value, mut c_err) = (0u8, 0i32);
        let c_default = unsafe {
            cref_chmap_init_custom(
                map.as_ptr(),
                num_ch,
                len,
                pass,
                pass2,
                ch,
                map_idx,
                &mut c_value,
                &mut c_err,
            )
        };
        assert_eq!((default_chosen as i32, err, value), (c_default, c_err, c_value), "{map:?}");
    }
    check_map_values_of_32_and_more();
}

/// The channel maps of syslib_channelMapDescr.cpp (file-static; a copy compiled into the shim),
/// element by element, and the entries of MAP_INFO_TAB_DFLT (`mapInfoTabDflt`) and
/// FDK_MAP_INFO_TAB_WG4: the map each entry points to (by name) and its numChannels.
#[test]
fn channel_map_tables() {
    use fdk_aac_rs_cref::fdk_syslib_channel_map_descr_tables as t;
    let maps: [(&str, &[u8]); 20] = [
        ("mapFallback", &MAP_FALLBACK),
        ("mapCfg1", &MAP_CFG1),
        ("mapCfg2", &MAP_CFG2),
        ("mapCfg3", &MAP_CFG3),
        ("mapCfg4", &MAP_CFG4),
        ("mapCfg5", &MAP_CFG5),
        ("mapCfg6", &MAP_CFG6),
        ("mapCfg7", &MAP_CFG7),
        ("mapCfg11", &MAP_CFG11),
        ("mapCfg12", &MAP_CFG12),
        ("mapCfg13", &MAP_CFG13),
        ("mapCfg14", &MAP_CFG14),
        ("mapWg4Cfg1", &MAP_WG4_CFG1),
        ("mapWg4Cfg2", &MAP_WG4_CFG2),
        ("mapWg4Cfg3", &MAP_WG4_CFG3),
        ("mapWg4Cfg4", &MAP_WG4_CFG4),
        ("mapWg4Cfg5", &MAP_WG4_CFG5),
        ("mapWg4Cfg6", &MAP_WG4_CFG6),
        ("mapWg4Cfg7", &MAP_WG4_CFG7),
        ("mapWg4Cfg14", &MAP_WG4_CFG14),
    ];
    for (name, r) in maps {
        let cname = std::ffi::CString::new(name).unwrap();
        let mut bytes = 0;
        let p = unsafe { t::cref_chmap_table(cname.as_ptr(), &mut bytes) };
        assert!(!p.is_null(), "{name}");
        assert_eq!(unsafe { core::slice::from_raw_parts(p, bytes as usize) }, r, "{name}");
    }
    for (wg4, tab) in [(0, &MAP_INFO_TAB_DFLT[..]), (1, &FDK_MAP_INFO_TAB_WG4[..])] {
        for (i, e) in tab.iter().enumerate() {
            let (mut n, mut len) = (0, 0);
            let name = unsafe { t::cref_chmap_info(wg4, i as i32, &mut n, &mut len) };
            assert_eq!(len as usize, tab.len());
            assert!(!name.is_null(), "table {wg4} entry {i}");
            let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_str().unwrap();
            let r = maps.iter().find(|m| m.0 == name).unwrap().1;
            assert!(core::ptr::eq(e.channel_map, r), "table {wg4} entry {i}: {name}");
            assert_eq!(e.num_channels as i32, n, "table {wg4} entry {i}");
        }
    }
}

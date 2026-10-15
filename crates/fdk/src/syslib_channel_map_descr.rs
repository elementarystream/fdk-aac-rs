// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSYS/include/syslib_channelMapDescr.h, libSYS/src/syslib_channelMapDescr.cpp.

//! Port of `syslib_channelMapDescr.h` and `syslib_channelMapDescr.cpp`: channel map
//! descriptors (the output channel order of the decoder and the input order of the encoder).
//!
//! `CHANNEL_MAP_INFO` keeps its channel map as a slice; `FDK_channelMapDescr` keeps the table
//! of channel maps as an optional slice (`NULL` = `None`) with its own length, as in C++. For a
//! length larger than its table the port reads nothing and behaves as described at the
//! functions.

use crate::common_fix::ARCH_ARM;

/// `CHANNEL_MAP_INFO`: information about a single channel map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelMapInfo {
    /// `pChannelMap`: the channel mapping of one configuration.
    pub channel_map: &'static [u8],
    /// `numChannels`: the number of channels of the map, i.e. the maximum channel index + 1.
    pub num_channels: u8,
}

/// `FDK_channelMapDescr`: the mapping for all channel configurations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FdkChannelMapDescr {
    /// `pMapInfoTab`: table of channel maps.
    pub map_info_tab: Option<&'static [ChannelMapInfo]>,
    /// `mapInfoTabLen`: length of the channel map table.
    pub map_info_tab_len: u32,
    /// `fPassThrough`: 0: apply the mapping, otherwise pass the input through (MPEG order).
    pub f_pass_through: u32,
}

/// `DFLT_CH_MAP_TAB_LEN`: length of the default channel map info table.
const DFLT_CH_MAP_TAB_LEN: u32 = 15;

// The following arrays provide a channel map for each channel config (0 to 14). The i-th channel
// will be mapped to the position a[i-1]+1 with i>0 and a[] is one of the following mapping
// arrays, dumped with scripts/dump_table.py (the command is given at each table).
/// `mapFallback`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapFallback UCHAR --name MAP_FALLBACK --per-line 24`
pub static MAP_FALLBACK: [u8; 24] =
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23];
/// `mapCfg1`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg1 UCHAR --name MAP_CFG1 --per-line 24`
pub static MAP_CFG1: [u8; 2] = [0, 1];
/// `mapCfg2`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg2 UCHAR --name MAP_CFG2 --per-line 24`
pub static MAP_CFG2: [u8; 2] = [0, 1];
/// `mapCfg3`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg3 UCHAR --name MAP_CFG3 --per-line 24`
pub static MAP_CFG3: [u8; 3] = [2, 0, 1];
/// `mapCfg4`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg4 UCHAR --name MAP_CFG4 --per-line 24`
pub static MAP_CFG4: [u8; 4] = [2, 0, 1, 3];
/// `mapCfg5`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg5 UCHAR --name MAP_CFG5 --per-line 24`
pub static MAP_CFG5: [u8; 5] = [2, 0, 1, 3, 4];
/// `mapCfg6`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg6 UCHAR --name MAP_CFG6 --per-line 24`
pub static MAP_CFG6: [u8; 6] = [2, 0, 1, 4, 5, 3];
/// `mapCfg7`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg7 UCHAR --name MAP_CFG7 --per-line 24`
pub static MAP_CFG7: [u8; 8] = [2, 6, 7, 0, 1, 4, 5, 3];
/// `mapCfg11`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg11 UCHAR --name MAP_CFG11 --per-line 24`
pub static MAP_CFG11: [u8; 7] = [2, 0, 1, 4, 5, 6, 3];
/// `mapCfg12`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg12 UCHAR --name MAP_CFG12 --per-line 24`
pub static MAP_CFG12: [u8; 8] = [2, 0, 1, 6, 7, 4, 5, 3];
/// `mapCfg13`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg13 UCHAR --name MAP_CFG13 --per-line 24`
pub static MAP_CFG13: [u8; 24] =
    [2, 6, 7, 0, 1, 10, 11, 4, 5, 8, 3, 9, 14, 12, 13, 18, 19, 15, 16, 17, 20, 21, 22, 23];
/// `mapCfg14`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapCfg14 UCHAR --name MAP_CFG14 --per-line 24`
pub static MAP_CFG14: [u8; 8] = [2, 0, 1, 4, 5, 3, 6, 7];
/// `mapWg4Cfg1`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg1 UCHAR --name MAP_WG4_CFG1 --per-line 24`
pub static MAP_WG4_CFG1: [u8; 2] = [0, 1];
/// `mapWg4Cfg2`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg2 UCHAR --name MAP_WG4_CFG2 --per-line 24`
pub static MAP_WG4_CFG2: [u8; 2] = [0, 1];
/// `mapWg4Cfg3`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg3 UCHAR --name MAP_WG4_CFG3 --per-line 24`
pub static MAP_WG4_CFG3: [u8; 3] = [2, 0, 1];
/// `mapWg4Cfg4`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg4 UCHAR --name MAP_WG4_CFG4 --per-line 24`
pub static MAP_WG4_CFG4: [u8; 4] = [3, 0, 1, 2];
/// `mapWg4Cfg5`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg5 UCHAR --name MAP_WG4_CFG5 --per-line 24`
pub static MAP_WG4_CFG5: [u8; 5] = [4, 0, 1, 2, 3];
/// `mapWg4Cfg6`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg6 UCHAR --name MAP_WG4_CFG6 --per-line 24`
pub static MAP_WG4_CFG6: [u8; 6] = [4, 0, 1, 2, 3, 5];
/// `mapWg4Cfg7`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg7 UCHAR --name MAP_WG4_CFG7 --per-line 24`
pub static MAP_WG4_CFG7: [u8; 8] = [6, 0, 1, 2, 3, 4, 5, 7];
/// `mapWg4Cfg14`
///
/// `scripts/dump_table.py libSYS/src/syslib_channelMapDescr.cpp mapWg4Cfg14 UCHAR --name MAP_WG4_CFG14 --per-line 24`
pub static MAP_WG4_CFG14: [u8; 8] = [6, 0, 1, 2, 3, 4, 5, 7];

const fn cmi(channel_map: &'static [u8], num_channels: u8) -> ChannelMapInfo {
    ChannelMapInfo { channel_map, num_channels }
}

/// `mapInfoTabDflt`: default table with the channel map of each channel config (0 to 14).
pub static MAP_INFO_TAB_DFLT: [ChannelMapInfo; DFLT_CH_MAP_TAB_LEN as usize] = [
    /*  0 */ cmi(&MAP_FALLBACK, 24),
    /*  1 */ cmi(&MAP_CFG1, 2),
    /*  2 */ cmi(&MAP_CFG2, 2),
    /*  3 */ cmi(&MAP_CFG3, 3),
    /*  4 */ cmi(&MAP_CFG4, 4),
    /*  5 */ cmi(&MAP_CFG5, 5),
    /*  6 */ cmi(&MAP_CFG6, 6),
    /*  7 */ cmi(&MAP_CFG7, 8),
    /*  8 */ cmi(&MAP_FALLBACK, 24),
    /*  9 */ cmi(&MAP_FALLBACK, 24),
    /* 10 */ cmi(&MAP_FALLBACK, 24),
    /* 11 */ cmi(&MAP_CFG11, 7),
    /* 12 */ cmi(&MAP_CFG12, 8),
    /* 13 */ cmi(&MAP_CFG13, 24),
    /* 14 */ cmi(&MAP_CFG14, 8),
];

/// `FDK_mapInfoTabWg4`: channel maps for the WG4 channel order.
pub static FDK_MAP_INFO_TAB_WG4: [ChannelMapInfo; 15] = [
    /*  0 */ cmi(&MAP_FALLBACK, 24),
    /*  1 */ cmi(&MAP_WG4_CFG1, 2),
    /*  2 */ cmi(&MAP_WG4_CFG2, 2),
    /*  3 */ cmi(&MAP_WG4_CFG3, 3),
    /*  4 */ cmi(&MAP_WG4_CFG4, 4),
    /*  5 */ cmi(&MAP_WG4_CFG5, 5),
    /*  6 */ cmi(&MAP_WG4_CFG6, 6),
    /*  7 */ cmi(&MAP_WG4_CFG7, 8),
    /*  8 */ cmi(&MAP_FALLBACK, 24),
    /*  9 */ cmi(&MAP_FALLBACK, 24),
    /* 10 */ cmi(&MAP_FALLBACK, 24),
    /* 11 */ cmi(&MAP_FALLBACK, 24), // Unhandled for Wg4 yet
    /* 12 */ cmi(&MAP_FALLBACK, 24), // Unhandled for Wg4 yet
    /* 13 */ cmi(&MAP_FALLBACK, 24), // Unhandled for Wg4 yet
    /* 14 */ cmi(&MAP_FALLBACK, 24), // Unhandled for Wg4 yet
];

/// `FDK_mapInfoTabLenWg4`
pub const FDK_MAP_INFO_TAB_LEN_WG4: u32 = 15;

/// `FDK_chMapDescr_getMapValue(pMapDescr, chIdx, mapIdx)`: the mapped position of channel
/// `ch_idx` in the configuration `map_idx`; `ch_idx` itself (pass through) if the descriptor
/// passes through, has no table, `map_idx` is out of range, or `ch_idx` is not below the number
/// of channels of the map.
///
/// For lengths larger than the slices the port passes `ch_idx` through.
pub fn fdk_ch_map_descr_get_map_value(
    p_map_descr: &FdkChannelMapDescr,
    ch_idx: u8,
    map_idx: u32,
) -> u8 {
    let mut map_value = ch_idx; // Pass through by default.

    if p_map_descr.f_pass_through == 0 && p_map_descr.map_info_tab_len > map_idx {
        // Nest sanity check to avoid possible memory access violation.
        if let Some(info) = p_map_descr.map_info_tab.and_then(|t| t.get(map_idx as usize)) {
            if ch_idx < info.num_channels {
                if let Some(&v) = info.channel_map.get(ch_idx as usize) {
                    map_value = v;
                }
            }
        }
    }
    map_value
}

/// `1 << value` of the `i`-th of `num_channels < 32` map values in `fdk_chMapDescr_isValidMap`
/// as the reference builds compute it. The shift is undefined for values of 32 or more, and its
/// result depends on the code clang generated (the library function and its copy inlined into
/// `FDK_chMapDescr_init`):
///
/// - `aarch64` (ref-arm64, Apple clang): for `num_channels >= 8`, the loop is vectorized for the
///   first `num_channels & 0x18` values with `ushl` (0 for values of 32 or more); the scalar code
///   uses `lsl` (amount modulo 32);
/// - `x86` (ref-x86_64, Debian clang 19) and `x86_32` (ref-i686): scalar `bts` for every value
///   (`FDK_chMapDescr_isValid` 0x111-0x16f and `FDK_chMapDescr_init` 0x281-0x2df in ref-x86_64),
///   i.e. the amount modulo 32.
///
/// The `generic` variant uses the scalar form (its code generation is not known). Apple clang
/// for x86-64 (ref-x86_64-mac) vectorizes the first `num_channels & 0x18` values instead, as a
/// `float` 2^value converted with `cvttps2dq` (`0x80000000` for values from 31 to 128); the port
/// does not reproduce that build.
#[inline]
fn channel_bit(i: u32, num_channels: u32, value: u8) -> u32 {
    let vectorized = num_channels >= 8 && i < (num_channels & 0x18);
    if vectorized && ARCH_ARM {
        if value < 32 {
            1 << value
        } else {
            0
        }
    } else {
        1u32.wrapping_shl(value as u32)
    }
}

/// `fdk_chMapDescr_isValidMap(pMapInfo)` (file-static): non-zero if all map values are inside
/// the range 0 to `numChannels - 1` and unique.
///
/// For fewer than 32 channels, the C++ code sets the bit `1 << value` of every value; values of
/// 32 or more are undefined shifts, see [`channel_bit`]. Channel map entries beyond the slice are
/// read as 0.
fn fdk_ch_map_descr_is_valid_map(p_map_info: Option<&ChannelMapInfo>) -> i32 {
    let mut result = 1;

    let Some(map_info) = p_map_info else {
        return 0;
    };
    let num_channels = map_info.num_channels as u32;
    let map = |i: u32| map_info.channel_map.get(i as usize).copied().unwrap_or(0);

    // Check for all map values if they are inside the range 0 to numChannels-1 and unique.
    if num_channels < 32 {
        // Optimized version for less than 32 channels. Needs only one loop.
        let mut mapped_ch_mask = 0u32;
        for i in 0..num_channels {
            mapped_ch_mask |= channel_bit(i, num_channels, map(i));
        }
        if mapped_ch_mask != (1u32 << num_channels) - 1 {
            result = 0;
        }
    } else {
        // General case that can handle all number of channels but needs one more loop.
        let mut i = 0;
        while i < num_channels && result != 0 {
            let value0 = map(i);
            if value0 as u32 > num_channels - 1 {
                // out of range?
                result = 0;
            }
            let mut j = num_channels - 1;
            while j > i && result != 0 {
                if value0 == map(j) {
                    // not unique
                    result = 0;
                }
                j -= 1;
            }
            i += 1;
        }
    }

    result
}

/// `FDK_chMapDescr_isValid(pMapDescr)`: non-zero if all channel maps of the descriptor are
/// valid. A table entry beyond the slice (a length larger than the table) counts as invalid.
pub fn fdk_ch_map_descr_is_valid(p_map_descr: Option<&FdkChannelMapDescr>) -> i32 {
    let result = 0;

    if let Some(d) = p_map_descr {
        // The tables are immutable statics, so the result for the default table (checked by
        // the decoder before every frame) is computed once by this very function.
        if d.map_info_tab_len == DFLT_CH_MAP_TAB_LEN
            && d.map_info_tab.is_some_and(|t| core::ptr::eq(t, &MAP_INFO_TAB_DFLT[..]))
        {
            static DFLT_VALID: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
            return *DFLT_VALID.get_or_init(|| {
                let dflt = FdkChannelMapDescr {
                    map_info_tab: Some(&MAP_INFO_TAB_DFLT[..]),
                    map_info_tab_len: DFLT_CH_MAP_TAB_LEN,
                    f_pass_through: 0,
                };
                fdk_ch_map_descr_is_valid_uncached(&dflt)
            });
        }
        return fdk_ch_map_descr_is_valid_uncached(d);
    }
    result
}

/// [`fdk_ch_map_descr_is_valid`] without the cache of the default table.
fn fdk_ch_map_descr_is_valid_uncached(d: &FdkChannelMapDescr) -> i32 {
    let mut result;
    {
        result = 1;
        let mut i = 0;
        while i < d.map_info_tab_len && result != 0 {
            let info = d.map_info_tab.and_then(|t| t.get(i as usize));
            if fdk_ch_map_descr_is_valid_map(info) == 0 {
                result = 0;
            }
            i += 1;
        }
    }
    result
}

/// `FDK_chMapDescr_init(pMapDescr, pMapInfoTab, mapInfoTabLen, fPassThrough)`: initializes the
/// descriptor with the given table, or with the default table (WAV-like order) if no table is
/// given, its length is 0, or it is not valid.
pub fn fdk_ch_map_descr_init(
    p_map_descr: &mut FdkChannelMapDescr,
    p_map_info_tab: Option<&'static [ChannelMapInfo]>,
    map_info_tab_len: u32,
    f_pass_through: u32,
) {
    let mut use_default_tab = 1;

    p_map_descr.f_pass_through = if f_pass_through == 0 { 0 } else { 1 };

    if p_map_info_tab.is_some() && map_info_tab_len > 0 {
        // Set the valid custom mapping table.
        p_map_descr.map_info_tab = p_map_info_tab;
        p_map_descr.map_info_tab_len = map_info_tab_len;
        // Validate the complete descriptor.
        use_default_tab = if fdk_ch_map_descr_is_valid(Some(p_map_descr)) == 0 { 1 } else { 0 };
    }
    if use_default_tab != 0 {
        // Set default table.
        p_map_descr.map_info_tab = Some(&MAP_INFO_TAB_DFLT);
        p_map_descr.map_info_tab_len = DFLT_CH_MAP_TAB_LEN;
    }
}

/// `FDK_chMapDescr_setPassThrough(pMapDescr, fPassThrough)`: sets the pass-through flag of an
/// initialized descriptor. Returns 0 on success, non-zero if the descriptor has no table.
pub fn fdk_ch_map_descr_set_pass_through(
    p_map_descr: &mut FdkChannelMapDescr,
    f_pass_through: u32,
) -> i32 {
    let mut err = 1;

    if p_map_descr.map_info_tab.is_some() && p_map_descr.map_info_tab_len > 0 {
        p_map_descr.f_pass_through = if f_pass_through == 0 { 0 } else { 1 };
        err = 0;
    }

    err
}

//! Builds seed inputs in the formats of src/dec.rs and src/enc.rs (used by fuzz/seeds.sh).
//!
//! ```text
//! seedgen stream  TARGET OUT_DIR MAX_BYTES FILE...           stream-mode seeds (the first MAX_BYTES bytes)
//! seedgen packets TARGET OUT_DIR MAX_BYTES CONF_HEX|- SIZES FILE
//!                                                            packet-mode seeds from a stream and its
//!                                                            access-unit sizes (fdkenc-ref --au-sizes);
//!                                                            TARGET decode_raw_mcp0: decode_raw, LATM MCP0
//! seedgen mp4     OUT_DIR MAX_BYTES FILE...                  decode_raw seeds (ASC + access units) from MP4
//! seedgen drm     OUT_DIR MAX_BYTES FILE...                  decode_drm seeds: SDC configurations with
//!                                                            ER AAC LC / USAC access units of MP4 files
//! seedgen enc     OUT_DIR                                    encode seeds: parameter programs
//! ```
//!
//! Each seed is written as OUT_DIR/<hash of the content>.

#[path = "../../../crates/cli/src/harness/mp4demux.rs"]
#[allow(dead_code)]
mod mp4demux;

use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_fuzz::dec::{DecProgram, DecTarget};
use fdk_aac_rs_fuzz::enc::{param_value, ENC_PARAMS, MAX_CHANNELS};
use std::path::Path;

/// Header flags of packet seeds: packet mode, one DecodeFrame per access unit (as applications
/// and the harness's packet mode).
const PACKET: u8 = 1 | 4;

fn write_seed(dir: &Path, data: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    let name = format!("{:016x}", fdk_aac_rs_fuzz::hash(data));
    std::fs::write(dir.join(name), data).unwrap();
}

fn program(t: DecTarget, config: Option<Vec<u8>>, chunks: Vec<Vec<u8>>) -> DecProgram {
    let mut p = DecProgram::parse(t, &[]);
    p.config = config;
    p.chunks = chunks;
    p
}

/// Access units from the start of `aus` up to `max` bytes in total (at least one).
fn take_aus(aus: Vec<Vec<u8>>, max: usize) -> Vec<Vec<u8>> {
    let mut total = 0;
    let mut v = Vec::new();
    for au in aus {
        total += au.len() + 2;
        if total > max && !v.is_empty() {
            break;
        }
        v.push(au);
    }
    v
}

fn demux(path: &str) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let data = std::fs::read(path).ok()?;
    let t = mp4demux::mp4_demux(&data, 0).ok()?;
    let asc = t.asc(&data).to_vec();
    let aus = t
        .samples
        .iter()
        .filter_map(|s| {
            data.get(s.offset as usize..s.offset as usize + s.size as usize).map(|b| b.to_vec())
        })
        .collect();
    Some((asc, aus))
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

/// The audio object type of an AudioSpecificConfig.
fn asc_aot(asc: &[u8]) -> u32 {
    if asc.len() < 2 {
        return 0;
    }
    let aot = (asc[0] >> 3) as u32;
    if aot == 31 {
        32 + ((((asc[0] & 7) as u32) << 3) | (asc[1] >> 5) as u32)
    } else {
        aot
    }
}

/// An op byte pair setting `param` to `value` (encode target), if the value tables have it.
fn enc_set(param: i32, value: u32, rate: u32) -> Option<[u8; 2]> {
    let idx = ENC_PARAMS.iter().position(|&p| p == param)? as u8;
    (0u8..0xF7).find(|&b| param_value(param, b, rate, &mut || 0) == value).map(|b| [idx, b])
}

/// (aot, sbr mode, channel mode, channels, rate, bitrate, frame length)
type EncConfig = (u32, Option<u32>, u32, u32, u32, u32, u32);

fn enc_seeds(out: &Path) {
    // (aot, sbr mode, channel mode, channels, rate, bitrate, frame length)
    let configs: &[EncConfig] = &[
        (2, None, 1, 1, 44100, 64000, 1024),
        (2, None, 2, 2, 48000, 128000, 1024),
        (2, None, 6, 6, 48000, 320000, 1024),
        (2, None, 7, 8, 48000, 512000, 1024),
        (5, None, 2, 2, 44100, 64000, 1024),
        (5, None, 1, 1, 32000, 32000, 1024),
        (29, None, 2, 2, 48000, 32000, 1024),
        (23, None, 2, 2, 48000, 128000, 512),
        (23, None, 1, 1, 32000, 48000, 480),
        (39, Some(0), 2, 2, 48000, 96000, 512),
        (39, Some(1), 2, 2, 48000, 64000, 512),
        (39, None, 128, 2, 48000, 64000, 512),
        (39, Some(0), 1, 1, 16000, 24000, 256),
        (129, None, 1, 1, 32000, 48000, 1024),
        (132, None, 2, 2, 44100, 64000, 1024),
        (2, None, 2, 2, 8000, 16000, 1024),
        (2, None, 2, 2, 96000, 256000, 1024),
    ];
    let transports = [0u32, 1, 2, 6, 7, 10];
    let mut k = 0usize;
    for &(aot, sbr, mode, ch, rate, br, fl) in configs {
        for &tt in &transports {
            for variant in 0..3 {
                k += 1;
                let mut ops = Vec::new();
                let mut set = |p: i32, v: u32| {
                    if let Some(b) = enc_set(p, v, rate) {
                        ops.extend_from_slice(&[b[0], b[1]]);
                    }
                };
                set(AACENC_AOT, aot);
                if let Some(s) = sbr {
                    set(AACENC_SBR_MODE, s);
                }
                set(AACENC_SAMPLERATE, rate);
                set(AACENC_CHANNELMODE, mode);
                if aot == 23 || aot == 39 {
                    set(AACENC_GRANULE_LENGTH, fl);
                }
                match variant {
                    0 => set(AACENC_BITRATE, br),
                    1 => {
                        set(AACENC_BITRATEMODE, 3);
                        set(AACENC_AFTERBURNER, 1);
                    }
                    _ => {
                        set(AACENC_BITRATE, br);
                        set(AACENC_PROTECTION, 1);
                        set(AACENC_SIGNALING_MODE, (k % 3) as u32);
                    }
                }
                set(AACENC_TRANSMUX, tt);
                ops.push(0x40); // initialization
                ops.push(0x60 | (k % 7) as u8); // PCM style
                ops.push(0xA0 | 3); // 4 frames
                if variant == 2 {
                    ops.extend_from_slice(&[0xC1, 0x13, 0x22, 0x31, 0x44, 0x05, 0x16, 0x47, 0x58]); // metadata
                    ops.extend_from_slice(&[0xE0, 0x05]); // ancillary data
                    ops.extend_from_slice(&[0x20 | 24, 0x00, 0x00]); // 1024 samples per channel
                    ops.push(0xA1);
                }
                let maxch = MAX_CHANNELS.iter().position(|&m| m == ch).unwrap_or(0) as u8;
                let mut data = vec![0u8, maxch, ops.len() as u8];
                data.extend_from_slice(&ops);
                // PCM tape: a tone with noise
                let mut x = 0x1234_5678u32 ^ k as u32;
                for i in 0..2048 {
                    x = x.wrapping_mul(1103515245).wrapping_add(12345);
                    let s = ((i * (37 + 11 * (k % 5))) % 400) as i32 * 60 - 12000
                        + ((x >> 16) & 1023) as i32;
                    data.extend_from_slice(&(s as i16).to_le_bytes());
                }
                write_seed(out, &data);
            }
        }
    }
    // Random parameter sequences: one op of every kind.
    let mut x = 0x9E37_79B9u32;
    for _ in 0..64 {
        let mut data = vec![0u8, 2, 48];
        for _ in 0..48 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            data.push(x as u8);
        }
        write_seed(out, &data);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || -> ! {
        eprintln!("see the documentation at the top of fuzz/src/bin/seedgen.rs");
        std::process::exit(2)
    };
    if args.is_empty() {
        usage();
    }
    match args[0].as_str() {
        "stream" if args.len() >= 5 => {
            let t = DecTarget::from_name(&args[1]).unwrap_or_else(|| usage());
            let out = Path::new(&args[2]);
            let max: usize = args[3].parse().unwrap();
            for f in &args[4..] {
                let Ok(d) = std::fs::read(f) else { continue };
                let p = program(t, None, vec![d[..d.len().min(max)].to_vec()]);
                // ADIF: one decode call per fill (PORTING.md, "Reference harness")
                let flags = if t == DecTarget::Adif { 4 } else { 0 };
                write_seed(out, &p.to_bytes(t, flags, 0));
            }
        }
        "packets" if args.len() == 7 => {
            // decode_raw_mcp0: decode_raw with LATM MCP0 (flag bit 7)
            let mcp0 = args[1] == "decode_raw_mcp0";
            let t = if mcp0 {
                DecTarget::Raw
            } else {
                DecTarget::from_name(&args[1]).unwrap_or_else(|| usage())
            };
            let out = Path::new(&args[2]);
            let max: usize = args[3].parse().unwrap();
            let conf = (args[4] != "-").then(|| hex(&args[4]));
            let sizes: Vec<usize> = std::fs::read_to_string(&args[5])
                .unwrap()
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .collect();
            let data = std::fs::read(&args[6]).unwrap();
            let mut aus = Vec::new();
            let mut pos = 0;
            for s in sizes {
                if pos + s > data.len() {
                    break;
                }
                aus.push(data[pos..pos + s].to_vec());
                pos += s;
            }
            let p = program(t, conf, take_aus(aus, max));
            write_seed(out, &p.to_bytes(t, PACKET | if mcp0 { 0x80 } else { 0 }, 0));
        }
        "mp4" if args.len() >= 4 => {
            let out = Path::new(&args[1]);
            let max: usize = args[2].parse().unwrap();
            for f in &args[3..] {
                let Some((asc, aus)) = demux(f) else { continue };
                if asc.len() > 255 {
                    continue;
                }
                let p = program(DecTarget::Raw, Some(asc), take_aus(aus, max));
                write_seed(out, &p.to_bytes(DecTarget::Raw, PACKET, 0));
            }
        }
        "drm" if args.len() >= 3 => {
            let out = Path::new(&args[1]);
            let max: usize = args[2].parse().unwrap();
            // SDC audio information: coding (2) sbr (1) mode (2) rate (3) | text, enh (2)
            // coder field (5) rfa (1)
            let mut aac = Vec::new();
            for rate in [1u8, 2, 3, 5] {
                for (sbr, mode) in [(0u8, 0u8), (0, 2), (1, 0), (1, 1), (1, 2)] {
                    for coder in [0u8, 4] {
                        aac.push(vec![(sbr << 5) | (mode << 3) | rate, coder << 1]);
                    }
                }
            }
            let mut usac = Vec::new();
            for rate in 0u8..8 {
                for mode in [0u8, 2] {
                    usac.push(vec![(3 << 6) | (mode << 3) | rate, 0]);
                }
            }
            // Random access units with every configuration.
            let mut x = 0x2468_ACE1u32;
            for c in aac.iter().chain(&usac) {
                let mut aus = Vec::new();
                for _ in 0..4 {
                    let n = 40 + (x % 300) as usize;
                    let au: Vec<u8> = (0..n)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 17;
                            x ^= x << 5;
                            x as u8
                        })
                        .collect();
                    aus.push(au);
                }
                write_seed(
                    out,
                    &program(DecTarget::Drm, Some(c.clone()), aus).to_bytes(
                        DecTarget::Drm,
                        PACKET,
                        0,
                    ),
                );
            }
            // Access units of ER AAC LC (AOT 17) and USAC (AOT 42) streams.
            for (i, f) in args[3..].iter().enumerate() {
                let Some((asc, aus)) = demux(f) else { continue };
                let aot = asc_aot(&asc);
                let conf = match aot {
                    17 => aac[i % aac.len()].clone(),
                    42 => {
                        // xHE-AAC: the header, then the configuration bytes of the ASC after
                        // its first two bytes (approximate; the fuzzer adapts them)
                        let mut c = usac[i % usac.len()].clone();
                        c.extend_from_slice(asc.get(2..).unwrap_or(&[]));
                        c
                    }
                    _ => continue,
                };
                if conf.len() > 255 {
                    continue;
                }
                write_seed(
                    out,
                    &program(DecTarget::Drm, Some(conf), take_aus(aus, max)).to_bytes(
                        DecTarget::Drm,
                        PACKET,
                        0,
                    ),
                );
            }
        }
        "enc" if args.len() == 2 => enc_seeds(Path::new(&args[1])),
        _ => usage(),
    }
}

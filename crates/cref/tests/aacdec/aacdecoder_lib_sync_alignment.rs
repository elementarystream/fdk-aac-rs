// Differential tests of the decoder (through its public API, see aacdecoder_lib.rs) for the
// synchronization of ADTS and LOAS streams.
//
// synchronization() (tpdec_lib.cpp), used only for ADTS and LOAS, asserts
// `bitsAvail % TPDEC_SYNCSKIP == 0` before it searches a sync word. The assertion is compiled out
// in the reference, which then searches in steps of 8 bits from the unaligned position. The
// transport decoder tests (tpdec_lib.rs, stream_params) reach it through the burst buffer
// fullness check: additionalHoldOffNeeded() returns TRANSPORTDEC_SYNC_ERROR after a LOAS header
// (of any bit length) was parsed, and only TRANSPORTDEC_NOT_ENOUGH_BITS rewinds the header bits.
// That needs TPDEC_PARAM_BURST_PERIOD and TPDEC_PARAM_SET_BITRATE set and
// TPDEC_PARAM_IGNORE_BUFFERFULLNESS cleared; the AAC decoder never sets the first two
// (aacDecoder_Open() sets only TPDEC_PARAM_IGNORE_BUFFERFULLNESS, never cleared, and
// TPDEC_PARAM_RESET), so the check returns at its first line, and it never sets
// TPDEC_PARAM_MINIMIZE_DELAY or TPDEC_PARAM_EARLY_CONFIG (frame traversal).
//
// These tests decode ADTS and LOAS streams through the decoder with several fill sizes, decode
// flags (concealment, flush, interruption), AAC_TPDEC_CLEAR_BUFFER and mutations (bit flips,
// dropped, inserted and duplicated byte ranges, truncation), including ADTS frames whose header
// carries a program config element (channel configuration 0, with and without CRC), compare
// every call with the reference, and require that the port never searched from an unaligned
// position (synchronization_unaligned_searches()). A nonzero count would be a decoder path to an
// unaligned search that these tests do not know yet; the comparison with the reference still
// covers it.

use crate::aacdecoder_lib::{encode, read, run_stream, Pair, AOT, BITRATE, TRANSMUX};
use fdk_aac_rs_aacdec::aacdecoder_lib::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::{TT_MP4_ADTS, TT_MP4_LOAS};
use fdk_aac_rs_tpdec::tpdec_lib::synchronization_unaligned_searches;

const PROTECTION: i32 = 0x0306;
const HEADER_PERIOD: i32 = 0x0301;
const META_MODE: i32 = 0x0600;

/// Mutates a stream: bit flips, dropped, inserted and duplicated byte ranges, truncation.
fn mutate(data: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut d = data.to_vec();
    for _ in 0..1 + rng.next_u32() % 6 {
        if d.len() < 16 {
            break;
        }
        let pos = (rng.next_u32() as usize) % d.len();
        match rng.next_u32() % 5 {
            0 => d[pos] ^= 1 << (rng.next_u32() % 8),
            1 => {
                let n = (1 + rng.next_u32() as usize % 40).min(d.len() - pos);
                d.drain(pos..pos + n);
            }
            2 => {
                let n = 1 + rng.next_u32() as usize % 40;
                let ins: Vec<u8> = (0..n).map(|_| rng.next_u32() as u8).collect();
                d.splice(pos..pos, ins);
            }
            3 => {
                let n = (1 + rng.next_u32() as usize % 300).min(d.len() - pos);
                let dup = d[pos..pos + n].to_vec();
                let at = (rng.next_u32() as usize) % d.len();
                d.splice(at..at, dup);
            }
            _ => {
                let keep = d.len() - (rng.next_u32() as usize) % (d.len() / 4);
                d.truncate(keep);
            }
        }
    }
    d
}

/// Decode flags: mostly 0, sometimes concealment, flush or an interruption.
fn flags(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 16 {
        0 => AACDEC_CONCEAL,
        1 => AACDEC_FLUSH,
        2 => AACDEC_INTR,
        _ => 0,
    }
}

/// Sets channel_configuration to 0 in every ADTS header of a stream (frames found through
/// aac_frame_length); returns the number of headers changed.
fn adts_channel_config_zero(d: &mut [u8]) -> usize {
    let (mut pos, mut n) = (0, 0);
    while pos + 7 <= d.len() && d[pos] == 0xff && d[pos + 1] & 0xf0 == 0xf0 {
        d[pos + 2] &= !1;
        d[pos + 3] &= 0x3f;
        n += 1;
        let len = ((d[pos + 3] as usize & 3) << 11) | ((d[pos + 4] as usize) << 3) | (d[pos + 5] as usize >> 5);
        if len < 7 {
            break;
        }
        pos += len;
    }
    n
}

#[test]
fn adts_pce_in_header() {
    // The reference encoder writes a PCE (with the matrix mixdown index) as the first element of
    // the raw data blocks of 5.0 and 5.1 ADTS streams with metadata; with channel_configuration
    // set to 0 in the headers, the decoder's ADTS parser reads it as part of the header
    // (num_pce_bits). The headers with CRC then fail their check. The original streams are
    // decoded as well.
    let meta = [1, 3, -80, -80, 1, 108, 1, 1, 2, 3, 0, 1];
    let cases: Vec<(&str, i32, Vec<(i32, i32)>)> = vec![
        ("5.1", 6, vec![(AOT, 2), (BITRATE, 256000), (TRANSMUX, 2), (META_MODE, 1)]),
        ("5.1 crc", 6, vec![(AOT, 2), (BITRATE, 256000), (TRANSMUX, 2), (META_MODE, 1), (PROTECTION, 1)]),
        ("5.0", 5, vec![(AOT, 2), (BITRATE, 224000), (TRANSMUX, 2), (META_MODE, 3), (HEADER_PERIOD, 1)]),
        ("5.1 he", 6, vec![(AOT, 5), (BITRATE, 160000), (TRANSMUX, 2), (META_MODE, 2), (HEADER_PERIOD, 3)]),
    ];
    let (mut runs, mut unaligned, mut headers, mut valid_cc0) = (0, 0u64, 0, 0);
    let mut rng = Rng::new(0x1414);
    for (name, ch, params) in &cases {
        let Some((_, aus)) = encode(*ch, 48000, params, Some(meta), false, 40) else {
            eprintln!("adts_pce_in_header: {name}: encoder failed");
            continue;
        };
        let original: Vec<u8> = aus.concat();
        let mut zero = original.clone();
        headers += adts_channel_config_zero(&mut zero);
        for (variant, data) in [("original", &original), ("channel configuration 0", &zero)] {
            for fill in [&[4096u32][..], &[1000, 3], &[333]] {
                for k in 0..3 {
                    let d = if k == 0 { data.clone() } else { mutate(data, &mut rng) };
                    let before = synchronization_unaligned_searches();
                    let ctx = format!("adts pce {name}, {variant}, fill {fill:?}, mutation {k}");
                    let mut pair = Pair::open(TT_MP4_ADTS, 1, &ctx).unwrap();
                    let mut frng = Rng::new(runs as u64);
                    run_stream(&mut pair, &d, fill, 300, false, |_| if k == 0 { 0 } else { flags(&mut frng) });
                    unaligned += synchronization_unaligned_searches() - before;
                    if k == 0 && data == &zero {
                        valid_cc0 += pair.n_valid;
                    }
                    runs += 1;
                }
            }
        }
    }
    eprintln!("adts_pce_in_header: {runs} runs, {headers} headers set to channel configuration 0 ({valid_cc0} valid frames from them unmutated), {unaligned} unaligned sync searches");
    assert!(runs > 0 && headers > 0 && valid_cc0 > 0);
    assert_eq!(unaligned, 0, "unaligned sync searches through the decoder");
}

#[test]
fn corrupt_adts_loas_streams() {
    let mut streams: Vec<(String, i32, Vec<u8>)> = Vec::new();
    for (rel, tt) in [
        ("conformance/14496-26/compressedAdts/add-opt/al18_08.adts", TT_MP4_ADTS),
        ("conformance/14496-26/compressedAdts/add-opt/al18_96.adts", TT_MP4_ADTS),
        ("conformance/14496-26/compressedAss/add-opt/al04_44_amv0.ass", TT_MP4_LOAS),
        ("conformance/14496-26/compressedAss/add-opt/al06_48_amv1.ass", TT_MP4_LOAS),
        ("conformance/14496-26/compressedAss/add-opt/er_al10_08_ep0_amv0.ass", TT_MP4_LOAS),
        ("rw/ffaac_lc128.aac", TT_MP4_ADTS),
        ("rw/apple_he64.aac", TT_MP4_ADTS),
    ] {
        if let Some(d) = read(rel) {
            streams.push((rel.to_string(), tt, d[..d.len().min(60000)].to_vec()));
        }
    }
    for (name, ch, params, tt) in [
        ("lc adts", 2, vec![(AOT, 2), (BITRATE, 96000), (TRANSMUX, 2)], TT_MP4_ADTS),
        ("lc adts crc", 2, vec![(AOT, 2), (BITRATE, 96000), (TRANSMUX, 2), (PROTECTION, 1)], TT_MP4_ADTS),
        ("he loas", 2, vec![(AOT, 5), (BITRATE, 48000), (TRANSMUX, 10)], TT_MP4_LOAS),
        ("hev2 loas", 2, vec![(AOT, 29), (BITRATE, 32000), (TRANSMUX, 10)], TT_MP4_LOAS),
        ("eld loas", 1, vec![(AOT, 39), (BITRATE, 48000), (TRANSMUX, 10)], TT_MP4_LOAS),
        ("5.1 adts pce", 6, vec![(AOT, 2), (BITRATE, 256000), (TRANSMUX, 2), (META_MODE, 1)], TT_MP4_ADTS),
    ] {
        if let Some((_, aus)) = encode(ch, 44100, &params, None, false, 40) {
            streams.push((name.to_string(), tt, aus.concat()));
        }
    }
    let mut rng = Rng::new(0x1415);
    let (mut runs, mut unaligned) = (0, 0u64);
    let mut per_tt = [0u64; 2];
    for it in 0..(24 * streams.len()) {
        let (name, tt, data) = &streams[it % streams.len()];
        let d = mutate(data, &mut rng);
        let fill: Vec<u32> = (0..3).map(|_| [1, 7, 100, 777, 4096][(rng.next_u32() % 5) as usize]).collect();
        let before = synchronization_unaligned_searches();
        let mut pair = Pair::open(*tt, 1, &format!("corrupt {name}, iteration {it}")).unwrap();
        let mut frng = Rng::new(0x9000 + it as u64);
        run_stream(&mut pair, &d, &fill, 250, rng.next_u32() % 2 == 0, |_| flags(&mut frng));
        if rng.next_u32() % 3 == 0 {
            // AAC_TPDEC_CLEAR_BUFFER (TPDEC_PARAM_RESET), then the stream again from an odd
            // offset
            pair.set_param(AAC_TPDEC_CLEAR_BUFFER, 1);
            let off = (rng.next_u32() as usize) % (d.len() / 2 + 1);
            run_stream(&mut pair, &d[off..], &fill, 400, false, |_| flags(&mut frng));
        }
        let n = synchronization_unaligned_searches() - before;
        unaligned += n;
        per_tt[(*tt == TT_MP4_LOAS) as usize] += n;
        runs += 1;
    }
    eprintln!(
        "corrupt_adts_loas_streams: {runs} runs over {} streams, {unaligned} unaligned sync searches (ADTS {}, LOAS {})",
        streams.len(),
        per_tt[0],
        per_tt[1]
    );
    assert!(runs > 0);
    assert_eq!(unaligned, 0, "unaligned sync searches through the decoder");
}

// Differential tests of fdk_aac_rs_aacdec::rvlc against the C++ reference.
//
// The RVLC data are built by an RVLC encoder (scale factors, noise energies and intensity
// positions with escapes, coded with the codeword tables derived from the decoding trees) and then
// corrupted (single and multiple bit flips, wrong lengths, random data), in sequences of frames on
// one element, so that every concealment strategy is reached with the previous frame's state:
// CRvlc_Read(), CRvlc_Decode() and CRvlc_ElementCheck() are compared after every call, and the
// steps of CRvlc_Decode() one by one. aacdec_hcr_stream.rs replays the calls of the reference
// decoder on real ER streams.

use super::aacdec_hcr::*;
use fdk_aac_rs_aacdec::aac_rom::{A_HUFF_TREE_RVLC_ESCAPE, A_HUFF_TREE_RVL_CODEWDS};
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_aacdec::rvlc::*;
use fdk_aac_rs_aacdec::rvlc_info::*;
use fdk_aac_rs_cref::aacdec_rvlc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::AC_ER_RVLC;
use fdk_aac_rs_fdk::mdct::BLOCK_SHORT;

// ---------------------------------------------------------------------------------------------
// Calls on both sides
// ---------------------------------------------------------------------------------------------

pub fn call_crvlc_read(what: &str, flat: &Flat, bs: &Bs, ch: usize) -> (Flat, Bs) {
    let (f, b, _) = both(
        what,
        flat,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_rvlc_CRvlc_Read(ctx, ch as i32, st, buf) };
            0
        },
        |rs, b| {
            crvlc_read(&mut rs.ch[ch], b.unwrap(), &mut rs.com);
            0
        },
    );
    (f, b.unwrap())
}

pub fn call_crvlc_decode(what: &str, flat: &Flat, bs: &Bs, ch: usize) -> (Flat, Bs) {
    let (f, b, _) = both(
        what,
        flat,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_rvlc_CRvlc_Decode(ctx, ch as i32, st, buf) };
            0
        },
        |rs, b| {
            crvlc_decode(&mut rs.ch[ch], &mut rs.st[ch], b.unwrap(), &mut rs.com);
            0
        },
    );
    (f, b.unwrap())
}

pub fn call_crvlc_element_check(what: &str, flat: &Flat, with_static: bool, flags: u32, el: i32) -> Flat {
    both(
        what,
        flat,
        None,
        |ctx, _| {
            unsafe { c::cref_rvlc_CRvlc_ElementCheck(ctx, with_static as i32, flags, el) };
            0
        },
        |rs, _| {
            let Rs { ch, st, com, .. } = rs;
            let (c0, c1) = ch.split_at_mut(1);
            let (s0, s1) = st.split_at_mut(1);
            let mut chs = [&mut c0[0], &mut c1[0]];
            let mut sts = [&mut s0[0], &mut s1[0]];
            crvlc_element_check(&mut chs, if with_static { Some(&mut sts) } else { None }, flags, el, com);
            0
        },
    )
    .0
}

/// The steps of CRvlc_Decode() one by one, each on both sides from the C++ state after the
/// previous step (`flat` and `bs` are the state before CRvlc_Decode()).
pub fn crvlc_decode_steps(what: &str, flat: &Flat, bs: &Bs, ch: usize) {
    let chi = ch as i32;
    let (f, b, _) = both(
        &format!("{what}: rvlcInit"),
        flat,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_rvlc_rvlcInit(ctx, chi, st, buf) };
            0
        },
        |rs, b| {
            let a = &mut rs.com.overlay.aac;
            rvlc_init(
                &mut a.er_rvlc_info,
                &mut rs.ch[ch],
                b.unwrap(),
                &mut a.a_rvlc_scf_esc,
                &mut a.a_rvlc_scf_fwd,
                &mut a.a_rvlc_scf_bwd,
            );
            0
        },
    );
    let mut f = f;
    let mut b = b.unwrap();
    if f.clone().to_rs().com.overlay.aac.er_rvlc_info.sf_escapes_present != 0 {
        let (f2, b2, _) = both(
            &format!("{what}: rvlcDecodeEscapes"),
            &f,
            Some(&b),
            |ctx, b| {
                let (st, buf) = b.unwrap().c();
                unsafe { c::cref_rvlc_rvlcDecodeEscapes(ctx, st, buf) };
                0
            },
            |rs, b| {
                let a = &mut rs.com.overlay.aac;
                rvlc_decode_escapes(&mut a.er_rvlc_info, &mut a.a_rvlc_scf_esc, b.unwrap());
                0
            },
        );
        f = f2;
        b = b2.unwrap();
    }
    let (f, b, _) = both(
        &format!("{what}: rvlcDecodeForward"),
        &f,
        Some(&b),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_rvlc_rvlcDecodeForward(ctx, chi, st, buf) };
            0
        },
        |rs, b| {
            let a = &mut rs.com.overlay.aac;
            rvlc_decode_forward(&mut a.er_rvlc_info, &mut rs.ch[ch], b.unwrap(), &mut a.a_rvlc_scf_fwd, &a.a_rvlc_scf_esc);
            0
        },
    );
    let (f, _, _) = both(
        &format!("{what}: rvlcDecodeBackward"),
        &f,
        Some(&b.unwrap()),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_rvlc_rvlcDecodeBackward(ctx, chi, st, buf) };
            0
        },
        |rs, b| {
            let a = &mut rs.com.overlay.aac;
            rvlc_decode_backward(&mut a.er_rvlc_info, &mut rs.ch[ch], b.unwrap(), &mut a.a_rvlc_scf_bwd, &a.a_rvlc_scf_esc);
            0
        },
    );
    both(
        &format!("{what}: rvlcFinalErrorDetection"),
        &f,
        None,
        |ctx, _| {
            unsafe { c::cref_rvlc_rvlcFinalErrorDetection(ctx, chi) };
            0
        },
        |rs, _| {
            let Rs { ch: chs, st, com, .. } = rs;
            rvlc_final_error_detection(&mut chs[ch], &mut st[ch], com);
            0
        },
    );
}

// ---------------------------------------------------------------------------------------------
// RVLC encoder
// ---------------------------------------------------------------------------------------------

/// The codewords of a decoding tree: `(value, bits)` for every leaf, bits MSB first.
fn codewords(tree: &[u32]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    fn walk(tree: &[u32], node: usize, prefix: &mut Vec<u8>, out: &mut Vec<Vec<u8>>) {
        for bit in 0..2u8 {
            let br = if bit == 0 { (tree[node] & 0xFFF000) >> 12 } else { tree[node] & 0xFFF };
            prefix.push(bit);
            if br & 0x400 != 0 {
                let v = (br & 0x3FF) as usize;
                if out.len() <= v {
                    out.resize(v + 1, Vec::new());
                }
                out[v] = prefix.clone();
            } else {
                walk(tree, (br & 0x3FF) as usize, prefix, out);
            }
            prefix.pop();
        }
    }
    walk(tree, 0, &mut Vec::new(), &mut out);
    out
}

struct BitWriter {
    bits: Vec<u8>,
}

impl BitWriter {
    fn put(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.bits.push(((v >> i) & 1) as u8);
        }
    }
    fn put_bits(&mut self, b: &[u8]) {
        self.bits.extend_from_slice(b);
    }
}

/// An RVLC frame: the ESC1 side info and the ESC2 data (RVL codewords, escapes), as bits.
struct RvlcFrame {
    esc1: Vec<u8>,
    rvl: Vec<u8>,
    esc: Vec<u8>,
}

/// Encodes random scale factors, noise energies and intensity positions for the codebooks of
/// `p` (the RVLC side info of `CRvlc_Read()` and the data of `CRvlc_Decode()`).
fn encode_rvlc(rng: &mut Rng, p: &CAacDecoderChannelInfo) -> Option<RvlcFrame> {
    let rvl = codewords(&A_HUFF_TREE_RVL_CODEWDS);
    let escw = codewords(&A_HUFF_TREE_RVLC_ESCAPE);
    let ics = &p.ics_info;
    let cbs = &p.p_dyn_data.a_code_book;
    let gg = p.p_dyn_data.raw_data_info.global_gain as i32;
    let mut data = BitWriter { bits: Vec::new() };
    let mut esc = BitWriter { bits: Vec::new() };
    let big = rng.next_u32() % 4 == 0;
    let dpcm = |rng: &mut Rng| -> i32 {
        if big && rng.next_u32() % 4 == 0 {
            let m = 7 + (rng.next_u32() % 40) as i32;
            if rng.next_u32() % 2 == 0 { m } else { -m }
        } else {
            (rng.next_u32() % 13) as i32 - 6
        }
    };
    let code = |d: i32, data: &mut BitWriter, esc: &mut BitWriter| {
        let c = d.clamp(-7, 7);
        data.put_bits(&rvl[(c + 7) as usize]);
        if d.abs() >= 7 {
            esc.put_bits(&escw[(d.abs() - 7) as usize]);
        }
    };
    let mut factor = gg - SF_OFFSET;
    let mut position = -SF_OFFSET;
    let mut noise: Option<i32> = None;
    let mut noise_nrg = 0;
    let mut intensity = false;
    for g in 0..ics.window_groups as usize {
        for b in 0..ics.max_sf_bands as usize {
            match cbs[16 * g + b] {
                ZERO_HCB => {}
                INTENSITY_HCB | INTENSITY_HCB2 => {
                    intensity = true;
                    let d = dpcm(rng);
                    position += d;
                    code(d, &mut data, &mut esc);
                }
                NOISE_HCB => match noise {
                    None => {
                        noise_nrg = (rng.next_u32() % 512) as i32;
                        noise = Some(gg - SF_OFFSET - 90 - 256 + noise_nrg);
                    }
                    Some(n) => {
                        let d = dpcm(rng);
                        noise = Some(n + d);
                        code(d, &mut data, &mut esc);
                    }
                },
                _ => {
                    let d = dpcm(rng);
                    factor += d;
                    code(d, &mut data, &mut esc);
                }
            }
        }
    }
    let is_last = if intensity {
        let d = dpcm(rng);
        code(d, &mut data, &mut esc);
        d
    } else {
        0
    };
    let _ = position;
    let rev_gg = factor + SF_OFFSET;
    if !(0..256).contains(&rev_gg) {
        return None;
    }
    let mut len = data.bits.len() as i32;
    let mut noise_last = 0;
    if let Some(n) = noise {
        noise_last = n - rev_gg + SF_OFFSET + 90 + 256;
        if !(0..512).contains(&noise_last) {
            return None;
        }
        len += 9;
    }
    let short = ics.window_sequence == BLOCK_SHORT;
    if len >= if short { 2048 } else { 512 } || esc.bits.len() > 255 {
        return None;
    }
    let mut e1 = BitWriter { bits: Vec::new() };
    e1.put(rng.next_u32() % 2, 1); /* sf_concealment */
    e1.put(rev_gg as u32, 8);
    e1.put(len as u32, if short { 11 } else { 9 });
    if noise.is_some() {
        e1.put(noise_nrg as u32, 9);
    }
    let escapes = !esc.bits.is_empty() || rng.next_u32() % 8 == 0;
    e1.put(escapes as u32, 1);
    if escapes {
        e1.put(esc.bits.len() as u32, 8);
    }
    if noise.is_some() {
        e1.put(noise_last as u32, 9);
    }
    let _ = is_last;
    Some(RvlcFrame { esc1: e1.bits, rvl: data.bits, esc: esc.bits })
}

fn to_bytes(bits: &[u8], size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size];
    for (i, &b) in bits.iter().enumerate() {
        out[i / 8] |= b << (7 - i % 8);
    }
    out
}

/// Random codebooks for the RVLC tests (with noise and intensity bands) for the channel stream
/// info `ics`, and a random global gain.
fn random_rvlc_codebooks(rng: &mut Rng, p: &mut CAacDecoderChannelInfo, ics: CIcsInfo) {
    p.ics_info = ics;
    let d = &mut *p.p_dyn_data;
    d.a_code_book = [0; 128];
    let kinds = rng.next_u32() % 4;
    for g in 0..ics.window_groups as usize {
        for b in 0..ics.max_sf_bands as usize {
            d.a_code_book[16 * g + b] = match (kinds, rng.next_u32() % 8) {
                (0, _) => 1 + (rng.next_u32() % 11) as u8,
                (_, 0) => ZERO_HCB,
                (1 | 3, 1) => NOISE_HCB,
                (2 | 3, 2) => [INTENSITY_HCB, INTENSITY_HCB2][(rng.next_u32() % 2) as usize],
                _ => 1 + (rng.next_u32() % 11) as u8,
            };
        }
    }
    d.raw_data_info.global_gain = (60 + rng.next_u32() % 140) as u8;
}

/// Flips `n` random bits of `bits`.
fn flip(rng: &mut Rng, bits: &mut [u8], from: usize, n: u32) {
    for _ in 0..n {
        if bits.len() > from {
            let i = from + (rng.next_u32() as usize) % (bits.len() - from);
            bits[i] ^= 1;
        }
    }
}

/// A sequence of frames on one element of `nch` channels: for every channel CRvlc_Read() and
/// CRvlc_Decode() (plus its steps), then CRvlc_ElementCheck(). Returns the number of channels
/// whose scale factors were not OK.
pub fn rvlc_random_sequence(rng: &mut Rng, frames: usize, nch: usize, steps: bool) -> usize {
    let mut flat = Flat::zero();
    let mut not_ok = 0;
    for fr in 0..frames {
        let mut rs = flat.to_rs();
        let short = rng.next_u32() % 3 == 0;
        let sri = Sri::random(rng, short);
        let common = nch == 2 && rng.next_u32() % 2 == 0;
        let ics0 = random_ics(rng, &sri.t, short);
        for ch in 0..nch {
            let ics = if ch == 0 || common { ics0 } else { random_ics(rng, &sri.t, short) };
            random_rvlc_codebooks(rng, &mut rs.ch[ch], ics);
        }
        rs.com.joint_stereo_data.ms_mask_present = (rng.next_u32() % 3) as u8;
        let mut f = Flat::of(&mut rs);
        for ch in 0..nch {
            let what = format!("frame {fr} ch {ch}");
            let rsf = f.to_rs();
            let mode = rng.next_u32() % 8;
            let mut bits = match encode_rvlc(rng, &rsf.ch[ch]) {
                Some(e) if mode != 0 => {
                    let mut esc1 = e.esc1;
                    let mut esc2 = [e.rvl, e.esc].concat();
                    match mode {
                        1 | 2 => flip(rng, &mut esc2, 0, 1),
                        3 => {
                            let n = 1 + rng.next_u32() % 4;
                            flip(rng, &mut esc2, 0, n)
                        }
                        4 => flip(rng, &mut esc1, 1, 1), /* a wrong side info value */
                        _ => {}
                    }
                    [esc1, esc2].concat()
                }
                _ => (0..600).map(|_| (rng.next_u32() & 1) as u8).collect(),
            };
            continue_with(&mut bits, rng);
            let pos = rng.next_u32() % 64;
            let mut all = vec![0u8; pos as usize];
            all.extend_from_slice(&bits);
            let mut buf = to_bytes(&all, 4096);
            for x in buf[all.len().div_ceil(8)..].iter_mut() {
                *x = rng.next_u32() as u8;
            }
            let bs = Bs::new(buf, pos);
            let (f1, bs) = call_crvlc_read(&format!("{what}: CRvlc_Read"), &f, &bs, ch);
            if steps {
                crvlc_decode_steps(&what, &f1, &bs, ch);
            }
            let (f2, _) = call_crvlc_decode(&format!("{what}: CRvlc_Decode"), &f1, &bs, ch);
            f = f2;
            if f.to_rs().ch[ch].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok == 0 {
                not_ok += 1;
            }
        }
        let flags = if rng.next_u32() % 8 == 0 { 0 } else { AC_ER_RVLC };
        f = call_crvlc_element_check(&format!("frame {fr}: CRvlc_ElementCheck"), &f, true, flags, nch as i32);
        flat = f;
    }
    not_ok
}

/// Appends a few random bits (the data after the RVLC data).
fn continue_with(bits: &mut Vec<u8>, rng: &mut Rng) {
    for _ in 0..64 {
        bits.push((rng.next_u32() & 1) as u8);
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn rvlc_sequences_mono() {
    let mut rng = Rng::new(0x5f1);
    let mut not_ok = 0;
    for _ in 0..60 {
        not_ok += rvlc_random_sequence(&mut rng, 12, 1, false);
    }
    assert!(not_ok > 0);
}

#[test]
fn rvlc_sequences_stereo() {
    let mut rng = Rng::new(0x5f2);
    for _ in 0..60 {
        rvlc_random_sequence(&mut rng, 12, 2, false);
    }
}

#[test]
fn rvlc_decode_steps_random() {
    let mut rng = Rng::new(0x5f3);
    for _ in 0..30 {
        rvlc_random_sequence(&mut rng, 8, 2, true);
    }
}

#[test]
fn crvlc_element_check_random() {
    let mut rng = Rng::new(0xec);
    for i in 0..3000 {
        let mut rs = Rs::new();
        for ch in 0..2 {
            let a = &mut rs.ch[ch].p_dyn_data.specific_to.aac;
            a.rvlc_current_scale_factor_ok = (rng.next_u32() % 3) as i8;
            a.rvlc_intensity_used = (rng.next_u32() % 3) as i8;
            rs.ch[ch].ics_info.window_sequence = (rng.next_u32() % 4) as i32;
            rs.st[ch].concealment_info.rvlc_previous_block_type = rng.next_u32() as i8;
            rs.st[ch].concealment_info.rvlc_previous_scale_factor_ok = rng.next_u32() as i8;
        }
        rs.com.joint_stereo_data.ms_mask_present = (rng.next_u32() % 3) as u8;
        let flags = rng.next_u32() & (AC_ER_RVLC | 0x45);
        let el = (rng.next_u32() % 4) as i32 - 1;
        let with_static = rng.next_u32() % 4 != 0;
        call_crvlc_element_check(&format!("CRvlc_ElementCheck {i}"), &Flat::of(&mut rs), with_static, flags, el);
    }
}

/// A random RVLC state for the codeword decoders.
fn random_rvlc_state(rng: &mut Rng) -> Flat {
    let mut rs = Rs::new();
    let r = &mut rs.com.overlay.aac.er_rvlc_info;
    r.direction = (rng.next_u32() % 2) as u8;
    r.p_rvl_bit_cnt_rvl = (rng.next_u32() % 2) as u8;
    r.p_bitstr_indx_rvl_rvl = (rng.next_u32() % 2) as u8;
    r.length_of_rvlc_sf_fwd = (rng.next_u32() % 40) as i16 - 5;
    r.length_of_rvlc_sf_bwd = (rng.next_u32() % 40) as i16 - 5;
    r.length_of_rvlc_escapes = (rng.next_u32() % 60) as i16 - 5;
    r.bs_anchor = (rng.next_u32() % 4096) as i32;
    r.bitstream_index_rvl_fwd = (rng.next_u32() % 2048) as i32 - 1024;
    r.bitstream_index_rvl_bwd = (rng.next_u32() % 2048) as i32 - 1024;
    r.bitstream_index_esc = (rng.next_u32() % 2048) as i32 - 1024;
    r.p_huff_tree_rvl_codewds = &A_HUFF_TREE_RVL_CODEWDS;
    r.p_huff_tree_rvlc_escape = &A_HUFF_TREE_RVLC_ESCAPE;
    r.error_log_rvlc = if rng.next_u32() % 2 == 0 { 0 } else { rng.next_u32() };
    Flat::of(&mut rs)
}

#[test]
fn decode_rvl_codeword_random() {
    let mut rng = Rng::new(0xdc);
    for i in 0..5000 {
        let f = random_rvlc_state(&mut rng);
        let bs = Bs::random(&mut rng, 9);
        both(
            &format!("decodeRVLCodeword {i}"),
            &f,
            Some(&bs),
            |ctx, b| {
                let (st, buf) = b.unwrap().c();
                unsafe { c::cref_rvlc_decodeRVLCodeword(ctx, st, buf) as i64 }
            },
            |rs, b| decode_rvl_codeword(b.unwrap(), &mut rs.com.overlay.aac.er_rvlc_info) as i64,
        );
    }
}

#[test]
fn rvlc_decode_escape_word_random() {
    let mut rng = Rng::new(0xde);
    for i in 0..5000 {
        let f = random_rvlc_state(&mut rng);
        let bs = Bs::random(&mut rng, 9);
        let (f, bs, _) = both(
            &format!("rvlcDecodeEscapeWord {i}"),
            &f,
            Some(&bs),
            |ctx, b| {
                let (st, buf) = b.unwrap().c();
                unsafe { c::cref_rvlc_rvlcDecodeEscapeWord(ctx, st, buf) as i64 }
            },
            |rs, b| rvlc_decode_escape_word(&mut rs.com.overlay.aac.er_rvlc_info, b.unwrap()) as i64,
        );
        both(
            &format!("rvlcDecodeEscapes {i}"),
            &f,
            bs.as_ref(),
            |ctx, b| {
                let (st, buf) = b.unwrap().c();
                unsafe { c::cref_rvlc_rvlcDecodeEscapes(ctx, st, buf) };
                0
            },
            |rs, b| {
                let a = &mut rs.com.overlay.aac;
                rvlc_decode_escapes(&mut a.er_rvlc_info, &mut a.a_rvlc_scf_esc, b.unwrap());
                0
            },
        );
    }
}

#[test]
fn rvlc_check_intensity_cb_random() {
    let mut rng = Rng::new(0x1c);
    for i in 0..3000 {
        let mut rs = Rs::new();
        let short = rng.next_u32() % 2 == 0;
        let sri = Sri::random(&mut rng, short);
        let ics = random_ics(&mut rng, &sri.t, short);
        random_rvlc_codebooks(&mut rng, &mut rs.ch[0], ics);
        let r = &mut rs.com.overlay.aac.er_rvlc_info;
        r.num_window_groups = rs.ch[0].ics_info.window_groups;
        r.max_sfb_transmitted = rs.ch[0].ics_info.max_sf_bands;
        r.intensity_used = (rng.next_u32() % 2) as i8;
        both(
            &format!("rvlcCheckIntensityCb {i}"),
            &Flat::of(&mut rs),
            None,
            |ctx, _| {
                unsafe { c::cref_rvlc_rvlcCheckIntensityCb(ctx, 0) };
                0
            },
            |rs, _| {
                rvlc_check_intensity_cb(&mut rs.com.overlay.aac.er_rvlc_info, &rs.ch[0]);
                0
            },
        );
    }
}

#[test]
fn rvlc_codeword_tables() {
    // The encoder of these tests: 15 valid symmetric codewords (and 8 forbidden ones) and 54
    // escape codewords.
    let rvl = codewords(&A_HUFF_TREE_RVL_CODEWDS);
    assert_eq!(rvl.len(), 23);
    for cw in &rvl[..15] {
        let mut r = cw.clone();
        r.reverse();
        assert_eq!(&r, cw, "RVL codewords are symmetric");
    }
    assert_eq!(codewords(&A_HUFF_TREE_RVLC_ESCAPE).len(), 54);
}

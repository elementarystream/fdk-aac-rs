// Differential tests of fdk_aac_rs_aacdec::aacdec_hcrs against the C++ reference.
//
// DecodeNonPCWs() runs within the HcrDecoder() steps of aacdec_hcr.rs (random side info) and
// aacdec_hcr_stream.rs (real streams). The state functions run on random states of the non-PCW
// side info (every state, both read directions, segments running empty, escape sequences in all
// phases); the helpers on random inputs.

use super::aacdec_hcr::*;
use fdk_aac_rs_aacdec::aac_rom::A_HUFF_TABLE;
use fdk_aac_rs_aacdec::aacdec_hcr_types::*;
use fdk_aac_rs_aacdec::aacdec_hcrs::*;
use fdk_aac_rs_cref::aacdec_aacdec_hcr as c;
use fdk_aac_rs_cref::testutil::Rng;

pub fn call_decode_non_pcws(what: &str, flat: &Flat, bs: &Bs, ch: usize) -> (Flat, Bs) {
    let (f, b, _) = both(
        what,
        flat,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_aacdec_hcrs_DecodeNonPCWs(ctx, ch as i32, st, buf) };
            0
        },
        |rs, b| {
            let (h, _, s) = rs.hcr(ch);
            decode_non_pcws(b.unwrap(), h, s);
            0
        },
    );
    (f, b.unwrap())
}

/// A random codebook of the non-PCW side info (1 to 11, 16 to 31).
fn random_cb(rng: &mut Rng) -> u8 {
    loop {
        let cb = (rng.next_u32() % 32) as u8;
        if cb != 0 && !(12..=15).contains(&cb) {
            return cb;
        }
    }
}

/// A random state for the state function `k` (1..=7) at a random segment and codeword.
fn random_state(rng: &mut Rng, k: u8) -> (Flat, Bs, usize) {
    let mut rs = Rs::new();
    let ch = (rng.next_u32() % 2) as usize;
    let h = &mut rs.com.overlay.aac.er_hcr_info;
    h.dec_in_out.bitstream_anchor = (rng.next_u32() % 8192) as i32;
    h.dec_in_out.error_log = if rng.next_u32() % 4 == 0 { rng.next_u32() } else { 0 };
    let sg = &mut h.segment_info;
    sg.num_segment = 1 + rng.next_u32() % 512;
    sg.read_direction = (rng.next_u32() % 2) as u8;
    for i in 0..512 {
        sg.p_remaining_bits_in_segment[i] = match rng.next_u32() % 4 {
            0 => (rng.next_u32() % 4) as i8,
            1 => (rng.next_u32() % 8) as i8 - 3,
            _ => (rng.next_u32() % 60) as i8,
        };
        sg.p_left_start_of_segment[i] = (rng.next_u32() % 8192) as i32 - 2048;
        sg.p_right_start_of_segment[i] = (rng.next_u32() % 8192) as i32 - 2048;
    }
    for w in sg.p_segment_bitfield.iter_mut().chain(sg.p_codeword_bitfield.iter_mut()) {
        *w = rng.next_u32();
    }
    sg.segment_offset = rng.next_u32() % 512;
    let np = &mut h.non_pcw_sideinfo;
    let co = (rng.next_u32() % 256) as usize;
    np.codeword_offset = co as u32;
    np.p_state = k;
    for i in 0..256 {
        np.p_codebook[i] = random_cb(rng);
        np.p_sta[i] = (rng.next_u32() % 8) as i8;
        np.p_cnt_sign[i] = (rng.next_u32() % 5) as u8;
        np.i_result_pointer[i] = (rng.next_u32() % 1020) as u16;
        np.p_escape_sequence_info[i] = rng.next_u32() & 0x003F_FFFF;
        let tree = A_HUFF_TABLE[np.p_codebook[i] as usize];
        np.i_node[i] = tree[(rng.next_u32() as usize) % tree.len()];
    }
    match k {
        BODY_SIGN__SIGN | BODY_SIGN_ESC__SIGN => {
            np.p_cnt_sign[co] = 1 + (rng.next_u32() % 4) as u8;
            if k == BODY_SIGN_ESC__SIGN {
                np.p_cnt_sign[co] = 1 + (rng.next_u32() % 2) as u8;
                // the backup of the result pointer of BODY_SIGN_ESC__BODY
                np.i_node[co] = np.i_result_pointer[co] as u32;
            }
        }
        BODY_SIGN_ESC__BODY => {
            let tree = A_HUFF_TABLE[ESCAPE_CODEBOOK as usize];
            np.i_node[co] = tree[(rng.next_u32() as usize) % tree.len()];
        }
        BODY_SIGN_ESC__ESC_PREFIX => {
            let up = rng.next_u32() % 9;
            let flags = (rng.next_u32() % 3 + 1) << POSITION_OF_FLAG_B;
            np.p_escape_sequence_info[co] = flags | (up << 16);
        }
        BODY_SIGN_ESC__ESC_WORD => {
            let up = 4 + rng.next_u32() % 9;
            let down = match rng.next_u32() % 8 {
                0 => 0,
                _ => 1 + rng.next_u32() % up,
            };
            let word = rng.next_u32() % (1 << (up - down).min(12));
            let flags = (rng.next_u32() % 4) << POSITION_OF_FLAG_B;
            np.p_escape_sequence_info[co] = flags | (up << 16) | (down << 12) | word;
        }
        _ => {}
    }
    for v in rs.spec.iter_mut() {
        *v = match rng.next_u32() % 4 {
            0 => 0,
            1 => [16, -16][(rng.next_u32() % 2) as usize],
            _ => (rng.next_u32() % 33) as i32 - 16,
        };
    }
    if rng.next_u32() % 3 == 0 {
        // a codeword at the end of the spectrum, followed by zeros up to its end (the sign
        // states search for a line different from zero up to line 1023)
        let p = 1000 + (rng.next_u32() % 20) as usize;
        let np = &mut rs.com.overlay.aac.er_hcr_info.non_pcw_sideinfo;
        np.i_result_pointer[co] = p as u16;
        if k == BODY_SIGN_ESC__SIGN {
            np.i_node[co] = p as u32;
        }
        let s = &mut rs.spec[ch * 1024..ch * 1024 + 1024];
        let z = p + (rng.next_u32() % 3) as usize;
        s[z..].fill(0);
        if rng.next_u32() % 2 == 0 {
            s[1023] = 1;
        }
    }
    if k == BODY_SIGN__SIGN || k == BODY_SIGN_ESC__SIGN {
        // The sign states need pCntSign to be at most the number of non-zero lines from
        // iResultPointer on (as after the body state); other states have no defined result in
        // the reference, so line 1023 is cleared for them. The states that start with the body
        // (and the escape states) keep this consistency.
        let np = &rs.com.overlay.aac.er_hcr_info.non_pcw_sideinfo;
        let (p, cnt) = (np.i_result_pointer[co] as usize, np.p_cnt_sign[co] as usize);
        let s = &mut rs.spec[ch * 1024..ch * 1024 + 1024];
        if s[p..].iter().filter(|&&v| v != 0).count() < cnt {
            s[1023] = 0;
        }
    }
    (Flat::of(&mut rs), Bs::random(rng, 10), ch)
}

#[test]
fn state_functions_random() {
    let mut rng = Rng::new(0x57a7e);
    for i in 0..20000 {
        let k = 1 + (rng.next_u32() % 7) as u8;
        let (f, bs, ch) = random_state(&mut rng, k);
        both(
            &format!("state {k} ({i})"),
            &f,
            Some(&bs),
            |ctx, b| {
                let (st, buf) = b.unwrap().c();
                unsafe { c::cref_aacdec_hcrs_state(ctx, ch as i32, k as i32, st, buf) as i64 }
            },
            |rs, b| {
                let (h, _, s) = rs.hcr(ch);
                hcr_state(k, b.unwrap(), h, s) as i64
            },
        );
    }
}

#[test]
fn state_machine_chains_random() {
    // Runs the state machine from a random start state until it stops, as DecodeNonPCWs()
    // does, comparing after every state.
    let mut rng = Rng::new(0xc4a1);
    for i in 0..3000 {
        let k = 1 + (rng.next_u32() % 7) as u8;
        let (mut f, mut bs, ch) = random_state(&mut rng, k);
        for n in 0..64 {
            let st = f.hcr[layout().names[1].iter().find(|(_, nm)| *nm == "pState").unwrap().0];
            if st == 0 {
                break;
            }
            let (f2, b2, r) = both(
                &format!("chain {i} step {n} (state {st})"),
                &f,
                Some(&bs),
                |ctx, b| {
                    let (s, buf) = b.unwrap().c();
                    unsafe { c::cref_aacdec_hcrs_state(ctx, ch as i32, st, s, buf) as i64 }
                },
                |rs, b| {
                    let (h, _, s) = rs.hcr(ch);
                    hcr_state(st as u8, b.unwrap(), h, s) as i64
                },
            );
            f = f2;
            bs = b2.unwrap();
            if r != 0 {
                break;
            }
        }
    }
}

#[test]
fn init_segment_bitfield_random() {
    let mut rng = Rng::new(0x15b);
    for i in 0..3000 {
        let mut rs = Rs::new();
        let sg = &mut rs.com.overlay.aac.er_hcr_info.segment_info;
        sg.num_segment = match rng.next_u32() % 4 {
            0 => rng.next_u32() % 3,
            1 => 32 * (rng.next_u32() % 17),
            _ => rng.next_u32() % 513,
        };
        for r in sg.p_remaining_bits_in_segment.iter_mut() {
            *r = if rng.next_u32() % 3 == 0 { 0 } else { rng.next_u32() as i8 };
        }
        for w in sg.p_segment_bitfield.iter_mut() {
            *w = rng.next_u32();
        }
        both(
            &format!("InitSegmentBitfield {i}"),
            &Flat::of(&mut rs),
            None,
            |ctx, _| unsafe { c::cref_aacdec_hcrs_InitSegmentBitfield(ctx) as i64 },
            |rs, _| {
                let sg = &mut rs.com.overlay.aac.er_hcr_info.segment_info;
                init_segment_bitfield(
                    &sg.num_segment,
                    &sg.p_remaining_bits_in_segment,
                    &mut sg.p_segment_bitfield,
                    &mut sg.num_word_for_bitfield,
                    &mut sg.p_num_bit_valid_in_last_word,
                ) as i64
            },
        );
    }
}

#[test]
fn init_non_pcw_side_information_random() {
    let mut rng = Rng::new(0x1fc);
    for i in 0..3000 {
        let mut rs = Rs::new();
        let h = &mut rs.com.overlay.aac.er_hcr_info;
        let si = &mut h.section_info;
        si.num_extended_sorted_codeword_in_section_idx = (rng.next_u32() % 270) as i32;
        si.extended_sorted_codebook_idx = (rng.next_u32() % 270) as i32;
        si.num_extended_sorted_sections_in_sets_idx = (rng.next_u32() % 14) as i32;
        for v in si.p_num_extended_sorted_codeword_in_section.iter_mut() {
            *v = (rng.next_u32() % 40) as u16;
        }
        for v in si.p_num_extended_sorted_sections_in_sets.iter_mut() {
            *v = (rng.next_u32() % 30) as u16;
        }
        for v in si.p_extended_sorted_codebook.iter_mut() {
            *v = (rng.next_u32() % 32) as u8;
        }
        h.dec_in_out.quantized_spectral_coefficients_idx = (rng.next_u32() % 1024) as i32;
        both(
            &format!("InitNonPCWSideInformationForCurrentSet {i}"),
            &Flat::of(&mut rs),
            None,
            |ctx, _| {
                unsafe { c::cref_aacdec_hcrs_InitNonPCWSideInformationForCurrentSet(ctx) };
                0
            },
            |rs, _| {
                init_non_pcw_side_information_for_current_set(&mut rs.com.overlay.aac.er_hcr_info);
                0
            },
        );
    }
}

#[test]
fn modulo_value_random() {
    let mut rng = Rng::new(0x30d);
    for _ in 0..100000 {
        let n = 1 + (rng.next_u32() % 600) as i32;
        let x = (rng.next_u32() % 3000) as i32 - 1500;
        assert_eq!(unsafe { c::cref_aacdec_hcrs_ModuloValue(x, n) }, modulo_value(x, n), "ModuloValue({x}, {n})");
    }
}

#[test]
fn clear_bit_from_bitfield_random() {
    let mut rng = Rng::new(0xc1b);
    for _ in 0..20000 {
        let off = rng.next_u32() % (17 * 32);
        let bf: Vec<u32> = (0..17).map(|_| rng.next_u32()).collect();
        let mut cbf = bf.clone();
        let cr = unsafe { c::cref_aacdec_hcrs_ClearBitFromBitfield(off, cbf.as_mut_ptr()) };
        let mut rbf = bf.clone();
        let mut st: u8 = 1;
        clear_bit_from_bitfield(&mut st, off, &mut rbf);
        assert_eq!(cbf, rbf, "ClearBitFromBitfield({off})");
        assert_eq!(cr, (st == 0) as i32);
    }
}

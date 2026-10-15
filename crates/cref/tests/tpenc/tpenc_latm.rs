// Differential tests of fdk_aac_rs_tpenc::tpenc_latm against the C++ reference.
//
// - The static helpers on random inputs: transportEnc_LatmWriteValue(),
//   WriteAuPayloadLengthInfo(), transportEnc_LatmSetNrOfSubframes(),
//   allStreamsSameTimeFraming(), transportEnc_InitLatmStream(),
//   transportEnc_LatmAddOtherDataBits().
// - The bit demand counters (transportEnc_LatmCountFix/Var/TotalBitDemandHeader) and
//   transportEnc_LatmAdjustSubframeBits() on random LATM_STREAM images (random field contents,
//   both framing modes, every frameLengthType), with random payload lengths
//   including wrapped (negative) ones.
// - CreateStreamMuxConfig() and AdvanceAudioMuxElement() on random states and configurations
//   (AudioMuxVersion 0 and 1, other data, every AOT incl. TwinVQ/CELP/HVXC frame lengths,
//   callbacks writing SBR headers and SSCs).
// - Stream sequences: transportEnc_Latm_Init(), other data bits, then frames of
//   transportEnc_LatmCountTotalBitDemandHeader(), transportEnc_LatmWrite(), payload,
//   transportEnc_LatmAdjustSubframeBits(), transportEnc_LatmGetFrame() for LOAS, LATM MCP1
//   and MCP0 with 1 or 2 subframes, header periods 0..9 and occasional CreateStreamMuxConfig()
//   calls (transportEnc_GetConf).
//
// LATM_STREAM is compared as its byte image after every call, with the bitstream, the buffer,
// the configuration (writeASC may change it) and the callback log.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpenc_tpenc_latm as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::fdk_get_valid_bits;
use fdk_aac_rs_tpenc::tpenc_latm::*;
use core::mem::size_of;

/// Offsets in `LATM_STREAM`: the pointer `config[0][0]` has 8 bytes on LP64 and 4 bytes on 32-bit
/// x86, which moves the fields after it by 8 bytes (alignment).
const ILP32: bool = cfg!(target_pointer_width = "32");
const OFF_CONFIG: usize = if ILP32 { 28 } else { 32 };
const OFF_NO_PROGRAM: usize = if ILP32 { 60 } else { 68 };
const OFF_FILL_BITS: usize = if ILP32 { 74 } else { 82 };

use super::tpenc_asc::{cc_to, eld_sbr_null_pce, rand_cc, rec_pair, with_cc, BsPair, CRec, Rec};

struct Img {
    c: [u8; LATM_STREAM_SIZE],
    r: [u8; LATM_STREAM_SIZE],
}

impl Img {
    fn zero() -> Self {
        Img { c: [0; LATM_STREAM_SIZE], r: [0; LATM_STREAM_SIZE] }
    }

    /// Random bytes; config[0][0] 0 or 1, noProgram 0 or 1 (the values with a defined reference
    /// result, see the module documentation of the port).
    fn random(rng: &mut Rng) -> Self {
        let mut c = [0u8; LATM_STREAM_SIZE];
        for b in c.iter_mut() {
            *b = rng.next_u32() as u8;
        }
        let p = rng.range(0, 1) as usize;
        c[OFF_CONFIG..OFF_CONFIG + size_of::<usize>()].copy_from_slice(&p.to_le_bytes());
        c[OFF_NO_PROGRAM] = rng.range(0, 1) as u8;
        let mut l = LatmStream::load(&c);
        // Bias towards meaningful values.
        if rng.range(0, 1) == 0 {
            l.m_linfo[0][0].frame_length_type = [0, 1, 4, 6, 2, -1][rng.range(0, 5) as usize];
        }
        if rng.range(0, 1) == 0 {
            l.m_linfo[0][0].stream_id = rng.range(-1, 2);
        }
        if rng.range(0, 1) == 0 {
            l.all_streams_same_time_framing = rng.range(0, 1) as u8;
        }
        if rng.range(0, 1) == 0 {
            l.var_mode = rng.range(0, 1);
        }
        if rng.range(0, 1) == 0 {
            l.tt = [TT_MP4_LOAS, TT_MP4_LATM_MCP0, TT_MP4_LATM_MCP1, TT_MP4_ADTS][rng.range(0, 3) as usize];
        }
        if rng.range(0, 1) == 0 {
            l.sub_frame_cnt = rng.range(0, 2) as u8;
            l.latm_frame_counter = rng.range(0, 2) as u8;
            l.mux_config_period = rng.range(0, 3) as u8;
        }
        if rng.range(0, 1) == 0 {
            l.stream_mux_config_bits = rng.range(0, 300) as u32;
            l.other_data_len_bits = rng.range(0, 40) as u8;
        }
        if rng.range(0, 1) == 0 {
            l.audio_mux_version = rng.range(0, 1) as u8;
            l.no_subframes = rng.range(0, 3) as u8;
            l.no_layer[0] = rng.range(0, 2) as u8;
        }
        l.store(&mut c);
        Img { c, r: c }
    }

    fn rust<R>(&mut self, f: impl FnOnce(&mut LatmStream) -> R) -> R {
        let mut a = LatmStream::load(&self.r);
        let x = f(&mut a);
        a.store(&mut self.r);
        x
    }

    fn check(&self, what: &str) {
        if self.c != self.r {
            let i = (0..LATM_STREAM_SIZE).find(|&i| self.c[i] != self.r[i]).unwrap();
            panic!("{what}: LATM_STREAM differs at byte {i}:\nC    {:?}\nRust {:?}", self.c, self.r);
        }
    }
}

#[test]
fn write_value_and_payload_length() {
    let mut rng = Rng::new(0x7e31);
    assert_eq!(unsafe { c::cref_tpenc_latm_size() } as usize, LATM_STREAM_SIZE);
    for it in 0..20000 {
        let mut bs = BsPair::new(&mut rng, 1024);
        bs.pat(rng.range(0, 40), rng.next_u32());
        if rng.range(0, 1) == 0 {
            let v = match rng.range(0, 3) {
                0 => rng.i32(),
                1 => [255, 256, 65535, 65536, (1 << 24) - 1, 1 << 24, -1, 0][rng.range(0, 7) as usize],
                _ => rng.range(0, 1 << 25),
            };
            let rc = unsafe { c::cref_tpenc_latm_write_value(bs.c, v) };
            let rr = transport_enc_latm_write_value(&mut bs.r, v);
            assert_eq!(rc, rr, "{it}: transportEnc_LatmWriteValue({v})");
            bs.check(&format!("{it}: transportEnc_LatmWriteValue({v})"));
        } else {
            let v = match rng.range(0, 3) {
                0 => rng.range(-3000, 30000),
                1 => rng.range(-100, 6000) & !7,
                _ => (rng.range(0, 3000) * 8) - 8 * rng.range(0, 1),
            };
            let rc = unsafe { c::cref_tpenc_latm_payload_len(bs.c, v) };
            let rr = write_au_payload_length_info(&mut bs.r, v);
            assert_eq!(rc, rr, "{it}: WriteAuPayloadLengthInfo({v})");
            bs.check(&format!("{it}: WriteAuPayloadLengthInfo({v})"));
        }
    }
}

#[test]
fn counters_on_random_states() {
    let mut rng = Rng::new(0x7e32);
    for it in 0..40000 {
        let mut img = Img::random(&mut rng);
        let len: u32 = match rng.range(0, 4) {
            0 => rng.next_u32(),
            1 => rng.range(-50, 0) as u32,
            2 => 0,
            _ => rng.range(0, 40000) as u32,
        };
        let what = format!("{it}: len {len}");
        match rng.range(0, 6) {
            0 => {
                let rc = unsafe { c::cref_tpenc_latm_count_fix(img.c.as_mut_ptr()) };
                let rr = img.rust(transport_enc_latm_count_fix_bit_demand_header);
                assert_eq!(rc, rr, "{what}: CountFix");
            }
            1 => {
                let rc = unsafe { c::cref_tpenc_latm_count_var(img.c.as_mut_ptr(), len) };
                let rr = img.rust(|l| transport_enc_latm_count_var_bit_demand_header(l, len));
                assert_eq!(rc, rr, "{what}: CountVar");
            }
            2 => {
                let rc = unsafe { c::cref_tpenc_latm_count_total(img.c.as_mut_ptr(), len) };
                let rr = img.rust(|l| transport_enc_latm_count_total_bit_demand_header(l, len));
                assert_eq!(rc, rr, "{what}: CountTotal");
            }
            3 => {
                let b0 = rng.i32();
                let (mut bc, mut br) = (b0, b0);
                unsafe { c::cref_tpenc_latm_adjust(img.c.as_mut_ptr(), &mut bc) };
                img.rust(|l| transport_enc_latm_adjust_subframe_bits(l, &mut br));
                assert_eq!(bc, br, "{what}: AdjustSubframeBits");
            }
            4 => {
                let n = match rng.range(0, 2) {
                    0 => rng.range(-3, 4),
                    _ => rng.i32(),
                };
                let rc = unsafe { c::cref_tpenc_latm_set_nr_sub(img.c.as_mut_ptr(), n) };
                let rr = img.rust(|l| transport_enc_latm_set_nr_of_subframes(l, n));
                assert_eq!(rc, rr, "{what}: SetNrOfSubframes({n})");
            }
            _ => {
                let n = match rng.range(0, 2) {
                    0 => rng.range(-20, 300),
                    _ => rng.i32(),
                };
                let rc = unsafe { c::cref_tpenc_latm_add_other(img.c.as_mut_ptr(), n) };
                let rr = img.rust(|l| transport_enc_latm_add_other_data_bits(l, n));
                assert_eq!(rc, rr, "{what}: AddOtherDataBits({n})");
            }
        }
        img.check(&what);
    }
}

#[test]
fn init_stream_and_framing() {
    let mut rng = Rng::new(0x7e33);
    for it in 0..20000 {
        let mut img = Img::random(&mut rng);
        let mut cc = rand_cc(&mut rng);
        if rng.range(0, 1) == 0 {
            cc.sampling_rate = super::tpenc_asc::RATES[rng.range(0, 12) as usize];
        }
        if cc.sampling_rate == 0 || cc.sampling_rate == -1 {
            // No defined x86 reference result (x86 division by zero; see the port).
            cc.sampling_rate = 44100;
        }
        let (mut v, mut raw) = cc_to(&cc);
        let what = format!("{it}");
        if rng.range(0, 1) == 0 {
            let np = rng.range(0, 1);
            let rc = unsafe { c::cref_tpenc_latm_all_same(img.c.as_mut_ptr(), np, v.as_mut_ptr(), raw.as_mut_ptr()) };
            let rr = img.rust(|l| all_streams_same_time_framing(l, np as u8));
            assert_eq!(rc, rr, "{what}: allStreamsSameTimeFraming");
        } else {
            let fdp = rng.range(-1, 2);
            let mcp = match rng.range(0, 2) {
                0 => rng.i32(),
                _ => rng.range(0, 300),
            };
            let amv = rng.range(0, 3) as u32;
            let tt = rng.range(-1, 12);
            img.c[OFF_NO_PROGRAM] = 1;
            img.r[OFF_NO_PROGRAM] = 1;
            let rc = unsafe {
                c::cref_tpenc_latm_init_stream(img.c.as_mut_ptr(), fdp, mcp, amv, tt, v.as_mut_ptr(), raw.as_mut_ptr())
            };
            let rr = img.rust(|l| transport_enc_init_latm_stream(l, fdp, mcp, amv, tt));
            assert_eq!(rc, rr, "{what}: transportEnc_InitLatmStream");
        }
        img.check(&what);
    }
}

/// A configuration that the LATM writer can carry (or not, now and then).
fn latm_cc(rng: &mut Rng, has: i32) -> CoderConfig {
    let mut cc = rand_cc(rng);
    if rng.range(0, 5) != 0 {
        cc.aot = [2, 2, 5, 29, 23, 39, 39, 42, 1, 3, 4, 6, 7, 17][rng.range(0, 13) as usize];
    }
    if eld_sbr_null_pce(&cc, has, true) {
        cc.channel_mode = MODE_1;
    }
    cc
}

fn create_smc(rng: &mut Rng, img: &mut Img, cc: &mut CoderConfig, crec: &CRec, rrec: &mut Rec, what: &str) {
    let mut bs = BsPair::new(rng, 4096);
    bs.pat(rng.range(0, 40), rng.next_u32());
    let bf = if rng.range(0, 1) == 0 { rng.i32() } else { rng.range(0, 0xFF) };
    let has = rrec.has;
    let (rc, ccc) = with_cc(cc, |v, raw| unsafe {
        c::cref_tpenc_latm_create_smc(img.c.as_mut_ptr(), bs.c, bf, v, raw, crec.0, has)
    });
    let rr = img.rust(|l| create_stream_mux_config(l, &mut bs.r, bf, cc, rrec));
    assert_eq!(rc, rr, "{what}: CreateStreamMuxConfig");
    img.check(&format!("{what}: CreateStreamMuxConfig"));
    bs.check(&format!("{what}: CreateStreamMuxConfig"));
    assert_eq!(ccc, *cc, "{what}: configuration");
    assert_eq!(crec.take(), rrec.log, "{what}: callbacks");
    rrec.log.clear();
}

#[test]
fn stream_mux_config_on_random_states() {
    let mut rng = Rng::new(0x7e34);
    for it in 0..20000 {
        let mut img = Img::random(&mut rng);
        let (crec, mut rrec) = rec_pair(&mut rng);
        let mut cc = latm_cc(&mut rng, rrec.has);
        let l = LatmStream::load(&img.r);
        if l.all_streams_same_time_framing == 0 && cc.aot == AOT_AAC_SCAL {
            // No defined reference result.
            cc.aot = AOT_AAC_LC;
        }
        let what = format!("{it}: aot {}", cc.aot);
        if rng.range(0, 1) == 0 {
            create_smc(&mut rng, &mut img, &mut cc, &crec, &mut rrec, &what);
        } else {
            let mut bs = BsPair::new(&mut rng, 4096);
            bs.pat(rng.range(0, 40), rng.next_u32());
            let au = match rng.range(0, 2) {
                0 => rng.range(-40, 20000),
                _ => rng.range(0, 3000) * 8,
            };
            let bf = rng.range(0, 0xFF);
            let has = rrec.has;
            let (rc, ccc) = with_cc(&cc, |v, raw| unsafe {
                c::cref_tpenc_latm_advance(img.c.as_mut_ptr(), bs.c, au, bf, v, raw, crec.0, has)
            });
            let rr = img.rust(|l| advance_audio_mux_element(l, &mut bs.r, au, bf, &mut cc, &mut rrec));
            assert_eq!(rc, rr, "{what}: AdvanceAudioMuxElement");
            img.check(&what);
            bs.check(&what);
            assert_eq!(ccc, cc, "{what}: configuration");
            assert_eq!(crec.take(), rrec.log, "{what}: callbacks");
        }
    }
}

#[test]
fn streams() {
    let mut rng = Rng::new(0x7e35);
    for it in 0..3000 {
        let mut img = if rng.range(0, 1) == 0 { Img::zero() } else { Img::random(&mut rng) };
        // fillBits that keep FDKwriteBits() in transportEnc_LatmGetFrame() at 32 bits or less.
        img.c[OFF_FILL_BITS] %= 25;
        img.r[OFF_FILL_BITS] %= 25;
        let (crec, mut rrec) = rec_pair(&mut rng);
        let mut cc = latm_cc(&mut rng, rrec.has);
        cc.n_sub_frames = match rng.range(0, 5) {
            0 => rng.range(-1, 4),
            1 | 2 => 2,
            _ => 1,
        };
        cc.header_period = match rng.range(0, 4) {
            0 => 0,
            1 => rng.next_u32() as u8,
            _ => rng.range(1, 9) as u8,
        };
        let tt = match rng.range(0, 7) {
            0 => rng.range(-1, 12),
            1 | 2 => TT_MP4_LOAS,
            3 | 4 => TT_MP4_LATM_MCP1,
            _ => TT_MP4_LATM_MCP0,
        };
        let amv = rng.range(0, 2) as u32;
        let mut bs = BsPair::new(&mut rng, 8192);
        let has = rrec.has;
        let what = format!("{it}: tt {tt} aot {} amv {amv}", cc.aot);

        let (rc, ccc) = with_cc(&cc, |v, raw| unsafe {
            c::cref_tpenc_latm_init(img.c.as_mut_ptr(), bs.c, v, raw, amv, tt, crec.0, has)
        });
        let rr = img.rust(|l| transport_enc_latm_init(l, &mut bs.r, &mut cc, amv, tt, &mut rrec));
        assert_eq!(rc, rr, "{what}: transportEnc_Latm_Init");
        img.check(&what);
        bs.check(&what);
        assert_eq!(ccc, cc, "{what}: configuration");
        assert_eq!(crec.take(), rrec.log, "{what}: callbacks");
        rrec.log.clear();
        if rr != 0 {
            continue;
        }

        if rng.range(0, 2) == 0 {
            // Other data bits that keep FDKwriteBits() at 32 bits or less (see the port).
            let n = if tt == TT_MP4_LATM_MCP0 {
                8 * rng.range(1, 4)
            } else {
                8 * rng.range(2, 5)
            };
            let n = if rng.range(0, 7) == 0 { n + rng.range(1, 7) } else { n };
            let rc = unsafe { c::cref_tpenc_latm_add_other(img.c.as_mut_ptr(), n) };
            let rr = img.rust(|l| transport_enc_latm_add_other_data_bits(l, n));
            assert_eq!(rc, rr, "{what}: AddOtherDataBits({n})");
            img.check(&what);
        }

        for frame in 0..rng.range(1, 20) {
            let what = format!("{what} frame {frame}");
            if rng.range(0, 9) == 0 {
                let (crec2, mut rrec2) = rec_pair(&mut rng);
                if eld_sbr_null_pce(&cc, rrec2.has, true) {
                    rrec2.has &= !1;
                    crec2.set(rrec2.sbr_bits, rrec2.ssc_bits, rrec2.seed);
                }
                // The C recorder's callbacks follow `has` (passed per call).
                create_smc(&mut rng, &mut img, &mut cc, &crec2, &mut rrec2, &what);
            }
            let au_bits = match rng.range(0, 9) {
                0 => rng.range(-20, 3000),
                _ => 8 * rng.range(0, 700),
            };
            let len = if rng.range(0, 3) == 0 { rng.next_u32() } else { au_bits as u32 };
            let tc = unsafe { c::cref_tpenc_latm_count_total(img.c.as_mut_ptr(), len) };
            let tr = img.rust(|l| transport_enc_latm_count_total_bit_demand_header(l, len));
            assert_eq!(tc, tr, "{what}: CountTotal");
            img.check(&what);

            let bf = rng.range(0, 0xFF);
            let (rc, ccc) = with_cc(&cc, |v, raw| unsafe {
                c::cref_tpenc_latm_write(img.c.as_mut_ptr(), bs.c, au_bits, bf, v, raw, crec.0, has)
            });
            let rr = img.rust(|l| transport_enc_latm_write(l, &mut bs.r, au_bits, bf, &mut cc, &mut rrec));
            assert_eq!(rc, rr, "{what}: transportEnc_LatmWrite");
            img.check(&format!("{what}: transportEnc_LatmWrite"));
            bs.check(&format!("{what}: transportEnc_LatmWrite"));
            assert_eq!(ccc, cc, "{what}: configuration");
            assert_eq!(crec.take(), rrec.log, "{what}: callbacks");
            rrec.log.clear();

            if au_bits > 0 {
                bs.pat(au_bits, rng.next_u32());
            }
            let vb = unsafe { fdk_aac_rs_cref::tpenc_tpenc_asc::cref_tpenc_bs_valid_bits(bs.c) };
            assert_eq!(vb, fdk_get_valid_bits(&mut bs.r));
            let b0 = if rng.range(0, 5) == 0 { rng.i32() } else { vb as i32 };
            let (mut bc, mut br) = (b0, b0);
            unsafe { c::cref_tpenc_latm_adjust(img.c.as_mut_ptr(), &mut bc) };
            img.rust(|l| transport_enc_latm_adjust_subframe_bits(l, &mut br));
            assert_eq!(bc, br, "{what}: AdjustSubframeBits");

            let n0 = rng.i32();
            let (mut nc, mut nr) = (n0, n0);
            let rc = unsafe { c::cref_tpenc_latm_get_frame(img.c.as_mut_ptr(), bs.c, &mut nc) };
            let rr = img.rust(|l| transport_enc_latm_get_frame(l, &mut bs.r, &mut nr));
            assert_eq!(rc, rr, "{what}: transportEnc_LatmGetFrame");
            assert_eq!(nc, nr, "{what}: transportEnc_LatmGetFrame bytes");
            img.check(&format!("{what}: transportEnc_LatmGetFrame"));
            bs.check(&format!("{what}: transportEnc_LatmGetFrame"));
        }
    }
}

/// CELP_FRAME_LENGTH_TABLE (`celpFrameLengthTable`, file-static; a copy compiled into the
/// shim), element by element.
#[test]
fn celp_frame_length_table() {
    let mut bytes = 0;
    let p = unsafe { c::cref_tpenc_latm_celp_frame_length_table(&mut bytes) };
    let cv = unsafe { std::slice::from_raw_parts(p, bytes as usize / 2) };
    assert_eq!(cv, &CELP_FRAME_LENGTH_TABLE[..]);
}

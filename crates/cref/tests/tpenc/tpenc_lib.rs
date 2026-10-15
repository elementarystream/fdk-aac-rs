// Differential tests of fdk_aac_rs_tpenc::tpenc_lib against the C++ reference.
//
// The transport encoder runs as a whole on both sides, as the AAC encoder drives it: open,
// register callbacks (recorders writing SBR headers and SSCs), then one or more
// transportEnc_Init() calls with random transports (raw, ADIF, ADTS with and without CRC and
// with 1..4 raw data blocks, LATM MCP1/MCP0 and LOAS with AudioMuxVersion 0/1, 1 or 2
// subframes, header periods, other data bits, invalid formats) and random configurations (PCE
// in the raw_data_block with matrix mixdown, channel configuration zero, ELD with SBR and
// LD-SAC, ...), followed by frames of transportEnc_GetStaticBits(), transportEnc_WriteAccessUnit()
// (random bit counts, buffer fullness and channel counts), payloads with CRC regions,
// transportEnc_EndAccessUnit() and transportEnc_GetFrame(), and transportEnc_GetConf() now and
// then. Calls in an order the encoder does not use (EndAccessUnit/GetFrame right after Init)
// are made as well; the port reproduces the writer union from the C layout. After
// every call the return values, the complete state (configuration, bitstream, counters, the
// writer union as bytes, the registered callbacks), the whole output buffer and the callback
// log are compared. The Rust side alternates between the registered callbacks and the `_cb`
// functions.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use core::ffi::c_void;

/// The reference is a 32-bit x86 build: 4-byte pointers in the writer union.
const ILP32: bool = cfg!(target_pointer_width = "32");

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpenc_tpenc_lib as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::fdk_get_valid_bits;
use fdk_aac_rs_tpenc::tpenc_lib::*;

use super::tpenc_asc::{
    bs_dump, cc_to, eld_sbr_null_pce, pat, rand_cc, rec_pair, rec_ssc_fn, rec_sbr_fn, rec_usac_fn,
    with_cc, BsPair, CRec, Rec, MODES, RATES,
};

const BUF: usize = 16384;

struct Pair {
    c: *mut c_void,
    r: Option<Box<TransportEnc>>,
    cbuf: Vec<u8>,
    rbuf: BitMem,
    crec: CRec,
    rrec: Box<Rec>,
    /// The registered callbacks of the Rust side (`has` of the C side).
    has: i32,
}

impl Pair {
    fn new(rng: &mut Rng) -> Self {
        let c = unsafe { c::cref_tpenc_lib_open() };
        assert!(!c.is_null());
        let mut r = None;
        assert_eq!(transport_enc_open(&mut r), TRANSPORTENC_OK);
        let mut cbuf = vec![0u8; BUF];
        if rng.range(0, 1) == 0 {
            for b in cbuf.iter_mut() {
                *b = rng.next_u32() as u8;
            }
        }
        let rbuf = BitMem::from_slice(&cbuf);
        let (crec, rrec) = rec_pair(rng);
        Pair { c, r, cbuf, rbuf, crec, rrec: Box::new(rrec), has: 0 }
    }

    fn h(&mut self) -> &mut TransportEnc {
        self.r.as_mut().unwrap()
    }

    fn register(&mut self, has: i32) {
        let mut out = [0i32; 3];
        unsafe { c::cref_tpenc_lib_register(self.c, self.crec.0, has, out.as_mut_ptr()) };
        let user = &mut *self.rrec as *mut Rec as *mut c_void;
        let h = self.r.as_deref_mut();
        let r0 = transport_enc_register_sbr_callback(h, if has & 1 != 0 { Some(rec_sbr_fn) } else { None }, user);
        let h = self.r.as_deref_mut();
        let r1 = transport_enc_register_ssc_callback(h, if has & 2 != 0 { Some(rec_ssc_fn) } else { None }, user);
        let h = self.r.as_deref_mut();
        let r2 = transport_enc_register_usac_callback(h, if has & 4 != 0 { Some(rec_usac_fn) } else { None }, user);
        assert_eq!(out, [r0, r1, r2], "transportEnc_Register*Callback");
        self.has = has;
        self.rrec.has = has;
    }

    /// The callbacks for a `_cb` call: the recorder itself (same behavior as the registered
    /// function pointers).
    fn use_cb(rng: &mut Rng) -> bool {
        rng.range(0, 1) == 0
    }

    fn check(&mut self, what: &str) {
        let mut out = [0i32; 57];
        let mut wr = [0u8; TRANSPORTENC_WRITER_SIZE];
        unsafe { c::cref_tpenc_lib_state(self.c, out.as_mut_ptr(), wr.as_mut_ptr(), self.cbuf.as_mut_ptr()) };
        let hi = unsafe { c::cref_tpenc_lib_cfg_ptr_hi(self.c) };
        let h = self.r.as_ref().unwrap();
        let mut s = [0i32; 57];
        let (v, raw) = cc_to(&h.config);
        s[..23].copy_from_slice(&v);
        for k in 0..16 {
            s[23 + k] = i32::from_ne_bytes([raw[4 * k], raw[4 * k + 1], raw[4 * k + 2], raw[4 * k + 3]]);
        }
        s[39] = h.transport_fmt;
        s[40..48].copy_from_slice(&bs_dump(&h.bit_stream));
        s[48] = h.bs_buffer_size;
        s[49] = h.pce_frame_counter;
        s[50] = h.callbacks.cb_sbr.is_some() as i32;
        s[51] = !h.callbacks.cb_sbr_data.is_null() as i32;
        s[52] = h.callbacks.cb_ssc.is_some() as i32;
        s[53] = !h.callbacks.cb_ssc_data.is_null() as i32;
        s[54] = h.callbacks.cb_usac.is_some() as i32;
        s[55] = !h.callbacks.cb_usac_data.is_null() as i32;
        s[56] = h.bs_buffer.ptr_eq(&self.rbuf) as i32;
        if h.bs_buffer.is_empty() {
            // Never initialized: the C pointer is NULL as well.
            s[56] = out[56];
        }
        assert_eq!(out, s, "{what}: TRANSPORTENC state");
        let rw = h.writer.bytes;
        for i in 0..TRANSPORTENC_WRITER_SIZE {
            if wr[i] != rw[i] {
                // The high half of the LATM configuration pointer (C) where the port has the
                // high half of LATM_CONFIG_SET (0), until a member writes these bytes. (32-bit
                // x86 has no high half: the shim replaces the whole pointer.)
                let tainted = !ILP32
                    && (36..40).contains(&i)
                    && rw[36..40] == [0, 0, 0, 0]
                    && wr[36..40] == hi.to_le_bytes();
                assert!(tainted, "{what}: writer union differs at byte {i}:\nC    {wr:?}\nRust {rw:?}");
            }
        }
        let rbuf = self.rbuf.to_vec();
        if self.cbuf != rbuf {
            let i = (0..BUF).find(|&i| self.cbuf[i] != rbuf[i]).unwrap();
            panic!("{what}: output buffer differs at byte {i}: C {:02x}, Rust {:02x}", self.cbuf[i], rbuf[i]);
        }
        assert_eq!(self.crec.take(), self.rrec.log, "{what}: callbacks");
        self.rrec.log.clear();
    }

    /// Whether bytes 32..40 of the C writer union are in a state that the port does not model
    /// (it has LATM_CONFIG_SET there).
    fn tainted(&mut self) -> bool {
        let mut out = [0i32; 57];
        let mut wr = [0u8; TRANSPORTENC_WRITER_SIZE];
        unsafe { c::cref_tpenc_lib_state(self.c, out.as_mut_ptr(), wr.as_mut_ptr(), self.cbuf.as_mut_ptr()) };
        let hi = unsafe { c::cref_tpenc_lib_cfg_ptr_hi(self.c) };
        if ILP32 {
            // config[0][0] at 28..32 overlays ADTS's headerBits.
            return wr[28..32] == 1u32.to_le_bytes();
        }
        wr[32..40] == 1u64.to_le_bytes() || (hi != 0 && wr[36..40] == hi.to_le_bytes())
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        unsafe { c::cref_tpenc_lib_close(self.c) };
        transport_enc_close(&mut self.r);
        assert!(self.r.is_none());
    }
}

fn rand_fmt(rng: &mut Rng) -> TransportType {
    match rng.range(0, 13) {
        0 => [-1, 3, 12, 5, 11][rng.range(0, 4) as usize],
        1 | 2 => TT_MP4_RAW,
        3 => TT_MP4_ADIF,
        4..=6 => TT_MP4_ADTS,
        7 | 8 => TT_MP4_LATM_MCP1,
        9 | 10 => TT_MP4_LATM_MCP0,
        _ => TT_MP4_LOAS,
    }
}

fn config_for(rng: &mut Rng, fmt: TransportType, has: i32) -> CoderConfig {
    let mut cc = rand_cc(rng);
    match fmt {
        TT_MP4_ADIF | TT_MP4_ADTS => {
            if rng.range(0, 7) != 0 {
                cc.aot = AOT_AAC_LC;
                cc.samples_per_frame = 1024;
            }
            if rng.range(0, 5) != 0 {
                cc.n_sub_frames = rng.range(1, 4);
            }
        }
        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
            if rng.range(0, 5) != 0 {
                cc.aot = [2, 2, 5, 29, 23, 39, 39, 42, 7, 17, 1][rng.range(0, 10) as usize];
            }
            cc.n_sub_frames = match rng.range(0, 5) {
                0 => rng.range(-1, 4),
                1 => 2,
                _ => 1,
            };
        }
        _ => {
            cc.n_sub_frames = rng.range(0, 4);
        }
    }
    if rng.range(0, 2) == 0 {
        cc.sampling_rate = RATES[rng.range(0, 12) as usize];
    }
    if rng.range(0, 2) == 0 {
        cc.channel_mode = MODES[rng.range(1, 15) as usize];
        cc.channel_config_zero = (rng.range(0, 3) == 0) as u8;
    }
    if rng.range(0, 3) == 0 {
        cc.header_period = rng.range(0, 4) as u8;
    }
    if eld_sbr_null_pce(&cc, has, true) {
        cc.channel_mode = MODE_2;
    }
    cc
}

fn init(p: &mut Pair, rng: &mut Rng, fmt: TransportType, cc: &CoderConfig, what: &str) -> i32 {
    // A power of two, as FDK_InitBitBuffer() requires (the AAC encoder allocates one).
    let size = 1 << rng.range(9, 14);
    let flags = match rng.range(0, 3) {
        0 => rng.next_u32(),
        1 => TP_FLAG_LATM_AMV,
        _ => 0,
    };
    let (mut v, mut raw) = cc_to(cc);
    let rc = unsafe {
        c::cref_tpenc_lib_init(p.c, p.cbuf.as_mut_ptr(), size, fmt, v.as_mut_ptr(), raw.as_mut_ptr(), flags)
    };
    let buf = p.rbuf.clone();
    let rr = if Pair::use_cb(rng) {
        let rec = &mut *p.rrec as *mut Rec;
        // SAFETY: the recorder is a separate allocation that lives as long as the pair.
        transport_enc_init_cb(p.r.as_mut().unwrap(), &buf, size, fmt, cc, flags, unsafe { &mut *rec })
    } else {
        transport_enc_init(p.h(), &buf, size, fmt, cc, flags)
    };
    assert_eq!(rc, rr, "{what}: transportEnc_Init(fmt {fmt}, size {size}, flags {flags:x})");
    p.check(&format!("{what}: transportEnc_Init"));
    rr
}

fn get_conf(p: &mut Pair, rng: &mut Rng, what: &str) {
    let mut cc = config_for(rng, p.h().transport_fmt, p.has);
    if rng.range(0, 3) == 0 {
        cc = p.h().config;
    }
    let mut bs = BsPair::new(rng, 4096);
    bs.pat(rng.range(0, 40), rng.next_u32());
    let mut ct: u32 = 7;
    let (rc, ccc) = with_cc(&cc, |v, raw| unsafe { c::cref_tpenc_lib_get_conf(p.c, v, raw, bs.c, &mut ct) });
    let mut rt: u32 = 7;
    let rr = if Pair::use_cb(rng) {
        let rec = &mut *p.rrec as *mut Rec;
        // SAFETY: the recorder is a separate allocation.
        transport_enc_get_conf_cb(p.r.as_mut().unwrap(), &mut cc, &mut bs.r, &mut rt, unsafe { &mut *rec })
    } else {
        transport_enc_get_conf(p.h(), &mut cc, &mut bs.r, &mut rt)
    };
    assert_eq!(rc, rr, "{what}: transportEnc_GetConf");
    assert_eq!(ct, rt, "{what}: transportEnc_GetConf confType");
    assert_eq!(ccc, cc, "{what}: transportEnc_GetConf configuration");
    bs.check(&format!("{what}: transportEnc_GetConf"));
    p.check(&format!("{what}: transportEnc_GetConf"));
}

fn static_bits(p: &mut Pair, rng: &mut Rng, what: &str) {
    let au = match rng.range(0, 4) {
        0 => rng.i32(),
        1 => rng.range(-100, 0),
        _ => 8 * rng.range(0, 1000),
    };
    let rc = unsafe { c::cref_tpenc_lib_static_bits(p.c, au) };
    let rr = transport_enc_get_static_bits(p.h(), au);
    assert_eq!(rc, rr, "{what}: transportEnc_GetStaticBits({au})");
    p.check(&format!("{what}: transportEnc_GetStaticBits"));
}

fn end_au(p: &mut Pair, rng: &mut Rng, what: &str) {
    let vb = unsafe { c::cref_tpenc_lib_valid_bits(p.c) };
    assert_eq!(vb, fdk_get_valid_bits(transport_enc_get_bitstream(p.h())), "{what}: valid bits");
    let b0 = if rng.range(0, 5) == 0 { rng.i32() } else { vb as i32 };
    let (mut bc, mut br) = (b0, b0);
    let rc = unsafe { c::cref_tpenc_lib_end_au(p.c, &mut bc) };
    let rr = transport_enc_end_access_unit(p.h(), &mut br);
    assert_eq!(rc, rr, "{what}: transportEnc_EndAccessUnit");
    assert_eq!(bc, br, "{what}: transportEnc_EndAccessUnit bits");
    p.check(&format!("{what}: transportEnc_EndAccessUnit"));
}

fn get_frame(p: &mut Pair, rng: &mut Rng, what: &str) {
    let n0 = rng.i32();
    let (mut nc, mut nr) = (n0, n0);
    let rc = unsafe { c::cref_tpenc_lib_get_frame(p.c, &mut nc) };
    let rr = transport_enc_get_frame(p.h(), &mut nr);
    assert_eq!(rc, rr, "{what}: transportEnc_GetFrame");
    assert_eq!(nc, nr, "{what}: transportEnc_GetFrame bytes");
    p.check(&format!("{what}: transportEnc_GetFrame"));
}

/// The payload of an access unit with CRC regions (no-ops for formats other than ADTS).
fn payload(p: &mut Pair, rng: &mut Rng, bits: i32, what: &str) {
    let mut open: Vec<i32> = Vec::new();
    let mut left = bits;
    while left > 0 {
        let n = rng.range(1, left.min(400));
        let seed = rng.next_u32();
        unsafe { c::cref_tpenc_lib_payload(p.c, n, seed) };
        pat(transport_enc_get_bitstream(p.h()), n, seed);
        left -= n;
        match rng.range(0, 3) {
            0 if open.len() < 3 => {
                let m = match rng.range(0, 3) {
                    0 => 192,
                    1 => 128,
                    2 => rng.range(-200, 400),
                    _ => 0,
                };
                let rc = unsafe { c::cref_tpenc_lib_crc_start(p.c, m) };
                let rr = transport_enc_crc_start_reg(p.h(), m);
                assert_eq!(rc, rr, "{what}: transportEnc_CrcStartReg({m})");
                p.check(&format!("{what}: transportEnc_CrcStartReg"));
                open.push(rr);
            }
            1 if !open.is_empty() => {
                let reg = open.remove(0);
                unsafe { c::cref_tpenc_lib_crc_end(p.c, reg) };
                transport_enc_crc_end_reg(p.h(), reg);
                p.check(&format!("{what}: transportEnc_CrcEndReg"));
            }
            _ => {}
        }
    }
    for reg in open {
        unsafe { c::cref_tpenc_lib_crc_end(p.c, reg) };
        transport_enc_crc_end_reg(p.h(), reg);
        p.check(&format!("{what}: transportEnc_CrcEndReg"));
    }
    p.check(&format!("{what}: payload"));
}

#[test]
fn sequences() {
    let mut cov = std::collections::BTreeMap::<String, u32>::new();
    let mut hit = |k: String| *cov.entry(k).or_insert(0) += 1;
    let mut rng = Rng::new(0x7e41);
    assert_eq!(unsafe { c::cref_tpenc_lib_writer_size() } as usize, TRANSPORTENC_WRITER_SIZE);
    for it in 0..2500 {
        let mut p = Pair::new(&mut rng);
        p.check(&format!("{it}: transportEnc_Open"));
        let has = rng.range(0, 7);
        p.register(has);
        p.check(&format!("{it}: register"));

        for round in 0..rng.range(1, 4) {
            let what = format!("{it}/{round}");
            if rng.range(0, 5) == 0 {
                let has = rng.range(0, 7);
                p.register(has);
                p.check(&format!("{what}: register"));
            }
            if rng.range(0, 3) == 0 {
                let (s, c2, seed) = (rng.range(0, 60), rng.range(0, 3000), rng.next_u32());
                p.crec.set(s, c2, seed);
                p.rrec.sbr_bits = s;
                p.rrec.ssc_bits = c2;
                p.rrec.seed = seed;
            }
            let fmt = rand_fmt(&mut rng);
            let cc = config_for(&mut rng, fmt, p.has);
            let what = format!("{what} fmt {fmt} aot {} mode {} hp {}", cc.aot, cc.channel_mode, cc.header_period);
            let ri = init(&mut p, &mut rng, fmt, &cc, &what);
            hit(format!("init fmt {fmt} ok {}", ri == 0));
            if ri == 0 && p.h().pce_frame_counter >= 0 {
                hit(format!("pce in raw_data_block fmt {fmt}"));
            }
            if ri == 0 && fmt == TT_MP4_ADTS {
                let a = p.h().writer.adts();
                hit(format!("adts protection {} blocks {}", a.protection_absent == 0, a.num_raw_blocks + 1));
            }
            if ri == 0 && matches!(fmt, TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS) {
                let l = p.h().writer.latm();
                hit(format!("latm fmt {fmt} amv {} subframes {}", l.audio_mux_version, l.no_subframes));
            }
            if fmt == TT_MP4_ADTS && ri != 0 && p.tainted() {
                // The following calls have no defined reference result; skipped.
                continue;
            }

            // Calls out of the encoder's order. Not in the state that `tainted` detects.
            if rng.range(0, 5) == 0 && !(fmt == TT_MP4_ADTS && p.tainted()) {
                end_au(&mut p, &mut rng, &what);
            }
            if rng.range(0, 5) == 0 {
                get_frame(&mut p, &mut rng, &what);
            }

            if rng.range(0, 2) == 0 {
                let n = match fmt {
                    TT_MP4_LATM_MCP0 => 8 * rng.range(1, 4),
                    TT_MP4_LATM_MCP1 | TT_MP4_LOAS => 8 * rng.range(2, 5),
                    _ => rng.range(-10, 100),
                };
                let n = if rng.range(0, 7) == 0 { n + rng.range(1, 7) } else { n };
                let rc = unsafe { c::cref_tpenc_lib_add_other(p.c, n) };
                let rr = transport_enc_add_other_data_bits(p.h(), n);
                assert_eq!(rc, rr, "{what}: transportEnc_AddOtherDataBits({n})");
                hit(format!("other data bits fmt {fmt} ok {}", rr == 0));
                p.check(&format!("{what}: transportEnc_AddOtherDataBits"));
            }
            if rng.range(0, 2) == 0 {
                get_conf(&mut p, &mut rng, &what);
            }

            for frame in 0..rng.range(1, 12) {
                let what = format!("{what} frame {frame}");
                for _ in 0..rng.range(0, 3) {
                    static_bits(&mut p, &mut rng, &what);
                }
                let au_bits = match rng.range(0, 9) {
                    0 => rng.range(-20, 3000),
                    _ => 8 * rng.range(0, 700),
                };
                let total = if rng.range(0, 4) == 0 { rng.i32() } else { au_bits + 8 * rng.range(0, 8) };
                let bf = match rng.range(0, 4) {
                    0 => rng.i32(),
                    1 => rng.range(-10000, 0),
                    _ => rng.range(0, 6144 * 8),
                };
                let mut ncc = match rng.range(0, 6) {
                    0 => rng.range(-3, 9),
                    _ => rng.range(1, 8),
                };
                // No defined x86 reference result (x86 division by zero): only on AArch64.
                if !cfg!(target_arch = "aarch64") && (ncc == 0 || (ncc == -1 && bf == i32::MIN)) {
                    ncc = 1;
                }
                let rc = unsafe { c::cref_tpenc_lib_write_au(p.c, total, bf, ncc) };
                let rr = if Pair::use_cb(&mut rng) {
                    let rec = &mut *p.rrec as *mut Rec;
                    // SAFETY: the recorder is a separate allocation.
                    transport_enc_write_access_unit_cb(p.r.as_deref_mut(), total, bf, ncc, unsafe { &mut *rec })
                } else {
                    transport_enc_write_access_unit(p.r.as_deref_mut(), total, bf, ncc)
                };
                assert_eq!(rc, rr, "{what}: transportEnc_WriteAccessUnit({total}, {bf}, {ncc})");
                p.check(&format!("{what}: transportEnc_WriteAccessUnit"));

                if au_bits > 0 {
                    payload(&mut p, &mut rng, au_bits, &what);
                }
                end_au(&mut p, &mut rng, &what);
                get_frame(&mut p, &mut rng, &what);
                if rng.range(0, 15) == 0 {
                    get_conf(&mut p, &mut rng, &what);
                }
                hit(format!("frame fmt {fmt}"));
            }
        }
    }
    let keys = [
        "init fmt 0 ok true",
        "init fmt 1 ok true",
        "init fmt 2 ok true",
        "init fmt 6 ok true",
        "init fmt 7 ok true",
        "init fmt 10 ok true",
        "init fmt 2 ok false",
        "init fmt 6 ok false",
        "pce in raw_data_block fmt 0",
        "pce in raw_data_block fmt 1",
        "pce in raw_data_block fmt 2",
        "pce in raw_data_block fmt 6",
        "pce in raw_data_block fmt 7",
        "adts protection true blocks 1",
        "adts protection true blocks 4",
        "adts protection false blocks 3",
        "latm fmt 10 amv 1 subframes 2",
        "latm fmt 6 amv 0 subframes 1",
        "latm fmt 7 amv 1 subframes 2",
        "other data bits fmt 10 ok true",
        "other data bits fmt 7 ok true",
    ];
    for k in keys {
        assert!(cov.get(k).copied().unwrap_or(0) > 0, "not covered: {k}\n{cov:#?}");
    }
}

#[test]
fn frame_length_type_after_adts() {
    // LATM MCP0 after ADTS: the port reproduces m_linfo[0][0].frameLengthType from the C layout
    // of the writer union.
    let mut rng = Rng::new(0x7e44);
    let mut cc = CoderConfig::default();
    cc.aot = AOT_AAC_LC;
    cc.samples_per_frame = 1024;
    cc.sampling_rate = 48000;
    cc.ext_sampling_rate = 48000;
    cc.channel_mode = MODE_2;
    cc.no_channels = 2;
    cc.n_sub_frames = 1;
    cc.bit_rate = 128000;
    cc.header_period = 10;
    let mut fresh = Pair::new(&mut rng);
    fresh.register(0);
    init(&mut fresh, &mut rng, TT_MP4_LATM_MCP0, &cc, "fresh MCP0");
    let mut after = Pair::new(&mut rng);
    after.register(0);
    init(&mut after, &mut rng, TT_MP4_ADTS, &cc, "ADTS");
    init(&mut after, &mut rng, TT_MP4_LATM_MCP0, &cc, "MCP0 after ADTS");
    let au = 8 * 4000;
    let (cf, ca) = unsafe { (c::cref_tpenc_lib_static_bits(fresh.c, au), c::cref_tpenc_lib_static_bits(after.c, au)) };
    let (rf, ra) = (transport_enc_get_static_bits(fresh.h(), au), transport_enc_get_static_bits(after.h(), au));
    assert_eq!((cf, ca), (rf, ra));
    assert_eq!(after.h().writer.latm().m_linfo[0][0].frame_length_type, 48000);
    // Measured: the fresh MCP0 counts the PayloadLengthInfo of 4000 bytes (16 bytes: 15 x 255
    // and the rest), the reinitialized one does not.
    assert_eq!((rf, ra), (8 * 16, 0));
}

#[test]
fn null_handles_and_pce_rate() {
    assert_eq!(unsafe { c::cref_tpenc_lib_open_null() }, TRANSPORTENC_INVALID_PARAMETER);
    let mut out = [0i32; 3];
    unsafe { c::cref_tpenc_lib_register(core::ptr::null_mut(), core::ptr::null_mut(), 7, out.as_mut_ptr()) };
    let r0 = transport_enc_register_sbr_callback(None, Some(rec_sbr_fn), core::ptr::null_mut());
    let r1 = transport_enc_register_ssc_callback(None, Some(rec_ssc_fn), core::ptr::null_mut());
    let r2 = transport_enc_register_usac_callback(None, Some(rec_usac_fn), core::ptr::null_mut());
    assert_eq!(out, [r0, r1, r2]);
    assert_eq!(
        unsafe { c::cref_tpenc_lib_write_au(core::ptr::null_mut(), 0, 0, 1) },
        transport_enc_write_access_unit(None, 0, 0, 1)
    );

    let mut rng = Rng::new(0x7e42);
    let fmts = [-1, 0, 1, 2, 3, 6, 7, 10, 12];
    for _ in 0..30000 {
        let mode = super::tpenc_asc::rand_mode(&mut rng);
        let czero = rng.range(0, 2);
        let fmt = fmts[rng.range(0, fmts.len() as i32 - 1) as usize];
        let hp = match rng.range(0, 3) {
            0 => rng.i32(),
            _ => rng.range(-2, 10),
        };
        let mma = rng.range(-1, 5);
        assert_eq!(
            unsafe { c::cref_tpenc_lib_pce_rate(mode, czero, fmt, hp, mma) },
            get_pce_repetition_rate(mode, czero, fmt, hp, mma),
            "getPceRepetitionRate({mode}, {czero}, {fmt}, {hp}, {mma})"
        );
    }
}

#[test]
fn lib_info() {
    for prefill in [0, 1, 5, FDK_MODULE_LAST - 1, FDK_MODULE_LAST] {
        for null in [0, 1] {
            let mut out = [0i32; 4];
            let mut title = [0u8; 32];
            let mut vstr = [0u8; 32];
            let rc = unsafe {
                c::cref_tpenc_lib_lib_info(prefill, null, out.as_mut_ptr(), title.as_mut_ptr(), vstr.as_mut_ptr())
            };
            let mut info = vec![LibInfo::default(); FDK_MODULE_LAST as usize];
            fdk_init_lib_info(&mut info);
            for (i, e) in info.iter_mut().enumerate().take(prefill as usize) {
                e.module_id = 100 + i as i32;
            }
            let rr = transport_enc_get_lib_info(if null != 0 { None } else { Some(&mut info) });
            assert_eq!(rc, rr, "transportEnc_GetLibInfo({prefill}, {null})");
            let idx = info.iter().position(|e| e.module_id == FDK_TPENC).map(|i| i as i32).unwrap_or(-1);
            assert_eq!(out[1], idx);
            if idx >= 0 {
                let e = &info[idx as usize];
                assert_eq!(out[2], e.version);
                assert_eq!(out[3], e.flags as i32);
                let mut t = [0u8; 32];
                t[..e.title.len()].copy_from_slice(e.title.as_bytes());
                assert_eq!(title, t);
                // C leaves the bytes after the terminating NUL as they were.
                let n = e.version_str.iter().position(|&b| b == 0).unwrap() + 1;
                assert_eq!(vstr[..n], e.version_str[..n]);
            }
        }
    }
}

#[test]
fn raw_frames_after_other_formats() {
    // The writers read union fields that an earlier format wrote (reproduced from the C
    // layout); run every pair of formats with calls before the first WriteAccessUnit.
    let mut rng = Rng::new(0x7e43);
    let fmts = [TT_MP4_RAW, TT_MP4_ADIF, TT_MP4_ADTS, TT_MP4_LATM_MCP1, TT_MP4_LATM_MCP0, TT_MP4_LOAS];
    for it in 0..400 {
        let mut p = Pair::new(&mut rng);
        p.register(rng.range(0, 3));
        for (k, &fmt) in [fmts[it % 6], fmts[(it / 6) % 6], fmts[rng.range(0, 5) as usize]].iter().enumerate() {
            let mut cc = config_for(&mut rng, fmt, p.has);
            if matches!(fmt, TT_MP4_ADIF | TT_MP4_ADTS) {
                cc.aot = AOT_AAC_LC;
                cc.samples_per_frame = 1024;
            }
            let what = format!("{it}/{k} fmt {fmt}");
            let ri = init(&mut p, &mut rng, fmt, &cc, &what);
            if fmt == TT_MP4_ADTS && ri != 0 && p.tainted() {
                continue;
            }
            static_bits(&mut p, &mut rng, &what);
            if !(fmt == TT_MP4_ADTS && p.tainted()) {
                end_au(&mut p, &mut rng, &what);
            }
            get_frame(&mut p, &mut rng, &what);
            for f in 0..3 {
                let what = format!("{what} frame {f}");
                let au = 8 * rng.range(0, 300);
                let rc = unsafe { c::cref_tpenc_lib_write_au(p.c, au, 1000, 2) };
                let rr = transport_enc_write_access_unit(p.r.as_deref_mut(), au, 1000, 2);
                assert_eq!(rc, rr, "{what}: transportEnc_WriteAccessUnit");
                p.check(&what);
                payload(&mut p, &mut rng, au, &what);
                end_au(&mut p, &mut rng, &what);
                get_frame(&mut p, &mut rng, &what);
            }
        }
    }
}

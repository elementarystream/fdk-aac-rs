// Differential tests of fdk_aac_rs_tpenc::tpenc_asc against the C++ reference.
//
// getChannelConfig(), transportEnc_GetChannelMode(), getPceEntry() and transportEnc_GetPCEBits()
// over all channel modes and random values; transportEnc_writePCE(), writeAot(),
// writeSampleRate(), the GA/ELD/USAC specific configurations and transportEnc_writeASC() on
// random configurations (every AOT, channel mode, signaling mode, flag, downscaled ELD, LD-SAC
// with SSC lengths of every escape size, SBR headers, raw USAC configurations), written after a
// random number of bits into a buffer with random content. The return value, the bitstream
// state, the whole buffer, the configuration afterwards and the callback log are compared.
//
// This file also holds the helpers of the other tpenc tests: the recorder callbacks, the
// pattern writer, CODER_CONFIG conversion and generation, and bitstream pairs.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use core::ffi::c_void;

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpenc_tpenc_asc as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, fdk_write_bits, FdkBitStream, BS_WRITER};
use fdk_aac_rs_tpenc::tpenc_asc::*;
use fdk_aac_rs_tpenc::tpenc_lib::TpEncCallbacks;

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

/// `tpenc_pat()` of the shim.
pub fn pat(bs: &mut FdkBitStream, mut nbits: i32, seed: u32) {
    let mut x = seed;
    while nbits > 0 {
        let n = nbits.min(32);
        x = x.wrapping_mul(1103515245).wrapping_add(12345);
        fdk_write_bits(bs, x.rotate_right(3), n as u32);
        nbits -= n;
    }
}

/// `tpenc_bs_dump()` of the shim.
pub fn bs_dump(bs: &FdkBitStream) -> [i32; 8] {
    [
        bs.cache_word as i32,
        bs.bits_in_cache as i32,
        bs.h_bit_buf.valid_bits as i32,
        bs.h_bit_buf.read_offset as i32,
        bs.h_bit_buf.write_offset as i32,
        bs.h_bit_buf.bit_ndx as i32,
        bs.h_bit_buf.buf_size() as i32,
        bs.config_cache as i32,
    ]
}

/// `CODER_CONFIG` as the int array of the shim.
pub fn cc_to(cc: &CoderConfig) -> ([i32; 23], [u8; 64]) {
    (
        [
            cc.aot,
            cc.ext_aot,
            cc.channel_mode,
            cc.channel_config_zero as i32,
            cc.sampling_rate,
            cc.ext_sampling_rate,
            cc.downscale_sampling_rate,
            cc.bit_rate,
            cc.samples_per_frame,
            cc.no_channels,
            cc.bits_frame,
            cc.n_sub_frames,
            cc.bsac_num_of_sub_frame,
            cc.bsac_layer_length,
            cc.flags as i32,
            cc.matrix_mixdown_a as i32,
            cc.header_period as i32,
            cc.stereo_config_index as i32,
            cc.sbr_mode as i32,
            cc.sbr_signaling,
            cc.raw_config_bits,
            cc.sbr_present as i32,
            cc.ps_present as i32,
        ],
        cc.raw_config,
    )
}

/// The recorder callbacks (`TpEncRec` of the shim).
#[derive(Default)]
pub struct Rec {
    pub log: Vec<i32>,
    pub has: i32,
    pub sbr_bits: i32,
    pub ssc_bits: i32,
    pub seed: u32,
}

impl TpEncCallbacks for Rec {
    fn has_sbr(&self) -> bool {
        self.has & 1 != 0
    }

    fn sbr(
        &mut self,
        hbs: &mut FdkBitStream,
        sample_rate_in: i32,
        sample_rate_out: i32,
        samples_per_frame: i32,
        core_codec: AudioObjectType,
        element_id: Mp4ElementId,
        element_index: i32,
        harmonic_sbr: u8,
        stereo_config_index: u8,
        config_mode: u8,
        config_changed: Option<&mut u8>,
        downscale_factor: i32,
    ) -> i32 {
        let vb = fdk_get_valid_bits(hbs) as i32;
        self.log.extend_from_slice(&[
            1,
            sample_rate_in,
            sample_rate_out,
            samples_per_frame,
            core_codec,
            element_id,
            element_index,
            harmonic_sbr as i32,
            stereo_config_index as i32,
            config_mode as i32,
            config_changed.is_none() as i32,
            downscale_factor,
            vb,
        ]);
        pat(hbs, self.sbr_bits + element_index, self.seed.wrapping_add(element_index as u32));
        0
    }

    fn has_ssc(&self) -> bool {
        self.has & 2 != 0
    }

    fn ssc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        core_codec: AudioObjectType,
        sampling_rate: i32,
        frame_size: i32,
        num_channels: i32,
        stereo_config_index: i32,
        core_sbr_frame_length_index: i32,
        config_bytes: i32,
        config_mode: u8,
        config_changed: Option<&mut u8>,
    ) -> i32 {
        let is_null = hbs.is_none();
        let mut hbs = hbs;
        let vb = match hbs.as_deref_mut() {
            Some(b) => fdk_get_valid_bits(b) as i32,
            None => -1,
        };
        self.log.extend_from_slice(&[
            2,
            is_null as i32,
            core_codec,
            sampling_rate,
            frame_size,
            num_channels,
            stereo_config_index,
            core_sbr_frame_length_index,
            config_bytes,
            config_mode as i32,
            config_changed.is_none() as i32,
            vb,
        ]);
        if let Some(b) = hbs {
            pat(b, self.ssc_bits, self.seed);
        }
        self.ssc_bits
    }
}

/// `tpenc_rec_sbr` as a registered function pointer (`user` is a `*mut Rec`).
pub fn rec_sbr_fn(
    user: *mut c_void,
    hbs: &mut FdkBitStream,
    a: i32,
    b: i32,
    c: i32,
    d: AudioObjectType,
    e: Mp4ElementId,
    f: i32,
    g: u8,
    h: u8,
    i: u8,
    j: Option<&mut u8>,
    k: i32,
) -> i32 {
    // SAFETY: the tests register a pointer to a live `Rec`.
    let r = unsafe { &mut *(user as *mut Rec) };
    r.sbr(hbs, a, b, c, d, e, f, g, h, i, j, k)
}

/// `tpenc_rec_ssc` as a registered function pointer (`user` is a `*mut Rec`).
pub fn rec_ssc_fn(
    user: *mut c_void,
    hbs: Option<&mut FdkBitStream>,
    a: AudioObjectType,
    b: i32,
    c: i32,
    d: i32,
    e: i32,
    f: i32,
    g: i32,
    h: u8,
    i: Option<&mut u8>,
) -> i32 {
    // SAFETY: the tests register a pointer to a live `Rec`.
    let r = unsafe { &mut *(user as *mut Rec) };
    r.ssc(hbs, a, b, c, d, e, f, g, h, i)
}

/// `tpenc_rec_usac` as a registered function pointer (`user` is a `*mut Rec`).
pub fn rec_usac_fn(user: *mut c_void, hbs: &mut FdkBitStream) -> i32 {
    // SAFETY: the tests register a pointer to a live `Rec`.
    let r = unsafe { &mut *(user as *mut Rec) };
    let vb = fdk_get_valid_bits(hbs) as i32;
    r.log.extend_from_slice(&[3, vb]);
    0
}

/// The C recorder.
pub struct CRec(pub *mut c_void);

impl CRec {
    pub fn new(sbr_bits: i32, ssc_bits: i32, seed: u32) -> Self {
        CRec(unsafe { c::cref_tpenc_rec_new(sbr_bits, ssc_bits, seed) })
    }
    pub fn set(&self, sbr_bits: i32, ssc_bits: i32, seed: u32) {
        unsafe { c::cref_tpenc_rec_set(self.0, sbr_bits, ssc_bits, seed) }
    }
    pub fn take(&self) -> Vec<i32> {
        let mut v = vec![0i32; 1 << 16];
        let n = unsafe { c::cref_tpenc_rec_take(self.0, v.as_mut_ptr(), v.len() as i32) };
        v.truncate(n as usize);
        v
    }
}

impl Drop for CRec {
    fn drop(&mut self) {
        unsafe { c::cref_tpenc_rec_free(self.0) }
    }
}

/// A recorder pair with the same parameters.
pub fn rec_pair(rng: &mut Rng) -> (CRec, Rec) {
    let has = rng.range(0, 7);
    let sbr_bits = rng.range(0, 60);
    let ssc_bits = match rng.range(0, 5) {
        0 => rng.range(0, 8 * 14),
        1 => rng.range(8 * 14, 8 * 15 + 8),
        2 => rng.range(8 * 15, 8 * (15 + 255) + 8),
        3 => rng.range(8 * (15 + 254), 4000),
        4 => 0,
        _ => rng.range(1, 9),
    };
    let seed = rng.next_u32();
    (CRec::new(sbr_bits, ssc_bits, seed), Rec { log: Vec::new(), has, sbr_bits, ssc_bits, seed })
}

/// A writer bitstream on a buffer with random content, on both sides.
pub struct BsPair {
    pub c: *mut c_void,
    pub cbuf: Vec<u8>,
    pub rbuf: BitMem,
    pub r: FdkBitStream,
}

impl BsPair {
    pub fn new(rng: &mut Rng, size: usize) -> Self {
        let fill = rng.range(0, 2);
        let mut cbuf = vec![0u8; size];
        for b in cbuf.iter_mut() {
            *b = match fill {
                0 => 0,
                1 => 0xFF,
                _ => rng.next_u32() as u8,
            };
        }
        let rbuf = BitMem::from_slice(&cbuf);
        let c = unsafe { c::cref_tpenc_bs_new(cbuf.as_mut_ptr(), size as u32) };
        let mut r = FdkBitStream::default();
        fdk_init_bit_stream(&mut r, &rbuf, size as u32, 0, BS_WRITER);
        BsPair { c, cbuf, rbuf, r }
    }

    /// Writes `nbits` pattern bits on both sides.
    pub fn pat(&mut self, nbits: i32, seed: u32) {
        unsafe { c::cref_tpenc_bs_pat(self.c, nbits, seed) };
        pat(&mut self.r, nbits, seed);
    }

    pub fn c_dump(&self) -> [i32; 8] {
        let mut d = [0i32; 8];
        unsafe { c::cref_tpenc_bs_dump(self.c, d.as_mut_ptr()) };
        d
    }

    /// Compares the bitstream states and the buffers.
    pub fn check(&self, what: &str) {
        assert_eq!(self.c_dump(), bs_dump(&self.r), "{what}: bitstream state");
        let rbuf = self.rbuf.to_vec();
        if self.cbuf != rbuf {
            let i = (0..self.cbuf.len()).find(|&i| self.cbuf[i] != rbuf[i]).unwrap();
            panic!("{what}: buffer differs at byte {i}: C {:02x}, Rust {:02x}", self.cbuf[i], rbuf[i]);
        }
    }
}

impl Drop for BsPair {
    fn drop(&mut self) {
        unsafe { c::cref_tpenc_bs_free(self.c) }
    }
}

pub const RATES: [i32; 13] =
    [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

pub const MODES: [i32; 18] = [-1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 33, 34, 128, 9];

/// A sampling rate: table rates, rates without index, 0 and random values.
pub fn rand_rate(rng: &mut Rng) -> i32 {
    match rng.range(0, 9) {
        0 => 0,
        1 => rng.range(1, 200000),
        2 => rng.i32(),
        3 => [57600, 51200, 40000, 38400, 34150, 28800, 25600, 20000, 19200][rng.range(0, 8) as usize],
        _ => RATES[rng.range(0, 12) as usize],
    }
}

pub fn rand_mode(rng: &mut Rng) -> i32 {
    match rng.range(0, 9) {
        0 => rng.i32(),
        _ => MODES[rng.range(0, MODES.len() as i32 - 1) as usize],
    }
}

/// A random CODER_CONFIG covering every field.
pub fn rand_cc(rng: &mut Rng) -> CoderConfig {
    let mut cc = CoderConfig::default();
    const AOTS: [i32; 30] = [
        2, 2, 2, 5, 29, 23, 23, 39, 39, 39, 42, 1, 3, 4, 6, 7, 8, 9, 17, 19, 20, 21, 22, 24, 25,
        26, 27, 0, 31, 129,
    ];
    cc.aot = match rng.range(0, 19) {
        0 => rng.range(-2, 160),
        1 => [32, 33, 44, 132, -1][rng.range(0, 4) as usize],
        _ => AOTS[rng.range(0, AOTS.len() as i32 - 1) as usize],
    };
    cc.ext_aot = match rng.range(0, 5) {
        0 => rng.range(-2, 60),
        1 => 0,
        2 => 29,
        3 => 2,
        _ => 5,
    };
    cc.channel_mode = rand_mode(rng);
    cc.channel_config_zero = match rng.range(0, 7) {
        0 => 1,
        1 => rng.next_u32() as u8,
        _ => 0,
    };
    cc.sampling_rate = rand_rate(rng);
    cc.ext_sampling_rate = if rng.range(0, 2) == 0 { cc.sampling_rate } else { rand_rate(rng) };
    cc.downscale_sampling_rate = match rng.range(0, 4) {
        0 | 1 => 0,
        2 => cc.ext_sampling_rate,
        _ => rand_rate(rng),
    };
    cc.bit_rate = match rng.range(0, 3) {
        0 => rng.i32(),
        _ => rng.range(0, 1 << 23),
    };
    cc.samples_per_frame = match rng.range(0, 12) {
        0 => rng.i32(),
        k => [1024, 960, 512, 480, 256, 240, 128, 120, 160, 64, 60, 1024][(k - 1) as usize],
    };
    cc.no_channels = rng.range(-1, 10);
    cc.bits_frame = match rng.range(0, 6) {
        0 => rng.i32(),
        1 => [40, 80][rng.range(0, 1) as usize],
        2 => [154, 170, 338, 216, 0, 110][rng.range(0, 5) as usize],
        3 => rng.range(-10, 200),
        _ => rng.range(0, 6000),
    };
    cc.n_sub_frames = match rng.range(0, 6) {
        0 => rng.i32(),
        _ => rng.range(0, 5),
    };
    cc.bsac_num_of_sub_frame = rng.range(-2, 40);
    cc.bsac_layer_length = rng.range(-2, 3000);
    let flag_bits = [
        CC_MPEG_ID,
        CC_IS_BASELAYER,
        CC_PROTECTION,
        CC_SBR,
        CC_SBRCRC,
        CC_SAC,
        CC_RVLC,
        CC_VCB11,
        CC_HCR,
        CC_PSEUDO_SURROUND,
        CC_USAC_NOISE,
        CC_USAC_TW,
        CC_USAC_HBE,
    ];
    cc.flags = if rng.range(0, 9) == 0 {
        rng.next_u32()
    } else {
        flag_bits.iter().fold(0, |f, &b| if rng.range(0, 2) == 0 { f | b } else { f })
    };
    cc.matrix_mixdown_a = match rng.range(0, 7) {
        0 => rng.next_u32() as u8,
        k => (k - 1) as u8,
    };
    cc.header_period = match rng.range(0, 7) {
        0 => rng.next_u32() as u8,
        1 => 0,
        _ => rng.range(1, 10) as u8,
    };
    cc.stereo_config_index = rng.next_u32() as u8;
    cc.sbr_mode = rng.next_u32() as u8;
    cc.sbr_signaling = rng.range(-1, 3);
    for b in cc.raw_config.iter_mut() {
        *b = rng.next_u32() as u8;
    }
    cc.raw_config_bits = match rng.range(0, 7) {
        0 => rng.i32(),
        1 => [-1, 0, 512, 513, i32::MAX, i32::MAX - 6][rng.range(0, 5) as usize],
        _ => rng.range(1, 512),
    };
    cc.sbr_present = match rng.range(0, 6) {
        0 => rng.next_u32() as u8,
        1 | 2 => 0,
        _ => 1,
    };
    cc.ps_present = match rng.range(0, 4) {
        0 => rng.next_u32() as u8,
        1 => 0,
        _ => 1,
    };
    cc
}

/// Whether this configuration has no defined reference result (not tested).
pub fn eld_sbr_null_pce(cc: &CoderConfig, has: i32, via_asc: bool) -> bool {
    let mut mode = cc.channel_mode;
    if via_asc && mode == MODE_INVALID {
        mode = fdk_aac_rs_tpenc::tpenc_lib::transport_enc_get_channel_mode(cc.no_channels);
    }
    cc.aot == AOT_ER_AAC_ELD && (cc.flags & CC_SBR) != 0 && (has & 1) != 0 && get_pce_entry(mode).is_none()
}

/// Calls a C function on the int-array form of `cc` and returns the configuration afterwards.
pub fn with_cc<R>(cc: &CoderConfig, f: impl FnOnce(*mut i32, *mut u8) -> R) -> (R, CoderConfig) {
    let (mut v, mut raw) = cc_to(cc);
    let r = f(v.as_mut_ptr(), raw.as_mut_ptr());
    let mut out = *cc;
    out.aot = v[0];
    out.ext_aot = v[1];
    out.channel_mode = v[2];
    out.channel_config_zero = v[3] as u8;
    out.sampling_rate = v[4];
    out.ext_sampling_rate = v[5];
    out.downscale_sampling_rate = v[6];
    out.bit_rate = v[7];
    out.samples_per_frame = v[8];
    out.no_channels = v[9];
    out.bits_frame = v[10];
    out.n_sub_frames = v[11];
    out.bsac_num_of_sub_frame = v[12];
    out.bsac_layer_length = v[13];
    out.flags = v[14] as u32;
    out.matrix_mixdown_a = v[15] as u8;
    out.header_period = v[16] as u8;
    out.stereo_config_index = v[17] as u8;
    out.sbr_mode = v[18] as u8;
    out.sbr_signaling = v[19];
    out.raw_config_bits = v[20];
    out.sbr_present = v[21] as u8;
    out.ps_present = v[22] as u8;
    out.raw_config = raw;
    (r, out)
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn channel_config_mode_pce_bits() {
    let mut rng = Rng::new(0x7e01);
    let mut modes: Vec<i32> = (-3..=40).collect();
    modes.extend_from_slice(&[128, 129, 255, 256, i32::MIN, i32::MAX]);
    for _ in 0..200 {
        modes.push(rng.i32());
    }
    for &m in &modes {
        for czero in [0, 1, 2, 255] {
            assert_eq!(
                unsafe { c::cref_tpenc_asc_get_channel_config(m, czero) },
                get_channel_config(m, czero as u8),
                "getChannelConfig({m}, {czero})"
            );
        }
        // getPceEntry
        let mut out = [0i32; 18];
        let found = unsafe { c::cref_tpenc_asc_get_pce_entry(m, out.as_mut_ptr()) };
        match get_pce_entry(m) {
            None => assert_eq!(found, 0, "getPceEntry({m})"),
            Some(p) => {
                assert_eq!(found, 1, "getPceEntry({m})");
                let n = (p.num_front_channel_elements
                    + p.num_side_channel_elements
                    + p.num_back_channel_elements
                    + p.num_lfe_channel_elements) as usize;
                let mut r = [0i32; 18];
                r[0] = p.num_front_channel_elements as i32;
                r[1] = p.num_side_channel_elements as i32;
                r[2] = p.num_back_channel_elements as i32;
                r[3] = p.num_lfe_channel_elements as i32;
                for i in 0..8 {
                    r[4 + i] = if i < n { p.p_el_type[i] } else { -9 };
                }
                for i in 0..6 {
                    r[12 + i] = match p.p_height_num {
                        None => -1,
                        Some(h) => [
                            h.num_front_height_channel_elements[0],
                            h.num_front_height_channel_elements[1],
                            h.num_side_height_channel_elements[0],
                            h.num_side_height_channel_elements[1],
                            h.num_back_height_channel_elements[0],
                            h.num_back_height_channel_elements[1],
                        ][i] as i32,
                    };
                }
                assert_eq!(out, r, "getPceEntry({m})");
                assert_eq!(p.p_el_type.len(), n, "element list length of mode {m}");
            }
        }
        for mma in [0, 1, 2, 3, 4, 5, 255, -1] {
            for bits in [0, 3, 7, 8, 13, 100, -5, i32::MAX - 300, rng.i32()] {
                assert_eq!(
                    unsafe { c::cref_tpenc_asc_get_pce_bits(m, mma, bits) },
                    transport_enc_get_pce_bits(m, mma, bits),
                    "transportEnc_GetPCEBits({m}, {mma}, {bits})"
                );
            }
        }
    }
    for n in (-5..=20).chain([i32::MIN, i32::MAX, 100]) {
        assert_eq!(
            unsafe { c::cref_tpenc_asc_get_channel_mode(n) },
            transport_enc_get_channel_mode(n),
            "transportEnc_GetChannelMode({n})"
        );
    }
}

#[test]
fn write_pce() {
    let mut rng = Rng::new(0x7e02);
    for it in 0..6000 {
        let mut bs = BsPair::new(&mut rng, 256);
        let pre = rng.range(0, 70);
        bs.pat(pre, rng.next_u32());
        let anchor = match rng.range(0, 3) {
            0 => 0,
            1 => pre as u32,
            2 => rng.range(0, pre) as u32,
            _ => rng.next_u32(),
        };
        let mode = rand_mode(&mut rng);
        let sr = rand_rate(&mut rng);
        let tag = rng.range(-1, 17);
        let profile = rng.range(-1, 4);
        let mma = rng.range(-1, 6);
        let ps = rng.range(-1, 2);
        let rc = unsafe { c::cref_tpenc_asc_write_pce(bs.c, mode, sr, tag, profile, mma, ps, anchor) };
        let rr = transport_enc_write_pce(&mut bs.r, mode, sr, tag, profile, mma, ps, anchor);
        assert_eq!(rc, rr, "{it}: transportEnc_writePCE return");
        bs.check(&format!("{it}: transportEnc_writePCE(mode {mode}, sr {sr})"));
    }
    // Every mode with a valid rate, with and without mixdown.
    for &mode in &MODES {
        for mma in 0..5 {
            for pre in 0..9 {
                let mut bs = BsPair::new(&mut rng, 128);
                bs.pat(pre, 7);
                let rc = unsafe { c::cref_tpenc_asc_write_pce(bs.c, mode, 48000, 3, 1, mma, 1, 0) };
                let rr = transport_enc_write_pce(&mut bs.r, mode, 48000, 3, 1, mma, 1, 0);
                assert_eq!(rc, rr);
                bs.check(&format!("transportEnc_writePCE(mode {mode}, mma {mma}, pre {pre})"));
            }
        }
    }
}

#[test]
fn write_aot_sample_rate() {
    let mut rng = Rng::new(0x7e03);
    for it in 0..5000 {
        let mut bs = BsPair::new(&mut rng, 64);
        bs.pat(rng.range(0, 40), rng.next_u32());
        if rng.range(0, 1) == 0 {
            let aot = match rng.range(0, 3) {
                0 => rng.i32(),
                _ => rng.range(-3, 100),
            };
            unsafe { c::cref_tpenc_asc_write_aot(bs.c, aot) };
            write_aot(&mut bs.r, aot);
            bs.check(&format!("{it}: writeAot({aot})"));
        } else {
            let sr = rand_rate(&mut rng);
            let nbits = rng.range(1, 5);
            unsafe { c::cref_tpenc_asc_write_sample_rate(bs.c, sr, nbits) };
            write_sample_rate(&mut bs.r, sr, nbits);
            bs.check(&format!("{it}: writeSampleRate({sr}, {nbits})"));
        }
    }
}

/// Which writer `specific_configs` calls.
#[derive(Clone, Copy, Debug)]
enum Which {
    Ga,
    Eld,
    Usac,
    Asc,
}

/// Returns a coverage key of the call.
fn run_config_writer(rng: &mut Rng, which: Which, it: usize) -> String {
    let mut cc = rand_cc(rng);
    if matches!(which, Which::Eld) {
        cc.aot = AOT_ER_AAC_ELD;
    }
    let (crec, mut rrec) = rec_pair(rng);
    if (matches!(which, Which::Eld) || matches!(which, Which::Asc))
        && eld_sbr_null_pce(&cc, rrec.has, matches!(which, Which::Asc))
    {
        // No defined result in C; use a mode with a PCE configuration.
        cc.channel_mode = MODE_2;
    }
    let mut bs = BsPair::new(rng, 4096);
    let pre = rng.range(0, 64);
    bs.pat(pre, rng.next_u32());
    let anchor = match rng.range(0, 2) {
        0 => pre as u32,
        1 => 0,
        _ => rng.range(0, pre) as u32,
    };
    let ext = rng.range(0, 1);
    let has = rrec.has;
    let (rc, ccc) = with_cc(&cc, |v, raw| unsafe {
        match which {
            Which::Ga => c::cref_tpenc_asc_write_ga(bs.c, v, raw, ext, anchor),
            Which::Eld => c::cref_tpenc_asc_write_eld(bs.c, v, raw, 0, crec.0, has),
            Which::Usac => c::cref_tpenc_asc_write_usac(bs.c, v, raw, ext, crec.0, has),
            Which::Asc => c::cref_tpenc_asc_write_asc(bs.c, v, raw, crec.0, has),
        }
    });
    let mut rcc = cc;
    let rr = match which {
        Which::Ga => transport_enc_write_ga_specific_config(&mut bs.r, &rcc, ext, anchor),
        Which::Eld => transport_enc_write_eld_specific_config(&mut bs.r, &rcc, 0, &mut rrec),
        Which::Usac => transport_enc_write_usac_specific_config(&mut bs.r, ext, &rcc, &mut rrec),
        Which::Asc => transport_enc_write_asc(&mut bs.r, &mut rcc, &mut rrec),
    };
    let what = format!("{it}: {which:?} aot {} mode {} flags {:x}", cc.aot, cc.channel_mode, cc.flags);
    assert_eq!(rc, rr, "{what}: return");
    bs.check(&what);
    assert_eq!(ccc, rcc, "{what}: configuration afterwards");
    assert_eq!(crec.take(), rrec.log, "{what}: callbacks");
    let ssc_len = rrec.ssc_bits.wrapping_add(7) >> 3;
    format!(
        "{which:?} aot {} ret {rr} sbr {} ssc {} sig {}",
        cc.aot,
        rrec.log.first() == Some(&1),
        if rrec.log.contains(&2) { if ssc_len < 15 { 1 } else if ssc_len < 15 + 255 { 2 } else { 3 } } else { 0 },
        cc.sbr_signaling
    )
}

#[test]
fn specific_configs() {
    let mut rng = Rng::new(0x7e04);
    for it in 0..3000 {
        run_config_writer(&mut rng, Which::Ga, it);
        run_config_writer(&mut rng, Which::Eld, it);
        run_config_writer(&mut rng, Which::Usac, it);
    }
}

#[test]
fn write_asc() {
    let mut rng = Rng::new(0x7e05);
    let mut cov = std::collections::BTreeSet::new();
    for it in 0..20000 {
        cov.insert(run_config_writer(&mut rng, Which::Asc, it));
    }
    for k in [
        "Asc aot 2 ret 0 sbr false ssc 0 sig 0",
        "Asc aot 2 ret 0 sbr false ssc 0 sig 1",
        "Asc aot 2 ret 0 sbr false ssc 0 sig 2",
        "Asc aot 23 ret 0 sbr false ssc 0 sig 0",
        "Asc aot 39 ret 0 sbr true ssc 0 sig 0",
        "Asc aot 39 ret 0 sbr true ssc 1 sig 0",
        "Asc aot 39 ret 0 sbr true ssc 2 sig 1",
        "Asc aot 39 ret 0 sbr false ssc 3 sig 2",
        "Asc aot 42 ret 0 sbr false ssc 0 sig 0",
        "Asc aot 42 ret 3 sbr false ssc 0 sig 0",
        "Asc aot 8 ret -1 sbr false ssc 0 sig 0",
    ] {
        assert!(cov.contains(k), "not covered: {k}\n{cov:#?}");
    }
}

/// The file-static tables of tpenc_asc.cpp (a copy compiled into the shim), element by element:
/// ELTYPE_1 ... ELTYPE_7_1_FRONT_CENTER (`elType_*`), HEIGHT_NUM_7_1_TOP_FRONT
/// (`heightNum_7_1_top_front`), and PCE_CONFIG_TAB (`pceConfigTab`) with PCE_CONFIG_TAB_FLAT:
/// channel mode, element numbers, the `elType_*` table each entry points to (by name, so the
/// index order of ELTYPES is checked) and the height table.
#[test]
fn asc_tables() {
    let tabs: [(&str, &[i32]); 12] = [
        ("elType_1", &ELTYPE_1),
        ("elType_2", &ELTYPE_2),
        ("elType_1_2", &ELTYPE_1_2),
        ("elType_1_2_1", &ELTYPE_1_2_1),
        ("elType_1_2_2", &ELTYPE_1_2_2),
        ("elType_1_2_2_1", &ELTYPE_1_2_2_1),
        ("elType_1_2_2_2_1", &ELTYPE_1_2_2_2_1),
        ("elType_6_1", &ELTYPE_6_1),
        ("elType_7_1_back", &ELTYPE_7_1_BACK),
        ("elType_7_1_top_front", &ELTYPE_7_1_TOP_FRONT),
        ("elType_7_1_rear_surround", &ELTYPE_7_1_REAR_SURROUND),
        ("elType_7_1_front_center", &ELTYPE_7_1_FRONT_CENTER),
    ];
    let table = |name: &str| -> Vec<u8> {
        let cname = std::ffi::CString::new(name).unwrap();
        let mut bytes = 0;
        let p = unsafe { c::cref_tpenc_asc_table(cname.as_ptr(), &mut bytes) };
        assert!(!p.is_null(), "{name}");
        unsafe { std::slice::from_raw_parts(p as *const u8, bytes as usize) }.to_vec()
    };
    for (k, (name, r)) in tabs.iter().enumerate() {
        let b = table(name);
        let cv: Vec<i32> = b.chunks(4).map(|w| i32::from_ne_bytes(w.try_into().unwrap())).collect();
        assert_eq!(&cv[..], *r, "{name}");
        assert!(core::ptr::eq(ELTYPES[k], *r), "ELTYPES[{k}]");
    }
    assert_eq!(table("heightNum_7_1_top_front"), HEIGHT_NUM_7_1_TOP_FRONT.to_vec());
    let mut i = 0;
    loop {
        let mut out = [0i32; 6];
        let name = unsafe { c::cref_tpenc_asc_pce_config_tab(i, out.as_mut_ptr()) };
        if name.is_null() {
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_str().unwrap();
        let k = tabs.iter().position(|t| t.0 == name).unwrap_or_else(|| panic!("pceConfigTab[{i}]: {name}"));
        let f = &PCE_CONFIG_TAB_FLAT[7 * i as usize..7 * i as usize + 7];
        assert_eq!(&f[..5], &out[..5], "pceConfigTab[{i}]");
        assert_eq!((f[5], f[6]), (k as i32, out[5]), "pceConfigTab[{i}]");
        let e = &PCE_CONFIG_TAB[i as usize];
        let p = &e.pce_configuration;
        assert_eq!(
            [e.channel_mode, p.num_front_channel_elements as i32, p.num_side_channel_elements as i32,
             p.num_back_channel_elements as i32, p.num_lfe_channel_elements as i32],
            out[..5],
            "PCE_CONFIG_TAB[{i}]"
        );
        assert!(core::ptr::eq(p.p_el_type, tabs[k].1), "PCE_CONFIG_TAB[{i}].p_el_type");
        assert_eq!(p.p_height_num.is_some() as i32, out[5], "PCE_CONFIG_TAB[{i}].p_height_num");
        i += 1;
    }
    assert_eq!(i as usize, PCE_CONFIG_TAB.len());
}

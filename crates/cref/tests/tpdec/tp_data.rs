// Differential tests of fdk_aac_rs_tpdec::tp_data against the C++ reference.
//
// Also holds the helpers shared by the other tpdec tests:
// - `Rec`: recording callbacks (TpDecCallbacks) with the behavior of the C++ recorder of
//   shim/tpdec_tp_data.cpp, and `Recs`, a Rust recorder plus its C++ counterpart;
// - `asc_dump()`: the dump of a CSAudioSpecificConfig, as `tpdec_asc_dump()` of the shim;
// - `Bs`: a bitstream kept twice (Rust value; C++ state array and memory).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use core::ffi::c_void;

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpdec_tp_data as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_init_bit_stream, fdk_push_for, fdk_read_bit, fdk_read_bits,
    FdkBitStream, BS_READER,
};
use fdk_aac_rs_tpdec::tp_data::*;
use fdk_aac_rs_tpdec::tpdec_asc::audio_specific_config_init;

// ---------------------------------------------------------------------------------------------
// ASC dump
// ---------------------------------------------------------------------------------------------

fn asc_active(aot: AudioObjectType) -> i32 {
    match aot {
        AOT_AAC_LC | AOT_AAC_SCAL | AOT_ER_AAC_LC | AOT_ER_AAC_LD | AOT_ER_AAC_SCAL
        | AOT_ER_BSAC => 1,
        AOT_ER_AAC_ELD => 2,
        AOT_USAC => 3,
        _ => 0,
    }
}

/// `tpdec_asc_dump()` of shim/tpdec_tp_data.cpp.
pub fn asc_dump(a: &CSAudioSpecificConfig) -> Vec<i32> {
    let mut o = vec![
        a.m_aot,
        a.m_sampling_frequency as i32,
        a.m_samples_per_frame as i32,
        a.m_direct_mapping as i32,
        a.m_extension_audio_object_type,
        a.m_extension_sampling_frequency as i32,
        a.m_channel_configuration as i32,
        a.m_ep_config as i32,
        a.m_vcb11_flag as i32,
        a.m_rvlc_flag as i32,
        a.m_hcr_flag as i32,
        a.m_sbr_present_flag as i32,
        a.m_ps_present_flag as i32,
        a.m_sampling_frequency_index as i32,
        a.m_extension_sampling_frequency_index as i32,
        a.m_extension_channel_configuration as i32,
        a.config_mode as i32,
        a.aac_config_changed as i32,
        a.sbr_config_changed as i32,
        a.sac_config_changed as i32,
        a.config_bits as i32,
    ];
    o.extend(a.config.iter().map(|&b| b as i32));
    let act = asc_active(a.m_aot);
    o.push(act);
    match act {
        1 => {
            let g = &a.m_sc.m_ga_specific_config;
            o.extend([
                g.m_frame_length_flag,
                g.m_depends_on_core_coder,
                g.m_core_coder_delay,
                g.m_extension_flag,
                g.m_extension_flag3,
                g.m_layer,
                g.m_num_of_sub_frame,
                g.m_layer_length,
            ]
            .iter()
            .map(|&v| v as i32));
        }
        2 => {
            let e = &a.m_sc.m_eld_specific_config;
            o.extend([
                e.m_frame_length_flag as i32,
                e.m_sbr_present_flag as i32,
                e.m_use_ld_qmf_time_align as i32,
                e.m_sbr_sampling_rate as i32,
                e.m_sbr_crc_flag as i32,
                e.m_downscaled_sampling_frequency as i32,
            ]);
        }
        3 => {
            let u = &a.m_sc.m_usac_config;
            o.extend([
                u.m_frame_length_flag as i32,
                u.m_core_sbr_frame_length_index as i32,
                u.m_sbr_ratio_index as i32,
                u.m_n_usac_channels as i32,
                u.m_channel_configuration_index as i32,
                u.m_usac_num_elements as i32,
            ]);
            for e in u.element.iter() {
                o.extend([
                    e.usac_element_type,
                    e.m_noise_filling as i32,
                    e.m_harmonic_sbr as i32,
                    e.m_inter_tes as i32,
                    e.m_pvc as i32,
                    e.m_stereo_config_index as i32,
                    e.ext_element.usac_ext_element_type,
                    e.ext_element.usac_ext_element_config_length as i32,
                    e.ext_element.usac_ext_element_default_length as i32,
                    e.ext_element.usac_ext_element_payload_frag as i32,
                    e.ext_element.usac_ext_element_has_audio_pre_roll as i32,
                ]);
            }
            o.extend([
                u.num_audio_channels as i32,
                u.m_usac_config_extension_present as i32,
                u.element_length_present as i32,
                u.usac_config_bits as i32,
            ]);
            o.extend(u.usac_config.iter().map(|&b| b as i32));
        }
        _ => {}
    }
    o.extend(a.m_progr_config_element.as_bytes().iter().map(|&b| b as i32));
    o
}

/// `tpdec_hash()`: FNV-1a over the ints.
pub fn hash(v: &[i32]) -> u32 {
    let mut h: u32 = 2166136261;
    for &x in v {
        h ^= x as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// Size of the dump buffers for the shims.
pub const DUMP_MAX: usize = 4096;

/// Compares two ASC dumps and names the first difference.
pub fn assert_dump_eq(rs: &[i32], c: &[i32], what: &str) {
    if rs != c {
        let i = rs.iter().zip(c.iter()).position(|(a, b)| a != b).unwrap_or(rs.len().min(c.len()));
        panic!(
            "{what}: ASC dumps differ at index {i} (rs {:?}, c {:?}; lengths {} / {})",
            rs.get(i),
            c.get(i),
            rs.len(),
            c.len()
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Recording callbacks
// ---------------------------------------------------------------------------------------------

/// Recording callbacks with the behavior of `TpRec` of shim/tpdec_tp_data.cpp.
pub struct Rec {
    pub log: Vec<i32>,
    rets: [i32; 64],
    ccset: [i32; 64],
    ncalls: usize,
    has: i32,
}

fn skip_sbr(hbs: &mut FdkBitStream, is_usac: bool) {
    if !is_usac {
        fdk_push_for(hbs, 6);
    }
    fdk_push_for(hbs, 8);
    let e1 = fdk_read_bit(hbs);
    let e2 = fdk_read_bit(hbs);
    fdk_push_for(hbs, 5 * e1 + 6 * e2);
}

impl Rec {
    pub fn new(rets: &[i32; 64], ccset: &[i32; 64], has: i32) -> Rec {
        Rec { log: Vec::new(), rets: *rets, ccset: *ccset, ncalls: 0, has }
    }

    fn ret(&mut self, cc: Option<&mut u8>) -> i32 {
        let k = self.ncalls & 63;
        self.ncalls += 1;
        if let Some(cc) = cc {
            if self.ccset[k] == 1 {
                *cc = 1;
            } else if self.ccset[k] == 2 {
                *cc = 0;
            }
            self.log.push(*cc as i32);
        }
        self.log.push(self.rets[k]);
        self.rets[k]
    }
}

impl TpDecCallbacks for Rec {
    fn update_config(
        &mut self,
        asc: &CSAudioSpecificConfig,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32 {
        self.log.extend([1, config_mode as i32, *config_changed as i32, hash(&asc_dump(asc)) as i32]);
        self.ret(Some(config_changed))
    }

    fn free_mem(&mut self, asc: &CSAudioSpecificConfig) -> i32 {
        self.log.extend([6, hash(&asc_dump(asc)) as i32]);
        self.ret(None)
    }

    fn ctrl_cfg_change(&mut self, c: &CCtrlCFGChange) -> i32 {
        self.log.extend([
            5,
            c.flush_cnt as i32,
            c.flush_status as i32,
            c.build_up_cnt as i32,
            c.build_up_status as i32,
            c.cfg_changed as i32,
            c.content_changed as i32,
            c.force_cfg_change as i32,
        ]);
        self.ret(None)
    }

    fn has_ssc(&self) -> bool {
        self.has & 1 != 0
    }

    fn ssc(
        &mut self,
        hbs: &mut FdkBitStream,
        core_codec: AudioObjectType,
        sampling_rate: i32,
        frame_size: i32,
        num_channels: i32,
        stereo_config_index: i32,
        core_sbr_frame_length_index: i32,
        config_bytes: i32,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32 {
        self.log.extend([
            3,
            core_codec,
            sampling_rate,
            frame_size,
            num_channels,
            stereo_config_index,
            core_sbr_frame_length_index,
            config_bytes,
            config_mode as i32,
            *config_changed as i32,
            fdk_get_valid_bits(hbs) as i32,
        ]);
        if config_bytes > 0 {
            fdk_push_for(hbs, (config_bytes as u32).wrapping_mul(8));
        }
        self.ret(Some(config_changed))
    }

    fn has_sbr(&self) -> bool {
        self.has & 2 != 0
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
        config_changed: &mut u8,
        downscale_factor: i32,
    ) -> i32 {
        self.log.extend([
            2,
            sample_rate_in,
            sample_rate_out,
            samples_per_frame,
            core_codec,
            element_id,
            element_index,
            harmonic_sbr as i32,
            stereo_config_index as i32,
            config_mode as i32,
            *config_changed as i32,
            downscale_factor,
            fdk_get_valid_bits(hbs) as i32,
        ]);
        skip_sbr(hbs, core_codec != AOT_ER_AAC_ELD);
        self.ret(Some(config_changed))
    }

    fn has_usac(&self) -> bool {
        self.has & 4 != 0
    }

    fn usac(&mut self, _hbs: &mut FdkBitStream) -> i32 {
        self.log.push(7);
        self.ret(None)
    }

    fn has_uni_drc(&self) -> bool {
        self.has & 8 != 0
    }

    fn uni_drc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        full_payload_length: i32,
        payload_type: i32,
        sub_stream_index: i32,
        payload_start: i32,
        aot: AudioObjectType,
    ) -> i32 {
        match hbs {
            Some(b) => {
                let v = fdk_get_valid_bits(b) as i32;
                self.log.extend([4, v, full_payload_length, payload_type, sub_stream_index, payload_start, aot]);
                if full_payload_length > 0 {
                    fdk_push_for(b, (full_payload_length.wrapping_mul(8) as u32) / 2);
                }
            }
            None => {
                self.log.extend([4, -1, full_payload_length, payload_type, sub_stream_index, payload_start, aot]);
            }
        }
        self.ret(None)
    }
}

/// A random callback script: mostly successful calls, sometimes errors and config changes.
pub fn random_script(rng: &mut Rng, error_rate: u32) -> ([i32; 64], [i32; 64]) {
    let mut rets = [0i32; 64];
    let mut ccset = [0i32; 64];
    for i in 0..64 {
        if error_rate > 0 && rng.next_u32() % error_rate == 0 {
            rets[i] = [1, -1, 0x203, 0x402, 0x401, 0x201, 0x101][(rng.next_u32() % 7) as usize];
        }
        ccset[i] = match rng.next_u32() % 4 {
            0 => 1,
            1 => 2,
            _ => 0,
        };
    }
    (rets, ccset)
}

/// A Rust recorder and its C++ counterpart.
pub struct Recs {
    pub rs: Rec,
    pub c: *mut c_void,
}

impl Recs {
    pub fn new(rets: &[i32; 64], ccset: &[i32; 64], has: i32) -> Recs {
        Recs { rs: Rec::new(rets, ccset, has), c: unsafe { c::cref_tpdec_rec_new(rets.as_ptr(), ccset.as_ptr(), has) } }
    }

    pub fn random(error_rate: u32, rng: &mut Rng) -> Recs {
        let (rets, ccset) = random_script(rng, error_rate);
        let has = match rng.next_u32() % 4 {
            0 => (rng.next_u32() & 15) as i32,
            _ => 15,
        };
        Recs::new(&rets, &ccset, has)
    }

    pub fn c_log(&self) -> Vec<i32> {
        let n = unsafe { c::cref_tpdec_rec_log(self.c, core::ptr::null_mut(), 0) } as usize;
        let mut v = vec![0i32; n];
        unsafe { c::cref_tpdec_rec_log(self.c, v.as_mut_ptr(), n as i32) };
        v
    }

    /// Compares the logs of both recorders and clears them.
    pub fn check(&mut self, what: &str) {
        let cl = self.c_log();
        if self.rs.log != cl {
            let i = self.rs.log.iter().zip(cl.iter()).position(|(a, b)| a != b).unwrap_or(self.rs.log.len().min(cl.len()));
            let lo = i.saturating_sub(16);
            panic!(
                "{what}: callback logs differ at {i}:\n rs {:?}\n c  {:?}",
                &self.rs.log[lo..(i + 16).min(self.rs.log.len())],
                &cl[lo..(i + 16).min(cl.len())]
            );
        }
        self.rs.log.clear();
        unsafe { c::cref_tpdec_rec_clear(self.c) };
    }
}

impl Drop for Recs {
    fn drop(&mut self) {
        unsafe { c::cref_tpdec_rec_free(self.c) };
    }
}

// ---------------------------------------------------------------------------------------------
// Bitstream pair
// ---------------------------------------------------------------------------------------------

/// The state array of the shims (layout of shim/fdk_bitstream.cpp).
pub fn bs_state(bs: &FdkBitStream) -> [u32; 9] {
    let b = &bs.h_bit_buf;
    [
        bs.cache_word,
        bs.bits_in_cache,
        bs.config_cache,
        b.valid_bits,
        b.read_offset,
        b.write_offset,
        b.bit_ndx,
        b.buf_size(),
        b.buf_bits(),
    ]
}

/// A reader bitstream kept twice: as a Rust value with its memory, and as a C++ state array with
/// its own copy of the memory.
pub struct Bs {
    pub rs: FdkBitStream,
    pub rbuf: BitMem,
    pub cst: [u32; 9],
    pub cbuf: Vec<u8>,
}

impl Bs {
    /// A bitstream over `data` (zero padded to a power of two of at least `min_size` bytes) with
    /// `valid_bits` valid bits. With `rng`, the start position, the cache and the offsets are
    /// randomized (the data is rotated accordingly, so the same bits are read).
    pub fn new(data: &[u8], valid_bits: u32, min_size: usize, rng: Option<&mut Rng>) -> Bs {
        let size = (data.len().max(min_size).max(4)).next_power_of_two();
        let mut buf = vec![0u8; size];
        let rs = FdkBitStream::default();
        match rng {
            None => {
                buf[..data.len()].copy_from_slice(data);
                let mut b = Bs { rs, rbuf: BitMem::from_slice(&buf), cst: [0; 9], cbuf: Vec::new() };
                fdk_init_bit_stream(&mut b.rs, &b.rbuf, size as u32, 0, BS_READER);
                // (set directly: the number may exceed the buffer, which FDK_InitBitBuffer()
                // asserts against)
                b.rs.h_bit_buf.valid_bits = valid_bits;
                b.sync_c();
                b
            }
            Some(rng) => {
                // Place the data at a random byte offset and read some leading bits so that the
                // cache holds data.
                let off = (rng.next_u32() as usize) % size;
                let lead = (rng.next_u32() % 5) as usize; // extra leading bytes
                let total = data.len() + lead;
                let mut lin = vec![0u8; total];
                for b in lin[..lead].iter_mut() {
                    *b = rng.next_u32() as u8;
                }
                lin[lead..].copy_from_slice(data);
                for (i, &v) in lin.iter().enumerate().take(size) {
                    buf[(off + i) % size] = v;
                }
                let mut b = Bs { rs, rbuf: BitMem::from_slice(&buf), cst: [0; 9], cbuf: Vec::new() };
                fdk_init_bit_stream(&mut b.rs, &b.rbuf, size as u32, 0, BS_READER);
                b.rs.h_bit_buf.valid_bits = valid_bits.wrapping_add(lead as u32 * 8);
                b.rs.h_bit_buf.read_offset = off as u32;
                b.rs.h_bit_buf.bit_ndx = (off as u32) * 8;
                b.rs.h_bit_buf.write_offset = ((off + total) % size) as u32;
                // skip the leading bytes through the cache
                let mut left = lead as u32 * 8;
                while left > 0 {
                    let n = left.min(1 + rng.next_u32() % 32);
                    fdk_read_bits(&mut b.rs, n);
                    left -= n;
                }
                b.sync_c();
                b
            }
        }
    }

    /// Copies the Rust state and memory to the C++ side.
    pub fn sync_c(&mut self) {
        self.cst = bs_state(&self.rs);
        self.cbuf = self.rbuf.to_vec();
    }

    pub fn check(&self, what: &str) {
        assert_eq!(bs_state(&self.rs), self.cst, "{what}: bitstream state");
        assert_eq!(self.rbuf.to_vec(), self.cbuf, "{what}: bitstream memory");
    }
}

// ---------------------------------------------------------------------------------------------
// Tests of tp_data.h
// ---------------------------------------------------------------------------------------------

#[test]
fn sampling_rate_table_() {
    let n = unsafe { c::cref_tp_data_sampling_rate_table_len() } as usize;
    assert_eq!(n, SAMPLING_RATE_TABLE.len());
    for i in 0..n {
        assert_eq!(SAMPLING_RATE_TABLE[i], unsafe { c::cref_tp_data_sampling_rate_table(i as i32) }, "entry {i}");
        assert_eq!(sampling_rate_table(i), SAMPLING_RATE_TABLE[i]);
    }
    assert_eq!(sampling_rate_table(n), 0);
}

#[test]
fn get_sampling_rate_index_() {
    let mut rng = Rng::new(1);
    for n_bits in 0..=5u32 {
        let mut rates: Vec<u32> = SAMPLING_RATE_TABLE.to_vec();
        rates.extend([1, 7349, 7351, 95999, 96001, 0xFFFF_FFFF]);
        for _ in 0..200 {
            rates.push(rng.next_u32() % 200_000);
        }
        for &r in &rates {
            let e = unsafe { c::cref_tp_data_get_sampling_rate_index(r, n_bits) };
            assert_eq!(get_sampling_rate_index(r, n_bits), e, "rate {r} nBits {n_bits}");
        }
    }
}

#[test]
fn channel_counts() {
    for cc in -20..=40 {
        assert_eq!(
            get_number_of_total_channels(cc),
            unsafe { c::cref_tp_data_get_number_of_total_channels(cc) },
            "getNumberOfTotalChannels({cc})"
        );
    }
    // getNumberOfEffectiveChannels() indexes a local array of 16 entries unchecked.
    for cc in 0..=15 {
        assert_eq!(
            get_number_of_effective_channels(cc),
            unsafe { c::cref_tp_data_get_number_of_effective_channels(cc) },
            "getNumberOfEffectiveChannels({cc})"
        );
    }
}

#[test]
fn layout() {
    let mut l = [0i32; 10];
    unsafe { c::cref_tp_data_layout(l.as_mut_ptr()) };
    assert_eq!(l[0] as usize, CPROGRAMCONFIG_SIZE, "sizeof(CProgramConfig)");
    assert_eq!(l[0] as usize, core::mem::size_of::<CProgramConfig>());
    assert_eq!(l[1] as usize, PCE_OFFSET_FRONT_ELEMENT_IS_CPE);
    assert_eq!(l[2] as usize, PCE_OFFSET_SIDE_ELEMENT_IS_CPE);
    assert_eq!(l[3] as usize, PCE_OFFSET_BACK_ELEMENT_IS_CPE);
    let p = CProgramConfig::zeroed();
    let base = &p as *const _ as usize;
    assert_eq!(l[4] as usize, p.comment.as_ptr() as usize - base, "offsetof Comment");
    assert_eq!(l[5] as usize, &p.el_counter as *const u8 as usize - base, "offsetof elCounter");
    // The union argument of tp_data.rs: usacExtElementHasAudioPreRoll of element 0 lies behind
    // the GA and ELD members.
    assert_eq!(l[7], 33);
    assert_eq!(l[8], 32);
    assert_eq!(l[9], 12);
}

#[test]
fn asc_init() {
    let mut cd = vec![0i32; DUMP_MAX];
    let n = unsafe { c::cref_tp_data_asc_dump_init(0, cd.as_mut_ptr()) } as usize;
    assert_dump_eq(&asc_dump(&CSAudioSpecificConfig::zeroed()), &cd[..n], "zeroed");
    assert_eq!(CSAudioSpecificConfig::default(), CSAudioSpecificConfig::zeroed());
    let n = unsafe { c::cref_tp_data_asc_dump_init(1, cd.as_mut_ptr()) } as usize;
    let mut a = CSAudioSpecificConfig::zeroed();
    audio_specific_config_init(&mut a);
    assert_dump_eq(&asc_dump(&a), &cd[..n], "AudioSpecificConfig_Init");
}

/// The function-pointer callbacks (CSTpCallBacks) forward to the registered functions.
#[test]
fn cs_tp_callbacks() {
    use std::sync::atomic::{AtomicI32, Ordering};
    static CALLS: AtomicI32 = AtomicI32::new(0);
    fn upd(d: *mut c_void, _a: &CSAudioSpecificConfig, m: u8, cc: &mut u8) -> i32 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        *cc = m;
        d as usize as i32
    }
    let mut cb = CSTpCallBacks::default();
    assert!(!cb.has_sbr() && !cb.has_ssc() && !cb.has_usac() && !cb.has_uni_drc());
    let a = CSAudioSpecificConfig::zeroed();
    let mut cc = 0u8;
    // Not registered: skipped, result 0.
    assert_eq!(cb.update_config(&a, 2, &mut cc), 0);
    assert_eq!(cc, 0);
    cb.cb_update_config = Some(upd);
    cb.cb_update_config_data = 7usize as *mut c_void;
    assert_eq!(cb.update_config(&a, 2, &mut cc), 7);
    assert_eq!(cc, 2);
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------------------------
// Bit writer for test streams
// ---------------------------------------------------------------------------------------------

/// MSB-first bit writer for generated test streams.
#[derive(Clone, Default)]
pub struct BitWriter {
    pub bytes: Vec<u8>,
    pub nbits: usize,
}

impl BitWriter {
    pub fn new() -> BitWriter {
        BitWriter::default()
    }

    /// Writes the `n` (0..=32) low bits of `v`.
    pub fn put(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            let bit = (v >> i) & 1;
            if self.nbits % 8 == 0 {
                self.bytes.push(0);
            }
            if bit != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.nbits % 8);
            }
            self.nbits += 1;
        }
    }

    /// `escapedValue()` syntax: `v` with fields of `n1`, `n2`, `n3` bits.
    pub fn escaped(&mut self, v: u32, n1: u32, n2: u32, n3: u32) {
        let m1 = (1u32 << n1) - 1;
        if v < m1 {
            self.put(v, n1);
            return;
        }
        self.put(m1, n1);
        let m2 = (1u32 << n2) - 1;
        let r = v - m1;
        if r < m2 || n3 == 0 {
            self.put(r.min(m2), n2);
            return;
        }
        self.put(m2, n2);
        self.put((r - m2).min((1u32 << n3) - 1), n3);
    }

    /// Pads with zero bits to a byte boundary.
    pub fn align(&mut self) {
        while self.nbits % 8 != 0 {
            self.put(0, 1);
        }
    }

    pub fn random_bytes(&mut self, n: usize, rng: &mut Rng) {
        for _ in 0..n {
            self.put(rng.next_u32() & 0xff, 8);
        }
    }
}

/// Flips `n` random bits of `data`.
pub fn flip_bits(data: &mut [u8], n: u32, rng: &mut Rng) {
    if data.is_empty() {
        return;
    }
    for _ in 0..n {
        let i = (rng.next_u32() as usize) % data.len();
        data[i] ^= 1 << (rng.next_u32() % 8);
    }
}

// Differential tests of fdk_aac_rs_tpdec::tpdec_latm against the C++ reference.
//
// Sessions of a CLatmDemux and its ASC array on both sides run random sequences of
// ReadStreamMuxConfig, Read (AudioMuxElement) and ReadPayloadLengthInfo on generated
// StreamMuxConfigs (with the ASCs of tpdec_asc.rs, USAC among them), their mutations and random
// bytes, starting from random demultiplexer states. After every call the return value, the
// configFound flag, the demultiplexer state, both ASCs, the bitstream and the callback log are
// compared, and the getters.

use core::ffi::c_void;

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpdec_tpdec_latm as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_tpdec::tp_data::*;
use fdk_aac_rs_tpdec::tpdec_asc::{audio_specific_config_init, audio_specific_config_parse};
use fdk_aac_rs_tpdec::tpdec_latm::*;

use super::tp_data::{asc_dump, assert_dump_eq, flip_bits, BitWriter, Bs, Recs, DUMP_MAX};
use super::tpdec_asc::gen_asc;

/// The state array of shim/tpdec_tpdec_latm.cpp.
pub fn latm_state(d: &CLatmDemux) -> Vec<i32> {
    let l = &d.m_linfo[0][0];
    vec![
        l.m_frame_length_type as i32,
        l.m_buffer_fullness as i32,
        l.m_stream_id as i32,
        l.m_frame_length_in_bits as i32,
        d.m_tara_buffer_fullness as i32,
        d.m_other_data_length as i32,
        d.m_audio_mux_length_bytes as i32,
        d.m_use_same_stream_mux as i32,
        d.m_audio_mux_version as i32,
        d.m_audio_mux_version_a as i32,
        d.m_all_streams_same_time_framing as i32,
        d.m_no_sub_frames as i32,
        d.m_num_program as i32,
        d.m_num_layer[0] as i32,
        d.m_other_data_present as i32,
        d.m_crc_check_present as i32,
        d.buffer_fullness_achieved as i32,
        d.usac_explicit_cfg_changed as i32,
        d.apply_asc as i32,
        d.new_cfg_has_audio_pre_roll as i32,
    ]
}

fn set_state(d: &mut CLatmDemux, s: &[i32]) {
    let l = &mut d.m_linfo[0][0];
    l.m_frame_length_type = s[0] as u32;
    l.m_buffer_fullness = s[1] as u32;
    l.m_stream_id = s[2] as u32;
    l.m_frame_length_in_bits = s[3] as u32;
    d.m_tara_buffer_fullness = s[4] as u32;
    d.m_other_data_length = s[5] as u32;
    d.m_audio_mux_length_bytes = s[6] as u32;
    d.m_use_same_stream_mux = s[7] as u8;
    d.m_audio_mux_version = s[8] as u8;
    d.m_audio_mux_version_a = s[9] as u8;
    d.m_all_streams_same_time_framing = s[10] as u8;
    d.m_no_sub_frames = s[11] as u8;
    d.m_num_program = s[12] as u8;
    d.m_num_layer[0] = s[13] as u8;
    d.m_other_data_present = s[14] as u8;
    d.m_crc_check_present = s[15] as u8;
    d.buffer_fullness_achieved = s[16] as i8;
    d.usac_explicit_cfg_changed = s[17] as u8;
    d.apply_asc = s[18] as u8;
    d.new_cfg_has_audio_pre_roll = s[19] as u8;
}

struct Sess {
    d: CLatmDemux,
    asc: [CSAudioSpecificConfig; 2],
    recs: Recs,
    c: *mut c_void,
}

impl Sess {
    fn new(recs: Recs) -> Sess {
        let c = unsafe { c::cref_latm_new(recs.c) };
        Sess { d: CLatmDemux::default(), asc: [CSAudioSpecificConfig::zeroed(); 2], recs, c }
    }

    fn check(&mut self, what: &str) {
        let mut cs = [0i32; 20];
        unsafe { c::cref_latm_get_state(self.c, cs.as_mut_ptr()) };
        assert_eq!(latm_state(&self.d), cs, "{what}: state");
        for i in 0..2 {
            let mut cd = vec![0i32; DUMP_MAX];
            let n = unsafe { c::cref_latm_asc(self.c, i, cd.as_mut_ptr()) } as usize;
            assert_dump_eq(&asc_dump(&self.asc[i as usize]), &cd[..n], &format!("{what}: asc[{i}]"));
        }
        self.recs.check(what);
        for (which, a, b) in [(0, 0, 0), (0, 0, 1), (0, 1, 0), (1, 0, 0), (2, 0, 0), (3, 0, 0), (4, 0, 0), (4, 1, 0)] {
            let r = match which {
                0 => clatm_demux_get_frame_length_in_bits(&self.d, a, b),
                1 => clatm_demux_get_other_data_present_flag(&self.d),
                2 => clatm_demux_get_other_data_length(&self.d),
                3 => clatm_demux_get_nr_of_sub_frames(&self.d),
                _ => clatm_demux_get_nr_of_layers(&self.d, a),
            };
            assert_eq!(r, unsafe { c::cref_latm_getter(self.c, which, a, b) }, "{what}: getter {which}({a}, {b})");
        }
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        unsafe { c::cref_latm_free(self.c) };
    }
}

/// A random demultiplexer state of the shape the parser produces (one program, one layer at
/// most).
fn random_state(rng: &mut Rng) -> Vec<i32> {
    let mut s: Vec<i32> = (0..20).map(|_| (rng.next_u32() % 4) as i32).collect();
    s[0] = (rng.next_u32() % 8) as i32;
    s[1] = if rng.next_u32() % 2 == 0 { 0xff } else { (rng.next_u32() & 0xff) as i32 };
    s[3] = (rng.next_u32() % 4000) as i32;
    s[5] = (rng.next_u32() % 100) as i32;
    s[6] = (rng.next_u32() % 2000) as i32;
    s[11] = 1 + (rng.next_u32() % 4) as i32;
    s[12] = (rng.next_u32() % 2) as i32;
    s[13] = (rng.next_u32() % 2) as i32;
    s[18] = (rng.next_u32() % 2) as i32;
    s
}

fn latm_value(w: &mut BitWriter, v: u32) {
    let n = if v < 0x100 { 0 } else if v < 0x10000 { 1 } else { 2 };
    w.put(n, 2);
    for i in (0..=n).rev() {
        w.put((v >> (8 * i)) & 0xff, 8);
    }
}

/// A StreamMuxConfig (followed by PayloadLengthInfo and some payload).
fn gen_smc_and_payload(rng: &mut Rng, w: &mut BitWriter) {
    let mut asc = BitWriter::new();
    gen_asc(rng, &mut asc);
    let ver = (rng.next_u32() % 3 == 0) as u32;
    w.put(ver, 1);
    if ver == 1 {
        w.put((rng.next_u32() % 16 == 0) as u32, 1);
        latm_value(w, rng.next_u32() & 0xff);
    }
    let same = if rng.next_u32() % 16 == 0 { 0 } else { 1 };
    w.put(same, 1);
    w.put(rng.next_u32() % 4, 6);
    w.put(if rng.next_u32() % 32 == 0 { 1 } else { 0 }, 4);
    w.put(if rng.next_u32() % 32 == 0 { 1 } else { 0 }, 3);
    if ver == 1 {
        latm_value(w, if rng.next_u32() % 16 == 0 { rng.next_u32() % 400 } else { asc.nbits as u32 });
    }
    for i in 0..asc.nbits {
        w.put(((asc.bytes[i / 8] >> (7 - i % 8)) & 1) as u32, 1);
    }
    let flt = if rng.next_u32() % 12 == 0 { rng.next_u32() & 7 } else { 0 };
    w.put(flt, 3);
    if flt == 0 {
        w.put(if rng.next_u32() % 2 == 0 { 0xff } else { rng.next_u32() & 0xff }, 8);
    } else if flt == 1 {
        w.put(rng.next_u32() & 0x1ff, 9);
    }
    let other = rng.next_u32() % 6 == 0;
    w.put(other as u32, 1);
    if other {
        if ver == 1 {
            latm_value(w, rng.next_u32() % 64);
        } else {
            let esc = rng.next_u32() % 4 == 0;
            w.put(esc as u32, 1);
            w.put(rng.next_u32() & 0xff, 8);
            if esc {
                w.put(0, 1);
                w.put(rng.next_u32() & 0xff, 8);
            }
        }
    }
    let crc = rng.next_u32() % 4 == 0;
    w.put(crc as u32, 1);
    if crc {
        w.put(rng.next_u32() & 0xff, 8);
    }
    // PayloadLengthInfo and payload
    let len = (rng.next_u32() % 600) as usize;
    let mut l = len;
    while l >= 255 {
        w.put(255, 8);
        l -= 255;
    }
    w.put(l as u32, 8);
    w.random_bytes(len.min(40), rng);
}

#[test]
fn latm_sequences() {
    let mut rng = Rng::new(0x1a7);
    for it in 0..4000 {
        let recs = Recs::random(if rng.next_u32() % 3 == 0 { 4 } else { 0 }, &mut rng);
        let mut s = Sess::new(recs);
        // initial state
        let st = random_state(&mut rng);
        set_state(&mut s.d, &st);
        unsafe { c::cref_latm_set_state(s.c, st.as_ptr()) };
        audio_specific_config_init(&mut s.asc[0]);
        audio_specific_config_init(&mut s.asc[1]);
        unsafe { c::cref_latm_asc_init(s.c) };
        if rng.next_u32() % 2 == 0 {
            // a previous configuration in asc[0]
            let mut w = BitWriter::new();
            gen_asc(&mut rng, &mut w);
            let mut bs = Bs::new(&w.bytes, w.nbits as u32, 64, None);
            let r = audio_specific_config_parse(&mut s.asc[0], &mut bs.rs, 1, &mut s.recs.rs, AC_CM_ALLOC_MEM, 0, AOT_NULL_OBJECT);
            let e = unsafe { c::cref_latm_asc_parse(s.c, 0, bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr()) };
            assert_eq!(r, e);
        }
        s.check(&format!("{it} init"));
        for step in 0..6 {
            let mut w = BitWriter::new();
            if rng.next_u32() % 8 == 0 {
                w.random_bytes(1 + (rng.next_u32() % 40) as usize, &mut rng);
            } else {
                if rng.next_u32() % 2 == 0 {
                    w.put((rng.next_u32() % 4 == 0) as u32, 1); // useSameStreamMux for Read
                }
                gen_smc_and_payload(&mut rng, &mut w);
            }
            let mut bytes = w.bytes.clone();
            if rng.next_u32() % 4 == 0 {
                flip_bits(&mut bytes, 1 + rng.next_u32() % 3, &mut rng);
            }
            let bits = if rng.next_u32() % 6 == 0 { rng.next_u32() % (w.nbits as u32 + 1) } else { bytes.len() as u32 * 8 };
            let mut bs = Bs::new(&bytes, bits, 64, if rng.next_u32() % 3 == 0 { Some(&mut rng) } else { None });
            let mut found = [0, 1, 32][(rng.next_u32() % 3) as usize];
            let mut cfound = found;
            let what = format!("{it}/{step}");
            match rng.next_u32() % 3 {
                0 => {
                    let mode = [AC_CM_DET_CFG_CHANGE, AC_CM_ALLOC_MEM][(rng.next_u32() % 2) as usize];
                    let changed = (rng.next_u32() % 2) as u8;
                    let r = clatm_demux_read_stream_mux_config(&mut bs.rs, &mut s.d, &mut s.recs.rs, &mut s.asc, &mut found, mode, changed);
                    let e = unsafe {
                        c::cref_latm_read_stream_mux_config(s.c, bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), &mut cfound, mode, changed)
                    };
                    assert_eq!(r, e, "{what}: ReadStreamMuxConfig");
                }
                1 => {
                    let tt = [TT_MP4_LATM_MCP0, TT_MP4_LATM_MCP1, TT_MP4_LOAS][(rng.next_u32() % 3) as usize];
                    let ign = (rng.next_u32() % 2) as i32;
                    let r = clatm_demux_read(&mut bs.rs, &mut s.d, tt, &mut s.recs.rs, &mut s.asc, &mut found, ign);
                    let e = unsafe { c::cref_latm_read(s.c, bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), tt, &mut cfound, ign) };
                    assert_eq!(r, e, "{what}: Read");
                }
                _ => {
                    let r = clatm_demux_read_payload_length_info(&mut bs.rs, &mut s.d);
                    let e = unsafe { c::cref_latm_read_payload_length_info(s.c, bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr()) };
                    assert_eq!(r, e, "{what}: ReadPayloadLengthInfo");
                }
            }
            assert_eq!(found, cfound, "{what}: configFound");
            bs.check(&what);
            s.check(&what);
        }
    }
}

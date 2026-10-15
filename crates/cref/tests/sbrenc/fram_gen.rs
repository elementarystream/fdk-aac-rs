// Differential tests of fdk_aac_rs_sbrenc::fram_gen against the C++ reference.
//
// The tables of fram_gen.cpp, and the frame generator of the library and of the port on long
// random sequences of transient information (positions inside the frame, as the transient
// detectors produce them, with sparse and tight transients and right border requests), for
// every frame length, low delay and HE-AAC tuning, spreading on and off, static framing and
// the frequency resolutions; SBR_ENVELOPE_FRAME is compared after every frame.

use fdk_aac_rs_cref::sbrenc_fram_gen as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrenc::env_est::{V_TUNING_ELD, V_TUNING_HEAAC};
use fdk_aac_rs_sbrenc::fram_gen::*;

use super::sbr_encoder_dump::{assert_dump_eq, env_frame, frame_info, Dump};

#[test]
fn tables() {
    let infos = [
        &FRAME_INFO_1_2048,
        &FRAME_INFO_2_2048,
        &FRAME_INFO_4_2048,
        &FRAME_INFO_1_2304,
        &FRAME_INFO_2_2304,
        &FRAME_INFO_4_2304,
        &FRAME_INFO_1_1920,
        &FRAME_INFO_2_1920,
        &FRAME_INFO_4_1920,
        &FRAME_INFO_1_1152,
        &FRAME_INFO_2_1152,
        &FRAME_INFO_4_1152,
        &FRAME_INFO_1_512LD,
        &FRAME_INFO_2_512LD,
        &FRAME_INFO_4_512LD,
    ];
    let mut out = vec![0i32; 256];
    for (id, f) in infos.iter().enumerate() {
        let n = unsafe { c::cref_fram_gen_table(id as i32, out.as_mut_ptr()) };
        let mut d = Dump::default();
        frame_info(&mut d, f);
        assert_eq!(&out[..n as usize], &d.0[..], "frame info {id}");
    }
    let flat = |t: &[&[i32]]| t.iter().flat_map(|r| r.iter().copied()).collect::<Vec<i32>>();
    let e8: Vec<&[i32]> = ENVELOPE_TABLE_8.iter().map(|r| &r[..]).collect();
    let e16: Vec<&[i32]> = ENVELOPE_TABLE_16.iter().map(|r| &r[..]).collect();
    let e15: Vec<&[i32]> = ENVELOPE_TABLE_15.iter().map(|r| &r[..]).collect();
    for (id, v) in [
        (15, flat(&e8)),
        (16, flat(&e16)),
        (17, flat(&e15)),
        (18, FREQ_RES_TABLE_8.to_vec()),
        (19, FREQ_RES_TABLE_16.to_vec()),
        (20, vec![MIN_FRAME_TRAN_DISTANCE]),
    ] {
        let n = unsafe { c::cref_fram_gen_table(id, out.as_mut_ptr()) };
        assert_eq!(&out[..n as usize], &v[..], "table {id}");
    }
}

struct Pair {
    c: *mut core::ffi::c_void,
    r: SbrEnvelopeFrame,
}

impl Pair {
    fn new() -> Self {
        Pair { c: unsafe { c::cref_fram_gen_new() }, r: SbrEnvelopeFrame::default() }
    }
    fn check(&self, what: &str) {
        let mut buf = vec![0i32; 1024];
        let n = unsafe { c::cref_fram_gen_dump(self.c, buf.as_mut_ptr(), buf.len() as i32) };
        let mut d = Dump::default();
        env_frame(&mut d, &self.r);
        assert_dump_eq(&buf[..n as usize], &d.0, what);
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        unsafe { c::cref_fram_gen_free(self.c) };
    }
}

/// One configuration: time slots, low delay grid, spreading, static framing.
fn run(rng: &mut Rng, time_slots: i32, ld_grid: i32, allow_spread: i32, stat: i32, frames: usize) {
    let mut p = Pair::new();
    let num_env_static = [1, 2, 4][rng.range(0, 2) as usize];
    let fr = [rng.range(0, 1), rng.range(0, 1)];
    let low = rng.range(0, 1);
    unsafe {
        c::cref_fram_gen_init(p.c, allow_spread, num_env_static, stat, time_slots, fr[0], fr[1], low, ld_grid)
    };
    fdk_sbr_enc_init_frame_info_generator(
        &mut p.r,
        allow_spread,
        num_env_static,
        stat,
        time_slots,
        &fr,
        low as u8,
        ld_grid,
    );
    p.check("init");
    let tuning: &'static [i32] = if ld_grid != 0 { &V_TUNING_ELD } else { &V_TUNING_HEAAC };
    let mut pre = [0u8; 2];
    // Transient probability: a mixture of sparse and tight transients.
    let density = rng.range(1, 6);
    for f in 0..frames {
        let tran_flag = (rng.range(0, 9) < density) as u8;
        let tran_pos = if tran_flag != 0 {
            rng.range(0, time_slots - 1) as u8
        } else {
            // the frame splitter's flag
            rng.range(0, 1) as u8
        };
        let mut tran = [tran_pos, tran_flag, rng.range(0, 1) as u8];
        let rbf = (rng.range(0, 30) == 0) as i32;
        let mut pre_c = pre;
        let rc = unsafe {
            c::cref_fram_gen_generate(p.c, tran.as_mut_ptr(), rbf, pre_c.as_mut_ptr(), ld_grid, tuning.as_ptr())
        };
        assert_eq!(rc, 1);
        fdk_sbr_enc_frame_info_generator(&mut p.r, &tran, rbf, &pre, ld_grid, tuning);
        p.check(&format!("slots {time_slots} ld {ld_grid} spread {allow_spread} stat {stat} frame {f}"));
        pre = [tran[0], tran[1]];
    }
}

#[test]
fn generator_sequences() {
    let mut rng = Rng::new(0x5eed_0201);
    for _ in 0..40 {
        for &(slots, ld) in &[(16, 0), (15, 0), (16, 1), (15, 1)] {
            for spread in 0..=1 {
                run(&mut rng, slots, ld, spread, 0, 300);
            }
            run(&mut rng, slots, ld, 1, 1, 20);
        }
    }
}

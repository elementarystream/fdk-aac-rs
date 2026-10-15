// Differential tests of fdk_aac_rs_fdk::mdct against the C++ reference.
//
// The transforms are run in sequences of frames with the window sequences, window shapes and
// transform lengths of the decoder (CBlock_FrequencyToTime) and the encoder
// (FDKaacEnc_Transform_Real), including invalid transitions (which make imlt_block adapt the
// window slopes), the alias symmetry flags of USAC, gains, FAC ZIR and asymmetric overlap. After
// every call the outputs, the in-place modified spectrum and the complete state (fields, overlap
// buffer, window slope) are compared.

use core::ffi::c_void;

use crate::fft::signal;
use fdk_aac_rs_cref::fdk_mdct::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::archdef::FixpWtp;
use fdk_aac_rs_fdk::mdct::*;
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;

/// The window slopes the tests use, as (length, shape); `wrs_id` identifies `prev_wrs` by its
/// first index in this list.
const WINDOWS: &[(i32, i32)] = &[
    (1024, 0),
    (1024, 1),
    (128, 0),
    (128, 1),
    (960, 0),
    (960, 1),
    (120, 0),
    (120, 1),
    (768, 0),
    (768, 1),
    (96, 0),
    (96, 1),
    (512, 0),
    (480, 0),
    (256, 0),
    (256, 1),
    (64, 0),
    (192, 0),
    (240, 0),
    (32, 0),
    (48, 0),
];

fn slope(len: i32, shape: i32) -> Option<&'static [FixpWtp]> {
    if len > 0 {
        fdk_get_window_slope(len, shape)
    } else {
        None
    }
}

/// The C `mdct_t` (with its buffers) and the port's `MdctT`.
struct Pair {
    c: *mut c_void,
    r: MdctT,
}

impl Drop for Pair {
    fn drop(&mut self) {
        unsafe { cref_mdct_free(self.c) };
    }
}

impl Pair {
    /// A zeroed state initialized with `mdct_init(ov_size)`, the overlap buffer filled with
    /// `fill`.
    fn new(ov_size: i32, fill: i32) -> Self {
        let c = unsafe { cref_mdct_new(ov_size, fill) };
        let mut r = MdctT { overlap: vec![fill; ov_size as usize], ..Default::default() };
        mdct_init(&mut r, ov_size);
        Pair { c, r }
    }

    fn wrs_id_r(&self) -> i32 {
        let Some(w) = self.r.prev_wrs else { return -1 };
        WINDOWS
            .iter()
            .position(|&(l, s)| slope(l, s).is_some_and(|x| x.as_ptr() == w.as_ptr()))
            .map_or(-2, |i| i as i32)
    }

    fn wrs_id_c(&self) -> i32 {
        let lens: Vec<i32> = WINDOWS.iter().map(|w| w.0).collect();
        let shapes: Vec<i32> = WINDOWS.iter().map(|w| w.1).collect();
        unsafe { cref_mdct_wrs_id(self.c, lens.as_ptr(), shapes.as_ptr(), WINDOWS.len() as i32) }
    }

    /// Compares the complete state.
    fn check(&self, what: &str) {
        let mut f = [0i32; 9];
        let mut ovl = vec![0i32; self.r.ov_size.max(0) as usize];
        unsafe { cref_mdct_get_state(self.c, f.as_mut_ptr(), ovl.as_mut_ptr()) };
        let r = &self.r;
        let fr = [
            r.prev_tl,
            r.prev_nr,
            r.prev_fr,
            r.ov_offset,
            r.ov_size,
            r.prev_alias_symmetry,
            r.prev_prev_alias_symmetry,
            r.p_fac_zir.is_some() as i32,
            r.p_asym_ovlp.is_some() as i32,
        ];
        assert_eq!(fr, f, "{what}: state (prev_tl, prev_nr, prev_fr, ov_offset, ov_size, ...)");
        assert_eq!(r.overlap, ovl, "{what}: overlap buffer");
        assert_eq!(self.wrs_id_r(), self.wrs_id_c(), "{what}: prev_wrs");
    }

    /// Sets the state fields (`f` as in `check`, without `ov_size`) and `prev_wrs`.
    fn set_state(&mut self, f: [i32; 7], wrs: (i32, i32)) {
        unsafe { cref_mdct_set_state(self.c, f.as_ptr(), wrs.0, wrs.1) };
        let r = &mut self.r;
        r.prev_tl = f[0];
        r.prev_nr = f[1];
        r.prev_fr = f[2];
        r.ov_offset = f[3];
        r.prev_alias_symmetry = f[5];
        r.prev_prev_alias_symmetry = f[6];
        r.prev_wrs = slope(wrs.0, wrs.1);
    }

    fn set_overlap(&mut self, ovl: &[i32]) {
        unsafe { cref_mdct_set_overlap(self.c, ovl.as_ptr(), ovl.len() as i32) };
        self.r.overlap.copy_from_slice(ovl);
    }

    fn set_fac_zir(&mut self, v: Option<&[i32]>) {
        match v {
            Some(v) => unsafe { cref_mdct_set_fac_zir(self.c, v.as_ptr(), v.len() as i32) },
            None => unsafe { cref_mdct_set_fac_zir(self.c, core::ptr::null(), -1) },
        }
        self.r.p_fac_zir = v.map(|v| v.to_vec());
    }

    fn set_asym_ovlp(&mut self, v: Option<&[i32]>) {
        match v {
            Some(v) => unsafe { cref_mdct_set_asym_ovlp(self.c, v.as_ptr(), v.len() as i32) },
            None => unsafe { cref_mdct_set_asym_ovlp(self.c, core::ptr::null(), -1) },
        }
        self.r.p_asym_ovlp = v.map(|v| v.to_vec());
    }

    /// `imlt_block` on both; compares the return value, the output, the spectrum and the state.
    #[allow(clippy::too_many_arguments)]
    fn imlt_block(
        &mut self,
        r: &mut Rng,
        spectrum: &[i32],
        scalefactor: &[i16],
        n_spec: i32,
        no_out_samples: i32,
        tl: i32,
        wls: (i32, i32),
        fl: i32,
        wrs: (i32, i32),
        fr: i32,
        gain: i32,
        flags: i32,
        what: &str,
    ) -> Vec<i32> {
        let garbage: Vec<i32> = (0..4096).map(|_| r.i32()).collect();
        let (mut out_r, mut out_c) = (garbage.clone(), garbage);
        let (mut spec_r, mut spec_c) = (spectrum.to_vec(), spectrum.to_vec());
        let n_r = imlt_block(
            &mut self.r,
            &mut out_r,
            &mut spec_r,
            scalefactor,
            n_spec,
            no_out_samples,
            tl,
            slope(wls.0, wls.1),
            fl,
            slope(wrs.0, wrs.1),
            fr,
            gain,
            flags,
        );
        let n_c = unsafe {
            cref_mdct_imlt_block(
                self.c,
                out_c.as_mut_ptr(),
                spec_c.as_mut_ptr(),
                scalefactor.as_ptr(),
                n_spec,
                no_out_samples,
                tl,
                wls.0,
                wls.1,
                fl,
                wrs.0,
                wrs.1,
                fr,
                gain,
                flags,
            )
        };
        assert_eq!(n_r, n_c, "{what}: imlt_block return value");
        assert_eq!(spec_r, spec_c, "{what}: spectrum");
        assert_eq!(out_r, out_c, "{what}: output");
        self.check(what);
        out_r
    }

    fn drain(&mut self, r: &mut Rng, room: i32, what: &str) {
        let garbage: Vec<i32> = (0..4096).map(|_| r.i32()).collect();
        let (mut out_r, mut out_c) = (garbage.clone(), garbage);
        let n_r = imdct_drain(&mut self.r, &mut out_r, room);
        let n_c = unsafe { cref_mdct_imdct_drain(self.c, out_c.as_mut_ptr(), room) };
        assert_eq!(n_r, n_c, "{what}: imdct_drain return value");
        assert_eq!(out_r, out_c, "{what}: imdct_drain output");
        self.check(what);
    }

    fn copy_ov_and_nr(&mut self, r: &mut Rng, n: i32, what: &str) {
        let garbage: Vec<i32> = (0..4096).map(|_| r.i32()).collect();
        let (mut out_r, mut out_c) = (garbage.clone(), garbage);
        let n_r = imdct_copy_ov_and_nr(&self.r, &mut out_r, n);
        let n_c = unsafe { cref_mdct_imdct_copy_ov_and_nr(self.c, out_c.as_mut_ptr(), n) };
        assert_eq!(n_r, n_c, "{what}: imdct_copy_ov_and_nr return value");
        assert_eq!(out_r, out_c, "{what}: imdct_copy_ov_and_nr output");
        self.check(what);
    }
}

/// Random spectrum of `n` values: the signal generator of the FFT tests, mostly with headroom.
fn spectrum(r: &mut Rng, n: usize) -> Vec<i32> {
    let k = if r.next_u32() % 8 == 0 { r.range(0, 8) as usize } else { 9 };
    signal(r, k, n)
}

fn scalefactors(r: &mut Rng, n: usize) -> Vec<i16> {
    (0..n)
        .map(|_| match r.next_u32() % 8 {
            0 => r.range(-40, 40) as i16,
            1 => r.i16(),
            _ => r.range(-4, 12) as i16,
        })
        .collect()
}

fn gain(r: &mut Rng) -> i32 {
    match r.next_u32() % 4 {
        0 => r.i32(),
        1 => r.range(0x2000_0000, 0x7fff_ffff),
        _ => 0,
    }
}

/// A decoder configuration: frame length and the window shapes of its frames.
struct DecConfig {
    frame_len: i32,
    shapes: &'static [i32],
    /// Only long blocks (AAC-LD).
    long_only: bool,
    /// USAC: random alias symmetry flags.
    usac: bool,
}

const DEC_CONFIGS: &[DecConfig] = &[
    DecConfig { frame_len: 1024, shapes: &[0, 1], long_only: false, usac: false },
    DecConfig { frame_len: 960, shapes: &[0, 1], long_only: false, usac: false },
    DecConfig { frame_len: 1024, shapes: &[0, 1], long_only: false, usac: true },
    DecConfig { frame_len: 768, shapes: &[0, 1], long_only: false, usac: true },
    DecConfig { frame_len: 512, shapes: &[0, 2], long_only: true, usac: false },
    DecConfig { frame_len: 480, shapes: &[0, 2], long_only: true, usac: false },
];

/// `getWindow2Nr(length, shape)` of block.cpp.
fn window2_nr(length: i32, shape: i32) -> i32 {
    if shape == 2 {
        (length * 3) >> 2
    } else {
        0
    }
}

/// Frame sequences of the decoder (`CBlock_FrequencyToTime`).
#[test]
fn imlt_block_decoder_sequences() {
    for (ci, cfg) in DEC_CONFIGS.iter().enumerate() {
        let mut r = Rng::new(0x1a17_0000 + ci as u64);
        for run in 0..12 {
            let fill = if run % 2 == 0 { 0 } else { r.i32() };
            let mut p = Pair::new(768, fill);
            p.check("mdct_init");
            let mut seq = BLOCK_LONG;
            for frame in 0..60 {
                let what = format!("config {ci} run {run} frame {frame}");
                // Mostly valid transitions, sometimes any window sequence.
                seq = if cfg.long_only {
                    BLOCK_LONG
                } else if r.next_u32() % 4 == 0 {
                    r.range(0, 3)
                } else {
                    match seq {
                        BLOCK_LONG | BLOCK_STOP => {
                            if r.next_u32() % 3 == 0 {
                                BLOCK_START
                            } else {
                                BLOCK_LONG
                            }
                        }
                        _ => {
                            if r.next_u32() % 2 == 0 {
                                BLOCK_SHORT
                            } else {
                                BLOCK_STOP
                            }
                        }
                    }
                };
                let shape = cfg.shapes[r.range(0, cfg.shapes.len() as i32 - 1) as usize];
                let frame_len = cfg.frame_len;
                let mut tl = frame_len;
                let mut n_spec = 1;
                let (fl, fr) = match seq {
                    BLOCK_STOP => (frame_len >> 3, frame_len),
                    BLOCK_START => (frame_len, frame_len >> 3),
                    BLOCK_SHORT => {
                        tl >>= 3;
                        n_spec = 8;
                        (frame_len >> 3, frame_len >> 3)
                    }
                    _ => {
                        let fr = frame_len - window2_nr(frame_len, shape);
                        let fl = if p.r.prev_tl == 0 { fr } else { frame_len };
                        (fl, fr)
                    }
                };
                let flags = if cfg.usac { (r.next_u32() % 2) as i32 } else { 0 };
                // Fewer output samples than the frame now and then (the rest is diverted to the
                // overlap buffer and drained afterwards).
                let no_out = if seq == BLOCK_LONG && frame_len == 1024 && r.next_u32() % 8 == 0 {
                    frame_len / 2
                } else {
                    frame_len
                };
                let spec = spectrum(&mut r, (n_spec * tl) as usize);
                let sf = scalefactors(&mut r, n_spec as usize);
                let g = gain(&mut r);
                p.imlt_block(
                    &mut r,
                    &spec,
                    &sf,
                    n_spec,
                    no_out,
                    tl,
                    (fl, shape),
                    fl,
                    (fr, shape),
                    fr,
                    g,
                    flags,
                    &what,
                );
                if r.next_u32() % 4 == 0 {
                    let n = r.range(0, frame_len);
                    p.copy_ov_and_nr(&mut r, n, &what);
                }
                if no_out < frame_len || r.next_u32() % 8 == 0 {
                    p.drain(&mut r, frame_len, &what);
                }
            }
        }
    }
}

/// The FAC ZIR (`pFacZir`) added to the first output samples (`prev_nr == fl / 2`) or after the
/// window crossing, and the asymmetric overlap (`pAsymOvlp`).
#[test]
fn imlt_block_fac_zir_and_asym_ovlp() {
    let mut r = Rng::new(0x1a17_1000);
    for case in 0..3000 {
        let what = format!("case {case}");
        let (frame_len, shape) = [(1024, 0), (1024, 1), (768, 0), (768, 1)][case % 4];
        let short = r.next_u32() % 2 == 0;
        let (tl, n_spec, fl, fr) = if short {
            (frame_len >> 3, 8, frame_len >> 3, frame_len >> 3)
        } else {
            let fl = [frame_len, frame_len >> 3][r.range(0, 1) as usize];
            (frame_len, 1, fl, [frame_len, frame_len >> 3][r.range(0, 1) as usize])
        };
        let mut p = Pair::new(768, r.i32());
        // A consistent previous block whose right slope matches fl (no adaptation). The
        // overlap read by the first window (prev_nr + fl / 2 values from the end) fits into the
        // overlap buffer.
        let prev_nr = if r.next_u32() % 3 == 0 && fl <= 768 {
            fl / 2 // the first FAC ZIR branch
        } else {
            (frame_len - fl) >> 1
        };
        let ovl: Vec<i32> = signal(&mut r, 9, 768);
        p.set_overlap(&ovl);
        let alias = (r.next_u32() % 2) as i32;
        let alias2 = (r.next_u32() % 2) as i32;
        p.set_state([frame_len, prev_nr, fl, 0, 768, alias, alias2], (fl, shape));
        p.check(&what);
        // The FAC ZIR after the window crossing is limited to fl / 2 <= 128 (FDK_ASSERT).
        if r.next_u32() % 4 != 0 && (prev_nr == fl / 2 || fl <= 256) {
            let zir: Vec<i32> = (0..512).map(|_| r.i32() >> r.range(0, 8)).collect();
            p.set_fac_zir(Some(&zir));
        }
        if r.next_u32() % 3 == 0 {
            let asym: Vec<i32> = signal(&mut r, 9, (fl / 2) as usize);
            p.set_asym_ovlp(Some(&asym));
        }
        p.check(&what);
        let spec = spectrum(&mut r, (n_spec * tl) as usize);
        let sf = scalefactors(&mut r, n_spec as usize);
        let g = gain(&mut r);
        let flags = (r.next_u32() % 2) as i32;
        p.imlt_block(
            &mut r,
            &spec,
            &sf,
            n_spec,
            frame_len,
            tl,
            (fl, shape),
            fl,
            (fr, shape),
            fr,
            g,
            flags,
            &what,
        );
    }
}

/// `imdct_adapt_parameters` on random states and parameters.
#[test]
fn imdct_adapt_parameters_random() {
    let mut r = Rng::new(0x1a17_2000);
    for case in 0..50_000 {
        let what = format!("case {case}");
        let mut p = Pair::new(768, 0);
        let w = WINDOWS[r.range(0, WINDOWS.len() as i32 - 1) as usize];
        let prev_tl = if r.next_u32() % 4 == 0 { 0 } else { r.range(0, 1024) };
        p.set_state(
            [prev_tl, r.range(-512, 512), r.range(0, 1024), r.range(0, 512), 768, 0, 0],
            w,
        );
        let wl = WINDOWS[r.range(0, WINDOWS.len() as i32 - 1) as usize];
        let (mut fl_r, mut nl_r) = (r.range(0, 1024), r.range(-512, 1024));
        let (mut fl_c, mut nl_c) = (fl_r, nl_r);
        let tl = r.range(0, 1024);
        let no_out = r.range(0, 1024);
        imdct_adapt_parameters(&mut p.r, &mut fl_r, &mut nl_r, tl, slope(wl.0, wl.1), no_out);
        unsafe {
            cref_mdct_imdct_adapt_parameters(p.c, &mut fl_c, &mut nl_c, tl, wl.0, wl.1, no_out)
        };
        assert_eq!((fl_r, nl_r), (fl_c, nl_c), "{what}: fl, nl");
        p.check(&what);
    }
}

#[test]
fn imdct_gain_all_lengths() {
    let mut r = Rng::new(0x1a17_3000);
    for tl in 0..2100 {
        for k in 0..20 {
            let m0 = match k {
                0 => 0,
                1 => i32::MAX,
                2 => i32::MIN,
                _ => r.i32(),
            };
            let e0 = r.range(-40, 40);
            let (mut ma, mut ea) = (m0, e0);
            let (mut mb, mut eb) = (m0, e0);
            unsafe { cref_mdct_imdct_gain(&mut mb, &mut eb, tl) };
            // FDK_ASSERT(0) for unsupported lengths is a debug_assert! in the port; tl < 4
            // shifts by a negative amount in C.
            let log2 = 31 - (tl as u32).leading_zeros() as i32;
            let supported = tl == 0 || (tl >= 4 && (4..=7).contains(&(tl >> (log2 - 2))));
            if cfg!(debug_assertions) && !supported {
                continue;
            }
            imdct_gain(&mut ma, &mut ea, tl);
            assert_eq!((ma, ea), (mb, eb), "imdct_gain({m0:#x}, {e0}, {tl})");
        }
    }
}

/// Forward MDCT sequences of the encoder (`FDKaacEnc_Transform_Real`).
#[test]
fn mdct_block_encoder_sequences() {
    // (frameLength, shapes)
    let configs: &[(i32, &[i32])] =
        &[(1024, &[0, 1]), (960, &[0, 1]), (768, &[0, 1]), (512, &[0, 2]), (480, &[0, 2])];
    for (ci, &(frame_len, shapes)) in configs.iter().enumerate() {
        let mut r = Rng::new(0x1a17_4000 + ci as u64);
        for run in 0..8 {
            let mut p = Pair::new(0, 0);
            let mut block_type = 0;
            for frame in 0..40 {
                let what = format!("config {ci} run {run} frame {frame}");
                let ld = frame_len <= 512;
                // Only the transitions of the encoder's block switching.
                block_type = if ld {
                    0
                } else {
                    match block_type {
                        0 | 3 => [0, 0, 1][r.range(0, 2) as usize],
                        _ => [2, 3][r.range(0, 1) as usize],
                    }
                };
                let shape = shapes[r.range(0, shapes.len() as i32 - 1) as usize];
                let (n_spec, tl) = if block_type == 2 { (8, frame_len >> 3) } else { (1, frame_len) };
                let offset = if shape == 2 { (frame_len * 3) >> 2 } else { 0 };
                let fr = match block_type {
                    0 | 3 => frame_len - offset,
                    _ => frame_len >> 3,
                };
                let time: Vec<i16> = match r.next_u32() % 8 {
                    0 => vec![i16::MIN; 2 * frame_len as usize],
                    1 => vec![i16::MAX; 2 * frame_len as usize],
                    2 => (0..2 * frame_len).map(|i| if i % 2 == 0 { i16::MIN } else { i16::MAX }).collect(),
                    _ => (0..2 * frame_len).map(|_| r.i16()).collect(),
                };
                let garbage: Vec<i32> = (0..2 * frame_len).map(|_| r.i32()).collect();
                let (mut md_r, mut md_c) = (garbage.clone(), garbage);
                let (mut e_r, mut e_c) = ([0x55i16; 8], [0x55i16; 8]);
                let n_r = mdct_block(
                    &mut p.r,
                    &time,
                    frame_len,
                    &mut md_r,
                    n_spec,
                    tl,
                    slope(fr, shape),
                    fr,
                    &mut e_r,
                );
                let n_c = unsafe {
                    cref_mdct_mdct_block(
                        p.c,
                        time.as_ptr(),
                        frame_len,
                        md_c.as_mut_ptr(),
                        n_spec,
                        tl,
                        fr,
                        shape,
                        fr,
                        e_c.as_mut_ptr(),
                    )
                };
                assert_eq!(n_r, n_c, "{what}: mdct_block return value");
                assert_eq!(md_r, md_c, "{what}: mdctData");
                assert_eq!(e_r, e_c, "{what}: exponents");
                p.check(&what);
            }
        }
    }
}

/// `mdct_init` on a used state keeps the overlap contents and resets the window state.
#[test]
fn mdct_init_reinit() {
    let mut r = Rng::new(0x1a17_5000);
    let mut p = Pair::new(768, 7);
    for frame in 0..5 {
        let spec = spectrum(&mut r, 1024);
        let sf = scalefactors(&mut r, 1);
        p.imlt_block(
            &mut r,
            &spec,
            &sf,
            1,
            1024,
            1024,
            (1024, 0),
            1024,
            (1024, 0),
            1024,
            0,
            0,
            &format!("frame {frame}"),
        );
    }
    mdct_init(&mut p.r, 768);
    unsafe { cref_mdct_mdct_init(p.c, 768) };
    p.check("mdct_init");
}

/// Transitions without a defined reference result (the reference's buffers get 2048 values of
/// padding in front): the port skips these iterations and counts them; `mdctData`, the
/// exponents and the counts are compared.
#[test]
fn mdct_block_undefined_transitions() {
    const PAD: usize = 2048;
    let prev: &[(i32, i32)] = &[
        (1024, 0),
        (1024, 1),
        (960, 0),
        (960, 1),
        (768, 0),
        (512, 0),
        (480, 0),
        (256, 0),
        (240, 0),
        (128, 0),
        (128, 1),
        (120, 0),
        (64, 0),
    ];
    let mut r = Rng::new(0x1a17_6000);
    let (mut cases, mut with_writes, mut with_reads) = (0, 0, 0);
    let configs: &[(i32, &[i32])] =
        &[(1024, &[0, 1]), (960, &[0, 1]), (768, &[0, 1]), (512, &[0, 1, 2]), (480, &[0, 1, 2]), (256, &[0])];
    for &(frame_len, shapes) in configs {
        for block_type in 0..4 {
            for &shape in shapes {
                let (n_spec, tl) = if block_type == 2 { (8, frame_len >> 3) } else { (1, frame_len) };
                let offset = if shape == 2 { (frame_len * 3) >> 2 } else { 0 };
                let fr = if block_type == 0 || block_type == 3 { frame_len - offset } else { frame_len >> 3 };
                if slope(fr, shape).is_none() {
                    continue;
                }
                for &(pl, ps) in prev {
                    if pl <= tl || slope(pl, ps).is_none() {
                        continue;
                    }
                    let what = format!("frame {frame_len} block {block_type} shape {shape}, previous slope {pl}/{ps}");
                    cases += 1;
                    let mut p = Pair::new(0, 0);
                    p.set_state([pl, 0, pl, 0, 0, 0, 0], (pl, ps));
                    let time: Vec<i16> = (0..PAD + 4 * frame_len as usize).map(|_| r.i16()).collect();
                    let garbage: Vec<i32> = (0..PAD + 2 * frame_len as usize).map(|_| r.i32()).collect();
                    let mut md_c = garbage.clone();
                    let mut md_r = garbage[PAD..].to_vec();
                    let (mut e_r, mut e_c) = ([0x55i16; 8], [0x55i16; 8]);
                    let before = mdct_block_dropped_accesses();
                    let n_r = mdct_block(
                        &mut p.r,
                        &time[PAD..],
                        frame_len,
                        &mut md_r,
                        n_spec,
                        tl,
                        slope(fr, shape),
                        fr,
                        &mut e_r,
                    );
                    let after = mdct_block_dropped_accesses();
                    let n_c = unsafe {
                        cref_mdct_mdct_block(
                            p.c,
                            time[PAD..].as_ptr(),
                            frame_len,
                            md_c[PAD..].as_mut_ptr(),
                            n_spec,
                            tl,
                            fr,
                            shape,
                            fr,
                            e_c.as_mut_ptr(),
                        )
                    };
                    assert_eq!(n_r, n_c, "{what}: mdct_block return value");
                    assert_eq!(md_r, md_c[PAD..], "{what}: mdctData");
                    assert_eq!(e_r, e_c, "{what}: exponents");
                    p.check(&what);
                    let written_before = (0..PAD).filter(|&k| md_c[k] != garbage[k]).count() as u64;
                    let first = (0..PAD).find(|&k| md_c[k] != garbage[k]).unwrap_or(PAD);
                    assert_eq!(PAD - first, written_before as usize, "{what}: contiguous padding block");
                    assert_eq!(after.0 - before.0, written_before, "{what}: dropped writes");
                    assert_eq!(after.2, before.2, "{what}: no missing slope");
                    with_writes += (written_before > 0) as usize;
                    with_reads += (after.1 > before.1) as usize;
                }
            }
        }
    }
    eprintln!("mdct_block_undefined_transitions: {cases} transitions, {with_writes} with dropped writes, {with_reads} with dropped reads");
    assert!(with_writes > 0 && with_reads > 0);
}

/// A missing window slope (no defined reference result) reads as zeros in the port and is
/// counted; the port does not panic.
#[test]
fn mdct_block_missing_slope() {
    let mut r = Rng::new(0x1a17_7000);
    let mut m = MdctT::default();
    mdct_init(&mut m, 0);
    let time: Vec<i16> = (0..960).map(|_| r.i16()).collect();
    let mut md = vec![0i32; 480];
    let mut e = [0i16; 8];
    let before = mdct_block_dropped_accesses();
    // fr = 60 has no slope
    assert!(fdk_get_window_slope(60, 1).is_none());
    let n = mdct_block(&mut m, &time, 480, &mut md, 1, 480, fdk_get_window_slope(60, 1), 60, &mut e);
    assert_eq!(n, 480);
    let after = mdct_block_dropped_accesses();
    assert_eq!(after.2 - before.2, 2, "the right slope, then the left slope (prev_wrs = NULL)");
}

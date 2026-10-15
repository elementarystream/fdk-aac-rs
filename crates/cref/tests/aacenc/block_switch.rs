// Differential tests of fdk_aac_rs_aacenc::block_switch against the C++ reference.
//
// The C functions work directly on the port's BlockSwitchingControl (it has the C layout, which
// `layout_matches` checks). The block switching is run over many consecutive frames of real PCM
// (the corpus in work/, skipped if absent) and of synthetic signals (transients, silence, full
// scale square waves, noise, ...), for every granule length, AAC-LC and low delay, LFE, mono and
// stereo with and without a common window; the complete structures are compared after every call.
//
// This file also provides the PCM sources of the other test modules of this group.

use fdk_aac_rs_aacenc::block_switch::*;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_cref::aacenc_block_switch as c;
use fdk_aac_rs_cref::testutil::Rng;
use std::ffi::c_void;
use std::path::PathBuf;

// ---------------------------------------------------------------------------------------------
// PCM sources (shared with the other test modules of this group)
// ---------------------------------------------------------------------------------------------

/// `work/` of the main checkout (the directory of the reference library's cache).
pub fn work_dir() -> Option<PathBuf> {
    let lib = PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    lib.ancestors().find(|p| p.file_name().is_some_and(|n| n == "work")).map(|p| p.to_path_buf())
}

/// `n` samples per channel of the s16le file `work/corpus/<name>` (`channels` interleaved),
/// starting at sample frame `start`; `None` if the file is not available.
pub fn corpus(name: &str, channels: usize, start: usize, n: usize) -> Option<Vec<Vec<i16>>> {
    let path = work_dir()?.join("corpus").join(name);
    let bytes = std::fs::read(path).ok()?;
    let frames = bytes.len() / (2 * channels);
    if frames < n {
        return None;
    }
    let start = start.min(frames - n);
    let mut out = vec![Vec::with_capacity(n); channels];
    for f in start..start + n {
        for (ch, o) in out.iter_mut().enumerate() {
            let k = 2 * (f * channels + ch);
            o.push(i16::from_le_bytes([bytes[k], bytes[k + 1]]));
        }
    }
    Some(out)
}

/// The real PCM excerpts of the corpus (stereo; mono files are duplicated), as
/// `(name, [left, right])`.
pub fn corpus_signals(n: usize) -> Vec<(String, Vec<Vec<i16>>)> {
    let mut v = Vec::new();
    for (name, ch, start) in [
        ("music_c_48k.raw", 2, 48000 * 40),
        ("music_f_44k.raw", 2, 44100 * 20),
        ("music_g_48k.raw", 2, 48000 * 30),
        ("broadcast_a_48k.raw", 2, 48000 * 60),
        ("music_b_mono_44k.raw", 1, 44100 * 5),
        ("syn_clicks.raw", 2, 0),
        ("syn_square_fs.raw", 2, 0),
        ("syn_sweep.raw", 2, 0),
        ("syn_antiphase.raw", 2, 0),
    ] {
        if let Some(mut s) = corpus(name, ch, start, n) {
            if s.len() == 1 {
                let l = s[0].clone();
                s.push(l);
            }
            v.push((name.to_string(), s));
        }
    }
    v
}

/// Synthetic signals of `n` samples per channel (stereo), rich in transients and extreme
/// values.
pub fn synthetic_signals(n: usize, seed: u64) -> Vec<(String, Vec<Vec<i16>>)> {
    let mut r = Rng::new(seed);
    let mut v: Vec<(String, Vec<Vec<i16>>)> = Vec::new();
    let mut gen = |name: &str, f: &mut dyn FnMut(&mut Rng, usize, usize) -> i16| {
        let s = (0..2).map(|ch| (0..n).map(|i| f(&mut r, ch, i)).collect()).collect();
        v.push((name.to_string(), s));
    };
    gen("silence", &mut |_, _, _| 0);
    gen("square_full_scale", &mut |_, ch, i| {
        if (i / (37 + 11 * ch)) % 2 == 0 {
            i16::MAX
        } else {
            i16::MIN
        }
    });
    gen("alternating_extremes", &mut |_, _, i| if i % 2 == 0 { i16::MAX } else { i16::MIN });
    gen("noise_full_scale", &mut |r, _, _| r.next_u32() as i16);
    gen("noise_quiet", &mut |r, _, _| (r.next_u32() as i16) >> 11);
    // Low-level noise with bursts and clicks of random level at random positions; the bursts
    // are often only in one channel.
    let mut state = [(0usize, 0i32, 0usize); 2];
    gen("bursts", &mut |r, ch, i| {
        let (ref mut left, ref mut shift, ref mut gap) = state[ch];
        if *left == 0 {
            if *gap == 0 {
                *left = 1 + (r.next_u32() % 3000) as usize;
                *shift = (r.next_u32() % 16) as i32;
                *gap = (r.next_u32() % 6000) as usize;
            } else {
                *gap -= 1;
                return (r.next_u32() as i16) >> 12;
            }
        }
        *left -= 1;
        let _ = i;
        (r.next_u32() as i16) >> *shift
    });
    gen("clicks", &mut |r, _, _| match r.next_u32() % 1500 {
        0 => i16::MAX,
        1 => i16::MIN,
        2 => (r.next_u32() as i16) >> (r.next_u32() % 8),
        _ => 0,
    });
    // Steps between silence and full scale DC, and level changes every few hundred samples.
    let mut lvl = [0i16; 2];
    gen("steps", &mut |r, ch, i| {
        if i % (200 + 50 * ch) == 0 {
            lvl[ch] = match r.next_u32() % 4 {
                0 => 0,
                1 => i16::MAX,
                2 => i16::MIN,
                _ => r.next_u32() as i16,
            };
        }
        lvl[ch]
    });
    gen("random_edge", &mut |r, _, _| r.i16());
    v
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

fn bytes<T>(t: &T) -> &[u8] {
    // SAFETY: plain integer data.
    unsafe { std::slice::from_raw_parts(t as *const T as *const u8, std::mem::size_of_val(t)) }
}

#[test]
fn tables_match() {
    let rust: [&[u8]; 10] = [
        bytes(&BLOCK_TYPE2_WINDOW_SHAPE),
        bytes(&HI_PASS_COEFF),
        bytes(&ACC_WINDOW_NRG_FAC),
        bytes(&ONE_MINUS_ACC_WINDOW_NRG_FAC),
        bytes(&INV_ATTACK_RATIO),
        bytes(&MIN_ATTACK_NRG),
        bytes(&SUGGESTED_GROUPING_TABLE),
        bytes(&CHG_WND_SQ),
        bytes(&CHG_WND_SQ_LK_AHD),
        bytes(&SYNCHRONIZED_BLOCK_TYPE_TABLE),
    ];
    for (i, r) in rust.iter().enumerate() {
        let mut size = 0;
        let p = unsafe { c::cref_block_switch_table(i as i32, &mut size) };
        assert!(!p.is_null());
        let cmem = unsafe { std::slice::from_raw_parts(p as *const u8, size as usize) };
        assert_eq!(*r, cmem, "table {i}");
    }
    let mut size = 0;
    assert!(unsafe { c::cref_block_switch_table(10, &mut size) }.is_null());
}

#[test]
fn layout_matches() {
    use std::mem::offset_of;
    type B = BlockSwitchingControl;
    let rust = [
        offset_of!(B, last_window_sequence),
        offset_of!(B, window_shape),
        offset_of!(B, last_window_shape),
        offset_of!(B, n_block_switch_windows),
        offset_of!(B, attack),
        offset_of!(B, lastattack),
        offset_of!(B, attack_index),
        offset_of!(B, last_attack_index),
        offset_of!(B, allow_short_frames),
        offset_of!(B, allow_look_ahead),
        offset_of!(B, no_of_groups),
        offset_of!(B, group_len),
        offset_of!(B, max_window_nrg),
        offset_of!(B, window_nrg),
        offset_of!(B, window_nrg_f),
        offset_of!(B, acc_window_nrg),
        offset_of!(B, iir_states),
    ];
    let mut o = [0i32; 17];
    let size = unsafe { c::cref_block_switch_layout(o.as_mut_ptr()) };
    assert_eq!(size as usize, std::mem::size_of::<B>());
    assert_eq!(o.map(|x| x as usize), rust);
}

fn ptr(b: &mut BlockSwitchingControl) -> *mut c_void {
    b as *mut _ as *mut c_void
}

/// A random but consistent state (as reached by the state machine), for `low_delay` 0/1.
pub(crate) fn random_state(r: &mut Rng, low_delay: i32) -> BlockSwitchingControl {
    let mut b = BlockSwitchingControl::default();
    fdk_aac_enc_init_block_switching(&mut b, low_delay);
    let n = b.n_block_switch_windows as i32;
    b.last_window_sequence = if low_delay != 0 {
        [LONG_WINDOW, START_WINDOW, STOP_WINDOW, _LOWOV_WINDOW][(r.next_u32() % 4) as usize]
    } else {
        r.range(LONG_WINDOW, STOP_WINDOW)
    };
    b.window_shape = r.range(0, 2);
    b.last_window_shape = r.range(0, 2);
    b.attack = r.range(0, 1);
    b.lastattack = r.range(0, 1);
    b.attack_index = r.range(0, n - 1);
    b.last_attack_index = r.range(0, n - 1);
    b.no_of_groups = r.range(1, 4);
    for g in b.group_len.iter_mut() {
        *g = r.range(0, 8);
    }
    let nrg = |r: &mut Rng| match r.next_u32() % 4 {
        0 => r.i32().max(0),
        1 => (r.next_u32() >> (r.next_u32() % 32)) as i32 & i32::MAX,
        2 => [0, 1, MIN_ATTACK_NRG - 1, MIN_ATTACK_NRG, i32::MAX][(r.next_u32() % 5) as usize],
        _ => (r.next_u32() >> 12) as i32,
    };
    b.max_window_nrg = nrg(r);
    for w in 0..2 {
        for i in 0..BLOCK_SWITCH_WINDOWS {
            b.window_nrg[w][i] = nrg(r);
            b.window_nrg_f[w][i] = nrg(r);
        }
    }
    b.acc_window_nrg = nrg(r);
    // iirStates[0] is a sample << 15, iirStates[1] the filter output; also extreme values.
    b.iir_states[0] = if r.next_u32() % 4 == 0 { r.i32() } else { (r.i16() as i32) << 15 };
    b.iir_states[1] = r.i32();
    b
}

#[test]
fn init_block_switching() {
    let mut r = Rng::new(1);
    for low_delay in [0, 1, 2, -1] {
        let mut a = random_state(&mut r, 0);
        let mut b = a;
        fdk_aac_enc_init_block_switching(&mut a, low_delay);
        unsafe { c::cref_block_switch_InitBlockSwitching(ptr(&mut b), low_delay) };
        assert_eq!(a, b, "low_delay {low_delay}");
    }
}

#[test]
fn get_window_energy() {
    let mut r = Rng::new(2);
    let v: Vec<i32> = (0..16).map(|_| r.i32()).collect();
    for i in 0..16 {
        assert_eq!(fdk_aac_enc_get_window_energy(&v, i), unsafe {
            c::cref_block_switch_GetWindowEnergy(v.as_ptr(), i)
        });
    }
}

#[test]
fn calc_window_energy_random() {
    let mut r = Rng::new(3);
    for it in 0..4000 {
        let low_delay = r.range(0, 1);
        let mut a = random_state(&mut r, low_delay);
        let mut b = a;
        let window_len = match r.next_u32() % 4 {
            0 => [128, 120, 64, 60, 32, 30, 16, 15][(r.next_u32() % 8) as usize],
            1 => r.range(-2, 3),
            _ => r.range(0, 128),
        };
        let pcm: Vec<i16> = match r.next_u32() % 3 {
            0 => (0..1024).map(|_| r.i16()).collect(),
            1 => (0..1024).map(|_| if r.next_u32() % 2 == 0 { i16::MAX } else { i16::MIN }).collect(),
            _ => (0..1024).map(|_| (r.next_u32() as i16) >> (r.next_u32() % 16)).collect(),
        };
        fdk_aac_enc_calc_window_energy(&mut a, window_len, &pcm);
        unsafe { c::cref_block_switch_CalcWindowEnergy(ptr(&mut b), window_len, pcm.as_ptr()) };
        assert_eq!(a, b, "iteration {it}");
    }
}

#[test]
fn block_switching_random_states() {
    let mut r = Rng::new(4);
    let granules = [1024, 960, 512, 480, 256, 240, 128, 120];
    for it in 0..20000 {
        let low_delay = r.range(0, 1);
        let mut a = random_state(&mut r, low_delay);
        let mut b = a;
        let granule = granules[(r.next_u32() % 8) as usize];
        let is_lfe = (r.next_u32() % 8 == 0) as i32;
        let shift = r.next_u32() % 16;
        let pcm: Vec<i16> = (0..1024).map(|_| r.i16() >> shift).collect();
        let ra = fdk_aac_enc_block_switching(&mut a, granule, is_lfe, &pcm);
        let rb = unsafe {
            c::cref_block_switch_BlockSwitching(ptr(&mut b), granule, is_lfe, pcm.as_ptr())
        };
        assert_eq!((ra, a), (rb, b), "iteration {it}");
    }
}

#[test]
fn sync_block_switching_random_states() {
    let mut r = Rng::new(5);
    let mut wrong = 0;
    for it in 0..40000 {
        let low_delay = r.range(0, 1);
        // Mixed LC/LD pairs give WRONG_WINDOW (-1).
        let low_delay_r = if r.next_u32() % 8 == 0 { 1 - low_delay } else { low_delay };
        let mut a = [random_state(&mut r, low_delay), random_state(&mut r, low_delay_r)];
        if r.next_u32() % 2 == 0 {
            // SHORT_WINDOW in LC; LOWOV_WINDOW is produced in LD.
            for x in a.iter_mut() {
                x.last_window_sequence = if x.allow_short_frames != 0 {
                    [LONG_WINDOW, START_WINDOW, SHORT_WINDOW, STOP_WINDOW][(r.next_u32() % 4) as usize]
                } else {
                    x.last_window_sequence
                };
            }
        }
        let mut b = a;
        let n_channels = r.range(1, 2);
        let common_window = if r.next_u32() % 4 == 0 { r.range(0, 1) } else { 1 };
        let (a0, a1) = a.split_at_mut(1);
        let ra = fdk_aac_enc_sync_block_switching(
            &mut a0[0],
            (n_channels == 2).then_some(&mut a1[0]),
            n_channels,
            common_window,
        );
        let (b0, b1) = b.split_at_mut(1);
        let rb = unsafe {
            c::cref_block_switch_SyncBlockSwitching(
                ptr(&mut b0[0]),
                if n_channels == 2 { ptr(&mut b1[0]) } else { std::ptr::null_mut() },
                n_channels,
                common_window,
            )
        };
        wrong += (ra != 0) as i32;
        assert_eq!((ra, a), (rb, b), "iteration {it}");
    }
    assert!(wrong > 0);
}

/// Runs the block switching of psyMain (`FDKaacEnc_BlockSwitching` per channel, then
/// `FDKaacEnc_SyncBlockSwitching`) over the whole signal with the port and with C and compares
/// the complete states after every call. Returns the sequence of (window sequence, window
/// shape) of channel 0 after the synchronization.
pub fn run_block_switching(
    signal: &[Vec<i16>],
    granule: usize,
    low_delay: i32,
    is_lfe: i32,
    n_channels: usize,
    common_window: i32,
    what: &str,
) -> Vec<(i32, i32)> {
    // C works on its own copy; start both from garbage to check that the init clears.
    let mut a = [BlockSwitchingControl { attack: 7, iir_states: [3, -5], ..Default::default() }; 2];
    let mut b = a;
    for ch in 0..n_channels {
        fdk_aac_enc_init_block_switching(&mut a[ch], low_delay);
        unsafe { c::cref_block_switch_InitBlockSwitching(ptr(&mut b[ch]), low_delay) };
    }
    assert_eq!(a, b, "{what}: init");
    let frames = signal[0].len() / granule;
    let mut seq = Vec::with_capacity(frames);
    for f in 0..frames {
        for ch in 0..n_channels {
            let pcm = &signal[ch][f * granule..(f + 1) * granule];
            let ra = fdk_aac_enc_block_switching(&mut a[ch], granule as i32, is_lfe, pcm);
            let rb = unsafe {
                c::cref_block_switch_BlockSwitching(
                    ptr(&mut b[ch]),
                    granule as i32,
                    is_lfe,
                    pcm.as_ptr(),
                )
            };
            assert_eq!((ra, a[ch]), (rb, b[ch]), "{what}: frame {f} channel {ch} BlockSwitching");
        }
        let (a0, a1) = a.split_at_mut(1);
        let ra = fdk_aac_enc_sync_block_switching(
            &mut a0[0],
            (n_channels > 1).then_some(&mut a1[0]),
            n_channels as i32,
            common_window,
        );
        let (b0, b1) = b.split_at_mut(1);
        let rb = unsafe {
            c::cref_block_switch_SyncBlockSwitching(
                ptr(&mut b0[0]),
                if n_channels > 1 { ptr(&mut b1[0]) } else { std::ptr::null_mut() },
                n_channels as i32,
                common_window,
            )
        };
        assert_eq!((ra, a), (rb, b), "{what}: frame {f} SyncBlockSwitching");
        assert_eq!(ra, 0, "{what}: frame {f}");
        seq.push((a[0].last_window_sequence, a[0].window_shape));
    }
    seq
}

#[test]
fn block_switching_sequences() {
    let n = 1024 * 160;
    let mut signals = corpus_signals(n);
    let have_corpus = !signals.is_empty();
    signals.extend(synthetic_signals(n, 6));
    let mut transitions = std::collections::BTreeSet::new();
    for (name, s) in &signals {
        for (granule, low_delay) in
            [(1024, 0), (960, 0), (512, 1), (480, 1), (1024, 1), (512, 0), (256, 1), (120, 0)]
        {
            for (is_lfe, n_channels, common_window) in
                [(0, 1, 1), (0, 2, 1), (0, 2, 0), (1, 1, 1)]
            {
                let what = format!(
                    "{name} granule {granule} low_delay {low_delay} lfe {is_lfe} \
                     channels {n_channels} common_window {common_window}"
                );
                let seq = run_block_switching(
                    s,
                    granule,
                    low_delay,
                    is_lfe,
                    n_channels,
                    common_window,
                    &what,
                );
                for w in seq.windows(2) {
                    transitions.insert((low_delay, w[0], w[1]));
                }
            }
        }
    }
    // Every transition of the AAC-LC state machine and the low-overlap switching of LD.
    let lc = |a: i32, b: i32| transitions.iter().any(|&(ld, x, y)| ld == 0 && x.0 == a && y.0 == b);
    for (a, b) in [
        (LONG_WINDOW, LONG_WINDOW),
        (LONG_WINDOW, START_WINDOW),
        (START_WINDOW, SHORT_WINDOW),
        (SHORT_WINDOW, SHORT_WINDOW),
        (SHORT_WINDOW, STOP_WINDOW),
        (STOP_WINDOW, LONG_WINDOW),
        (STOP_WINDOW, START_WINDOW),
    ] {
        assert!(lc(a, b), "LC transition {a} -> {b} not covered");
    }
    let ld = |a: (i32, i32), b: (i32, i32)| transitions.contains(&(1, a, b));
    let long = (LONG_WINDOW, SINE_WINDOW);
    let lol = (LONG_WINDOW, LOL_WINDOW);
    for (a, b) in [(long, long), (long, lol), (lol, lol), (lol, long)] {
        assert!(ld(a, b), "LD transition {a:?} -> {b:?} not covered");
    }
    eprintln!("corpus available: {have_corpus}; transitions: {transitions:?}");
}

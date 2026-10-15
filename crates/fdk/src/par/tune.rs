// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not a port: the reference library is single-threaded.

//! The choice of a codec instance's thread mode by measurement (docs/optimization.md, "Thread mode selection").
//!
//! A codec instance with more than one thread can run a frame in several *modes*, for example
//! the decoder sequentially, sequentially with the channels on several threads, or with stage B
//! of the frame pipeline on a second thread. Every mode gives exactly the output of the
//! sequential code; they differ only in speed and CPU time, and which one is fastest depends on
//! the stream and the machine (a mode that keeps a second thread busy can be slower than the
//! sequential code where the work it moves is small, see the measurements in the document). The
//! modes of an instance are ordered by their CPU cost (mode 0: the cheapest, one thread; the
//! last: every parallel feature, which is the mode of the first frames).
//!
//! [`Tuner`] measures the time between the starts of consecutive frames in each mode
//! (interleaved blocks of frames, so that the content of the stream affects every mode alike)
//! and runs the cheapest mode whose median is within [`MARGIN_PERMILLE`] of the fastest one,
//! measuring again after an exploitation period that doubles while the choice stays the same.
//! The choice depends on timing, so the code path of a frame does; the output does not: every
//! mode is exact, and a switch between two modes is a switch the parallel paths make for their
//! sequential fallbacks as well (a sequential call after a pipelined frame, for example). The
//! deterministic schedule [`Tuning::Cycle`] takes every mode and every switch independent of
//! timing; the tests and the decoder's poisoning builds use it.

use std::sync::OnceLock;
use std::time::Instant;

/// The environment variable with the default [`Tuning`] of new codec instances
/// (`FDK_AAC_RS_TUNE`): `adaptive` (the default), `full` (always the last mode: every parallel
/// feature), `cycle`, or a mode number.
pub const TUNE_ENV: &str = "FDK_AAC_RS_TUNE";

/// The largest number of modes of an instance.
pub const MAX_MODES: usize = 5;

/// How a codec instance chooses its thread mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tuning {
    /// By measurement (the default, see the module documentation).
    Adaptive,
    /// Always this mode (the last one if the number is too large).
    Fixed(u8),
    /// A fixed pseudo-random schedule that switches the mode every few frames (tests: every
    /// mode and every switch between two modes is taken, independent of timing).
    Cycle,
}

impl Tuning {
    /// Always the last mode (every parallel feature).
    pub const FULL: Tuning = Tuning::Fixed(u8::MAX);

    /// Parses a value of [`TUNE_ENV`] (`None` for an unknown value).
    pub fn parse(v: &str) -> Option<Tuning> {
        match v.trim() {
            "adaptive" | "" => Some(Tuning::Adaptive),
            "full" => Some(Tuning::FULL),
            "cycle" => Some(Tuning::Cycle),
            n => n.parse::<u8>().ok().map(Tuning::Fixed),
        }
    }

    /// The default of new instances: [`TUNE_ENV`] (read once), else `fallback`.
    pub fn from_env(fallback: Tuning) -> Tuning {
        static ENV: OnceLock<Option<Tuning>> = OnceLock::new();
        ENV.get_or_init(|| std::env::var(TUNE_ENV).ok().and_then(|v| Tuning::parse(&v)))
            .unwrap_or(fallback)
    }
}

/// The decisions are written to stderr if the environment variable `FDK_AAC_RS_TUNE_LOG` is
/// set (read once).
fn log_enabled() -> bool {
    static LOG: OnceLock<bool> = OnceLock::new();
    *LOG.get_or_init(|| std::env::var_os("FDK_AAC_RS_TUNE_LOG").is_some())
}

/// Frames of the last mode before the first measurement.
const WARMUP: u32 = 16;
/// Frames per block of a measurement.
const BLOCK: u32 = 8;
/// The periods of the first frames in a mode that are not used (the switch and its after
/// effects: a refilled pipeline, caches).
const SKIP: u32 = 2;
/// Blocks per mode of a measurement (interleaved, in an order rotated every round of modes).
const BLOCKS_PER_MODE: u32 = 4;
/// Periods per mode of a measurement.
const SAMPLES: usize = ((BLOCK - SKIP) * BLOCKS_PER_MODE) as usize;
/// The first exploitation period in frames (doubled while the choice stays, up to
/// [`EXPLOIT_MAX`]).
const EXPLOIT_MIN: u32 = 512;
/// The longest exploitation period in frames.
const EXPLOIT_MAX: u32 = 8192;
/// A mode whose median period exceeds the fastest one's by more than this (per mille) is not
/// measured in the next rounds ...
const SLOW_PERMILLE: u64 = 80;
/// ... except in every this many-th round.
const FULL_ROUND: u32 = 4;
/// A more expensive mode is chosen only if its median period is shorter than that of every
/// cheaper one by more than this (per mille).
pub const MARGIN_PERMILLE: u64 = 30;
/// Frames per mode of [`Tuning::Cycle`].
const CYCLE_RUN: u32 = 3;

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// The last mode for so many more frames.
    Warmup(u32),
    /// Block `block` (mode `block % modes`) with `left` more frames.
    Explore { block: u32, left: u32 },
    /// The chosen mode for so many more frames.
    Exploit(u32),
}

/// The mode choice of a codec instance (see the module documentation).
#[derive(Clone, Debug)]
pub struct Tuner {
    tuning: Tuning,
    modes: usize,
    /// The mode of the next frame.
    mode: usize,
    phase: Phase,
    /// The start of the last frame, its mode, and whether its period is measured.
    last: Option<(Instant, usize, bool)>,
    /// Consecutive frames in the mode of the last frame.
    streak: u32,
    samples: [[u32; SAMPLES]; MAX_MODES],
    counts: [usize; MAX_MODES],
    /// The mode chosen by the last measurement.
    chosen: Option<usize>,
    exploit: u32,
    /// The modes of the current measurement.
    active: [u8; MAX_MODES],
    n_active: u8,
    /// Bit m: mode m was slow in its last measurement.
    slow: u8,
    /// Measurements so far.
    round: u32,
    /// Frames since the last change of the mode count ([`Tuning::Cycle`]).
    frames: u64,
    rng: u64,
}

impl Tuner {
    /// A tuner with one mode.
    pub fn new(tuning: Tuning) -> Tuner {
        Tuner {
            tuning,
            modes: 1,
            mode: 0,
            phase: Phase::Warmup(WARMUP),
            last: None,
            streak: 0,
            samples: [[0; SAMPLES]; MAX_MODES],
            counts: [0; MAX_MODES],
            chosen: None,
            exploit: EXPLOIT_MIN,
            active: [0; MAX_MODES],
            n_active: 0,
            slow: 0,
            round: 0,
            frames: 0,
            rng: 0x2545_f491_4f6c_dd1d,
        }
    }

    /// The tuning.
    pub fn tuning(&self) -> Tuning {
        self.tuning
    }

    /// Changes the tuning (starts again).
    pub fn set_tuning(&mut self, tuning: Tuning) {
        let modes = self.modes;
        *self = Tuner::new(tuning);
        self.modes = 0;
        self.set_modes(modes);
    }

    /// Sets the number of modes (1 to [`MAX_MODES`]); a change starts again with the warm-up.
    pub fn set_modes(&mut self, modes: usize) {
        let modes = modes.clamp(1, MAX_MODES);
        if modes == self.modes {
            return;
        }
        self.modes = modes;
        self.phase = Phase::Warmup(WARMUP);
        self.last = None;
        self.counts = [0; MAX_MODES];
        self.chosen = None;
        self.exploit = EXPLOIT_MIN;
        self.slow = 0;
        self.round = 0;
        self.frames = 0;
        self.mode = self.first_mode();
    }

    /// The number of modes.
    pub fn modes(&self) -> usize {
        self.modes
    }

    /// The mode of the next frame.
    pub fn mode(&self) -> usize {
        self.mode
    }

    /// The mode chosen by the last measurement (`None` before the first one).
    pub fn chosen(&self) -> Option<usize> {
        self.chosen
    }

    fn first_mode(&self) -> usize {
        match self.tuning {
            Tuning::Fixed(k) => (k as usize).min(self.modes - 1),
            _ => self.modes - 1,
        }
    }

    /// A frame started at `start` in the mode [`Tuner::mode`] returned before it: measures
    /// the period of the previous frame and sets the mode of the next frame.
    pub fn tick(&mut self, start: Instant) {
        if let Some((t0, m0, measured)) = self.last {
            if measured {
                let d = start.saturating_duration_since(t0).as_nanos().min(u32::MAX as u128);
                let c = &mut self.counts[m0];
                if *c < SAMPLES {
                    self.samples[m0][*c] = d as u32;
                    *c += 1;
                }
            }
        }
        let mode = self.mode;
        self.streak = if self.last.is_some_and(|l| l.1 == mode) { self.streak + 1 } else { 1 };
        let exploring = matches!(self.phase, Phase::Explore { .. });
        self.last = Some((start, mode, exploring && self.streak > SKIP));
        self.frames += 1;
        self.mode = self.next_mode();
    }

    fn next_mode(&mut self) -> usize {
        let n = self.modes;
        if n == 1 {
            return 0;
        }
        match self.tuning {
            Tuning::Fixed(_) => self.first_mode(),
            Tuning::Cycle => {
                if self.frames % CYCLE_RUN as u64 != 0 {
                    return self.mode;
                }
                self.rng =
                    self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((self.rng >> 33) % n as u64) as usize
            }
            Tuning::Adaptive => {
                self.phase = match self.phase {
                    Phase::Warmup(l) if l > 1 => Phase::Warmup(l - 1),
                    Phase::Exploit(l) if l > 1 => Phase::Exploit(l - 1),
                    Phase::Warmup(_) | Phase::Exploit(_) => self.begin_round(),
                    Phase::Explore { block, left } if left > 1 => {
                        Phase::Explore { block, left: left - 1 }
                    }
                    Phase::Explore { block, .. }
                        if block + 1 < self.n_active as u32 * BLOCKS_PER_MODE =>
                    {
                        Phase::Explore { block: block + 1, left: BLOCK }
                    }
                    Phase::Explore { .. } => {
                        let m = self.decide();
                        self.choose(m)
                    }
                };
                match self.phase {
                    Phase::Warmup(_) => n - 1,
                    // (rotated every round of modes, so that each mode follows a different one)
                    Phase::Explore { block, .. } => {
                        let (b, k) = (block as usize, self.n_active as usize);
                        self.active[(b + b / k) % k] as usize
                    }
                    Phase::Exploit(_) => self.chosen.unwrap_or(n - 1),
                }
            }
        }
    }

    /// The start of a measurement: the modes to measure are all, except those that were slow in
    /// the last measurement (unless every [`FULL_ROUND`]th round); with only the chosen mode
    /// left, its exploitation continues.
    fn begin_round(&mut self) -> Phase {
        self.round += 1;
        let all = self.chosen.is_none() || self.round % FULL_ROUND == 0;
        self.n_active = 0;
        for m in 0..self.modes {
            if all || self.slow & (1 << m) == 0 || self.chosen == Some(m) {
                self.active[self.n_active as usize] = m as u8;
                self.n_active += 1;
            }
        }
        self.counts = [0; MAX_MODES];
        if self.n_active < 2 {
            let m = self.chosen.unwrap_or(self.modes - 1);
            return self.choose(m);
        }
        Phase::Explore { block: 0, left: BLOCK }
    }

    /// Exploits mode `m` (for longer if it was chosen before).
    fn choose(&mut self, m: usize) -> Phase {
        self.exploit = match self.chosen {
            Some(c) if c == m => (self.exploit * 2).min(EXPLOIT_MAX),
            _ => EXPLOIT_MIN,
        };
        self.chosen = Some(m);
        Phase::Exploit(self.exploit)
    }

    /// The cheapest mode whose median period is within the margin of the fastest mode's (a mode
    /// with too few periods is not chosen; the last mode if none has enough).
    fn decide(&mut self) -> usize {
        let n = self.modes;
        let mut med = [u64::MAX; MAX_MODES];
        for (m, med) in med.iter_mut().enumerate().take(n) {
            let c = self.counts[m];
            if c >= SAMPLES / 2 {
                let s = &mut self.samples[m][..c];
                s.sort_unstable();
                *med = s[c / 2] as u64;
            }
        }
        let best = med[..n].iter().copied().min().unwrap_or(u64::MAX);
        if best == u64::MAX {
            return n - 1;
        }
        let limit = best.saturating_mul(1000 + MARGIN_PERMILLE) / 1000;
        let m = (0..n).find(|&m| med[m] <= limit).unwrap_or(n - 1);
        // (a mode that was not measured keeps its mark)
        let slow_limit = best.saturating_mul(1000 + SLOW_PERMILLE) / 1000;
        for (k, &v) in med.iter().enumerate().take(n) {
            if self.counts[k] >= SAMPLES / 2 {
                self.slow = (self.slow & !(1 << k)) | (((v > slow_limit) as u8) << k);
            }
        }
        if log_enabled() {
            eprintln!("tune: medians {:?} ns -> mode {m} (slow {:#b})", &med[..n], self.slow);
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Runs `frames` frames whose period in mode m is `period(m)` µs; returns the modes.
    fn run(t: &mut Tuner, frames: usize, mut period: impl FnMut(usize) -> u64) -> Vec<usize> {
        let mut now = Instant::now();
        let mut modes = Vec::new();
        for _ in 0..frames {
            let m = t.mode();
            modes.push(m);
            t.tick(now);
            now += Duration::from_micros(period(m));
        }
        modes
    }

    #[test]
    fn chooses_the_cheapest_mode_within_the_margin() {
        // mode 2 is 2% faster than mode 1, which is 20% faster than mode 0: mode 1
        let mut t = Tuner::new(Tuning::Adaptive);
        t.set_modes(3);
        let modes = run(&mut t, 2000, |m| [120, 100, 98][m]);
        assert!(modes[..WARMUP as usize].iter().all(|&m| m == 2));
        assert_eq!(t.chosen(), Some(1));
        assert!(modes[1000..].iter().filter(|&&m| m == 1).count() > 900);
        // a clear winner
        let mut t = Tuner::new(Tuning::Adaptive);
        t.set_modes(4);
        run(&mut t, 600, |m| [100, 100, 100, 50][m]);
        assert_eq!(t.chosen(), Some(3));
        // all equal: the cheapest
        let mut t = Tuner::new(Tuning::Adaptive);
        t.set_modes(2);
        run(&mut t, 600, |_| 100);
        assert_eq!(t.chosen(), Some(0));
    }

    #[test]
    fn slow_modes_are_measured_less_often() {
        let mut t = Tuner::new(Tuning::Adaptive);
        t.set_modes(3);
        let modes = run(&mut t, 40_000, |m| [95, 200, 90][m]);
        assert_eq!(t.chosen(), Some(2));
        let slow = modes[1000..].iter().filter(|&&m| m == 1).count();
        let fair = modes[1000..].iter().filter(|&&m| m == 0).count();
        // (mode 1 only in every FULL_ROUND-th round)
        assert!(slow > 0 && slow * 2 < fair, "{slow} {fair}");
        // only the chosen mode left: no measurement at all, but every FULL_ROUND-th round
        let mut t = Tuner::new(Tuning::Adaptive);
        t.set_modes(2);
        let modes = run(&mut t, 40_000, |m| [200, 100][m]);
        assert_eq!(t.chosen(), Some(1));
        assert!(modes[1000..].iter().filter(|&&m| m == 0).count() < 200);
    }

    #[test]
    fn measures_again() {
        let mut t = Tuner::new(Tuning::Adaptive);
        t.set_modes(2);
        let mut now = Instant::now();
        let mut fast = 1;
        for i in 0..6000 {
            if i == 2000 {
                fast = 0;
            }
            let m = t.mode();
            t.tick(now);
            now += Duration::from_micros(if m == fast { 50 } else { 100 });
        }
        assert_eq!(t.chosen(), Some(0));
    }

    #[test]
    fn outliers_do_not_decide() {
        let mut t = Tuner::new(Tuning::Adaptive);
        t.set_modes(2);
        let mut k = 0u64;
        run(&mut t, 600, |m| {
            k += 1;
            // mode 0 is slower, mode 1 has a few huge periods (the caller paused)
            if m == 1 && k % 10 == 0 {
                100_000
            } else {
                [100, 60][m]
            }
        });
        assert_eq!(t.chosen(), Some(1));
    }

    #[test]
    fn fixed_and_cycle() {
        let mut t = Tuner::new(Tuning::Fixed(1));
        t.set_modes(3);
        assert!(run(&mut t, 100, |_| 10).iter().all(|&m| m == 1));
        let mut t = Tuner::new(Tuning::FULL);
        t.set_modes(3);
        assert!(run(&mut t, 100, |_| 10).iter().all(|&m| m == 2));
        let mut t = Tuner::new(Tuning::Cycle);
        t.set_modes(4);
        let modes = run(&mut t, 400, |_| 10);
        let mut seen = [[false; 4]; 4];
        for w in modes.windows(2) {
            seen[w[0]][w[1]] = true;
        }
        assert!(seen.iter().flatten().all(|&s| s), "{seen:?}");
        // deterministic
        let mut t2 = Tuner::new(Tuning::Cycle);
        t2.set_modes(4);
        assert_eq!(run(&mut t2, 400, |_| 99), modes);
    }

    #[test]
    fn parse() {
        assert_eq!(Tuning::parse("adaptive"), Some(Tuning::Adaptive));
        assert_eq!(Tuning::parse("full"), Some(Tuning::FULL));
        assert_eq!(Tuning::parse(" cycle\n"), Some(Tuning::Cycle));
        assert_eq!(Tuning::parse("2"), Some(Tuning::Fixed(2)));
        assert_eq!(Tuning::parse("x"), None);
    }
}

// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: chunked parallel decoding of whole files.

//! Chunked parallel decoding of a whole input (not in C).
//!
//! The decoder of a single stream is sequential by nature (frame to frame state). A whole file
//! can be decoded faster by decoding ranges of it (chunks) at the same time, each with its own
//! decoder that starts some steps earlier (warm-up, output discarded). The output of such a
//! decoder is only identical to the sequential decoding once its state equals the state of the
//! sequential decoder, which is not guaranteed after any fixed warm-up (and some state never
//! converges: the PNS random seeds, the SBR noise and harmonic phases, ...). Every chunk boundary
//! is therefore **verified** by comparing the digests of the complete decoder states
//! ([`AacDecoderInstance::state_digest`](fdk_aac_rs_aacdec::aacdecoder::AacDecoderInstance::state_digest)),
//! and the sequential decoding continues wherever they differ:
//!
//! - The input is a sequence of *steps* (the fills of a byte stream, or the access units of a
//!   packet input; [`ChunkJob`]), split into chunks. The *true run* decodes from the first step
//!   on the calling thread, like the sequential decoder. Every further chunk is decoded by a
//!   *speculative run* on a worker thread (a single-threaded decoder), which starts some steps
//!   before its chunk ([`ChunkJob::warmup_start`]) and computes its state digest before the first
//!   steps of its chunk (the verification points, [`ChunkedOptions::verify_offsets`]) and before
//!   the end of its chunk.
//! - The calling thread passes the output of the current run to the sink in order. At the end of
//!   chunk k it compares the digest of the current run with the digest of the speculative run of
//!   chunk k + 1 before the same step. If they are equal, the output is taken from the speculative
//!   run from there on, which continues at the end of its chunk. If not, the comparison is
//!   repeated at the next verification point (the current run decodes on); if no verification
//!   point matches, the current run continues through chunk k + 1.
//! - A *hazard* ([`HazardProbe`](fdk_aac_rs_aacdec::digest::HazardProbe)): a step whose output may
//!   depend on state that is not part of the digest (the comfort noise of the concealment, the
//!   physical layout of the limiter buffers). The output of a speculative run is used only up to
//!   its first hazard; from there on, the true run (paused at the boundary where it handed over)
//!   decodes up to that step without output and continues.
//! - After [`ChunkedOptions::max_rejections`] consecutive boundaries whose states differ while the
//!   true run produces the output, the chunked decoding is given up: [`run_chunked`] returns the
//!   true run, and the caller decodes the rest sequentially (with the frame pipelining).
//! - Speculative runs whose boundaries are rejected cost CPU time for nothing, so (with the
//!   automatic chunk count) the true run first decodes one step alone and the chunked decoding
//!   is not started for a stream whose states rarely become equal ([`ChunkJob::converges_rarely`]:
//!   SBR, USAC); beyond a first run per worker thread, runs start only as far as the boundaries
//!   decided so far justify (`Gate`); and
//!   without probe chunks the first rejected boundary gives the chunked decoding up unless one
//!   was accepted before. None of this changes the output: it only decides which boundaries are
//!   tried.
//!
//! Counters that the decoder only accumulates and reports (the byte and access unit statistics of
//! the stream information) are not part of the digest; a run reports them relative to its start,
//! and the sink adds the difference to the true run at the boundary ([`ChunkJob::Counters`]).
//!
//! [`decode_chunked`] is the decoding of a whole input with the safe API.

mod decode;

pub use decode::*;

use fdk_aac_rs_fdk::digest::{diff_traces, Digest};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

/// The output of one step of a run ([`ChunkJob::step`]).
pub struct Step<O> {
    /// The output of the step.
    pub out: O,
    /// The step was a hazard: its output may depend on state that is not part of the digest.
    pub hazard: bool,
    /// The run has no further step (end of the input, or a stop condition).
    pub finished: bool,
}

/// A decoding that can be split into chunks: the sequential decoding is a sequence of steps of a
/// *run*, each with some output, and a run can start at any step (a fresh decoder that is fed
/// the input from there on).
pub trait ChunkJob: Sync {
    /// A decoder with the position in the input (and whatever the steps keep between them).
    type Run: Send;
    /// The output of a step.
    type Out: Send;
    /// The counters that a run accumulates and reports, but that are not part of its state
    /// digest (statistics). A run that starts later reports them relative to its start; the
    /// output of a run is passed to the sink with the counters of the true run at the boundary
    /// minus its own (the *base*).
    type Counters: Copy
        + Default
        + Send
        + core::ops::Add<Output = Self::Counters>
        + core::ops::Sub<Output = Self::Counters>;
    /// An error that ends a run.
    type Error: Send;

    /// The number of steps of the input (the number of the sequential decoding, unless it
    /// stops early or needs more steps; the runs decide when they are finished).
    fn steps(&self) -> usize;
    /// The step where the speculative run of a chunk starting at `step` starts (at least the
    /// warm-up before); `None` if there is none (too early in the input).
    fn warmup_start(&self, step: usize) -> Option<usize>;
    /// A run positioned at `step` (a new decoder; the true run is the only run started at step
    /// 0, and it is started once).
    fn start(&self, step: usize) -> Result<Self::Run, Self::Error>;
    /// The next step of the run.
    fn position(&self, run: &Self::Run) -> usize;
    /// Decodes the next step.
    fn step(&self, run: &mut Self::Run) -> Result<Step<Self::Out>, Self::Error>;
    /// The digest of the complete state of the run (decoder and step state).
    fn digest(&self, run: &Self::Run) -> Digest;
    /// The digests of the fields of the state (for the debug output of rejected boundaries).
    fn trace(&self, run: &Self::Run) -> Vec<(String, Digest)>;
    /// The counters of the run so far.
    fn counters(&self, run: &Self::Run) -> Self::Counters;
    /// The memory held by an output (for [`ChunkedOptions::max_buffered_bytes`]).
    fn out_bytes(out: &Self::Out) -> usize;
    /// Test of the digest (`FDK_AAC_RS_CHUNK_POISON`): fills the state of the run that is not
    /// part of its digest with a pattern derived from `seed`. Called for the speculative runs
    /// after each of their digests; the output must stay the sequential one.
    fn poison(&self, _run: &mut Self::Run, _seed: u64) {}
    /// Whether the states of runs that start at different steps rarely become equal for the
    /// stream of `run` (state that never converges, such as the SBR noise and harmonic phases):
    /// with the automatic chunk count, the chunked decoding is then given up at the first
    /// rejected boundary (unless one was accepted before). Only a shortcut: the digests decide
    /// every boundary.
    fn converges_rarely(&self, _run: &Self::Run) -> bool {
        false
    }
}

/// Options of [`run_chunked`].
#[derive(Clone, Debug)]
pub struct ChunkedOptions {
    /// Number of threads (at least 1): the calling thread decodes too (the true run and the
    /// continuations) and passes the output on; the others decode the speculative runs.
    pub threads: usize,
    /// Number of chunks (0: automatic, see [`ChunkedOptions::num_chunks`]). With one chunk the
    /// input is decoded sequentially.
    pub chunks: usize,
    /// Steps after the start of a chunk at which its boundary is verified (in this order; the
    /// first one where the states are equal is taken).
    pub verify_offsets: Vec<usize>,
    /// Minimum number of steps per chunk of the automatic chunk count.
    pub min_chunk_len: usize,
    /// Minimum number of threads for the automatic chunk count to split the input at all (with
    /// two threads the frame pipeline of the sequential decoding needs far less memory).
    pub min_auto_threads: usize,
    /// Limit of the memory of the speculative runs that wait for their turn (bytes of output,
    /// [`ChunkJob::out_bytes`], plus [`ChunkedOptions::run_cost`] per waiting run). A speculative
    /// run waits while the limit is exceeded (unless the output is taken from it or its digest is
    /// awaited).
    pub max_buffered_bytes: usize,
    /// Memory of a decoder, counted while a speculative run waits for its turn (estimate).
    pub run_cost: usize,
    /// After this many consecutive boundaries whose states differ (while the true run produces the
    /// output), the chunked decoding is given up (0: never). With the automatic chunk count, the
    /// first this many boundaries are placed close to the start (*probe chunks*, as short as the
    /// warm-up and the verification points allow), so that a stream whose decoder states never
    /// become equal (SBR, PNS, ...) is given up early.
    pub max_rejections: usize,
    /// Log the chunk decisions to stderr (also with the environment variable
    /// `FDK_AAC_RS_CHUNK_LOG`); with `FDK_AAC_RS_CHUNK_DEBUG` the fields in which the states differ
    /// are listed at every verification point where they differ.
    pub log: bool,
    /// Negative control (tests only): accept every boundary without comparing the states (also
    /// with the environment variable `FDK_AAC_RS_CHUNK_NO_VERIFY`). The output is then not the
    /// sequential one in general.
    pub no_verify: bool,
}

impl Default for ChunkedOptions {
    fn default() -> Self {
        ChunkedOptions {
            threads: 1,
            chunks: 0,
            verify_offsets: vec![0, 2, 8],
            min_chunk_len: 128,
            min_auto_threads: 4,
            max_buffered_bytes: 256 << 20,
            run_cost: 4 << 20,
            max_rejections: 3,
            log: false,
            no_verify: false,
        }
    }
}

impl ChunkedOptions {
    /// The number of chunks for `n` steps whose output takes `bytes_per_step` bytes.
    ///
    /// Automatic: none (1) for fewer than [`ChunkedOptions::min_auto_threads`] threads or less
    /// than two [`ChunkedOptions::min_chunk_len`] steps; otherwise four per thread, but chunks of
    /// at least `min_chunk_len` steps, and at least so many that the output of one chunk per
    /// worker thread fits into [`ChunkedOptions::max_buffered_bytes`].
    pub fn num_chunks(&self, n: usize, bytes_per_step: usize) -> usize {
        if self.chunks > 0 {
            return self.chunks.min(n.max(1));
        }
        let threads = self.threads.max(1);
        let base = (n / self.min_chunk_len.max(1)).min(4 * threads);
        if threads < self.min_auto_threads.max(2) || base < 2 {
            return 1;
        }
        let mem_len = self.max_buffered_bytes / ((threads - 1).max(1) * bytes_per_step.max(1));
        base.max(n / mem_len.max(self.min_chunk_len).max(1))
    }
}

/// Statistics of [`run_chunked`].
#[derive(Clone, Debug, Default)]
pub struct ChunkedStats {
    /// Number of chunks.
    pub chunks: usize,
    /// Boundaries at which the output was handed over to a speculative run.
    pub handoffs: usize,
    /// Speculative runs that were not used (states not equal at any verification point, errors,
    /// not started in time).
    pub rejected: usize,
    /// Hazards after which the true run took over.
    pub fallbacks: usize,
    /// Steps decoded in total (all runs, including warm-up and discarded output).
    pub steps: usize,
    /// State digests computed.
    pub digests: usize,
    /// The step from which on the input is left to the sequential decoding (`None`: the chunked
    /// decoding decoded everything).
    pub sequential_from: Option<usize>,
}

/// How [`run_chunked`] ended (with the run that produced the last output).
pub struct ChunkedEnd<J: ChunkJob> {
    /// The run that produced the last output: finished, or (when the chunked decoding was given
    /// up) the true run at [`ChunkedStats::sequential_from`], to be continued sequentially.
    pub run: J::Run,
    /// The base of its counters (see [`ChunkJob::Counters`]).
    pub base: J::Counters,
    /// The run is finished.
    pub finished: bool,
    pub stats: ChunkedStats,
}

/// Errors of [`run_chunked`]: the errors of the sequential decoding, reported after the output
/// before them.
#[derive(Debug)]
pub enum ChunkedRunError<E, S> {
    /// A run that produces the output failed at the step.
    Run(usize, E),
    /// The sink returned an error.
    Sink(S),
}

impl<E: core::fmt::Display, S: core::fmt::Display> core::fmt::Display for ChunkedRunError<E, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ChunkedRunError::Run(step, e) => write!(f, "decoding failed at step {step}: {e}"),
            ChunkedRunError::Sink(e) => write!(f, "output failed: {e}"),
        }
    }
}

impl<E, S> std::error::Error for ChunkedRunError<E, S>
where
    E: std::error::Error + 'static,
    S: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ChunkedRunError::Run(_, e) => Some(e),
            ChunkedRunError::Sink(e) => Some(e),
        }
    }
}

/* ---------------------------------------------------------------------------------------- */
/* Runs                                                                                     */
/* ---------------------------------------------------------------------------------------- */

/// State digest of a run before a step (with the field digests in debug mode).
struct DigestInfo<C> {
    digest: Digest,
    counters: C,
    trace: Option<Arc<Vec<(String, Digest)>>>,
}

impl<C: Copy> Clone for DigestInfo<C> {
    fn clone(&self) -> Self {
        DigestInfo { digest: self.digest, counters: self.counters, trace: self.trace.clone() }
    }
}

/// Events of a run.
enum Event<J: ChunkJob> {
    /// The output of a step (and whether it was a hazard).
    Out(usize, J::Out, bool),
    Digest(usize, DigestInfo<J::Counters>),
    Error(usize, J::Error),
    /// The run reached its stop position (the run is handed back).
    Paused(Box<J::Run>),
    /// The run is finished (the run is handed back).
    Finished(Box<J::Run>),
}

/// How a run segment ended.
enum RunEnd<E> {
    Paused,
    Finished,
    /// The event handler asked to stop.
    Stopped,
    Error(usize, E),
}

/// Decodes from the position of `run` up to `stop` (exclusive), or until it is finished. Before
/// the steps in `digest_at` the state digest is passed to `on` (and with `poison` the state that
/// is not part of it is poisoned, [`ChunkJob::poison`]); the output of steps before `emit_from`
/// is dropped. `on` returns `false` to stop.
#[allow(clippy::too_many_arguments)]
fn drive<J: ChunkJob>(
    job: &J,
    run: &mut J::Run,
    stop: usize,
    digest_at: &[usize],
    emit_from: usize,
    debug: bool,
    poison: bool,
    cancel: Option<&AtomicBool>,
    counts: &mut (usize, usize),
    on: &mut dyn FnMut(Event<J>) -> bool,
) -> RunEnd<J::Error> {
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return RunEnd::Stopped;
        }
        let pos = job.position(run);
        if digest_at.contains(&pos) {
            counts.1 += 1;
            let info = DigestInfo {
                digest: job.digest(run),
                counters: job.counters(run),
                trace: debug.then(|| Arc::new(job.trace(run))),
            };
            if poison {
                job.poison(run, (pos as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5bd1_e995);
            }
            if !on(Event::Digest(pos, info)) {
                return RunEnd::Stopped;
            }
        }
        if pos >= stop {
            return RunEnd::Paused;
        }
        let s = match job.step(run) {
            Ok(s) => s,
            Err(e) => return RunEnd::Error(pos, e),
        };
        counts.0 += 1;
        if pos >= emit_from && !on(Event::Out(pos, s.out, s.hazard)) {
            return RunEnd::Stopped;
        }
        if s.finished {
            return RunEnd::Finished;
        }
    }
}

/* ---------------------------------------------------------------------------------------- */
/* Speculative runs                                                                         */
/* ---------------------------------------------------------------------------------------- */

const NOT_STARTED: u8 = 0;
const STARTED: u8 = 1;
const ABSORBED: u8 = 2;

/// Shared state of a speculative chunk.
struct ChunkShared {
    state: AtomicU8,
    cancel: AtomicBool,
    /// Exempt from the buffer limit: the output of the run is consumed at once, or the
    /// coordinator waits for its state digest.
    exempt: AtomicBool,
}

/// The memory held by the speculative runs for the coordinator (per chunk; a discarded chunk
/// releases all of it at once).
struct BudgetState {
    total: usize,
    per_chunk: Vec<usize>,
    closed: Vec<bool>,
}

/// Limit of the memory held by the speculative runs.
struct Budget {
    state: Mutex<BudgetState>,
    cv: Condvar,
    limit: usize,
}

impl Budget {
    fn new(chunks: usize, limit: usize) -> Budget {
        Budget {
            state: Mutex::new(BudgetState {
                total: 0,
                per_chunk: vec![0; chunks],
                closed: vec![false; chunks],
            }),
            cv: Condvar::new(),
            limit,
        }
    }

    /// Accounts `n` bytes that chunk `j` sends to the coordinator; waits before while the limit
    /// is exceeded and the chunk is not exempt. Returns `false` if the chunk was discarded.
    fn acquire(&self, j: usize, n: usize, c: &ChunkShared) -> bool {
        let mut st = self.state.lock().unwrap();
        while st.total > self.limit && !c.exempt.load(Ordering::Acquire) && !st.closed[j] {
            st = self.cv.wait_timeout(st, std::time::Duration::from_millis(50)).unwrap().0;
        }
        if st.closed[j] {
            return false;
        }
        st.total += n;
        st.per_chunk[j] += n;
        true
    }

    /// The coordinator took `n` bytes of chunk `j`.
    fn release(&self, j: usize, n: usize) {
        let mut st = self.state.lock().unwrap();
        let n = n.min(st.per_chunk[j]);
        st.per_chunk[j] -= n;
        st.total -= n;
        self.cv.notify_all();
    }

    /// Chunk `j` is discarded: releases everything it holds, later sends are refused.
    fn close(&self, j: usize) {
        let mut st = self.state.lock().unwrap();
        st.closed[j] = true;
        let n = std::mem::take(&mut st.per_chunk[j]);
        st.total -= n;
        self.cv.notify_all();
    }

    fn wake(&self) {
        let _g = self.state.lock().unwrap();
        self.cv.notify_all();
    }
}

/// The plan of the chunks.
struct Plan {
    /// Chunk starts; `starts[chunks]` is the number of steps.
    starts: Vec<usize>,
    /// Where the speculative run of chunk `j` starts; `None` for chunk 0 and for chunks
    /// without a start.
    run_starts: Vec<Option<usize>>,
    run_cost: usize,
    verify_offsets: Vec<usize>,
}

impl Plan {
    fn chunks(&self) -> usize {
        self.starts.len() - 1
    }

    /// The end of chunk `j` (where its run stops; the last chunk runs until it is finished).
    fn end(&self, j: usize) -> usize {
        if j + 1 >= self.chunks() {
            usize::MAX
        } else {
            self.starts[j + 1]
        }
    }

    /// Verification points of the boundary at the start of chunk `j`.
    fn candidates(&self, j: usize) -> Vec<usize> {
        let (s, e) = (self.starts[j], self.end(j));
        let mut v: Vec<usize> =
            self.verify_offsets.iter().map(|&o| s + o).filter(|&p| p < e).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// The speculative run of chunk `j` (on a worker thread). Returns the number of decoded steps and
/// digests.
#[allow(clippy::too_many_arguments)]
fn speculate<J: ChunkJob>(
    job: &J,
    plan: &Plan,
    j: usize,
    debug: bool,
    poison: bool,
    sh: &ChunkShared,
    budget: &Budget,
    tx: &mpsc::Sender<Event<J>>,
) -> (usize, usize) {
    let s = plan.starts[j];
    let end = plan.end(j);
    let Some(w) = plan.run_starts[j] else { return (0, 0) };
    let mut run = match job.start(w) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Event::Error(w, e));
            return (0, 0);
        }
    };
    let mut digest_at = plan.candidates(j);
    if end != usize::MAX {
        digest_at.push(end);
    }
    let mut on = |e: Event<J>| -> bool {
        if let Event::Out(_, o, _) = &e {
            if !budget.acquire(j, J::out_bytes(o), sh) {
                return false;
            }
        }
        tx.send(e).is_ok() && !sh.cancel.load(Ordering::Relaxed)
    };
    let mut counts = (0, 0);
    let r = drive(
        job,
        &mut run,
        end,
        &digest_at,
        s,
        debug,
        poison,
        Some(&sh.cancel),
        &mut counts,
        &mut on,
    );
    match r {
        RunEnd::Paused => {
            /* the decoder waits for its turn */
            if budget.acquire(j, plan.run_cost, sh) {
                let _ = tx.send(Event::Paused(Box::new(run)));
            }
        }
        RunEnd::Finished => {
            let _ = tx.send(Event::Finished(Box::new(run)));
        }
        RunEnd::Stopped => {}
        RunEnd::Error(at, e) => {
            let _ = tx.send(Event::Error(at, e));
        }
    }
    counts
}

/// A speculative run waiting for a worker thread.
type Task<J> = (usize, Arc<ChunkShared>, mpsc::Sender<Event<J>>);

struct GateState<J: ChunkJob> {
    /// The runs not started yet, in chunk order.
    tasks: VecDeque<Task<J>>,
    /// The highest chunk whose run may start.
    limit: usize,
}

/// The speculative runs waiting for a worker thread, and the highest chunk whose run may start.
///
/// A speculative run whose boundary is rejected is CPU time spent for nothing, and a stream whose
/// states differ at one boundary mostly differs at the next ones too (SBR, PNS, USAC noise
/// filling). So beyond a first run per worker thread (the probe chunks first) the runs start only
/// as far as the boundaries decided so far justify: after an accepted boundary all; after a
/// rejected boundary before any accepted one, or after two rejected ones in a row, the next
/// chunk. A run that is started stays (its boundary may still be accepted). Which runs exist
/// never changes the output, only which boundaries are taken.
struct Gate<J: ChunkJob> {
    state: Mutex<GateState<J>>,
    cv: Condvar,
}

impl<J: ChunkJob> Gate<J> {
    /// The next run to start (waits while the next one is beyond the limit); `None` when
    /// closed.
    fn next(&self) -> Option<Task<J>> {
        let mut st = self.state.lock().unwrap();
        loop {
            match st.tasks.front() {
                None => return None,
                Some(t) if t.0 <= st.limit => return st.tasks.pop_front(),
                Some(_) => st = self.cv.wait(st).unwrap(),
            }
        }
    }

    /// Runs up to chunk `limit` may start.
    fn set_limit(&self, limit: usize) {
        let mut st = self.state.lock().unwrap();
        if st.limit != limit {
            st.limit = limit;
            self.cv.notify_all();
        }
    }

    /// No further run starts.
    fn close(&self) {
        let mut st = self.state.lock().unwrap();
        st.tasks.clear();
        self.cv.notify_all();
    }
}

/// A speculative chunk as seen by the coordinator.
struct ChunkHandle<J: ChunkJob> {
    index: usize,
    shared: Arc<ChunkShared>,
    rx: Option<mpsc::Receiver<Event<J>>>,
    /// Events read ahead while waiting for a digest.
    stash: VecDeque<Event<J>>,
    digests: BTreeMap<usize, DigestInfo<J::Counters>>,
    /// The run ended (error, paused, finished) or its channel is closed.
    ended: bool,
}

impl<J: ChunkJob> ChunkHandle<J> {
    /// The digest of the run before step `at` (waits for it); `None` if the run ended or passed
    /// `at` without one.
    fn wait_digest(&mut self, at: usize) -> Option<DigestInfo<J::Counters>> {
        loop {
            if let Some(d) = self.digests.get(&at) {
                return Some(d.clone());
            }
            if self.ended || self.digests.range(at..).next().is_some() {
                return None;
            }
            let rx = self.rx.as_ref()?;
            match rx.recv() {
                Ok(Event::Digest(p, d)) => {
                    self.digests.insert(p, d);
                }
                Ok(e) => {
                    if matches!(e, Event::Error(..) | Event::Paused(_) | Event::Finished(_)) {
                        self.ended = true;
                    }
                    self.stash.push_back(e);
                }
                Err(_) => self.ended = true,
            }
        }
    }

    /// Whether the run has no digest at `at` or later for sure.
    fn ended_before(&self, at: usize) -> bool {
        self.ended && self.digests.range(at..).next().is_none()
    }

    fn next_event(&mut self) -> Option<Event<J>> {
        if let Some(e) = self.stash.pop_front() {
            return Some(e);
        }
        self.rx.as_ref()?.recv().ok()
    }

    /// Drops the chunk: cancels its run and releases the memory it holds (without waiting for
    /// the run to stop).
    fn discard(&mut self, budget: &Budget) {
        self.shared.cancel.store(true, Ordering::Release);
        budget.close(self.index);
        self.stash.clear();
        self.rx = None;
        self.ended = true;
    }
}

/* ---------------------------------------------------------------------------------------- */
/* Coordinator                                                                              */
/* ---------------------------------------------------------------------------------------- */

/// The receiver of the output of [`run_chunked`]: the output of a step and the base of its
/// counters.
pub type Sink<'a, J, S> =
    dyn FnMut(<J as ChunkJob>::Out, &<J as ChunkJob>::Counters) -> Result<(), S> + 'a;

/// Decodes the steps of `job` with several threads and passes the output of every step, with
/// the base of its counters, to `sink` in order (see the [module documentation](self)): the same
/// output as the sequential decoding with one run from step 0, up to the end of the input or to
/// the step where the chunked decoding was given up ([`ChunkedEnd`]).
///
/// `bytes_per_step` estimates the output of a step, for the automatic chunk count.
pub fn run_chunked<J: ChunkJob, S>(
    job: &J,
    opts: &ChunkedOptions,
    bytes_per_step: usize,
    sink: &mut Sink<J, S>,
) -> Result<ChunkedEnd<J>, ChunkedRunError<J::Error, S>> {
    let n = job.steps();
    let log = opts.log || std::env::var_os("FDK_AAC_RS_CHUNK_LOG").is_some();
    // The test hooks that change the output (feature `test-hooks`): accept every boundary,
    // poison the unfed state of the speculative runs, trace the runs.
    let hook = |name: &str| cfg!(feature = "test-hooks") && std::env::var_os(name).is_some();
    let debug = hook("FDK_AAC_RS_CHUNK_DEBUG");
    let no_verify = opts.no_verify || hook("FDK_AAC_RS_CHUNK_NO_VERIFY");
    let poison = hook("FDK_AAC_RS_CHUNK_POISON");
    let mut chunks = opts.num_chunks(n, bytes_per_step).max(1);
    let mut starts: Vec<usize> = (0..=chunks).map(|j| j * n / chunks).collect();
    let mut probes = 0;
    if opts.chunks == 0 && chunks > 1 && opts.max_rejections > 0 {
        /* Probe chunks: the first boundaries close to the start (as close as a warm-up and the
        verification points allow), so that a stream whose states do not become equal is given
        up early, after max_rejections short chunks. */
        let max_verify = opts.verify_offsets.iter().copied().max().unwrap_or(0);
        let first = (1..n.min(starts[1])).find(|&s| job.warmup_start(s).is_some_and(|w| w > 0));
        if let Some(p) = first.map(|p| p.max(max_verify + 1)) {
            let last = p * opts.max_rejections;
            if last < starts[1] {
                let mut s: Vec<usize> = (0..=opts.max_rejections).map(|k| k * p).collect();
                s.extend((1..=chunks).map(|j| last + j * (n - last) / chunks));
                starts = s;
                chunks = starts.len() - 1;
                probes = opts.max_rejections;
            }
        }
    }
    let mut stats = ChunkedStats { chunks, ..Default::default() };
    let mut run_starts = vec![None; chunks];
    for j in 1..chunks {
        // (a run from step 0 would be another true run: only the true run starts there)
        run_starts[j] = job.warmup_start(starts[j]).filter(|&w| w > 0 && w <= starts[j]);
    }
    let viable = run_starts.iter().filter(|s| s.is_some()).count();
    let threads = opts.threads.max(1);
    let workers = threads.saturating_sub(1).min(chunks.saturating_sub(1));
    if chunks < 2 || viable == 0 || workers == 0 {
        if log {
            eprintln!("chunked: {n} steps, no chunks: sequential decoding");
        }
        stats.chunks = 1;
        stats.sequential_from = Some(0);
        let run = job.start(0).map_err(|e| ChunkedRunError::Run(0, e))?;
        return Ok(ChunkedEnd { run, base: J::Counters::default(), finished: false, stats });
    }
    let plan = Plan {
        starts,
        run_starts,
        run_cost: opts.run_cost,
        verify_offsets: if opts.verify_offsets.is_empty() {
            vec![0]
        } else {
            opts.verify_offsets.clone()
        },
    };
    if log {
        eprintln!(
            "chunked: {n} steps, {chunks} chunks ({viable} with a warm-up), {workers} worker threads"
        );
    }

    let mut true_run = job.start(0).map_err(|e| ChunkedRunError::Run(0, e))?;
    if opts.chunks == 0 {
        /* The pre-check of the automatic chunk count: the true run decodes the first step before
        any speculative run starts; a stream whose states rarely become equal at any boundary
        (ChunkJob::converges_rarely) is decoded sequentially, which saves the speculative runs
        whose boundaries would be rejected. */
        let mut sink_err = None;
        let mut counts = (0, 0);
        let base = J::Counters::default();
        let mut on = |e: Event<J>| -> bool {
            match e {
                Event::Out(_, o, _) => match sink(o, &base) {
                    Ok(()) => true,
                    Err(e) => {
                        sink_err = Some(e);
                        false
                    }
                },
                _ => true,
            }
        };
        let end = drive(job, &mut true_run, 1, &[], 0, false, false, None, &mut counts, &mut on);
        stats.steps += counts.0;
        if let Some(e) = sink_err {
            return Err(ChunkedRunError::Sink(e));
        }
        match end {
            RunEnd::Paused => {}
            RunEnd::Finished => {
                return Ok(ChunkedEnd { run: true_run, base, finished: true, stats });
            }
            RunEnd::Error(at, e) => return Err(ChunkedRunError::Run(at, e)),
            RunEnd::Stopped => unreachable!("run stopped without a sink error"),
        }
        if job.converges_rarely(&true_run) {
            let p = job.position(&true_run);
            if log {
                eprintln!("chunked: the states of this stream rarely become equal; sequential decoding from step {p}");
            }
            stats.rejected = chunks - 1;
            stats.sequential_from = Some(p);
            return Ok(ChunkedEnd { run: true_run, base, finished: false, stats });
        }
    }

    let budget = Budget::new(chunks, opts.max_buffered_bytes);
    let mut handles: Vec<ChunkHandle<J>> = Vec::with_capacity(chunks);
    let mut tasks = VecDeque::new();
    for j in 0..chunks {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(ChunkShared {
            state: AtomicU8::new(NOT_STARTED),
            cancel: AtomicBool::new(false),
            exempt: AtomicBool::new(false),
        });
        if j > 0 && plan.run_starts[j].is_some() {
            tasks.push_back((j, shared.clone(), tx));
        }
        handles.push(ChunkHandle {
            index: j,
            shared,
            rx: Some(rx),
            stash: VecDeque::new(),
            digests: BTreeMap::new(),
            ended: false,
        });
    }
    // One run per worker thread to begin with (the probe chunks first), the next ones as the
    // boundaries justify; see `Gate`.
    let gate = Gate {
        state: Mutex::new(GateState { tasks, limit: workers.max(probes).max(1) }),
        cv: Condvar::new(),
    };
    let steps_by_workers = AtomicUsize::new(0);
    let digests_by_workers = AtomicUsize::new(0);

    let result = std::thread::scope(|scope| {
        // (a worker that cannot be created is skipped: the sequential decoding absorbs the
        // chunks that no worker started)
        for _ in 0..workers {
            let spawned = std::thread::Builder::new().spawn_scoped(scope, || {
                while let Some((j, sh, tx)) = gate.next() {
                    if sh
                        .state
                        .compare_exchange(NOT_STARTED, STARTED, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        continue;
                    }
                    let (s, d) = speculate(job, &plan, j, debug, poison, &sh, &budget, &tx);
                    steps_by_workers.fetch_add(s, Ordering::Relaxed);
                    digests_by_workers.fetch_add(d, Ordering::Relaxed);
                }
            });
            if spawned.is_err() {
                break;
            }
        }
        let ctl = Control {
            debug,
            log,
            no_verify,
            max_rejections: opts.max_rejections,
            first_decides: opts.chunks == 0 && probes == 0,
            auto: opts.chunks == 0,
        };
        let r =
            coordinate(job, &plan, true_run, &mut handles, &budget, &gate, ctl, &mut stats, sink);
        /* stop the workers */
        gate.close();
        for h in handles.iter_mut() {
            h.discard(&budget);
        }
        r
    });
    stats.steps += steps_by_workers.into_inner();
    stats.digests += digests_by_workers.into_inner();
    if log {
        eprintln!("chunked: {stats:?}");
    }
    result.map(|(run, base, finished)| ChunkedEnd { run, base, finished, stats })
}

/// The run that currently produces the output.
enum Auth<J: ChunkJob> {
    /// Driven by the coordinator.
    Inline(Box<J::Run>),
    /// A speculative run still running on a worker (its events are consumed from its handle).
    Remote(usize),
}

/// Settings of the coordinator.
#[derive(Clone, Copy)]
struct Control {
    debug: bool,
    log: bool,
    no_verify: bool,
    max_rejections: usize,
    /// The automatic chunk count without probe chunks: the first boundary is the probe (the
    /// chunked decoding is given up if it is rejected before any boundary was accepted).
    first_decides: bool,
    /// The automatic chunk count.
    auto: bool,
}

type CoordResult<J, S> = Result<
    (<J as ChunkJob>::Run, <J as ChunkJob>::Counters, bool),
    ChunkedRunError<<J as ChunkJob>::Error, S>,
>;

#[allow(clippy::too_many_arguments)]
fn coordinate<J: ChunkJob, S>(
    job: &J,
    plan: &Plan,
    true_run: J::Run,
    handles: &mut [ChunkHandle<J>],
    budget: &Budget,
    gate: &Gate<J>,
    ctl: Control,
    stats: &mut ChunkedStats,
    sink: &mut Sink<J, S>,
) -> CoordResult<J, S> {
    let Control { debug, log, .. } = ctl;
    let chunks = plan.chunks();
    /* consecutive boundaries whose states differ */
    let mut rejections = 0usize;
    /* the true run while a speculative run produces the output (paused where it handed over) */
    let mut paused_true: Option<Box<J::Run>> = None;
    let mut auth: Auth<J> = Auth::Inline(Box::new(true_run));
    let mut auth_is_true = true;
    /* the base of the counters of the current run */
    let mut base = J::Counters::default();
    /* the output of steps before `valid_from` is not taken from the current run */
    let mut valid_from = 0usize;
    /* the chunk whose range the current run decodes */
    let mut k = 0usize;
    /* the digests of the current run */
    let mut auth_digests: BTreeMap<usize, DigestInfo<J::Counters>> = BTreeMap::new();
    let mut sink_err: Option<S> = None;
    /* where the current run stops next: the end of chunk k or a verification point */
    let mut stop = plan.end(0);

    loop {
        /* 1. the current run decodes up to `stop`; its output is passed on */
        let mut hazard_at: Option<usize> = None;
        let end: RunEnd<J::Error>;
        let cur_base = base;
        let mut emit = |step: usize, o: J::Out, sink_err: &mut Option<S>| -> bool {
            if step < valid_from {
                return true;
            }
            if let Err(e) = sink(o, &cur_base) {
                *sink_err = Some(e);
                return false;
            }
            true
        };
        let mut finished_run: Option<Box<J::Run>> = None;
        let remote = match &auth {
            Auth::Remote(h) => Some(*h),
            Auth::Inline(_) => None,
        };
        if let Some(h) = remote {
            let mut got: Option<RunEnd<J::Error>> = None;
            let mut paused_run = None;
            /* the step after the last output taken from the run */
            let mut next = valid_from;
            while got.is_none() {
                let Some(e) = handles[h].next_event() else {
                    // A speculative run that produces the output is never cancelled or limited:
                    // a closed channel means that its thread ended without a result (a panic,
                    // which the thread scope propagates at the end). The true run takes over
                    // as after a hazard.
                    got = Some(RunEnd::Stopped);
                    hazard_at = Some(next);
                    break;
                };
                match e {
                    Event::Out(step, o, hz) => {
                        budget.release(h, J::out_bytes(&o));
                        if hz && step >= valid_from {
                            hazard_at = Some(step);
                            got = Some(RunEnd::Stopped);
                        } else if !emit(step, o, &mut sink_err) {
                            got = Some(RunEnd::Stopped);
                        } else {
                            next = next.max(step + 1);
                        }
                    }
                    Event::Digest(p, d) => {
                        auth_digests.insert(p, d);
                    }
                    Event::Error(at, e) => got = Some(RunEnd::Error(at, e)),
                    Event::Paused(run) => {
                        budget.release(h, plan.run_cost);
                        paused_run = Some(run);
                        got = Some(RunEnd::Paused);
                    }
                    Event::Finished(run) => {
                        finished_run = Some(run);
                        got = Some(RunEnd::Finished);
                    }
                }
            }
            for (p, d) in std::mem::take(&mut handles[h].digests) {
                auth_digests.insert(p, d);
            }
            end = got.unwrap();
            if let Some(run) = paused_run {
                auth = Auth::Inline(run);
            }
        } else {
            let Auth::Inline(run) = &mut auth else { unreachable!() };
            let digest_at: Vec<usize> = if stop != usize::MAX { vec![stop] } else { Vec::new() };
            let mut counts = (0, 0);
            let mut on = |e: Event<J>| -> bool {
                match e {
                    Event::Out(step, o, hz) => {
                        if hz && !auth_is_true && step >= valid_from {
                            hazard_at = Some(step);
                            return false;
                        }
                        emit(step, o, &mut sink_err)
                    }
                    Event::Digest(p, d) => {
                        auth_digests.insert(p, d);
                        true
                    }
                    _ => true,
                }
            };
            end = drive(
                job,
                run,
                stop,
                &digest_at,
                valid_from,
                debug,
                false,
                None,
                &mut counts,
                &mut on,
            );
            stats.steps += counts.0;
            stats.digests += counts.1;
        }
        if let Some(e) = sink_err.take() {
            return Err(ChunkedRunError::Sink(e));
        }

        /* 2. a hazard in a speculative run: the true run takes over from there */
        if let Some(at) = hazard_at {
            if let Auth::Remote(h) = auth {
                handles[h].discard(budget);
            }
            let mut t = paused_true.take().expect("paused true run");
            if log {
                eprintln!(
                    "chunked: hazard at step {at} (chunk {k}); the true run continues from {}",
                    job.position(&t)
                );
            }
            stats.fallbacks += 1;
            /* decode up to the hazard without output */
            let mut counts = (0, 0);
            let mut on = |_e: Event<J>| true;
            match drive(job, &mut t, at, &[], usize::MAX, false, false, None, &mut counts, &mut on)
            {
                RunEnd::Paused => {}
                RunEnd::Error(at, e) => return Err(ChunkedRunError::Run(at, e)),
                RunEnd::Finished | RunEnd::Stopped => {
                    unreachable!("the true run ended before the hazard")
                }
            }
            stats.steps += counts.0;
            auth = Auth::Inline(t);
            auth_is_true = true;
            base = J::Counters::default();
            valid_from = at;
            auth_digests.clear();
            continue;
        }

        match end {
            RunEnd::Finished => {
                let run = match (finished_run, auth) {
                    (Some(r), _) => r,
                    (None, Auth::Inline(r)) => r,
                    (None, Auth::Remote(_)) => unreachable!(),
                };
                return Ok((*run, base, true));
            }
            RunEnd::Stopped => {
                // (only after a sink error, handled above)
                unreachable!("run stopped without a sink error")
            }
            RunEnd::Error(at, e) => return Err(ChunkedRunError::Run(at, e)),
            RunEnd::Paused => {}
        }

        /* 3. at a verification point of the boundary to chunk j */
        let Auth::Inline(run) = &auth else { unreachable!() };
        let p = job.position(run);
        let j = k + 1;
        debug_assert!(j < chunks);
        let hj = &mut handles[j];
        let started = match hj.shared.state.compare_exchange(
            NOT_STARTED,
            ABSORBED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => false,
            Err(s) => s == STARTED,
        };
        let mut handoff = false;
        let mut reason = "not started";
        let mut theirs_counters = None;
        if started {
            reason = "no state digest";
            /* its state digest is awaited: exempt from the buffer limit */
            hj.shared.exempt.store(true, Ordering::Release);
            budget.wake();
            if let (Some(mine), Some(theirs)) = (auth_digests.get(&p), hj.wait_digest(p)) {
                reason = "the states differ";
                if mine.digest == theirs.digest || ctl.no_verify {
                    handoff = true;
                    theirs_counters = Some((mine.counters, theirs.counters));
                } else if let (Some(a), Some(b)) = (&mine.trace, &theirs.trace) {
                    eprintln!("chunked: the states differ before step {p} (chunk {j}):");
                    for f in diff_traces(a, b) {
                        eprintln!("  {f}");
                    }
                }
            }
        }
        if handoff {
            rejections = 0;
            /* every speculative run may start */
            gate.set_limit(usize::MAX);
            if log {
                eprintln!("chunked: chunk {j} takes over at step {p}");
            }
            stats.handoffs += 1;
            let (mine, theirs) = theirs_counters.unwrap();
            base = base + mine - theirs;
            let Auth::Inline(run) = std::mem::replace(&mut auth, Auth::Remote(j)) else {
                unreachable!()
            };
            if auth_is_true {
                paused_true = Some(run);
            }
            auth_is_true = false;
            valid_from = p;
            k = j;
            auth_digests.clear();
            hj.shared.exempt.store(true, Ordering::Release);
            budget.wake();
            stop = plan.end(k);
            continue;
        }
        let next = plan.candidates(j).into_iter().find(|&c| c > p);
        if let Some(next) = next.filter(|&c| started && !hj.ended_before(c)) {
            /* verify again at the next verification point */
            stop = next;
            continue;
        }
        /* the current run continues through chunk j */
        if log {
            let why = match hj.stash.iter().find_map(|e| match e {
                Event::Error(at, _) => Some(*at),
                _ => None,
            }) {
                Some(at) => format!("{reason}: error at step {at}"),
                None => reason.to_string(),
            };
            eprintln!("chunked: chunk {j} rejected at step {p} ({why}); the current run continues");
        }
        stats.rejected += 1;
        hj.discard(budget);
        k = j;
        stop = plan.end(k);
        if reason == "the states differ" {
            rejections += 1;
        }
        /* the runs that may start: all while the boundaries are accepted but for single
        rejections, otherwise (before the first accepted boundary, after two rejections in a row)
        the next one */
        if stats.handoffs == 0 || rejections >= 2 {
            gate.set_limit(j + 1);
        }
        let rarely = ctl.auto
            && auth_is_true
            && matches!(&auth, Auth::Inline(run) if job.converges_rarely(run));
        let max_rejections = if (ctl.first_decides || rarely) && stats.handoffs == 0 {
            ctl.max_rejections.min(1)
        } else {
            ctl.max_rejections
        };
        if auth_is_true && max_rejections > 0 && rejections >= max_rejections {
            /* the states do not become equal: the rest is left to the sequential decoding */
            if log {
                eprintln!(
                    "chunked: {rejections} boundaries rejected; sequential decoding from step {p}"
                );
            }
            for h in handles[j + 1..].iter_mut() {
                let _ = h.shared.state.compare_exchange(
                    NOT_STARTED,
                    ABSORBED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                h.discard(budget);
            }
            stats.rejected += chunks - 1 - j;
            stats.sequential_from = Some(p);
            let Auth::Inline(run) = auth else { unreachable!() };
            return Ok((*run, base, false));
        }
    }
}

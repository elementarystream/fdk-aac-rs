// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the worker thread of the frame pipeline.

//! The worker thread that runs stage B of the frame pipeline.
//!
//! One job is in flight at a time. The hand-off uses an atomic state word: the worker spins
//! for up to [`WORKER_SPIN`] after a job before it parks (a frame takes 20 to 300 µs, so a
//! decoder that decodes continuously never parks it), and the caller of [`Worker::wait`] spins
//! for up to [`WAIT_SPIN`] before it parks. The layer is small on purpose, so that it can be
//! replaced by a shared thread pool.

use super::{stage_b, HandOver, MAX_FRAME_SAMPLES};
use crate::aacdecoder::AacDecoderInstance;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

/// How long the worker spins for the next job before it parks.
const WORKER_SPIN: Duration = Duration::from_micros(300);
/// How long [`Worker::wait`] spins before it parks.
const WAIT_SPIN: Duration = Duration::from_millis(2);

const IDLE: u32 = 0;
const POSTED: u32 = 1;
const DONE: u32 = 2;
const EXIT: u32 = 3;

/// A stage B job: the instance that runs it, the hand-over of stage A, the output buffer, the
/// thread count of the per channel parallelism.
pub(crate) struct Job {
    pub shadow: Box<AacDecoderInstance>,
    pub ho: Box<HandOver>,
    pub out: Vec<IntPcm>,
    pub cap: usize,
}

/// A finished job: the instance, the hand-over, the output and the return value of the call.
pub(crate) struct Done {
    pub shadow: Box<AacDecoderInstance>,
    pub ho: Box<HandOver>,
    pub out: Vec<IntPcm>,
    /// The number of valid output samples.
    pub n: usize,
    pub err: i32,
}

struct Shared {
    state: AtomicU32,
    job: Mutex<Option<Job>>,
    done: Mutex<Option<Box<Done>>>,
    worker_parked: AtomicBool,
    waiter: Mutex<Option<Thread>>,
}

/// The worker thread of a decoder. If the thread cannot be created, jobs run in the calling
/// thread when they are posted (stage B of a frame and stage A of the next one are independent,
/// so this order gives the same results).
pub(crate) struct Worker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    inline: bool,
}

impl Worker {
    pub(crate) fn new() -> Worker {
        let shared = Arc::new(Shared {
            state: AtomicU32::new(IDLE),
            job: Mutex::new(None),
            done: Mutex::new(None),
            worker_parked: AtomicBool::new(false),
            waiter: Mutex::new(None),
        });
        let s = shared.clone();
        match thread::Builder::new().name("fdk-aac-rs-dec-b".into()).spawn(move || run(&s)) {
            Ok(thread) => Worker { shared, thread: Some(thread), inline: false },
            Err(_) => Worker { shared, thread: None, inline: true },
        }
    }

    /// Starts a job (no job may be in flight).
    pub(crate) fn post(&self, job: Job) {
        if self.inline {
            execute(&self.shared, job);
            return;
        }
        *self.shared.job.lock().unwrap_or_else(|e| e.into_inner()) = Some(job);
        self.shared.state.store(POSTED, Ordering::SeqCst);
        if self.shared.worker_parked.load(Ordering::SeqCst) {
            if let Some(t) = self.thread.as_ref() {
                t.thread().unpark();
            }
        }
    }

    /// Waits for the job in flight.
    pub(crate) fn wait(&mut self) -> Box<Done> {
        let s = &*self.shared;
        let start = Instant::now();
        let mut n = 0u32;
        while s.state.load(Ordering::Acquire) != DONE {
            n = n.wrapping_add(1);
            if n % 64 != 0 {
                core::hint::spin_loop();
                continue;
            }
            if start.elapsed() < WAIT_SPIN {
                core::hint::spin_loop();
                continue;
            }
            *s.waiter.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread::current());
            if s.state.load(Ordering::SeqCst) != DONE {
                thread::park_timeout(Duration::from_millis(50));
            }
            *s.waiter.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let done = s.done.lock().unwrap_or_else(|e| e.into_inner()).take();
        match done {
            Some(d) => {
                s.state.store(IDLE, Ordering::SeqCst);
                d
            }
            // Stage B panicked (a bug); propagate it. The state stays DONE, so that every later
            // wait for the lost job panics as well instead of waiting forever.
            None => panic!("the decoder worker thread failed"),
        }
    }

    /// Stops the worker thread (no job may be in flight).
    pub(crate) fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shared.state.store(EXIT, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}

fn run(s: &Shared) {
    loop {
        // Wait for a job.
        let mut start = Instant::now();
        let mut n = 0u32;
        loop {
            match s.state.load(Ordering::Acquire) {
                POSTED => break,
                EXIT => return,
                _ => {}
            }
            n = n.wrapping_add(1);
            if n % 64 != 0 || start.elapsed() < WORKER_SPIN {
                core::hint::spin_loop();
                continue;
            }
            s.worker_parked.store(true, Ordering::SeqCst);
            let st = s.state.load(Ordering::SeqCst);
            if st != POSTED && st != EXIT {
                thread::park();
            }
            s.worker_parked.store(false, Ordering::SeqCst);
            start = Instant::now();
        }
        let job = s.job.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(job) = job else {
            continue;
        };
        execute(s, job);
    }
}

/// Runs a job and publishes its result (`None` if it panicked).
fn execute(s: &Shared, job: Job) {
    let Job { mut shadow, mut ho, mut out, cap } = job;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut td2 = shadow.p_time_data2.take().unwrap_or_default();
        let (err, n) = stage_b(&mut shadow, &mut ho, &mut td2, &mut out, None, cap);
        shadow.p_time_data2 = Some(td2);
        debug_assert!(n <= MAX_FRAME_SAMPLES);
        (err, n)
    }));
    *s.done.lock().unwrap_or_else(|e| e.into_inner()) = match result {
        Ok((err, n)) => Some(Box::new(Done { shadow, ho, out, n, err })),
        Err(_) => None,
    };
    s.state.store(DONE, Ordering::SeqCst);
    if let Some(t) = s.waiter.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        t.unpark();
    }
}

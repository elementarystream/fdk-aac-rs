// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not a port: the reference library is single-threaded.

//! Fork-join thread pool of the opt-in multi-threading (PORTING.md, "Optimization rules").
//!
//! A codec instance with a thread count above 1 owns one [`ThreadPool`] and splits parts of a
//! frame into independent tasks (for example the scale factor estimation of every channel). The
//! calling thread takes part in the work, and [`ThreadPool::for_each`] returns when all tasks are
//! done. A frame takes 20 to 500 µs, so the parallel sections are short and closely spaced: the
//! workers spin for a while ([`DEFAULT_SPIN`]) after their last task before they park, which
//! keeps a fork-join round at about a microsecond while frames are encoded back to back and
//! costs no CPU time while the instance is idle. Parked workers are woken only when a job has a
//! task for them.
//!
//! The pool never changes results: the tasks of a job must be independent of each other (every
//! caller documents why its tasks are), so the order and the thread in which they run do not
//! matter. Nested use (a task calling the pool again) and concurrent use from several threads
//! are allowed; such calls run their tasks sequentially on the calling thread. A panic of a task
//! is propagated to the caller after all tasks have finished.
//!
//! Thread-local state: the port counts deviations and hazards in thread-local counters (for
//! tests), e.g. [`crate::fdk_assert::violations`]. A task that runs on a worker thread runs
//! inside [`Carry::run_on_worker`], which moves what the task added to the worker's counters to
//! the calling thread, so the counters read the same for every thread count. [`FdkCarry`] does
//! this for the counters of this crate; crates with counters of their own wrap it.
//!
//! Safety argument (the two `unsafe` blocks):
//!
//! - The job closure is borrowed by the workers with an erased lifetime. The job is published
//!   through `Shared::job` and retracted by `JobGuard` before [`ThreadPool::for_each_carry`]
//!   returns or unwinds; `JobGuard` then waits until `Shared::active` is 0. A worker increments
//!   `active` before it loads the job pointer and decrements it after its last access, and all
//!   three operations are `SeqCst`: a worker whose increment comes after the guard's last load
//!   of `active` in the single total order loads the pointer after the retraction (null, or a
//!   later job), so no worker accesses a job after its stack frame is gone.
//! - [`ThreadPool::for_each_mut`] hands out `&mut items[i]`; every index `i < n` is claimed
//!   exactly once (`Job::next.fetch_add`), so the mutable references are disjoint, and `items`
//!   is mutably borrowed for the whole call.

pub mod tune;

pub use tune::{Tuner, Tuning};

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Default time a worker spins waiting for the next job before it parks.
pub const DEFAULT_SPIN: Duration = Duration::from_micros(100);

/// Idle time after which a spinning worker yields its core between polls.
const YIELD_AFTER: Duration = Duration::from_micros(50);

/// Stack size of the worker threads (the default main thread stack of Linux and macOS).
const WORKER_STACK_SIZE: usize = 8 << 20;

/// The environment variable with the thread count of the C ABI (`FDK_AAC_RS_THREADS`).
pub const THREADS_ENV: &str = "FDK_AAC_RS_THREADS";

/// The environment variable with the thread count of the C ABI's decoders
/// (`FDK_AAC_RS_DEC_THREADS`); when set, it takes precedence over [`THREADS_ENV`].
pub const DEC_THREADS_ENV: &str = "FDK_AAC_RS_DEC_THREADS";

/// The environment variable with the thread count of the C ABI's encoders
/// (`FDK_AAC_RS_ENC_THREADS`); when set, it takes precedence over [`THREADS_ENV`].
pub const ENC_THREADS_ENV: &str = "FDK_AAC_RS_ENC_THREADS";

/// The thread count given by [`THREADS_ENV`] (see [`parse_threads`]).
pub fn env_threads() -> usize {
    parse_threads(std::env::var(THREADS_ENV).ok().as_deref())
}

/// The thread count of a C ABI decoder: [`DEC_THREADS_ENV`] if it is set, else [`THREADS_ENV`].
/// A decoder with more than one thread returns every frame one call later, so a caller that sets
/// only the encoder's variable keeps the sequential decoder.
pub fn env_dec_threads() -> usize {
    match std::env::var(DEC_THREADS_ENV) {
        Ok(v) => parse_threads(Some(&v)),
        Err(_) => env_threads(),
    }
}

/// The thread count of a C ABI encoder: [`ENC_THREADS_ENV`] if it is set, else [`THREADS_ENV`].
pub fn env_enc_threads() -> usize {
    match std::env::var(ENC_THREADS_ENV) {
        Ok(v) => parse_threads(Some(&v)),
        Err(_) => env_threads(),
    }
}

/// A thread count: a positive decimal integer (surrounding white space allowed), otherwise
/// (`None`, empty, invalid or 0) 1.
pub fn parse_threads(value: Option<&str>) -> usize {
    value.and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(1)
}

/// Aligns a value to its own cache line pair (false sharing between the spinning workers and
/// the counters written by the tasks).
#[repr(align(128))]
struct Padded<T>(T);

/// One fork-join round, owned by the stack frame of [`ThreadPool::for_each_carry`].
struct Job {
    epoch: u64,
    n: usize,
    next: AtomicUsize,
    /// The task function of the workers (the task inside [`Carry::run_on_worker`]); the lifetime
    /// is erased, see the module documentation.
    f: &'static (dyn Fn(usize) + Sync),
    /// Set if a task panicked in a worker.
    panicked: AtomicBool,
    /// The payload of the first panic of a task in a worker.
    panic: Mutex<Option<Box<dyn std::any::Any + Send>>>,
}

struct Shared {
    /// The current job, null if none. Workers only dereference it while they are counted in
    /// `active`.
    job: Padded<AtomicPtr<Job>>,
    /// Incremented for every job (after `job` is set).
    epoch: Padded<AtomicU64>,
    /// Number of workers that may access the current job.
    active: Padded<AtomicUsize>,
    /// Per worker: set while the worker is parked (or about to park).
    parked: Box<[Padded<AtomicBool>]>,
    shutdown: AtomicBool,
    spin: Duration,
}

/// A fork-join thread pool with `threads - 1` worker threads (the thread calling
/// [`ThreadPool::for_each`] is the remaining one).
pub struct ThreadPool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    /// Set while a job runs (nested or concurrent calls run sequentially).
    busy: AtomicBool,
}

impl std::fmt::Debug for ThreadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadPool").field("threads", &self.threads()).finish()
    }
}

/// Thread-local state of the calling thread that the tasks on worker threads share (see the
/// module documentation).
pub trait Carry: Sync {
    /// Runs `task` on a worker thread on behalf of the calling thread.
    fn run_on_worker(&self, task: &mut dyn FnMut());
    /// Called on the calling thread after all tasks of the job have finished (also if one
    /// panicked), before [`ThreadPool::for_each_carry`] returns.
    fn finish(&self);
}

/// Accumulates what the tasks on worker threads added to a `u64` counter.
#[derive(Debug, Default)]
pub struct CounterCarry(AtomicU64);

impl CounterCarry {
    /// Runs `task` and keeps what it added to the counter read by `get`.
    #[inline]
    pub fn around(&self, get: fn() -> u64, task: &mut dyn FnMut()) {
        let before = get();
        task();
        let added = get().wrapping_sub(before);
        if added != 0 {
            self.0.fetch_add(added, Ordering::Relaxed);
        }
    }

    /// Keeps `n` more (what a task added to the counter).
    #[inline]
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    /// Adds the kept amount to the calling thread's counter with `add` and clears it.
    #[inline]
    pub fn finish(&self, add: fn(u64)) {
        let added = self.0.swap(0, Ordering::Relaxed);
        if added != 0 {
            add(added);
        }
    }
}

/// The [`Carry`] of the thread-local counters of this crate: [`crate::fdk_assert::violations`]
/// and [`crate::mdct::mdct_block_dropped_accesses`].
#[derive(Debug, Default)]
pub struct FdkCarry {
    violations: CounterCarry,
    dropped: [CounterCarry; 3],
}

impl Carry for FdkCarry {
    fn run_on_worker(&self, task: &mut dyn FnMut()) {
        let [w, r, s] = &self.dropped;
        self.violations.around(crate::fdk_assert::violations, &mut || {
            w.around(|| crate::mdct::mdct_block_dropped_accesses().0, &mut || {
                r.around(|| crate::mdct::mdct_block_dropped_accesses().1, &mut || {
                    s.around(|| crate::mdct::mdct_block_dropped_accesses().2, task)
                })
            })
        });
    }

    fn finish(&self) {
        let [w, r, s] = &self.dropped;
        self.violations.finish(crate::fdk_assert::add_violations);
        w.finish(|v| crate::mdct::add_mdct_block_dropped_accesses((v, 0, 0)));
        r.finish(|v| crate::mdct::add_mdct_block_dropped_accesses((0, v, 0)));
        s.finish(|v| crate::mdct::add_mdct_block_dropped_accesses((0, 0, v)));
    }
}

impl ThreadPool {
    /// Creates a pool for `threads` threads in total (at least 2), i.e. spawns `threads - 1`
    /// workers, which spin [`DEFAULT_SPIN`] before they park.
    pub fn new(threads: usize) -> std::io::Result<ThreadPool> {
        ThreadPool::with_spin(threads, DEFAULT_SPIN)
    }

    /// Like [`ThreadPool::new`] with the time `spin` the workers wait actively for the next job.
    pub fn with_spin(threads: usize, spin: Duration) -> std::io::Result<ThreadPool> {
        let n_workers = threads.max(2) - 1;
        let shared = Arc::new(Shared {
            job: Padded(AtomicPtr::new(std::ptr::null_mut())),
            epoch: Padded(AtomicU64::new(0)),
            active: Padded(AtomicUsize::new(0)),
            parked: (0..n_workers).map(|_| Padded(AtomicBool::new(false))).collect(),
            shutdown: AtomicBool::new(false),
            spin,
        });
        let mut pool = ThreadPool { shared, workers: Vec::new(), busy: AtomicBool::new(false) };
        for idx in 0..n_workers {
            let shared = pool.shared.clone();
            // (on an error the workers spawned so far are stopped by the drop of `pool`)
            let handle = thread::Builder::new()
                .name(format!("fdk-aac-rs-worker-{idx}"))
                .stack_size(WORKER_STACK_SIZE)
                .spawn(move || worker_main(&shared, idx))?;
            pool.workers.push(handle);
        }
        Ok(pool)
    }

    /// The number of threads (workers and the calling thread).
    pub fn threads(&self) -> usize {
        self.workers.len() + 1
    }

    /// Runs `f(i)` for every `i` in `0..n` on the pool and the calling thread and returns when
    /// all calls returned. The calls must be independent of each other. A panic of a task is
    /// propagated after all tasks finished. The counters of this crate are carried
    /// ([`FdkCarry`]).
    pub fn for_each<F: Fn(usize) + Sync>(&self, n: usize, f: F) {
        self.for_each_carry(&FdkCarry::default(), n, f)
    }

    /// [`ThreadPool::for_each`] with the thread-local state `carry`.
    pub fn for_each_carry<C: Carry, F: Fn(usize) + Sync>(&self, carry: &C, n: usize, f: F) {
        if n <= 1 || self.workers.is_empty() {
            (0..n).for_each(f);
            return;
        }
        if self.busy.swap(true, Ordering::Acquire) {
            // nested or concurrent use
            (0..n).for_each(f);
            return;
        }
        let worker_f = |i: usize| carry.run_on_worker(&mut || f(i));
        let worker_f: &(dyn Fn(usize) + Sync) = &worker_f;
        // SAFETY: only the lifetime is changed. The job (holding the reference) is retracted and
        // all workers have left it before this function returns or unwinds (`JobGuard`, see the
        // module documentation).
        let worker_f: &'static (dyn Fn(usize) + Sync) = unsafe { std::mem::transmute(worker_f) };
        let shared = &*self.shared;
        let epoch = shared.epoch.0.load(Ordering::Relaxed) + 1;
        let job = Job {
            epoch,
            n,
            next: AtomicUsize::new(0),
            f: worker_f,
            panicked: AtomicBool::new(false),
            panic: Mutex::new(None),
        };
        let guard = JobGuard { pool: self };
        shared.job.0.store(&job as *const Job as *mut Job, Ordering::SeqCst);
        shared.epoch.0.store(epoch, Ordering::SeqCst);
        // wake parked workers that have a task (worker w takes part if n > w + 1)
        for (w, handle) in self.workers.iter().enumerate().take(n - 1) {
            let flag = &shared.parked[w].0;
            if flag.load(Ordering::SeqCst) && flag.swap(false, Ordering::SeqCst) {
                handle.thread().unpark();
            }
        }
        // the calling thread's share (its counters need no carry)
        let r = catch_unwind(AssertUnwindSafe(|| loop {
            let i = job.next.fetch_add(1, Ordering::Relaxed);
            if i >= n {
                break;
            }
            f(i);
        }));
        drop(guard);
        carry.finish();
        if let Err(payload) = r {
            resume_unwind(payload);
        }
        if job.panicked.load(Ordering::Relaxed) {
            let payload = job.panic.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(payload) = payload {
                resume_unwind(payload);
            }
        }
    }

    /// Runs `f(i, &mut items[i])` for every item on the pool and the calling thread (see
    /// [`ThreadPool::for_each`]).
    pub fn for_each_mut<T: Send, F: Fn(usize, &mut T) + Sync>(&self, items: &mut [T], f: F) {
        self.for_each_mut_carry(&FdkCarry::default(), items, f)
    }

    /// [`ThreadPool::for_each_mut`] with the thread-local state `carry`.
    pub fn for_each_mut_carry<C: Carry, T: Send, F: Fn(usize, &mut T) + Sync>(
        &self,
        carry: &C,
        items: &mut [T],
        f: F,
    ) {
        let base = SyncMutPtr(items.as_mut_ptr());
        self.for_each_carry(carry, items.len(), |i| {
            // SAFETY: `for_each_carry` calls the closure exactly once per index `i <
            // items.len()`, so the mutable references are disjoint; `items` is borrowed for the
            // whole call.
            let item = unsafe { base.at(i) };
            f(i, item)
        });
    }
}

/// Runs `f(i, &mut items[i])` for every item, on `pool` if given, otherwise sequentially in
/// index order.
#[inline]
pub fn for_each_mut<T: Send, F: Fn(usize, &mut T) + Sync>(
    pool: Option<&ThreadPool>,
    items: &mut [T],
    f: F,
) {
    match pool {
        Some(pool) => pool.for_each_mut(items, f),
        None => {
            for (i, item) in items.iter_mut().enumerate() {
                f(i, item);
            }
        }
    }
}

struct SyncMutPtr<T>(*mut T);

// SAFETY: used only to hand out disjoint `&mut T` to other threads (requires `T: Send`).
unsafe impl<T: Send> Sync for SyncMutPtr<T> {}

impl<T> SyncMutPtr<T> {
    /// # Safety
    /// `i` must be in bounds and no other reference to the element may exist.
    #[inline(always)]
    unsafe fn at<'a>(&self, i: usize) -> &'a mut T {
        &mut *self.0.add(i)
    }
}

/// Retracts the job and waits until no worker accesses it (also when a task of the calling
/// thread panics).
struct JobGuard<'a> {
    pool: &'a ThreadPool,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        let shared = &*self.pool.shared;
        shared.job.0.store(std::ptr::null_mut(), Ordering::SeqCst);
        let mut spins = 0u32;
        while shared.active.0.load(Ordering::SeqCst) != 0 {
            spins += 1;
            if spins < 1 << 12 {
                std::hint::spin_loop();
            } else {
                // a worker was descheduled while running a task
                thread::yield_now();
            }
        }
        self.pool.busy.store(false, Ordering::Release);
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        let shared = &*self.shared;
        shared.shutdown.store(true, Ordering::SeqCst);
        for (w, handle) in self.workers.iter().enumerate() {
            shared.parked[w].0.store(false, Ordering::SeqCst);
            handle.thread().unpark();
        }
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

fn worker_main(shared: &Shared, idx: usize) {
    let mut seen = 0u64;
    let mut last_work = Instant::now();
    loop {
        // wait for the next job
        let mut spins = 0u32;
        let epoch = loop {
            let e = shared.epoch.0.load(Ordering::Acquire);
            if e != seen {
                break e;
            }
            spins = spins.wrapping_add(1);
            if spins % 64 == 0 {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                let idle = last_work.elapsed();
                if idle >= shared.spin {
                    park(shared, idx, seen);
                    if shared.shutdown.load(Ordering::SeqCst) {
                        return;
                    }
                    last_work = Instant::now();
                } else if idle >= YIELD_AFTER {
                    // let other threads run on this core if the machine is oversubscribed
                    thread::yield_now();
                }
            }
            std::hint::spin_loop();
        };

        // join the job
        shared.active.0.fetch_add(1, Ordering::SeqCst);
        let p = shared.job.0.load(Ordering::SeqCst);
        if p.is_null() {
            // the job of `epoch` is already finished
            seen = epoch;
        } else {
            // SAFETY: the job stays valid while this worker is counted in `active` and the
            // pointer was loaded after the increment (see `JobGuard`).
            let job = unsafe { &*p };
            seen = job.epoch;
            if job.n > idx + 1 {
                last_work = Instant::now();
                loop {
                    let i = job.next.fetch_add(1, Ordering::Relaxed);
                    if i >= job.n {
                        break;
                    }
                    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| (job.f)(i))) {
                        let mut slot = job.panic.lock().unwrap_or_else(|e| e.into_inner());
                        slot.get_or_insert(payload);
                        job.panicked.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        shared.active.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Parks the worker until a job newer than `seen` is published (or the pool is dropped).
fn park(shared: &Shared, idx: usize, seen: u64) {
    let flag = &shared.parked[idx].0;
    flag.store(true, Ordering::SeqCst);
    while flag.load(Ordering::SeqCst) {
        if shared.epoch.0.load(Ordering::SeqCst) != seen || shared.shutdown.load(Ordering::SeqCst) {
            flag.store(false, Ordering::SeqCst);
            break;
        }
        thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_every_task_once() {
        let pool = ThreadPool::new(4).unwrap();
        for n in [0usize, 1, 2, 3, 7, 100] {
            let mut v = vec![0u32; n];
            pool.for_each_mut(&mut v, |i, x| *x += i as u32 + 1);
            assert!(v.iter().enumerate().all(|(i, &x)| x == i as u32 + 1));
        }
    }

    #[test]
    fn many_rounds_and_parking() {
        let pool = ThreadPool::with_spin(3, Duration::from_micros(10)).unwrap();
        let mut v = vec![0u64; 5];
        for round in 0..20000u64 {
            pool.for_each_mut(&mut v, |i, x| *x += i as u64 * round);
            if round % 1000 == 0 {
                thread::sleep(Duration::from_micros(200));
            }
        }
        let s: u64 = (0..20000u64).sum();
        assert!(v.iter().enumerate().all(|(i, &x)| x == i as u64 * s));
    }

    #[test]
    fn workers_park_when_idle() {
        let pool = ThreadPool::with_spin(4, Duration::from_micros(50)).unwrap();
        let mut v = vec![0u8; 4];
        pool.for_each_mut(&mut v, |_, x| *x = 1);
        // after the spin time every worker parks (the flag is set while it is parked)
        let t = Instant::now();
        while !pool.shared.parked.iter().all(|p| p.0.load(Ordering::SeqCst)) {
            assert!(t.elapsed() < Duration::from_secs(10), "workers did not park");
            thread::sleep(Duration::from_millis(1));
        }
        // and they wake up for the next job
        pool.for_each_mut(&mut v, |_, x| *x = 2);
        assert!(v.iter().all(|&x| x == 2));
    }

    #[test]
    fn nested_calls_run_sequentially() {
        let pool = ThreadPool::new(4).unwrap();
        let mut v = vec![[0u32; 4]; 4];
        pool.for_each_mut(&mut v, |_, row| {
            pool.for_each_mut(row, |j, x| *x = j as u32);
        });
        assert!(v.iter().all(|r| *r == [0, 1, 2, 3]));
    }

    #[test]
    fn panics_propagate() {
        let pool = ThreadPool::new(4).unwrap();
        for bad in [0usize, 63] {
            let r = catch_unwind(AssertUnwindSafe(|| {
                pool.for_each(64, |i| {
                    if i == bad {
                        panic!("task panic");
                    }
                    thread::sleep(Duration::from_micros(10));
                })
            }));
            assert!(r.is_err());
        }
        // the pool stays usable
        let mut v = vec![0u8; 16];
        pool.for_each_mut(&mut v, |_, x| *x = 1);
        assert!(v.iter().all(|&x| x == 1));
    }

    #[test]
    fn counters_are_carried_to_the_caller() {
        let pool = ThreadPool::new(4).unwrap();
        crate::fdk_assert::reset_violations();
        let before = crate::mdct::mdct_block_dropped_accesses();
        for _ in 0..100 {
            pool.for_each(8, |_| {
                crate::fdk_assert::add_violations(1);
                crate::mdct::add_mdct_block_dropped_accesses((1, 2, 3));
                thread::sleep(Duration::from_micros(20));
            });
        }
        assert_eq!(crate::fdk_assert::violations(), 800);
        let after = crate::mdct::mdct_block_dropped_accesses();
        assert_eq!((after.0 - before.0, after.1 - before.1, after.2 - before.2), (800, 1600, 2400));
    }

    #[test]
    fn thread_counts_parse() {
        assert_eq!(parse_threads(None), 1);
        assert_eq!(parse_threads(Some("")), 1);
        assert_eq!(parse_threads(Some("0")), 1);
        assert_eq!(parse_threads(Some("-2")), 1);
        assert_eq!(parse_threads(Some("x")), 1);
        assert_eq!(parse_threads(Some("4")), 4);
        assert_eq!(parse_threads(Some(" 8\n")), 8);
    }

    /// Measures fork-join rounds of two 20 µs tasks every `gap` µs (FDK_AAC_RS_POOL_BENCH=1).
    #[test]
    fn wake_latency_bench() {
        if std::env::var_os("FDK_AAC_RS_POOL_BENCH").is_none() {
            return;
        }
        let busy = |us: u64| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_micros(us) {
                std::hint::spin_loop();
            }
        };
        for spin in [0u64, 10, 30, 100] {
            for gap in [10u64, 30, 60, 150] {
                let pool = ThreadPool::with_spin(2, Duration::from_micros(spin)).unwrap();
                let mut worst = Duration::ZERO;
                let mut total = Duration::ZERO;
                let rounds = 4000;
                for _ in 0..rounds {
                    busy(gap);
                    let t = Instant::now();
                    pool.for_each(2, |_| busy(20));
                    let d = t.elapsed();
                    total += d;
                    worst = worst.max(d);
                }
                eprintln!(
                    "spin {spin:3} us, gap {gap:3} us: round {:6.1} us (ideal 20), worst {:6.1} us",
                    total.as_secs_f64() * 1e6 / rounds as f64,
                    worst.as_secs_f64() * 1e6
                );
            }
        }
    }

    #[test]
    fn optional_pool() {
        let mut v = vec![0usize; 10];
        for_each_mut(None, &mut v, |i, x| *x = i);
        assert!(v.iter().enumerate().all(|(i, &x)| x == i));
    }
}

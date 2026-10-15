//! Guarded calls into the C++ reference (src/guard.c).

use core::ffi::c_void;

extern "C" {
    fn fz_ref_begin(budget_ms: i64, pattern: i32);
    fn fz_ref_end();
    fn fz_ref_arena() -> i32;
    fn fz_ref_arena_max() -> u64;
    fn fz_guard_call(f: extern "C" fn(*mut c_void), ctx: *mut c_void, budget: i32) -> i32;
}

/// Why a reference call did not return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abort {
    /// A fatal signal (its number) inside the call.
    Signal(i32),
    /// The CPU-time budget of the input ran out.
    Timeout,
    /// The reference asked for more memory than its arena holds (src/guard.c).
    ArenaFull,
    /// Not called: the call reaches a documented deviation without a defined reference result
    /// (the reason; PORTING.md, "Deviations from the reference"). The port decides it in a first
    /// run.
    Deviation(&'static str),
}

/// The largest arena use of a reference run so far, in bytes (0 without the arena).
pub fn ref_arena_max() -> u64 {
    // SAFETY: plain C call.
    unsafe { fz_ref_arena_max() }
}

/// Whether the reference allocates from an arena of its own (Linux; src/guard.c).
pub fn ref_arena() -> bool {
    // SAFETY: plain C call.
    unsafe { fz_ref_arena() != 0 }
}

impl core::fmt::Display for Abort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Abort::Signal(s) => write!(f, "signal {s} ({})", signal_name(s)),
            Abort::Timeout => write!(f, "CPU-time budget exhausted"),
            Abort::ArenaFull => write!(f, "reference arena full"),
            Abort::Deviation(why) => write!(f, "not called: {why}"),
        }
    }
}

pub fn signal_name(s: i32) -> &'static str {
    match s {
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 | 10 => "SIGBUS",
        8 => "SIGFPE",
        11 => "SIGSEGV",
        _ => "?",
    }
}

/// The CPU-time budget of the reference part of one input, in milliseconds
/// (`FDK_FUZZ_REF_BUDGET_MS`, default 10000; 0 disables it). With the reference arena (Linux) a
/// call that exceeds it is abandoned; otherwise the budget is soft: the call completes and the
/// following calls report it (src/guard.c).
pub fn budget_ms() -> i64 {
    static B: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *B.get_or_init(|| {
        std::env::var("FDK_FUZZ_REF_BUDGET_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000)
    })
}

/// The reference part of one input: arms the budget for its lifetime.
pub struct Session {
    /// Byte written into the stack below each reference call and into its new heap blocks.
    pub pattern: i32,
}

impl Session {
    pub fn begin(pattern: i32) -> Session {
        Session::with_budget(pattern, budget_ms())
    }

    pub fn with_budget(pattern: i32, budget_ms: i64) -> Session {
        // SAFETY: plain C calls.
        unsafe { fz_ref_begin(budget_ms, pattern) };
        Session { pattern }
    }

    /// Runs `f` (which must only call the C++ reference, and must not own values with
    /// destructors: a fatal signal leaves it through `siglongjmp`). Once the CPU budget of the session
    /// is used up, returns `Abort::Timeout` without calling `f`.
    pub fn call<F: FnMut()>(&self, f: F) -> Result<(), Abort> {
        self.call_with(f, true)
    }

    /// As [`Session::call`], also after the budget is used up (to close the handle).
    pub fn call_unbudgeted<F: FnMut()>(&self, f: F) -> Result<(), Abort> {
        self.call_with(f, false)
    }

    fn call_with<F: FnMut()>(&self, mut f: F, budget: bool) -> Result<(), Abort> {
        extern "C" fn tramp<F: FnMut()>(ctx: *mut c_void) {
            // SAFETY: ctx is the `&mut F` passed below, alive for the call.
            let f = unsafe { &mut *(ctx as *mut F) };
            f();
        }
        // SAFETY: the trampoline and the closure hold no values with destructors; the C side
        // only jumps back over them.
        let r =
            unsafe { fz_guard_call(tramp::<F>, &mut f as *mut F as *mut c_void, budget as i32) };
        match r {
            0 => Ok(()),
            -1 => Err(Abort::Timeout),
            -2 => Err(Abort::ArenaFull),
            s => Err(Abort::Signal(s)),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: plain C call.
        unsafe { fz_ref_end() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_faults_and_hangs() {
        let s = Session::with_budget(0, 300);
        // SAFETY: none; the write faults and the guard jumps back.
        let r = s.call(|| unsafe { core::ptr::null_mut::<i32>().write_volatile(1) });
        assert!(matches!(r, Err(Abort::Signal(7 | 10 | 11))), "{r:?}");
        // a call that uses up the budget: abandoned with the arena, else it completes; the
        // following calls report it
        let start = std::time::Instant::now();
        let mut x = 0u64;
        let r = s.call(|| {
            while start.elapsed().as_millis() < 600 {
                x = std::hint::black_box(x.wrapping_add(1));
            }
        });
        assert_eq!(r, if ref_arena() { Err(Abort::Timeout) } else { Ok(()) });
        assert_eq!(s.call(|| {}), Err(Abort::Timeout));
        assert_eq!(s.call_unbudgeted(|| {}), Ok(()));
        drop(s);
        let s = Session::with_budget(0, 300);
        assert_eq!(s.call(|| {}), Ok(()));
    }
}

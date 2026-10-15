// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSYS/include/machine_type.h (FDK_ASSERT).

//! `FDK_ASSERT` (PORTING.md, "Errors and assertions").
//!
//! The reference is built with `NDEBUG`, so its assertions are compiled out and it continues on
//! every input, including corrupt streams that violate an assertion. The port must continue the
//! same way. [`fdk_assert!`](macro@crate::fdk_assert) therefore never changes behavior:
//!
//! - without `debug_assertions` (the `release` profile) the condition is not evaluated;
//! - with `debug_assertions` (the `checked` profile) a violated condition is counted in a
//!   thread-local counter ([`violations`]) and does not panic, so that the `checked` profile can
//!   run the corrupt-stream corpus;
//! - with `FDK_AAC_RS_STRICT_ASSERT=1` in the environment, a violation panics, which finds the
//!   first violated assertion of a run.

use std::cell::Cell;
use std::sync::OnceLock;

thread_local! {
    static VIOLATIONS: Cell<u64> = const { Cell::new(0) };
}

/// The number of violated `FDK_ASSERT`s on this thread since the last [`reset_violations`].
pub fn violations() -> u64 {
    VIOLATIONS.with(|v| v.get())
}

/// Adds `n` to the counter of [`violations`] (the carry of the thread pool, [`crate::par`]).
pub fn add_violations(n: u64) {
    VIOLATIONS.with(|v| v.set(v.get().wrapping_add(n)));
}

/// Resets the counter of [`violations`].
pub fn reset_violations() {
    VIOLATIONS.with(|v| v.set(0));
}

fn strict() -> bool {
    static STRICT: OnceLock<bool> = OnceLock::new();
    *STRICT.get_or_init(|| std::env::var_os("FDK_AAC_RS_STRICT_ASSERT").is_some_and(|v| v == "1"))
}

/// Called by [`fdk_assert!`](macro@crate::fdk_assert) for a violated condition.
#[cold]
#[inline(never)]
pub fn violated(file: &'static str, line: u32, cond: &'static str) {
    VIOLATIONS.with(|v| v.set(v.get() + 1));
    if strict() {
        panic!("FDK_ASSERT({cond}) violated at {file}:{line}");
    }
}

/// `FDK_ASSERT(cond)`: see the [module documentation](mod@crate::fdk_assert). An optional message is
/// accepted for compatibility with `debug_assert!` and ignored.
#[macro_export]
macro_rules! fdk_assert {
    ($cond:expr $(,)?) => {
        if cfg!(debug_assertions) && !($cond) {
            $crate::fdk_assert::violated(file!(), line!(), stringify!($cond));
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if cfg!(debug_assertions) && !($cond) {
            // The message is not printed; formatting it keeps its arguments used.
            let _ = format_args!($($arg)+);
            $crate::fdk_assert::violated(file!(), line!(), stringify!($cond));
        }
    };
}

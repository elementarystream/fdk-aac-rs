//! Differential fuzzing of fdk-aac-rs against the C++ reference fdk-aac 2.0.3
//! (PORTING.md, "Differential fuzzing"; fuzz/README.md).
//!
//! Every target decodes its input into a call program, runs the program on the reference
//! (`fdk_aac_rs_cref`, reference library plus shims, every call guarded by src/guard.c) and on
//! the port's straight-port API, records a trace of everything that must match after every call
//! (PORTING.md, "Definition of bit-exact"), and compares the traces:
//!
//! - equal traces: nothing to report;
//! - the reference call was stopped by the guard (no defined reference result): the traces are
//!   compared up to the stopped call;
//! - a difference: the reference is run again with other fill patterns; if its own trace
//!   changes, the input has no defined reference result and is skipped; otherwise the target
//!   panics with a report, and libFuzzer stores the input.
//!
//! The port runs unguarded: a panic or a hang of the port is a finding as well.

pub mod dec;
pub mod enc;
pub mod guard;

use guard::Abort;
use std::fmt::{self, Write as _};

/// Reads the fuzz input; reads past the end give 0.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn u8(&mut self) -> u8 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    pub fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    /// The next `n` bytes (fewer at the end of the input).
    pub fn bytes(&mut self, n: usize) -> &'a [u8] {
        let start = self.pos.min(self.data.len());
        let end = (start + n).min(self.data.len());
        self.pos = start + n;
        &self.data[start..end]
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let start = self.pos.min(self.data.len());
        self.pos = self.data.len();
        &self.data[start..]
    }
}

/// 64-bit hash of a byte string (not cryptographic; it only has to separate outputs).
pub fn hash(data: &[u8]) -> u64 {
    let mut h = 0x9E37_79B9_7F4A_7C15u64 ^ data.len() as u64;
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes(c.try_into().unwrap());
        h = (h ^ w).wrapping_mul(0xFF51_AFD7_ED55_8CCD).rotate_left(31);
    }
    for (i, &b) in chunks.remainder().iter().enumerate() {
        h = (h ^ ((b as u64) << (8 * i))).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^ (h >> 33)
}

/// A byte string compared by hash and length; the bytes are kept only in verbose runs (for the
/// report of a difference).
#[derive(Clone)]
pub struct Blob {
    pub hash: u64,
    pub len: usize,
    pub bytes: Option<Vec<u8>>,
}

impl Blob {
    pub fn new(data: &[u8], verbose: bool) -> Blob {
        Blob { hash: hash(data), len: data.len(), bytes: verbose.then(|| data.to_vec()) }
    }

    /// The samples as little-endian bytes (both reference architectures are little endian).
    pub fn from_i16(data: &[i16], verbose: bool) -> Blob {
        // SAFETY: i16 has no padding; the byte view covers the same memory.
        let b = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
        Blob::new(b, verbose)
    }
}

impl PartialEq for Blob {
    fn eq(&self, o: &Blob) -> bool {
        self.hash == o.hash && self.len == o.len
    }
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bytes, hash {:016x}>", self.len, self.hash)
    }
}

/// With `FDK_FUZZ_ECHO=1`, every call is printed as it is recorded (`fuzz/replay.sh`: shows the
/// calls up to a port panic).
pub fn echo<E: fmt::Debug>(i: usize, e: &E) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("FDK_FUZZ_ECHO").is_some_and(|v| v == "1")) {
        eprintln!("    #{i} {e:?}");
    }
}

/// The calls of one side and how the run ended.
pub struct Trace<E> {
    pub events: Vec<E>,
    /// The guard stopped the reference call after `events.len()` events.
    pub abort: Option<Abort>,
}

impl<E> Default for Trace<E> {
    fn default() -> Self {
        Trace { events: Vec::new(), abort: None }
    }
}

/// Result of one input.
#[derive(Debug)]
pub enum Outcome {
    /// Both sides gave the same trace.
    Match { events: usize },
    /// The guard stopped a reference call; the traces matched up to that call.
    RefAbort { at: usize, why: Abort },
    /// The traces differ, but so do two runs of the reference with different fill patterns:
    /// the reference has no defined result there.
    RefNondeterministic { at: usize },
    /// A difference (the report).
    Mismatch(String),
}

/// Index of the first differing event, if any. The traces must be equal; if the reference
/// aborted, only the events before the aborted call are compared.
pub fn first_difference<E: PartialEq>(r: &Trace<E>, p: &Trace<E>) -> Option<usize> {
    let n = if r.abort.is_some() { r.events.len() } else { r.events.len().max(p.events.len()) };
    (0..n).find(|&i| r.events.get(i) != p.events.get(i))
}

/// Whether two reference traces are the same run.
pub fn same_run<E: PartialEq>(a: &Trace<E>, b: &Trace<E>) -> bool {
    a.abort == b.abort && a.events == b.events
}

/// Fill patterns of the repeated reference runs (the first run uses 0).
pub const RECHECK_PATTERNS: [i32; 3] = [0xA5, 0x5A, 0xFF];

/// The comparison of one input, shared by the targets: `run_ref(pattern, verbose)`,
/// `run_port(verbose)`, `detail(ref, port, i)` adds target-specific detail to a report.
pub fn compare<E: PartialEq + fmt::Debug>(
    target: &str,
    describe: &str,
    mut run_ref: impl FnMut(i32, bool) -> Trace<E>,
    mut run_port: impl FnMut(bool) -> Trace<E>,
    detail: impl Fn(&E, &E) -> String,
) -> Outcome {
    let r = run_ref(0, false);
    let p = run_port(false);
    let Some(i) = first_difference(&r, &p) else {
        return match r.abort {
            Some(why) => Outcome::RefAbort { at: r.events.len(), why },
            None => Outcome::Match { events: r.events.len() },
        };
    };
    for pat in RECHECK_PATTERNS {
        if !same_run(&r, &run_ref(pat, false)) {
            return Outcome::RefNondeterministic { at: i };
        }
    }
    // Verbose runs for the report (they keep the output bytes).
    let rv = run_ref(0, true);
    let pv = run_port(true);
    let i = first_difference(&rv, &pv).unwrap_or(i);
    let mut s = String::new();
    writeln!(s, "fdk-aac-rs differential fuzzing: DIFFERENCE in {target}").unwrap();
    writeln!(s, "input: {describe}").unwrap();
    writeln!(s, "first differing call: #{i}").unwrap();
    let ev = |t: &Trace<E>, k: usize| match t.events.get(k) {
        Some(e) => format!("{e:?}"),
        None => match t.abort {
            Some(a) if k == t.events.len() => format!("(reference call aborted: {a})"),
            _ => "(no call: the run ended)".to_string(),
        },
    };
    writeln!(s, "  reference: {}", ev(&rv, i)).unwrap();
    writeln!(s, "  port:      {}", ev(&pv, i)).unwrap();
    if let (Some(a), Some(b)) = (rv.events.get(i), pv.events.get(i)) {
        let d = detail(a, b);
        if !d.is_empty() {
            writeln!(s, "  {d}").unwrap();
        }
    }
    let from = i.saturating_sub(8);
    if from < i {
        writeln!(s, "preceding calls (equal on both sides):").unwrap();
        for k in from..i {
            writeln!(s, "  #{k} {}", ev(&rv, k)).unwrap();
        }
    }
    writeln!(
        s,
        "calls: reference {} ({}), port {}",
        rv.events.len(),
        rv.abort.map_or("completed".to_string(), |a| format!("aborted: {a}")),
        pv.events.len()
    )
    .unwrap();
    Outcome::Mismatch(s)
}

/// The first differing position of two byte strings of a verbose run, as text.
pub fn diff_bytes(what: &str, a: &Blob, b: &Blob, elem: usize) -> String {
    let (Some(x), Some(y)) = (&a.bytes, &b.bytes) else { return String::new() };
    let n = x.len().min(y.len()) / elem;
    let get = |v: &[u8], k: usize| -> i64 {
        match elem {
            2 => i16::from_le_bytes([v[2 * k], v[2 * k + 1]]) as i64,
            _ => v[k] as i64,
        }
    };
    let diffs: Vec<usize> = (0..n).filter(|&k| get(x, k) != get(y, k)).collect();
    match diffs.first() {
        Some(&k) => format!(
            "{what}: {} of {n} elements differ; first at {k}: reference {} port {}",
            diffs.len(),
            get(x, k),
            get(y, k)
        ),
        None if x.len() != y.len() => format!("{what}: lengths {} and {}", x.len(), y.len()),
        None => String::new(),
    }
}

/// Compares `aacDecoder_GetLibInfo` and `aacEncGetLibInfo` once per process (PORTING.md,
/// "Definition of bit-exact"; the build date and time are not compared).
pub fn lib_info_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Err(e) = dec::check_lib_info().and_then(|_| enc::check_lib_info()) {
            panic!("fdk-aac-rs differential fuzzing: DIFFERENCE in GetLibInfo: {e}");
        }
    });
}

/// What the fuzz targets do with an outcome: report a difference by panicking (libFuzzer stores
/// the input); count the skipped inputs (printed every 2^k skips).
pub fn handle(target: &str, o: Outcome) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static ABORTS: AtomicU64 = AtomicU64::new(0);
    static NONDET: AtomicU64 = AtomicU64::new(0);
    match o {
        Outcome::Match { .. } => {}
        Outcome::RefAbort { why, .. } => {
            let n = ABORTS.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                eprintln!("[{target}] {n} inputs with an aborted reference call (latest: {why})");
            }
        }
        Outcome::RefNondeterministic { .. } => {
            let n = NONDET.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                eprintln!("[{target}] {n} inputs skipped: no defined reference result");
            }
        }
        Outcome::Mismatch(report) => panic!("{report}"),
    }
}

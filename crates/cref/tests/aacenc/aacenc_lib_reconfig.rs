// Differential tests of fdk_aac_rs_aacenc::aacenc_lib: reconfiguration between the filter banks
// (AAC-LC/HE-AAC 1024, AAC-LD 480/512, AAC-ELD 480/512), with and without AACENC_CONTROL_STATE
// values that drop AACENC_INIT_STATES or AACENC_INIT_TRANSPORT, 1 to 8 channels.
//
// The MDCT state, the block switching state and the short-block psychoacoustic configuration
// survive such switches. Some of the following encode calls have no defined result in the
// reference; the port counts them (`counters`, `deviation_events` of aacenc_lib.rs), and the
// comparison of a case stops at such a call (see `reconfiguration_matrix`). Calls with counted
// reads are compared (see `run_range`).

use crate::aacenc_lib::{signal, EncCall, Pair};
use crate::adj_thr_enc::Signal;
use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc_lib::*;
use std::io::{BufRead, Write};

/// One encoder configuration: (AOT, SBR mode, granule length).
type Cfg = (u32, Option<u32>, u32);

const CFGS: [Cfg; 9] = [
    (2, None, 1024),
    (5, None, 1024),
    (29, None, 1024),
    (23, None, 480),
    (23, None, 512),
    (39, Some(0), 480),
    (39, Some(0), 512),
    (39, Some(1), 480),
    (39, Some(1), 512),
];

#[derive(Clone, Copy, Debug)]
struct Case {
    a: Cfg,
    b: Cfg,
    /// AACENC_CONTROL_STATE after the switch
    cs: Option<u32>,
    /// channel mode, input channels, maxChannels of aacEncOpen
    mode: u32,
    ch: usize,
    max_ch: u32,
    /// encode calls before the switch, and the signal
    n1: usize,
    sig: Signal,
    rate: i32,
    /// AACENC_TRANSMUX
    tt: u32,
}

/// Every switch between the configurations of CFGS, with AACENC_CONTROL_STATE values with and
/// without AACENC_INIT_STATES/AACENC_INIT_TRANSPORT, on ADTS (which fails for LD/ELD when the
/// transport is initialized) and LOAS; then the LFE channel (which no initialization resets) at
/// several sample rates.
fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    for tt in [2u32, 10] {
        for &a in &CFGS {
            for &b in &CFGS {
                for cs in [None, Some(1u32), Some(3), Some(0x1001), Some(0x2001), Some(0x3001)] {
                    // (a switch that reaches a counter of `deviation_events` is compared up to
                    // that call)
                    for (mode, ch, max_ch) in [(1u32, 1usize, 1u32), (2, 2, 2), (2, 2, 8), (3, 3, 3), (6, 6, 6)] {
                        if (a.0 == 29 || b.0 == 29) && ch != 2 {
                            continue;
                        }
                        for (n1, sig) in [(3usize, Signal::Music(0)), (4, Signal::Clicks), (5, Signal::Clicks)] {
                            v.push(Case { a, b, cs, mode, ch, max_ch, n1, sig, rate: 48000, tt });
                        }
                    }
                }
            }
        }
    }
    for a in [CFGS[0], CFGS[1]] {
        for b in [CFGS[3], CFGS[4], CFGS[5], CFGS[6]] {
            for cs in [None, Some(3u32)] {
                for (mode, ch) in [(6u32, 6usize), (7, 8)] {
                    for rate in [22050, 24000, 32000, 44100, 48000] {
                        for (n1, sig) in [(3usize, Signal::Music(0)), (4, Signal::Music(1))] {
                            v.push(Case { a, b, cs, mode, ch, max_ch: ch as u32, n1, sig, rate, tt: 10 });
                        }
                    }
                }
            }
        }
    }
    v
}

/// The calls of a case: (parameter changes before the encode calls, encode calls).
fn steps(c: &Case) -> [(Vec<(AacencParam, u32)>, usize); 2] {
    let mut p1 = vec![(AACENC_AOT, c.a.0)];
    if let Some(s) = c.a.1 {
        p1.push((AACENC_SBR_MODE, s));
    }
    p1.extend([
        (AACENC_GRANULE_LENGTH, c.a.2),
        (AACENC_SAMPLERATE, c.rate as u32),
        (AACENC_CHANNELMODE, c.mode),
        (AACENC_BITRATE, 64000 * c.ch as u32),
        (AACENC_TRANSMUX, c.tt),
    ]);
    let mut p2 = vec![(AACENC_AOT, c.b.0)];
    if let Some(s) = c.b.1 {
        p2.push((AACENC_SBR_MODE, s));
    }
    p2.push((AACENC_GRANULE_LENGTH, c.b.2));
    if let Some(v) = c.cs {
        p2.push((AACENC_CONTROL_STATE, v));
    }
    [(p1, c.n1), (p2, 6)]
}

/// `FDK_RECONFIG_FULL=1`: compare also the encode calls that the port counts in mdct_block,
/// which have no defined result in the reference; without it, such a call ends the comparison
/// of a case and the case is run on the port alone.
fn full() -> bool {
    std::env::var_os("FDK_RECONFIG_FULL").is_some()
}

/// `Pair::ref_reads_hook` of the child: tells the parent (on stdout, flushed) that the port
/// counted reads for the next reference call (`@@ reads N`), and that the call returned
/// (`@@ reads end`), so that the parent knows whether the child ended inside such a call.
fn announce_reads(reads: Option<u64>) {
    let mut out = std::io::stdout().lock();
    match reads {
        Some(n) => writeln!(out, "@@ reads {n}").unwrap(),
        None => writeln!(out, "@@ reads end").unwrap(),
    }
    out.flush().unwrap();
}

/// Runs a case on both encoders (Pair compares every call). Returns `Pair::guard_hits`, and
/// whether the comparison stopped at a counted call.
fn run_pair(c: &Case) -> (usize, bool) {
    let what = format!("{c:?}");
    let Some(mut p) = Pair::open(0, c.max_ch, &what) else { return (0, false) };
    if full() {
        p.guard = 128;
    } else {
        p.stop_undefined = true;
        p.ref_reads_hook = Some(announce_reads);
    }
    let pcm = signal(c.sig, c.rate, c.ch, 1024 * 24);
    let mut pos = 0usize;
    for (k, (params, frames)) in steps(c).iter().enumerate() {
        for &(a, v) in params {
            p.set(a, v);
        }
        if k == 0 && (!p.ch_set || p.init() != AACENC_OK) {
            return (0, false);
        }
        for _ in 0..*frames {
            let fl = p.info("fl").frame_length.max(1) as usize;
            let n = (fl * c.ch).min(pcm.len() - pos);
            let (e, o) = p.encode(&EncCall::pcm(pcm[pos..pos + n].to_vec(), 20480));
            if p.stopped_undefined {
                return (p.guard_hits, true);
            }
            pos = (pos + o.num_in_samples.max(0) as usize).min(pcm.len());
            if e != AACENC_OK {
                return (p.guard_hits, false);
            }
        }
    }
    (p.guard_hits, false)
}

/// Runs a case on the port alone (for the calls that the reference does not define).
fn run_port(c: &Case) {
    let mut h: Option<Box<AacEncoder>> = None;
    if aac_enc_open(Some(&mut h), 0, c.max_ch) != AACENC_OK {
        return;
    }
    let pcm = signal(c.sig, c.rate, c.ch, 1024 * 24);
    let mut pos = 0usize;
    let mut out = vec![0u8; 20480];
    for (k, (params, frames)) in steps(c).iter().enumerate() {
        for &(a, v) in params {
            aac_encoder_set_param(h.as_deref_mut(), a, v);
        }
        // SAFETY: no buffers.
        if k == 0 && unsafe { aac_enc_encode(h.as_deref_mut(), None, None, None, None) } != AACENC_OK {
            return;
        }
        for _ in 0..*frames {
            let mut info = AacencInfoStruct::default();
            aac_enc_info(h.as_deref_mut(), Some(&mut info));
            let n = (info.frame_length.max(1) as usize * c.ch).min(pcm.len() - pos);
            let mut buf = pcm[pos..pos + n].to_vec();
            let mut in_ptr = [buf.as_mut_ptr() as *mut c_void];
            let mut in_id = [IN_AUDIO_DATA];
            let mut in_size = [n as i32 * 2];
            let mut in_el = [2i32];
            let in_desc = AacencBufDesc {
                num_bufs: 1,
                bufs: in_ptr.as_mut_ptr(),
                buffer_identifiers: in_id.as_mut_ptr(),
                buf_sizes: in_size.as_mut_ptr(),
                buf_el_sizes: in_el.as_mut_ptr(),
            };
            let mut out_ptr = [out.as_mut_ptr() as *mut c_void];
            let mut out_id = [OUT_BITSTREAM_DATA];
            let mut out_size = [out.len() as i32];
            let mut out_el = [1i32];
            let out_desc = AacencBufDesc {
                num_bufs: 1,
                bufs: out_ptr.as_mut_ptr(),
                buffer_identifiers: out_id.as_mut_ptr(),
                buf_sizes: out_size.as_mut_ptr(),
                buf_el_sizes: out_el.as_mut_ptr(),
            };
            let in_args = AacencInArgs { num_in_samples: n as i32, num_anc_bytes: 0 };
            let mut out_args = AacencOutArgs::default();
            // SAFETY: the descriptors point to live buffers of the announced sizes.
            let e = unsafe {
                aac_enc_encode(h.as_deref_mut(), Some(&in_desc), Some(&out_desc), Some(&in_args), Some(&mut out_args))
            };
            pos = (pos + out_args.num_in_samples.max(0) as usize).min(pcm.len());
            if e != AACENC_OK {
                aac_enc_close(Some(&mut h));
                return;
            }
        }
    }
    aac_enc_close(Some(&mut h));
}

fn panic_msg(e: Box<dyn std::any::Any + Send>) -> String {
    let m = e
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    m.lines().next().unwrap_or("").to_string()
}

/// The port's counters for these switches (`mdct_block_dropped_accesses` and two psy_main
/// counters).
fn counters() -> [u64; 5] {
    let (w, r, n) = fdk_aac_rs_fdk::mdct::mdct_block_dropped_accesses();
    [
        w,
        r,
        n,
        fdk_aac_rs_aacenc::psy_main::psy_main_negative_lowpass_clears(),
        fdk_aac_rs_aacenc::psy_main::psy_main_stale_short_blocks(),
    ]
}


/// The child: runs the cases `from..to` (`FDK_RECONFIG_FROM`, `FDK_RECONFIG_TO`) and reports
/// each on stdout.
fn child(from: usize, to: usize) {
    let out = std::io::stdout();
    for (k, c) in cases().iter().enumerate().take(to).skip(from) {
        writeln!(out.lock(), "@@ {k} start").unwrap();
        out.lock().flush().unwrap();
        let before = counters();
        let r = std::panic::catch_unwind(|| run_pair(c));
        let after = counters();
        let d: Vec<String> = (0..5).map(|i| (after[i] - before[i]).to_string()).collect();
        let line = match r {
            Ok((hits, stopped)) => {
                // the rest of a stopped case on the port alone
                match std::panic::catch_unwind(|| if stopped { run_port(c) }) {
                    Ok(()) => format!("@@ {k} ok {hits} {} {}", d.join(" "), stopped as u8),
                    Err(e) => format!("@@ {k} fail port alone: {}", panic_msg(e)),
                }
            }
            Err(e) => format!("@@ {k} fail {}", panic_msg(e)),
        };
        writeln!(out.lock(), "{line}").unwrap();
        out.lock().flush().unwrap();
    }
    writeln!(out.lock(), "@@ done").unwrap();
}

/// The cases per child process (a chunk of the matrix).
const CHUNK: usize = 100;

/// The number of chunks run at once: RUST_TEST_THREADS (libtest's own setting) if set,
/// otherwise the available parallelism.
fn worker_threads() -> usize {
    std::env::var("RUST_TEST_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// The results of a set of cases.
#[derive(Default)]
struct Tally {
    ok: usize,
    with_stop: usize,
    /// `Pair::guard_hits` (full), the five `counters`, stopped cases
    sum: [u64; 7],
    /// (case, message)
    fails: Vec<(usize, String)>,
    /// (case, exit status of the child, panic of the port alone)
    crashes: Vec<(usize, String, Option<String>)>,
    /// compared reference encode calls with counted reads
    read_calls: u64,
    /// cases whose child ended inside such a call, and which were therefore run again alone, in
    /// a new child
    reruns: Vec<usize>,
    /// (case, exit status, reads of the call): the child ended inside such a call again when
    /// the case ran alone; compared up to that call, the rest on the port alone
    stop_exits: Vec<(usize, String, u64)>,
    /// (case, line) for FDK_RECONFIG_VERBOSE
    verbose: Vec<(usize, String)>,
}

impl Tally {
    fn merge(&mut self, t: Tally) {
        self.ok += t.ok;
        self.with_stop += t.with_stop;
        for i in 0..7 {
            self.sum[i] += t.sum[i];
        }
        self.fails.extend(t.fails);
        self.crashes.extend(t.crashes);
        self.read_calls += t.read_calls;
        self.reruns.extend(t.reruns);
        self.stop_exits.extend(t.stop_exits);
        self.verbose.extend(t.verbose);
    }
}

/// Whether a child ended with one of the signals that `run_range` tolerates.
fn tolerated_exit(st: &std::process::ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        const SIGBUS: i32 = if cfg!(any(target_os = "linux", target_os = "android")) { 7 } else { 10 };
        matches!(st.signal(), Some(11) | Some(SIGBUS))
    }
    #[cfg(not(unix))]
    {
        let _ = st;
        false
    }
}

/// Runs the cases `from..to` in child processes (single-threaded), restarted after an abnormal
/// exit.
///
/// In some encode calls the port counts reads that the reference does not define. The child
/// announces such calls (`announce_reads`). An abnormal exit of the child inside one of them is
/// not a difference: the case is run again alone, in a new child, where it is normally compared
/// completely; if the child ends in such a call there as well, the case is compared up to that
/// call (like the stops of `deviation_events`) and the rest runs on the port alone. Every other
/// abnormal exit fails the test.
fn run_range(all: &[Case], mut from: usize, to: usize, t: &mut Tally) {
    // One malloc arena (glibc), independent of the chunking.
    let tunables = match std::env::var("GLIBC_TUNABLES") {
        Ok(v) if !v.is_empty() => format!("{v}:glibc.malloc.arena_max=1"),
        _ => "glibc.malloc.arena_max=1".to_string(),
    };
    // the case to run alone (see above)
    let mut alone: Option<usize> = None;
    while from < to {
        let end = if alone == Some(from) { from + 1 } else { to };
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "aacenc_lib_reconfig::reconfiguration_matrix", "--test-threads=1", "--nocapture"])
            .env("FDK_RECONFIG_FROM", from.to_string())
            .env("FDK_RECONFIG_TO", end.to_string())
            .env("GLIBC_TUNABLES", &tunables)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut last = None;
        let mut done = false;
        // counted reads of the reference call in progress (0: none in progress)
        let mut reads = 0u64;
        // completed reference calls of the current case with such reads
        let mut read_calls = 0u64;
        for line in std::io::BufReader::new(child.stdout.take().unwrap()).lines() {
            let line = line.unwrap();
            // the first line follows libtest's "test ... " on the same line
            let Some(at) = line.find("@@ ") else { continue };
            let f: Vec<&str> = line[at..].splitn(4, ' ').collect();
            if f[1] == "done" {
                done = true;
                break;
            }
            if f[1] == "reads" {
                if f[2] == "end" {
                    reads = 0;
                    read_calls += 1;
                } else {
                    reads = f[2].parse().unwrap();
                }
                continue;
            }
            let k: usize = f[1].parse().unwrap();
            match f[2] {
                "start" => {
                    last = Some(k);
                    reads = 0;
                    read_calls = 0;
                }
                "ok" => {
                    t.ok += 1;
                    t.read_calls += read_calls;
                    let v: Vec<u64> = f[3].split(' ').map(|x| x.parse().unwrap()).collect();
                    for i in 0..7 {
                        t.sum[i] += v[i];
                    }
                    if v[..6].iter().any(|&x| x > 0) {
                        t.with_stop += 1;
                        if std::env::var_os("FDK_RECONFIG_VERBOSE").is_some() {
                            t.verbose.push((
                                k,
                                format!(
                                    "COUNTED #{k} {:?}: guard bytes changed in {} calls; port: {} writes, {} reads dropped, {} missing slopes, {} negative clears, {} stale short blocks{}",
                                    all[k],
                                    v[0],
                                    v[1],
                                    v[2],
                                    v[3],
                                    v[4],
                                    v[5],
                                    if v[6] != 0 { "; comparison stopped there" } else { "" }
                                ),
                            ));
                        }
                    }
                    last = None;
                }
                _ => {
                    t.read_calls += read_calls;
                    t.fails.push((k, f.get(3).unwrap_or(&"").to_string()));
                    last = None;
                }
            }
        }
        let st = child.wait().unwrap();
        if done {
            from = end;
            alone = None;
            continue;
        }
        let k = last.unwrap_or_else(|| panic!("child exited ({st}) outside a case"));
        if reads > 0 && tolerated_exit(&st) {
            if alone != Some(k) {
                // the child ended inside such a call: run the case again alone
                t.reruns.push(k);
                alone = Some(k);
                from = k;
                continue;
            }
            // again: compared up to this call; the rest on the port alone
            t.read_calls += read_calls;
            match std::panic::catch_unwind(|| run_port(&all[k])) {
                Ok(()) => t.stop_exits.push((k, st.to_string(), reads)),
                Err(e) => t.fails.push((k, format!("port alone after an abnormal exit: {}", panic_msg(e)))),
            }
        } else {
            // the child ended abnormally in case k: run the port alone
            let r = std::panic::catch_unwind(|| run_port(&all[k]));
            t.crashes.push((k, st.to_string(), r.err().map(panic_msg)));
        }
        alone = None;
        from = k + 1;
    }
}

/// The reconfiguration matrix (`cases`), compared call by call (Pair). By default a case is
/// compared up to the encode call in which a counter of `deviation_events` (aacenc_lib.rs)
/// moves, and then run on the port alone, which must not panic. The cases run in child
/// processes (single-threaded), `CHUNK` cases each, restarted after an abnormal exit; an
/// abnormal exit in a call with counted reads is tolerated as described at
/// `run_range`. `worker_threads` chunks run at once, and the results of all chunks are reported
/// together, in the case order. `FDK_RECONFIG_FULL=1` compares through these calls (see `full`);
/// `FDK_RECONFIG_VERBOSE=1` lists the cases with counted accesses.
#[test]
fn reconfiguration_matrix() {
    if let Some(from) = std::env::var_os("FDK_RECONFIG_FROM") {
        let to = std::env::var("FDK_RECONFIG_TO").map_or(usize::MAX, |v| v.parse().unwrap());
        child(from.to_str().unwrap().parse().unwrap(), to);
        return;
    }
    let mut l = [0i32; 8];
    // SAFETY: the shim writes 6 values.
    unsafe { fdk_aac_rs_cref::aacenc_aacenc_lib_reconfig::cref_aacenc_lib_reconfig_layout(l.as_mut_ptr()) };
    eprintln!(
        "QC_OUT_CHANNEL: size {}, stride {}, mdctSpectrum at {}, sfbSpreadEnergy at {}, MAX_GROUPED_SFB {}, P_BUF_0 {}",
        l[0], l[1], l[2], l[3], l[4], l[5]
    );
    let all = cases();
    let chunks = all.len().div_ceil(CHUNK);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let total = std::sync::Mutex::new(Tally::default());
    std::thread::scope(|scope| {
        for _ in 0..worker_threads().min(chunks) {
            scope.spawn(|| loop {
                let c = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if c >= chunks {
                    break;
                }
                let mut t = Tally::default();
                run_range(&all, c * CHUNK, ((c + 1) * CHUNK).min(all.len()), &mut t);
                total.lock().unwrap().merge(t);
            });
        }
    });
    let mut t = total.into_inner().unwrap();
    t.verbose.sort_by_key(|v| v.0);
    t.fails.sort_by_key(|f| f.0);
    t.crashes.sort_by_key(|c| c.0);
    for (_, line) in &t.verbose {
        eprintln!("{line}");
    }
    for (k, m) in &t.fails {
        eprintln!("DIFF #{k} {:?}: {m}", all[*k]);
    }
    for (k, st, port) in &t.crashes {
        eprintln!("CRASH #{k} {:?}: {st}; port alone: {}", all[*k], port.as_deref().unwrap_or("no panic"));
    }
    t.reruns.sort();
    t.stop_exits.sort_by_key(|c| c.0);
    for k in &t.reruns {
        let again = t.stop_exits.iter().find(|c| c.0 == *k);
        eprintln!(
            "RERUN #{k} {:?}: the child ended in a call with counted reads; run alone: {}",
            all[*k],
            match again {
                Some((_, st, reads)) => format!("ended there again ({st}, {reads} reads), compared up to that call"),
                None => "compared".to_string(),
            }
        );
    }
    eprintln!(
        "counted reads: {} compared reference calls; {} abnormal exits in such calls, {} again when run alone",
        t.read_calls,
        t.reruns.len(),
        t.stop_exits.len()
    );
    let sum = t.sum;
    eprintln!(
        "{} cases: {} run, {} stopped at an abnormal exit in a call with counted reads, {} differences, {} child crashes; {} cases with counted accesses (guard bytes changed in {} calls; port: {} writes and {} reads dropped, {} missing slopes, {} negative clears, {} stale short blocks; {} comparisons stopped there)",
        all.len(),
        t.ok,
        t.stop_exits.len(),
        t.fails.len(),
        t.crashes.len(),
        t.with_stop,
        sum[0],
        sum[1],
        sum[2],
        sum[3],
        sum[4],
        sum[5],
        sum[6]
    );
    if full() {
        assert!(t.crashes.iter().all(|c| c.2.is_none()), "the port panics");
    } else {
        assert!(t.fails.is_empty() && t.crashes.is_empty());
        assert!(sum[6] > 0 && sum[2] > 0 && sum[5] > 0);
    }
}

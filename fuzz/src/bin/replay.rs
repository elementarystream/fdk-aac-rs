//! Replays fuzz inputs outside libFuzzer and reports what each one does.
//!
//! ```text
//! replay [--trace] [--hang-check] TARGET FILE|DIR...
//! ```
//!
//! TARGET is a target name (`decode_adts`, ..., `encode`), or `auto` to take it from the
//! directory name (`fuzz/artifacts/<target>/...`, `fuzz/corpus/<target>/...`). `--trace` also
//! prints both call traces. `--hang-check` (implied for `timeout-*` files) first runs each side
//! alone in a child process with a wall-clock limit (`FDK_FUZZ_HANG_SECONDS`, default 120) and
//! reports which side does not finish. Exit code 1 if any input shows a difference, a port panic or a port
//! hang.

use fdk_aac_rs_fuzz::dec::{self, DecProgram, DecTarget};
use fdk_aac_rs_fuzz::enc::{self, EncProgram};
use fdk_aac_rs_fuzz::Outcome;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
enum Target {
    Dec(DecTarget),
    Enc,
}

fn target(name: &str) -> Option<Target> {
    if name == "encode" {
        return Some(Target::Enc);
    }
    DecTarget::from_name(name).map(Target::Dec)
}

fn infer(path: &Path) -> Option<Target> {
    path.ancestors().filter_map(|a| a.file_name()).find_map(|n| target(&n.to_string_lossy()))
}

fn print_trace<E: std::fmt::Debug>(side: &str, t: &fdk_aac_rs_fuzz::Trace<E>) {
    println!(
        "  {side} trace ({} calls{}):",
        t.events.len(),
        t.abort.map_or(String::new(), |a| format!(", then stopped: {a}"))
    );
    for (i, e) in t.events.iter().enumerate() {
        println!("    #{i} {e:?}");
    }
}

/// Runs one side alone (`--side ref|port TARGET FILE`, used by the hang check).
fn run_side(side: &str, t: Target, data: &[u8]) {
    match (t, side) {
        (Target::Dec(d), "ref") => drop(dec::run_ref(&DecProgram::parse(d, data), 0, false)),
        (Target::Dec(d), _) => drop(dec::run_port(&DecProgram::parse(d, data), false)),
        // without the port's stop: the port may be the side that hangs
        (Target::Enc, "ref") => drop(enc::run_ref(&EncProgram::parse(data), 0, false, None)),
        (Target::Enc, _) => drop(enc::run_port(&EncProgram::parse(data), false, None)),
    }
}

/// Whether `side` finishes within the limit, run in a child process.
fn finishes(side: &str, t: &str, f: &Path) -> Option<bool> {
    let limit: u64 =
        std::env::var("FDK_FUZZ_HANG_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let mut child = std::process::Command::new(std::env::current_exe().ok()?)
        .args(["--side", side, t])
        .arg(f)
        .env("FDK_FUZZ_REF_BUDGET_MS", "0")
        .stdout(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(st)) = child.try_wait() {
            return Some(st.success());
        }
        if start.elapsed().as_secs() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Some(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--side") && args.len() == 4 {
        let t = target(&args[2]).expect("target");
        let data = std::fs::read(&args[3]).expect("input");
        run_side(&args[1], t, &data);
        return;
    }
    let mut trace = false;
    let mut hang_check = false;
    while let Some(a) = args.first() {
        match a.as_str() {
            "--trace" => trace = true,
            "--hang-check" => hang_check = true,
            _ => break,
        }
        args.remove(0);
    }
    if args.len() < 2 {
        eprintln!("usage: replay [--trace] [--hang-check] TARGET|auto FILE|DIR...");
        std::process::exit(2);
    }
    let fixed = if args[0] == "auto" {
        None
    } else {
        Some(target(&args[0]).unwrap_or_else(|| {
            eprintln!("unknown target {}", args[0]);
            std::process::exit(2)
        }))
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for a in &args[1..] {
        let p = PathBuf::from(a);
        if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&p)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_none_or(|e| e != "log"))
                .collect();
            v.sort();
            files.extend(v);
        } else {
            files.push(p);
        }
    }
    // Port panics are reported per file; keep the default hook's message.
    let mut bad = 0;
    let (mut matched, mut aborted, mut nondet) = (0, 0, 0);
    for f in &files {
        let Some(t) = fixed.or_else(|| infer(f)) else {
            eprintln!("{}: cannot tell the target; give it explicitly", f.display());
            std::process::exit(2);
        };
        let data = std::fs::read(f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let is_timeout = f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("timeout-"));
        if hang_check || is_timeout {
            let name = match t {
                Target::Dec(d) => d.name(),
                Target::Enc => "encode",
            };
            let r = finishes("ref", name, f);
            let p = finishes("port", name, f);
            match (r, p) {
                (Some(true), Some(true)) => {}
                (Some(false), Some(true)) => {
                    aborted += 1;
                    println!(
                        "{}: no defined reference result without its budget, the port finishes; not compared",
                        f.display()
                    );
                    continue;
                }
                (_, Some(false)) => {
                    bad += 1;
                    println!(
                        "{}: PORT HANG or crash (the reference {})",
                        f.display(),
                        if r == Some(true) { "finishes" } else { "has no result either" }
                    );
                    continue;
                }
                _ => println!("{}: the hang check could not run", f.display()),
            }
        }
        let r = std::panic::catch_unwind(|| match t {
            Target::Dec(d) => {
                if trace {
                    let p = DecProgram::parse(d, &data);
                    print_trace("reference", &dec::run_ref(&p, 0, false));
                    print_trace("port", &dec::run_port(&p, false));
                }
                dec::check(d, &data)
            }
            Target::Enc => {
                if trace {
                    let p = EncProgram::parse(&data);
                    let stop = enc::deviation_stop(&p);
                    print_trace("reference", &enc::run_ref(&p, 0, false, stop));
                    print_trace("port", &enc::run_port(&p, false, stop));
                }
                enc::check(&data)
            }
        });
        match r {
            Ok(Outcome::Match { events }) => {
                matched += 1;
                println!("{}: match ({events} calls)", f.display());
            }
            Ok(Outcome::RefAbort { at, why }) => {
                aborted += 1;
                println!("{}: reference stopped at call #{at} ({why}); equal before it (skipped: no defined reference result)", f.display());
            }
            Ok(Outcome::RefNondeterministic { at }) => {
                nondet += 1;
                println!("{}: differs at call #{at}, but the reference has no defined result there (skipped)", f.display());
            }
            Ok(Outcome::Mismatch(report)) => {
                bad += 1;
                println!("{}: DIFFERENCE\n{report}", f.display());
            }
            Err(_) => {
                bad += 1;
                println!("{}: PORT PANIC (message above)", f.display());
            }
        }
    }
    println!(
        "{} inputs: {matched} match, {aborted} reference stopped, {nondet} reference nondeterministic, {bad} differences or panics",
        files.len()
    );
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        if let Some(l) = st.lines().find(|l| l.starts_with("VmHWM:")) {
            println!("peak resident memory: {}", l[6..].trim());
        }
    }
    if fdk_aac_rs_fuzz::guard::ref_arena() {
        println!(
            "reference arena: at most {} KiB in one run",
            fdk_aac_rs_fuzz::guard::ref_arena_max() >> 10
        );
    }
    std::process::exit(if bad > 0 { 1 } else { 0 });
}

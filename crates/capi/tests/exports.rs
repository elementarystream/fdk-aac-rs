// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The symbols of the shared and the static library (PORTING.md, "C ABI"):
//! the shared library exports exactly the 18 symbols of `fdk-aac.sym`; the static library
//! defines them too, and `aacDecoder_RawISOBMFFData`, which upstream's static library has and
//! its shared library hides. Cargo builds the library's `cdylib` and `staticlib` together with
//! the `rlib` the tests link, in the `deps/` directory of the test binary.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Declared in `aacdecoder_lib.h`, defined in the static library only.
const STATIC_ONLY: &str = "aacDecoder_RawISOBMFFData";

fn expected() -> BTreeSet<String> {
    let sym = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fdk-aac.sym"))
        .expect("fdk-aac.sym");
    let all: BTreeSet<String> =
        sym.lines().map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    assert_eq!(all.len(), 18, "fdk-aac.sym has 18 symbols");
    assert!(!all.contains(STATIC_ONLY));
    all
}

/// The library in the `deps/` directory of the test binary, where Cargo builds it for the tests
/// (`cargo build` also copies it to the parent directory).
fn artifact(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let deps = exe.parent().expect("deps directory");
    deps.join(name)
}

/// Defined global symbols of a binary, without the Mach-O underscore.
fn defined_symbols(path: &Path, dynamic: bool) -> BTreeSet<String> {
    let mut cmd = Command::new("nm");
    if cfg!(target_vendor = "apple") {
        cmd.arg("-gU");
    } else if dynamic {
        cmd.args(["-D", "--defined-only"]);
    } else {
        cmd.args(["-g", "--defined-only"]);
    }
    let out = cmd.arg(path).output().expect("run nm");
    assert!(out.status.success(), "nm {}", path.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .filter(|s| !s.ends_with(':') && !s.is_empty())
        .map(|s| {
            if cfg!(target_vendor = "apple") {
                s.strip_prefix('_').unwrap_or(s).to_string()
            } else {
                s.to_string()
            }
        })
        .collect()
}

#[test]
fn shared_library_exports_exactly_fdk_aac_sym() {
    let ext = if cfg!(target_vendor = "apple") { "dylib" } else { "so" };
    let lib = artifact(&format!("libfdk_aac.{ext}"));
    let got = defined_symbols(&lib, true);
    // ELF shared objects also list the version-script-independent linker symbols.
    let got: BTreeSet<String> = got
        .into_iter()
        .filter(|s| !matches!(s.as_str(), "_init" | "_fini" | "_edata" | "_end" | "__bss_start"))
        .collect();
    // The targets without the global_asm! label export it as an ordinary symbol.
    let asm_label = cfg!(any(
        all(target_vendor = "apple", any(target_arch = "aarch64", target_arch = "x86_64")),
        all(
            any(target_os = "linux", target_os = "android"),
            any(target_arch = "aarch64", target_arch = "x86_64", target_arch = "x86")
        )
    ));
    let mut want = expected();
    if !asm_label {
        want.insert(STATIC_ONLY.to_string());
    }
    assert_eq!(got, want, "exports of {}", lib.display());
}

#[test]
fn static_library_defines_the_symbols() {
    let lib = artifact("libfdk_aac.a");
    let got = defined_symbols(&lib, false);
    for s in expected().iter().map(String::as_str).chain([STATIC_ONLY]) {
        assert!(got.contains(s), "{s} missing in {}", lib.display());
    }
}

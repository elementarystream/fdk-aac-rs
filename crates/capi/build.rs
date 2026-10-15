// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from fdk-aac.sym, Makefile.am and CMakeLists.txt (shared library exports and names).

//! Link arguments of the shared library (PORTING.md, "C ABI").
//!
//! **Exports.** The shared library must export exactly the 18 symbols of `fdk-aac.sym`. rustc
//! already restricts a cdylib's exports to its `#[no_mangle]` items, with its own linker list
//! (`-exported_symbols_list` on Apple targets, an anonymous `--version-script` on ELF targets).
//! This script adds `fdk-aac.sym` as a second list where the linker allows it:
//!
//! - **Apple (ld64):** `-exported_symbols_list` with the symbols of `fdk-aac.sym`. ld64 merges
//!   several lists (measured: a second list neither removes nor hides a symbol of rustc's
//!   list), so this list cannot hide anything; it makes the link fail if a listed symbol is
//!   missing. `-unexported_symbol` cannot be combined with an export list (ld64: "cannot be
//!   used with -exported_symbol*").
//! - **ELF:** no second list. GNU ld rejects a second anonymous version script ("anonymous
//!   version tag cannot be combined with other version tags", measured with Debian trixie's
//!   ld 2.44), and rustc's script already exports exactly the `#[no_mangle]` items.
//!
//! The set of exported symbols is therefore that of the `#[no_mangle]` items, and
//! `tests/exports.rs` checks it against `fdk-aac.sym`. `aacDecoder_RawISOBMFFData` must be in
//! the static library but not in the shared library, as upstream: an item that rustc does not
//! know as an export keeps it out of rustc's list. `src/dec.rs` therefore defines the global
//! symbol (`_aacDecoder_RawISOBMFFData` on Apple targets) with `global_asm!`, as a label that
//! branches to a Rust `extern "C"` function referenced with `sym`: the symbol is global in the
//! static library and not exported by the shared library (`tests/exports.rs` checks both).
//!
//! **Names.** The shared library gets upstream's names: install name
//! `@rpath/libfdk-aac.2.dylib` with compatibility version 3.0.0 and current version 3.3.0
//! (CMake's `MACHO_COMPATIBILITY_VERSION`/`MACHO_CURRENT_VERSION`), and soname
//! `libfdk-aac.so.2` on ELF targets. Cargo names the file `libfdk_aac.{dylib,so}`; installing
//! it as `libfdk-aac.*` is the installer's task (`scripts/capi_test.sh` does it for its tests).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=fdk-aac.sym");

    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let sym = fs::read_to_string(dir.join("fdk-aac.sym")).expect("fdk-aac.sym");
    let symbols: Vec<&str> = sym.lines().map(str::trim).filter(|s| !s.is_empty()).collect();

    let vendor = env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default();
    let family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if vendor == "apple" {
        let list = out.join("fdk-aac.exp");
        let text: String = symbols.iter().map(|s| format!("_{s}\n")).collect();
        fs::write(&list, text).expect("write fdk-aac.exp");
        println!("cargo:rustc-cdylib-link-arg=-Wl,-exported_symbols_list,{}", list.display());
        println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libfdk-aac.2.dylib");
        println!("cargo:rustc-cdylib-link-arg=-Wl,-compatibility_version,3.0.0");
        println!("cargo:rustc-cdylib-link-arg=-Wl,-current_version,3.3.0");
    } else if family == "unix" && os != "windows" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libfdk-aac.so.2");
    }
}

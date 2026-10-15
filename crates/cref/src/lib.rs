// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! Differential tests against the C++ reference (PORTING.md, "Differential tests").
//!
//! `build.rs` compiles the reference (once per variant, into the shared cache
//! `work/build-cref/<variant>/`) and the shims in `shim/`. Each shim `shim/<name>.cpp` defines
//! `extern "C"` functions named `cref_*`; their declarations are generated into the module
//! `<name>` of this crate. The tests in `tests/<crate>/` compare the port with them.
//!
//! If the C++ sources are not available, the `cref_missing` cfg is set and the bindings and the
//! tests are compiled out.

#[cfg(not(cref_missing))]
include!(concat!(env!("OUT_DIR"), "/cref_bindings.rs"));

pub mod testutil;

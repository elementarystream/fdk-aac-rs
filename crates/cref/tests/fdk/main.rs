// Differential tests of fdk-aac-rs-fdk against the C++ reference. Every other file in this
// directory is a module of this test binary (see build.rs).

#![cfg(not(cref_missing))]
// Test tables of tuples are clearer inline than behind type aliases.
#![allow(clippy::type_complexity)]

include!(concat!(env!("OUT_DIR"), "/tests_fdk.rs"));

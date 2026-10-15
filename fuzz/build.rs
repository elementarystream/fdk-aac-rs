// Compiles the signal and time guard of the reference calls (src/guard.c). The reference itself and
// its shims come from fdk-aac-rs-cref (crates/cref/build.rs, shared cache in work/build-cref).
fn main() {
    println!("cargo::rerun-if-changed=src/guard.c");
    cc::Build::new().file("src/guard.c").opt_level(2).warnings(true).compile("fzguard");
}

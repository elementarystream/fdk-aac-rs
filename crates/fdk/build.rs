// Selects the architecture variant of the reference that the port reproduces
// (PORTING.md, "Architecture variants"). The reference computes some primitives and a few whole
// functions differently depending on the architecture it is built for, so its output differs
// between architectures in the least significant bits.
//
// Default: `aarch64` on aarch64 targets, `x86` on x86-64 targets, `x86_32` on 32-bit x86 targets
// and `aarch64` on all other targets. The build-time environment variable
// `FDK_AAC_RS_ARCH=aarch64|x86|x86_32|generic` overrides the choice.
//
// The x87 expressions of the `x86_32` variant (src/x87.rs) have three implementations with the
// same results: `asm` (the x87 instructions of the reference; x86 and x86-64 targets only), `f64`
// (double precision with a proven error bound, the software model where the bound does not
// decide; not on 32-bit x86 targets without SSE2, whose `f64` arithmetic is not IEEE double
// precision) and `soft` (the software model `F80` only). Default: `f64` wherever its arithmetic is
// IEEE double precision (it was measured about 3% faster than `asm` on i686, and equal to the x87
// unit for all 2^32 operands), otherwise `asm`. The build-time environment variable
// `FDK_AAC_RS_X87=asm|f64|soft` overrides the choice.

fn main() {
    println!("cargo::rerun-if-env-changed=FDK_AAC_RS_ARCH");
    println!("cargo::rerun-if-env-changed=FDK_AAC_RS_X87");
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let arch = match std::env::var("FDK_AAC_RS_ARCH").as_deref() {
        Ok(a @ ("aarch64" | "x86" | "x86_32" | "generic")) => a.to_string(),
        Ok(other) => panic!(
            "FDK_AAC_RS_ARCH must be \"aarch64\", \"x86\", \"x86_32\" or \"generic\", not {other:?}"
        ),
        Err(_) => match target_arch.as_str() {
            "x86_64" => "x86".to_string(),
            "x86" => "x86_32".to_string(),
            _ => "aarch64".to_string(),
        },
    };
    println!("cargo::rustc-cfg=fdk_arch=\"{arch}\"");

    let has_x87 = matches!(target_arch.as_str(), "x86" | "x86_64");
    let features = std::env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    let ieee_f64 = target_arch != "x86" || features.split(',').any(|f| f == "sse2");
    let x87 = match std::env::var("FDK_AAC_RS_X87").as_deref() {
        Ok("asm") if !has_x87 => panic!("FDK_AAC_RS_X87=asm needs an x86 or x86-64 target"),
        Ok("f64") if !ieee_f64 => panic!("FDK_AAC_RS_X87=f64 needs SSE2 on 32-bit x86 targets"),
        Ok(x @ ("asm" | "f64" | "soft")) => x.to_string(),
        Ok(other) => panic!("FDK_AAC_RS_X87 must be \"asm\", \"f64\" or \"soft\", not {other:?}"),
        Err(_) if ieee_f64 => "f64".to_string(),
        Err(_) if has_x87 => "asm".to_string(),
        Err(_) => "soft".to_string(),
    };
    println!("cargo::rustc-check-cfg=cfg(fdk_x87, values(\"asm\", \"f64\", \"soft\"))");
    println!("cargo::rustc-cfg=fdk_x87=\"{x87}\"");
}

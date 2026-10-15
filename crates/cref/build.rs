// Builds the C++ reference (fdk-aac v2.0.3) and the C shims in `shim/`, so that the tests in
// `tests/` can compare the port with the original code (PORTING.md, "Differential tests").
//
// * The reference library is compiled once per variant and flag set into a shared cache,
//   `work/build-cref/<variant>/<key>/libfdkaac_ref.a`, with the flags of the CMake Release build
//   (`-O3 -DNDEBUG -fno-exceptions -fno-rtti`) plus `-include fdk_aac_rs_cref_prefix.h`, which
//   renames the `extern "C"` API functions to `cref_*` so that they cannot clash with the C ABI
//   crate. `<key>` is a hash of the compiler, the flags, the source revision and the prefix
//   header. Parallel worktrees share the cache (a lock directory serializes the first build).
// * The shims are compiled per build, with the same compiler and flags (inline functions of the
//   reference headers are compiled into the shims and must give the same code).
// * The Rust bindings of the shims are generated here: every function definition in
//   `shim/<name>.cpp` that starts a line as `RET cref_name(ARGS) {` becomes a declaration in the
//   module `fdk_aac_rs_cref::<name>`.
// * The test binaries `tests/<crate>/main.rs` include every other file of their directory as a
//   module, so adding a test file needs no change to a shared file.
//
// The work directory is `$FDK_AAC_RS_WORK_DIR`, or `work/` of the main checkout (found through
// git, so that worktrees use the main checkout's reference). The C++ sources are
// `$FDK_AAC_RS_C_DIR`, by default `<work>/fdk-aac-v2.0.3`. If they are missing, the
// `cref_missing` cfg is set and all differential tests are skipped.
//
// The variant of the reference follows the target: aarch64 targets build the AArch64 reference,
// x86-64 targets the x86 reference (on Apple silicon, `--target x86_64-apple-darwin` runs the
// tests under Rosetta 2), 32-bit x86 Linux targets (`i686-unknown-linux-gnu`) the x86_32
// reference. On an x86-64 Linux host, the x86_32 reference is compiled with `-m32`, as the CMake
// build `ref-i686` is, so that clang targets the same triple and CPU (i386-pc-linux-gnu, i686).
// `FDK_AAC_RS_CREF_JOBS` limits the parallel compiler processes (default: 5).

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// The `extern "C"` functions of the public API (`fdk-aac.sym` plus `aacDecoder_RawISOBMFFData`).
const API: &[&str] = &[
    "aacDecoder_AncDataGet",
    "aacDecoder_AncDataInit",
    "aacDecoder_Close",
    "aacDecoder_ConfigRaw",
    "aacDecoder_DecodeFrame",
    "aacDecoder_Fill",
    "aacDecoder_GetFreeBytes",
    "aacDecoder_GetLibInfo",
    "aacDecoder_GetStreamInfo",
    "aacDecoder_Open",
    "aacDecoder_RawISOBMFFData",
    "aacDecoder_SetParam",
    "aacEncClose",
    "aacEncEncode",
    "aacEncGetLibInfo",
    "aacEncInfo",
    "aacEncOpen",
    "aacEncoder_GetParam",
    "aacEncoder_SetParam",
];

/// Include directories of the reference build, in the order of upstream's CMakeLists.txt.
const LIB_INCLUDES: &[&str] = &[
    "libAACdec/include",
    "libAACenc/include",
    "libSYS/include",
    "libArithCoding/include",
    "libDRCdec/include",
    "libSACdec/include",
    "libSACenc/include",
    "libSBRdec/include",
    "libSBRenc/include",
    "libMpegTPDec/include",
    "libMpegTPEnc/include",
    "libFDK/include",
    "libPCMutils/include",
];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(cref_missing)");
    for v in [
        "FDK_AAC_RS_WORK_DIR",
        "FDK_AAC_RS_C_DIR",
        "FDK_AAC_RS_ARCH",
        "FDK_AAC_RS_CREF_JOBS",
        "CXX",
    ] {
        println!("cargo::rerun-if-env-changed={v}");
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    for d in ["shim", "tests", "include"] {
        println!("cargo::rerun-if-changed={}", manifest.join(d).display());
    }

    // Test module lists and shim bindings do not need the reference.
    gen_test_modules(&manifest, &out);
    let shims = list(&manifest.join("shim"), "cpp");
    gen_bindings(&shims, &out);

    let work = work_dir(&manifest);
    let src = env::var_os("FDK_AAC_RS_C_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("fdk-aac-v2.0.3"));
    if !src.join("libFDK/src").is_dir() {
        println!(
            "cargo::warning=C++ reference sources not found at {}; differential tests are skipped \
             (run scripts/setup_reference.sh)",
            src.display()
        );
        println!("cargo::rustc-cfg=cref_missing");
        return;
    }
    let target = env::var("TARGET").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let variant = match target_arch.as_str() {
        "aarch64" => "aarch64",
        "x86_64" => "x86",
        "x86" if target.contains("linux") => "x86_32",
        _ => {
            println!(
                "cargo::warning=no C++ reference variant for {target}; differential tests are skipped"
            );
            println!("cargo::rustc-cfg=cref_missing");
            return;
        }
    };
    if let Ok(a) = env::var("FDK_AAC_RS_ARCH") {
        if a != variant {
            panic!(
                "FDK_AAC_RS_ARCH={a} does not match the C++ reference built for {target} ({variant}); \
                 build for the matching target instead (e.g. --target x86_64-apple-darwin)"
            );
        }
    }

    let cxx = env::var("CXX").unwrap_or_else(|_| "clang++".to_string());
    let mut flags: Vec<String> = ["-O3", "-DNDEBUG"].iter().map(|s| s.to_string()).collect();
    if target.contains("apple") {
        flags.push("-arch".into());
        flags.push(if variant == "x86" { "x86_64" } else { "arm64" }.into());
    } else if variant == "x86_32" && env::var("HOST").unwrap().starts_with("x86_64-unknown-linux") {
        flags.push("-m32".into());
    } else if target != env::var("HOST").unwrap() {
        flags.push(format!("--target={target}"));
    }
    flags.extend(["-fno-exceptions", "-fno-rtti", "-w"].iter().map(|s| s.to_string()));
    // The prefix header is copied into the shared work directory, so that the flags (and the
    // cache key) are the same in every worktree.
    let prefix_src = manifest.join("include/fdk_aac_rs_cref_prefix.h");
    check_prefix_header(&prefix_src);
    let prefix_text = fs::read_to_string(&prefix_src).unwrap();
    let prefix = work
        .join("build-cref/include")
        .join(format!("{:016x}", fnv1a(prefix_text.as_bytes())))
        .join("fdk_aac_rs_cref_prefix.h");
    write_if_changed(&prefix, &prefix_text);
    flags.push("-include".into());
    flags.push(prefix.display().to_string());
    for i in LIB_INCLUDES {
        flags.push(format!("-I{}", src.join(i).display()));
    }

    // --- Reference library (shared cache) -------------------------------------------------
    let version = Command::new(&cxx)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_else(|e| panic!("cannot run {cxx}: {e}"));
    let revision = git(&src, &["rev-parse", "HEAD"]).unwrap_or_default();
    let dirty = git(&src, &["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
    if !dirty.trim().is_empty() {
        println!("cargo::warning=the C++ reference at {} has local modifications", src.display());
    }
    let mut key = String::new();
    write!(key, "{version}\n{flags:?}\n{revision}\n{dirty}\n").unwrap();
    key += &prefix_text;
    let key = format!("{:016x}", fnv1a(key.as_bytes()));
    let cache = work.join("build-cref").join(variant).join(&key);
    let lib = cache.join("libfdkaac_ref.a");
    if !lib.is_file() {
        build_reference(&cxx, &flags, &src, &cache);
    }

    // --- Shims ---------------------------------------------------------------------------
    let mut shim_flags = flags.clone();
    shim_flags.push(format!("-I{}", manifest.join("include").display()));
    shim_flags.push(format!("-I{}", src.display()));
    // Shims that compile a copy of a reference .cpp file inside a namespace leave references to
    // functions they never call. ld64 drops those with the unused code (-dead_strip); GNU ld only
    // ignores them in sections removed by --gc-sections, which needs one section per function.
    // Section placement does not change the generated code.
    shim_flags.push("-ffunction-sections".into());
    shim_flags.push("-fdata-sections".into());
    for lib in libs(&src) {
        shim_flags.push(format!("-I{}", src.join(&lib).join("src").display()));
    }
    let shim_dir = out.join("shims");
    fs::create_dir_all(&shim_dir).unwrap();
    // Recompile only the shims that changed since the last build with the same flags.
    let stamp = shim_dir.join("FLAGS");
    let mut flags_text = format!("{cxx} {}\n{key}\n", shim_flags.join(" "));
    for h in list(&manifest.join("include"), "h") {
        flags_text += &fs::read_to_string(h).unwrap();
    }
    if fs::read_to_string(&stamp).ok().as_deref() != Some(flags_text.as_str()) {
        for o in list(&shim_dir, "o") {
            let _ = fs::remove_file(o);
        }
    }
    let objs: Vec<PathBuf> = shims.iter().map(|f| shim_obj(&shim_dir, f)).collect();
    let stale: Vec<PathBuf> = shims
        .iter()
        .filter(|f| {
            let o = shim_obj(&shim_dir, f);
            let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
            match (mtime(f), mtime(&o)) {
                (Some(src), Some(obj)) => src >= obj,
                _ => true,
            }
        })
        .cloned()
        .collect();
    compile_all(&cxx, &shim_flags, &stale, &shim_dir, |f| {
        f.file_stem().unwrap().to_string_lossy().into_owned()
    });
    // Objects of deleted shims.
    for o in list(&shim_dir, "o") {
        if !objs.contains(&o) {
            let _ = fs::remove_file(o);
        }
    }
    fs::write(&stamp, flags_text).unwrap();
    archive(&objs, &out.join("libfdkaac_shims.a"));

    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-search=native={}", cache.display());
    println!("cargo::rustc-link-lib=static=fdkaac_shims");
    println!("cargo::rustc-link-lib=static=fdkaac_ref");
    if target.contains("apple") {
        println!("cargo::rustc-link-lib=c++");
    } else {
        println!("cargo::rustc-link-lib=stdc++");
        println!("cargo::rustc-link-lib=m");
    }
    println!("cargo::rustc-env=FDK_AAC_RS_CREF_LIB={}", lib.display());
}

fn shim_obj(dir: &Path, shim: &Path) -> PathBuf {
    dir.join(format!("{}.o", shim.file_stem().unwrap().to_string_lossy()))
}

/// `work/` of the main checkout.
fn work_dir(manifest: &Path) -> PathBuf {
    if let Some(w) = env::var_os("FDK_AAC_RS_WORK_DIR") {
        return PathBuf::from(w);
    }
    if let Some(common) =
        git(manifest, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
    {
        let common = PathBuf::from(common.trim());
        if let Some(repo) = common.parent() {
            if repo.join("work").is_dir() {
                return repo.join("work");
            }
        }
    }
    manifest.join("../../work")
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The prefix header must rename every API function.
fn check_prefix_header(path: &Path) {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for f in API {
        assert!(
            text.contains(&format!("#define {f} cref_{f}\n")),
            "{} does not rename {f}",
            path.display()
        );
    }
}

fn write_if_changed(path: &Path, text: &str) {
    if fs::read_to_string(path).ok().as_deref() != Some(text) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn libs(src: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(src)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.path().join("src").is_dir() && e.file_name().to_string_lossy().starts_with("lib")
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Files with the given extension in `dir`, sorted.
fn list(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == ext))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The library sources of upstream's CMake build: `lib*/src/*.cpp`. The files in `arm/`, `x86/`
/// and `mips/` are #included by the generic sources.
fn reference_sources(src: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for lib in libs(src) {
        v.extend(list(&src.join(lib).join("src"), "cpp"));
    }
    v
}

fn build_reference(cxx: &str, flags: &[String], src: &Path, cache: &Path) {
    let parent = cache.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    let lock = cache.with_extension("lock");
    let start = SystemTime::now();
    loop {
        match fs::create_dir(&lock) {
            Ok(()) => break,
            Err(_) => {
                if cache.join("libfdkaac_ref.a").is_file() {
                    return;
                }
                // A lock older than 30 minutes belongs to a build that died.
                let age = fs::metadata(&lock)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .unwrap_or_default();
                if age > Duration::from_secs(1800) {
                    let _ = fs::remove_dir_all(&lock);
                    continue;
                }
                if start.elapsed().unwrap_or_default() > Duration::from_secs(3600) {
                    panic!("timed out waiting for {}", lock.display());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    let result = std::panic::catch_unwind(|| {
        if cache.join("libfdkaac_ref.a").is_file() {
            return;
        }
        let tmp = cache.with_extension("tmp");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let sources = reference_sources(src);
        let objs = compile_all(cxx, flags, &sources, &tmp, |f| {
            // libX/src/foo.cpp -> libX_foo
            let rel = f.strip_prefix(src).unwrap();
            let lib = rel.components().next().unwrap().as_os_str().to_string_lossy().into_owned();
            format!("{lib}_{}", f.file_stem().unwrap().to_string_lossy())
        });
        archive(&objs, &tmp.join("libfdkaac_ref.a"));
        fs::write(
            tmp.join("FLAGS"),
            format!("{cxx} {}\n{} sources\n", flags.join(" "), sources.len()),
        )
        .unwrap();
        let _ = fs::remove_dir_all(cache);
        fs::rename(&tmp, cache).unwrap();
    });
    let _ = fs::remove_dir_all(&lock);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// Compiles `files` in parallel into `dir`, naming each object with `name(file)`.
fn compile_all(
    cxx: &str,
    flags: &[String],
    files: &[PathBuf],
    dir: &Path,
    name: impl Fn(&Path) -> String + Sync,
) -> Vec<PathBuf> {
    let jobs = env::var("FDK_AAC_RS_CREF_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(5usize)
        .clamp(1, 64);
    let next = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::new());
    let objs: Vec<PathBuf> = files.iter().map(|f| dir.join(format!("{}.o", name(f)))).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= files.len() {
                    break;
                }
                let o = Command::new(cxx)
                    .args(flags)
                    .arg("-c")
                    .arg(&files[i])
                    .arg("-o")
                    .arg(&objs[i])
                    .output()
                    .unwrap_or_else(|e| panic!("cannot run {cxx}: {e}"));
                if !o.status.success() {
                    errors.lock().unwrap().push(format!(
                        "{}:\n{}",
                        files[i].display(),
                        String::from_utf8_lossy(&o.stderr)
                    ));
                }
            });
        }
    });
    let errors = errors.into_inner().unwrap();
    if !errors.is_empty() {
        panic!("compilation failed:\n{}", errors.join("\n"));
    }
    objs
}

fn archive(objs: &[PathBuf], lib: &Path) {
    let _ = fs::remove_file(lib);
    let ar = env::var("AR").unwrap_or_else(|_| "ar".to_string());
    let st = Command::new(&ar).arg("crs").arg(lib).args(objs).status().unwrap();
    assert!(st.success(), "{ar} failed for {}", lib.display());
}

// --- Bindings and test modules ---------------------------------------------------------------

fn rust_type(t: &str) -> Result<String, String> {
    let t = t.trim();
    let (is_const, rest) = match t.strip_prefix("const ") {
        Some(r) => (true, r.trim()),
        None => (false, t),
    };
    let stars = rest.matches('*').count();
    let base = rest.replace('*', "").replace(" const", "").trim().to_string();
    let scalar = match base.as_str() {
        "void" => "()",
        "INT" | "int" | "LONG" | "FIXP_DBL" | "int32_t" | "INT_PCM_DBL" => "i32",
        "UINT" | "unsigned" | "ULONG" | "uint32_t" => "u32",
        "SHORT" | "FIXP_SGL" | "int16_t" | "INT_PCM" => "i16",
        "USHORT" | "uint16_t" => "u16",
        "SCHAR" | "char" | "int8_t" => "i8",
        "UCHAR" | "uint8_t" => "u8",
        "INT64" | "int64_t" | "long long" => "i64",
        "UINT64" | "uint64_t" => "u64",
        "float" => "f32",
        "double" => "f64",
        "size_t" => "usize",
        _ if stars > 0 => "core::ffi::c_void",
        other => return Err(format!("unsupported shim parameter type `{other}`")),
    };
    if stars == 0 {
        return Ok(scalar.to_string());
    }
    let scalar = if scalar == "()" { "core::ffi::c_void" } else { scalar };
    let mut r = scalar.to_string();
    for i in 0..stars {
        r = format!("{}{r}", if is_const && i == 0 { "*const " } else { "*mut " });
    }
    Ok(r)
}

const RUST_KEYWORDS: &[&str] = &[
    "in", "fn", "type", "ref", "mod", "use", "loop", "match", "move", "box", "self", "impl",
    "trait", "where", "as", "crate", "dyn", "pub", "mut", "let", "gen", "struct", "enum", "const",
    "static", "unsafe", "extern", "return", "break", "continue", "true", "false", "if", "else",
    "while", "for", "super", "async", "await", "abstract", "final", "override", "macro", "yield",
    "try", "virtual", "priv", "typeof", "unsized", "do", "become",
];

fn gen_bindings(shims: &[PathBuf], out: &Path) {
    let mut mods = String::new();
    let mut errors = Vec::new();
    // All shims are linked into one binary: a name defined in two shims would silently resolve
    // to one of the definitions.
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for shim in shims {
        let name = shim.file_stem().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(shim).unwrap();
        let mut decls = String::new();
        for line in text.lines() {
            let Some(pos) = line.find("cref_") else { continue };
            let head = &line[..pos];
            if head.is_empty()
                || head.starts_with(char::is_whitespace)
                || head.contains('(')
                || head.contains("//")
                || head.contains('#')
                || head.contains('=')
                || head.contains(';')
            {
                continue;
            }
            let rest = &line[pos..];
            let Some(open) = rest.find('(') else { continue };
            let fname = &rest[..open];
            if !fname.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let Some(close) = rest.find(')') else { continue };
            if !rest[close + 1..].trim_start().starts_with('{') {
                continue;
            }
            let ret = head.trim().trim_start_matches("static ").trim_start_matches("inline ");
            let args = rest[open + 1..close].trim();
            let mut params = Vec::new();
            if !args.is_empty() && args != "void" {
                for a in args.split(',') {
                    let a = a.trim();
                    let split =
                        a.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(|i| i + 1);
                    let (ty, nm) = match split {
                        Some(i) if i > 0 => (&a[..i], &a[i..]),
                        _ => {
                            errors.push(format!(
                                "{}: parameter `{a}` of {fname} has no name",
                                shim.display()
                            ));
                            continue;
                        }
                    };
                    let nm =
                        if RUST_KEYWORDS.contains(&nm) { format!("{nm}_") } else { nm.to_string() };
                    match rust_type(ty) {
                        Ok(t) => params.push(format!("{nm}: {t}")),
                        Err(e) => errors.push(format!("{}: {fname}: {e}", shim.display())),
                    }
                }
            }
            let rt = match rust_type(ret) {
                Ok(t) => t,
                Err(e) => {
                    errors.push(format!("{}: {fname}: {e}", shim.display()));
                    continue;
                }
            };
            if let Some(other) = seen.insert(fname.to_string(), name.clone()) {
                errors.push(format!("{fname} is defined in shim/{other}.cpp and shim/{name}.cpp"));
            }
            write!(decls, "    pub fn {fname}({})", params.join(", ")).unwrap();
            if rt != "()" {
                write!(decls, " -> {rt}").unwrap();
            }
            decls += ";\n";
        }
        writeln!(
            mods,
            "/// Bindings of `shim/{name}.cpp`.\npub mod {name} {{\n    #![allow(clippy::missing_safety_doc)]\n    \
             extern \"C\" {{\n{decls}    }}\n}}\n"
        )
        .unwrap();
    }
    if !errors.is_empty() {
        panic!("shim binding errors:\n{}", errors.join("\n"));
    }
    write_if_changed(&out.join("cref_bindings.rs"), &mods);
}

fn gen_test_modules(manifest: &Path, out: &Path) {
    let tests = manifest.join("tests");
    let Ok(dirs) = fs::read_dir(&tests) else { return };
    let mut by_dir: BTreeMap<String, String> = BTreeMap::new();
    for d in dirs.flatten() {
        let p = d.path();
        if !p.is_dir() {
            continue;
        }
        let dir = d.file_name().to_string_lossy().into_owned();
        let mut s = String::new();
        for f in list(&p, "rs") {
            let m = f.file_stem().unwrap().to_string_lossy().into_owned();
            if m == "main" {
                continue;
            }
            writeln!(s, "#[path = {:?}]\nmod {m};", f.display().to_string()).unwrap();
        }
        by_dir.insert(dir, s);
    }
    for (dir, s) in by_dir {
        write_if_changed(&out.join(format!("tests_{dir}.rs")), &s);
    }
}

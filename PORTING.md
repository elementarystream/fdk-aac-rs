# Porting guide

fdk-aac-rs ports the Fraunhofer FDK AAC codec library, as distributed by
[mstorsjo/fdk-aac](https://github.com/mstorsjo/fdk-aac) at **v2.0.3**, to Rust: the decoder
(`aacDecoder_*`) and the encoder (`aacEnc*`), with every transport, audio object type and API
parameter. The goals are **bit-exact behavior** with the reference for every configuration and
every input, valid or corrupt; the three reference architectures AArch64, x86-64 and 32-bit x86,
whose outputs differ in the least significant bits and which the port reproduces each on its own
target; **three API layers** (a straight port of the C API, a safe Rust API, a C ABI that replaces
`libfdk-aac`); **speed** without giving up exactness; and **no panics** on any input.

Not in scope: the MIPS, PowerPC, ARMv5-v7 and MSVC code paths, fixing upstream bugs (the port
reproduces the quirks), code the reference build compiles out, and bit-exactness with builds other
than the reference builds. The [README](README.md) gives the overview and usage.

"The reference" is the C++ library built as described under [Reference builds](#reference-builds);
"the port" is the Rust code; "C" refers to the C++ sources, a C-like subset of C++.

## Reference

### Version

- **Release:** fdk-aac 2.0.3, git tag `v2.0.3` of mstorsjo/fdk-aac; `scripts/setup_reference.sh`
  verifies the commit of the tag. The `fdk-aac-2.0.3.tar.gz` release on SourceForge, which
  FFmpeg's documentation points to, is made from this tag.
- **Later commits** on master change only two codec files: `#define`s for SPARC and an Android-only
  logging guard. For macOS, Linux and Windows the codec is unchanged.
- **AOSP** (`platform/external/aac`) is the original source. Compared with AOSP main,
  mstorsjo/fdk-aac has two changes that can affect output: a saturating addition in the SBR encoder
  (`getEnvSfbEnergy` in `libSBRenc/src/env_est.cpp`) and the WG4 channel order in the encoder.

A later release becomes the reference only after the full verification passes against it and every
difference is explained by the release's code changes.

### Reference builds

| Name | Host | Compiler | Build |
|---|---|---|---|
| `ref-arm64` | macOS on Apple silicon | Apple clang (version recorded in `work/build-ref/ref-arm64/VERSION`) | CMake `-DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF`: `-O3 -DNDEBUG`, plus the project's `-fno-exceptions -fno-rtti` |
| `ref-x86_64` | Linux x86-64 | Debian clang 19.1.7 | Same |
| `ref-i686` | Linux x86-64, `-m32` | Debian clang 19.1.7 (i386-pc-linux-gnu, CPU i686: no SSE, x87 control word 0x037f) | Same, with `-m32` in `CMAKE_C_FLAGS`/`CMAKE_CXX_FLAGS` |

- **clang, not gcc:** they differ in the results of undefined shifts after vectorization.
- **CMake Release** defines `NDEBUG`. Without it, `FDK_ASSERT` becomes `assert()` on x86 builds.
- **`ref-x86_64` is authoritative for x86-64.** `scripts/linux_ref.sh` builds and runs it in a
  Debian `linux/amd64` container (`scripts/docker/linux-x86_64.Dockerfile`).
- **`ref-x86_64-mac`** (Apple clang, run under Rosetta 2) is a development aid for the `x86`
  variant on Apple silicon. It reproduces `ref-x86_64` on every corpus entry; where Apple clang
  compiles unreachable undefined arithmetic differently, the port follows `ref-x86_64`.
- All builds reproduce upstream's `test-encode-decode` hashes (`scripts/upstream_test.sh`):
  `ref-arm64` the AArch64 hashes, the x86 builds the x86 hashes.

### Architecture variants

The reference configures itself in `libFDK/include/FDK_archdef.h`: `__aarch64__` defines `__arm__`
and `__ARM_ARCH_8__`, which select all ARM code paths. Three facts keep the variants manageable:
**table precision is the same** on all reference architectures (`ARCH_PREFER_MULT_32x16`,
`SINETABLE_16BIT`, `WINDOWTABLE_16BIT`, `POW2COEFF_16BIT`, `LDCOEFF_16BIT`, and `QMF_COEFF_16BIT`
from the first), so the tables are identical; **there is no NEON code** (the ARM code is plain C
and inline assembly); and **floating point is confined to four x86 helpers** (plus three x87
expressions on 32-bit x86). The variants differ in primitives and in a few whole functions.

#### Primitives

| Primitive | `aarch64` | `x86` | `generic` |
|---|---|---|---|
| `fixmul_DD` (`fMult`) | `(a·b) >> 31` (`smull`, `asr #31`) | `((a·b) >> 32) << 1` (`imul`, `shl`) | `((a·b) >> 32) << 1` |
| `fixmuldiv2_DD` (`fMultDiv2`) | `(a·b) >> 32` (assembly) | `(a·b) >> 32` (assembly) | `(a·b) >> 32` |
| `fixmadddiv2_DD`, `fixmsubdiv2_DD` | Assembly (`fixmadd_arm.h`) | Generic | Generic |
| `cplxMultDiv2` (three overloads) | Two 64-bit products summed, one shift (`smull`/`smsubl`/`smaddl`, `asr #32`) | Generic | Generic |
| `fixnormz_D`, `fixnorm_D`, `fixnormz_S`, `fixnorm_S` | Assembly (`clz_arm.h`) | `__builtin_clz` (`clz_x86.h`) | Generic |
| `sqrtFixp`, `invSqrtNorm2`, `invFixp` | Integer code (`fixpoint_math.h`) | `float`/`double` with `sqrt`, `frexpf`, `ldexpf` (`x86/fixpoint_math_x86.h`) | Integer code |
| `schur_div` | Bit-serial integer loop | `((INT64)num << 31) / denum` | Bit-serial integer loop |

Some overrides compute the same values as the generic code and are not variants (checked by `cref`
tests on both variants): `fixmuldiv2_SD`, `scaleValueInPlace`, `fixabs_S/D/I` (used only through
`fAbs`), `fixmadddiv2_DD`/`fixmsubdiv2_DD` (the assembly ends in a 32-bit `add`/`sub`),
`fixpow2_D`, `fixmadd_DD`, `fLog2`, and the `SATURATE_*` macros of `scale_arm.h`.

**Undefined C arithmetic** (signed overflow, out-of-range shift amounts) is compiled differently by
the reference builds: one build vectorizes a shift where another uses a scalar instruction that
takes the amount modulo 32, or decides by the mathematical difference where another uses the
wrapped one. The port follows the measured code of each build, per variant. Inline functions are
measured as compiled into the `cref` shims; other call sites may compile the same operation
differently.

#### Function-level variants

`__arm__` also selects whole C implementations, which differ in rounding, overflow or edge cases:

| Function | `aarch64` | `x86` and `generic` |
|---|---|---|
| `dit_fft` (radix 2) | `libFDK/src/arm/fft_rad2_arm.cpp` | Generic `fft_rad2.cpp`; apart from `cplxMultDiv2` the same operations for `ldn >= 3`, the only lengths used |
| `fft_16` (and every length built from it: 80, 192, 240, ...) | `SHIFT_A`/`SHIFT_B`: `(a + b) >> 1` | `(a >> 1) + (b >> 1)`, which differs in the LSB and in overflow (`fft_32` uses the macros in a form that gives the same values) |
| `scaleValuesWithFactor` | `libFDK/src/arm/scale_arm.cpp`: clamps the shift before negating it, so large negative scalefactors give out-of-range shift amounts (the port follows the arm64 code) | Generic `scale.cpp`: clamps the negative shift to 31 |
| `CBlock_ScaleSpectralData_func1` | `libAACdec/src/arm/block_arm.cpp`, a `do ... while ((runs -= 4) != 0)` loop | Generic loop |

`FDK_FORCEINLINE` in `libSBRdec/src/hbe.cpp` and `noinline` in `libSACdec/src/sac_reshapeBBEnv.cpp`
are architecture conditionals that change no value.

#### Floating point and the x87 variant

- **The definition that is actually compiled is taken from the preprocessed `.ii` file** of the
  reference build, checked against the disassembly where inline assembly or `float` is involved,
  and covered by `cref` tests on that architecture. The tables above are a guide, not the
  specification.
- **x86-64.** `sqrtFixp` calls `sqrt((float)(INT)op)`, the single-precision overload on libc++ and
  libstdc++; `invSqrtNorm2` and `invFixp` compile to `sqrtss`/`divss` plus `frexpf` and `ldexpf`.
  Rust's `f32::sqrt` and an exact `frexp`/`ldexp` give the same values.
- **32-bit x86 (`x86_32`).** `ref-i686` runs the four helpers on the x87 unit: `sqrtFixp` keeps
  `fild`/`fsqrt`/`fmuls` in extended precision and truncates; `invSqrtNorm2`/`invFixp` round to
  `float` only where the result is stored for `frexpf`. Three encoder expressions also run on x87
  (`CalcGaussWindow`, `calcPeCorrection`, the DRC `limDecay`). `ref-i686` further differs from
  `ref-x86_64` in the code generation of undefined arithmetic (scalar shifts, no vectorization), in
  the packed ROM layout, in 4-byte pointers (the transport encoder's writer union, the DRC and SBR
  C layouts), in `abs.h` (`abs_x86.h` is x86-64 only) and in `schur_div` (`__divdi3`).
- **x87 in the port.** The x87 expressions are evaluated exactly; `FDK_AAC_RS_X87=f64|asm|soft`
  selects the implementation at build time:
  - `f64` (default wherever `f64` is IEEE double): double precision with a proven error bound
    (`fdk_aac_rs_fdk::x87::fast`; checked for all 2^32 operands against the model on aarch64 and
    against the x87 unit on i686), falling back to a software model of the 80-bit format
    (`x87::F80`) where the bound does not decide;
  - `asm`: the reference's x87 instructions (`x87::hw`, x86 hosts only; control word 0x037f for the
    arithmetic, 0x0f7f for the truncation, the caller's word restored);
  - `soft`: the model only.

  `CalcGaussWindow` and `calcPeCorrection` use exact integer forms.

#### Selecting a variant

- **Default** (`crates/fdk/build.rs`): `aarch64` on `aarch64` targets, `x86` on `x86_64`, `x86_32`
  on 32-bit `x86` (SSE2 required: i686 targets, not i586), and `aarch64` on all other targets.
  - On armv7 (`armv7-unknown-linux-musleabi`/`-musleabihf`) the `aarch64` variant reproduces
    `ref-arm64`: a 308-entry corpus sample and five long benchmark streams are bit-exact on a
    an ARMv8 core in AArch32 mode ([docs/optimization.md](docs/optimization.md#armv7)).
  - On other targets the default matches no C build (a C build on, for example, RISC-V uses the
    generic primitives and the generic `fft` and `scale` code). The `generic` variant is not
    complete or verified.
- **Override:** the build-time variable `FDK_AAC_RS_ARCH=aarch64|x86|x86_32|generic`, which also
  allows testing every variant on one host.
- `fdk_aac_rs_fdk::ARCH` exposes the selected variant.

### Reference material

`scripts/setup_reference.sh` creates the reference material under the git-ignored `work/`:

| Path | Content |
|---|---|
| `work/fdk-aac-v2.0.3` | Pristine sources (a git worktree of the clone `work/fdk-aac`, tag verified) |
| `work/build-ref/<build>/` | `libfdk-aac.a`, `aac-enc`, `test-encode-decode`, the harness in `harness/`, `VERSION` |
| `work/fdk-aac-resolved/<variant>/` | Sources with the architecture conditionals resolved by `unifdef`, license headers stripped. **Read these when porting.** |
| `work/fdk-aac-pp/<build>/` | Preprocessed translation units (`*.ii`). **The ground truth of what is compiled.** |
| `work/fdk-aac-asm/<build>/` | Disassembly with relocations, for inline assembly, `float` code and fused multiply-adds |
| `work/build-cref/<variant>/<key>/` | The prefixed reference library of the [differential tests](#differential-tests) |

`scripts/dump_table.py` compiles a small program against a reference source file and prints a
table with its exact values: **tables are never retyped**.

## Workspace

One crate per C++ library; libSYS is merged into the libFDK crate. The dependencies follow the C++
include graph:

```
fdk-aac-rs-fdk  (libFDK + libSYS)
  ^-- fdk-aac-rs-arith, fdk-aac-rs-drc, fdk-aac-rs-pcmutils, fdk-aac-rs-sbrdec, fdk-aac-rs-sacdec,
      fdk-aac-rs-sbrenc, fdk-aac-rs-sacenc, fdk-aac-rs-tpdec, fdk-aac-rs-tpenc
fdk-aac-rs-tpdec  <-- fdk-aac-rs-tpenc        (only for the shared tp_data types)
fdk, arith, drc, pcmutils, tpdec, sbrdec, sacdec  <-- fdk-aac-rs-aacdec
fdk, pcmutils, tpenc, sbrenc, sacenc              <-- fdk-aac-rs-aacenc
aacdec, aacenc  <-- fdk-aac-rs        (safe API; features "decoder", "encoder")
fdk, aacdec, aacenc  <-- fdk-aac-rs-capi   (C ABI, cdylib + staticlib + rlib)
fdk-aac-rs      <-- fdk-aac-rs-cli    (fdk-aac-rs-dec, fdk-aac-rs-enc)
all             <-- fdk-aac-rs-cref   (reference build, shims, differential tests; not published)
```

- **`tp_data.h` exists twice** (libMpegTPDec and libMpegTPEnc, differing only in a comment). Its
  types live in `fdk-aac-rs-tpdec::tp_data`; `fdk-aac-rs-tpenc` re-exports them.
- **Modules:** one per C++ source pair (`foo.h` + `foo.cpp`, plus the `arm/` or `x86/` variant), in
  snake case. After the license header, a ported file names its C++ files (`// Ported from ...`);
  code without a C++ counterpart says so (`// Not a port: ...`).
- **Features:** the public crate has `decoder` and `encoder`, both on by default.
- **Profiles:** `release` (`lto = "fat"`, `codegen-units = 1`) and `checked` (release with
  `overflow-checks` and `debug-assertions`), which runs every `cref` and end-to-end test a second
  time and finds arithmetic that relies on wrap-around.
- **Lints:** `[workspace.lints]` in `Cargo.toml` allows the lints that C-style code triggers. The
  fuzz targets are a separate workspace (`fuzz/Cargo.toml`).

### Naming and publication

- The public crate is `fdk-aac-rs` (library `fdk_aac_rs`); the other crates use the prefix
  `fdk-aac-rs-`. The names `fdk-aac` and `fdk-aac-sys` belong to other projects on crates.io.
- `fdk-aac-rs-cref` (it builds the C++ reference) and `fdk-aac-rs-capi` (a C library that is
  built and installed from the repository) are not published. The per-library crates are
  published as internal crates of `fdk-aac-rs`: their descriptions and READMEs say so, and they
  carry no keywords or categories. Their API is not stable.
- The published crates build without the C++ sources and without network access; the tables are
  generated once into Rust source.
- "fdk-aac" in the name identifies the library being ported, as the upstream package name does.
  The license forbids using the name "Fraunhofer" to endorse or promote derived products, and the
  project is not endorsed by Fraunhofer.

## Porting rules

### Names and types

| C++ | Rust |
|---|---|
| functions `CBlock_ReadSpectralData`, `fMultDiv2` | snake case: `cblock_read_spectral_data`, `f_mult_div2` |
| struct typedefs `CAacDecoderChannelInfo`, `FDK_BITSTREAM` | camel case: `CAacDecoderChannelInfo`, `FdkBitStream` |
| struct fields `WindowSequence`, `pDynData` | snake case: `window_sequence`, `p_dyn_data` |
| macros and constants `MAX_SFB_LONG` | the same name, `pub const` |
| `typedef enum` used as integers | `pub type Name = i32;` plus `pub const` values |
| `FIXP_DBL`, `LONG`, `INT` / `FIXP_SGL`, `SHORT` / `INT64` | `i32` (`FixpDbl`) / `i16` (`FixpSgl`) / `i64` |
| `UINT`, `ULONG` / `USHORT` / `UCHAR` / `SCHAR` | `u32` / `u16` / `u8` / `i8` |

The doc comment of every public item names the C++ item (`` /// `fMultDiv2(a, b)` ``), so that
`grep` finds it. Overloads get suffixes (`_sgl` for `FIXP_SGL`, `_to` for `(dst, src)`). The
`fMult`, `fMultDiv2`, `fMultAdd`, `fPow2`, `fAbs`, `fNorm` overload sets are generic functions that
dispatch on the operand types like C++ overload resolution: a wrong operand type selects a
different overload, and the rounding changes.

### Arithmetic

- **Integer promotion.** `UCHAR`/`SHORT` operands are promoted to `int`; mixing `int` and `UINT`
  converts to unsigned. The port converts explicitly.
- **Overflow.** Where the reference can overflow, the port uses `wrapping_*`; the `checked` profile
  finds such sites.
- **Undefined behavior: reproduce the clang result.** Where a shift amount can be out of range or a
  signed operation can overflow, the port uses the result the reference produces, as seen in its
  disassembly, with a comment giving the C++ expression; where the builds differ, per variant
  ([Primitives](#primitives)).
- **Floating point** keeps the precision of each operation (`float` or `double`, which `sqrt`
  overload) as determined from the disassembly.
- **Tables are never retyped**; `scripts/dump_table.py` generates them from the reference.

### Memory

- **Static RAM** (`aac_ram.cpp`, `sbr_ram.cpp`, `sac_ram.cpp`, `C_ALLOC_MEM`, `RAM_ALIGN`) becomes
  owned allocations of the structure that uses it.
- **Scratch buffers** (`C_ALLOC_SCRATCH_START`, the work buffers) become local arrays or
  caller-owned buffers. Code that reads stale content of a reused buffer is replicated explicitly,
  with a comment; it is found by poisoning the reference's scratch buffers before every call and
  checking that the output does not change.
- **Unions** become separate storage, unless code reads through a different member than it wrote;
  such reads are replicated explicitly.
- **The bitstream** (`FDK_BITSTREAM` over the `FDK_BITBUF` ring buffer) works on a `BitMem`, a
  reference-counted byte array shared by every clone of the bitstream. Saving and restoring a
  position by struct assignment becomes a clone and an assignment; clones read and write the same
  bytes, as copies of the C struct do. The memory lives as long as any bitstream refers to it. The
  bytes are held in relaxed atomic 64-bit words, so bitstreams can move between threads and the bit
  reader loads its five bytes with two word loads (byte writes replace their word with a
  compare-and-exchange). Owners whose C structures are copied together with the memory a bitstream
  points into (the SBR encoder's payload delay line) copy the memory and rebind their bitstreams.
  Reads past the valid data return what the C code returns.
- **Out-of-bounds accesses.** A read outside an array but inside its allocation is reproduced
  deterministically, with a comment. For accesses outside the allocation, see
  [Deviations from the reference](#deviations-from-the-reference).

### Structures with C layout

Where C code compares structures with `FDKmemcmp`, reads past an array but stays inside the
structure, or reads a union through another member, the structure is `#[repr(C)]` with the exact C
layout and every padding byte an explicit `_pad*` field, and a `cref` test compares the layout with
C `offsetof`/`sizeof` (`pod_struct!`/`pod_union!` in `crates/drc/src/drc_dec_types.rs`,
`CProgramConfig`, the byte image of `CArcoData`). Pointers into such structures become indices.

### Errors and assertions

- **Error codes stay error codes** (`AAC_DECODER_ERROR`, `AACENC_ERROR`, `TRANSPORTDEC_ERROR`,
  `SBR_ERROR`, ...) with their numeric values. The safe API converts them at its boundary.
- **`FDK_ASSERT`** is compiled out in the reference. The port's `fdk_assert!`
  (`fdk_aac_rs_fdk::fdk_assert`) never changes behavior: code that would assert continues the way
  the reference continues. In `release` the condition is not evaluated; in `checked` a violation is
  counted (`fdk_assert::violations()`) and does not panic, so `checked` can run corrupt streams;
  with `FDK_AAC_RS_STRICT_ASSERT=1` a violation panics, to find the first one. The condition must
  have no side effects.
- Plain `debug_assert!` is only for invariants of the port that no input can violate.

### Deviations from the reference

The port reproduces the reference's results wherever the reference defines them. Where it has no
defined result for an input, the port defines one and never panics:

| Reference | Port |
|---|---|
| x86 integer division trap | The arm64 result, on every variant |
| Access outside an array's allocation | A read gives 0; a write is dropped |
| Read of uninitialized memory | Gives 0 |
| Loop that would not terminate | Stops |
| NULL or missing state | The function returns its error code |

Accesses that stay inside the C structure or allocation, such as a read past an array into the next
field, are reproduced from the C layout ([Structures with C layout](#structures-with-c-layout)).
Inputs that reach a site without a defined result are excluded from the bit-exact comparison: the
fuzz targets and the tests stop before them. The code marks each site with a comment. Undefined C
arithmetic follows the code each reference build generated ([Primitives](#primitives)).

### What is not ported

- The MIPS, PowerPC, ARMv5-v7 and MSVC variants.
- Android logging (`android/log.h`) and `FDKprintf` diagnostics.
- Library-info boilerplate, except what the public API returns (`LIB_INFO`).

## Public API

### Straight port

`fdk_aac_rs_aacdec::aacdecoder_lib` and `fdk_aac_rs_aacenc::aacenc_lib` hold the functions of
`aacdecoder_lib.h` and `aacenc_lib.h` with the same parameters (handles as `&mut`, buffers as
slices) and error codes. The C ABI and the differential tests use this layer.

### Safe API

The crate `fdk-aac-rs` ([README, Rust library](README.md#rust-library)):

- Every method makes exactly the straight-port calls its documentation names, with the caller's
  slices as buffers.
- Parameters are enums with the C IDs and values as discriminants; every error carries its C code.
- `Decoder` and `Encoder` are `Send` but not `Sync`. Separate handles are independent.
- **Threads (opt-in).** `Decoder::set_threads(n)`, `EncoderBuilder::threads(n)` and
  `Encoder::set_threads(n)` (default 1); the CLIs take `-j N`. The results are the single-threaded
  ones, but two features deliver them one call later: the decoder's frame pipelining (drain with
  `Decoder::drain()`, `pending_frames()`) and the encoder's SBR pipelining
  (`Encoder::output_delay()`, drained by `flush()`). `fdk_aac_rs::chunked::decode_chunked` decodes
  a whole input with several threads and returns exactly the output of `decode_sequential`.

### C ABI

`fdk-aac-rs-capi` is a drop-in replacement for `libfdk-aac`:

- **Symbols:** the shared library exports exactly the 18 symbols of `fdk-aac.sym`. Upstream's
  static library also exposes `aacDecoder_RawISOBMFFData`, which the list omits; the port defines
  it with `global_asm!` (rustc exports every `#[no_mangle]` item), so it is global in the static
  library and absent from the shared one on macOS, Linux and Android.
- **Headers:** the upstream public headers (`aacdecoder_lib.h`, `aacenc_lib.h`, `FDK_audio.h`,
  `genericStds.h`, `machine_type.h`, `syslib_channelMapDescr.h`) with unchanged declarations and
  the license term required for modified versions ([License obligations](#license-obligations)).
- **Build files:** `fdk-aac.pc` (module `fdk-aac`, `Name:` changed to that term) and a CMake config
  with the same targets. FFmpeg's `check_pkg_config libfdk_aac fdk-aac "fdk-aac/aacenc_lib.h"
  aacEncOpen` succeeds unchanged.
- **Layout:** `CStreamInfo`, `LIB_INFO`, `AACENC_BufDesc`, `AACENC_InArgs`, `AACENC_OutArgs` and
  `AACENC_InfoStruct` are checked against the C headers (`offsetof`, `sizeof`) by a test
  (`crates/capi/tests/layout.rs`).
- **Panics.** Every function runs inside `catch_unwind`; the code returned after a panic is
  documented per function. No input is known to reach it.
- **Threads.** `aacEncOpen()` reads `FDK_AAC_RS_ENC_THREADS`, `aacDecoder_Open()` reads
  `FDK_AAC_RS_DEC_THREADS`; where unset, both read `FDK_AAC_RS_THREADS` (a positive integer;
  unset, invalid or 0 mean 1). With several decoder threads, every `aacDecoder_DecodeFrame()` that
  decodes a frame returns the previous frame's result: after the last `aacDecoder_Fill()`, call
  `aacDecoder_DecodeFrame(0)` until `AAC_DEC_NOT_ENOUGH_BITS` comes twice in a row (callers that do
  not drain, such as `test-encode-decode`, lose the last frame), and pass output buffers of the same
  size. With SBR and several encoder threads, every `aacEncEncode()` with output arguments returns
  the previous call's result (`numInSamples` excepted); flushing until `AACENC_ENCODE_EOF` drains
  it.

## Testing

### Definition of bit-exact

**Decoder.** For every call, the port returns the same:

| Call | Must match |
|---|---|
| `aacDecoder_Fill` | Return code and `bytesValid` |
| `aacDecoder_DecodeFrame` | Return code, all PCM samples, every field of `CStreamInfo` including the channel type and index arrays |
| `aacDecoder_AncDataGet` | Return code, ancillary data and its size |
| `aacDecoder_SetParam`, `aacDecoder_ConfigRaw` | Return code |
| `aacDecoder_GetFreeBytes` | Return code and free bytes |
| `aacDecoder_GetLibInfo` | The `LIB_INFO` entries |

This holds for every sequence of calls, every flag (`AACDEC_CONCEAL`, `AACDEC_FLUSH`,
`AACDEC_INTR`, `AACDEC_CLRHIST`) and every input, valid or corrupt. Beyond the returned samples
(measured with `fdkdec-ref --poison-output` on 85 streams and about 95 000 calls): with valid
output the reference never writes past `frameSize * numChannels`; without valid output (including
`AAC_DEC_NOT_ENOUGH_BITS`) it clears the **whole** buffer of `timeDataSize` samples. The C ABI and
the straight port reproduce this. The DRC decoder's `LIB_INFO` entry leaves `flags` unwritten in
the reference, and in the port.

**Encoder.** For every call, the port returns the same:

| Call | Must match |
|---|---|
| `aacEncEncode` | Return code, output bytes, `AACENC_OutArgs` (`numOutBytes`, `numInSamples`, `numAncBytes`, `bitResState`) |
| `aacEncInfo` | `AACENC_InfoStruct`, including the configuration buffer |
| `aacEncoder_SetParam`, `aacEncoder_GetParam` | Return codes and returned values |
| `aacEncGetLibInfo` | The `LIB_INFO` entries |

### Differential tests

`fdk-aac-rs-cref` links the reference library and the port into one test binary per crate. **Every
ported function has a `cref` test**, run in the `release` and `checked` profiles on every variant.

- **Build.** `crates/cref/build.rs` compiles the complete reference library from
  `work/fdk-aac-v2.0.3` with the reference flags and a prefixing header, together with the shims in
  `crates/cref/shim/`, once per variant and flag set into `work/build-cref/<variant>/<key>/`. The
  objects have the same disassembly as those of the CMake builds, apart from the `cref_` prefix.
  Git worktrees use the main checkout's `work/`. Without the reference sources every test is
  skipped.
- **Variant.** An aarch64 target links the AArch64 reference, x86-64 the x86 reference, i686 the
  i686 reference.
- **Symbols.** The C ABI crate exports the same names as the reference's `extern "C"` API; the
  prefixing header (`crates/cref/include/fdk_aac_rs_cref_prefix.h`) renames only those to `cref_*`.
  The other C++ symbols are mangled and cannot clash.
- **Tests** live in `crates/cref/tests/<crate>/` (`main.rs` plus modules; `infra/` checks linking,
  the prefix and the variant). Each compares a C++ function with its port on random and edge-case
  inputs (`fdk_aac_rs_cref::testutil::Rng`), and internal state where there is any. Edge cases
  always include `MINVAL_DBL`, `MAXVAL_DBL`, zero, values around saturation and shift amounts at or
  beyond 31.
- **Shims** wrap C++ functions as `extern "C" cref_<module>_<function>`, one definition per line
  start, with scalar or pointer parameters; `build.rs` fails on a duplicate name. They are compiled
  with the reference compiler and flags, because inline functions of the reference headers are
  compiled into them. File-local functions are reached by including the C++ file into a namespace
  after its headers:

  ```cpp
  #include "libFDK/include/fft.h"      /* every header that fft.cpp includes, first */
  namespace ref_fft {
  #include "libFDK/src/fft.cpp"        /* include guards skip the headers */
  }
  extern "C" void cref_fft_16(FIXP_DBL *x) { ref_fft::fft_16(x); }
  ```

### Reference harness

`fdkdec-ref` and `fdkenc-ref` (sources in `scripts/ref-harness/`, built into
`work/build-ref/<build>/harness/`) drive the reference's public API with a call sequence fully
determined by the command line, and write the PCM or bitstream and a JSON Lines log with one record
per API call. `fdkdec-ref` reads raw, ADIF, ADTS, LATM, LOAS, DRM or MP4 input; `fdkenc-ref` reads
WAV or raw PCM. Every parameter, flag and buffer size can be set before the run or before a given
call.

The Rust CLIs `fdk-aac-rs-dec` and `fdk-aac-rs-enc` implement **the same interface**: the same
options, call sequence and byte-identical logs. The call sequences and record formats are defined by
the harness sources and `crates/cli/src/harness/`; each program prints its options. What a
contributor must know:

- **Logs are deterministic** (no file names, times or other run-dependent data), so they are
  compared by hash; on a mismatch `scripts/e2e.py` prints the first differing record.
- **Exit codes:** 0 when the run completed (API errors are in the log); 1 when `aacDecoder_Open` or
  `aacEncOpen` failed; 2 for usage and I/O errors; 3 when the MP4 demuxer cannot use the input.
- **Packet input decodes once per access unit** by default, as FFmpeg does: USAC answers a
  `DecodeFrame` call on an empty buffer with `AAC_DEC_DECODE_FRAME_ERROR` instead of
  `AAC_DEC_NOT_ENOUGH_BITS`. **ADIF** has no frame lengths; decode it with `--decode-once`.
- **`--max-errors`** (default 1000) ends a run after that many consecutive decode errors, so a
  decoder that makes no progress does not run forever.
- **`fdkenc-ref` sets only what it is told:** apart from the sample rate and channel mode of the
  input (unless given), no parameter is set implicitly, unlike upstream's `aac-enc.c`.
- **Encoder frame lengths:** v2.0.3 accepts 1024, 512, 480, 256, 240, 128 and 120; 960 returns
  `AACENC_INVALID_CONFIG`.

### End-to-end corpus

| Corpus | Content | Used for |
|---|---|---|
| Upstream | `test-encode-decode` on `Sample01_4.wav` and its mono downmix, with the published x86 and AArch64 hashes | Both |
| Encoder matrix | PCM inputs × encoder configurations; every access unit and log compared | Encoder; its streams are also the main decoder corpus |
| External streams | ffmpeg `aac` (default, `fast`, PNS/TNS/IS/MS variations), Apple AudioToolbox (LC, HE, HE v2, VBR), commercially encoded AAC-LC | Decoder |
| Conformance | The ISO conformance bitstreams that fdk-aac supports ([Conformance bitstreams](#conformance-bitstreams)) | Decoder |
| Fraunhofer playback tests (local only) | The streams of Fraunhofer's public test pages ([ETSI material](#etsi-material)) | Decoder: DVB metadata, DRC, downmix |
| Corrupt streams and API sequences | Bit flips, truncation, splicing; call sequences with concealment, flush, interruption, mid-stream `SetParam` and encoder reconfiguration | Both |

**PCM inputs:** music, speech, broadcast, 5.1, synthetic signals (sweep, noise, clicks, square
waves at fs/2, silence, antiphase) and a mono downmix; 8 to 96 kHz; 1 to 8 channels. **Encoder
configurations:** AOTs 2, 5, 29, 23, 39 (with and without SBR, and ELDv2), 129 and 132; CBR at
three bitrates each and VBR 1-5; afterburner on and off; frame lengths 1024, 512, 480 and the ELD
lengths 256, 240, 128, 120; transports raw, ADIF, ADTS, LATM, LOAS, with CRC; signaling modes 0-2;
metadata modes 0-3; ancillary data; channel orders MPEG, WAV and WG4. All configurations run on a
core set of inputs, the full corpus on a pairwise covering.

**Manifest and inputs.** `scripts/corpus.json` lists the 6 814 entries (name, decode or encode,
input below `work/`, harness arguments). No test material is committed: synthetic signals come from
committed generator code; ISO conformance files are identified by name and ISO's checksum list,
Fraunhofer test streams by URL, and other inputs by their path below `work/` and SHA-256. Entries whose inputs cannot be distributed are named `local-*`;
without the inputs they are skipped, and `cref` tests that read `work/corpus` fall back to
synthetic signals or skip.

**Reference hashes.** `scripts/e2e.py` runs every entry through the reference harness of a build
and records the SHA-256 of every output and log in a hash file under `work/`; it then runs the Rust
CLIs on the same entries and compares their outputs and logs with these hashes. Producing the hashes
needs the reference build of the architecture.

### Conformance bitstreams

ISO publishes the conformance bitstreams and reference waveforms free of charge:

| Source | Location |
|---|---|
| ISO/IEC 14496-26, 2nd ed. (MPEG-4 Audio): one archive per directory (`compressedMp4.zip`, `compressedAss.zip` for LATM/LOAS, `compressedAdts.zip`, `compressedAdif.zip`, `referencesWav.zip`, ...) and an MD5 list | `https://standards.iso.org/iso-iec/14496/-26/ed-2/en/` |
| ISO/IEC 14496-26:2010/Amd 5 | `https://standards.iso.org/iso-iec/14496/-26/ed-1/en/amd/5/14496-26_Amd5_inserts.zip` |
| ISO/IEC 23003-7:2022 (USAC): 906 bitstreams, 892 reference WAVs, zip CRC-32s only | `https://standards.iso.org/iso-iec/23003/-7/ed-1/en/` |

**Download and verification.** `scripts/fetch_conformance.py` downloads the 14496-26 archives into
`work/conformance/` (about 1.35 GB; resumable), checks every file against ISO's MD5 list (7 111
files) and writes `work/conformance/index.json` with the parsed configuration of every stream.
`--usac` adds the USAC bitstreams (140 MB read from a 3.5 GB archive by range requests, checked
against the CRC-32s). `--summary` and `--er-coverage` print the inventory and coverage from the
files.

**Inventory:** 4 461 bitstreams, 2 804 positive and 1 657 negative. **Positive** are the families
fdk-aac supports: AAC LC, HE-AAC, HE-AAC v2, ER AAC LC/LD/ELD (with HCR, RVLC, VCB11), ELD with
LD-SBR, LD MPEG Surround, ADTS, ADIF, LATM/LOAS, and USAC (FD and LPD cores, eSBR, MPS212, audio
pre-roll, MPEG-D DRC). **Negative** are the unsupported tools (AAC Main, SSR, LTP, scalable, BSAC,
CELP, HVXC, HILN, TwinVQ, parametric, SSC, SLS, ALS, DST, Structured Audio, AudioBIFS, MPEG-1/2
layers); the port rejects them with the reference's error codes, and decodes like the reference the
few negative files that carry a configuration fdk-aac accepts.

**Coverage:**

- All 906 USAC streams decode to the end with every `DecodeFrame` call returning `AAC_DEC_OK`.
- All AAC LC MP4 files except the 24 `al08*` files (48-channel program_config_element, rejected by
  `aacDecoder_ConfigRaw`), all HE-AAC MP4 files and the ADTS and ADIF files decode without errors.
- ER AAC LD and ELD cover all 8 combinations of VCB11, RVLC and HCR; ER AAC LC covers 6 (RVLC + HCR
  without VCB11 and VCB11 + RVLC without HCR are missing).
- ER streams with epConfig 1 store each error-sensitivity category in its own MP4 track, which
  fdk-aac cannot take as one layer; epConfig 3 is rejected (`AAC_DEC_UNSUPPORTED_ER_FORMAT`).
- 503 streams have frameLengthFlag = 1: 207 with 960 samples (202 positive), the only external
  source for the decoder's 960-sample paths, and 296 with 480 samples. The corpus tags them
  `frame-length-flag`.

**License.** The download pages permit use of the inserts "in their original format without any
modifications for the purposes specified in their respective ISO standard(s)". The files are not
redistributed (not committed, not attached to releases, not in crate packages);
`scripts/fetch_conformance.py` downloads them for each developer.

### ETSI material

| Specification | What fdk-aac implements | Where |
|---|---|---|
| ETSI TS 101 154 (DVB) | DVB ancillary data: heavy compression, downmix levels, presentation mode; parsed by the decoder, written by the encoder in metadata modes 2 and 3 | `libAACdec/src/aacdec_drc.cpp`, `libPCMutils/src/pcmdmx_lib.cpp`, `libAACenc/src/metadata_main.cpp` |
| ETSI ES 201 980 (DRM) | Decoder only: the `TT_DRM` transport, the SDC audio configuration, the virtual AOTs 143-147 | `libMpegTPDec/src/tpdec_drm.cpp`, `tpdec_asc.cpp`, AAC/SBR/PS/MPS decoders |
| ETSI TS 102 563 (DAB+) | No dedicated code; 960-sample AAC decoded as raw access units | Generic AAC paths |

- **DVB.** Fraunhofer's public playback test pages (`https://www2.iis.fraunhofer.de/AAC/`) offer
  streams with DVB and MPEG DRC, downmix and loudness metadata, multichannel, HE-AAC signaling and
  xHE-AAC. Their terms permit personal use only, so `scripts/corpus.json` lists them by URL as
  local-only entries; `scripts/corpus.py fraunhofer` prints the download commands. The reference
  encoder writes DVB ancillary data itself, which covers the encoder side.
- **DAB+.** The v2.0.3 encoder cannot encode 960-sample frames. The decoder's 960 paths are covered
  by the 207 conformance streams, `cref` tests and fuzzing.
- **DRM.** There are no public DRM test streams and the encoder cannot write DRM. The DRM paths are
  covered by `cref` tests and the `drm` fuzz target.

### Differential fuzzing

`fuzz/` holds `cargo-fuzz` targets that run the reference (through `fdk-aac-rs-cref`) and the port
on the same input and compare everything under [Definition of bit-exact](#definition-of-bit-exact):
the decoder per transport, the decoder with `ConfigRaw` and raw access units, and the encoder with
random PCM and parameter sequences. They also check that the port never panics. Calls that would
reach a site without a defined reference result are stopped before the call. Fuzzing runs in a
container with `scripts/fuzz.sh`; see [fuzz/README.md](fuzz/README.md).

### Running the checks

The checks run locally. **Prerequisites:** macOS on Apple silicon (the `aarch64` variant, and
`x86` under Rosetta 2 as a development aid) or Linux x86-64 (`x86`, `x86_32`); Rust 1.85 or later
(`rustup target add x86_64-apple-darwin` for `x86` on Apple silicon); clang (other compilers give
other results at undefined-behavior sites); CMake, make, git, `unifdef`, `objdump`, curl, Python 3
(standard library only), pkg-config, ffmpeg and perl; Docker for the Linux reference builds and
fuzzing (on Apple silicon the `linux/amd64` container runs under emulation and is slow).

**Setting up `work/`.** Most scripts use `work/` of the checkout; the `cref` build, `fuzz/seeds.sh`
and the C ABI scripts find the main checkout's `work/` from a git worktree;
`FDK_AAC_RS_WORK_DIR` overrides it.

```sh
scripts/setup_reference.sh            # clone, check out v2.0.3, build, derive
scripts/linux_ref.sh                  # ref-x86_64 in the Debian container
scripts/linux_ref.sh scripts/setup_reference.sh --skip-clone --build ref-i686   # optional: x86_32
scripts/fetch_conformance.py --usac   # ISO conformance streams into work/conformance/
scripts/corpus.py gen                 # synthetic signals into work/corpus-gen/
scripts/corpus.py check               # verify present inputs against scripts/corpus.json
```

`setup_reference.sh` builds `ref-arm64` and `ref-x86_64-mac` on Apple silicon and `ref-x86_64` on
Linux (`--build NAME` selects one). Local audio and real-world streams go to `work/corpus/`,
`work/mx/`, `work/songs/`, `work/rw/` and `work/pcm/`; `scripts/corpus.py check -v` lists what is
missing.

**Differential tests.** `FDK_AAC_RS_CREF_JOBS` limits the compiler processes of the first build;
`RUST_TEST_THREADS` limits the two long tests (`e2e::e2e_streams` in `sbrdec`,
`aacenc_lib_reconfig::reconfiguration_matrix` in `aacenc`).

```sh
cargo test --release         -p fdk-aac-rs-cref --test fdk -- fixmul::
cargo test --profile checked -p fdk-aac-rs-cref --test fdk -- fixmul::
cargo test --release -p fdk-aac-rs-cref --test fdk --target x86_64-apple-darwin -- fixmul::  # x86
scripts/cref_test.sh                  # all test binaries, release and checked, with a summary
```

`scripts/cref_test.sh` takes `--targets`, `--profiles` and `--no-corpus` (as on a machine without
the local audio), among others. `FDK_AAC_RS_LD_DATA_EXHAUSTIVE` and `FDK_AAC_RS_LD_POLY_EXHAUSTIVE`
make the logarithm tests exhaustive (slow). With the cargo feature `test-hooks`,
`FDK_AAC_RS_CHUNK_POISON`, `FDK_AAC_RS_CHUNK_NO_VERIFY` and `FDK_AAC_RS_DIGEST_NEGATIVE` exercise
the rejection paths of chunked decoding; they change the output and are compiled out otherwise.

**End-to-end corpus.**

```sh
cargo build --release -p fdk-aac-rs-cli
scripts/e2e.py --manifest scripts/corpus.json --build ref-arm64 --update     # reference hashes
scripts/e2e.py --manifest scripts/corpus.json --build ref-arm64 --impl rust  # compare the port
```

The header of `scripts/e2e.py` gives the command lines for `x86` (in the container, or with
`ref-x86_64-mac` and `--dec-exe`/`--enc-exe` under Rosetta 2) and `x86_32`. `--dec-args '-j 4'`
and `--enc-args '-j 4'` check that the output does not depend on the thread count. To change the
corpus, edit the factors in `scripts/corpus.json` and regenerate the entries with
`scripts/corpus.py pin`.

**Linux.** The authoritative x86 checks run in the container. `scripts/linux_check.sh` (needs
`ref-x86_64`, the conformance streams and `scripts/corpus.py gen`) runs the reference harness, the
`cref` tests in both profiles and the Rust CLIs on the corpus; logs go to `target/linux/`.

```sh
scripts/linux_ref.sh scripts/linux_check.sh [JOBS]          # x86 variant
scripts/linux_ref.sh scripts/linux_check.sh [JOBS] --i686   # x86_32 variant against ref-i686
scripts/linux_ref.sh scripts/linux_check.sh [JOBS] --v3     # x86-64-v3 build (needs such a CPU)
```

**C ABI and FFmpeg** (each script runs every variant of the host, or the given target triple):

- `scripts/upstream_test.sh BUILD [PROGRAM]` compares upstream's `test-encode-decode` output with
  the published hashes.
- `scripts/capi_test.sh` installs the C library, headers, `fdk-aac.pc` and CMake package into a
  staging prefix and checks the exported symbols, FFmpeg's `pkg-config` test,
  `find_package(fdk-aac)`, C differential tests (`crates/capi/tests/capi_*_diff.c`, static and
  shared port against the reference), upstream's `aac-enc` and `test-encode-decode` built against
  the port.
- `scripts/ffmpeg_test.sh` builds the FFmpeg release named in the script against the reference and
  the shared and static port, and compares exit status, output hashes and normalized logs. FFmpeg
  8.1.3 against the port behaves exactly like FFmpeg against the reference on 7 180 of 7 180
  commands on macOS (arm64, and x86_64 under Rosetta 2) and 4 240 of 4 240 on Linux x86-64.
- `scripts/linux_ref.sh scripts/capi_mem.sh` compares the heap use of decoder and encoder
  instances with the reference.

**Benchmarks:** `scripts/bench.py` (results in [docs/optimization.md](docs/optimization.md)).
**Formatting and lints:** `cargo fmt --all --check` and `cargo clippy --workspace --all-targets`.

## Optimization rules

- **Exactly equivalent transformations only:** the same rounding, saturation and wrap-around, the
  same order of operations wherever it is observable, the same handling of corrupt input.
- **Every optimized kernel keeps its straight-port version**, and a test compares the two on random
  and extreme inputs.
- **SIMD code is written per variant** (the AArch64 `fMult` rounding cannot be used in an x86
  kernel). AArch64 kernels use NEON; x86-64 kernels use AVX2, selected by run-time CPU detection.
  `FDK_AAC_RS_NO_SIMD` (any value, read at run time) disables the AVX2 kernels; the scalar paths
  give the same results.
- **Optional x86-64-v3 build:** `RUSTFLAGS='-C target-cpu=x86-64-v3'`. The output is identical: the
  integer code is exact under any vectorization, and Rust never contracts floating-point operations
  into FMA.
- **Multi-threading is opt-in** (default 1 thread, so users who decode many streams in parallel get
  no unwanted threads).
  - The decoder pipelines frames (the end of a frame is decoded on a worker while the next access
    unit is parsed) and decodes the channels of a frame in parallel. Pipelining that could reorder
    state changes is speculative (snapshot and undo) or falls back to the sequential path.
  - The encoder parallelizes channels and pipelines the SBR encoder.
  - The features a stream uses are chosen per instance by measurement among exact modes
    (`fdk_aac_rs_fdk::par::tune`; `FDK_AAC_RS_TUNE` fixes the choice). The output never depends on
    it.
  - The pool is `fdk_aac_rs_fdk::par::ThreadPool` (fork-join; the calling thread takes part; nested
    use runs sequentially; panics propagate). Parallel variants live next to the straight functions
    (`*_pool`), each with a written exactness argument.
  - Every parallel path is verified by the end-to-end corpus at several thread counts, a concurrent
    stress test and buffer poisoning (`fdk-aac-rs-cli --features poison`, `FDK_AAC_RS_POISON`).
- **Chunked decoding (opt-in):** `fdk_aac_rs::chunked::decode_chunked`, or `fdk-aac-rs-dec -j N`
  with N >= 4 and a long input. Ranges are decoded by speculative single-threaded decoders that
  start a warm-up earlier. A boundary is taken only where the 128-bit digest of the complete
  decoder state (`AacDecoderInstance::state_digest`, which destructures every structure
  exhaustively, so a new field does not compile until it is fed or skipped with a reason) equals
  that of the decoder that decoded up to it; elsewhere sequential decoding continues. AAC-LC/LD/ER
  without PNS converges; PNS seeds and SBR noise and harmonic phases do not, so such streams are
  decoded sequentially.

The measurements are in [docs/optimization.md](docs/optimization.md).

## Open questions

1. **Compilers and flags.** Upstream CI runs its hash test with gcc and clang, autotools and CMake,
   `-O2` and `-O3`, with and without assertions, and gets the same output. Undefined-behavior sites
   reached only by other inputs, typically corrupt streams, can still differ between compilers. The
   port follows the clang reference builds.
2. **The autotools default build enables `FDK_ASSERT` on x86** (no `NDEBUG`), so it behaves
   differently on inputs that violate an assertion. The port follows the `NDEBUG` reference.
3. **A `generic` variant** for targets other than AArch64 and x86: it needs the generic function
   bodies of [Function-level variants](#function-level-variants) and the target's semantics at
   every undefined-behavior site.

## License obligations

The port is a modified version of the FDK AAC Codec and is distributed under its license, unchanged
([LICENSE.txt](LICENSE.txt)):

- **The full license text** accompanies source distributions and the documentation of binary
  distributions.
- **Source code is provided free of charge** to recipients of binaries.
- **No copyright license fees** may be charged.
- **Prominent change notices** with the date of each change: the README states the base version and
  the dates of the port, and the ported source files name the C++ files they come from.
- **The name.** "Fraunhofer FDK AAC Codec Library for Android" is replaced by **"Third-Party
  Modified Version of the Fraunhofer FDK AAC Codec Library for Android"** wherever it names this
  port: the license headers of the source files, the README and the documentation. The library's run-time
  strings do not contain the term (the `LIB_INFO` titles are, for example, "AAC Decoder Lib" and
  "AAC Encoder"), so the observable behavior stays identical to the reference.
- **No endorsement.** The name "Fraunhofer" is not used to endorse or promote the port.
- **Patents.** The license grants no patent rights.

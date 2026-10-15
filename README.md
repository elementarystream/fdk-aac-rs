# fdk-aac-rs

[![crates.io](https://img.shields.io/crates/v/fdk-aac-rs.svg)](https://crates.io/crates/fdk-aac-rs)
[![CI](https://github.com/elementarystream/fdk-aac-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/elementarystream/fdk-aac-rs/actions/workflows/ci.yml)

A bit-exact Rust port of the [Fraunhofer FDK AAC Codec Library](https://github.com/mstorsjo/fdk-aac)
2.0.3 (as distributed by mstorsjo/fdk-aac and used by FFmpeg's `libfdk_aac`), decoder and encoder.

For every API call and every input, valid or corrupt, the port returns what the C++ library
returns: decoded PCM, encoded bytes, stream information and error codes. It is about twice as fast
on one thread, and faster again with opt-in multi-threading.

> **Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.**
>
> This project is a modified version of the Fraunhofer FDK AAC Codec Library for Android
> (fdk-aac 2.0.3, git tag `v2.0.3`, commit `716f439`, as distributed by mstorsjo/fdk-aac): the C++
> sources were translated to Rust and modified from 2026-10-04 to 2026-10-15. Every function of
> the decoder and encoder libraries was translated with the same arithmetic and the same results;
> a safe Rust API, a C ABI and command-line tools were added; optimizations and opt-in
> multi-threading that keep the output identical were added. The ported source files name the C++
> files they come from. The project is not endorsed by Fraunhofer, and the name "Fraunhofer" is not
> used to endorse or promote it. The complete license text is in [LICENSE.txt](https://github.com/elementarystream/fdk-aac-rs/blob/main/LICENSE.txt); it
> applies to this port unchanged. Note that the license grants **no patent rights**: using AAC may
> require patent licenses from the respective patent owners.

## Features

- **Bit-exact.** Return codes, decoded PCM, `CStreamInfo`, encoded bytes and output arguments are
  identical to the reference library for every sequence of calls and every input, valid or corrupt
  ([PORTING.md, Definition of bit-exact](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#definition-of-bit-exact)). The C++ builds
  differ between architectures in the least significant bits; the port reproduces the AArch64,
  x86-64 and 32-bit x86 builds, each on its own target
  ([PORTING.md, Architecture variants](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#architecture-variants)). Measured on AArch64
  (macOS), x86-64 (macOS and Linux) and 32-bit x86 (Linux):
  - every ported function has a differential test against the C++ function, run with and without
    overflow checks;
  - all 6 814 entries of the end-to-end corpus are bit-exact on every variant: the ISO conformance
    streams for MPEG-4 Audio and USAC, an encoder matrix over every object type, transport,
    bitrate mode, frame length and channel layout, real-world streams, corrupt streams and API call
    sequences, with outputs and per-call logs compared by SHA-256 with those of the reference
    harness;
  - upstream's `test-encode-decode`, linked against the C ABI, prints the published hashes;
  - FFmpeg built against the C ABI behaves byte-identically to FFmpeg built against the C++
    library, on 7 180 encode and decode commands (macOS) and 4 240 (Linux);
  - differential fuzzing covers all decoder transports and the encoder;
  - on armv7 (static musl, an ARMv8 core in AArch32 mode), a 308-entry corpus sample is bit-exact
    against the AArch64 reference.

  Where the C++ library has no defined result, the port defines one
  ([PORTING.md, Deviations from the reference](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#deviations-from-the-reference)). The
  test setup is described in [PORTING.md, Testing](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#testing).
- **Fast.** About 1.8–1.9x faster than the C++ library on one thread. Opt-in multi-threading for
  single streams (default: one thread) reaches 2.6–3.1x at 8 threads, with output identical to
  the single-threaded decoder and encoder ([Performance](#performance)).
- **Complete.**
  - Decoder: AAC-LC, HE-AAC, HE-AAC v2, AAC-LD, AAC-ELD (with LD-SBR), ELDv2 (LD MPEG Surround),
    USAC / xHE-AAC (FD and LPD cores, eSBR, MPS212), MPEG-D DRC, DVB and MPEG metadata, downmix,
    limiter and concealment. Transports: raw, ADIF, ADTS, LATM/LOAS and DRM.
  - Encoder: AAC-LC, HE-AAC, HE-AAC v2, AAC-LD, AAC-ELD (with SBR) and ELDv2, CBR and VBR,
    afterburner, metadata and DRC compressor, ancillary data. Transports: raw, ADIF, ADTS,
    LATM/LOAS.
- **Three APIs.** A safe Rust API (`fdk-aac-rs`), the straight port of the C API
  (`fdk_aac_rs_aacdec::aacdecoder_lib`, `fdk_aac_rs_aacenc::aacenc_lib`) and a C ABI
  (`fdk-aac-rs-capi`) that replaces `libfdk-aac` without changes to its users, with the same 18
  exported symbols, headers, `fdk-aac.pc` and CMake package.
- **Pure Rust, no panics** on any input; the C ABI also catches panics at its boundary.

| C++ library | Rust crate |
| --- | --- |
| libFDK, libSYS | `fdk-aac-rs-fdk` |
| libArithCoding | `fdk-aac-rs-arith` |
| libDRCdec | `fdk-aac-rs-drc` |
| libPCMutils | `fdk-aac-rs-pcmutils` |
| libMpegTPDec | `fdk-aac-rs-tpdec` |
| libMpegTPEnc | `fdk-aac-rs-tpenc` |
| libSBRdec | `fdk-aac-rs-sbrdec` |
| libSACdec | `fdk-aac-rs-sacdec` |
| libAACdec | `fdk-aac-rs-aacdec` |
| libSBRenc | `fdk-aac-rs-sbrenc` |
| libSACenc | `fdk-aac-rs-sacenc` |
| libAACenc | `fdk-aac-rs-aacenc` |
| safe Rust API | `fdk-aac-rs` |
| command-line tools | `fdk-aac-rs-cli` (binaries `fdk-aac-rs-dec`, `fdk-aac-rs-enc`) |
| C API (`libfdk-aac`) | `fdk-aac-rs-capi` (not published) |

The per-library crates are internal crates of `fdk-aac-rs` and not a public API.

## Performance

Speedup over the C++ library (CMake Release build, clang `-O3`), wall-clock time, 16 decoder and
encoder cases, every output byte-identical ([docs/optimization.md](https://github.com/elementarystream/fdk-aac-rs/blob/main/docs/optimization.md)):

| Threads | arm64 (Apple M1 Max) | x86-64 (Linux) | i686 (Linux) |
|---|---|---|---|
| 1 (default) | 1.90x | 1.78x | 1.81x |
| 2 | 2.63x | 2.23x | |
| 4 | 2.93x | 2.42x | 2.42x |
| 8 | 3.10x | 2.59x | |

Decoding a long AAC-LC file reaches 10.6x at 8 threads on arm64 (chunked decoding). The decoder
instance takes the same heap as the C++ library's, the encoder instance up to 16% less; neither
leaks.

## Command-line tools

```sh
CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo install fdk-aac-rs-cli
```

(The two settings are those of the workspace's release profile, with which the speed figures were
measured; `cargo install` does not use the workspace profiles.)

`fdk-aac-rs-dec` and `fdk-aac-rs-enc` expose every API parameter and write a JSON log per API call;
they take the same arguments as the reference harness programs
([PORTING.md, Reference harness](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#reference-harness)).

## Rust library

```toml
[dependencies]
fdk-aac-rs = "0.1"
```

Decoding ADTS:

```rust
use fdk_aac_rs::decoder::{DecodeFlags, Decoder, DecoderError, Transport};

let mut dec = Decoder::new(Transport::Adts)?;
let mut pcm = vec![0i16; 8 * 2048];
let mut input = &adts[..];
while !input.is_empty() {
    let consumed = dec.fill(input)?;
    input = &input[consumed..];
    loop {
        match dec.decode_frame(&mut pcm, DecodeFlags::empty()) {
            Ok(frame) => { /* frame.samples(): interleaved s16; dec.stream_info() */ }
            Err(DecoderError::NotEnoughBits) => break,
            Err(e) => return Err(e.into()),
        }
    }
}
```

Encoding HE-AAC v2:

```rust
use fdk_aac_rs::encoder::{Aot, Bitrate, ChannelMode, Encoder, Transport};

let mut enc = Encoder::builder()
    .aot(Aot::HeAacV2)
    .sample_rate(48_000)
    .channel_mode(ChannelMode::Stereo)
    .bitrate(Bitrate::Cbr(32_000))
    .transport(Transport::Adts)
    .build()?;
let mut out = vec![0u8; 8192];
let r = enc.encode(&pcm, &mut out)?;      // interleaved s16 input
let bytes = &out[..r.out_bytes()];
let r = enc.flush(&mut out)?;             // at the end of the input
```

Errors carry the C error code (`e.code()`). `Decoder::set_threads(n)` and
`EncoderBuilder::threads(n)` enable multi-threading; see the crate documentation for how frames are
delivered and drained then.

## C library

`fdk-aac-rs-capi` builds a drop-in replacement for `libfdk-aac`
([PORTING.md, C ABI](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#c-abi)):

```sh
cargo build --release -p fdk-aac-rs-capi   # target/release/libfdk_aac.{a,so,dylib}
```

Install the library as `libfdk-aac.a` and `libfdk-aac.so.2` / `libfdk-aac.2.dylib`, with the
headers of `crates/capi/include/fdk-aac/` and `fdk-aac.pc` from `crates/capi/fdk-aac.pc.in`;
`scripts/capi_test.sh` shows the complete installation, including the CMake package files.
FFmpeg's `--enable-libfdk-aac` then builds against it unchanged. `FDK_AAC_RS_DEC_THREADS`,
`FDK_AAC_RS_ENC_THREADS` and `FDK_AAC_RS_THREADS` enable multi-threading without code changes.

## Development

Hosts: macOS on Apple silicon and Linux x86-64. Needed: Rust 1.85 or later, clang, CMake, git,
`unifdef`, `objdump`, `curl` and Python 3; Docker for the Linux reference builds and fuzzing;
`pkg-config`, `ffmpeg` and `perl` for the C ABI and FFmpeg checks.

The reference and the test material live in `work/` (git-ignored; `FDK_AAC_RS_WORK_DIR` overrides
the location). Audio, ISO conformance streams and Fraunhofer test streams are never committed;
`scripts/corpus.json` identifies them by name and checksum, URL, or path and SHA-256
([PORTING.md, End-to-end corpus](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#end-to-end-corpus)).

| Path | Content | Source |
| --- | --- | --- |
| `work/fdk-aac-v2.0.3/` | C++ sources of fdk-aac v2.0.3 (tag verified) | `scripts/setup_reference.sh` |
| `work/build-ref/<build>/` | reference builds and the reference harness | `scripts/setup_reference.sh`, `scripts/linux_ref.sh` |
| `work/conformance/` | ISO conformance streams (not redistributable) | `scripts/fetch_conformance.py` |
| `work/corpus-gen/` | synthetic signals | `scripts/corpus.py gen` |
| `work/corpus/` and others | local audio, real-world streams, Fraunhofer test streams | local only; `scripts/corpus.py check -v` lists what is missing |
| `work/sums/` | reference output hashes | `scripts/e2e.py` |

```sh
scripts/setup_reference.sh            # clone fdk-aac v2.0.3, verify the commit, build, derive
scripts/fetch_conformance.py --usac   # ISO conformance streams into work/conformance/
scripts/corpus.py gen                 # synthetic signals into work/corpus-gen/
```

The main checks:

```sh
scripts/cref_test.sh                  # per-function differential tests, release and checked
scripts/e2e.py --manifest scripts/corpus.json --build ref-arm64 --update     # reference hashes
scripts/e2e.py --manifest scripts/corpus.json --build ref-arm64 --impl rust  # Rust CLIs
scripts/linux_ref.sh                  # x86-64 reference build in the Linux container
scripts/linux_ref.sh scripts/linux_check.sh   # x86 variant: harness, cref tests and CLIs
scripts/bench.py                      # speed against the reference, outputs compared
cargo clippy --workspace --all-targets && cargo fmt --all --check
```

The end-to-end run of the Rust CLIs needs `cargo build --release -p fdk-aac-rs-cli` first. Corpus
entries that need local-only material (`local-*`) are skipped when the files are missing. Every
option, the x86 variant on Apple silicon, the C ABI and FFmpeg checks and fuzzing are described in
[PORTING.md, Running the checks](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#running-the-checks).

The repository layout:

- `crates/`: one crate per C++ library (`fdk`, `arith`, `drc`, `pcmutils`, `tpdec`, `tpenc`,
  `sbrdec`, `sacdec`, `aacdec`, `sbrenc`, `sacenc`, `aacenc`), the safe API (`fdk-aac-rs`), the C
  ABI (`capi`, built from the repository) and the command-line tools (`cli`)
- `crates/cref`: differential tests against the C++ reference (`fdk-aac-rs-cref`, not published)
- `fuzz/`: differential fuzz targets ([fuzz/README.md](https://github.com/elementarystream/fdk-aac-rs/blob/main/fuzz/README.md))
- `scripts/`: reference setup, reference harness, corpus manifest, end-to-end, C ABI, FFmpeg and
  benchmark scripts
- `docs/`: [speed and memory measurements](https://github.com/elementarystream/fdk-aac-rs/blob/main/docs/optimization.md), and why each optimization is
  exact

See [PORTING.md](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md) for the reference builds, the architecture variants, the porting
rules, the public API and the testing.

## License

[Software License for The Fraunhofer FDK AAC Codec Library for Android](https://github.com/elementarystream/fdk-aac-rs/blob/main/LICENSE.txt), unchanged
([PORTING.md, License obligations](https://github.com/elementarystream/fdk-aac-rs/blob/main/PORTING.md#license-obligations)). The license grants no patent
rights.

# Differential fuzzing

`cargo-fuzz` targets that run the C++ reference (linked through `fdk-aac-rs-cref`) and the port on
the same input and compare, after every API call, everything that
[PORTING.md, Definition of bit-exact](../PORTING.md#definition-of-bit-exact) requires to match
([PORTING.md, Differential fuzzing](../PORTING.md#differential-fuzzing)). A port panic or hang is
a finding as well. This directory is a cargo workspace of its own, built with nightly Rust in the
container of `scripts/fuzz.sh`.

## Targets

| Target | Transport |
|---|---|
| `decode_adts`, `decode_adif`, `decode_latm`, `decode_loas` | `TT_MP4_ADTS`, `TT_MP4_ADIF`, `TT_MP4_LATM_MCP1`, `TT_MP4_LOAS` |
| `decode_raw` | `TT_MP4_RAW` with `ConfigRaw` (AudioSpecificConfig), or `TT_MP4_LATM_MCP0` with `ConfigRaw` (StreamMuxConfig) |
| `decode_drm` | `TT_DRM` with `ConfigRaw` (SDC audio entity) |
| `encode` | every transport (`AACENC_TRANSMUX`) |

The decoder targets call Open, GetFreeBytes, Fill, DecodeFrame, SetParam, AncDataInit,
AncDataGet, ConfigRaw and Close, and compare return codes, `bytesValid`, free bytes, the whole
output buffer (also beyond the valid samples), the complete `CStreamInfo`, every AncDataGet result
and the whole ancillary buffer. The encoder target calls aacEncOpen, SetParam, aacEncEncode
(initialization, frames, odd sizes, flush; 8/16/32-bit input, ancillary data, metadata, small
output buffers, `OUT_AU_SIZES`, NULL descriptors), aacEncInfo, GetParam and aacEncClose, and
compares return codes, `AACENC_OutArgs`, the output and AU-size buffers, `AACENC_InfoStruct` with
the configuration buffer and every GetParam value. The `GetLibInfo` functions are compared once per
process.

A decoder input holds a small call program (fill sizes, the flags `AACDEC_CONCEAL`/`FLUSH`/`INTR`/
`CLRHIST`, mid-stream SetParam, output buffer sizes, ConfigRaw, AncDataInit, dropped data), the
configuration and the bitstream, as a byte stream or as length-prefixed access units. An encoder
input holds the aacEncOpen arguments, a program of parameter changes and calls, and a PCM tape.
The formats are documented at the top of [`src/dec.rs`](src/dec.rs) and [`src/enc.rs`](src/enc.rs).

## Running

Fuzzing needs only Docker on the host (`scripts/docker/fuzz.Dockerfile`: Debian, nightly Rust,
cargo-fuzz, clang). The default platform is `linux/amd64`, the `x86` reference variant;
`FUZZ_PLATFORM=linux/arm64` fuzzes the `aarch64` variant. `fuzz/seeds.sh` and native replays use
the host's stable Rust. Seeds come from `work/`: conformance streams, real-world streams, streams
of the reference encoder from the synthetic signals, generated DRM configurations and encoder
programs.

```sh
fuzz/seeds.sh [MAX_BYTES]              # seed corpora into fuzz/corpus/<target>/
scripts/fuzz.sh exec ./seeds.sh        # the same in the container (also builds image and cref)
scripts/fuzz.sh decode_adts 600 4      # one target for 600 s with 4 processes
scripts/fuzz.sh all 300 4              # every target in turn
scripts/fuzz.sh exec COMMAND...        # any command in the container (in fuzz/)
scripts/fuzz_status.sh [--duration SECONDS]   # latest statistics and findings per target
```

`scripts/fuzz.sh TARGET [SECONDS] [JOBS]` builds the target (`cargo fuzz build -O -s none`: release
semantics, no sanitizer), runs libFuzzer in fork mode (it continues after findings), writes
findings to `fuzz/artifacts/<target>/` and the log to `fuzz/artifacts/<target>.log`, minimizes new
crashes with `cargo fuzz tmin` and replays them. Environment: `FUZZ_CHECKED=1` builds with debug
assertions and overflow checks (an overflow panic shows arithmetic that relies on wrap-around),
`FUZZ_MAX_LEN` (default 16384, `encode` 8192), `FUZZ_TIMEOUT` (default 60 s),
`FUZZ_REF_BUDGET_MS` (default 10000), `FUZZ_TMIN=0`, `FUZZ_ARGS`. Corpora, artifacts and logs are
git-ignored.

**A long campaign on a many-core Linux host** needs Docker, git and Python 3: fetch the reference
sources (`scripts/setup_reference.sh --skip-build --skip-derived`) and the seed material
(`scripts/fetch_conformance.py --usac`, `scripts/corpus.py gen`), run the seed step, then start
one `scripts/fuzz.sh TARGET 86400 JOBS` per group of cores. Without the seed step, start one
target alone first: parallel first builds wait for each other's lock. `decode_raw` (USAC, ELD and
MPEG Surround, the slowest decodes) and `encode` profit most from more processes.

## Findings

A difference panics with a report (`fdk-aac-rs differential fuzzing: DIFFERENCE in ...`) that
names the first differing call, both results, the first differing sample or byte and the
preceding calls.

```sh
fuzz/replay.sh FILE|DIR...                        # natively, this host's variant
fuzz/replay.sh --docker FILE...                   # in the container (FUZZ_PLATFORM; default x86)
fuzz/replay.sh --trace decode_raw FILE            # with both call traces
FDK_FUZZ_ECHO=1 fuzz/replay.sh FILE               # every call as it runs
scripts/fuzz.sh exec cargo fuzz tmin -O -s none encode artifacts/encode/crash-...  # minimize
```

The target is taken from the path (`fuzz/artifacts/<target>/`) unless given. Each input replays as
a match, a stop before a call that reaches a reference site without a defined result, a reference
that depends on memory contents, or a difference. For `timeout-*` artifacts each side runs alone
with a wall-clock limit. A finding is meaningful only on the variant it was found on: replay x86
findings with `--docker` on an arm64 host, and Linux findings in the container (macOS replays have
neither the reference arena nor a hard CPU budget). A difference is a port bug unless it traces
back to a reference site without a defined result; such sites are handled as described in
[PORTING.md, Deviations from the reference](../PORTING.md#deviations-from-the-reference).

Every kept finding replays as a match or as a stop before a reference site without a defined
result.

## Reference undefined behavior

Inputs that reach undefined behavior of the reference are stopped before the call; the traces are
compared up to that call (`src/guard.c`, `src/dec.rs`, `src/enc.rs`). API sequences without a
defined result in the reference are not generated, and states that the port detects stop both
runs before the call.

- **Crashes:** every reference call runs under a signal guard and is left with `siglongjmp`.
- **Heap:** on Linux the guarded reference allocates from an arena of its own (interposed
  `malloc` family, reset per run), so it cannot corrupt the fuzzer's heap and a jump out of it
  cannot leave a `malloc` lock held.
- **Hangs:** the reference part of an input has a CPU budget (`FUZZ_REF_BUDGET_MS`). The port has
  none; a port hang is a libFuzzer timeout.
- **Uninitialized memory:** the stack below every call, every `malloc` block and a redzone are
  filled with a pattern; when the traces differ, the reference is rerun with three other patterns,
  and an input whose reference trace changes is skipped.

Accesses that reach initialized memory, and undefined arithmetic compiled differently at another
call site or by another compiler, are not detected automatically and are analyzed by hand when
reported. With `FUZZ_CHECKED=1`, a `debug_assert!` that guards such a site is not a port bug if the
reference reaches the same site.

A campaign against a gcc build of the reference
([PORTING.md, Open questions](../PORTING.md#open-questions)) is not set up: the `cref` cache key
includes the compiler, but the container builds with clang.

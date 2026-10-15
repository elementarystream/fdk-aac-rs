# Performance

This document records the speed and memory use of the port against the reference library and
summarizes the optimizations. Every optimization keeps the output bit-exact: the corpus, the
`cref` tests and the C ABI checks pass unchanged on every variant. The rules are in
[PORTING.md, Optimization rules](../PORTING.md#optimization-rules).

## Speed

Wall-clock seconds, best of 5, `scripts/bench.py`; every output byte-identical to the reference.
The speedup is the reference time divided by the port time. arm64: Apple M1 Max (macOS); x86-64
and i686: Linux.

Single-threaded (the default):

| Case | arm64 ref | arm64 port | | x86-64 ref | x86-64 port | | i686 ref | i686 port | |
|---|---|---|---|---|---|---|---|---|---|
| dec lc320 (243 s) | 0.543 | 0.209 | 2.59x | 0.773 | 0.322 | 2.40x | 0.887 | 0.366 | 2.43x |
| dec lc128 | 0.445 | 0.171 | 2.60x | 0.670 | 0.296 | 2.26x | 0.769 | 0.335 | 2.29x |
| dec lc128-noab | 0.445 | 0.171 | 2.60x | 0.660 | 0.297 | 2.23x | 0.764 | 0.325 | 2.35x |
| dec he64 | 0.616 | 0.293 | 2.10x | 0.926 | 0.533 | 1.74x | 1.082 | 0.664 | 1.63x |
| dec hev2-32 | 0.680 | 0.344 | 1.98x | 1.040 | 0.662 | 1.57x | 1.229 | 0.787 | 1.56x |
| dec ld128 | 0.397 | 0.173 | 2.29x | 0.618 | 0.296 | 2.09x | 0.712 | 0.336 | 2.12x |
| dec eld-sbr64 | 0.801 | 0.438 | 1.83x | 1.183 | 0.734 | 1.61x | 1.409 | 0.886 | 1.59x |
| dec lc384-5.1 | 1.134 | 0.473 | 2.40x | 1.791 | 0.816 | 2.19x | 2.004 | 0.904 | 2.22x |
| dec usac (12 streams) | 1.474 | 0.676 | 2.18x | 1.568 | 0.913 | 1.72x | 1.828 | 1.111 | 1.65x |
| enc lc128 | 1.576 | 0.873 | 1.81x | 2.301 | 1.234 | 1.87x | 2.536 | 1.409 | 1.80x |
| enc lc128-noab | 0.741 | 0.427 | 1.73x | 0.991 | 0.602 | 1.64x | 1.154 | 0.700 | 1.65x |
| enc he64 | 1.872 | 0.831 | 2.25x | 2.565 | 1.223 | 2.10x | 2.986 | 1.418 | 2.11x |
| enc hev2-32 | 1.182 | 0.593 | 1.99x | 1.450 | 0.864 | 1.68x | 2.826 | 1.034 | 2.73x |
| enc ld128 | 1.985 | 1.204 | 1.65x | 2.882 | 1.778 | 1.62x | 3.159 | 2.009 | 1.57x |
| enc eld-sbr64 | 1.979 | 1.311 | 1.51x | 2.851 | 2.042 | 1.40x | 3.260 | 2.335 | 1.40x |
| enc lc384-5.1 | 3.978 | 2.238 | 1.78x | 5.925 | 3.190 | 1.86x | 6.495 | 3.626 | 1.79x |
| **Total** | 19.847 | 10.426 | **1.90x** | 28.195 | 15.802 | **1.78x** | 33.100 | 18.244 | **1.81x** |

The optional [x86-64-v3 build](#x86-64-v3-build): total 1.88x single-threaded (dec usac 2.01x,
enc lc128 1.95x; measured before the [bit buffer memory](#bit-buffer-memory) change).

With threads (opt-in, `-j N`; wall-clock speedup against the single-threaded reference; the
paths are chosen per stream by measurement, see [thread mode selection](#thread-mode-selection)):

| Case | arm64 -j 1 / 2 / 4 / 8 | x86-64 -j 1 / 2 / 4 / 8 | i686 -j 1 / 4 |
|---|---|---|---|
| dec lc320 (243 s) | 2.59x / 4.19x / 5.61x / 10.64x | 2.40x / 3.19x / 4.75x / 8.72x | 2.43x / 4.62x |
| dec lc128 | 2.60x / 4.12x / 5.34x / 7.99x | 2.26x / 2.76x / 3.59x / 5.13x | 2.29x / 3.93x |
| dec lc128-noab | 2.60x / 4.18x / 5.41x / 8.12x | 2.23x / 2.83x / 3.67x / 5.18x | 2.35x / 3.75x |
| dec he64 | 2.10x / 2.88x / 3.17x / 3.13x | 1.74x / 2.36x / 2.32x / 2.36x | 1.63x / 2.26x |
| dec hev2-32 | 1.98x / 2.07x / 2.05x / 2.06x | 1.57x / 1.63x / 1.62x / 1.65x | 1.56x / 1.56x |
| dec ld128 | 2.29x / 2.35x / 4.62x / 6.92x | 2.09x / 2.27x / 3.22x / 4.76x | 2.12x / 3.45x |
| dec eld-sbr64 | 1.83x / 2.50x / 2.48x / 2.49x | 1.61x / 1.89x / 1.90x / 1.89x | 1.59x / 1.86x |
| dec lc384-5.1 | 2.40x / 4.20x / 6.23x / 10.82x | 2.19x / 2.92x / 4.81x / 6.95x | 2.22x / 4.91x |
| dec usac (12 streams) | 2.18x / 2.20x / 2.20x / 2.20x | 1.72x / 1.74x / 1.74x / 1.73x | 1.65x / 1.63x |
| enc lc128 | 1.81x / 2.57x / 2.57x / 2.57x | 1.87x / 2.33x / 2.35x / 2.32x | 1.80x / 2.14x |
| enc lc128-noab | 1.73x / 2.01x / 2.01x / 2.02x | 1.64x / 1.64x / 1.64x / 1.63x | 1.65x / 1.64x |
| enc he64 | 2.25x / 3.90x / 4.01x / 4.05x | 2.10x / 3.40x / 3.36x / 3.35x | 2.11x / 3.35x |
| enc hev2-32 | 1.99x / 2.74x / 2.73x / 2.74x | 1.68x / 2.05x / 2.03x / 2.07x | 2.73x / 3.28x |
| enc ld128 | 1.65x / 2.32x / 2.32x / 2.33x | 1.62x / 1.99x / 1.98x / 2.04x | 1.57x / 1.90x |
| enc eld-sbr64 | 1.51x / 2.04x / 2.52x / 2.51x | 1.40x / 1.81x / 1.91x / 1.92x | 1.40x / 1.88x |
| enc lc384-5.1 | 1.78x / 2.63x / 3.30x / 3.84x | 1.86x / 2.48x / 2.99x / 3.59x | 1.79x / 2.83x |
| **Total** | 1.90x / 2.63x / 2.93x / 3.10x | 1.78x / 2.23x / 2.42x / 2.59x | 1.81x / 2.42x |

Totals of the 16 cases, wall-clock / CPU seconds of the port (each speedup above uses the
reference times of the same run):

| | -j 1 | -j 2 | -j 4 | -j 8 |
|---|---|---|---|---|
| arm64 | 10.426 / 10.36 | 7.592 / 14.10 | 6.777 / 16.77 | 6.394 / 18.31 |
| x86-64 | 15.802 / 15.77 | 12.642 / 22.37 | 11.691 / 27.00 | 10.938 / 28.27 |
| i686 | 18.244 / 18.17 |  | 13.695 / 31.00 |  |

The CPU time above the wall-clock time belongs to modes that are faster (the pool workers spin
between forks; see [thread mode selection](#thread-mode-selection)), e.g. x86-64 enc lc384-5.1 at
-j 8: 1.658 s wall, 8.586 s CPU.

## Memory

**Codec instances** (`scripts/capi_mem.sh`, Linux). `scripts/capi_mem.c` replaces glibc's malloc
family, counts the usable size of every live block in all threads, and drives one instance
through the C API (open, configure, the whole bench input, close), linked once against the
reference library and once against fdk-aac-rs-capi. KiB relative to the heap before the open:
after the open (the instance) / peak while processing / left after the close / left after a
second run in the same process. Thread stacks and other memory outside malloc are not counted.

| Case | x86-64 reference | x86-64 port -j 1 | port -j 2 | port -j 4 | port -j 8 |
|---|---|---|---|---|---|
| dec lc128 | 434.0 / 488.7 / 0 / 0 | 435.1 / 522.2 / 31.9 / 0 | 436.3 / 723.0 / 32.2 / 0 | 436.3 / 723.0 | 436.3 / 723.0 |
| dec he64 | 434.0 / 562.0 / 0 / 0 | 435.1 / 610.6 / 35.0 / 0 | 436.3 / 895.8 / 35.7 / 0 | 436.3 / 895.8 | 436.3 / 895.8 |
| dec hev2-32 | 434.0 / 549.1 / 0 / 0 | 435.1 / 595.1 / 34.8 / 0 | 436.3 / 857.9 / 35.4 / 0 | 436.3 / 857.9 | 436.3 / 857.9 |
| dec ld128 | 434.0 / 488.7 / 0 / 0 | 435.1 / 522.2 / 31.9 / 0 | 436.3 / 722.8 / 32.2 / 0 | 436.3 / 722.9 | 436.3 / 722.9 |
| dec eld-sbr64 | 434.0 / 532.8 / 0 / 0 | 435.1 / 572.5 / 33.8 / 0 | 436.3 / 789.4 / 34.4 / 0 | 436.3 / 789.4 | 436.3 / 789.4 |
| dec lc384-5.1 | 434.0 / 603.1 / 0 / 0 | 435.1 / 632.8 / 31.9 / 0 | 436.3 / 932.3 / 32.6 / 0 | 436.3 / 967.3 | 436.3 / 985.5 |
| enc lc128 | 329.6 / 329.6 / 0 / 0 | 278.3 / 310.7 / 31.9 / 0 | 278.3 / 324.3 / 32.2 / 0 | 278.3 / 324.3 | 278.3 / 324.3 |
| enc he64 | 329.6 / 329.6 / 0 / 0 | 278.2 / 311.3 / 32.6 / 0 | 278.2 / 474.7 / 32.9 / 0 | 278.2 / 487.9 | 278.2 / 487.9 |
| enc hev2-32 | 329.6 / 329.6 / 0 / 0 | 278.2 / 314.4 / 33.8 / 0 | 278.2 / 461.2 / 34.2 / 0 | 278.2 / 461.2 | 278.2 / 461.2 |
| enc ld128 | 329.6 / 329.6 / 0 / 0 | 278.3 / 310.7 / 31.9 / 0 | 278.3 / 324.3 / 32.2 / 0 | 278.3 / 324.3 | 278.3 / 324.3 |
| enc eld-sbr64 | 329.6 / 329.6 / 0 / 0 | 278.2 / 311.3 / 32.5 / 0 | 278.2 / 474.2 / 32.9 / 0 | 278.2 / 487.4 | 278.2 / 487.4 |
| enc lc384-5.1 | 505.1 / 505.1 / 0 / 0 | 488.7 / 521.4 / 31.9 / 0 | 488.7 / 595.6 / 32.2 / 0 | 488.7 / 597.7 | 488.7 / 599.2 |

(The -j 4 and -j 8 columns: open / peak.) i686 gives the same picture: decoder instance
432.1 against 433.0 KiB, peak 486.6 (reference) and 519.8 (port) for lc128; encoder instance
325.3 against 274.3 KiB (5.1: 497.5 against 482.5).

- **No leaks** in either library: a second open, process and close leaves 0 bytes.
- **FFT twiddle tables.** The 32-35 KiB left after the first close are the x86 AVX2 FFT's twiddle
  tables (`fft_rad2/x86.rs`), built once per process; the reference keeps its tables in read-only
  data. They are most of the port's higher decoder peak (lc128: +33.5 KiB).
- **Smaller encoder instance:** 51 KiB (16%) smaller for stereo, 16 KiB for 5.1; the port
  allocates a few buffers on first use, the reference everything at the open.
- **Thread overhead.** Decoder: pipeline shadow state and double-buffered channel memory (about
  +200 KiB stereo, +300 KiB 5.1 at -j 2). Encoder: the SBR pipeline's double-buffered input
  (about +160 KiB) and per-thread channel scratch (5.1: +75 KiB).

**Processes** (`scripts/bench.py --mem`, peak resident set size of the CLI). It is dominated by how
the programs buffer their files, not by the codec. Both encoder programs read the whole PCM input
into memory (43 MB stereo, 129 MB 5.1); the reference harness converts it into a second array, so
it peaks at 93 MiB (5.1: 271 MiB) against the port's 47-48 MiB (5.1: 136-137 MiB) on arm64,
x86-64 and i686. The decoder programs peak at 3-17 MiB on arm64 and at 12-16 MiB in both on
Linux. With 8 threads, chunked decoding keeps decoded chunks that wait for their turn
(`--chunk-mem`, default 256 MiB): arm64 dec lc320 62.6 MiB, lc384-5.1 87.2 MiB (x86-64: 42.1
and 82 MiB); the streams that are not chunked stay at 7-13 MiB.

## Method

`scripts/bench.py` runs each case with the reference harness (`fdkdec-ref`, `fdkenc-ref` of the
reference build) and with the Rust CLIs (`fdk-aac-rs-dec`, `fdk-aac-rs-enc`). Both take the same
options and make the same library calls, without a log. It reports the wall-clock and CPU time of
the best of N runs and checks that the outputs are byte-identical. `scripts/capi_mem.sh` measures
codec instances as described under [Memory](#memory).

Inputs (`work/bench`, created by the script on first use): `dec lc320` is the real-world stream
`work/rw/lc320.aac` (243 s, stereo, 44.1 kHz, AAC-LC 320 kbit/s). Its PCM, decoded by the
reference, and a 5.1 PCM made from it are encoded by the reference encoder as AAC-LC 128 kbit/s
(with and without the afterburner), HE-AAC 64, HE-AAC v2 32, AAC-LD 128 (LOAS), AAC-ELD with SBR
64 (LOAS) and 5.1 AAC-LC 384 kbit/s; each encoding is a decoder case and an encoder case with the
same parameters. `dec usac` is the 12 largest USAC conformance streams
(`work/conformance/23003-7`).

```sh
scripts/bench.py --runs 5                                   # arm64: ref-arm64
scripts/linux_ref.sh python3 scripts/bench.py --runs 5        # x86-64: ref-x86_64
scripts/linux_ref.sh python3 scripts/bench.py --runs 5 --build ref-i686 \
  --dec target/linux/i686-unknown-linux-gnu/release/fdk-aac-rs-dec \
  --enc target/linux/i686-unknown-linux-gnu/release/fdk-aac-rs-enc
```

## Optimizations

Each change keeps its straight port (`*_ref`, `*_straight`) with a fast-versus-straight test on
random and extreme inputs, besides the `cref` tests, and passes the full corpus on aarch64, x86-64
and i686 (on x86 also with `FDK_AAC_RS_NO_SIMD=1`). Vector kernels are tested on every variant.

### DSP kernels

FFT, DCT, MDCT and QMF kernels in NEON; per-lane coefficients reproduce each C butterfly with the
variant's truncation, and vector blocks only cover independent iterations. arm64: fft 512 3.69x,
dct_iv 1024 3.16x against the straight ports.

### Decoder core and PCM utilities

Inverse quantization from a table built by the same computation; Huffman decoding with one 10-bit
table per codebook on a cached reader that refills exactly when the straight reader would; the
limiter's non-limiting steady state (gain provably 1) in bulk; slice loops for interleaving and M/S.

### SBR and PS decoder

NEON decorrelator, LPC, energy and envelope kernels with the variant's `cplxMultDiv2` and closed
forms of per-subband state; `fDivNorm` as one division (equal to the restoring division for
`num < denom`). arm64 cycles: HE-AAC v2 -14.8%.

### USAC, ELD and HE-AAC v2 decoder

`Pred_lt4`, `Syn_filt`, the bass post-filter and the autocorrelations in vector blocks where every
value a block reads precedes its first output (the minimum pitch lag guarantees it); the QMF
banks' small DCTs keep values in registers across the FFT.

### Encoder

Branch-free and NEON quantizer kernels with non-saturating forms (special values take the scalar
code); `fLog2` and `LdDataVector` checked for every input on every variant; downsampler, form
factor, PS downmix and SBR tonality in vector lanes; no per-frame heap allocation.

### x87 expressions (i686)

The x86_32 variant reproduces x87 extended-precision results (`crates/fdk/src/x87.rs`). The
default computes in f64 with a proven error bound and uses a software x87 model only in undecided
cases (checked for all 2^32 operands). This raised the i686 total from 0.37x to 0.95x.

### x86 SIMD paths

`fdk::simd` detects AVX2 at run time (`avx2()`, `avx2_clone!`); SSE2 is baseline. The DSP, SBR,
PS, TNS, arithmetic decoder and encoder kernels have x86 forms that reproduce the x86 arithmetic
exactly, e.g. `(x * w) >> 16` in 32-bit lanes as `(x >> 16) * w + (((x & 0xFFFF) * w) >> 16)`.

### x86-64-v3 build

`RUSTFLAGS='-C target-cpu=x86-64-v3'` uses AVX2/BMI2/LZCNT everywhere (no FMA). The output stays
identical (corpus with and without `FDK_AAC_RS_NO_SIMD`, exhaustive x87-expression comparison).

### Encoder multi-threading

`EncoderBuilder::threads(n)`, `fdk-aac-rs-enc -j N` or `FDK_AAC_RS_ENC_THREADS` (C API). A pool of
`min(threads, channels)` threads runs the per-channel quantization into scratch copies and the
per-element psychoacoustics; reductions stay sequential. arm64 enc lc384-5.1: 2.535 -> 1.163 s.

### Decoder frame pipelining

Stage B of frame N (inverse quantization to output) runs on a worker while stage A of frame N+1
(parsing) runs in the caller; each call returns the previous frame
([PORTING.md, C ABI](../PORTING.md#c-abi)). B's state lives in a shadow instance; frames that
cannot be handed over roll back and decode sequentially. `FDK_AAC_RS_POISON` verifies it.

### SBR encoder pipelining

The core of frame N-1 runs on the caller while a second thread runs call N up to its core; output
comes one call later. A failing or oversized core redoes the call sequentially from a snapshot.
arm64 enc he64: 0.998 -> 0.530 s at 4 threads.

### Per-channel parallelism in the decoder

Stereo SBR elements decode both channels on two threads; frames with 3 or more channels decode
elements and IMDCTs in parallel, with concealment and noise seeds kept in channel order. arm64
lc384-5.1: 0.513 -> 0.289 s at 3 threads.

### Chunked decoding of whole inputs

From `-j 4`, `fdk-aac-rs-dec` (`fdk_aac_rs::chunked`, safe API `decode_chunked`) decodes long
inputs in speculative chunks; a boundary is accepted only where a 128-bit state digest
(`crates/aacdec/src/digest.rs`) equals the true run's. SBR and USAC streams rarely converge and
fall back to sequential decoding. dec lc320 at -j 8: 10.6x (arm64).

### Thread mode selection

`fdk_aac_rs_fdk::par::tune` times each mode (sequential, per-channel, pipeline, combinations) per
frame and runs the cheapest within 3% of the fastest; every mode is exact, so the output does not
depend on the choice. `FDK_AAC_RS_TUNE` fixes it; `FDK_AAC_RS_TUNE_LOG` prints the decisions.

### Bit buffer memory

The bit buffers' memory is a reference-counted `BitMem` (see
[PORTING.md, Memory](../PORTING.md#memory)) storing bytes in atomic 64-bit words, so a 32-bit
read is two word loads. Against raw pointers the single-threaded total is +1.2% to +1.7% on arm64
(10.426 to 10.470 s against 10.300 s) and +0.7% on x86-64 (15.802 s against 15.693 s); atomic
bytes cost +1.4% and +1.5%.

## armv7

Static musl builds for `armv7-unknown-linux-musleabi` and `armv7-unknown-linux-musleabihf` link
with `rust-lld` (`CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER=rust-lld`). The port computes
the `aarch64` variant there
([PORTING.md, Architecture variants](../PORTING.md#architecture-variants)). On an ARMv8 core in
AArch32 mode, a stratified sample of 308 corpus entries matches the arm64 reference outputs in 616
of 616 files for all three builds (soft-float, hard-float, hard-float with
`-C target-feature=+neon`); the full corpus was not run. Without NEON the port is about as
fast as an armv7 build of the reference for AAC-LC (dec lc128 0.99x wall) and slower for SBR/PS
and the encoder, since its SIMD kernels target aarch64 and x86 only; with `+neon` LLVM vectorizes
the portable code (dec lc128 1.48x), but the binary then requires NEON.

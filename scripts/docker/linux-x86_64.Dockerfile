# Linux x86-64 environment for the ref-x86_64 reference build and for running the port on Linux
# (PORTING.md, "Reference builds"). On Apple silicon it runs under Docker Desktop's x86-64
# emulation (Rosetta 2).
FROM --platform=linux/amd64 debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
      clang cmake make git python3 unifdef binutils curl ca-certificates ffmpeg file pkg-config xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && git config --system --add safe.directory '*'
# Rust (a pinned stable toolchain).
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.88.0 \
    && chmod -R a+rwX /opt/rustup /opt/cargo
# 32-bit x86 (ref-i686, the x86_32 variant; PORTING.md, "Reference builds"): the i386 C and C++
# runtime for clang -m32, and the Rust target. A separate layer, so that the toolchain of the
# layers above (and with it ref-x86_64) stays as it was. Rust links i686 programs with clang
# (the target passes -m32).
RUN apt-get update && apt-get install -y --no-install-recommends \
      libc6-dev-i386 lib32stdc++-14-dev lib32gcc-14-dev \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add i686-unknown-linux-gnu \
    && chmod -R a+rwX /opt/rustup /opt/cargo
ENV CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER=clang
# perf for profiling (scripts/perf_profile.py; the container then needs --cap-add PERFMON, see
# scripts/linux_ref.sh).
RUN apt-get update && apt-get install -y --no-install-recommends linux-perf \
    && rm -rf /var/lib/apt/lists/*

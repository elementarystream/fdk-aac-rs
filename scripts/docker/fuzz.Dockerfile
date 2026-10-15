# Differential fuzzing environment (PORTING.md, "Differential fuzzing"; fuzz/README.md): Debian
# trixie as scripts/docker/linux-x86_64.Dockerfile, plus a nightly Rust toolchain, cargo-fuzz and
# clang.
# Built for linux/amd64 by default (the x86 reference variant); scripts/fuzz.sh can build it for
# linux/arm64 as well (FUZZ_PLATFORM). On Apple silicon the amd64 image runs under emulation.
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
      clang lld llvm make git python3 binutils curl ca-certificates file pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && git config --system --add safe.directory '*'
# Nightly Rust (cargo-fuzz needs -Z flags) and cargo-fuzz. NIGHTLY pins the toolchain date.
# cargo-fuzz is built with the stable toolchain of the Linux image: its locked dependencies
# (rustix) probe nightly features that current nightlies reject.
ARG NIGHTLY=nightly-2026-10-01
ARG STABLE=1.88.0
ARG CARGO_FUZZ=0.13.1
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$NIGHTLY" \
    && rustup toolchain install --profile minimal "$STABLE" \
    && cargo "+$STABLE" install --locked cargo-fuzz --version "$CARGO_FUZZ" \
    && rustup toolchain uninstall "$STABLE" \
    && rm -rf /opt/cargo/registry /opt/cargo/git \
    && chmod -R a+rwX /opt/rustup /opt/cargo

# Preserve the exact v2 Linux/Rust toolchain and base identity.
ARG PREDECESSOR_IMAGE=spaceship-bench-base:linux-rust-v2
FROM ${PREDECESSOR_IMAGE}

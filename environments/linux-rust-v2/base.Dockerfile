# Preserve the exact v1 base layers; no package resolution.
ARG PREDECESSOR_IMAGE=spaceship-bench-base:linux-rust-v1
FROM ${PREDECESSOR_IMAGE}

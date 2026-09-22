# Preserve pinned v1 packages, remove state created by root during CLI validation.
ARG PREDECESSOR_IMAGE=spaceship-bench-codex:linux-rust-v1
FROM ${PREDECESSOR_IMAGE}
USER root
RUN rm -rf /home/agent/.claude /home/agent/.codex /home/agent/.claude.json /home/agent/.cache \
    && test ! -e /workspace/Cargo.toml
LABEL com.docker.sandboxes.flavor="codex"
USER agent
WORKDIR /workspace

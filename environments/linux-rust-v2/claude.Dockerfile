# Preserve pinned v1 packages, remove state created by root during CLI validation.
ARG PREDECESSOR_IMAGE=spaceship-bench-claude:linux-rust-v1
FROM ${PREDECESSOR_IMAGE}
USER root
RUN rm -rf /home/agent/.claude /home/agent/.codex /home/agent/.claude.json /home/agent/.cache \
    && install -d -o agent -g agent -m 0700 /home/agent/.claude \
    && test ! -e /workspace/Cargo.toml
LABEL com.docker.sandboxes.flavor="claude"
USER agent
WORKDIR /workspace

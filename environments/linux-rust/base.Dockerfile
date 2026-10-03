# No task, starter code, instructions, credentials, caches or agent state are copied.
FROM docker.io/library/rust:1.97.0-slim-bookworm@sha256:6d220bf85c74e842a79da63997af8d2e74455c0b8847d8bb3a5888572334991d AS rust
FROM docker.io/docker/sandbox-templates:shell@sha256:da4458d89a5f739df50a2a334c08a3d15f15e11f0024bf47b37c88ad348a4644
USER root
COPY --from=rust /usr/local/rustup /opt/rustup
COPY --from=rust /usr/local/cargo /opt/cargo
# Preparation records distro package versions and the complete image identity.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential clang cmake pkg-config libssl-dev libncurses-dev \
    locales util-linux python3 tmux ca-certificates \
    && apt-get clean && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /workspace /replay /opt/bench \
    && chown agent:agent /workspace /replay \
    && dpkg-query -W -f='${Package}\t${Version}\n' > /opt/bench/native-packages.tsv \
    && rm -rf /home/agent/.cache /home/agent/.claude /home/agent/.codex \
    && test ! -e /workspace/Cargo.toml
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/home/agent/.cargo \
    PATH=/opt/cargo/bin:/home/agent/.local/bin:/usr/local/share/npm-global/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    LANG=C.UTF-8 LC_ALL=C.UTF-8 TERM=xterm-256color \
    CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse \
    DISABLE_AUTOUPDATER=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    CLAUDE_CODE_DISABLE_LEGACY_MODEL_REMAP=1
USER agent
WORKDIR /workspace
RUN rustc --version && cargo --version && cc --version && python3 -c 'import pty; a,b=pty.openpty()'

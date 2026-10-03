ARG BASE_IMAGE=terminal-game-taste-bench-base:linux-rust
FROM ${BASE_IMAGE}
ARG AGENT_VERSION=0.155.1
ARG AGENT_NPM_INTEGRITY
USER root
RUN test "$(npm view @openai/codex@${AGENT_VERSION} dist.integrity)" = "${AGENT_NPM_INTEGRITY}" \
    && npm install -g --ignore-scripts @openai/codex@${AGENT_VERSION} \
    && npm cache clean --force \
    && rm -rf /home/agent/.claude /home/agent/.codex /home/agent/.claude.json /home/agent/.cache \
    && test ! -e /workspace/Cargo.toml
LABEL com.docker.sandboxes.flavor="codex"
USER agent
WORKDIR /workspace

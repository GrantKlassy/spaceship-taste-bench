ARG BASE_IMAGE=spaceship-bench-base:linux-rust
FROM ${BASE_IMAGE}
ARG AGENT_VERSION=2.1.280
ARG AGENT_NPM_INTEGRITY
USER root
RUN test "$(npm view @anthropic-ai/claude-code@${AGENT_VERSION} dist.integrity)" = "${AGENT_NPM_INTEGRITY}" \
    && npm install -g @anthropic-ai/claude-code@${AGENT_VERSION} \
    && npm cache clean --force \
    && rm -rf /home/agent/.claude /home/agent/.codex /home/agent/.claude.json /home/agent/.cache \
    && install -d -o agent -g agent -m 0700 /home/agent/.claude \
    && test ! -e /workspace/Cargo.toml
LABEL com.docker.sandboxes.flavor="claude"
USER agent
WORKDIR /workspace

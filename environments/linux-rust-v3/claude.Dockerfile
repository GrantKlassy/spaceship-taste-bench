# Upgrade only Claude Code; preserve the v2 Linux/Rust packages.
ARG PREDECESSOR_IMAGE=spaceship-bench-claude:linux-rust-v2
FROM ${PREDECESSOR_IMAGE}
ARG CLAUDE_VERSION
ARG CLAUDE_NPM_INTEGRITY
USER root
RUN test "$(npm view @anthropic-ai/claude-code@${CLAUDE_VERSION} dist.integrity)" = "${CLAUDE_NPM_INTEGRITY}" \
    && npm install -g @anthropic-ai/claude-code@${CLAUDE_VERSION} \
    && npm cache clean --force \
    && rm -rf /home/agent/.claude /home/agent/.codex /home/agent/.claude.json /home/agent/.cache \
    && install -d -o agent -g agent -m 0700 /home/agent/.claude \
    && test ! -e /workspace/Cargo.toml
LABEL com.docker.sandboxes.flavor="claude"
USER agent
WORKDIR /workspace

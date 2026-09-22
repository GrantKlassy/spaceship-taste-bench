ARG BASE_IMAGE=spaceship-bench-base:linux-rust-v1
FROM ${BASE_IMAGE}
USER root
RUN npm install -g @anthropic-ai/claude-code@2.1.278 \
    && npm cache clean --force && claude --version
USER agent
WORKDIR /workspace

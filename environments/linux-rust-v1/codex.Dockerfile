ARG BASE_IMAGE=spaceship-bench-base:linux-rust-v1
FROM ${BASE_IMAGE}
USER root
RUN npm install -g --ignore-scripts @openai/codex@0.155.1 \
    && npm cache clean --force && codex --version
USER agent
WORKDIR /workspace

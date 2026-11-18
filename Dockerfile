# Etapas:
#   build    → compila nativamente na arquitetura do host (x86_64 no PC, arm64 na Oracle)
#   base     → sistema com as CLIs, sem o binário
#   pronto   → base + binário já compilado em dist/storyteller (scripts/subir.sh compila no PC e manda só ele)
#   final    → base + binário da etapa build (padrão: dev e quem compila no próprio servidor)
FROM rust:1-slim-bookworm AS build
WORKDIR /app
# cache das dependências
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release && rm -rf src target/release/deps/storyteller*
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
RUN touch src/main.rs && cargo build --release

FROM node:22-bookworm-slim AS base
# Usuário do container = dono da pasta de dados no host (padrão 1000:1000)
ARG APP_UID=1000
ARG APP_GID=1000
# CLIs de IA: claude, codex e agy (usados só pelas famílias com cli_habilitado). Versões fixas para builds reproduzíveis.
ARG CLAUDE_CODE_VERSION=2.1.325
ARG CODEX_VERSION=0.169.0
ARG EDGE_TTS_VERSION=7.3.1
# Gemini: Antigravity CLI (agy), que substituiu o Gemini CLI para Google One em 18/06/2026.
# Fica em /usr/local/bin porque /home/app é um volume (guarda os logins).
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl ffmpeg git python3 python3-pip \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION} @openai/codex@${CODEX_VERSION} \
    && npm cache clean --force \
    && pip3 install --no-cache-dir --break-system-packages edge-tts==${EDGE_TTS_VERSION} \
    && curl -fsSL https://antigravity.google/cli/install.sh -o /tmp/agy-install.sh \
    && bash /tmp/agy-install.sh && mv /root/.local/bin/agy /usr/local/bin/agy && rm -rf /tmp/agy-install.sh /root/.local \
    && userdel -r node \
    && groupadd -g ${APP_GID} app && useradd -m -u ${APP_UID} -g ${APP_GID} app && mkdir -p /data && chown app:app /data
WORKDIR /app
COPY static ./static
COPY scripts/st-tts.py /usr/local/bin/st-tts.py
ENV DATA_DIR=/data BIND_ADDR=0.0.0.0:8080 HOME=/home/app
EXPOSE 8080
CMD ["storyteller"]

FROM base AS pronto
COPY dist/storyteller /usr/local/bin/storyteller
USER app

FROM base AS final
COPY --from=build /app/target/release/storyteller /usr/local/bin/storyteller
USER app

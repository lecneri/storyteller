# Etapas:
#   build    → compila nativamente na arquitetura do host (x86_64 no PC, arm64 na Oracle)
#   base     → sistema (ffmpeg, edge-tts), sem o binário. Os CLIs de IA ficam no serviço ia-cli.
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

FROM debian:bookworm-slim AS base
# Usuário do container = dono da pasta de dados no host (padrão 1000:1000)
ARG APP_UID=1000
ARG APP_GID=1000
ARG EDGE_TTS_VERSION=7.3.1
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg python3 python3-pip \
    && rm -rf /var/lib/apt/lists/* \
    && pip3 install --no-cache-dir --break-system-packages edge-tts==${EDGE_TTS_VERSION} \
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

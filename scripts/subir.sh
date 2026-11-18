#!/usr/bin/env bash
# Sobe a versão do HEAD para o servidor: testa, compila aqui para ARM64 (o servidor tem 2 núcleos e demora),
# manda os arquivos + o binário e o servidor só monta a imagem (etapa "pronto" do Dockerfile).
# Configuração em .deploy.env (fora do git):
#   SERVIDOR=usuario@host   CHAVE=~/.ssh/minha_chave   PASTA=/caminho/no/servidor   URL=https://site
# Opções: --sem-testes
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .deploy.env ] || { echo "Falta o .deploy.env (veja o começo deste script)"; exit 1; }
# shellcheck disable=SC1091
source .deploy.env
SSH=(ssh -i "${CHAVE/#\~/$HOME}" "$SERVIDOR")

if [ -n "$(git status --porcelain)" ]; then
  echo "⚠️  Há mudanças sem commit: sobe só o que está no HEAD ($(git log --oneline -1))."
fi

if [ "${1:-}" != "--sem-testes" ]; then
  echo "== Testes =="
  docker run --rm -v "$PWD":/app -v storyteller-cargo:/usr/local/cargo/registry -v storyteller-target:/app/target -w /app \
    rust:1-slim-bookworm cargo test -q --release
  node --test tests/js/*.test.js >/dev/null
fi

echo "== Compilando para ARM64 =="
docker build -q -t storyteller-arm64 -f docker/arm64.Dockerfile docker >/dev/null
rm -rf dist && mkdir -p dist
# Compila a partir do HEAD (o mesmo que vai para o servidor), não da pasta de trabalho.
FONTE=$(mktemp -d)
trap 'rm -rf "$FONTE"' EXIT
git archive HEAD | tar -x -C "$FONTE"
docker run --rm -v "$FONTE":/app -v "$PWD/dist":/saida -v storyteller-cargo:/usr/local/cargo/registry \
  -v storyteller-target-arm64:/app/target -w /app storyteller-arm64 \
  sh -c "cargo build -q --release --target aarch64-unknown-linux-gnu && cp target/aarch64-unknown-linux-gnu/release/storyteller /saida/ && chown $(id -u):$(id -g) /saida/storyteller"
ls -lh dist/storyteller

echo "== Enviando =="
git archive HEAD | "${SSH[@]}" "cd '$PASTA' && tar -x && mkdir -p dist"
scp -q -i "${CHAVE/#\~/$HOME}" dist/storyteller "$SERVIDOR:$PASTA/dist/storyteller"

echo "== Subindo =="
"${SSH[@]}" "cd '$PASTA' && docker compose -f docker-compose.prod.yml up -d --build 2>&1 | tail -3"
for _ in $(seq 1 60); do
  if curl -sf "$URL/saude" >/dev/null; then
    echo "✅ No ar: $(curl -s "$URL/saude")"
    "${SSH[@]}" "docker logs storyteller --since 2m 2>&1 | grep -iE 'migra|error|panic' | tail -3" || true
    exit 0
  fi
  sleep 3
done
echo "❌ O site não respondeu em 3 minutos"; exit 1

#!/usr/bin/env sh
# Roda todos os testes: Rust (dentro do Docker, sem precisar de Rust instalado) e JavaScript (node --test).
set -e
cd "$(dirname "$0")/.."
echo "== Rust =="
docker run --rm -v "$PWD":/app -v st-cargo:/usr/local/cargo/registry -w /app rust:1-slim-bookworm cargo test --release
echo "== JavaScript =="
node --test tests/js/*.test.js

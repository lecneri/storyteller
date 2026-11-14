# 📚 Storyteller

Histórias infantis em formato de livro ilustrado (imagem | texto), com narração por personagem, animações e sons.
Rust (axum) + Postgres 18 (pgvector) + Docker. Histórias geradas por IA (texto, imagens e narração), com personagens, estantes, famílias e compartilhamento.

## Rodar no PC (desenvolvimento)

```bash
cp .env.example .env        # ajuste senhas, CHAVE_MESTRA (openssl rand -base64 32), ADMIN_*
docker compose --profile dev up --build -d
# http://localhost:8090  (APP_BIND_IP=0.0.0.0 para acessar pela rede/VPN)
```

Testes: `docker run --rm -v "$PWD":/app -w /app rust:1-slim-bookworm cargo test --release`

## Produção (VPS Oracle, atrás do Caddy)

1. Na VPS, em `/opt/docker/container/storyteller`: código do projeto + `.env.db` (já existe) + `.env` (modelo mínimo em `.env.producao.example`).
2. Pasta de dados: `mkdir -p /opt/docker/volumes/storyteller_data && chown 1000:1000 /opt/docker/volumes/storyteller_data`.
3. `docker compose -f docker-compose.prod.yml up -d --build` (redes externas `db_net` e `proxy_net`; container `storyteller`, porta 8080).
4. Caddy: `historias.seudominio.com.br { reverse_proxy storyteller:8080 }`. DNS na Cloudflare.
5. Abra o site: no primeiro acesso aparece **/instalar** para criar o admin e a família.
6. Em **⚙️ Sistema**, sem SSH: endereço público, conectar as assinaturas (Claude, ChatGPT/Codex, Gemini/agy), chaves de API, modelos e e-mail (com botão de teste).
7. Backup: o banco já tem backup; a pasta de dados (mídias **e** `segredos/chave-mestra`) também precisa entrar — restaurar os dois juntos.

## Onde fica cada coisa

- `src/` — servidor (rotas por assunto: `historias`, `personagens`, `estantes`, `player`, `narracao`, `partilha`, `familia`, `sistema`…)
- `migrations/` — schemas do Postgres (`acesso`, `conteudo`, `partilha`, `midia`, `ia`, `jobs`)
- `templates/` — páginas (Askama) · `static/` — CSS/JS (editor, player, efeitos, vozes)
- `scripts/st-tts.py` — narração (edge-tts)

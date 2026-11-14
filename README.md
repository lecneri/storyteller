# 📚 Storyteller

Crie histórias infantis com IA no formato de **livro ilustrado**: em cada página dupla, uma imagem de um lado e o texto do outro. A história pode ser **narrada**, com uma voz diferente para cada personagem, e as páginas viram sozinhas, com animações e sons.

Foi feito para famílias: cada uma tem os próprios personagens, estantes e histórias, e pode compartilhar com outras.

> Projeto pessoal, em português do Brasil. Roda em Docker, com backend em Rust e Postgres.

---

## ✨ O que ele faz

**Criar histórias**
- **Briefing → IA:** escreva a ideia ("o @leo e o Tito vão à lua…") e a IA escreve a história em páginas.
- **`@` cita personagens** (com autocompletar e variante: `@leo:super-rangers`). **`#` cita histórias anteriores**: a IA recebe um resumo ("memória") e continua a série.
- **Editor de blocos:** narrador e falas de cada personagem, páginas inseridas em qualquer posição e tudo editável. Também dá para criar a história à mão, sem IA.
- **Kit da imagem** em cada página: prompt pronto, referências dos personagens para copiar e colar no ChatGPT/Gemini, e uma área para arrastar o resultado (imagem, GIF ou vídeo). Com uma chave de API, a imagem é gerada direto pelo app.

**Personagens**
- Criados à mão ou **com IA** a partir de uma descrição e/ou fotos (a IA gera a ficha e o desenho de referência).
- **Variantes** (forma × estilo): "Léo normal", "Super Léo no estilo tokusatsu"… Variantes antigas podem ser arquivadas.
- **Voz por personagem:** escolha da voz, tom e velocidade, com prévia.

**Ler e ouvir**
- **Player de livro:** abre em modo leitura. O ▶ narra, destaca a palavra que está sendo lida (karaokê) e vira a página. No celular em pé, vira uma coluna.
- **Animações e sons** por página (estrelas, chuva, neve, bolhas, tremor, explosão "KABUM!"…), sintetizados no navegador, sem arquivos de áudio.
- **Modo leitura para crianças:** mostra só as estantes delas e o player. Para sair, é preciso o PIN dos pais.

**Famílias**
- Login por família, com papéis (admin e membro), convite por link ou e-mail, e várias crianças (leitores) com faixa etária.
- **Estantes** agrupam histórias e guardam os padrões (personagens, variantes, estilo).
- **Compartilhamento** entre famílias conectadas (o que é definido no item vence a estante), **link público de leitura** e "copiar para minha família".
- **Privacidade:** baixar meus dados, apagar conta, exportar tudo em Markdown e limpeza automática de arquivos sem uso.

---

## 🧱 Como funciona

| Parte | Tecnologia |
|---|---|
| Servidor | Rust · axum · sqlx · askama |
| Banco | Postgres 18 (imagem `pgvector/pgvector:pg18`), schemas `acesso`, `conteudo`, `partilha`, `midia`, `ia`, `jobs` |
| Fila de tarefas | no próprio Postgres (`FOR UPDATE SKIP LOCKED`) |
| Texto (IA) | CLIs de assinatura (Claude Code, Antigravity `agy`) **ou** APIs (Anthropic, Gemini, OpenAI) |
| Imagem (IA) | OpenAI (gpt-image) ou Gemini, ou modo manual (kit de prompt) |
| Narração | [edge-tts](https://github.com/rany2/edge-tts) (gratuito), com o tempo de cada palavra para o karaokê |
| Frontend | HTML gerado no servidor + JavaScript sem framework (editor, player, efeitos) |

**Ordem de escolha da IA** para cada família: assinatura (CLI, se liberada pelo admin) → chave de API da família → chave do sistema (se o admin liberar, com limite semanal) → chave grátis do Gemini → modo manual.

Os segredos (chaves de API, tokens, senha do e-mail) ficam **criptografados no banco** (AES-256-GCM).

---

## 🚀 Rodando

Requisitos: Docker com Compose.

```bash
git clone https://github.com/<você>/storyteller.git
cd storyteller
cp .env.example .env            # ajuste a senha do Postgres
docker compose --profile dev up --build -d
```

Abra **http://localhost:8090**. No primeiro acesso aparece **/instalar**, onde você cria o admin e a sua família.

Todo o resto se configura **pelo site**, em **⚙️ Sistema**:
- **Assinaturas:** botão "Conectar" para Claude, ChatGPT (Codex) e Gemini (agy). O site mostra o link de login e você cola o código, sem precisar de terminal.
- Chaves de API, modelos, endereço público e e-mail (SMTP, com botão de teste).
- Famílias: quem usa as assinaturas, quem usa a chave do sistema e os limites.

### Produção

O `docker-compose.prod.yml` usa um Postgres e um proxy reverso que já existem (redes Docker externas). Ajuste os nomes das redes para o seu ambiente e veja `.env.producao.example`: o `.env` de produção é mínimo.

```bash
docker compose -f docker-compose.prod.yml up -d --build
```

- O container roda como o usuário `APP_UID:APP_GID` (padrão `1000:1000`). A pasta de dados precisa ter esse dono.
- **Backup:** faça backup do banco **e** da pasta de dados. A pasta guarda as mídias e o arquivo `segredos/chave-mestra`, sem o qual os segredos salvos não podem ser lidos.
- Funciona em x86_64 e arm64.

### Testes

```bash
./scripts/testar.sh   # Rust (no Docker) + JavaScript (node --test)
```

Os testes cobrem as regras sem banco: detecção de `@`/`#` no briefing, schema e prompt da IA, ajuste da
resposta da IA, vozes (chaves, extras, vozes do sistema), "virar variante", leitura das respostas dos CLIs,
login dos CLIs pelo site, tipos de mídia, exportação e as regras do editor (`static/editor-logica.js`).

---

## 📁 Estrutura

```
src/          rotas e regras; módulos maiores em pastas:
              historias/ (conteudo, deteccao, prompt, geracao, memoria, rotas)
              personagens/ (paginas, variantes, com_ia, juntar)
              ia/ (escolha da rota, cli = assinaturas, api = chaves)
migrations/   esquema do banco (sqlx)
templates/    páginas (askama)
static/       CSS e JS: comum.js (helpers), editor + editor-logica.js, player, efeitos/sons, vozes, briefing
tests/js/     testes do JavaScript (node --test)
scripts/      st-tts.py (narração com edge-tts), testar.sh
```

---

## 🔒 Privacidade e segurança

- **Fotos de crianças** só são usadas com autorização explícita, para gerar o desenho de referência. Depois disso, as histórias usam apenas o desenho.
- **CLIs isolados:** rodam sem ferramentas, num diretório vazio e com o ambiente limpo, porque o briefing vem do usuário.
- **Links públicos:** usam só vozes genéricas, podem ser cancelados e têm `noindex`.
- **Modo leitura:** a sessão fica travada no servidor, e para sair é preciso o PIN.
- **Senhas:** argon2. Tokens de sessão, convite e link são guardados só como hash.

---

## ⚠️ Avisos

- As assinaturas pessoais (Claude, ChatGPT, Google) são para uso pessoal. Compartilhar o acesso com outras pessoas pode violar os termos desses serviços. Por isso o uso de CLI é liberado **por família**, pelo admin.
- O edge-tts usa um serviço não oficial da Microsoft e pode deixar de funcionar.
- Os nomes de marcas usados como "estilo de referência" (ex.: um desenho animado famoso) servem só para orientar a IA. Este projeto não tem relação com essas marcas.

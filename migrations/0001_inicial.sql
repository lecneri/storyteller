-- Estrutura inicial: um schema por parte do sistema (ver PLANO.md, seção 2.0a)

CREATE SCHEMA IF NOT EXISTS acesso;
CREATE SCHEMA IF NOT EXISTS conteudo;
CREATE SCHEMA IF NOT EXISTS partilha;
CREATE SCHEMA IF NOT EXISTS midia;
CREATE SCHEMA IF NOT EXISTS ia;
CREATE SCHEMA IF NOT EXISTS jobs;

-- ───────────── acesso ─────────────

CREATE TABLE acesso.usuarios (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email         text NOT NULL,
    nome          text NOT NULL,
    senha_hash    text NOT NULL,
    admin_sistema boolean NOT NULL DEFAULT false,
    criado_em     timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX usuarios_email_uk ON acesso.usuarios (lower(email));

CREATE TABLE acesso.familias (
    id                    uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug                  text NOT NULL UNIQUE,          -- nome único normalizado (@familia-neri)
    nome                  text NOT NULL,
    visibilidade_padrao   text NOT NULL DEFAULT 'privado'
                          CHECK (visibilidade_padrao IN ('privado', 'compartilhado')),
    pin_leitura_hash      text,                          -- PIN para sair do modo leitura
    config                jsonb NOT NULL DEFAULT '{}',   -- vozes padrão etc.
    criado_em             timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE acesso.membros (
    familia_id  uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    usuario_id  uuid NOT NULL REFERENCES acesso.usuarios (id) ON DELETE CASCADE,
    papel       text NOT NULL CHECK (papel IN ('admin', 'membro')),
    criado_em   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (familia_id, usuario_id)
);

CREATE TABLE acesso.sessoes (
    token_hash  text PRIMARY KEY,
    usuario_id  uuid NOT NULL REFERENCES acesso.usuarios (id) ON DELETE CASCADE,
    familia_id  uuid REFERENCES acesso.familias (id) ON DELETE CASCADE,
    expira_em   timestamptz NOT NULL,
    criado_em   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE acesso.leitores (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    familia_id  uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    slug        text NOT NULL,
    nome        text NOT NULL,
    faixa       text NOT NULL CHECK (faixa IN ('2-3', '4-6', '7-9', '10-12')),
    perfil      jsonb NOT NULL DEFAULT '{}',   -- interesses, favoritos, estantes visíveis
    criado_em   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (familia_id, slug)
);

-- ───────────── midia ─────────────

CREATE TABLE midia.arquivos (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    familia_id  uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    caminho     text NOT NULL,          -- relativo a DATA_DIR
    tipo_mime   text NOT NULL,
    bytes       bigint NOT NULL,
    sha256      text NOT NULL,
    criado_em   timestamptz NOT NULL DEFAULT now()
);

-- ───────────── conteudo ─────────────

CREATE TABLE conteudo.personagens (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    familia_id      uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    slug            text NOT NULL,
    nome            text NOT NULL,
    tipo            text NOT NULL DEFAULT 'crianca'
                    CHECK (tipo IN ('crianca', 'animal', 'robo', 'vilao', 'adulto', 'criatura')),
    ficha           jsonb NOT NULL DEFAULT '{}',   -- aparência, personalidade, jeito de falar, apelidos
    voz_padrao      jsonb NOT NULL DEFAULT '{}',
    variante_atual  uuid,                          -- FK adicionada abaixo
    visibilidade    text NOT NULL DEFAULT 'herdar'
                    CHECK (visibilidade IN ('herdar', 'privado', 'compartilhado', 'publico')),
    criado_em       timestamptz NOT NULL DEFAULT now(),
    atualizado_em   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (familia_id, slug)
);

CREATE TABLE conteudo.variantes (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    personagem_id   uuid NOT NULL REFERENCES conteudo.personagens (id) ON DELETE CASCADE,
    slug            text NOT NULL,
    nome            text NOT NULL,
    forma           text,                 -- normal, super, gigante...
    estilo          text,                 -- "Power Rangers", "Cavaleiros do Zodíaco"...
    descricao       text,
    voz             jsonb,                -- opcional: voz própria da variante
    referencias     uuid[] NOT NULL DEFAULT '{}',  -- midia.arquivos
    criado_em       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (personagem_id, slug)
);

ALTER TABLE conteudo.personagens
    ADD CONSTRAINT personagens_variante_atual_fk
    FOREIGN KEY (variante_atual) REFERENCES conteudo.variantes (id) ON DELETE SET NULL;

CREATE TABLE conteudo.estantes (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    familia_id      uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    slug            text NOT NULL,
    nome            text NOT NULL,
    padroes         jsonb NOT NULL DEFAULT '{}',   -- personagens, variantes, estilo
    capa            uuid REFERENCES midia.arquivos (id) ON DELETE SET NULL,
    visibilidade    text NOT NULL DEFAULT 'herdar'
                    CHECK (visibilidade IN ('herdar', 'privado', 'compartilhado', 'publico')),
    ordem           int NOT NULL DEFAULT 0,
    criado_em       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (familia_id, slug)
);

CREATE TABLE conteudo.historias (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    familia_id          uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    estante_principal   uuid REFERENCES conteudo.estantes (id) ON DELETE SET NULL,
    slug                text NOT NULL,
    titulo              text NOT NULL,
    faixa               text NOT NULL CHECK (faixa IN ('2-3', '4-6', '7-9', '10-12')),
    narracao            text NOT NULL DEFAULT 'por-personagem'
                        CHECK (narracao IN ('narrador', 'por-personagem')),
    briefing            text,
    estado              text NOT NULL DEFAULT 'rascunho'
                        CHECK (estado IN ('rascunho', 'texto', 'imagens', 'narracao', 'pronta')),
    conteudo            jsonb NOT NULL DEFAULT '{"paginas": []}',  -- páginas e blocos
    memoria             text,                                     -- resumo usado pelo #
    capa                uuid REFERENCES midia.arquivos (id) ON DELETE SET NULL,
    visibilidade        text NOT NULL DEFAULT 'herdar'
                        CHECK (visibilidade IN ('herdar', 'privado', 'compartilhado', 'publico')),
    criado_por          uuid REFERENCES acesso.usuarios (id) ON DELETE SET NULL,
    criado_em           timestamptz NOT NULL DEFAULT now(),
    atualizado_em       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (familia_id, slug)
);

CREATE TABLE conteudo.estante_historias (
    estante_id  uuid NOT NULL REFERENCES conteudo.estantes (id) ON DELETE CASCADE,
    historia_id uuid NOT NULL REFERENCES conteudo.historias (id) ON DELETE CASCADE,
    ordem       int NOT NULL DEFAULT 0,
    PRIMARY KEY (estante_id, historia_id)
);

CREATE TABLE conteudo.versoes (
    id          bigserial PRIMARY KEY,
    historia_id uuid NOT NULL REFERENCES conteudo.historias (id) ON DELETE CASCADE,
    conteudo    jsonb NOT NULL,
    criado_por  uuid REFERENCES acesso.usuarios (id) ON DELETE SET NULL,
    criado_em   timestamptz NOT NULL DEFAULT now()
);

-- ───────────── ia ─────────────

CREATE TABLE ia.config_familia (
    familia_id          uuid PRIMARY KEY REFERENCES acesso.familias (id) ON DELETE CASCADE,
    cli_habilitado      boolean NOT NULL DEFAULT false,
    usar_chave_sistema  boolean NOT NULL DEFAULT false,
    limites             jsonb NOT NULL DEFAULT '{}'
);

-- ───────────── jobs ─────────────

CREATE TABLE jobs.fila (
    id              bigserial PRIMARY KEY,
    tipo            text NOT NULL,          -- gerar_texto, gerar_imagem, gerar_audio...
    familia_id      uuid REFERENCES acesso.familias (id) ON DELETE CASCADE,
    payload         jsonb NOT NULL DEFAULT '{}',
    estado          text NOT NULL DEFAULT 'pendente'
                    CHECK (estado IN ('pendente', 'rodando', 'feito', 'erro')),
    tentativas      int NOT NULL DEFAULT 0,
    erro            text,
    resultado       jsonb,
    agendado_para   timestamptz NOT NULL DEFAULT now(),
    criado_em       timestamptz NOT NULL DEFAULT now(),
    atualizado_em   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX fila_pendentes_idx ON jobs.fila (agendado_para) WHERE estado = 'pendente';

-- partilha: tabelas de conexões e permissões entram numa migração futura.

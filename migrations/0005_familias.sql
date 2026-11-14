-- Convites de membros (link com token de uso único) e chaves de API criptografadas.
CREATE TABLE acesso.convites (
    token_hash  text PRIMARY KEY,
    familia_id  uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    papel       text NOT NULL CHECK (papel IN ('admin', 'membro')),
    criado_por  uuid REFERENCES acesso.usuarios (id) ON DELETE SET NULL,
    expira_em   timestamptz NOT NULL,
    usado_em    timestamptz,
    usado_por   uuid REFERENCES acesso.usuarios (id) ON DELETE SET NULL,
    criado_em   timestamptz NOT NULL DEFAULT now()
);

-- Chaves de API: da família (familia_id preenchido) ou do sistema (familia_id nulo).
CREATE TABLE ia.chaves (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    familia_id  uuid REFERENCES acesso.familias (id) ON DELETE CASCADE,
    provedor    text NOT NULL CHECK (provedor IN ('anthropic', 'openai', 'gemini')),
    cifrada     bytea NOT NULL,      -- AES-256-GCM (nonce || texto cifrado)
    final       text NOT NULL,       -- últimos 4 caracteres, para mostrar na tela
    criado_em   timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX chaves_familia_provedor_uk ON ia.chaves (COALESCE(familia_id, '00000000-0000-0000-0000-000000000000'::uuid), provedor);

-- Registro de uso de IA (quem gastou o quê e por qual caminho).
CREATE TABLE ia.uso (
    id          bigserial PRIMARY KEY,
    familia_id  uuid REFERENCES acesso.familias (id) ON DELETE CASCADE,
    tarefa      text NOT NULL,       -- texto, imagem...
    via         text NOT NULL,       -- cli, chave_familia, chave_sistema, gratis
    provedor    text NOT NULL,
    criado_em   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX uso_familia_data_idx ON ia.uso (familia_id, criado_em);

-- Conexões entre famílias (convite pelo nome único + aceite).
CREATE TABLE partilha.conexoes (
    de          uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    para        uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    estado      text NOT NULL DEFAULT 'pendente' CHECK (estado IN ('pendente', 'aceita')),
    criado_em   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (de, para),
    CHECK (de <> para)
);

-- Compartilhamento com famílias específicas. Sem linhas para um item "compartilhado" = todas as conectadas.
CREATE TABLE partilha.destinos (
    item_tipo   text NOT NULL CHECK (item_tipo IN ('historia', 'estante', 'personagem')),
    item_id     uuid NOT NULL,
    familia_id  uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    PRIMARY KEY (item_tipo, item_id, familia_id)
);

-- Links públicos de leitura (história ou estante).
CREATE TABLE partilha.links (
    token_hash      text PRIMARY KEY,
    item_tipo       text NOT NULL CHECK (item_tipo IN ('historia', 'estante')),
    item_id         uuid NOT NULL,
    familia_id      uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    expira_em       timestamptz,
    revogado        boolean NOT NULL DEFAULT false,
    criado_em       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX links_item_idx ON partilha.links (item_tipo, item_id);

-- Configurações do sistema editadas pelo site (o que antes ficava no .env).
CREATE TABLE ia.config_sistema (
    chave       text PRIMARY KEY,
    valor       text,                -- texto simples (configurações comuns)
    cifrado     bytea,               -- segredos (AES-GCM), quando secreto
    atualizado_em timestamptz NOT NULL DEFAULT now()
);

-- Recuperação de senha por e-mail e preferência de notificações.
CREATE TABLE acesso.redefinicoes (
    token_hash  text PRIMARY KEY,
    usuario_id  uuid NOT NULL REFERENCES acesso.usuarios (id) ON DELETE CASCADE,
    expira_em   timestamptz NOT NULL,
    usado_em    timestamptz
);
ALTER TABLE acesso.usuarios ADD COLUMN notificacoes boolean NOT NULL DEFAULT true;

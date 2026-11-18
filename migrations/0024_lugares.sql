-- Lugares da família (cenários fixos: o castelo, a casa da vovó...). Citados no briefing com @lugar:slug.
CREATE TABLE conteudo.lugares (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    familia_id    uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    slug          text NOT NULL,
    nome          text NOT NULL,
    -- O que nunca muda no lugar (formato, cores, objetos marcantes). Luz, hora e clima vêm da cena.
    descricao     text NOT NULL DEFAULT '',
    referencias   uuid[] NOT NULL DEFAULT '{}',
    criado_em     timestamptz NOT NULL DEFAULT now(),
    atualizado_em timestamptz NOT NULL DEFAULT now(),
    UNIQUE (familia_id, slug)
);

-- Fila de imagens: desenho de referência de um lugar.
ALTER TABLE jobs.imagens ADD COLUMN lugar_id uuid REFERENCES conteudo.lugares (id) ON DELETE CASCADE;
ALTER TABLE jobs.imagens DROP CONSTRAINT IF EXISTS imagens_alvo_check;
ALTER TABLE jobs.imagens ADD CONSTRAINT imagens_alvo_check CHECK (alvo IN ('pagina', 'capa', 'variante', 'lugar'));

-- Respostas de outra IA coladas que não deu para importar: texto, erro e onde (para investigar). Só as últimas.
CREATE TABLE jobs.importacoes_erro (
    id bigserial PRIMARY KEY,
    familia_id uuid NOT NULL REFERENCES acesso.familias(id) ON DELETE CASCADE,
    historia_id uuid,
    tipo text NOT NULL,
    texto text NOT NULL,
    erro text NOT NULL,
    criado_em timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX importacoes_erro_familia ON jobs.importacoes_erro (familia_id, criado_em DESC);

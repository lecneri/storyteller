-- Fila de imagens: todo pedido de imagem (página, capa, variante) vira um item aqui. Cada fonte (codex, agy, api)
-- tem um trabalhador que desenha um item por vez; as fontes trabalham em paralelo.
CREATE TABLE jobs.imagens (
    id            bigserial PRIMARY KEY,
    familia_id    uuid NOT NULL REFERENCES acesso.familias(id) ON DELETE CASCADE,
    familia_slug  text NOT NULL,
    alvo          text NOT NULL CHECK (alvo IN ('pagina', 'capa', 'variante')),
    historia_id   uuid REFERENCES conteudo.historias(id) ON DELETE CASCADE,
    chave         text,
    variante_id   uuid REFERENCES conteudo.variantes(id) ON DELETE CASCADE,
    lote          uuid,
    prompt        text NOT NULL,
    refs          uuid[] NOT NULL DEFAULT '{}',
    pasta         text NOT NULL,
    recortar      boolean NOT NULL DEFAULT false,
    -- 1 = pedido avulso (passa na frente das páginas de um lote).
    prioridade    int NOT NULL DEFAULT 0,
    fila          text NOT NULL,
    tentadas      text[] NOT NULL DEFAULT '{}',
    estado        text NOT NULL DEFAULT 'pendente' CHECK (estado IN ('pendente', 'rodando', 'feito', 'erro', 'cancelado')),
    feita_por     text,
    midia         uuid,
    midia_tipo    text,
    erro          text,
    criado_em     timestamptz NOT NULL DEFAULT now(),
    atualizado_em timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX imagens_pendentes ON jobs.imagens (fila, prioridade DESC, id) WHERE estado = 'pendente';
CREATE INDEX imagens_historia ON jobs.imagens (historia_id);
CREATE INDEX imagens_variante ON jobs.imagens (variante_id);

-- Fonte que desenha as imagens da história (a primeira que desenhou fica; dá para trocar no editor).
ALTER TABLE conteudo.historias ADD COLUMN fonte_imagem text;

-- Lotes do jeito antigo que ainda estivessem na fila.
UPDATE jobs.fila SET estado = 'erro', erro = 'substituído pela fila de imagens', atualizado_em = now()
 WHERE tipo = 'gerar_imagens' AND estado IN ('pendente', 'rodando');

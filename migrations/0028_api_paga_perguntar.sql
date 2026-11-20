-- Antes de cair na API paga (quando as assinaturas falham), a imagem espera a família decidir.
ALTER TABLE jobs.imagens DROP CONSTRAINT IF EXISTS imagens_estado_check;
ALTER TABLE jobs.imagens ADD CONSTRAINT imagens_estado_check
    CHECK (estado IN ('pendente', 'rodando', 'feito', 'erro', 'cancelado', 'aguardando'));
ALTER TABLE jobs.imagens ADD COLUMN pago_ok boolean NOT NULL DEFAULT false;
-- Família que prefere seguir para a API paga sem perguntar (o jeito antigo).
ALTER TABLE ia.config_familia ADD COLUMN pagar_sem_perguntar boolean NOT NULL DEFAULT false;

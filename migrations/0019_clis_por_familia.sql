-- Cada família escolhe quais assinaturas (CLIs) pode usar, em ordem de preferência (claude, codex, agy).
-- Quem tinha "CLI ligado" fica com o CLI que era o geral (CLI_TEXTO; padrão claude).
ALTER TABLE ia.config_familia ADD COLUMN IF NOT EXISTS clis text[] NOT NULL DEFAULT '{}';

UPDATE ia.config_familia
   SET clis = ARRAY[COALESCE(NULLIF((SELECT valor FROM ia.config_sistema WHERE chave = 'CLI_TEXTO'), ''), 'claude')]
 WHERE cli_habilitado AND cardinality(clis) = 0;

-- Como escolher entre as CLIs: ordem (a primeira; se falhar, a próxima), rodizio (alterna a cada pedido) ou fixo (só a primeira).
ALTER TABLE ia.config_familia ADD COLUMN IF NOT EXISTS modo_cli text NOT NULL DEFAULT 'ordem'
    CHECK (modo_cli IN ('ordem', 'rodizio', 'fixo'));

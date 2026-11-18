-- Imagens pela assinatura (CLIs que geram imagem: codex, agy), em ordem de preferência.
ALTER TABLE ia.config_familia ADD COLUMN IF NOT EXISTS clis_imagem text[] NOT NULL DEFAULT '{}';
-- Ordem das fontes de IA da família (texto e imagem): cli = assinaturas, familia = chave da família,
-- sistema = chave do sistema (se liberada). A chave grátis do Gemini (texto) fica sempre por último.
ALTER TABLE ia.config_familia ADD COLUMN IF NOT EXISTS ordem_fontes text[] NOT NULL DEFAULT '{cli,familia,sistema}';

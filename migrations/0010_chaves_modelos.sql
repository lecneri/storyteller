-- Cada chave de API escolhe o seu modelo (texto e, quando houver, imagem). Novo provedor: Kimi (Moonshot).
ALTER TABLE ia.chaves DROP CONSTRAINT chaves_provedor_check;
ALTER TABLE ia.chaves ADD CONSTRAINT chaves_provedor_check CHECK (provedor IN ('anthropic', 'openai', 'gemini', 'kimi'));
ALTER TABLE ia.chaves ADD COLUMN modelo text;
ALTER TABLE ia.chaves ADD COLUMN modelo_imagem text;

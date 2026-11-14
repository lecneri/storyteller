-- ElevenLabs (vozes da narração): cada família pode cadastrar a sua chave.
ALTER TABLE ia.chaves DROP CONSTRAINT chaves_provedor_check;
ALTER TABLE ia.chaves ADD CONSTRAINT chaves_provedor_check CHECK (provedor IN ('anthropic', 'openai', 'gemini', 'kimi', 'elevenlabs'));

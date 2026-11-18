-- Modelo e esforço escolhidos por CLI, por família: {"claude": {"modelo": "opus", "esforco": "high"}, ...}. Vazio = padrão da CLI.
ALTER TABLE ia.config_familia ADD COLUMN cli_ajustes jsonb NOT NULL DEFAULT '{}';
-- Modelo usado de fato em cada chamada (quando a CLI ou a API informa).
ALTER TABLE ia.uso ADD COLUMN modelo text;

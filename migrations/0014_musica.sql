-- Música de fundo opcional da história: toca em loop no leitor e no vídeo, abaixando quando a narração fala.
ALTER TABLE conteudo.historias
    ADD COLUMN musica uuid REFERENCES midia.arquivos (id) ON DELETE SET NULL,
    ADD COLUMN musica_volume real NOT NULL DEFAULT 0.35;

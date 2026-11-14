-- Histórias citadas com # (continuação / série).
ALTER TABLE conteudo.historias ADD COLUMN continua_de uuid[] NOT NULL DEFAULT '{}';

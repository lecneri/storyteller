-- Versão do conteúdo da história: sobe a cada escrita feita por gente (salvar no editor, gerar de novo,
-- renomear ou juntar personagem). O editor manda a versão que abriu; se mudou, o salvar avisa em vez de
-- passar por cima do que outra aba ou outra pessoa salvou. A fila de imagens não mexe na versão.
ALTER TABLE conteudo.historias ADD COLUMN versao integer NOT NULL DEFAULT 0;

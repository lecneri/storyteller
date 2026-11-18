-- Quantas páginas a família pediu ao criar a história (NULL = a IA escolhe pela idade).
ALTER TABLE conteudo.historias ADD COLUMN paginas_pedidas int CHECK (paginas_pedidas BETWEEN 4 AND 30);

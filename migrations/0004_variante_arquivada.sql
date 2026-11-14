-- Variante arquivada: some das opções para histórias novas; as histórias antigas continuam iguais.
ALTER TABLE conteudo.variantes ADD COLUMN arquivada boolean NOT NULL DEFAULT false;

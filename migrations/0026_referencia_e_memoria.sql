-- A "memória" antiga (resumo usado quando outra história cita com #) vira "referência".
-- "Memória" agora é uma lista de marcações (ex.: palavras em inglês aprendidas) que a IA grava quando o pedido manda.
ALTER TABLE conteudo.historias RENAME COLUMN memoria TO referencia;
ALTER TABLE conteudo.historias ADD COLUMN memoria jsonb NOT NULL DEFAULT '[]';
-- Marcado na nova história: a geração recebe a memória das histórias da mesma estante.
ALTER TABLE conteudo.historias ADD COLUMN usar_memoria_estante boolean NOT NULL DEFAULT false;

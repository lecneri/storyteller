-- Memória separada do briefing na nova história: o que gravar e o que usar (#estante, #historia, #todas).
ALTER TABLE conteudo.historias ADD COLUMN memoria_gravar text, ADD COLUMN memoria_usar text;

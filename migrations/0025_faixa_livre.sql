-- Faixa etária da história vira texto livre ("5 anos", "8-10", "adulto"); vazio = a IA escolhe.
ALTER TABLE conteudo.historias DROP CONSTRAINT IF EXISTS historias_faixa_check;
ALTER TABLE conteudo.historias ADD CONSTRAINT historias_faixa_tamanho CHECK (length(faixa) <= 40);

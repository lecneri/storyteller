-- Vozes extras de um personagem (ex.: o mesmo personagem falando inglês): [{slug, nome, lingua, voz}].
ALTER TABLE conteudo.personagens ADD COLUMN vozes_extras jsonb NOT NULL DEFAULT '[]';

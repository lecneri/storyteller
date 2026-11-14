-- Link público pode usar as vozes da família (as escolhidas para cada personagem) em vez das genéricas.
ALTER TABLE partilha.links ADD COLUMN vozes_familia boolean NOT NULL DEFAULT false;

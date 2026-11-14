-- Cache dos áudios de narração: mesmo texto + mesma voz = mesmo arquivo.
CREATE TABLE midia.audios (
    familia_id  uuid NOT NULL REFERENCES acesso.familias (id) ON DELETE CASCADE,
    hash        text NOT NULL,              -- sha256 de (motor, voz, tom, velocidade, texto)
    arquivo     uuid NOT NULL REFERENCES midia.arquivos (id) ON DELETE CASCADE,
    duracao     real NOT NULL,              -- segundos
    palavras    jsonb NOT NULL DEFAULT '[]', -- [[inicio, duracao, "palavra"], ...]
    criado_em   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (familia_id, hash)
);

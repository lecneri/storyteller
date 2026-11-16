-- Músicas-modelo da plataforma (o admin do sistema envia; qualquer família pode usar numa história).
-- Ao escolher, a família ganha uma linha em midia.arquivos apontando para o mesmo arquivo.
CREATE TABLE midia.musicas_modelo (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    titulo      text NOT NULL,
    clima       text NOT NULL,
    idade_min   int NOT NULL DEFAULT 0, -- indicação: a partir de quantos anos (0 = qualquer idade)
    descricao   text,                   -- como soa (ajuda quem escolhe e a IA a sugerir)
    credito     text,                   -- licença CC BY pede crédito: aparece no fim do leitor e do vídeo
    caminho     text NOT NULL,          -- relativo a DATA_DIR
    bytes       bigint NOT NULL,
    sha256      text NOT NULL,
    duracao     real NOT NULL,
    criado_em   timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE conteudo.historias ADD COLUMN musica_credito text;

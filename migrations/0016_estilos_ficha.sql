-- Estilos na ficha do personagem (ficha.estilos = [{slug, nome, imagem}], o primeiro é o principal).
-- Personagens que já existem ganham o estilo "Principal" com a primeira imagem da variante atual
-- (ou da variante mais antiga que tenha imagem). Uma "imagem" solta na ficha (versão de teste) vira esse estilo.
UPDATE conteudo.personagens p
   SET ficha = (COALESCE(p.ficha, '{}'::jsonb) - 'imagem')
               || jsonb_build_object('estilos', jsonb_build_array(jsonb_build_object(
                    'slug', 'principal', 'nome', 'Principal',
                    'imagem', COALESCE(
                        p.ficha->>'imagem',
                        (SELECT v.referencias[1]::text FROM conteudo.variantes v WHERE v.id = p.variante_atual AND cardinality(v.referencias) > 0),
                        (SELECT v.referencias[1]::text FROM conteudo.variantes v
                          WHERE v.personagem_id = p.id AND cardinality(v.referencias) > 0 ORDER BY v.criado_em LIMIT 1)))))
 WHERE NOT (COALESCE(p.ficha, '{}'::jsonb) ? 'estilos')
   AND (p.ficha->>'imagem' IS NOT NULL
        OR EXISTS (SELECT 1 FROM conteudo.variantes v WHERE v.personagem_id = p.id AND cardinality(v.referencias) > 0));

UPDATE conteudo.personagens SET ficha = ficha - 'imagem' WHERE ficha ? 'imagem';

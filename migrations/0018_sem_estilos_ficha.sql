-- O estilo passa a ser só da variante (texto livre: "anime traço Naruto"). A ficha fica com a aparência e as fotos reais.
-- Imagens de estilo que nenhuma variante usa não se perdem: entram nas fotos de referência (privadas) da ficha.
UPDATE conteudo.personagens p
   SET ficha = (p.ficha - 'estilos') || jsonb_build_object('fotos',
         COALESCE(p.ficha->'fotos', '[]'::jsonb) || COALESCE((
           SELECT jsonb_agg(e->>'imagem')
             FROM jsonb_array_elements(p.ficha->'estilos') e
            WHERE e->>'imagem' IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM conteudo.variantes v
                               WHERE v.personagem_id = p.id AND (e->>'imagem')::uuid = ANY(v.referencias))
              AND NOT COALESCE(p.ficha->'fotos', '[]'::jsonb) ? (e->>'imagem')), '[]'::jsonb))
 WHERE p.ficha ? 'estilos';

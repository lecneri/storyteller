-- O primeiro estilo da ficha passa a se chamar "Base" (a imagem base do personagem, foto ou desenho):
-- o "Principal" criado pela migração 0016 vira "Base".
UPDATE conteudo.personagens
   SET ficha = jsonb_set(ficha, '{estilos,0}', (ficha->'estilos'->0) || '{"slug": "base", "nome": "Base"}'::jsonb)
 WHERE ficha->'estilos'->0->>'slug' = 'principal'
   AND NOT EXISTS (SELECT 1 FROM jsonb_array_elements(ficha->'estilos') e WHERE e->>'slug' = 'base');

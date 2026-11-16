// Testes das regras do editor: node --test tests/js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const L = require('../../static/editor-logica.js');

const personagens = [
  { slug: 'pedro', nome: 'Pedro', ficha: { aparencia: 'Menino de 4 anos' }, variante_atual: 'normal',
    variantes: [
      { slug: 'normal', nome: 'Normal', referencias: ['img-normal'], descricao: 'camiseta azul' },
      { slug: 'ranger', nome: 'Ranger', referencias: [], descricao: 'armadura vermelha', estilo: 'Power Rangers' },
    ] },
  { slug: 'noob', nome: 'Noob', ficha: { aparencia: 'shih tzu' }, variantes: [] },
];

test('chave da voz igual à do servidor', () => {
  assert.equal(L.chaveVoz({ quem: 'narrador' }), 'narrador');
  assert.equal(L.chaveVoz({ quem: 'pedro', voz: 'ingles' }), 'pedro+ingles');
  assert.equal(L.chaveVoz({ quem: 'narrador', voz: 'ingles' }), 'narrador');
  assert.equal(L.chaveVoz({ quem: 'voz:en-US-GuyNeural' }), 'voz:en-US-GuyNeural');
  assert.equal(L.chaveVoz({ quem: 'voz:en-US-GuyNeural', tom: -30 }), 'voz:en-US-GuyNeural|-30|0');
});

test('nome curto da voz do sistema', () => {
  const vozes = [{ id: 'en-US-GuyNeural', nome: 'Guy · homem, animado (só inglês)' }];
  assert.equal(L.nomeVoz(vozes, 'en-US-GuyNeural'), 'Guy');
  assert.equal(L.nomeVoz(vozes, 'desconhecida'), 'desconhecida');
});

test('miniatura cai para uma variante com imagem', () => {
  assert.equal(L.miniatura(personagens, 'pedro', 'normal'), '/midia/img-normal');
  assert.equal(L.miniatura(personagens, 'pedro', null), '/midia/img-normal');
  assert.equal(L.miniatura(personagens, 'noob'), null);
  assert.equal(L.miniatura(personagens, 'ninguem'), null);
});

test('variante sem imagem empresta o rosto de outra', () => {
  const [r] = L.referencias(personagens, { personagens: [{ slug: 'pedro', variante: 'ranger' }] });
  assert.equal(r.ref, 'img-normal');
  assert.equal(r.emprestada, 'Normal');
  assert.equal(r.num, 1);
  assert.equal(r.desc, 'armadura vermelha; Menino de 4 anos');
});

test('prompt de imagem: referências numeradas, descrições e secundários', () => {
  const pag = { cena: 'Na lua', personagens: [{ slug: 'noob' }, { slug: 'pedro', variante: 'ranger' }], secundarios: ['Pirata Caveira'] };
  const ctx = { personagens, secundarios: [{ nome: 'Pirata Caveira', descricao: 'esqueleto de chapéu' }], estilo: 'aquarela', faixa: '4-6' };
  const p = L.promptImagem(pag, ctx);
  assert.match(p, /Estilo: aquarela, Power Rangers, cores vivas/);
  assert.match(p, /crianças de 4-6 anos/);
  assert.match(p, /Cena: Na lua/);
  assert.match(p, /- Noob — shih tzu \(sem imagem de referência\)/);
  assert.match(p, /- Imagem 1: Pedro \(Ranger\) — use a imagem só para o rosto/);
  assert.match(p, /- Pirata Caveira — esqueleto de chapéu/);
  assert.match(p, /siga fielmente as imagens de referência anexadas/);
  assert.match(p, /Não escreva nenhum texto/);
});

test('prompt sem personagens nem cena', () => {
  const p = L.promptImagem({}, { personagens, faixa: '2-3' });
  assert.match(p, /Cena: \(descreva a cena\)/);
  assert.doesNotMatch(p, /Personagens/);
});

test('prompt de animação leva o efeito', () => {
  assert.match(L.promptAnimacao({ cena: 'x', animacao: 'chuva' }), /Efeito sugerido: chuva\./);
  assert.doesNotMatch(L.promptAnimacao({ cena: 'x', animacao: 'nenhuma' }), /Efeito/);
});

test('secundário vira personagem: voz, falas e cenas', () => {
  const paginas = [
    { secundarios: ['Pirata'], personagens: [], blocos: [
      { quem: 'narrador', texto: 'a' },
      { quem: 'voz:en-US-GuyNeural', nome: 'Pirata', tom: -25, texto: 'Arrr' },
      { quem: 'voz:en-US-RogerNeural', nome: 'Outro', texto: 'b' },
    ] },
  ];
  assert.deepEqual(L.vozDoSecundario(paginas, 'Pirata'), { motor: 'edge', id: 'en-US-GuyNeural', tom: -25, velocidade: 0 });
  assert.equal(L.vozDoSecundario(paginas, 'Fantasma'), null);
  L.secundarioVirouPersonagem(paginas, 'Pirata', 'pirata');
  assert.deepEqual(paginas[0].blocos[1], { quem: 'pirata', texto: 'Arrr' });
  assert.equal(paginas[0].blocos[2].quem, 'voz:en-US-RogerNeural');
  assert.deepEqual(paginas[0].secundarios, []);
  assert.deepEqual(paginas[0].personagens, [{ slug: 'pirata', variante: null }]);
});

test('renomear secundário atualiza as páginas', () => {
  const paginas = [{ secundarios: ['Pirata', 'Coruja'] }, {}];
  L.renomearSecundario(paginas, 'Pirata', 'Pirata Caveira');
  assert.deepEqual(paginas[0].secundarios, ['Pirata Caveira', 'Coruja']);
  assert.deepEqual(paginas[1].secundarios, []);
});

test('tipo do secundário: o da IA, ou deduzido pela descrição', () => {
  assert.equal(L.adivinharTipo({ nome: 'X', tipo: 'vilao', descricao: 'menino' }), 'vilao');
  assert.equal(L.adivinharTipo({ nome: 'Papagaio Pipo', descricao: 'penas verdes' }), 'animal');
  assert.equal(L.adivinharTipo({ nome: 'Lelê', descricao: 'Menina de 6 anos, tranças' }), 'crianca');
  assert.equal(L.adivinharTipo({ nome: 'Coruja Sábia', descricao: 'olhos grandes' }), 'animal');
  assert.equal(L.adivinharTipo({ nome: 'Pirata Caveira', descricao: 'rosto de caveira, chapéu' }), 'criatura');
  assert.equal(L.adivinharTipo({ nome: 'Capitão Trapalhão', descricao: 'pirata atrapalhado de bigode' }), 'vilao');
  assert.equal(L.adivinharTipo({ nome: 'Robozinho', descricao: 'robô de lata' }), 'robo');
  assert.equal(L.adivinharTipo({ nome: 'Dona Zelinda', descricao: 'senhora de óculos' }), 'adulto');
});

test('variante sem imagem usa o estilo dela na ficha (ou o principal)', () => {
  const ps = [{ slug: 'pedro', nome: 'Pedro', ficha: { estilos: [
    { slug: 'pixar', nome: 'Pixar', imagem: 'img-pixar' }, { slug: 'anime', nome: 'Anime', imagem: 'img-anime' }] },
  variante_atual: 'pokemon', variantes: [
    { slug: 'pokemon', nome: 'Pokémon', referencias: [], estilo: 'Anime' },
    { slug: 'pirata', nome: 'Pirata', referencias: [], estilo: 'Aquarela' },
    { slug: 'ranger', nome: 'Ranger', referencias: ['img-ranger'], estilo: 'Anime' }] }];
  const ref = (v) => L.referencias(ps, { personagens: [{ slug: 'pedro', variante: v }] })[0];
  assert.equal(ref('pokemon').ref, 'img-anime');
  assert.equal(ref('pokemon').emprestada, 'Pedro · Anime');
  assert.equal(ref('pirata').ref, 'img-pixar');
  assert.equal(ref('ranger').ref, 'img-ranger');
  assert.equal(ref('ranger').emprestada, null);
  assert.equal(L.miniatura(ps, 'pedro', 'pokemon'), '/midia/img-anime');
});

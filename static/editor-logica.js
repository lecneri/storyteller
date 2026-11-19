// Regras do editor sem tela (testadas com `node --test tests/js`): chave da voz de cada fala,
// referências e prompts de imagem, e secundário → personagem. No navegador: window.EditorLogica.
(function (raiz) {
  // Mesma regra do servidor (Bloco::chave_voz): "pedro", "pedro+ingles" ou "voz:<id>|tom|velocidade".
  function chaveVoz(b) {
    if (b.quem.startsWith('voz:')) {
      const t = b.tom || 0, v = b.velocidade || 0;
      return t || v ? b.quem + '|' + t + '|' + v : b.quem;
    }
    return b.voz && b.quem !== 'narrador' ? b.quem + '+' + b.voz : b.quem;
  }

  // "Guy · homem, animado" → "Guy".
  function nomeVoz(vozesSistema, id) {
    return ((vozesSistema.find((v) => v.id === id) || {}).nome || id).split(' · ')[0];
  }

  const personagem = (personagens, slug) => personagens.find((p) => p.slug === slug);

  // Primeira imagem da variante indicada (ou da atual); senão a de qualquer variante com imagem.
  function miniatura(personagens, slug, variante) {
    const p = personagem(personagens, slug);
    if (!p) return null;
    const vs = p.variantes || [];
    const v = vs.find((x) => x.slug === (variante || p.variante_atual));
    const outra = vs.find((x) => (x.referencias || []).length);
    const ref = (v && (v.referencias || [])[0]) || (outra && outra.referencias[0]);
    return ref ? '/midia/' + ref : null;
  }

  function varianteDe(personagens, n) {
    const p = personagem(personagens, n.slug);
    if (!p) return null;
    return (p.variantes || []).find((v) => v.slug === (n.variante || p.variante_atual)) || null;
  }

  // Referências da cena: quem tem imagem vira "Imagem N"; quem não tem vai pela descrição. Variante sem
  // imagem (ex.: transformação) empresta a imagem de outra variante só para rosto e traços.
  function referencias(personagens, pag) {
    let num = 0;
    return (pag.personagens || []).map((n) => {
      const p = personagem(personagens, n.slug) || { nome: n.slug, ficha: {} };
      const v = varianteDe(personagens, n);
      let ref = v && (v.referencias || [])[0];
      let emprestada = null;
      if (!ref) {
        const comImagem = (x) => x !== v && (x.referencias || []).length;
        const outra = (p.variantes || []).find((x) => comImagem(x) && x.slug === p.variante_atual) || (p.variantes || []).find(comImagem);
        if (outra) { ref = outra.referencias[0]; emprestada = outra.nome; }
      }
      const desc = [v && v.descricao, (p.ficha || {}).aparencia].filter(Boolean).join('; ');
      return { nome: p.nome, variante: v ? v.nome : null, estilo: v && v.estilo, ref, emprestada, num: ref ? ++num : null, desc };
    });
  }

  // Lugar da página: um da família (pag.lugar = slug, com imagem) ou um só desta história (pag.lugar = nome).
  function lugarDaPagina(pag, ctx) {
    if (!pag.lugar) return null;
    const f = (ctx.lugares || []).find((l) => l.slug === pag.lugar);
    if (f) return { nome: f.nome, descricao: f.descricao || '', ref: (f.referencias || [])[0] || null };
    const h = (ctx.lugaresHistoria || []).find((l) => l.nome === pag.lugar);
    return h ? { nome: h.nome, descricao: h.descricao || '', ref: null } : null;
  }

  // Imagens que vão junto no pedido, na ordem do prompt: personagens e depois o lugar.
  function refsDaPagina(pag, ctx) {
    const v = referencias(ctx.personagens, pag).filter((r) => r.ref).map((r) => r.ref);
    const l = lugarDaPagina(pag, ctx);
    if (l && l.ref) v.push(l.ref);
    return v;
  }

  // ctx: { personagens, secundarios, lugares, lugaresHistoria, estilo, faixa }
  function promptImagem(pag, ctx) {
    const refs = referencias(ctx.personagens, pag);
    const estilo = [...new Set([ctx.estilo, ...refs.map((r) => r.estilo)].filter(Boolean))].join(', ');
    const linhas = refs.map((r) =>
      '- ' + (r.num ? 'Imagem ' + r.num + ': ' : '') + r.nome + (r.variante ? ' (' + r.variante + ')' : '') +
      (r.emprestada ? ' — use a imagem só para o rosto, cabelo e traços; a roupa e a aparência são as desta variante' : '') +
      (r.desc ? ' — ' + r.desc : '') + (r.num ? '' : ' (sem imagem de referência)'));
    // Secundários (sem ficha): sempre a mesma descrição, para saírem iguais em todas as páginas.
    for (const nome of pag.secundarios || []) {
      const x = (ctx.secundarios || []).find((s) => s.nome === nome);
      if (x) linhas.push('- ' + x.nome + (x.descricao ? ' — ' + x.descricao : ''));
    }
    const comImagem = refs.some((r) => r.num);
    const lugar = lugarDaPagina(pag, ctx);
    const numLugar = lugar && lugar.ref ? refs.filter((r) => r.num).length + 1 : null;
    const cenario = lugar ? 'Cenário: ' + (numLugar ? 'Imagem ' + numLugar + ': ' : '') + lugar.nome +
      (lugar.descricao ? ' — ' + lugar.descricao.trim().replace(/[.\s]+$/, '') : '') +
      (numLugar ? ' (use a imagem só para o formato, os objetos e as cores do lugar; a luz, a hora do dia e o clima seguem a cena)' : '') +
      '. O lugar deve ser reconhecível e igual em todas as páginas.' : null;
    return [
      'Ilustração de livro, página inteira, formato retrato 3:4.',
      'Estilo: ' + (estilo ? estilo + ', ' : '') + 'cores vivas, traço limpo, adequado a um livro para ' + faixaTexto(ctx.faixa) + '.',
      '',
      'Cena: ' + (pag.cena || '(descreva a cena)'),
      cenario,
      '',
      linhas.length ? 'Personagens' + (comImagem ? ' (siga fielmente as imagens de referência anexadas, na ordem; os sem imagem, pela descrição):' : ' (siga a descrição de cada um):') : null,
      ...linhas,
      '',
      'Não escreva nenhum texto, letra, número ou balão de fala na imagem.',
    ].filter((l) => l !== null).join('\n');
  }

  function promptAnimacao(pag) {
    const efeito = pag.animacao && pag.animacao !== 'nenhuma' ? ' Efeito sugerido: ' + pag.animacao + '.' : '';
    return 'Anime esta ilustração (use a imagem anexada como quadro inicial), 5 a 8 segundos, em loop suave. ' +
      'Movimentos leves e naturais dos personagens e do cenário, câmera quase parada, sem cortes, sem texto.' + efeito +
      '\nCena: ' + (pag.cena || '');
  }

  // Voz das falas de um secundário (a da primeira fala com voz do sistema), para virar a voz do personagem.
  function vozDoSecundario(paginas, nome) {
    for (const p of paginas) for (const b of p.blocos || []) {
      if (b.quem.startsWith('voz:') && b.nome === nome) return { motor: 'edge', id: b.quem.slice(4), tom: b.tom || 0, velocidade: b.velocidade || 0 };
    }
    return null;
  }

  // Secundário salvo como personagem: falas e cenas passam a apontar para o slug novo.
  function secundarioVirouPersonagem(paginas, nome, slug) {
    for (const p of paginas) {
      for (const b of p.blocos || []) {
        if (b.quem.startsWith('voz:') && b.nome === nome) { b.quem = slug; delete b.nome; delete b.tom; delete b.velocidade; }
      }
      if ((p.secundarios || []).includes(nome)) {
        p.secundarios = p.secundarios.filter((n) => n !== nome);
        (p.personagens = p.personagens || []).push({ slug, variante: null });
      }
    }
  }

  // Secundário renomeado: as páginas acompanham o nome novo.
  function renomearSecundario(paginas, antigo, novo) {
    for (const p of paginas) p.secundarios = (p.secundarios || []).map((n) => (n === antigo ? novo : n));
  }

  // Tipo de personagem pelo nome/descrição, para histórias geradas antes de a IA informar o tipo.
  // Compara sem acentos (o \b do JavaScript não entende "robô").
  const PALAVRAS_TIPO = [
    ['robo', /\b(robos?|androide|maquina|carro|foguete|caminhao|trem|nave)\b/],
    ['animal', /\b(cachorr\w*|cao|gat\w*|papagaio|coruja|passar\w*|bode|vaca|cavalo|coelh\w*|urso|leao|tigre|macac\w*|elefante|girafa|peixe|tartaruga|sapo|pato|galinha|porc\w*|rato|ratinho|raposa|lobo|esquilo|abelha|borboleta|baleia|golfinho|tubarao|dinossauro|filhote|bicho|animal|penas|pelos?|focinho|patas?)\b/],
    ['criatura', /\b(dragao|monstr\w*|fantasma|caveira|esqueleto|alien\w*|fada|duende|gigante|unicornio|sereia|ogro|troll|zumbi|criatura|bruxa|mago)\b/],
    ['crianca', /\b(menin[oa]s?|garot[oa]s?|crianca|bebe|\d+ anos)\b/],
    ['vilao', /\b(vilao|vila|malvad\w*|bandid\w*|ladrao|pirata)\b/],
  ];
  const semAcento = (t) => t.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  function adivinharTipo(sec) {
    if (sec.tipo) return sec.tipo;
    const texto = semAcento((sec.nome || '') + ' ' + (sec.descricao || ''));
    const achado = PALAVRAS_TIPO.find(([, re]) => re.test(texto));
    return achado ? achado[0] : 'adulto';
  }

  // Faixa para mostrar: "4-6" → "4-6 anos"; "adulto" fica; vazia → "todas as idades" (igual ao servidor).
  function faixaTexto(f) {
    f = (f || '').trim();
    if (!f) return 'todas as idades';
    return /^[0-9 -]+$/.test(f) ? f + ' anos' : f;
  }

  const api = { faixaTexto, adivinharTipo, chaveVoz, nomeVoz, miniatura, varianteDe, referencias, promptImagem, promptAnimacao, lugarDaPagina, refsDaPagina,
    vozDoSecundario, secundarioVirouPersonagem, renomearSecundario };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else raiz.EditorLogica = api;
})(typeof window !== 'undefined' ? window : globalThis);

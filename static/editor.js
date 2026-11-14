// Editor de blocos: páginas → blocos (narrador / fala de personagem). Salva como JSON.
(function () {
  const dados = JSON.parse(document.getElementById('dados').textContent);
  const raiz = document.getElementById('editor');
  const estado = {
    titulo: dados.titulo,
    narracao: dados.narracao,
    paginas: (dados.conteudo && dados.conteudo.paginas) || [],
  };
  const personagens = dados.personagens || [];
  let sujo = false;

  const el = (tag, attrs = {}, ...filhos) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === 'class') e.className = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? '' : v);
    }
    for (const f of filhos.flat()) if (f !== null && f !== undefined) e.append(f.nodeType ? f : document.createTextNode(f));
    return e;
  };
  const ajustar = (t) => { t.style.height = 'auto'; t.style.height = t.scrollHeight + 2 + 'px'; };
  document.addEventListener('input', (e) => { if (e.target.matches('#editor textarea')) ajustar(e.target); });
  const mudou = () => { sujo = true; status.textContent = 'Alterações não salvas'; };

  const personagem = (slug) => personagens.find((p) => p.slug === slug);
  // Miniatura: primeira imagem da variante indicada (ou da atual, ou de qualquer uma).
  function miniatura(slug, variante) {
    const p = personagem(slug);
    if (!p) return null;
    const vs = p.variantes || [];
    const v = vs.find((x) => x.slug === (variante || p.variante_atual)) || vs.find((x) => (x.referencias || []).length);
    const ref = v && (v.referencias || [])[0];
    return ref ? '/midia/' + ref : null;
  }
  function avatar(quem) {
    if (quem === 'narrador') return el('div', { class: 'avatar', title: 'Narrador' }, '🎙️');
    const src = miniatura(quem);
    const emoji = { animal: '🐶', robo: '🤖', vilao: '😈', adulto: '🧑', criatura: '🐉' }[(personagem(quem) || {}).tipo] || '🧒';
    return src ? el('img', { class: 'avatar', src, alt: '' }) : el('div', { class: 'avatar' }, emoji);
  }
  const nomeDe = (quem) => (quem === 'narrador' ? 'Narrador' : (personagem(quem) || {}).nome || quem);

  let tocando = null;
  async function ouvirBloco(b, botao) {
    if (!(b.texto || '').trim()) { aviso('Bloco vazio.'); return; }
    botao.textContent = '⏳';
    try {
      const r = await fetch(location.pathname + '/bloco-audio', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ quem: b.quem, texto: b.texto }) });
      const j = await r.json();
      if (!r.ok) throw new Error(j.erro || 'erro');
      if (tocando) tocando.pause();
      tocando = new Audio(j.audio);
      await tocando.play();
    } catch (e) {
      aviso('Não deu para ouvir: ' + e.message);
    } finally {
      botao.textContent = '▶';
    }
  }

  // Narração da história inteira (job no servidor) + progresso.
  const statusNarracao = el('span', { class: 'suave' }, '');
  async function verNarracao() {
    try {
      const j = await (await fetch(location.pathname + '/narracao')).json();
      const rodando = j.job === 'pendente' || j.job === 'rodando';
      statusNarracao.textContent = '🎙️ ' + j.prontos + '/' + j.total + ' trechos narrados' +
        (rodando ? ' · gerando…' : j.job === 'erro' ? ' · erro: ' + (j.erro || '') : '');
      if (rodando) setTimeout(verNarracao, 3000);
    } catch (_) { /* ignora */ }
  }
  async function gerarNarracao() {
    if (sujo) await salvar();
    await fetch(location.pathname + '/narrar', { method: 'POST' });
    statusNarracao.textContent = '🎙️ gerando…';
    setTimeout(verNarracao, 1500);
  }

  function mover(lista, i, d) {
    const j = i + d;
    if (j < 0 || j >= lista.length) return;
    [lista[i], lista[j]] = [lista[j], lista[i]];
    mudou(); desenhar();
  }

  function blocoUI(pag, i) {
    const b = pag.blocos[i];
    const opcoes = [el('option', { value: 'narrador', selected: b.quem === 'narrador' }, '🎙️ Narrador')].concat(
      personagens.map((p) => el('option', { value: p.slug, selected: b.quem === p.slug }, p.nome))
    );
    const sel = el('select', { onchange: (e) => { b.quem = e.target.value; mudou(); desenhar(); } }, opcoes);
    if (estado.narracao === 'narrador') sel.disabled = true;
    const txt = el('textarea', { rows: Math.max(2, Math.ceil((b.texto || '').length / 70)),
      placeholder: b.quem === 'narrador' ? 'Texto do narrador…' : 'Fala de ' + nomeDe(b.quem) + '…',
      oninput: (e) => { b.texto = e.target.value; mudou(); } });
    txt.value = b.texto || '';
    return el('div', { class: 'bloco' },
      avatar(b.quem),
      el('div', {},
        el('div', { class: 'cab' }, sel,
          el('span', { class: 'acoes' },
            el('button', { type: 'button', title: 'Ouvir este trecho', onclick: (e) => ouvirBloco(b, e.currentTarget) }, '▶'),
            el('button', { type: 'button', title: 'Subir', onclick: () => mover(pag.blocos, i, -1) }, '↑'),
            el('button', { type: 'button', title: 'Descer', onclick: () => mover(pag.blocos, i, 1) }, '↓'),
            el('button', { type: 'button', title: 'Apagar bloco', onclick: () => { pag.blocos.splice(i, 1); mudou(); desenhar(); } }, '✕'))),
        txt));
  }

  // ───────────── Kit da imagem ─────────────
  function aviso(msg) {
    const a = document.getElementById('aviso');
    if (!a) return;
    a.textContent = msg; a.hidden = false;
    clearTimeout(a._t); a._t = setTimeout(() => (a.hidden = true), 2500);
  }
  async function copiarTexto(texto) {
    try {
      await navigator.clipboard.writeText(texto);
    } catch (_) {
      // Sem HTTPS o navegador bloqueia a API nova: usa o jeito antigo.
      const t = el('textarea', { style: 'position:fixed;opacity:0' });
      t.value = texto; document.body.append(t); t.select();
      document.execCommand('copy'); t.remove();
    }
    aviso('Prompt copiado! Cole no ChatGPT ou no Gemini.');
  }
  function varianteDe(n) {
    const p = personagem(n.slug);
    if (!p) return null;
    const vs = p.variantes || [];
    return vs.find((v) => v.slug === (n.variante || p.variante_atual)) || null;
  }
  // Lista de referências da cena: quem tem imagem vira "Imagem N"; quem não tem vai por descrição.
  function referencias(pag) {
    let num = 0;
    return (pag.personagens || []).map((n) => {
      const p = personagem(n.slug) || { nome: n.slug, ficha: {} };
      const v = varianteDe(n);
      const ref = v && (v.referencias || [])[0];
      const desc = [v && v.descricao, (p.ficha || {}).aparencia].filter(Boolean).join('; ');
      return { nome: p.nome, variante: v ? v.nome : null, estilo: v && v.estilo, ref, num: ref ? ++num : null, desc };
    });
  }
  function promptImagem(pag) {
    const refs = referencias(pag);
    const estilos = [dados.estilo, ...refs.map((r) => r.estilo)].filter(Boolean);
    const estilo = [...new Set(estilos)].join(', ');
    const linhas = refs.map((r) =>
      '- ' + (r.num ? 'Imagem ' + r.num + ': ' : '') + r.nome + (r.variante ? ' (' + r.variante + ')' : '') +
      (r.desc ? ' — ' + r.desc : '') + (r.num ? '' : ' (sem imagem de referência)'));
    return [
      'Ilustração de livro infantil, página inteira, formato retrato 3:4.',
      'Estilo: ' + (estilo ? estilo + ', ' : '') + 'cores vivas, traço limpo e amigável, para crianças de ' + dados.faixa + ' anos.',
      '',
      'Cena: ' + (pag.cena || '(descreva a cena)'),
      '',
      refs.length ? 'Personagens' + (refs.some((r) => r.num) ? ' (siga fielmente as imagens de referência anexadas, na ordem):' : ':') : null,
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
  async function enviarArquivo(pag, arquivo) {
    const fd = new FormData();
    fd.append('arquivo', arquivo);
    aviso('Enviando…');
    try {
      const r = await fetch(location.pathname + '/midia', { method: 'POST', body: fd });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      pag.midia = { id: j.id, tipo: j.tipo };
      mudou(); desenhar(); await salvar();
      aviso('Mídia recebida e salva ✓');
    } catch (e) {
      aviso('Não enviou: ' + e.message);
    }
  }
  async function gerarPelaApi(pag, botao) {
    const refs = referencias(pag).filter((r) => r.ref).map((r) => r.ref);
    botao.disabled = true; botao.textContent = '⏳ Gerando (até 1 min)…';
    try {
      const r = await fetch(location.pathname + '/imagem-ia', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ prompt: promptImagem(pag), refs }) });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      pag.midia = { id: j.id, tipo: j.tipo };
      mudou(); desenhar(); await salvar();
      aviso('Imagem gerada e salva ✓');
    } catch (e) {
      aviso('Não gerou: ' + e.message);
      botao.disabled = false; botao.textContent = '✨ Gerar imagem pela API';
    }
  }
  function previaMidia(pag) {
    if (!pag.midia) return null;
    const src = '/midia/' + pag.midia.id;
    const midia = pag.midia.tipo.startsWith('video/')
      ? el('video', { src, autoplay: true, loop: true, muted: true, playsinline: true, class: 'previa' })
      : el('img', { src, alt: '', class: 'previa' });
    if (midia.tagName === 'VIDEO') midia.muted = true;
    return el('div', { class: 'previa-box' }, midia,
      el('button', { type: 'button', class: 'link', onclick: () => {
        if (confirm('Tirar a mídia desta página?')) { delete pag.midia; mudou(); desenhar(); }
      } }, 'remover'));
  }
  function areaEnvio(pag) {
    const input = el('input', { type: 'file', accept: 'image/png,image/jpeg,image/webp,image/gif,video/mp4,video/webm,video/quicktime', hidden: true,
      onchange: (e) => e.target.files[0] && enviarArquivo(pag, e.target.files[0]) });
    const zona = el('div', { class: 'zona-envio', tabindex: 0,
      onclick: () => input.click(),
      ondragover: (e) => { e.preventDefault(); zona.classList.add('arrastando'); },
      ondragleave: () => zona.classList.remove('arrastando'),
      ondrop: (e) => { e.preventDefault(); zona.classList.remove('arrastando'); const f = e.dataTransfer.files[0]; if (f) enviarArquivo(pag, f); },
      onpaste: (e) => { const f = [...(e.clipboardData || {}).files || []][0]; if (f) { e.preventDefault(); enviarArquivo(pag, f); } } },
      pag.midia ? '⬆ Trocar: arraste, cole (Ctrl+V) ou clique' : '⬆ Arraste aqui, cole (Ctrl+V) ou clique para enviar a imagem, GIF ou vídeo', input);
    return zona;
  }

  function ouvirSom(pag) {
    const nome = pag.animacao || 'nenhuma';
    if (!window.Efeitos.temSom(nome)) { aviso(nome === 'nenhuma' ? 'Escolha uma animação primeiro.' : 'Esta animação não tem som.'); return; }
    window.Efeitos.tocarSom(nome);
  }
  function verAnimacao(pag, palco) {
    const nome = pag.animacao || 'nenhuma';
    if (nome === 'nenhuma') { aviso('Escolha uma animação primeiro.'); return; }
    palco.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    window.Efeitos.aplicar(palco, nome);
  }

  function lateralUI(pag) {
    const cena = el('textarea', { rows: 3, placeholder: 'O que aparece na ilustração desta página…',
      oninput: (e) => { pag.cena = e.target.value; mudou(); } });
    cena.value = pag.cena || '';
    const anim = el('select', { onchange: (e) => { pag.animacao = e.target.value; mudou(); } },
      (dados.animacoes || []).map((a) => el('option', { value: a, selected: (pag.animacao || 'nenhuma') === a }, a)));

    // Personagens na cena: adicionar/remover
    const naCena = (pag.personagens || []).map((n, i) => {
      const p = personagem(n.slug);
      const src = miniatura(n.slug, n.variante);
      const vs = (p && p.variantes) || [];
      // Seletor de variante: mostra cada variante uma vez; sem escolha na página, vale a atual do personagem.
      const escolhida = n.variante || (p && p.variante_atual) || '';
      const selVar = vs.length ? el('select', { class: 'chip-var', title: 'Variante nesta cena',
          onchange: (e) => { n.variante = e.target.value || null; mudou(); desenhar(); } },
        escolhida ? null : el('option', { value: '', selected: true }, '—'),
        vs.filter((v) => !v.arquivada || v.slug === escolhida)
          .map((v) => el('option', { value: v.slug, selected: escolhida === v.slug }, v.nome + (v.arquivada ? ' (arquivada)' : '')))) : null;
      return el('span', { class: 'chip' }, src ? el('img', { src, alt: '', class: 'chip-img' }) : null,
        ' ', p ? p.nome : n.slug, selVar,
        el('button', { type: 'button', class: 'link', title: 'Tirar da cena', onclick: () => { pag.personagens.splice(i, 1); mudou(); desenhar(); } }, '✕'));
    });
    const faltam = personagens.filter((p) => !(pag.personagens || []).some((n) => n.slug === p.slug));
    const addSel = faltam.length ? el('select', { onchange: (e) => {
      if (!e.target.value) return;
      (pag.personagens = pag.personagens || []).push({ slug: e.target.value, variante: null }); mudou(); desenhar();
    } }, el('option', { value: '' }, '+ personagem'), faltam.map((p) => el('option', { value: p.slug }, p.nome))) : null;

    // Referências (miniaturas: clique copia a imagem)
    const refs = referencias(pag);
    const minis = refs.map((r) => r.ref
      ? el('figure', { class: 'mini' },
          el('img', { src: '/midia/' + r.ref, alt: r.nome, 'data-copiar': true, title: 'Clique para copiar a imagem' }),
          el('figcaption', {}, 'Imagem ' + r.num + ' · ' + r.nome, el('br'),
            el('button', { type: 'button', class: 'link', 'data-copiar-de': '/midia/' + r.ref }, '📋 Copiar'), ' · ',
            el('a', { href: '/midia/' + r.ref, download: (r.num + '-' + r.nome).replace(/\s+/g, '-') }, '⬇')))
      : el('figure', { class: 'mini sem' }, el('div', { class: 'mini-desc' }, '📝'), el('figcaption', {}, r.nome, el('br'), el('small', { class: 'suave' }, 'só descrição'))));

    const palco = el('div', { class: 'palco' + (pag.midia ? '' : ' vazio') },
      pag.midia && pag.midia.tipo.startsWith('image/') ? el('img', { src: '/midia/' + pag.midia.id, alt: '' }) : el('span', {}, '🖼️'));

    const prompt = promptImagem(pag);
    const caixaPrompt = el('textarea', { rows: 6, readonly: true, class: 'prompt' });
    caixaPrompt.value = prompt;

    const kit = el('div', { class: 'kit' },
      el('strong', {}, '🎨 Kit da imagem'),
      minis.length ? el('div', { class: 'minis' }, minis) : el('small', { class: 'suave' }, 'Nenhum personagem na cena.'),
      el('div', { class: 'linha-kit' },
        el('span', {}, 'Prompt'),
        el('button', { type: 'button', class: 'pequeno', onclick: () => copiarTexto(promptImagem(pag)) }, '📋 Copiar prompt')),
      caixaPrompt,
      el('div', { class: 'linha-kit' },
        el('a', { href: 'https://chatgpt.com/', target: '_blank', rel: 'noopener' }, 'Abrir ChatGPT ↗'),
        el('a', { href: 'https://gemini.google.com/app', target: '_blank', rel: 'noopener' }, 'Abrir Gemini ↗'),
        el('a', { href: 'https://labs.google/fx/tools/flow', target: '_blank', rel: 'noopener' }, 'Abrir Flow ↗')),
      previaMidia(pag),
      dados.imagem_api ? el('button', { type: 'button', onclick: (e) => gerarPelaApi(pag, e.currentTarget) }, '✨ Gerar imagem pela API') : null,
      areaEnvio(pag),
      pag.midia && pag.midia.tipo.startsWith('image/') ? el('div', { class: 'linha-kit' },
        el('span', {}, '🎬 Animar esta imagem'),
        el('button', { type: 'button', class: 'pequeno secundario', onclick: () => copiarTexto(promptAnimacao(pag)) }, '📋 Prompt de animação')) : null);

    return el('aside', { class: 'lateral' },
      el('strong', {}, '🖼️ Ilustração'),
      el('label', {}, 'Cena', cena),
      el('div', {}, el('small', { class: 'suave' }, 'Personagens na cena'), el('div', { class: 'chips' }, naCena, addSel)),
      el('label', {}, 'Animação', anim),
      el('div', { class: 'linha-kit' },
        el('label', { class: 'check' },
          el('input', { type: 'checkbox', checked: pag.som !== false,
            onchange: (e) => { pag.som = e.target.checked; mudou(); } }), 'Usar som'),
        el('span', { class: 'linha' },
          el('button', { type: 'button', class: 'pequeno secundario', onclick: () => ouvirSom(pag) }, '🔊 Ouvir som'),
          el('button', { type: 'button', class: 'pequeno secundario', onclick: () => verAnimacao(pag, palco) }, '▶ Ver animação'))),
      palco,
      kit);
  }

  function paginaUI(pag, n) {
    const add = (quem) => { pag.blocos.push({ quem, texto: '' }); mudou(); desenhar(); };
    // Um botão só: o bloco nasce como narrador e o seletor do bloco troca para um personagem.
    const botoes = [el('button', { type: 'button', class: 'secundario pequeno', onclick: () => add('narrador') }, '+ Texto')];
    return el('section', { class: 'pagina' },
      el('div', {},
        el('h3', {}, '📄 Página ' + (n + 1),
          el('span', { class: 'acoes' },
            el('button', { type: 'button', class: 'link', onclick: () => mover(estado.paginas, n, -1) }, '↑'),
            el('button', { type: 'button', class: 'link', onclick: () => mover(estado.paginas, n, 1) }, '↓'),
            el('button', { type: 'button', class: 'link', onclick: () => {
              if (confirm('Apagar a página ' + (n + 1) + '?')) { estado.paginas.splice(n, 1); mudou(); desenhar(); }
            } }, 'apagar'))),
        pag.blocos.map((_, i) => blocoUI(pag, i)),
        el('div', { class: 'linha' }, botoes)),
      lateralUI(pag));
  }

  // Divisória "+ Página" antes da primeira, entre as páginas e depois da última.
  function inserirPaginaUI(pos) {
    const rotulo = pos === 0 ? '+ Página no início' : pos === estado.paginas.length ? '+ Página no fim' : '+ Página aqui';
    return el('div', { class: 'inserir-pagina' },
      el('button', { type: 'button', class: 'secundario pequeno', onclick: () => {
        estado.paginas.splice(pos, 0, { cena: '', personagens: [], animacao: 'nenhuma', blocos: [{ quem: 'narrador', texto: '' }] });
        mudou(); desenhar();
      } }, rotulo));
  }

  // Memória: resumo usado quando outra história cita esta com #.
  function memoriaUI() {
    const ta = el('textarea', { rows: 3, placeholder: 'Resumo com os fatos importantes (quem venceu, o que descobriram...). Usado quando outra história citar esta com #.' });
    ta.value = dados.memoria || '';
    const st = el('small', { class: 'suave' });
    return el('details', { class: 'memoria' }, el('summary', {}, '🧠 Memória da história' + (dados.memoria ? '' : ' (vazia)')),
      ta,
      el('div', { class: 'linha' },
        el('button', { type: 'button', class: 'pequeno secundario', onclick: async (e) => {
          e.target.disabled = true; st.textContent = 'Gerando com IA…';
          const r = await fetch(location.pathname + '/memoria', { method: 'POST' });
          const j = await r.json().catch(() => ({}));
          e.target.disabled = false;
          if (r.ok) { ta.value = dados.memoria = j.memoria; st.textContent = 'Gerada ✓'; } else st.textContent = 'Erro: ' + (j.erro || r.status);
        } }, '✨ Gerar com IA'),
        el('button', { type: 'button', class: 'pequeno', onclick: async () => {
          const r = await fetch(location.pathname + '/memoria/salvar', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ memoria: ta.value }) });
          if (r.ok) { dados.memoria = ta.value; st.textContent = 'Salva ✓'; }
        } }, '💾 Salvar memória'), st));
  }

  const status = el('span', { class: 'suave' }, '');
  async function salvar() {
    status.textContent = 'Salvando…';
    try {
      const r = await fetch(location.pathname + '/conteudo', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ titulo: estado.titulo, narracao: estado.narracao, conteudo: { paginas: estado.paginas } }),
      });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      sujo = false; status.textContent = 'Salvo ✓';
    } catch (e) {
      status.textContent = 'Não salvou: ' + e.message;
    }
  }

  function desenhar() {
    const titulo = el('input', { class: 'ed-titulo', value: estado.titulo, oninput: (e) => { estado.titulo = e.target.value; mudou(); } });
    const narr = el('select', { onchange: (e) => { estado.narracao = e.target.value; mudou(); desenhar(); } },
      el('option', { value: 'por-personagem', selected: estado.narracao !== 'narrador' }, 'Por personagem'),
      el('option', { value: 'narrador', selected: estado.narracao === 'narrador' }, 'Só narrador'));
    raiz.replaceChildren(...[
      el('div', { class: 'ed-topo' },
        el('label', {}, 'Título', titulo),
        el('label', {}, 'Narração', narr),
        el('span', { class: 'suave' }, 'Faixa ' + dados.faixa + ' anos')),
      dados.briefing ? el('p', { class: 'suave' }, el('em', {}, 'Briefing: '), dados.briefing) : null,
      (dados.continua || []).length ? el('p', { class: 'suave' }, '📚 Continua de: ',
        dados.continua.map((c, i) => [i ? ', ' : '', el('a', { href: '/historias/' + c.id }, c.titulo)])) : null,
      memoriaUI(),
      inserirPaginaUI(0),
      estado.paginas.map((p, n) => [paginaUI(p, n), inserirPaginaUI(n + 1)]),
      el('div', { class: 'barra-salvar' },
        el('a', { href: '/ler/' + dados.id, target: '_blank', rel: 'noopener' }, '📖 Abrir no player'),
        el('a', { href: '/compartilhar/historia/' + dados.id }, '🔗 Compartilhar'),
        statusNarracao,
        el('button', { type: 'button', class: 'secundario', onclick: gerarNarracao }, '🎙️ Gerar narração'),
        status, el('button', { type: 'button', onclick: salvar }, '💾 Salvar')),
    ].flat(Infinity).filter(Boolean));
    raiz.querySelectorAll('textarea').forEach(ajustar);
  }

  window.addEventListener('beforeunload', (e) => { if (sujo) { e.preventDefault(); e.returnValue = ''; } });
  document.addEventListener('keydown', (e) => { if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); salvar(); } });
  desenhar();
  verNarracao();
})();

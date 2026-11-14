// Editor de blocos: páginas → blocos (narrador / fala de personagem). Salva como JSON.
(function () {
  const { el, aviso } = window.St;
  const dados = JSON.parse(document.getElementById('dados').textContent);
  const raiz = document.getElementById('editor');
  const estado = {
    titulo: dados.titulo,
    narracao: dados.narracao,
    paginas: (dados.conteudo && dados.conteudo.paginas) || [],
    // Personagens só desta história: [{nome, descricao}] (aparência fixa para os prompts de imagem).
    secundarios: (dados.conteudo && dados.conteudo.secundarios) || [],
  };
  const secundario = (nome) => estado.secundarios.find((x) => x.nome === nome);
  const personagens = dados.personagens || [];
  let sujo = false;
  // Visualização: "lista" (todas as páginas) ou "carrossel" (uma por vez). Lembrada no navegador.
  let modo = 'lista';
  try { modo = localStorage.getItem('editor-modo') || 'lista'; } catch (_) { /* sem storage */ }
  let atual = 0;
  const NCORES = 5;
  const cor = (n) => 'cor-' + (n % NCORES);

  const ajustar = (t) => { t.style.height = 'auto'; t.style.height = t.scrollHeight + 2 + 'px'; };
  document.addEventListener('input', (e) => { if (e.target.matches('#editor textarea')) ajustar(e.target); });
  const mudou = () => { sujo = true; status.textContent = 'Alterações não salvas'; };

  const personagem = (slug) => personagens.find((p) => p.slug === slug);
  const vozesSistema = dados.vozes_sistema || [];
  // Regras sem tela (static/editor-logica.js), com o contexto desta história.
  const L = window.EditorLogica;
  const { chaveVoz, promptAnimacao } = L;
  const nomeVoz = (id) => L.nomeVoz(vozesSistema, id);
  const miniatura = (slug, variante) => L.miniatura(personagens, slug, variante);
  const referencias = (pag) => L.referencias(personagens, pag);
  const promptImagem = (pag) => L.promptImagem(pag, { personagens, secundarios: estado.secundarios, estilo: dados.estilo, faixa: dados.faixa });
  function avatar(quem) {
    if (quem === 'narrador') return el('div', { class: 'avatar', title: 'Narrador' }, '🎙️');
    if (quem.startsWith('voz:')) return el('div', { class: 'avatar', title: 'Voz do sistema' }, '🎭');
    const src = miniatura(quem);
    const emoji = { animal: '🐶', robo: '🤖', vilao: '😈', adulto: '🧑', criatura: '🐉' }[(personagem(quem) || {}).tipo] || '🧒';
    return src ? el('img', { class: 'avatar', src, alt: '' }) : el('div', { class: 'avatar' }, emoji);
  }
  const nomeDe = (quem, b) => (quem === 'narrador' ? 'Narrador'
    : quem.startsWith('voz:') ? ((b && b.nome) || nomeVoz(quem.slice(4)))
    : (personagem(quem) || {}).nome || quem);

  // Prévia de uma voz na lista: personagem (com a voz dele) ou voz do sistema.
  async function previaVoz(op, botao) {
    botao.textContent = '⏳';
    try {
      const r = op.sistema
        ? await fetch('/tts/previa', { method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ voz: { motor: 'edge', id: op.sistema, tom: 0, velocidade: 0 }, texto: 'Olá! Esta é a minha voz. Hello, this is my voice!' }) })
        : await fetch(location.pathname + '/bloco-audio', { method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ quem: op.valor, texto: op.valor === 'narrador' ? 'Era uma vez, num lugar muito distante…' : 'Oi! Eu sou ' + op.nome + '!' }) });
      const j = await r.json();
      if (!r.ok) throw new Error(j.erro || 'erro');
      if (tocando) tocando.pause();
      tocando = new Audio(j.audio);
      await tocando.play();
    } catch (e) { aviso('Não deu para ouvir: ' + e.message); }
    finally { botao.textContent = '▶'; }
  }

  // Seletor de quem fala: personagens e, à parte, as vozes do sistema (para um vilão de uma história só,
  // sem criar personagem). Cada opção tem ▶ para ouvir antes de escolher.
  function seletorQuem(b) {
    const ops = [
      { grupo: 'Personagens' },
      { valor: 'narrador', nome: 'Narrador', rot: '🎙️ Narrador' },
      ...personagens.map((p) => ({ valor: p.slug, nome: p.nome, rot: p.nome })),
      { grupo: 'Outras vozes (sem personagem)' },
      ...vozesSistema.map((v) => ({ valor: 'voz:' + v.id, sistema: v.id, nome: v.nome.split(' · ')[0], rot: '🎭 ' + v.nome })),
    ];
    const botao = el('button', { type: 'button', class: 'quem-botao secundario' }, nomeDe(b.quem, b) + ' ▾');
    if (estado.narracao === 'narrador') { botao.disabled = true; return botao; }
    const lista = el('div', { class: 'quem-lista', hidden: true },
      ops.map((op) => op.grupo ? el('div', { class: 'quem-grupo' }, op.grupo)
        : el('div', { class: 'quem-op' + (op.valor === b.quem ? ' ativa' : '') },
            el('button', { type: 'button', class: 'pequeno secundario', title: 'Ouvir', onclick: (e) => { e.stopPropagation(); previaVoz(op, e.currentTarget); } }, '▶'),
            el('button', { type: 'button', class: 'link quem-escolher', onclick: () => {
              b.quem = op.valor; delete b.voz; if (!op.sistema) { delete b.nome; delete b.tom; delete b.velocidade; }
              lista.hidden = true; mudou(); desenhar();
            } }, op.rot))));
    const fechar = (e) => { if (!caixa.contains(e.target)) { lista.hidden = true; document.removeEventListener('click', fechar); } };
    botao.addEventListener('click', () => {
      lista.hidden = !lista.hidden;
      if (!lista.hidden) setTimeout(() => document.addEventListener('click', fechar), 0);
    });
    const caixa = el('span', { class: 'quem-caixa' }, botao, lista);
    return caixa;
  }

  let tocando = null;
  async function ouvirBloco(b, botao) {
    if (!(b.texto || '').trim()) { aviso('Bloco vazio.'); return; }
    botao.textContent = '⏳';
    try {
      const r = await fetch(location.pathname + '/bloco-audio', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ quem: chaveVoz(b), texto: b.texto }) });
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
    if (lista === estado.paginas && atual === i) atual = j;
    mudou(); desenhar();
  }

  function blocoUI(pag, i) {
    const b = pag.blocos[i];
    const sel = seletorQuem(b);
    // Voz do sistema: nome de quem fala (aparece no player).
    const sistema = b.quem.startsWith('voz:') && estado.narracao !== 'narrador';
    const rotulo = (v, sufixo) => (v > 0 ? '+' : '') + v + sufixo;
    const ajuste = (campo, min, max, sufixo, titulo) => {
      const r = el('span', { class: 'suave' }, rotulo(b[campo] || 0, sufixo));
      return el('label', { class: 'ajuste-voz', title: titulo }, titulo + ' ', r,
        el('input', { type: 'range', min, max, step: 5, value: b[campo] || 0, oninput: (e) => {
          const v = +e.target.value; if (v) b[campo] = v; else delete b[campo]; r.textContent = rotulo(v, sufixo); mudou(); } }));
    };
    const nomeFala = sistema ? el('span', { class: 'voz-sistema' },
      el('input', { class: 'nome-fala', placeholder: 'Quem fala? (ex.: Vilão)', value: b.nome || '',
        oninput: (e) => { b.nome = e.target.value; mudou(); } }),
      ajuste('tom', -60, 60, '', 'Tom'), ajuste('velocidade', -40, 40, '%', 'Velocidade')) : null;
    // Vozes extras do personagem (ex.: Inglês): escolhe qual voz este trecho usa.
    const extras = b.quem === 'narrador' || b.quem.startsWith('voz:') || estado.narracao === 'narrador' ? [] : ((personagem(b.quem) || {}).vozes_extras || []);
    const selVoz = extras.length ? el('select', { title: 'Voz deste trecho', onchange: (e) => {
      if (e.target.value) b.voz = e.target.value; else delete b.voz; mudou(); } },
      el('option', { value: '' }, '🗣️ voz normal'),
      extras.map((x) => el('option', { value: x.slug, selected: b.voz === x.slug }, '🌎 ' + x.nome))) : null;
    const txt = el('textarea', { rows: Math.max(2, Math.ceil((b.texto || '').length / 70)),
      placeholder: b.quem === 'narrador' ? 'Texto do narrador…' : 'Fala de ' + nomeDe(b.quem, b) + '…',
      oninput: (e) => { b.texto = e.target.value; mudou(); } });
    txt.value = b.texto || '';
    return el('div', { class: 'bloco' },
      avatar(b.quem),
      el('div', {},
        el('div', { class: 'cab' }, sel, selVoz, nomeFala,
          el('span', { class: 'acoes' },
            el('button', { type: 'button', title: 'Ouvir este trecho', onclick: (e) => ouvirBloco(b, e.currentTarget) }, '▶'),
            el('button', { type: 'button', title: 'Subir', onclick: () => mover(pag.blocos, i, -1) }, '↑'),
            el('button', { type: 'button', title: 'Descer', onclick: () => mover(pag.blocos, i, 1) }, '↓'),
            el('button', { type: 'button', title: 'Apagar bloco', onclick: () => { pag.blocos.splice(i, 1); mudou(); desenhar(); } }, '✕'))),
        txt));
  }

  // ───────────── Kit da imagem ─────────────
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
  // Imagem/GIF ou vídeo (mudo, em loop) da página.
  function midiaEl(m) {
    if (!m) return null;
    const src = '/midia/' + m.id;
    if (!m.tipo.startsWith('video/')) return el('img', { src, alt: '' });
    const v = el('video', { src, autoplay: true, loop: true, muted: true, playsinline: true });
    v.muted = true;
    return v;
  }
  // Área de envio: tocar só seleciona (para colar com Ctrl+V) — não abre a janela de arquivos.
  // Para procurar no computador, há o botão "Escolher arquivo" separado.
  function areaEnvio(pag) {
    const input = el('input', { type: 'file', accept: 'image/png,image/jpeg,image/webp,image/gif,video/mp4,video/webm,video/quicktime', hidden: true,
      onchange: (e) => e.target.files[0] && enviarArquivo(pag, e.target.files[0]) });
    const zona = el('div', { class: 'zona-envio', tabindex: 0, title: 'Clique aqui e cole com Ctrl+V',
      onclick: () => zona.focus(),
      ondragover: (e) => { e.preventDefault(); zona.classList.add('arrastando'); },
      ondragleave: () => zona.classList.remove('arrastando'),
      ondrop: (e) => { e.preventDefault(); zona.classList.remove('arrastando'); const f = e.dataTransfer.files[0]; if (f) enviarArquivo(pag, f); },
      onpaste: (e) => { const f = [...(e.clipboardData || {}).files || []][0]; if (f) { e.preventDefault(); enviarArquivo(pag, f); } else aviso('Não há imagem copiada para colar.'); } },
      el('span', { class: 'zona-normal' }, pag.midia ? '📋 Trocar: clique aqui e cole (Ctrl+V) ou arraste' : '📋 Clique aqui e cole (Ctrl+V) ou arraste a imagem, GIF ou vídeo'),
      el('span', { class: 'zona-pronta' }, '✅ Pronto! Agora aperte Ctrl+V'));
    return el('div', { class: 'envio' }, zona,
      el('button', { type: 'button', class: 'pequeno secundario', onclick: () => input.click() }, '📁 Escolher arquivo'), input);
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

  function lateralUI(pag, n) {
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
          el('figcaption', {}, 'Imagem ' + r.num + ' · ' + r.nome,
            r.emprestada ? [el('br'), el('small', { class: 'suave', title: 'Esta variante ainda não tem imagem: a imagem de "' + r.emprestada + '" vai só como referência de rosto e traços.' }, 'rosto de ' + r.emprestada)] : null,
            el('br'),
            el('button', { type: 'button', class: 'link', 'data-copiar-de': '/midia/' + r.ref }, '📋 Copiar'), ' · ',
            el('a', { href: '/midia/' + r.ref, download: (r.num + '-' + r.nome).replace(/\s+/g, '-') }, '⬇')))
      : el('figure', { class: 'mini sem' }, el('div', { class: 'mini-desc' }, '📝'), el('figcaption', {}, r.nome, el('br'), el('small', { class: 'suave' }, 'só descrição'))));

    const palco = el('div', { class: 'palco' + (pag.midia ? '' : ' vazio') }, midiaEl(pag.midia) || el('span', {}, '🖼️'));

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
      el('strong', {}, '🖼️ Ilustração da página ' + (n + 1)),
      el('label', {}, 'Cena', cena),
      el('div', {}, el('small', { class: 'suave' }, 'Personagens na cena'), el('div', { class: 'chips' }, naCena, addSel)),
      estado.secundarios.length ? el('div', {}, el('small', { class: 'suave' }, 'Secundários na cena'),
        el('div', { class: 'chips' },
          (pag.secundarios || []).map((nome, i) => el('span', { class: 'chip', title: (secundario(nome) || {}).descricao || '' }, '🎭 ' + nome,
            el('button', { type: 'button', class: 'link', title: 'Tirar da cena', onclick: () => { pag.secundarios.splice(i, 1); mudou(); desenhar(); } }, '✕'))),
          (() => {
            const faltam = estado.secundarios.filter((x) => !(pag.secundarios || []).includes(x.nome));
            return faltam.length ? el('select', { onchange: (e) => {
              if (!e.target.value) return;
              (pag.secundarios = pag.secundarios || []).push(e.target.value); mudou(); desenhar();
            } }, el('option', { value: '' }, '+ secundário'), faltam.map((x) => el('option', { value: x.nome }, x.nome))) : null;
          })())) : null,
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
    return el('section', { class: 'pagina ' + cor(n), id: 'pagina-' + (n + 1) },
      el('div', {},
        el('h3', {}, el('span', { class: 'num-pagina' }, '📄 Página ' + (n + 1)),
          el('span', { class: 'acoes' },
            el('button', { type: 'button', class: 'link', onclick: () => mover(estado.paginas, n, -1) }, '↑'),
            el('button', { type: 'button', class: 'link', onclick: () => mover(estado.paginas, n, 1) }, '↓'),
            el('button', { type: 'button', class: 'link', onclick: () => {
              if (confirm('Apagar a página ' + (n + 1) + '?')) {
                estado.paginas.splice(n, 1); atual = Math.max(0, Math.min(atual, estado.paginas.length - 1)); mudou(); desenhar();
              }
            } }, 'apagar'))),
        pag.blocos.map((_, i) => blocoUI(pag, i)),
        el('div', { class: 'linha' }, botoes)),
      lateralUI(pag, n));
  }

  // Carrossel: uma página por vez, com ◀ ▶ e miniaturas para pular.
  // Também é a barra de marcadores no modo lista: clicar rola até a página.
  function navCarrossel() {
    const total = estado.paginas.length;
    const ir = (i) => {
      atual = Math.max(0, Math.min(total - 1, i));
      if (modo === 'carrossel') { desenhar(); if (soltarAncora) soltarAncora(); window.scrollTo({ top: raiz.offsetTop, behavior: 'smooth' }); return; }
      if (soltarAncora) soltarAncora();
      const alvo = document.getElementById('pagina-' + (atual + 1));
      if (alvo) window.scrollTo({ top: alvo.getBoundingClientRect().top + window.scrollY - 80, behavior: 'smooth' });
      marcarAtiva();
    };
    const tiras = estado.paginas.map((p, i) => el('button', { type: 'button', title: 'Página ' + (i + 1),
      class: 'tira ' + cor(i) + (i === atual ? ' ativa' : ''), onclick: () => ir(i) },
      midiaEl(p.midia),
      el('span', {}, String(i + 1))));
    return el('div', { class: 'carrossel-nav' },
      el('button', { type: 'button', class: 'secundario', disabled: atual === 0, onclick: () => ir(atual - 1) }, '◀'),
      el('div', { class: 'tiras' }, tiras),
      el('button', { type: 'button', class: 'secundario', disabled: atual >= total - 1, onclick: () => ir(atual + 1) }, '▶'));
  }

  // Divisória "+ Página" antes da primeira, entre as páginas e depois da última.
  function inserirPaginaUI(pos) {
    const rotulo = pos === 0 ? '+ Página no início' : pos === estado.paginas.length ? '+ Página no fim' : '+ Página aqui';
    return el('div', { class: 'inserir-pagina' },
      el('button', { type: 'button', class: 'secundario pequeno', onclick: () => {
        estado.paginas.splice(pos, 0, { cena: '', personagens: [], animacao: 'nenhuma', blocos: [{ quem: 'narrador', texto: '' }] });
        atual = pos; mudou(); desenhar();
      } }, rotulo));
  }

  // "Salvar como personagem": o secundário vira personagem da família (ficha, variante e a voz usada aqui),
  // e a história passa a usar o personagem novo nas falas e nas cenas.
  function salvarComoPersonagem(x) {
    const tipos = dados.tipos || [];
    const sel = el('select', { title: 'Tipo do personagem' }, tipos.map((t) => el('option', { value: t.id, selected: t.id === 'adulto' }, t.nome)));
    const botao = el('button', { type: 'button', class: 'pequeno', onclick: async () => {
      const voz = L.vozDoSecundario(estado.paginas, x.nome);
      botao.disabled = true; botao.textContent = '⏳ Salvando…';
      try {
        const r = await fetch('/personagens/de-secundario', { method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ nome: x.nome, tipo: sel.value, descricao: x.descricao || '', voz }) });
        const j = await r.json().catch(() => ({}));
        if (!r.ok || !j.personagem) throw new Error(j.erro || 'erro ' + r.status);
        const novo = j.personagem;
        personagens.push(novo);
        L.secundarioVirouPersonagem(estado.paginas, x.nome, novo.slug);
        estado.secundarios.splice(estado.secundarios.indexOf(x), 1);
        mudou(); desenhar(); await salvar();
        aviso(novo.nome + ' agora é personagem da família ✓');
      } catch (e) {
        aviso('Não salvou: ' + e.message);
        botao.disabled = false; botao.textContent = '💾 Salvar como personagem';
      }
    } }, '💾 Salvar como personagem');
    return el('div', { class: 'linha' }, sel, botao);
  }

  // Personagens só desta história (sem ficha): nome + aparência fixa, usada nos prompts de imagem.
  function secundariosUI() {
    const lista = estado.secundarios;
    return el('details', { class: 'memoria', open: lista.some((x) => !x.descricao) },
      el('summary', {}, '🎭 Personagens só desta história (' + lista.length + ')'),
      el('p', { class: 'suave' }, 'A descrição vai igual no prompt de imagem de todas as páginas em que eles aparecem.'),
      lista.map((x, i) => el('div', { class: 'sec-item' },
        el('input', { value: x.nome, placeholder: 'Nome (ex.: Pirata Caveira)', onchange: (e) => {
          const antigo = x.nome; x.nome = e.target.value.trim() || antigo;
          L.renomearSecundario(estado.paginas, antigo, x.nome);
          mudou(); desenhar();
        } }),
        (() => { const t = el('textarea', { rows: 2, placeholder: 'Aparência: corpo, rosto, roupa, cores, acessórios…',
          oninput: (e) => { x.descricao = e.target.value; mudou(); } }); t.value = x.descricao || ''; return t; })(),
        salvarComoPersonagem(x),
        el('button', { type: 'button', class: 'link perigo', onclick: () => {
          if (!confirm('Tirar ' + x.nome + ' desta história? (as falas continuam)')) return;
          lista.splice(i, 1);
          estado.paginas.forEach((p) => { p.secundarios = (p.secundarios || []).filter((n) => n !== x.nome); });
          mudou(); desenhar();
        } }, 'remover'))),
      el('button', { type: 'button', class: 'pequeno secundario', onclick: () => {
        lista.push({ nome: 'Novo personagem ' + (lista.length + 1), descricao: '' }); mudou(); desenhar();
      } }, '＋ Adicionar'));
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
        body: JSON.stringify({ titulo: estado.titulo, narracao: estado.narracao, conteudo: { paginas: estado.paginas, secundarios: estado.secundarios } }),
      });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      sujo = false; status.textContent = 'Salvo ✓';
    } catch (e) {
      status.textContent = 'Não salvou: ' + e.message;
    }
  }

  // Modo lista: marca na barra a página que está na tela.
  function marcarAtiva() {
    document.querySelectorAll('.carrossel-nav .tira').forEach((t, i) => t.classList.toggle('ativa', i === atual));
  }
  let observador = null;
  function observarPaginas() {
    if (observador) observador.disconnect();
    if (modo !== 'lista' || !('IntersectionObserver' in window)) return;
    observador = new IntersectionObserver((itens) => {
      for (const it of itens) if (it.isIntersecting) { atual = +it.target.id.replace('pagina-', '') - 1; marcarAtiva(); }
    }, { rootMargin: '-30% 0px -60% 0px' });
    raiz.querySelectorAll('.pagina').forEach((p) => observador.observe(p));
  }

  // Âncora da tela: a página visível e onde ela estava. Redesenhar (colar imagem, remover, + Texto)
  // não pode tirar a pessoa do lugar, nem quando as imagens de cima terminam de carregar e empurram tudo.
  function pegarAncora() {
    const paginas = [...raiz.querySelectorAll('.pagina')];
    const p = paginas.find((x) => x.getBoundingClientRect().bottom > 90);
    return p ? { id: p.id, topo: p.getBoundingClientRect().top } : null;
  }
  let soltarAncora = null;
  function manterAncora(a, rolagem) {
    if (soltarAncora) soltarAncora();
    const aplicar = () => {
      const p = a && document.getElementById(a.id);
      if (p) window.scrollBy(0, p.getBoundingClientRect().top - a.topo); else window.scrollTo(0, rolagem);
    };
    aplicar();
    // Reaplica conforme as mídias carregam, por 3 s ou até a pessoa rolar a tela.
    const midias = [...raiz.querySelectorAll('img, video')];
    midias.forEach((m) => { m.addEventListener('load', aplicar); m.addEventListener('loadedmetadata', aplicar); });
    const parar = () => soltarAncora && soltarAncora();
    const eventos = ['wheel', 'touchstart', 'keydown', 'mousedown'];
    eventos.forEach((e) => window.addEventListener(e, parar, { passive: true }));
    const t = setTimeout(parar, 3000);
    soltarAncora = () => {
      clearTimeout(t);
      midias.forEach((m) => { m.removeEventListener('load', aplicar); m.removeEventListener('loadedmetadata', aplicar); });
      eventos.forEach((e) => window.removeEventListener(e, parar));
      soltarAncora = null;
    };
  }

  function desenhar() {
    const rolagem = window.scrollY;
    const ancora = pegarAncora();
    const titulo = el('input', { class: 'ed-titulo', value: estado.titulo, oninput: (e) => { estado.titulo = e.target.value; mudou(); } });
    const narr = el('select', { onchange: (e) => { estado.narracao = e.target.value; mudou(); desenhar(); } },
      el('option', { value: 'por-personagem', selected: estado.narracao !== 'narrador' }, 'Por personagem'),
      el('option', { value: 'narrador', selected: estado.narracao === 'narrador' }, 'Só narrador'));
    raiz.replaceChildren(...[
      el('div', { class: 'ed-topo' },
        el('label', {}, 'Título', titulo),
        el('label', {}, 'Narração', narr),
        el('span', { class: 'suave' }, 'Faixa ' + dados.faixa + ' anos'),
        el('div', { class: 'modo-vista' },
          ['lista', 'carrossel'].map((m) => el('button', { type: 'button', class: modo === m ? '' : 'secundario',
            onclick: () => { modo = m; try { localStorage.setItem('editor-modo', m); } catch (_) { /* */ } desenhar(); } },
            m === 'lista' ? '📜 Lista' : '🎠 Carrossel')))),
      dados.briefing ? el('p', { class: 'suave' }, el('em', {}, 'Briefing: '), dados.briefing) : null,
      (dados.continua || []).length ? el('p', { class: 'suave' }, '📚 Continua de: ',
        dados.continua.map((c, i) => [i ? ', ' : '', el('a', { href: '/historias/' + c.id }, c.titulo)])) : null,
      memoriaUI(),
      secundariosUI(),
      modo === 'carrossel' && estado.paginas.length
        ? (atual = Math.min(atual, estado.paginas.length - 1),
           [navCarrossel(), inserirPaginaUI(atual), paginaUI(estado.paginas[atual], atual), inserirPaginaUI(atual + 1), navCarrossel()])
        : [estado.paginas.length > 1 ? navCarrossel() : null, inserirPaginaUI(0), estado.paginas.map((p, n) => [paginaUI(p, n), inserirPaginaUI(n + 1)])],
      el('div', { class: 'barra-salvar' },
        el('a', { href: '/ler/' + dados.id, target: '_blank', rel: 'noopener' }, '📖 Abrir no player'),
        el('a', { href: '/compartilhar/historia/' + dados.id }, '🔗 Compartilhar'),
        el('form', { method: 'post', action: location.pathname + '/apagar', class: 'inline',
          onsubmit: (e) => { if (!confirm('Apagar esta história? As imagens e áudios dela também serão apagados (cópias em outras famílias continuam).')) e.preventDefault(); else sujo = false; } },
          el('button', { type: 'submit', class: 'link perigo' }, '🗑 Apagar')),
        statusNarracao,
        el('button', { type: 'button', class: 'secundario', onclick: gerarNarracao }, '🎙️ Gerar narração'),
        status, el('button', { type: 'button', onclick: salvar }, '💾 Salvar')),
    ].flat(Infinity).filter(Boolean));
    raiz.querySelectorAll('textarea').forEach(ajustar);
    manterAncora(ancora, rolagem);
    observarPaginas();
  }

  window.addEventListener('beforeunload', (e) => { if (sujo) { e.preventDefault(); e.returnValue = ''; } });
  document.addEventListener('keydown', (e) => { if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); salvar(); } });
  desenhar();
  verNarracao();
})();

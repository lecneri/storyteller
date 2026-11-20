// Editor de blocos: páginas → blocos (narrador / fala de personagem). Salva como JSON.
(function () {
  const { el, aviso } = window.St;
  const dados = JSON.parse(document.getElementById('dados').textContent);
  const raiz = document.getElementById('editor');
  const estado = {
    titulo: dados.titulo,
    narracao: dados.narracao,
    faixa: dados.faixa,
    paginas: (dados.conteudo && dados.conteudo.paginas) || [],
    // Personagens só desta história: [{nome, descricao}] (aparência fixa para os prompts de imagem).
    secundarios: (dados.conteudo && dados.conteudo.secundarios) || [],
    // Lugares só desta história: [{nome, descricao}]. Os da família vêm em dados.lugares.
    lugares: (dados.conteudo && dados.conteudo.lugares) || [],
  };
  const lugaresFamilia = dados.lugares || [];
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
  document.addEventListener('input', (e) => { if (e.target.matches('#editor textarea:not(.prompt):not(.fixa):not(.recolhe)')) ajustar(e.target); });
  const mudou = () => { sujo = true; status.textContent = 'Alterações não salvas'; };

  const personagem = (slug) => personagens.find((p) => p.slug === slug);
  const vozesSistema = dados.vozes_sistema || [];
  // Regras sem tela (static/editor-logica.js), com o contexto desta história.
  const L = window.EditorLogica;
  const { chaveVoz, promptAnimacao } = L;
  const nomeVoz = (id) => L.nomeVoz(vozesSistema, id);
  const miniatura = (slug, variante) => L.miniatura(personagens, slug, variante);
  const referencias = (pag) => L.referencias(personagens, pag);
  const ctxImagem = () => ({ personagens, secundarios: estado.secundarios, lugares: lugaresFamilia, lugaresHistoria: estado.lugares,
    estilo: dados.estilo, faixa: estado.faixa });
  const promptImagem = (pag) => L.promptImagem(pag, ctxImagem());
  // Imagens que vão no pedido: as dos personagens e a do lugar.
  const refsImagem = (pag) => L.refsDaPagina(pag, ctxImagem());
  function avatar(quem, pag) {
    if (quem === 'narrador') return el('div', { class: 'avatar', title: 'Narrador' }, '🎙️');
    if (quem.startsWith('voz:')) return el('div', { class: 'avatar', title: 'Voz do sistema' }, '🎭');
    // Foto da variante que está na cena desta página (ex.: Ranger Vermelho), senão a atual.
    const naCena = ((pag && pag.personagens) || []).find((n) => n.slug === quem);
    const src = miniatura(quem, naCena && naCena.variante);
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
  function seletorQuem(b, pag) {
    // Variante na cena desta página (se o personagem estiver nela).
    const cenaDe = (slug) => (pag.personagens || []).find((n) => n.slug === slug);
    const ops = [
      { grupo: 'Personagens' },
      { valor: 'narrador', nome: 'Narrador', rot: '🎙️ Narrador' },
      ...personagens.flatMap((p) => {
        const vs = (p.variantes || []).filter((v) => !v.arquivada);
        // Com mais de uma variante, cada uma aparece abaixo do personagem: escolher põe essa variante na cena.
        return [{ valor: p.slug, nome: p.nome, rot: p.nome },
          ...(vs.length > 1 ? vs.map((v) => ({ valor: p.slug, variante: v.slug, nome: p.nome, rot: '↳ ' + v.nome })) : [])];
      }),
      { grupo: 'Outras vozes (sem personagem)' },
      ...vozesSistema.map((v) => ({ valor: 'voz:' + v.id, sistema: v.id, nome: v.nome.split(' · ')[0], rot: '🎭 ' + v.nome })),
    ];
    const vNaCena = (cenaDe(b.quem) || {}).variante;
    const vNome = vNaCena && ((((personagem(b.quem) || {}).variantes || []).filter((v) => !v.arquivada).length > 1
      && ((personagem(b.quem).variantes.find((v) => v.slug === vNaCena)) || {}).nome) || '');
    const botao = el('button', { type: 'button', class: 'quem-botao secundario' }, nomeDe(b.quem, b) + (vNome ? ' · ' + vNome : '') + ' ▾');
    if (estado.narracao === 'narrador') { botao.disabled = true; return botao; }
    const lista = el('div', { class: 'quem-lista', hidden: true },
      ops.map((op) => op.grupo ? el('div', { class: 'quem-grupo' }, op.grupo)
        : el('div', { class: 'quem-op' + (op.variante ? ' quem-sub' : '') + (op.valor === b.quem
            && (op.variante ? (cenaDe(op.valor) || {}).variante === op.variante : !(cenaDe(op.valor) || {}).variante) ? ' ativa' : '') },
            el('button', { type: 'button', class: 'pequeno secundario', title: 'Ouvir', onclick: (e) => { e.stopPropagation(); previaVoz(op, e.currentTarget); } }, '▶'),
            el('button', { type: 'button', class: 'link quem-escolher', onclick: () => {
              if (b.quem !== op.valor) delete b.voz;
              b.quem = op.valor; if (!op.sistema) { delete b.nome; delete b.tom; delete b.velocidade; }
              if (op.variante) {
                const n = cenaDe(op.valor);
                if (n) n.variante = op.variante; else (pag.personagens = pag.personagens || []).push({ slug: op.valor, variante: op.variante });
              }
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
    if (b.gravacao) {
      if (tocando) tocando.pause();
      tocando = new Audio('/midia/' + b.gravacao.id);
      tocando.play().catch((e) => aviso('Não deu para ouvir: ' + e.message));
      return;
    }
    if (!(b.texto || '').trim()) { aviso('Bloco vazio.'); return; }
    botao.textContent = '⏳';
    try {
      const r = await fetch(location.pathname + '/bloco-audio', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ quem: vozDaFala(b), texto: b.texto }) });
      const j = await r.json();
      if (!r.ok) throw new Error(j.erro || 'erro');
      // Ouvir gera a voz da IA (fica guardada): a fala passa a contar como narrada.
      (narradas = narradas || new Set()).add(chaveFala(b));
      atualizarEstados();
      if (tocando) tocando.pause();
      tocando = new Audio(j.audio);
      await tocando.play();
    } catch (e) {
      aviso('Não deu para ouvir: ' + e.message);
    } finally {
      botao.textContent = '▶';
    }
  }

  // ───────────── Voz gravada (dublagem) ─────────────
  // A família grava a própria voz numa fala; ela toca no lugar da voz automática (leitor, vídeo, link).
  let gravando = null; // { b, rec, inicio, timer }
  const segundos = (d) => (Math.round(d * 10) / 10).toString().replace('.', ',') + ' s';
  async function gravarFala(b) {
    if (gravando) { if (gravando.b === b) gravando.rec.stop(); else aviso('Termine a outra gravação primeiro.'); return; }
    if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia || !window.MediaRecorder) {
      aviso('Este navegador não grava áudio.'); return;
    }
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
    } catch (e) { aviso('Sem acesso ao microfone: libere o microfone para este site e tente de novo.', 5000); return; }
    const tipo = ['audio/webm;codecs=opus', 'audio/ogg;codecs=opus', 'audio/mp4'].find((t) => MediaRecorder.isTypeSupported(t));
    const rec = new MediaRecorder(stream, tipo ? { mimeType: tipo } : undefined);
    const partes = [];
    const g = gravando = { b, rec, inicio: Date.now(), timer: null };
    rec.ondataavailable = (e) => { if (e.data.size) partes.push(e.data); };
    rec.onstop = async () => {
      stream.getTracks().forEach((t) => t.stop());
      clearInterval(g.timer); gravando = null; desenhar();
      if (Date.now() - g.inicio < 500) { aviso('Gravação curta demais.'); return; }
      const fd = new FormData();
      fd.append('arquivo', new Blob(partes, { type: rec.mimeType || 'audio/webm' }), 'fala');
      aviso('Enviando a gravação…', 10000);
      try {
        const r = await fetch(location.pathname + '/gravacao', { method: 'POST', body: fd });
        const j = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
        b.gravacao = { id: j.id, duracao: j.duracao };
        mudou(); desenhar(); await salvar(); verNarracao();
        aviso('Voz gravada ✓ (' + segundos(j.duracao) + ')');
      } catch (e) { aviso('Não salvou a gravação: ' + e.message, 5000); }
    };
    rec.start();
    g.timer = setInterval(() => {
      const t = Math.floor((Date.now() - g.inicio) / 1000);
      document.querySelectorAll('[data-gravando]').forEach((x) => { x.textContent = '⏹ ' + t + 's'; });
      if (t >= 120) rec.stop();
    }, 250);
    desenhar();
  }
  // ───────────── Estado da narração de cada fala ─────────────
  // Voz com que a fala é lida: no modo "Só narrador", tudo é o narrador (como no servidor).
  const vozDaFala = (b) => (estado.narracao === 'narrador' ? 'narrador' : chaveVoz(b));
  const chaveFala = (b) => vozDaFala(b) + '\u0000' + (b.texto || '').trim();
  let narradas = null; // Set das falas com voz da IA pronta (null = ainda não carregou)
  let linhasEstado = []; // [{ b, caixa }] da tela atual
  function preencherEstado(b, caixa) {
    const vazio = !(b.texto || '').trim();
    caixa.replaceChildren(...[
      b.gravacao ? [el('span', { class: 'estado-fala gravada' }, '🎤 Voz gravada (' + segundos(b.gravacao.duracao) + ')'),
        el('button', { type: 'button', class: 'link', onclick: async () => {
          if (!confirm('Tirar a voz gravada desta fala? Ela volta a usar a voz da IA.')) return;
          delete b.gravacao; mudou(); desenhar(); await salvar(); verNarracao();
        } }, '✕ usar a voz da IA')]
      : vazio || !narradas ? null
      : narradas.has(chaveFala(b)) ? el('span', { class: 'estado-fala ia' }, '🤖 Narrado por IA')
      : el('button', { type: 'button', class: 'pequeno secundario', title: 'Gera a voz da IA só desta fala',
          onclick: (e) => narrarFala(b, e.currentTarget) }, '🤖 Narrar com IA'),
    ].flat().filter(Boolean));
  }
  function atualizarEstados() { linhasEstado = linhasEstado.filter((x) => x.caixa.isConnected); linhasEstado.forEach((x) => preencherEstado(x.b, x.caixa)); }
  function estadoUI(b) {
    const caixa = el('div', { class: 'linha-estado' });
    linhasEstado.push({ b, caixa });
    preencherEstado(b, caixa);
    return caixa;
  }
  async function narrarFala(b, botao) {
    botao.disabled = true; botao.textContent = '⏳ Narrando…';
    try {
      const r = await fetch(location.pathname + '/bloco-audio', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ quem: vozDaFala(b), texto: b.texto }) });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      (narradas = narradas || new Set()).add(chaveFala(b));
      atualizarEstados();
      if (!sujo) verNarracao();
    } catch (e) {
      botao.disabled = false; botao.textContent = '🤖 Narrar com IA';
      aviso('Não narrou: ' + e.message);
    }
  }

  // Narração da história inteira (job no servidor) + progresso.
  const statusNarracao = el('span', { class: 'suave' }, '');
  async function verNarracao() {
    try {
      const j = await (await fetch(location.pathname + '/narracao')).json();
      narradas = new Set((j.blocos || []).filter((x) => x.audio && !x.gravada).map((x) => x.quem + '\u0000' + x.texto));
      atualizarEstados();
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
    const sel = seletorQuem(b, pag);
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
      oninput: (e) => { b.texto = e.target.value; mudou(); atualizarEstados(); } });
    txt.value = b.texto || '';
    return el('div', { class: 'bloco' },
      avatar(b.quem, pag),
      el('div', {},
        el('div', { class: 'cab' }, sel, selVoz, nomeFala,
          el('span', { class: 'acoes' },
            el('button', { type: 'button', title: b.gravacao ? 'Ouvir a voz gravada' : 'Ouvir este trecho', onclick: (e) => ouvirBloco(b, e.currentTarget) }, '▶'),
            gravando && gravando.b === b
              ? el('button', { type: 'button', class: 'gravando', title: 'Parar e usar esta gravação', 'data-gravando': true, onclick: () => gravarFala(b) },
                '⏹ ' + Math.floor((Date.now() - gravando.inicio) / 1000) + 's')
              : el('button', { type: 'button', title: b.gravacao ? 'Gravar de novo com a sua voz' : 'Gravar a sua voz nesta fala (dublar)', onclick: () => gravarFala(b) }, '🎤'),
            el('button', { type: 'button', title: 'Subir', onclick: () => mover(pag.blocos, i, -1) }, '↑'),
            el('button', { type: 'button', title: 'Descer', onclick: () => mover(pag.blocos, i, 1) }, '↓'),
            el('button', { type: 'button', title: 'Apagar bloco', onclick: () => { pag.blocos.splice(i, 1); mudou(); desenhar(); } }, '✕'))),
        txt, estadoUI(b)));
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
  // ───────────── Fila de imagens ─────────────
  // Todo pedido de imagem (página, capa, "gerar todas") vai para a fila do servidor e volta na hora.
  // O editor acompanha: mostra "⏳ Na fila (2º)" / "🎨 Desenhando…" e põe cada imagem no lugar quando fica pronta.
  // Pode fechar a aba: o servidor põe a imagem na página (ou na capa) do mesmo jeito.
  const filaImg = { itens: [], filas: [], fonte: null, carregado: false };
  const acompanhados = new Set(); // ids que o editor viu na fila (só esses são aplicados quando terminam)
  let timerFila = null;
  const ativo = (i) => i.estado === 'pendente' || i.estado === 'rodando';
  const itemDaPagina = (pag) => pag.chave ? filaImg.itens.find((i) => ativo(i) && i.alvo === 'pagina' && i.chave === pag.chave) : null;
  const itemDaCapa = () => filaImg.itens.find((i) => ativo(i) && i.alvo === 'capa');
  // Último lote ("gerar todas"): { total, feitas, falhas, ativos, pendentes }.
  function loteAtual() {
    const comLote = filaImg.itens.filter((i) => i.lote);
    if (!comLote.length) return null;
    const lote = comLote[comLote.length - 1].lote;
    const its = comLote.filter((i) => i.lote === lote);
    return { total: its.filter((i) => i.estado !== 'cancelado').length, feitas: its.filter((i) => i.estado === 'feito').length,
      falhas: its.filter((i) => i.estado === 'erro'), ativos: its.filter(ativo).length,
      pendentes: its.filter((i) => i.estado === 'pendente').length, lote };
  }
  const novaChave = () => (crypto.randomUUID ? crypto.randomUUID() : Date.now().toString(36) + Math.random().toString(36).slice(2));
  async function pedirImagem(corpo) {
    const r = await fetch(location.pathname + '/imagem-ia', { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(corpo) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
    if (j.id) acompanhados.add(j.id);
    acompanharImagens();
  }
  async function gerarPelaApi(pag, botao) {
    const refs = refsImagem(pag);
    botao.disabled = true; botao.textContent = '⏳ Pondo na fila…';
    try {
      if (!pag.chave) { pag.chave = novaChave(); mudou(); }
      botao.dataset.imgChave = pag.chave;
      if (sujo) { await salvar(); if (sujo) throw new Error('a história não salvou'); }
      await pedirImagem({ prompt: promptImagem(pag), refs, chave: pag.chave });
      aviso('Imagem na fila: ela aparece aqui quando ficar pronta (pode fechar a página).', 4000);
    } catch (e) {
      aviso('Não pediu: ' + e.message, 6000);
      botao.disabled = false; botao.textContent = '✨ Gerar imagem com IA';
    }
  }
  // "Gerar todas as imagens": uma por página, todas na fila. Cada página ganha uma "chave" que não muda.
  async function gerarTodas(botao) {
    const l = loteAtual();
    if (l && l.ativos) {
      botao.textContent = '⏳ Parando após a imagem atual…';
      await fetch(location.pathname + '/imagens/parar', { method: 'POST' }).catch(() => {});
      acompanharImagens();
      return;
    }
    let alvo = estado.paginas.filter((p) => !p.midia && !itemDaPagina(p));
    if (!alvo.length) {
      if (!confirm('Todas as páginas já têm imagem. Gerar de novo todas? (as atuais saem das páginas)')) return;
      alvo = estado.paginas.filter((p) => !itemDaPagina(p));
    } else if (!confirm('Gerar com IA as imagens de ' + alvo.length + ' página(s) sem imagem? Cada uma leva até 1 minuto. Roda no servidor: pode fechar a página e voltar depois.')) {
      return;
    }
    alvo.forEach((p) => { if (!p.chave) { p.chave = novaChave(); mudou(); } });
    try {
      if (sujo) { await salvar(); if (sujo) throw new Error('a história não salvou'); }
      const r = await fetch(location.pathname + '/imagens', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ paginas: alvo.map((pag) => ({ chave: pag.chave, prompt: promptImagem(pag),
          refs: refsImagem(pag) })) }) });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      botao.textContent = '⏳ Na fila…';
      acompanharImagens();
    } catch (e) { aviso('Não pediu as imagens: ' + e.message, 6000); }
  }
  async function trocarFonte(fonte) {
    try {
      const r = await fetch(location.pathname + '/fonte-imagem', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ fonte }) });
      if (!r.ok) throw new Error('erro ' + r.status);
      filaImg.fonte = fonte;
      aviso('As próximas imagens desta história saem por ' + ((filaImg.filas.find((f) => f.id === fonte) || {}).nome || fonte) + '.', 3000);
    } catch (e) { aviso('Não trocou: ' + e.message); }
  }
  function fonteUI() {
    if (!dados.imagem_api || filaImg.filas.length < 2) return null;
    const atual = filaImg.fonte || filaImg.filas[0].id;
    return el('label', { class: 'fonte-imagem', title: 'Quem desenha as imagens desta história (a mesma para o livro ficar com um traço só)' },
      el('small', { class: 'suave' }, '🖼️ Desenha com '),
      el('select', { onchange: (e) => trocarFonte(e.target.value) },
        filaImg.filas.map((f) => el('option', { value: f.id, selected: f.id === atual }, f.nome))));
  }
  function rotuloTodas() {
    const l = loteAtual();
    if (!l || !l.ativos) return '🖼️ Gerar todas as imagens com IA';
    return l.pendentes === l.ativos && !l.feitas ? '⏳ Na fila… (⏹ parar)' : '⏹ Parar (' + l.feitas + '/' + l.total + ' prontas)';
  }
  // Rótulos dos botões sem redesenhar a página (não tira a pessoa do lugar).
  function atualizarBotoesFila() {
    raiz.querySelectorAll('[data-img-chave]').forEach((b) => {
      const pag = estado.paginas.find((p) => p.chave && p.chave === b.dataset.imgChave);
      const it = pag && itemDaPagina(pag);
      b.disabled = !!it;
      b.textContent = it ? it.rotulo : '✨ Gerar imagem com IA';
    });
    const bc = raiz.querySelector('[data-img-capa]');
    if (bc) { const it = itemDaCapa(); bc.disabled = !!it; bc.textContent = it ? it.rotulo : '✨ Gerar capa com IA'; }
    const bt = raiz.querySelector('[data-gerar-todas]');
    if (bt) bt.textContent = rotuloTodas();
  }
  async function acompanharImagens() {
    clearTimeout(timerFila);
    let j;
    try { j = await (await fetch(location.pathname + '/imagens')).json(); } catch (_) { timerFila = setTimeout(acompanharImagens, 8000); return; }
    const antesLote = loteAtual();
    const primeiraVez = !filaImg.carregado;
    const filasMudaram = JSON.stringify(j.filas || []) !== JSON.stringify(filaImg.filas);
    Object.assign(filaImg, { itens: j.itens || [], filas: j.filas || [], fonte: j.fonte || null, carregado: true });
    let chegou = false;
    const erros = [];
    for (const it of filaImg.itens) {
      if (ativo(it)) { acompanhados.add(it.id); continue; }
      if (!acompanhados.has(it.id)) continue;
      acompanhados.delete(it.id);
      if (it.estado === 'feito' && it.midia) {
        if (it.alvo === 'capa') { capa = it.midia.id; chegou = true; }
        const pag = it.alvo === 'pagina' && estado.paginas.find((p) => p.chave === it.chave);
        if (pag && (!pag.midia || pag.midia.id !== it.midia.id)) { pag.midia = it.midia; chegou = true; }
        if (!it.lote) aviso(it.alvo === 'capa' ? 'Capa nova pronta ✓' : 'Imagem pronta ✓', 3000);
      } else if (it.estado === 'erro' && !it.lote) erros.push(it.erro);
    }
    if (erros.length) aviso('A imagem não saiu: ' + erros.join('; '), 10000);
    const l = loteAtual();
    if (antesLote && antesLote.ativos && l && l.lote === antesLote.lote && !l.ativos) {
      const f = l.falhas.map((x) => x.erro).filter(Boolean);
      aviso(l.feitas + ' imagem(ns) gerada(s)' + (f.length ? '. Falharam ' + f.length + ': ' + f.join('; ') : ' ✓'), f.length ? 10000 : 3000);
    }
    if (chegou || (filasMudaram && primeiraVez) || (primeiraVez && filaImg.itens.some(ativo))) desenhar();
    else atualizarBotoesFila();
    if (filaImg.itens.some(ativo)) timerFila = setTimeout(acompanharImagens, 3500);
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
    const cena = el('textarea', { class: 'recolhe', title: 'Clique para ver e editar a cena inteira', placeholder: 'O que aparece na ilustração desta página…',
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
    const caixaPrompt = el('textarea', { readonly: true, class: 'prompt', title: 'Clique para ver o prompt inteiro' });
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
      dados.imagem_api ? (() => { const it = itemDaPagina(pag);
        return el('button', { type: 'button', 'data-img-chave': pag.chave || '', disabled: !!it, onclick: (e) => gerarPelaApi(pag, e.currentTarget) },
          it ? it.rotulo : '✨ Gerar imagem com IA'); })() : null,
      areaEnvio(pag),
      pag.midia && pag.midia.tipo.startsWith('image/') ? el('div', { class: 'linha-kit' },
        el('span', {}, '🎬 Animar esta imagem'),
        el('button', { type: 'button', class: 'pequeno secundario', onclick: () => copiarTexto(promptAnimacao(pag)) }, '📋 Prompt de animação')) : null);

    return el('aside', { class: 'lateral' },
      el('strong', {}, '🖼️ Ilustração da página ' + (n + 1)),
      palco,
      el('label', {}, 'Cena', cena),
      el('div', {}, el('small', { class: 'suave' }, 'Personagens na cena'), el('div', { class: 'chips' }, naCena, addSel)),
      lugarUI(pag),
      musicaPaginaUI(pag),
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
      if (modo === 'carrossel') { desenhar(); fixarPagina(atual); return; }
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

  // Carrossel: deixa a página escolhida logo abaixo da barra de miniaturas (que fica presa no topo),
  // com a divisória "+ Página aqui" à vista, e a mantém ali enquanto as imagens carregam.
  function fixarPagina(i) {
    const p = document.getElementById('pagina-' + (i + 1));
    const nav = raiz.querySelector('.carrossel-nav');
    if (!p || !nav) return;
    const antes = p.previousElementSibling;
    const folga = antes ? p.getBoundingClientRect().top - antes.getBoundingClientRect().top : 0;
    manterAncora({ id: p.id, topo: nav.offsetHeight + 8 + folga }, window.scrollY);
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
    const sugerido = L.adivinharTipo(x);
    const sel = el('select', { title: 'Tipo do personagem' }, tipos.map((t) => el('option', { value: t.id, selected: t.id === sugerido }, t.nome)));
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

  // "É o personagem…": o secundário passa a ser um personagem que a família já tem (ex.: "Papai" → Papai Dudu).
  // Falas usam a voz do personagem, as cenas passam a levar a imagem dele.
  function associarAPersonagem(x) {
    if (!personagens.length) return null;
    const norm = (s) => (s || '').normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase().trim();
    const parecido = personagens.find((p) => norm(p.nome) === norm(x.nome) || ((p.ficha || {}).apelidos || []).some((a) => norm(a) === norm(x.nome)));
    const sel = el('select', { title: 'Personagem da família' },
      el('option', { value: '' }, '🔗 É o personagem…'),
      personagens.map((p) => el('option', { value: p.slug }, p.nome + (parecido === p ? ' (parece ser)' : ''))));
    sel.addEventListener('change', async () => {
      const p = personagens.find((q) => q.slug === sel.value);
      if (!p) return;
      if (!confirm('“' + x.nome + '” desta história passa a ser ' + p.nome + '? As falas usam a voz de ' + p.nome + ' e as cenas levam a imagem dele.')) { sel.value = ''; return; }
      L.secundarioVirouPersonagem(estado.paginas, x.nome, p.slug);
      // Sem repetir o personagem na mesma cena.
      for (const pg of estado.paginas) {
        const vistos = new Set();
        pg.personagens = (pg.personagens || []).filter((n) => !vistos.has(n.slug) && vistos.add(n.slug));
      }
      estado.secundarios.splice(estado.secundarios.indexOf(x), 1);
      mudou(); desenhar(); await salvar();
      aviso(x.nome + ' agora é ' + p.nome + ' ✓');
    });
    return sel;
  }

  // Personagens só desta história (sem ficha): nome + aparência fixa, usada nos prompts de imagem.
  // 📍 Onde a cena acontece: um lugar da família (com imagem) ou um só desta história.
  // 🎵 Música desta página: continuar (padrão), trocar (a partir daqui), voltar à geral ou silêncio.
  const MODOS_MUSICA = [['', '▶️ Continuar a que está tocando'], ['trocar', '🔁 Trocar a partir daqui'], ['voltar', '↩️ Voltar para a música geral'], ['silencio', '🔇 Silêncio']];
  function musicaTocando(pag) {
    // O que toca nesta página, seguindo as anteriores (igual ao leitor).
    let atual = musica ? 'música geral' : null;
    for (const p of estado.paginas) {
      const m = p.musica;
      if (m && m.modo === 'trocar' && m.id) atual = m.nome || 'música da página';
      else if (m && m.modo === 'voltar') atual = musica ? 'música geral' : null;
      else if (m && m.modo === 'silencio') atual = null;
      if (p === pag) break;
    }
    return atual;
  }
  function musicaPaginaUI(pag) {
    const m = pag.musica || {};
    const modo = el('select', { onchange: (e) => {
      const v = e.target.value;
      if (!v) delete pag.musica;
      else if (v === 'trocar') pag.musica = { modo: 'trocar', id: m.id, nome: m.nome, credito: m.credito };
      else pag.musica = { modo: v };
      mudou(); desenhar();
    } }, MODOS_MUSICA.map(([v, n]) => el('option', { value: v, selected: (m.modo || '') === v }, n)));
    const tocando = musicaTocando(pag);
    const info = el('small', { class: 'suave' }, '🎧 Nesta página: ' + (tocando || 'sem música'));
    let escolha = null;
    if (m.modo === 'trocar') {
      const status = el('small', { class: 'suave' });
      const usar = async (url, opcoes) => {
        status.textContent = 'Preparando a música…';
        try {
          const r = await fetch(location.pathname + url, opcoes);
          const j = await r.json().catch(() => ({}));
          if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
          pag.musica = { modo: 'trocar', id: j.id, nome: j.nome, credito: j.credito || undefined };
          mudou(); desenhar();
        } catch (err) { status.textContent = 'Não deu: ' + err.message; }
      };
      const modelos = dados.musicas_modelo || [];
      const lista = modelos.length ? el('select', { onchange: (e) => {
        if (e.target.value) usar('/musica/pagina-modelo', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ modelo: e.target.value }) });
      } }, el('option', { value: '' }, '🎼 Música da plataforma…'),
        (dados.climas || []).map((c) => {
          const doClima = modelos.filter((x) => x.clima === c.id);
          return doClima.length ? el('optgroup', { label: c.nome }, doClima.map((x) => el('option', { value: x.id }, x.titulo))) : null;
        })) : null;
      const arquivo = el('input', { type: 'file', accept: 'audio/*', hidden: true, onchange: (e) => {
        const f = e.target.files[0]; if (!f) return;
        const fd = new FormData(); fd.append('arquivo', f);
        usar('/musica/pagina', { method: 'POST', body: fd });
      } });
      escolha = el('div', { class: 'musica-pagina' },
        m.id ? el('div', {}, el('strong', {}, '🎵 ' + (m.nome || 'Música')), m.credito ? el('small', { class: 'suave' }, ' · ' + m.credito) : null,
          el('audio', { src: '/midia/' + m.id, controls: true, preload: 'none' })) : el('small', {}, 'Escolha a música:'),
        el('div', { class: 'linha' }, lista,
          el('button', { type: 'button', class: 'pequeno secundario', onclick: () => arquivo.click() }, '📁 Enviar arquivo'), arquivo),
        status);
    }
    return el('div', { class: 'musica-pagina-ed' }, el('small', { class: 'suave' }, '🎵 Música'), modo, escolha, info);
  }

  function lugarUI(pag) {
    if (!lugaresFamilia.length && !estado.lugares.length && !pag.lugar) return null;
    const l = L.lugarDaPagina(pag, ctxImagem());
    const perdido = pag.lugar && !l;
    const sel = el('select', { onchange: (e) => { pag.lugar = e.target.value; mudou(); desenhar(); } },
      el('option', { value: '' }, '(nenhum: a cena descreve)'),
      lugaresFamilia.length ? el('optgroup', { label: 'Lugares da família' },
        lugaresFamilia.map((x) => el('option', { value: x.slug, selected: x.slug === pag.lugar }, x.nome))) : null,
      estado.lugares.length ? el('optgroup', { label: 'Só desta história' },
        estado.lugares.map((x) => el('option', { value: x.nome, selected: x.nome === pag.lugar }, x.nome))) : null,
      perdido ? el('option', { value: pag.lugar, selected: true }, pag.lugar + ' (não encontrado)') : null);
    return el('div', { class: 'lugar-pagina' },
      el('small', { class: 'suave' }, '📍 Lugar'),
      el('div', { class: 'linha' },
        l && l.ref ? el('img', { src: '/midia/' + l.ref, alt: '', class: 'mini-lugar', title: l.descricao }) : null,
        sel));
  }

  // Lugares só desta história: descrição fixa (vai no prompt de toda página que se passa ali) e "Salvar como lugar".
  function lugaresUI() {
    const lista = estado.lugares;
    return el('details', { class: 'memoria' },
      el('summary', {}, '📍 Lugares só desta história (' + lista.length + ')'),
      el('p', { class: 'suave' }, 'A descrição vai igual no prompt de imagem de todas as páginas que se passam ali. ' +
        'Para usar em outras histórias, salve como lugar da família (aparece em Lugares, com imagem).'),
      lista.map((x, i) => el('div', { class: 'sec-item' },
        el('input', { value: x.nome, placeholder: 'Nome (ex.: Caverna do Zorgo)', onchange: (e) => {
          const antigo = x.nome; x.nome = e.target.value.trim() || antigo;
          estado.paginas.forEach((p) => { if (p.lugar === antigo) p.lugar = x.nome; });
          mudou(); desenhar();
        } }),
        (() => { const t = el('textarea', { rows: 2, placeholder: 'Como é o lugar: formato, cores, objetos marcantes…',
          oninput: (e) => { x.descricao = e.target.value; mudou(); } }); t.value = x.descricao || ''; return t; })(),
        el('div', { class: 'linha' },
          el('button', { type: 'button', class: 'pequeno', onclick: async (e) => {
            const b = e.currentTarget; b.disabled = true; b.textContent = '⏳ Salvando…';
            try {
              const r = await fetch('/lugares/da-historia', { method: 'POST', headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ nome: x.nome, descricao: x.descricao || '' }) });
              const j = await r.json().catch(() => ({}));
              if (!r.ok || !j.slug) throw new Error(j.erro || 'erro ' + r.status);
              lugaresFamilia.push(j);
              estado.paginas.forEach((p) => { if (p.lugar === x.nome) p.lugar = j.slug; });
              lista.splice(lista.indexOf(x), 1);
              mudou(); desenhar(); await salvar();
              aviso(j.nome + ' agora é lugar da família ✓ (em Lugares dá para gerar a imagem dele)', 5000);
            } catch (err) { aviso('Não salvou: ' + err.message); b.disabled = false; b.textContent = '💾 Salvar como lugar'; }
          } }, '💾 Salvar como lugar'),
          el('button', { type: 'button', class: 'link perigo', onclick: () => {
            if (!confirm('Tirar ' + x.nome + ' desta história?')) return;
            lista.splice(i, 1);
            estado.paginas.forEach((p) => { if (p.lugar === x.nome) p.lugar = ''; });
            mudou(); desenhar();
          } }, 'remover')))),
      el('button', { type: 'button', class: 'pequeno secundario', onclick: () => {
        lista.push({ nome: 'Novo lugar ' + (lista.length + 1), descricao: '' }); mudou(); desenhar();
      } }, '＋ Adicionar'));
  }

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
        associarAPersonagem(x),
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

  // Referência: resumo usado quando outra história cita esta com #.
  function referenciaUI() {
    const ta = el('textarea', { rows: 3, placeholder: 'Resumo com os fatos importantes (quem venceu, o que descobriram...). Usado quando outra história citar esta com #.' });
    ta.value = dados.referencia || '';
    const st = el('small', { class: 'suave' });
    return el('details', { class: 'memoria' }, el('summary', {}, '📌 Referência' + (dados.referencia ? '' : ' (vazia)')),
      ta,
      el('div', { class: 'linha' },
        el('button', { type: 'button', class: 'pequeno secundario', onclick: async (e) => {
          e.target.disabled = true; st.textContent = 'Gerando com IA…';
          const r = await fetch(location.pathname + '/referencia', { method: 'POST' });
          const j = await r.json().catch(() => ({}));
          e.target.disabled = false;
          if (r.ok) { ta.value = dados.referencia = j.referencia; st.textContent = 'Gerada ✓'; } else st.textContent = 'Erro: ' + (j.erro || r.status);
        } }, '✨ Gerar com IA'),
        el('button', { type: 'button', class: 'pequeno', onclick: async () => {
          const r = await fetch(location.pathname + '/referencia/salvar', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ referencia: ta.value }) });
          if (r.ok) { dados.referencia = ta.value; st.textContent = 'Salva ✓'; }
        } }, '💾 Salvar referência'), st));
  }

  // Memória: marcações (uma por linha) gravadas pela IA quando o briefing pede, ou à mão.
  // Outras histórias usam com #esta-historia:memoria, #memoria ou a caixa "usar a memória da estante".
  function memoriaUI() {
    const itens = () => (Array.isArray(dados.memoria) ? dados.memoria : []);
    const ta = el('textarea', { rows: 4, placeholder: 'Uma marcação por linha, ex.:\napple (maçã)\nred (vermelho)' });
    ta.value = itens().join('\n');
    const st = el('small', { class: 'suave' });
    const oque = el('input', { placeholder: 'O que gravar? ex.: as palavras em inglês ensinadas' });
    // Soma itens ao que está no campo (inclusive o que foi digitado e ainda não salvo), sem repetir.
    function somar(novos) {
      const atuais = ta.value.split('\n').map((l) => l.trim()).filter(Boolean);
      const juntos = atuais.slice();
      novos.forEach((i) => { if (!juntos.some((x) => x.toLowerCase() === i.toLowerCase())) juntos.push(i); });
      ta.value = juntos.join('\n');
      st.textContent = (juntos.length - atuais.length) + ' novo(s). Revise e clique em 💾 Salvar memória.';
    }
    // Resposta de outra IA (depois de 📋 Copiar prompt).
    const resposta = el('textarea', { rows: 3, placeholder: 'Cole aqui a resposta da outra IA (começa com { "itens": ...)' });
    const colar = el('div', { hidden: '' }, resposta,
      el('button', { type: 'button', class: 'pequeno', onclick: async () => {
        const r = await fetch(location.pathname + '/memoria/importar', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ texto: resposta.value }) });
        const j = await r.json().catch(() => ({}));
        if (r.ok) { somar(j.itens); resposta.value = ''; } else st.textContent = 'Erro: ' + (j.erro || r.status);
      } }, '📥 Importar resposta'));
    return el('details', { class: 'memoria' }, el('summary', {}, '🧠 Memória' + (itens().length ? ' (' + itens().length + ')' : ' (vazia)')),
      ta,
      el('div', { class: 'linha' }, oque,
        el('button', { type: 'button', class: 'pequeno secundario', onclick: async (e) => {
          if (!oque.value.trim()) { st.textContent = 'Escreva o que gravar.'; oque.focus(); return; }
          e.target.disabled = true; st.textContent = 'A IA está lendo a história…';
          const r = await fetch(location.pathname + '/memoria/gerar', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ pedido: oque.value }) });
          const j = await r.json().catch(() => ({}));
          e.target.disabled = false;
          if (r.ok) somar(j.itens); else st.textContent = 'Erro: ' + (j.erro || r.status);
        } }, '✨ Gravar com IA'),
        el('button', { type: 'button', class: 'pequeno secundario', title: 'Para usar em outra IA (ChatGPT, Gemini, Claude): leva a história e o pedido', onclick: async () => {
          if (!oque.value.trim()) { st.textContent = 'Escreva o que gravar.'; oque.focus(); return; }
          const r = await fetch(location.pathname + '/memoria/prompt', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ pedido: oque.value }) });
          const j = await r.json().catch(() => ({}));
          if (!r.ok) { st.textContent = 'Erro: ' + (j.erro || r.status); return; }
          try { await navigator.clipboard.writeText(j.prompt); st.textContent = '📋 Prompt copiado. Cole na outra IA e traga a resposta abaixo.'; }
          catch (_) { window.prompt('Copie o prompt:', j.prompt); }
          colar.hidden = false;
        } }, '📋 Copiar prompt')),
      colar,
      el('div', { class: 'linha' },
        el('button', { type: 'button', class: 'pequeno', onclick: async () => {
          const lista = ta.value.split('\n').map((l) => l.trim()).filter(Boolean);
          const r = await fetch(location.pathname + '/memoria/salvar', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ itens: lista }) });
          const j = await r.json().catch(() => ({}));
          if (r.ok) { dados.memoria = j.itens; ta.value = j.itens.join('\n'); st.textContent = 'Salva ✓'; } else st.textContent = 'Erro: ' + (j.erro || r.status);
        } }, '💾 Salvar memória'), st));
  }

  // ───────────── Ajuda da IA para editar ─────────────
  // A IA devolve a história inteira ajustada; o editor mostra, e a família salva ou desfaz.
  const ajuda = { aberto: false, pedido: '', msg: '', ocupado: false, externo: false, prompt: '', resposta: '', backup: null };
  function aplicarAjuda(r) {
    ajuda.backup = JSON.parse(JSON.stringify({ titulo: estado.titulo, paginas: estado.paginas, secundarios: estado.secundarios, lugares: estado.lugares }));
    if (r.titulo) estado.titulo = r.titulo;
    estado.paginas = (r.conteudo && r.conteudo.paginas) || [];
    estado.secundarios = (r.conteudo && r.conteudo.secundarios) || [];
    estado.lugares = (r.conteudo && r.conteudo.lugares) || [];
    ajuda.msg = 'Pronto! Confira as páginas e clique em 💾 Salvar. Não gostou? Use ↩ Desfazer.';
    ajuda.externo = false; ajuda.resposta = '';
    mudou(); desenhar();
    aviso('A IA ajustou a história. Confira e salve.', 4000);
  }
  async function antesDePedir() {
    if (!ajuda.pedido.trim()) { aviso('Escreva o que a IA deve mudar.'); return false; }
    if (sujo) { await salvar(); if (sujo) { aviso('Salve a história antes de pedir ajuda.'); return false; } }
    return true;
  }
  async function pedirAjuda() {
    if (ajuda.ocupado || !(await antesDePedir())) return;
    ajuda.ocupado = true; ajuda.msg = '⏳ A IA está ajustando a história (de 30 s a 2 min)…'; desenhar();
    try {
      const r = await fetch(location.pathname + '/ajuda-ia', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ pedido: ajuda.pedido }) });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      const fim = Date.now() + 8 * 60 * 1000;
      for (;;) {
        await new Promise((ok) => setTimeout(ok, 3000));
        const v = await fetch(location.pathname + '/ajuda-ia/' + j.job).then((x) => x.json()).catch(() => ({}));
        if (v.estado === 'feito') { ajuda.ocupado = false; aplicarAjuda(v.resultado || {}); return; }
        if (v.estado === 'erro') throw new Error(v.erro || 'falhou');
        if (Date.now() > fim) throw new Error('demorou demais');
      }
    } catch (e) {
      ajuda.msg = 'Não deu: ' + e.message + '. Tente de novo ou use outra IA.';
    }
    ajuda.ocupado = false; desenhar();
  }
  async function promptAjuda() {
    if (ajuda.ocupado || !(await antesDePedir())) return;
    ajuda.ocupado = true; ajuda.msg = 'Montando o prompt…'; desenhar();
    try {
      const r = await fetch(location.pathname + '/ajuda-ia', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ pedido: ajuda.pedido, modo: 'prompt' }) });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      ajuda.prompt = j.prompt; ajuda.externo = true;
      ajuda.msg = 'Copie o prompt, cole numa IA e traga a resposta.';
    } catch (e) {
      ajuda.msg = 'Não deu: ' + e.message;
    }
    ajuda.ocupado = false; desenhar();
  }
  async function importarAjuda(botao) {
    if (!ajuda.resposta.trim()) { aviso('Cole a resposta da IA ou escolha o arquivo.'); return; }
    botao.disabled = true; botao.textContent = '⏳ Importando…';
    try {
      const r = await fetch(location.pathname + '/ajuda-ia/importar', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ texto: ajuda.resposta }) });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      aplicarAjuda(j);
    } catch (e) {
      aviso(e.message, 6000);
      botao.disabled = false; botao.textContent = '📥 Importar resposta';
    }
  }
  function ajudaUI() {
    const pedido = el('textarea', { rows: 3, maxlength: 2000, placeholder: 'Ex.: deixe a página 3 mais engraçada; troque "cachorro" por "dog" nas falas do Noob; acrescente uma página no final com o Pedro dormindo.' });
    pedido.value = ajuda.pedido;
    pedido.addEventListener('input', () => { ajuda.pedido = pedido.value; });
    let externo = null;
    if (ajuda.externo) {
      const caixa = el('textarea', { readonly: true, class: 'prompt', id: 'prompt-ajuda', title: 'Clique para ver o prompt inteiro' });
      caixa.value = ajuda.prompt;
      const resp = el('textarea', { class: 'fixa', placeholder: 'Cole aqui a resposta inteira da IA (começa com { "titulo": ...)' });
      resp.value = ajuda.resposta;
      resp.addEventListener('input', () => { ajuda.resposta = resp.value; });
      const arq = el('input', { type: 'file', accept: '.json,.txt,application/json,text/plain', 'data-sem-colar': true, onchange: async (e) => {
        const f = e.target.files[0];
        if (!f) return;
        if (f.size > 2000000) { aviso('Arquivo grande demais.'); return; }
        resp.value = ajuda.resposta = await f.text();
      } });
      externo = el('div', { class: 'kit' },
        el('div', { class: 'linha-kit' }, el('span', {}, '1. Copie o prompt'),
          el('button', { type: 'button', class: 'pequeno', onclick: () => copiarTexto(ajuda.prompt) }, '📋 Copiar prompt')),
        caixa,
        el('div', { class: 'linha-kit' }, el('span', {}, '2. Cole numa IA:'),
          el('a', { href: 'https://chatgpt.com/', target: '_blank', rel: 'noopener' }, 'ChatGPT ↗'),
          el('a', { href: 'https://gemini.google.com/app', target: '_blank', rel: 'noopener' }, 'Gemini ↗'),
          el('a', { href: 'https://claude.ai/new', target: '_blank', rel: 'noopener' }, 'Claude ↗')),
        el('span', {}, '3. Cole a resposta ou envie o arquivo (.json ou .txt):'),
        resp,
        el('div', { class: 'linha' }, arq,
          el('button', { type: 'button', onclick: (e) => importarAjuda(e.currentTarget) }, '📥 Importar resposta')));
    }
    const det = el('details', { class: 'memoria', open: ajuda.aberto }, el('summary', {}, '🤖 Pedir ajuda à IA para editar'),
      el('small', { class: 'suave' }, 'Diga o que mudar. A IA devolve a história ajustada aqui no editor; as imagens das páginas continuam. Nada é salvo até você clicar em 💾 Salvar.'),
      pedido,
      el('div', { class: 'linha' },
        el('button', { type: 'button', class: 'pequeno', disabled: ajuda.ocupado, onclick: pedirAjuda }, '✨ Pedir à IA'),
        el('button', { type: 'button', class: 'pequeno secundario', disabled: ajuda.ocupado, onclick: promptAjuda }, '📋 Usar outra IA'),
        ajuda.backup ? el('button', { type: 'button', class: 'pequeno secundario', onclick: () => {
          Object.assign(estado, ajuda.backup); ajuda.backup = null; ajuda.msg = 'Desfeito.'; mudou(); desenhar();
        } }, '↩ Desfazer') : null),
      ajuda.msg ? el('small', { class: 'suave' }, ajuda.msg) : null,
      externo);
    det.addEventListener('toggle', () => { ajuda.aberto = det.open; });
    return det;
  }

  const status = el('span', { class: 'suave' }, '');
  async function salvar() {
    status.textContent = 'Salvando…';
    try {
      const r = await fetch(location.pathname + '/conteudo', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ titulo: estado.titulo, narracao: estado.narracao, faixa: estado.faixa, conteudo: { paginas: estado.paginas, secundarios: estado.secundarios, lugares: estado.lugares } }),
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

  // ───────────── Capa ─────────────
  // Sem capa escolhida, vale a primeira imagem do livro (como sempre foi).
  let capa = dados.capa || null;
  const capaUi = { aberto: false };
  async function definirCapa(midia, msg) {
    const r = await fetch(location.pathname + '/capa', { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ midia }) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
    capa = midia; desenhar(); aviso(msg);
  }
  // Página "de mentira" para montar o prompt da capa: todos os personagens do livro, cada um com a 1ª variante usada.
  function paginaDaCapa() {
    const vistos = new Map();
    for (const pg of estado.paginas) for (const n of pg.personagens || []) if (!vistos.has(n.slug)) vistos.set(n.slug, n);
    return {
      cena: 'Capa do livro "' + estado.titulo + '". Os personagens principais juntos, em destaque, numa cena alegre que resume a história' +
        (estado.paginas[0] && estado.paginas[0].cena ? ' (começo: ' + estado.paginas[0].cena + ')' : '') +
        '. Deixe espaço livre no alto para o título, que entra depois.',
      personagens: [...vistos.values()].slice(0, 5),
    };
  }
  function capaUI() {
    const comImagem = estado.paginas.map((p, i) => ({ p, i })).filter(({ p }) => p.midia && (p.midia.tipo || '').startsWith('image/'));
    const padrao = comImagem[0];
    const src = capa ? '/midia/' + capa : padrao ? '/midia/' + padrao.p.midia.id : null;
    const pagCapa = paginaDaCapa();
    const prompt = promptImagem(pagCapa);
    const enviar = el('input', { type: 'file', accept: 'image/png,image/jpeg,image/webp', hidden: true, onchange: async (e) => {
      const f = e.target.files[0]; if (!f) return;
      const fd = new FormData(); fd.append('arquivo', f);
      aviso('Enviando…');
      try {
        const r = await fetch(location.pathname + '/midia', { method: 'POST', body: fd });
        const j = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
        await definirCapa(j.id, 'Capa nova salva ✓');
      } catch (err) { aviso('Não enviou: ' + err.message); }
    } });
    const gerar = async (botao) => {
      const refs = refsImagem(pagCapa);
      botao.disabled = true; botao.textContent = '⏳ Pondo na fila…';
      try {
        await pedirImagem({ prompt, refs, capa: true });
        aviso('Capa na fila: ela aparece aqui quando ficar pronta (pode fechar a página).', 4000);
      } catch (err) {
        botao.disabled = false; botao.textContent = '✨ Gerar capa com IA';
        aviso('Não pediu: ' + err.message, 6000);
      }
    };
    const det = el('details', { class: 'memoria capa-ed', open: capaUi.aberto },
      el('summary', {}, '🖼️ Capa', el('small', { class: 'suave' }, capa ? ' · imagem própria' : padrao ? ' · usando a página ' + (padrao.i + 1) : ' · sem imagem ainda')),
      el('div', { class: 'capa-corpo' },
        src ? el('img', { src, alt: 'Capa', class: 'capa-previa' }) : el('div', { class: 'capa-previa capa-vazia' }, '📖'),
        el('div', { class: 'capa-acoes' },
          el('p', { class: 'suave' }, capa ? 'Esta é a capa do livro (lista, leitor, PDF e vídeo).'
            : 'Sem capa própria, a capa é a primeira imagem do livro. Escolha outra se quiser:'),
          el('div', { class: 'linha' },
            el('button', { type: 'button', class: 'pequeno secundario', onclick: () => enviar.click() }, '📤 Enviar imagem'), enviar,
            dados.imagem_api ? (() => { const it = itemDaCapa();
              return el('button', { type: 'button', class: 'pequeno', 'data-img-capa': true, disabled: !!it, onclick: (e) => gerar(e.currentTarget) },
                it ? it.rotulo : '✨ Gerar capa com IA'); })() : null),
          comImagem.length > 1 ? el('select', { onchange: (e) => {
            const i = +e.target.value; if (Number.isNaN(i) || e.target.value === '') return;
            definirCapa(estado.paginas[i].midia.id, 'Capa: imagem da página ' + (i + 1) + ' ✓').catch((err) => aviso('Não salvou: ' + err.message));
          } }, el('option', { value: '' }, 'Usar a imagem de uma página…'),
            comImagem.map(({ p, i }) => el('option', { value: i, selected: capa === p.midia.id }, 'Página ' + (i + 1)))) : null,
          capa ? el('button', { type: 'button', class: 'link', onclick: () =>
            definirCapa(null, 'Capa volta a ser a primeira imagem ✓').catch((err) => aviso('Não salvou: ' + err.message)) },
            '↩ Voltar a usar a primeira imagem') : null,
          el('details', {}, el('summary', { class: 'suave' }, '📋 Prompt da capa (para usar em outra IA)'),
            el('textarea', { class: 'prompt', readonly: true, 'data-sem-colar': true }, prompt)))));
    det.addEventListener('toggle', () => { capaUi.aberto = det.open; });
    return det;
  }

  // ───────────── Música de fundo (opcional) ─────────────
  let musica = dados.musica || null;
  const musicaUi = { aberto: false, ocupado: false };
  async function ajustarMusica(corpo) {
    const r = await fetch(location.pathname + '/musica/ajustar', { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(corpo) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
  }
  function musicaUI() {
    const enviar = el('input', { type: 'file', accept: 'audio/*', hidden: true, onchange: async (e) => {
      const f = e.target.files[0]; if (!f) return;
      const fd = new FormData(); fd.append('arquivo', f);
      musicaUi.ocupado = true; desenhar(); aviso('Preparando a música (corta silêncio, ajusta volume)…');
      try {
        const r = await fetch(location.pathname + '/musica', { method: 'POST', body: fd });
        const j = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
        musica = { id: j.id, volume: musica ? musica.volume : 0.35 };
        aviso('Música salva ✓');
      } catch (err) { aviso('Não enviou: ' + err.message); }
      musicaUi.ocupado = false; desenhar();
    } });
    // Modelos da plataforma, por clima; a idade indicada aparece no nome (quem escolhe é quem cria).
    const modelos = dados.musicas_modelo || [];
    const previaModelo = el('audio', { controls: true, preload: 'none', hidden: true });
    const usarModelo = el('button', { type: 'button', class: 'pequeno', hidden: true, disabled: musicaUi.ocupado, onclick: async () => {
      const id = escolhaModelo.value; if (!id) return;
      musicaUi.ocupado = true; previaModelo.pause();
      try {
        const r = await fetch(location.pathname + '/musica/modelo', { method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ modelo: id }) });
        const j = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
        const m = modelos.find((x) => x.id === id);
        musica = { id: j.id, volume: musica ? musica.volume : 0.35, credito: m && m.credito };
        aviso('Música "' + (m ? m.titulo : '') + '" na história ✓');
      } catch (err) { aviso('Não salvou: ' + err.message); }
      musicaUi.ocupado = false; desenhar();
    } }, '✓ Usar esta');
    const descModelo = el('small', { class: 'suave' });
    const escolhaModelo = el('select', { onchange: (e) => {
      const m = modelos.find((x) => x.id === e.target.value);
      previaModelo.hidden = usarModelo.hidden = !m;
      descModelo.textContent = m ? [m.descricao, m.credito ? 'Crédito: ' + m.credito : ''].filter(Boolean).join(' · ') : '';
      if (m) { previaModelo.src = '/musicas-modelo/' + m.id; previaModelo.play().catch(() => {}); } else previaModelo.pause();
    } }, el('option', { value: '' }, '🎼 Escolher uma música da plataforma…'),
      (dados.climas || []).map((c) => {
        const doClima = modelos.filter((m) => m.clima === c.id);
        return doClima.length ? el('optgroup', { label: c.nome }, doClima.map((m) => el('option', { value: m.id },
          m.titulo + ' (' + Math.floor(m.duracao / 60) + ':' + String(Math.round(m.duracao % 60)).padStart(2, '0') + ')' + (m.idade_min ? ' · ' + m.idade_min + '+ anos' : '')))) : null;
      }));
    const modelosUI = modelos.length ? el('div', { class: 'musica-modelos' },
      el('div', { class: 'linha' }, escolhaModelo, usarModelo), previaModelo, descModelo) : null;
    const previa = musica ? el('audio', { src: '/midia/' + musica.id, controls: true, loop: true, preload: 'none' }) : null;
    if (previa) previa.volume = musica.volume;
    const rotulo = el('span', {}, musica ? Math.round(musica.volume * 100) + '%' : '');
    const det = el('details', { class: 'memoria musica-ed', open: musicaUi.aberto },
      el('summary', {}, '🎵 Música de fundo', el('small', { class: 'suave' }, musica ? ' · ligada' : ' · sem música')),
      el('p', { class: 'suave' }, 'Opcional. Toca em loop no leitor, no link e no vídeo; abaixa sozinha quando a narração fala e sobe nas pausas e viradas de página. Quem lê pode desligar no 🎵 do leitor.'),
      musica ? [
        previa,
        el('label', { class: 'linha' }, 'Volume sem narração ',
          el('input', { type: 'range', min: 0.05, max: 1, step: 0.05, value: musica.volume,
            oninput: (e) => { musica.volume = +e.target.value; previa.volume = musica.volume; rotulo.textContent = Math.round(musica.volume * 100) + '%'; },
            onchange: () => ajustarMusica({ volume: musica.volume }).then(() => aviso('Volume salvo ✓')).catch((err) => aviso('Não salvou: ' + err.message)) }),
          rotulo),
        el('small', { class: 'suave' }, 'Com narração, a música fica em 30% desse volume.'),
        musica.credito ? el('small', { class: 'suave' }, ' 🎵 Crédito (aparece no fim do leitor e do vídeo): ' + musica.credito) : null,
      ] : null,
      modelosUI,
      el('p', { class: 'suave' }, 'Ou envie a sua: músicas grátis e sem direitos autorais em Pixabay Music, YouTube Audio Library. Até 5 minutos (o resto é cortado).'),
      el('div', { class: 'linha' },
        el('button', { type: 'button', class: 'pequeno secundario', disabled: musicaUi.ocupado, onclick: () => enviar.click() },
          musicaUi.ocupado ? '⏳ Preparando…' : musica ? '📤 Trocar música' : '📤 Enviar música'), enviar,
        musica ? el('button', { type: 'button', class: 'link perigo', onclick: () =>
          ajustarMusica({ remover: true }).then(() => { musica = null; desenhar(); aviso('Música tirada ✓'); }).catch((err) => aviso('Não salvou: ' + err.message)) },
          '✕ Tirar música') : null));
    det.addEventListener('toggle', () => { musicaUi.aberto = det.open; });
    return det;
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
        el('label', { title: 'Para quem é o livro (texto livre: 4-6, 11 e 13 anos, adulto): vale para as imagens e para a ajuda da IA (o texto já escrito não muda sozinho)' }, 'Faixa',
          el('input', { list: 'faixas-sugestao', maxlength: '40', value: estado.faixa || '', placeholder: 'ex.: 4-6, adulto',
            onchange: (e) => { estado.faixa = e.target.value.trim(); mudou(); desenhar(); } }),
          el('datalist', { id: 'faixas-sugestao' }, ['2-3', '4-6', '7-9', '10-12', '13-17', 'adulto'].map((f) => el('option', { value: f })))),
        el('div', { class: 'modo-vista' },
          ['lista', 'carrossel'].map((m) => el('button', { type: 'button', class: modo === m ? '' : 'secundario',
            onclick: () => { modo = m; try { localStorage.setItem('editor-modo', m); } catch (_) { /* */ } desenhar(); } },
            m === 'lista' ? '📜 Lista' : '🎠 Carrossel')))),
      dados.briefing ? el('p', { class: 'suave' }, el('em', {}, 'Briefing: '), dados.briefing) : null,
      (dados.continua || []).length ? el('p', { class: 'suave' }, '📚 Continua de: ',
        dados.continua.map((c, i) => [i ? ', ' : '', el('a', { href: '/historias/' + c.id }, c.titulo)])) : null,
      // Painéis lado a lado (fechados viram "botões"); o aberto ocupa a linha inteira.
      el('div', { class: 'paineis' }, [capaUI(), musicaUI(), referenciaUI(), memoriaUI(), ajudaUI(), secundariosUI(), lugaresUI()].filter(Boolean)),
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
        fonteUI(),
        dados.imagem_api && estado.paginas.length ? el('button', { type: 'button', class: 'secundario', 'data-gerar-todas': true,
          title: 'Gera com IA a imagem de cada página sem imagem, na fila do servidor (pode fechar a aba)',
          onclick: (e) => gerarTodas(e.currentTarget) }, rotuloTodas()) : null,
        el('button', { type: 'button', class: 'secundario', onclick: gerarNarracao }, '🎙️ Gerar narração'),
        status, el('button', { type: 'button', onclick: salvar }, '💾 Salvar')),
    ].flat(Infinity).filter(Boolean));
    raiz.querySelectorAll('textarea:not(.prompt):not(.fixa):not(.recolhe)').forEach(ajustar);
    manterAncora(ancora, rolagem);
    observarPaginas();
  }

  window.addEventListener('beforeunload', (e) => { if (sujo) { e.preventDefault(); e.returnValue = ''; } });
  document.addEventListener('keydown', (e) => { if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); salvar(); } });
  desenhar();
  verNarracao();
  acompanharImagens();
})();

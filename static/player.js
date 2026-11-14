// Player do livro: capa → páginas (imagem | texto) → fim.
// Leitura livre (setas, deslizar, teclado) ou ▶ narração (áudio por trecho, destaque, virada automática).
(function () {
  const dados = JSON.parse(document.getElementById('dados').textContent);
  const raiz = document.getElementById('livro');
  const total = dados.paginas.length;
  // posições: 0 = capa, 1..total = páginas, total+1 = fim
  let pos = 0;
  let narrando = false;
  let mudo = false;
  let audio = null;
  let geracao = 0; // invalida narrações antigas ao pausar/trocar de página

  const el = (tag, attrs = {}, ...filhos) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === 'class') e.className = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? '' : v);
    }
    for (const f of filhos.flat()) if (f != null) e.append(f.nodeType ? f : document.createTextNode(f));
    return e;
  };
  const aviso = (msg) => {
    const a = document.getElementById('aviso');
    a.textContent = msg; a.hidden = false;
    clearTimeout(a._t); a._t = setTimeout(() => (a.hidden = true), 2500);
  };
  const espera = (ms) => new Promise((r) => setTimeout(r, ms));

  // ───────────── montagem das páginas ─────────────
  function midiaUI(m) {
    if (!m) return el('div', { class: 'sem-imagem' }, '🖼️');
    if (m.tipo.startsWith('video/')) {
      const v = el('video', { src: m.url, autoplay: true, loop: true, playsinline: true });
      v.muted = true;
      return v;
    }
    return el('img', { src: m.url, alt: '' });
  }
  function palavrasUI(texto) {
    // Cada palavra vira um <span> para o destaque karaokê.
    return texto.split(/(\s+)/).map((t) => (/\S/.test(t) ? el('span', { class: 'palavra' }, t) : t));
  }
  function trechoUI(b) {
    const fala = b.quem !== 'narrador' && dados.narracao !== 'narrador';
    return el('p', { class: 'trecho' + (fala ? ' fala' : '') },
      fala ? el('span', { class: 'quem' }, b.nome) : null,
      palavrasUI((fala ? '— ' : '') + b.texto));
  }
  function spreadUI() {
    if (pos === 0) {
      return el('div', { class: 'spread capa' },
        el('div', { class: 'lado lado-imagem' }, dados.capa ? el('img', { src: dados.capa, alt: '' }) : el('div', { class: 'sem-imagem' }, '📖')),
        el('div', { class: 'titulo-capa' }, dados.titulo));
    }
    if (pos > total) {
      return el('div', { class: 'spread' },
        el('div', { class: 'fim' }, el('div', {}, 'Fim! 🌟',
          el('div', { class: 'linha', style: 'justify-content:center;margin-top:16px' },
            el('button', { type: 'button', onclick: () => irPara(0) }, '↺ Ler de novo'),
            raiz.dataset.voltar !== 'about:blank'
              ? el('button', { type: 'button', class: 'secundario', onclick: () => (location.href = raiz.dataset.voltar) }, '📚 Outra história')
              : null))));
    }
    const p = dados.paginas[pos - 1];
    const imagem = el('div', { class: 'lado lado-imagem', onclick: () => efeito(p, true) }, midiaUI(p.midia));
    const texto = el('div', { class: 'lado lado-texto' }, p.blocos.map(trechoUI));
    return el('div', { class: 'spread' }, imagem, texto);
  }

  const botaoPlay = el('button', { type: 'button', class: 'grande', title: 'Narrar', onclick: () => (narrando ? pausar() : narrar()) }, '▶');
  const botaoAnt = el('button', { type: 'button', title: 'Voltar página', onclick: () => { pausar(); irPara(pos - 1, -1); } }, '◀');
  const botaoProx = el('button', { type: 'button', title: 'Próxima página', onclick: () => { pausar(); irPara(pos + 1, 1); } }, '▶▶');
  const botaoMudo = el('button', { type: 'button', title: 'Sons dos efeitos', onclick: () => { mudo = !mudo; botaoMudo.textContent = mudo ? '🔇' : '🔔'; } }, '🔔');
  const botaoTela = el('button', { type: 'button', title: 'Tela cheia', onclick: () => {
    if (document.fullscreenElement) document.exitFullscreen(); else document.documentElement.requestFullscreen?.();
  } }, '⛶');
  const pontos = el('div', { class: 'pontos' });
  const palco = el('div', { class: 'palco-livro' });
  raiz.replaceChildren(palco, el('div', { class: 'controles' },
    el('a', { class: 'voltar', href: raiz.dataset.voltar, title: 'Sair' }, '✕'),
    botaoAnt, botaoPlay, botaoProx, pontos, botaoMudo, botaoTela));

  function desenhar(direcao) {
    const s = spreadUI();
    if (direcao) s.classList.add('entrando');
    palco.replaceChildren(s);
    palco.classList.toggle('narrando', narrando);
    botaoAnt.disabled = pos === 0;
    botaoProx.disabled = pos > total;
    pontos.replaceChildren(...Array.from({ length: total + 2 }, (_, i) => el('span', { class: 'ponto' + (i === pos ? ' atual' : '') })));
  }

  async function irPara(nova, direcao = 1) {
    if (nova < 0 || nova > total + 1 || nova === pos) return;
    const atual = palco.querySelector('.spread');
    if (atual) {
      atual.classList.add(direcao >= 0 ? 'virando-frente' : 'virando-tras');
      await espera(450);
    }
    pos = nova;
    desenhar(direcao);
  }

  function efeito(p, porToque) {
    if (!p || !p.animacao || p.animacao === 'nenhuma') return;
    const alvo = palco.querySelector('.lado-imagem');
    window.Efeitos.aplicar(alvo, p.animacao);
    // Na leitura livre, o som só toca se a criança tocar na imagem.
    if (p.som && !mudo && (narrando || porToque)) window.Efeitos.tocarSom(p.animacao);
  }

  // ───────────── narração ─────────────
  // Ainda não gerado: pede ao servidor (uma vez só por trecho; a promessa fica guardada).
  function audioDoBloco(b) {
    if (b.audio) return Promise.resolve(b);
    if (!b._pedido) {
      b._pedido = fetch(dados.audio_url, { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ historia: dados.id, quem: b.voz || b.quem, texto: b.texto }) })
        .then((r) => (r.ok ? r.json() : null))
        .then((j) => { if (j) Object.assign(b, { audio: j.audio, duracao: j.duracao, palavras: j.palavras }); return b; })
        .catch(() => b)
        .finally(() => { if (!b.audio) b._pedido = null; });
    }
    return b._pedido;
  }
  // Enquanto um trecho toca, já pede os próximos (o resto da página e o começo da seguinte).
  function adiantar(pagina, bloco) {
    const fila = [];
    for (let p = pagina; p < dados.paginas.length && fila.length < 3; p++) {
      const bs = dados.paginas[p].blocos;
      for (let i = p === pagina ? bloco + 1 : 0; i < bs.length && fila.length < 3; i++) if (!bs[i].audio) fila.push(bs[i]);
    }
    fila.forEach((b) => audioDoBloco(b));
  }

  function destacarPalavras(trecho, b, a, minhaGeracao) {
    const spans = [...trecho.querySelectorAll('.palavra')].filter((s) => /[\p{L}\p{N}]/u.test(s.textContent));
    const marcas = b.palavras || [];
    if (!marcas.length || !spans.length) return;
    const passo = () => {
      // Narração trocada/pausada: só tira o destaque da palavra atual.
      if (minhaGeracao !== geracao) { spans.forEach((s) => s.classList.remove('agora')); return; }
      // Trecho terminou: todas lidas, nenhuma "agora".
      if (a.ended) { spans.forEach((s) => { s.classList.remove('agora'); s.classList.add('lida'); }); return; }
      const t = a.currentTime;
      let k = -1;
      for (let i = 0; i < marcas.length; i++) if (marcas[i][0] <= t + 0.05) k = i; else break;
      spans.forEach((s, i) => { s.classList.toggle('agora', i === k); s.classList.toggle('lida', i < k); });
      requestAnimationFrame(passo);
    };
    requestAnimationFrame(passo);
  }

  async function narrarPagina(minhaGeracao) {
    const p = dados.paginas[pos - 1];
    efeito(p, false);
    const trechos = palco.querySelectorAll('.trecho');
    for (let i = 0; i < p.blocos.length; i++) {
      if (minhaGeracao !== geracao) return false;
      trechos.forEach((t, j) => t.classList.toggle('ativo', j === i));
      trechos[i]?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      const pedido = audioDoBloco(p.blocos[i]);
      adiantar(pos - 1, i);
      const b = await pedido;
      if (minhaGeracao !== geracao) return false;
      if (b.audio) {
        audio = new Audio(b.audio);
        destacarPalavras(trechos[i], b, audio, minhaGeracao);
        try {
          await audio.play();
          await new Promise((ok) => { audio.onended = ok; audio.onerror = ok; audio.onpause = () => { if (minhaGeracao !== geracao) ok(); }; });
          trechos[i]?.querySelectorAll('.palavra.agora').forEach((s) => s.classList.remove('agora'));
        } catch (_) {
          await espera(Math.max(1500, b.texto.split(/\s+/).length * 380));
        }
      } else {
        // Sem áudio: dá tempo de ler.
        await espera(Math.max(1500, b.texto.split(/\s+/).length * 380));
      }
      await espera(250);
    }
    trechos.forEach((t) => t.classList.remove('ativo'));
    return minhaGeracao === geracao;
  }

  async function narrar() {
    narrando = true;
    const minha = ++geracao;
    botaoPlay.textContent = '⏸';
    palco.classList.add('narrando');
    if (pos === 0 || pos > total) await irPara(1, 1);
    while (narrando && minha === geracao && pos >= 1 && pos <= total) {
      const terminou = await narrarPagina(minha);
      if (!terminou) return;
      await espera(1000);
      if (minha !== geracao) return;
      await irPara(pos + 1, 1);
    }
    if (minha === geracao) pausar();
  }

  function pausar() {
    narrando = false;
    geracao++;
    if (audio) { audio.pause(); audio = null; }
    botaoPlay.textContent = '▶';
    palco.classList.remove('narrando');
    palco.querySelectorAll('.trecho.ativo').forEach((t) => t.classList.remove('ativo'));
  }

  // ───────────── navegação por toque e teclado ─────────────
  let x0 = null;
  palco.addEventListener('touchstart', (e) => (x0 = e.touches[0].clientX), { passive: true });
  palco.addEventListener('touchend', (e) => {
    if (x0 === null) return;
    const dx = e.changedTouches[0].clientX - x0;
    x0 = null;
    if (Math.abs(dx) > 50) { pausar(); irPara(pos + (dx < 0 ? 1 : -1), dx < 0 ? 1 : -1); }
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowRight') { pausar(); irPara(pos + 1, 1); }
    else if (e.key === 'ArrowLeft') { pausar(); irPara(pos - 1, -1); }
    else if (e.key === ' ') { e.preventDefault(); narrando ? pausar() : narrar(); }
  });

  desenhar();
})();

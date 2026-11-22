// Player do livro: capa → páginas (imagem | texto) → fim.
// Leitura livre (setas, deslizar, teclado) ou ▶ narração (áudio por trecho, destaque, virada automática).
(function () {
  const { el, aviso } = window.St;
  const dados = JSON.parse(document.getElementById('dados').textContent);
  const raiz = document.getElementById('livro');
  const total = dados.paginas.length;
  // posições: 0 = capa, 1..total = páginas, total+1 = fim
  let pos = 0;
  let narrando = false;
  // Pausou no meio da página: { pos, bloco } para o ▶ recomeçar desta fala (trocar de página esquece).
  let retomar = null;
  let blocoAtual = 0;
  let mudo = false;
  let audio = null;
  let geracao = 0; // invalida narrações antigas ao pausar/trocar de página

  const espera = (ms) => new Promise((r) => setTimeout(r, ms));

  // ───────────── música de fundo (opcional) ─────────────
  // Toca em loop: volume "cheio" sem narração e abaixa quando alguém fala (sobe de novo devagar nas pausas).
  // Música ligada/desligada pela pessoa (vale para todas as músicas da história).
  let ligada = true;
  try { ligada = localStorage.getItem('musica-ligada') !== 'nao'; } catch (_) { /* */ }
  function criarMusica(m) {
    const som = new Audio(m.url);
    som.loop = true;
    som.preload = 'auto';
    const CHEIO = m.volume || 0.35;
    const FUNDO = CHEIO * 0.3;
    let ctx = null, ganho = null, subir = null, vez = 0;
    // Web Audio para as rampas de volume (no iPhone o volume do <audio> não muda por código).
    function preparar() {
      if (ctx || ganho === false) return;
      try {
        ctx = new (window.AudioContext || window.webkitAudioContext)();
        ganho = ctx.createGain();
        ganho.gain.value = 0;
        ctx.createMediaElementSource(som).connect(ganho).connect(ctx.destination);
      } catch (_) { ctx = null; ganho = false; }
    }
    function alvo(v, seg) {
      if (ganho) {
        const t = ctx.currentTime, g = ganho.gain;
        g.cancelScheduledValues(t); g.setValueAtTime(g.value, t); g.linearRampToValueAtTime(v, t + seg);
      } else som.volume = Math.min(1, v);
    }
    const api = {
      get ligada() { return ligada; },
      get tocando() { return ligada && !som.paused; },
      tocar() {
        if (!ligada) return;
        preparar();
        if (ctx && ctx.state === 'suspended') ctx.resume();
        vez++;
        if (som.paused) { if (!ganho) som.volume = 0; som.play().catch(() => {}); }
        alvo(CHEIO, 1.5);
      },
      parar(seg = 1) {
        const minha = ++vez;
        clearTimeout(subir);
        alvo(0, seg);
        setTimeout(() => { if (minha === vez) som.pause(); }, seg * 1000 + 50);
      },
      fala() { clearTimeout(subir); if (api.tocando) alvo(FUNDO, 0.3); },
      calou() { clearTimeout(subir); subir = setTimeout(() => { if (api.tocando) alvo(CHEIO, 1.2); }, 700); },
    };
    return api;
  }
  // Cada página diz a música que toca nela (a geral, a trocada numa página anterior ou nenhuma).
  // Ao mudar, a anterior some aos poucos e a nova entra; se é a mesma, segue sem recomeçar.
  const volumeMusica = dados.musica_volume || (dados.musica && dados.musica.volume) || 0.35;
  const temMusica = !!dados.musica || dados.paginas.some((p) => p.musica);
  const sons = {};
  let musica = null, urlMusica = null;
  function urlDaPagina(n) {
    if (n > total) return null;
    const p = dados.paginas[Math.max(n, 1) - 1];
    return p ? p.musica || null : null;
  }
  function trocarMusica(seg = 1.5) {
    const url = urlDaPagina(pos);
    if (url === urlMusica) return;
    const tocava = musica && musica.tocando;
    if (musica) musica.parar(seg);
    urlMusica = url;
    musica = url ? (sons[url] = sons[url] || criarMusica({ url, volume: volumeMusica })) : null;
    if (musica && (tocava || narrando)) musica.tocar();
  }
  function alternarMusica() {
    ligada = !ligada;
    try { localStorage.setItem('musica-ligada', ligada ? 'sim' : 'nao'); } catch (_) { /* */ }
    if (!musica) return;
    if (ligada) { if (narrando) { musica.tocar(); if (audio && !audio.paused) musica.fala(); } } else musica.parar(0.5);
  }

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
  // Imagem inteira (sem cortar): o espaço que sobra ganha a própria imagem desfocada por trás.
  function comFundo(lado, url) {
    if (url) { lado.classList.add('com-fundo'); lado.style.setProperty('--fundo-img', 'url("' + url + '")'); }
    return lado;
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
        comFundo(el('div', { class: 'lado lado-imagem' }, dados.capa ? el('img', { src: dados.capa, alt: '' }) : el('div', { class: 'sem-imagem' }, '📖')), dados.capa),
        el('div', { class: 'titulo-capa' }, dados.titulo));
    }
    if (pos > total) {
      return el('div', { class: 'spread' },
        el('div', { class: 'fim' }, el('div', {}, 'Fim! 🌟',
          el('div', { class: 'linha', style: 'justify-content:center;margin-top:16px' },
            el('button', { type: 'button', onclick: () => irPara(0) }, '↺ Ler de novo'),
            raiz.dataset.voltar !== 'about:blank'
              ? el('button', { type: 'button', class: 'secundario', onclick: () => (location.href = raiz.dataset.voltar) }, '📚 Outra história')
              : null),
          (dados.creditos_musica || []).length ? el('p', { class: 'credito-musica' }, '🎵 Música: ' + dados.creditos_musica.join(' · ')) : null,
          el('p', { class: 'credito-musica' }, el('a', { href: '/creditos', target: '_blank', rel: 'noopener' }, '🌟 Créditos')))));
    }
    const p = dados.paginas[pos - 1];
    const imagem = comFundo(el('div', { class: 'lado lado-imagem', onclick: () => efeito(p, true) }, midiaUI(p.midia)),
      p.midia && !p.midia.tipo.startsWith('video/') ? p.midia.url : null);
    const texto = el('div', { class: 'lado lado-texto' }, p.blocos.map(trechoUI));
    return el('div', { class: 'spread' }, imagem, texto);
  }

  const botaoPlay = el('button', { type: 'button', class: 'grande', title: 'Narrar', onclick: () => (narrando ? pausar() : narrar()) }, '▶');
  const botaoAnt = el('button', { type: 'button', title: 'Voltar página', onclick: () => { pausar(); irPara(pos - 1, -1); } }, '◀');
  const botaoProx = el('button', { type: 'button', title: 'Próxima página', onclick: () => { pausar(); irPara(pos + 1, 1); } }, '▶▶');
  const botaoMudo = el('button', { type: 'button', title: 'Sons dos efeitos', onclick: () => { mudo = !mudo; botaoMudo.textContent = mudo ? '🔇' : '🔔'; } }, '🔔');
  const botaoMusica = temMusica ? el('button', { type: 'button', title: 'Música de fundo', onclick: () => { alternarMusica(); desenharMusica(); } }) : null;
  function desenharMusica() {
    if (!botaoMusica) return;
    botaoMusica.textContent = '🎵';
    botaoMusica.classList.toggle('desligado', !ligada);
    botaoMusica.title = ligada ? 'Música de fundo: ligada (toque para desligar)' : 'Música de fundo: desligada';
  }
  desenharMusica();
  const botaoTela = el('button', { type: 'button', title: 'Tela cheia', onclick: () => {
    if (document.fullscreenElement) document.exitFullscreen(); else document.documentElement.requestFullscreen?.();
  } }, '⛶');
  const pontos = el('div', { class: 'pontos' });
  const palco = el('div', { class: 'palco-livro' });
  raiz.replaceChildren(palco, el('div', { class: 'controles' },
    el('a', { class: 'voltar', href: raiz.dataset.voltar, title: 'Sair' }, '✕'),
    botaoAnt, botaoPlay, botaoProx, pontos, botaoMusica, botaoMudo, botaoTela));

  function desenhar(direcao) {
    const s = spreadUI();
    if (direcao) s.classList.add('entrando');
    palco.replaceChildren(s);
    palco.classList.toggle('narrando', narrando);
    botaoAnt.disabled = pos === 0;
    botaoProx.disabled = pos > total;
    trocarMusica(pos > total ? 3 : 1.5);
    pontos.replaceChildren(...Array.from({ length: total + 2 }, (_, i) => el('span', { class: 'ponto' + (i === pos ? ' atual' : '') })));
  }

  async function irPara(nova, direcao = 1) {
    if (nova < 0 || nova > total + 1 || nova === pos) return;
    retomar = null;
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

  async function narrarPagina(minhaGeracao, inicio = 0) {
    const p = dados.paginas[pos - 1];
    if (!inicio) efeito(p, false);
    // Com música, ela "respira" um pouco na página nova antes da narração.
    if (musica && musica.tocando) {
      musica.calou();
      await espera(1500);
      if (minhaGeracao !== geracao) return false;
    }
    const trechos = palco.querySelectorAll('.trecho');
    for (let i = inicio; i < p.blocos.length; i++) {
      if (minhaGeracao !== geracao) return false;
      blocoAtual = i;
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
          if (musica) musica.fala();
          await audio.play();
          await new Promise((ok) => { audio.onended = ok; audio.onerror = ok; audio.onpause = () => { if (minhaGeracao !== geracao) ok(); }; });
          trechos[i]?.querySelectorAll('.palavra.agora').forEach((s) => s.classList.remove('agora'));
          if (musica) musica.calou();
        } catch (_) {
          if (musica) musica.calou();
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
    if (musica) musica.tocar();
    if (pos === 0 || pos > total) await irPara(1, 1);
    // Pausado no meio desta página: recomeça da fala em que parou.
    let inicio = retomar && retomar.pos === pos ? retomar.bloco : 0;
    retomar = null;
    while (narrando && minha === geracao && pos >= 1 && pos <= total) {
      const terminou = await narrarPagina(minha, inicio);
      inicio = 0;
      if (!terminou) return;
      await espera(1000);
      if (minha !== geracao) return;
      await irPara(pos + 1, 1);
    }
    if (minha === geracao) pausar();
  }

  function pausar() {
    if (narrando && pos >= 1 && pos <= total) retomar = { pos, bloco: blocoAtual };
    narrando = false;
    geracao++;
    if (audio) { audio.pause(); audio = null; }
    // Pausou a narração: a música pausa junto (some aos poucos) e volta de onde parou no ▶.
    if (musica) musica.parar(0.6);
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

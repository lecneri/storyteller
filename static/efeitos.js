// Efeitos das páginas: animação visual (sobreposta) + som sintetizado (Web Audio, sem arquivos).
// Usado pelo editor (botão "Exemplo") e, depois, pelo player.
window.Efeitos = (function () {
  let ctx = null;
  const audio = () => (ctx = ctx || new (window.AudioContext || window.webkitAudioContext)());

  function ruido(seg) {
    const a = audio();
    const buf = a.createBuffer(1, a.sampleRate * seg, a.sampleRate);
    const d = buf.getChannelData(0);
    for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
    const src = a.createBufferSource();
    src.buffer = buf;
    return src;
  }
  function envelope(no, ini, pico, dur, vol) {
    const a = audio(), g = a.createGain();
    g.gain.setValueAtTime(0.0001, ini);
    g.gain.exponentialRampToValueAtTime(vol, ini + pico);
    g.gain.exponentialRampToValueAtTime(0.0001, ini + dur);
    no.connect(g).connect(a.destination);
    return g;
  }
  function tom(freq, ini, dur, vol = 0.2, tipo = 'sine', freqFim) {
    const a = audio(), o = a.createOscillator();
    o.type = tipo;
    o.frequency.setValueAtTime(freq, ini);
    if (freqFim) o.frequency.exponentialRampToValueAtTime(freqFim, ini + dur);
    envelope(o, ini, 0.01, dur, vol);
    o.start(ini); o.stop(ini + dur + 0.05);
  }
  function ruidoFiltrado(ini, dur, tipo, freq, vol, freqFim) {
    const a = audio(), n = ruido(dur + 0.1), f = a.createBiquadFilter();
    f.type = tipo; f.frequency.setValueAtTime(freq, ini);
    if (freqFim) f.frequency.exponentialRampToValueAtTime(freqFim, ini + dur);
    n.connect(f);
    envelope(f, ini, Math.min(0.3, dur / 4), dur, vol);
    n.start(ini); n.stop(ini + dur + 0.1);
  }

  const sons = {
    // "brummm": estrondo grave
    tremor: (t) => { ruidoFiltrado(t, 1.6, 'lowpass', 140, 0.9); tom(48, t, 1.4, 0.35, 'sine', 32); },
    // "pow!"
    'onomatopeia-pop': (t) => { ruidoFiltrado(t, 0.25, 'bandpass', 1800, 0.6, 400); tom(220, t, 0.25, 0.3, 'square', 80); },
    // "KABUM!": estalo + estrondo grave que vai sumindo
    explosao: (t) => {
      ruidoFiltrado(t, 0.15, 'highpass', 1500, 0.5);
      ruidoFiltrado(t + 0.02, 2.2, 'lowpass', 900, 1.0, 60);
      tom(90, t, 1.2, 0.45, 'sine', 28);
    },
    chuva: (t) => ruidoFiltrado(t, 2.5, 'highpass', 3000, 0.12),
    neve: (t) => ruidoFiltrado(t, 2.5, 'bandpass', 600, 0.15, 900),
    folhas: (t) => ruidoFiltrado(t, 2.5, 'bandpass', 500, 0.15, 1200),
    'estrelas-piscando': (t) => [0, 0.35, 0.8].forEach((d, i) => tom([1568, 2093, 2637][i], t + d, 0.6, 0.08)),
    brilhos: (t) => [0, 0.12, 0.24, 0.36].forEach((d, i) => tom(1318 + i * 260, t + d, 0.5, 0.07, 'triangle')),
    bolhas: (t) => [0, 0.2, 0.45, 0.6, 0.9].forEach((d) => tom(300 + Math.random() * 300, t + d, 0.12, 0.15, 'sine', 900)),
  };

  function tocarSom(nome) {
    const f = sons[nome];
    if (!f) return;
    const a = audio();
    if (a.state === 'suspended') a.resume();
    f(a.currentTime + 0.02);
  }

  // Visual: partículas/efeitos dentro de um elemento com position:relative.
  const particulas = {
    'estrelas-piscando': { qtd: 14, txt: () => '✦', cls: 'fx-estrela' },
    brilhos: { qtd: 16, txt: () => '✨', cls: 'fx-brilho' },
    chuva: { qtd: 40, txt: () => '', cls: 'fx-chuva' },
    neve: { qtd: 30, txt: () => '•', cls: 'fx-neve' },
    folhas: { qtd: 12, txt: () => (Math.random() < 0.5 ? '🍃' : '🍂'), cls: 'fx-folha' },
    bolhas: { qtd: 14, txt: () => '', cls: 'fx-bolha' },
  };
  function aplicar(alvo, nome, duracaoMs = 3500) {
    if (!alvo || !nome || nome === 'nenhuma') return;
    const camada = document.createElement('div');
    camada.className = 'fx-camada';
    alvo.append(camada);
    if (nome === 'tremor') {
      alvo.classList.add('fx-tremor');
      setTimeout(() => alvo.classList.remove('fx-tremor'), 1400);
    } else if (nome === 'explosao') {
      // Explosão de desenho animado: clarão, bola de fogo, anel, fumaça, faíscas e "KABUM!"
      alvo.classList.add('fx-tremor');
      setTimeout(() => alvo.classList.remove('fx-tremor'), 1200);
      ['fx-clarao', 'fx-fogo', 'fx-anel'].forEach((c) => { const d = document.createElement('div'); d.className = c; camada.append(d); });
      for (let i = 0; i < 8; i++) {
        const f = document.createElement('div');
        f.className = 'fx-fumaca';
        const ang = (i / 8) * Math.PI * 2;
        f.style.setProperty('--dx', Math.cos(ang) * 60 + 'px');
        f.style.setProperty('--dy', Math.sin(ang) * 60 - 20 + 'px');
        camada.append(f);
      }
      for (let i = 0; i < 14; i++) {
        const f = document.createElement('div');
        f.className = 'fx-faisca';
        const ang = Math.random() * Math.PI * 2, dist = 70 + Math.random() * 60;
        f.style.setProperty('--dx', Math.cos(ang) * dist + 'px');
        f.style.setProperty('--dy', Math.sin(ang) * dist + 'px');
        camada.append(f);
      }
      const p = document.createElement('div');
      p.className = 'fx-pow fx-kabum';
      p.textContent = 'KABUM!';
      camada.append(p);
    } else if (nome === 'onomatopeia-pop') {
      const p = document.createElement('div');
      p.className = 'fx-pow';
      p.textContent = ['POW!', 'BUM!', 'ZAP!', 'TCHAM!'][Math.floor(Math.random() * 4)];
      camada.append(p);
    } else if (particulas[nome]) {
      const cfg = particulas[nome];
      for (let i = 0; i < cfg.qtd; i++) {
        const e = document.createElement('span');
        e.className = 'fx ' + cfg.cls;
        e.textContent = cfg.txt();
        e.style.left = Math.random() * 100 + '%';
        e.style.top = Math.random() * 100 + '%';
        e.style.animationDelay = Math.random() * 1.5 + 's';
        e.style.animationDuration = 1 + Math.random() * 1.5 + 's';
        camada.append(e);
      }
    }
    setTimeout(() => camada.remove(), duracaoMs);
  }

  return { tocarSom, aplicar, temSom: (n) => !!sons[n] };
})();

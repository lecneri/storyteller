// Configurar voz (narrador ou personagem): escolher voz, tom e velocidade, ouvir prévia e salvar.
// Uso: <div data-voz data-quem="narrador|slug" data-salvar="/url" data-exemplo="texto"></div>
(function () {
  const caixas = document.querySelectorAll('[data-voz]');
  if (!caixas.length) return;

  const aviso = (msg) => {
    const a = document.getElementById('aviso');
    if (!a) return;
    a.textContent = msg; a.hidden = false;
    clearTimeout(a._t); a._t = setTimeout(() => (a.hidden = true), 2500);
  };
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
  let tocando = null;
  async function tocar(voz, texto, botao) {
    botao.disabled = true; botao.textContent = '⏳ Gerando…';
    try {
      const r = await fetch('/tts/previa', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ voz, texto }) });
      const j = await r.json();
      if (!r.ok) throw new Error(j.erro || 'erro');
      if (tocando) tocando.pause();
      tocando = new Audio(j.audio);
      await tocando.play();
    } catch (e) {
      aviso('Não deu para ouvir: ' + e.message);
    } finally {
      botao.disabled = false; botao.textContent = '▶ Ouvir';
    }
  }

  fetch('/vozes').then((r) => r.json()).then((dados) => {
    caixas.forEach((caixa) => {
      const quem = caixa.dataset.quem;
      const atual = quem === 'narrador' ? dados.narrador : ((dados.personagens.find((p) => p.slug === quem) || {}).voz || dados.narrador);
      const voz = { ...atual };
      const sel = el('select', { onchange: (e) => (voz.id = e.target.value) },
        dados.vozes.map((v) => el('option', { value: v.id, selected: v.id === voz.id }, v.nome)));
      const rotTom = el('span', { class: 'suave' });
      const rotVel = el('span', { class: 'suave' });
      const atualiza = () => {
        rotTom.textContent = (voz.tom > 0 ? '+' : '') + voz.tom + ' (' + (voz.tom > 0 ? 'mais agudo' : voz.tom < 0 ? 'mais grave' : 'normal') + ')';
        rotVel.textContent = (voz.velocidade > 0 ? '+' : '') + voz.velocidade + '%';
      };
      const tom = el('input', { type: 'range', min: -60, max: 60, step: 5, value: voz.tom, oninput: (e) => { voz.tom = +e.target.value; atualiza(); } });
      const vel = el('input', { type: 'range', min: -40, max: 40, step: 5, value: voz.velocidade, oninput: (e) => { voz.velocidade = +e.target.value; atualiza(); } });
      const texto = el('input', { value: caixa.dataset.exemplo || 'Era uma vez uma aventura incrível!' });
      const ouvir = el('button', { type: 'button', class: 'secundario', onclick: () => tocar(voz, texto.value, ouvir) }, '▶ Ouvir');
      const salvar = async (valor) => {
        const r = await fetch(caixa.dataset.salvar, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ voz: valor }) });
        aviso(r.ok ? 'Voz salva ✓' : 'Não salvou');
        if (r.ok && valor === null) location.reload();
      };
      atualiza();
      caixa.replaceChildren(el('div', { class: 'grade-form' },
        el('label', { class: 'inteiro' }, 'Voz', sel),
        el('label', {}, 'Tom ', rotTom, tom),
        el('label', {}, 'Velocidade ', rotVel, vel),
        el('label', { class: 'inteiro' }, 'Frase de teste', texto),
        el('div', { class: 'inteiro linha' }, ouvir,
          el('button', { type: 'button', onclick: () => salvar(voz) }, '💾 Salvar voz'),
          el('button', { type: 'button', class: 'link', onclick: () => salvar(null) }, '↺ voltar ao padrão'))));
    });
  });
})();

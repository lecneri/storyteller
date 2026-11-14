// Vozes extras de um personagem (ex.: "Inglês"): nome, língua, voz, tom e velocidade; N vozes.
// Uso: <div data-vozes-extras="/personagens/slug/vozes-extras" data-nome="Pedro"></div>
(function () {
  const { el, aviso } = window.St;
  const caixa = document.querySelector('[data-vozes-extras]');
  if (!caixa) return;
  const url = caixa.dataset.vozesExtras;
  let tocando = null;
  async function ouvir(voz, texto, botao) {
    botao.disabled = true; botao.textContent = '⏳';
    try {
      const r = await fetch('/tts/previa', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ voz, texto }) });
      const j = await r.json();
      if (!r.ok) throw new Error(j.erro || 'erro');
      if (tocando) tocando.pause();
      tocando = new Audio(j.audio); await tocando.play();
    } catch (e) { aviso('Não deu para ouvir: ' + e.message); }
    finally { botao.disabled = false; botao.textContent = '▶ Ouvir'; }
  }

  Promise.all([fetch(url).then((r) => r.json()), fetch('/vozes').then((r) => r.json())]).then(([dados, vozes]) => {
    const lista = dados.vozes_extras || [];
    const linguas = dados.linguas || [];
    const exemplo = (l) => (linguas.find((x) => x.nome === l) || {}).exemplo || 'Oi!';

    function linha(e, i) {
      const sel = el('select', { onchange: (ev) => (e.voz.id = ev.target.value) },
        vozes.vozes.map((v) => el('option', { value: v.id, selected: v.id === e.voz.id }, v.nome)));
      const frase = el('input', { value: exemplo(e.lingua) });
      const lingua = el('select', { onchange: (ev) => {
        const antigo = e.lingua; e.lingua = ev.target.value;
        if (!e.nome || e.nome === antigo) { e.nome = e.lingua; nome.value = e.nome; }
        frase.value = exemplo(e.lingua);
      } }, linguas.map((l) => el('option', { value: l.nome, selected: l.nome === e.lingua }, l.nome)));
      const nome = el('input', { value: e.nome, placeholder: 'Inglês', oninput: (ev) => (e.nome = ev.target.value) });
      const tom = el('input', { type: 'range', min: -60, max: 60, step: 5, value: e.voz.tom, oninput: (ev) => (e.voz.tom = +ev.target.value) });
      const vel = el('input', { type: 'range', min: -40, max: 40, step: 5, value: e.voz.velocidade, oninput: (ev) => (e.voz.velocidade = +ev.target.value) });
      const botao = el('button', { type: 'button', class: 'secundario', onclick: () => ouvir(e.voz, frase.value, botao) }, '▶ Ouvir');
      return el('div', { class: 'cartao grade-form' },
        el('label', {}, 'Nome da voz', nome),
        el('label', {}, 'Língua', lingua),
        el('label', { class: 'inteiro' }, 'Voz', sel),
        el('label', {}, 'Tom', tom),
        el('label', {}, 'Velocidade', vel),
        el('label', { class: 'inteiro' }, 'Frase de teste', frase),
        el('div', { class: 'inteiro linha' }, botao,
          el('button', { type: 'button', class: 'link', onclick: () => { lista.splice(i, 1); desenhar(); } }, '✕ remover esta voz')));
    }
    async function salvar(botao) {
      botao.disabled = true;
      try {
        const r = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(lista) });
        const j = await r.json();
        if (!r.ok) throw new Error(j.erro || 'erro');
        lista.splice(0, lista.length, ...j.vozes_extras);
        aviso('Vozes salvas ✓'); desenhar();
      } catch (e) { aviso('Não salvou: ' + e.message); }
      finally { botao.disabled = false; }
    }
    function desenhar() {
      const salvarBtn = el('button', { type: 'button', onclick: () => salvar(salvarBtn) }, '💾 Salvar vozes');
      caixa.replaceChildren(
        ...(lista.length ? lista.map(linha) : [el('p', { class: 'suave' }, 'Nenhuma voz extra ainda.')]),
        el('div', { class: 'linha' },
          el('button', { type: 'button', class: 'secundario', onclick: () => {
            lista.push({ nome: 'Inglês', lingua: 'Inglês', voz: { motor: 'edge', id: 'en-US-AndrewMultilingualNeural', tom: 30, velocidade: -10 } });
            desenhar();
          } }, '＋ Adicionar voz'),
          salvarBtn));
    }
    desenhar();
  });
})();

// Sistema: músicas-modelo da plataforma (enviar, ouvir, apagar).
(function () {
  const secao = document.getElementById('musicas-modelo');
  if (!secao) return;
  const { el, aviso } = window.St;
  const lista = el('div', { class: 'lista-musicas' });
  let climas = [];

  const tempo = (s) => Math.floor(s / 60) + ':' + String(Math.round(s % 60)).padStart(2, '0');

  async function carregar() {
    const r = await fetch('/sistema/musicas');
    if (!r.ok) { lista.replaceChildren(el('p', { class: 'erro' }, 'Não carregou (' + r.status + ').')); return; }
    const j = await r.json();
    climas = j.climas;
    const nomeClima = (id) => (climas.find((c) => c.id === id) || {}).nome || id;
    lista.replaceChildren(...(j.modelos.length ? j.modelos.map((m) => el('div', { class: 'musica-modelo linha' },
      el('div', { style: 'flex:1;min-width:220px' },
        el('strong', {}, m.titulo), ' ', el('small', { class: 'suave' }, nomeClima(m.clima) + ' · ' + tempo(m.duracao) + (m.idade_min ? ' · ' + m.idade_min + '+ anos' : ' · qualquer idade')),
        m.descricao ? el('div', { class: 'suave' }, m.descricao) : null,
        m.credito ? el('div', {}, el('small', { class: 'suave' }, 'Crédito: ' + m.credito)) : null),
      el('audio', { src: '/musicas-modelo/' + m.id, controls: true, preload: 'none' }),
      el('button', { type: 'button', class: 'link perigo', onclick: async () => {
        if (!confirm('Tirar "' + m.titulo + '" da plataforma? Histórias que já usam continuam com ela.')) return;
        const r = await fetch('/sistema/musicas/' + m.id + '/apagar', { method: 'POST' });
        if (r.ok) { aviso('Removida ✓'); carregar(); } else aviso('Não removeu (' + r.status + ').');
      } }, '🗑'))) : [el('p', { class: 'suave' }, 'Nenhuma música ainda.')]));
    form.querySelector('select[name=clima]').replaceChildren(...climas.map((c) => el('option', { value: c.id }, c.nome)));
  }

  const form = el('form', { class: 'pilha', onsubmit: async (e) => {
    e.preventDefault();
    const botao = form.querySelector('button[type=submit]');
    botao.disabled = true; botao.textContent = '⏳ Preparando…';
    try {
      const r = await fetch('/sistema/musicas', { method: 'POST', body: new FormData(form) });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
      form.reset(); aviso('Música adicionada ✓'); carregar();
    } catch (err) { aviso('Não enviou: ' + err.message); }
    botao.disabled = false; botao.textContent = '📤 Adicionar';
  } },
    el('div', { class: 'linha' },
      el('label', {}, 'Arquivo', el('input', { type: 'file', name: 'arquivo', accept: 'audio/*', required: true })),
      el('label', {}, 'Nome', el('input', { name: 'titulo', required: true, placeholder: 'Carefree' })),
      el('label', {}, 'Clima', el('select', { name: 'clima' })),
      el('label', {}, 'A partir de (anos)', el('input', { name: 'idade_min', type: 'number', min: 0, max: 18, value: 0, style: 'width:6em' }))),
    el('label', {}, 'Como soa (ajuda a escolher)', el('input', { name: 'descricao', placeholder: 'Ukulele leve e alegre' })),
    el('label', {}, 'Crédito (se a licença pedir)', el('input', { name: 'credito', placeholder: '"Carefree" Kevin MacLeod (incompetech.com) · CC BY 4.0' })),
    el('div', {}, el('button', { type: 'submit' }, '📤 Adicionar')));

  secao.querySelector('.carregando').replaceWith(lista, el('h3', {}, 'Adicionar música'), form);
  carregar();
})();

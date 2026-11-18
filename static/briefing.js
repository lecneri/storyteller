// Briefing: autocompletar de @personagem(:variante)(!voz), @grupo:nome, @lugar:nome e #historia, e chips do que foi detectado.
// Voz extra: "@pedro!ingles" (só nessa voz/língua) ou "@pedro!misto-ingles" (mistura com português).
// Caixas com data-mencoes="simples" (ex.: Relações da ficha) só sugerem @personagem, sem variante, voz ou #história.
(function () {
  const { el } = window.St;
  const caixas = document.querySelectorAll('textarea[name="briefing"], textarea[data-mencoes]');
  if (!caixas.length) return;
  const norm = (t) => (t || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

  fetch('/mencoes').then((r) => r.json()).then((dados) => caixas.forEach((ta) => ligar(ta, dados)));

  function ligar(ta, dados) {
    const lista = el('div', { class: 'autocompletar', hidden: '' });
    const chips = el('div', { class: 'chips detectados' });
    ta.parentNode.insertBefore(lista, ta.nextSibling);
    ta.parentNode.insertBefore(chips, lista.nextSibling);
    let opcoes = [], ativa = 0, inicio = -1;
    const simples = ta.dataset.mencoes === 'simples';

    function sugestoes() {
      const pos = ta.selectionStart;
      const antes = ta.value.slice(0, pos);
      const m = antes.match(simples ? /(@)([\p{L}\p{N}_-]*)$/u : /([@#])([\p{L}\p{N}_:!-]*)$/u);
      if (!m) { fechar(); return; }
      inicio = pos - m[0].length;
      const termo = norm(m[2]);
      if (m[1] === '@') {
        opcoes = [];
        // Grupos: "@grupo:familia" cita todos os personagens do grupo.
        if (!simples) {
          const t = termo.startsWith('grupo:') ? termo.slice(6) : termo;
          for (const g of dados.grupos || []) {
            if (!norm('grupo ' + g.slug + ' ' + g.nome).includes(t)) continue;
            const nomes = g.membros.map((s) => (dados.personagens.find((p) => p.slug === s) || { nome: s }).nome);
            opcoes.push({ txt: '@grupo:' + g.slug, rot: '👥 ' + g.nome + ' (' + nomes.join(', ') + ')' });
          }
        }
        // Lugares: "@lugar:castelo".
        if (!simples) {
          const t = termo.startsWith('lugar:') ? termo.slice(6) : termo;
          for (const l of dados.lugares || []) {
            if (norm('lugar ' + l.slug + ' ' + l.nome).includes(t)) opcoes.push({ txt: '@lugar:' + l.slug, rot: '📍 ' + l.nome });
          }
        }
        for (const p of dados.personagens) {
          if (!norm(p.slug + ' ' + p.nome).includes(termo.split(/[:!]/)[0])) continue;
          if (simples) {
            if (p.slug !== ta.dataset.proprio) opcoes.push({ txt: '@' + p.slug, rot: '🧒 ' + p.nome });
            continue;
          }
          const vozes = p.vozes || [];
          opcoes.push({ txt: '@' + p.slug, rot: '🧒 ' + p.nome + (vozes.length ? ' · 🗣️ voz normal' : '') });
          // Personagem com vozes extras: pergunta qual voz ele vai usar.
          for (const v of vozes) {
            opcoes.push({ txt: '@' + p.slug + '!' + v.slug, rot: '   🌎 ' + p.nome + ' falando ' + v.nome });
            if (v.lingua && v.lingua !== 'Português') {
              opcoes.push({ txt: '@' + p.slug + '!misto-' + v.slug, rot: '   🔀 ' + p.nome + ' misturando ' + v.nome + ' e português' });
            }
          }
          for (const v of p.variantes || []) {
            if (termo.includes(':') && !norm(v.slug).includes(termo.split(':')[1] || '')) continue;
            opcoes.push({ txt: '@' + p.slug + ':' + v.slug, rot: '   ↳ ' + p.nome + ' · ' + v.nome });
          }
        }
      } else {
        opcoes = dados.historias.filter((h) => norm(h.slug + ' ' + h.titulo).includes(termo))
          .map((h) => ({ txt: '#' + h.slug, rot: '📖 ' + h.titulo }));
      }
      opcoes = opcoes.slice(0, 12);
      if (!opcoes.length) { fechar(); return; }
      ativa = 0;
      desenharLista();
    }
    function desenharLista() {
      lista.replaceChildren(...opcoes.map((o, i) => el('div', { class: 'opcao' + (i === ativa ? ' ativa' : ''),
        onmousedown: (e) => { e.preventDefault(); escolher(i); } }, o.rot)));
      lista.hidden = false;
    }
    function fechar() { lista.hidden = true; opcoes = []; }
    function escolher(i) {
      const o = opcoes[i];
      const pos = ta.selectionStart;
      ta.value = ta.value.slice(0, inicio) + o.txt + ' ' + ta.value.slice(pos);
      const novo = inicio + o.txt.length + 1;
      ta.setSelectionRange(novo, novo);
      fechar(); ta.focus(); detectar();
    }
    ta.addEventListener('input', () => { sugestoes(); detectar(); });
    ta.addEventListener('keydown', (e) => {
      if (lista.hidden) return;
      if (e.key === 'ArrowDown') { e.preventDefault(); ativa = (ativa + 1) % opcoes.length; desenharLista(); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); ativa = (ativa - 1 + opcoes.length) % opcoes.length; desenharLista(); }
      else if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); escolher(ativa); }
      else if (e.key === 'Escape') fechar();
    });
    ta.addEventListener('blur', () => setTimeout(fechar, 150));

    // Chips: quem a IA vai receber (o servidor detecta do mesmo jeito).
    function detectar() {
      if (simples) return;
      const texto = ta.value;
      const t = '-' + norm(texto) + '-';
      const achados = [];
      for (const p of dados.personagens) {
        // Todas as menções diretas (o mesmo personagem pode vir com variantes diferentes: transformação).
        const re = new RegExp('@' + p.slug + '(?::([a-z0-9-]+))?(?:!([a-z0-9-]+))?(?![a-z0-9-])', 'gi');
        const diretas = [...texto.matchAll(re)];
        const nomes = [p.nome, ...(p.apelidos || [])].map(norm).filter(Boolean);
        const vistos = new Set();
        for (const d of diretas) {
          let voz = '';
          if (d[2]) {
            const misto = d[2].startsWith('misto-');
            const v = (p.vozes || []).find((x) => x.slug === d[2].replace(/^misto-/, ''));
            if (v) voz = misto ? ' · 🔀 ' + v.nome + ' + português' : ' · 🌎 ' + v.nome;
          }
          const varNome = d[1] ? ((p.variantes || []).find((v) => v.slug === d[1]) || { nome: d[1] }).nome : '';
          const rot = '🧒 ' + p.nome + (varNome ? ' · ' + varNome : '') + voz;
          if (vistos.has(rot)) continue;
          vistos.add(rot);
          achados.push({ rot, remover: () => { ta.value = ta.value.split(d[0]).join('').replace(/  +/g, ' '); detectar(); } });
        }
        if (!diretas.length && nomes.some((n) => t.includes('-' + n + '-'))) {
          achados.push({ rot: '🧒 ' + p.nome, remover: () => {
            for (const n of [p.nome, ...(p.apelidos || [])]) ta.value = ta.value.replace(new RegExp('\\b' + n + '\\b', 'gi'), '');
            detectar();
          } });
        }
      }
      for (const g of dados.grupos || []) {
        if (!new RegExp('@grupo:' + g.slug + '(?![a-z0-9-])', 'i').test(texto)) continue;
        const nomes = g.membros.map((s) => (dados.personagens.find((p) => p.slug === s) || { nome: s }).nome);
        achados.push({ rot: '👥 ' + g.nome + ': ' + nomes.join(', '), remover: () => { ta.value = ta.value.replace(new RegExp('@grupo:' + g.slug + '\\s?', 'gi'), ''); detectar(); } });
      }
      for (const l of dados.lugares || []) {
        if (!new RegExp('@lugar:' + l.slug + '(?![a-z0-9-])', 'i').test(texto)) continue;
        achados.push({ rot: '📍 ' + l.nome, remover: () => { ta.value = ta.value.replace(new RegExp('@lugar:' + l.slug + '\\s?', 'gi'), ''); detectar(); } });
      }
      for (const h of dados.historias) {
        if (new RegExp('#' + h.slug + '(?![a-z0-9-])', 'i').test(texto)) {
          achados.push({ rot: '📖 ' + h.titulo, remover: () => { ta.value = ta.value.replace(new RegExp('#' + h.slug + '\\s?', 'gi'), ''); detectar(); } });
        }
      }
      chips.replaceChildren(...(achados.length ? [el('small', { class: 'suave' }, 'Vai para a IA: ')] : []),
        ...achados.map((a) => el('span', { class: 'chip' }, a.rot, el('button', { type: 'button', class: 'link', title: 'Tirar do briefing', onclick: a.remover }, '✕'))));
    }
    detectar();
  }
})();

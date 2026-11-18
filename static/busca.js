// Busca na hora (sem recarregar) em listas da página.
// <form data-busca> com <input type=search name=q> e, opcionalmente, <select name=pers> / <select name=est>.
// Itens: [data-item] com data-texto (onde procurar), data-pers e data-est (slugs separados por espaço).
// Grupos (ex.: estantes): [data-grupo] com data-texto; se o grupo bate, aparece inteiro; senão só os itens que batem.
(function () {
  const norm = (t) => (t || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();
  document.querySelectorAll('form[data-busca]').forEach((form) => {
    const raiz = document.querySelector(form.dataset.busca) || document;
    const contador = form.querySelector('[data-contador]');
    const vazio = raiz.querySelector('[data-sem-resultado]');
    form.addEventListener('submit', (e) => e.preventDefault());
    function filtrar() {
      const q = norm(form.q.value).trim();
      // "@pedro" na busca vira filtro de personagem.
      const arrobas = (q.match(/@[a-z0-9-]+/g) || []).map((a) => a.slice(1));
      const palavras = q.replace(/@[a-z0-9-]+/g, ' ').split(/\s+/).filter(Boolean);
      const pers = [...arrobas, form.pers && form.pers.value].filter(Boolean);
      const est = form.est && form.est.value;
      const bate = (el) => {
        const texto = norm(el.dataset.texto);
        const ps = (el.dataset.pers || '').split(' ');
        return palavras.every((p) => texto.includes(p)) && pers.every((p) => ps.includes(p))
          && (!est || (el.dataset.est || '').split(' ').includes(est));
      };
      const filtrando = palavras.length || pers.length || est;
      let vistos = 0;
      const grupos = [...raiz.querySelectorAll('[data-grupo]')];
      if (grupos.length) {
        grupos.forEach((g) => {
          const itens = [...g.querySelectorAll('[data-item]')];
          const inteiro = !filtrando || (palavras.length && palavras.every((p) => norm(g.dataset.texto).includes(p)));
          let n = 0;
          itens.forEach((i) => { const ok = inteiro || bate(i); i.hidden = !ok; if (ok) n++; });
          g.hidden = !inteiro && !n;
          vistos += n;
        });
      } else {
        raiz.querySelectorAll('[data-item]').forEach((i) => { const ok = !filtrando || bate(i); i.hidden = !ok; if (ok) vistos++; });
      }
      if (contador) contador.textContent = filtrando ? vistos + ' encontrada(s)' : '';
      if (vazio) vazio.hidden = !filtrando || vistos > 0;
    }
    form.addEventListener('input', filtrar);
    form.addEventListener('change', filtrar);
  });
})();

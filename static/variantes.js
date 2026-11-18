// Página do personagem: variantes em lista (todas) ou carrossel (uma por vez, com miniaturas),
// como no editor de história. Lembra o modo e, no carrossel, a variante aberta (salvar recarrega a página).
(function () {
  // 📋 Copiar prompt da variante nova: o modelo vem pronto do servidor; troca o nome, o estilo e o que muda.
  document.querySelectorAll('[data-prompt-modelo]').forEach((b) => b.addEventListener('click', async () => {
    const f = b.form;
    const v = (n) => (f.querySelector('[name=' + n + ']')?.value || '').trim();
    let t = b.dataset.promptModelo.replaceAll('§NOME§', v('nome') || 'nova versão');
    // Sem estilo digitado, a linha do estilo vira o padrão (igual ao servidor).
    t = t.split('\n').map((l) => (!l.includes('§ESTILO§') ? l : v('estilo') ? l.replaceAll('§ESTILO§', v('estilo'))
      : 'Estilo: ilustração de livro infantil, cores vivas, traço limpo e amigável.')).join('\n');
    t = v('descricao') ? t.replaceAll('§MUDA§', v('descricao')) : t.split('\n').filter((l) => !l.includes('§MUDA§')).join('\n');
    const antes = b.textContent;
    try { await navigator.clipboard.writeText(t); b.textContent = '✅ Copiado'; } catch (_) { prompt('Copie o prompt:', t); }
    setTimeout(() => { b.textContent = antes; }, 1500);
  }));

  const secao = document.getElementById('variantes');
  if (!secao) return;
  const artigos = [...secao.querySelectorAll('article.variante')];
  if (artigos.length < 2) return;
  const criar = window.St.el;
  const chave = 'variante-aberta-' + secao.dataset.personagem;
  const ler = (k) => { try { return localStorage.getItem(k); } catch (_) { return null; } };
  const guardar = (k, v) => { try { localStorage.setItem(k, v); } catch (_) { /* */ } };

  let modo = ler('variantes-modo') === 'lista' ? 'lista' : 'carrossel';
  // Âncora #v-slug (volta de salvar/adicionar imagem): abre essa variante; senão, a última aberta.
  const daAncora = location.hash.startsWith('#v-') ? decodeURIComponent(location.hash.slice(3)) : null;
  let atual = Math.max(0, artigos.findIndex((a) => a.dataset.variante === (daAncora || ler(chave))));

  const botoesModo = criar('div', { class: 'modo-vista' });
  const nav = criar('div', { class: 'carrossel-nav' });
  secao.querySelector('h2').after(criar('div', { class: 'linha-p' }, botoesModo), nav);

  function mostrar() {
    botoesModo.replaceChildren(...['lista', 'carrossel'].map((m) => criar('button', { type: 'button', class: modo === m ? '' : 'secundario',
      onclick: () => { modo = m; guardar('variantes-modo', m); mostrar(); } }, m === 'lista' ? '📜 Lista' : '🎠 Carrossel')));
    const carrossel = modo === 'carrossel';
    nav.hidden = !carrossel;
    artigos.forEach((a, i) => { a.hidden = carrossel && i !== atual; });
    if (!carrossel) return;
    const ir = (i) => { atual = (i + artigos.length) % artigos.length; guardar(chave, artigos[atual].dataset.variante); mostrar(); };
    nav.replaceChildren(
      criar('button', { type: 'button', class: 'secundario', title: 'Anterior', onclick: () => ir(atual - 1) }, '◀'),
      criar('div', { class: 'tiras' }, artigos.map((a, i) => criar('button', {
        type: 'button', class: 'tira cor-' + (i % 5) + (i === atual ? ' ativa' : ''), title: a.dataset.nome, onclick: () => ir(i),
      }, a.dataset.mini ? criar('img', { src: a.dataset.mini, alt: '' }) : (a.dataset.nome || '?').slice(0, 1),
        criar('span', {}, String(i + 1))))),
      criar('button', { type: 'button', class: 'secundario', title: 'Próxima', onclick: () => ir(atual + 1) }, '▶'),
      criar('strong', { class: 'nome-variante' }, artigos[atual].dataset.nome));
  }
  mostrar();
  // No carrossel, a variante só aparece depois de mostrar(): rola até ela de novo.
  if (daAncora && artigos[atual]) { guardar(chave, artigos[atual].dataset.variante); artigos[atual].scrollIntoView(); }
})();

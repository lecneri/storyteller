// Acompanha desenhos na fila de imagens: <p data-fila-imagem="ID">. Atualiza o texto ("⏳ Na fila (2º)",
// "🎨 Desenhando…") e recarrega a página quando todos terminam, para mostrar as imagens novas.
// Se as assinaturas falharam e a próxima fonte é a API paga, mostra "💳 Usar API paga" / "✍️ Fazer manual".
(function () {
  const marcas = () => [...document.querySelectorAll('[data-fila-imagem]')];
  if (!marcas().length) return;
  async function decidir(id, acao) {
    try {
      await fetch('/imagens/decidir', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ids: [Number(id)], acao }) });
    } catch (_) { /* a próxima volta mostra de novo */ }
    ver();
  }
  function botoes(m, id) {
    const b = (txt, acao) => { const x = document.createElement('button'); x.type = 'button'; x.className = 'pequeno' + (acao === 'manual' ? ' secundario' : '');
      x.textContent = txt; x.onclick = () => { x.disabled = true; decidir(id, acao); }; return x; };
    m.append(' ', b('💳 Usar API paga', 'pagar'), ' ', b('✍️ Fazer manual', 'manual'));
  }
  let timer = null;
  async function ver() {
    clearTimeout(timer);
    const ids = marcas().map((m) => m.dataset.filaImagem).join(',');
    try {
      const j = await (await fetch('/imagens/fila?ids=' + ids)).json();
      let ativos = 0;
      for (const it of j.itens || []) {
        const m = document.querySelector('[data-fila-imagem="' + it.id + '"]');
        if (m) { m.textContent = it.rotulo; if (it.estado === 'aguardando') botoes(m, it.id); }
        if (['pendente', 'rodando', 'aguardando'].includes(it.estado)) ativos++;
      }
      if (!ativos) { location.reload(); return; }
    } catch (_) { /* tenta de novo */ }
    timer = setTimeout(ver, 4000);
  }
  timer = setTimeout(ver, 3000);
})();

// Acompanha desenhos na fila de imagens: <p data-fila-imagem="ID">. Atualiza o texto ("⏳ Na fila (2º)",
// "🎨 Desenhando…") e recarrega a página quando todos terminam, para mostrar as imagens novas.
(function () {
  const marcas = () => [...document.querySelectorAll('[data-fila-imagem]')];
  if (!marcas().length) return;
  async function ver() {
    const ids = marcas().map((m) => m.dataset.filaImagem).join(',');
    try {
      const j = await (await fetch('/imagens/fila?ids=' + ids)).json();
      let ativos = 0;
      for (const it of j.itens || []) {
        const m = document.querySelector('[data-fila-imagem="' + it.id + '"]');
        if (m) m.textContent = it.rotulo;
        if (it.estado === 'pendente' || it.estado === 'rodando') ativos++;
      }
      if (!ativos) { location.reload(); return; }
    } catch (_) { /* tenta de novo */ }
    setTimeout(ver, 4000);
  }
  setTimeout(ver, 3000);
})();

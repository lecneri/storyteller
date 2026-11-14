// Seção Chaves: carregar modelos do provedor, salvar o escolhido e testar.
(function () {
  const aviso = (msg) => window.St.aviso(msg, 4000); // resultados de teste: mais tempo para ler
  const post = (url, corpo) => fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: corpo ? JSON.stringify(corpo) : undefined })
    .then(async (r) => ({ ok: r.ok, j: await r.json().catch(() => ({})) }));
  function preencher(sel, lista) {
    if (!sel) return;
    const atual = sel.value;
    sel.replaceChildren(new Option('padrão', ''), ...lista.map((m) => new Option(m, m, false, m === atual)));
    if (atual && !lista.includes(atual)) sel.add(new Option(atual + ' (atual)', atual, false, true));
  }
  document.querySelectorAll('.chave-linha').forEach((linha) => {
    const base = '/chaves/' + linha.dataset.escopo + '/' + linha.dataset.provedor;
    const selTexto = linha.querySelector('select[name=modelo]');
    const selImagem = linha.querySelector('select[name=modelo_imagem]');
    linha.querySelector('[data-acao=modelos]').addEventListener('click', async (e) => {
      e.target.disabled = true; e.target.textContent = '⏳ Carregando…';
      const r = await post(base + '/modelos');
      e.target.disabled = false; e.target.textContent = '🔄 Carregar modelos';
      if (!r.ok) { aviso('❌ ' + (r.j.erro || 'falhou')); return; }
      preencher(selTexto, r.j.texto || []);
      preencher(selImagem, r.j.imagem || []);
      aviso('✅ ' + (r.j.texto || []).length + ' modelo(s) de texto' + (selImagem ? ', ' + (r.j.imagem || []).length + ' de imagem' : ''));
    });
    linha.querySelector('[data-acao=salvar]').addEventListener('click', async () => {
      const r = await post(base + '/modelo', { modelo: selTexto.value, modelo_imagem: selImagem ? selImagem.value : '' });
      aviso(r.ok ? 'Modelo salvo ✓' : '❌ ' + (r.j.erro || 'falhou'));
    });
    linha.querySelector('[data-acao=testar]').addEventListener('click', async (e) => {
      e.target.disabled = true; e.target.textContent = '⏳ Testando…';
      const r = await post(base + '/testar');
      e.target.disabled = false; e.target.textContent = '🧪 Testar';
      aviso(r.j.ok ? '✅ Respondeu: ' + r.j.resposta : '❌ ' + String(r.j.erro || 'falhou').slice(0, 220));
    });
  });
})();

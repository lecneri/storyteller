// Sistema: testar os CLIs (Claude, Codex, agy), que rodam no ia-cli.
(function () {
  const { aviso } = window.St;
  const post = (url, corpo) => fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: corpo ? JSON.stringify(corpo) : undefined })
    .then(async (r) => ({ ok: r.ok, j: await r.json().catch(() => ({})) }));

  document.querySelectorAll('.cli-linha').forEach((linha) => {
    const nome = linha.dataset.cli;
    const testar = linha.querySelector('[data-acao="testar"]');
    if (testar) testar.addEventListener('click', async () => {
      testar.disabled = true; testar.textContent = '⏳ Testando… (até 3 min)';
      let r;
      try { r = await post('/sistema/cli/' + nome + '/testar'); }
      catch (e) { r = { ok: false, j: { erro: 'a conexão caiu antes da resposta (' + e.message + ')' } }; }
      testar.disabled = false; testar.textContent = '🧪 Testar';
      // A situação da linha passa a mostrar o resultado do teste.
      const situ = linha.querySelector('[data-situacao]');
      if (situ && r.j.situacao) situ.textContent = '· ' + r.j.situacao;
      aviso(r.j.ok ? '✅ Funcionando' + (r.j.modelo ? ' · modelo: ' + r.j.modelo : '') : '❌ ' + (r.j.erro || 'falhou').slice(0, 200));
    });
  });
})();

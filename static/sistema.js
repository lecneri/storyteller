// Sistema: conectar os CLIs (Claude, Codex, agy) pelo site.
(function () {
  const { el, aviso } = window.St;
  const post = (url, corpo) => fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: corpo ? JSON.stringify(corpo) : undefined })
    .then(async (r) => ({ ok: r.ok, j: await r.json().catch(() => ({})) }));

  document.querySelectorAll('.cli-linha').forEach((linha) => {
    const nome = linha.dataset.cli;
    const fluxo = linha.querySelector('.cli-fluxo');
    let timer = null;

    // Elementos fixos: a atualização periódica só troca textos, nunca recria o campo do código.
    const msg = el('p', { class: 'suave' });
    const passo1 = el('p', {});
    const passo2 = el('p', {});
    const campo = el('input', { placeholder: 'cole o código aqui', style: 'flex:1', autocomplete: 'off' });
    const enviar = el('button', { type: 'button', class: 'pequeno' }, 'Enviar código');
    const linhaCodigo = el('div', { class: 'linha' }, campo, enviar);
    const detalhesTxt = el('pre', { class: 'cli-detalhes' });
    const detalhes = el('details', {}, el('summary', { class: 'suave' }, 'ver detalhes (o que o CLI mostrou)'), detalhesTxt);
    fluxo.replaceChildren(msg, passo1, passo2, linhaCodigo, detalhes);
    let ultimaUrl = null;

    enviar.addEventListener('click', async () => {
      const codigo = campo.value.trim();
      if (!codigo) return;
      clearTimeout(timer);
      enviar.disabled = true; campo.disabled = true;
      msg.className = 'suave'; msg.textContent = 'Código enviado, verificando… (pode levar até 1 minuto)';
      const r = await post('/sistema/cli/' + nome + '/codigo', { codigo });
      enviar.disabled = false; campo.disabled = false;
      mostrar(r.ok ? r.j : { fase: 'erro', mensagem: r.j.erro || 'erro' });
    });

    function mostrar(e) {
      fluxo.hidden = false;
      msg.className = e.fase === 'erro' ? 'erro' : e.fase === 'concluido' ? 'ok' : 'suave';
      msg.textContent = e.mensagem || '';
      const ativo = e.url && e.fase !== 'concluido' && e.fase !== 'erro';
      passo1.hidden = !ativo;
      if (ativo && e.url !== ultimaUrl) {
        ultimaUrl = e.url;
        passo1.replaceChildren('1. ', el('a', { href: e.url, target: '_blank', rel: 'noopener' }, 'Abrir o link de login ↗'));
        campo.value = '';
      }
      passo2.hidden = !(ativo && e.codigo);
      if (ativo && e.codigo) passo2.replaceChildren('2. Digite este código lá: ', el('code', { class: 'codigo-grande' }, e.codigo));
      linhaCodigo.hidden = !(ativo && e.pede_codigo);
      detalhesTxt.textContent = e.detalhes || '(nada ainda)';
      clearTimeout(timer);
      if (e.fase === 'aguardando' || e.fase === 'iniciando') {
        timer = setTimeout(async () => mostrar(await (await fetch('/sistema/cli/' + nome + '/estado')).json()), 3000);
      } else if (e.fase === 'concluido') {
        setTimeout(() => location.reload(), 2000);
      }
    }
    fluxo.hidden = true;

    linha.querySelector('[data-acao="iniciar"]').addEventListener('click', async (ev) => {
      ev.target.disabled = true;
      mostrar({ fase: 'iniciando', mensagem: 'Abrindo o login…' });
      const r = await post('/sistema/cli/' + nome + '/iniciar');
      ev.target.disabled = false;
      mostrar(r.ok ? r.j : { fase: 'erro', mensagem: r.j.erro || 'erro' });
    });
    const testar = linha.querySelector('[data-acao="testar"]');
    if (testar) testar.addEventListener('click', async () => {
      testar.disabled = true; testar.textContent = '⏳ Testando… (até 1 min)';
      let r;
      try { r = await post('/sistema/cli/' + nome + '/testar'); }
      catch (e) { r = { ok: false, j: { erro: 'a conexão caiu antes da resposta (' + e.message + ')' } }; }
      testar.disabled = false; testar.textContent = '🧪 Testar';
      // A situação da linha passa a mostrar o resultado do teste.
      const situ = linha.querySelector('[data-situacao]');
      if (situ && r.j.situacao) situ.textContent = '· ' + r.j.situacao;
      aviso(r.j.ok ? '✅ Respondeu: ' + r.j.resposta + (r.j.modelo ? ' · modelo: ' + r.j.modelo : '') : '❌ ' + (r.j.erro || 'falhou').slice(0, 200));
    });
  });
})();

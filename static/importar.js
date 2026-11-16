// Resposta de outra IA (colada ou de arquivo) vira o texto da história.
(function () {
  const caixa = document.getElementById('outra-ia');
  if (!caixa) return;
  const { aviso } = window.St;
  const resposta = document.getElementById('resposta-ia');
  const arquivo = document.getElementById('arquivo-ia');
  const botao = document.getElementById('importar-ia');

  arquivo.addEventListener('change', async () => {
    const f = arquivo.files[0];
    if (!f) return;
    if (f.size > 2_000_000) { aviso('Arquivo grande demais.'); return; }
    resposta.value = await f.text();
    aviso('Arquivo lido. Clique em “Importar resposta”.');
  });

  botao.addEventListener('click', async () => {
    const texto = resposta.value.trim();
    if (!texto) { aviso('Cole a resposta da IA ou escolha o arquivo.'); resposta.focus(); return; }
    botao.disabled = true;
    botao.textContent = '⏳ Importando…';
    try {
      const r = await fetch(`/historias/${caixa.dataset.historia}/importar`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ texto }),
      });
      const j = await r.json().catch(() => ({}));
      if (r.ok && j.ok) { location.reload(); return; }
      aviso(j.erro || 'Não deu para importar.', 6000);
    } catch (_) {
      aviso('Sem conexão com o servidor.', 5000);
    }
    botao.disabled = false;
    botao.textContent = '📥 Importar resposta';
  });
})();

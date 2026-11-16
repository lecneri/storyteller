// Página Exportar: faz o vídeo no servidor e acompanha até ficar pronto.
(function () {
  const caixa = document.getElementById('video');
  if (!caixa) return;
  const base = '/historias/' + caixa.dataset.historia;
  const estado = document.getElementById('video-estado');
  const fazer = document.getElementById('video-fazer');
  const baixar = document.getElementById('video-baixar');
  const ver = document.getElementById('video-ver');
  let espera = null;

  function mostrar(info) {
    const v = info.video || {};
    const a = info.arquivo;
    const ocupado = v.estado === 'fila' || v.estado === 'fazendo';
    fazer.disabled = ocupado;
    fazer.textContent = ocupado ? '⏳ Fazendo…' : (a ? '🔄 Fazer de novo' : '🎬 Fazer o vídeo');
    baixar.hidden = ver.hidden = !(a && v.estado === 'pronto');
    let t = '';
    if (ocupado) t = '⏳ ' + (v.progresso || 'fazendo') + '… pode sair desta página, o vídeo continua.';
    else if (v.estado === 'erro') t = '😕 Não deu: ' + (v.erro || 'erro');
    else if (a) t = '✅ Pronto (' + a.mb + ' MB, feito em ' + a.feito + ').' + (v.aviso ? ' ⚠️ ' + v.aviso : '');
    estado.textContent = t;
    clearTimeout(espera);
    if (ocupado) espera = setTimeout(atualizar, 3000);
  }
  async function atualizar() {
    try {
      const r = await fetch(base + '/video/estado');
      if (r.ok) mostrar(await r.json());
    } catch (_) { espera = setTimeout(atualizar, 5000); }
  }
  fazer.addEventListener('click', async () => {
    fazer.disabled = true;
    const r = await fetch(base + '/video', { method: 'POST' });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) { estado.textContent = '😕 ' + (j.erro || 'erro ' + r.status); fazer.disabled = false; return; }
    mostrar(j);
  });
  try { mostrar(JSON.parse(document.getElementById('dados-video').textContent)); } catch (_) { /* */ }
})();

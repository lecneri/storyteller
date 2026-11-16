// Clique numa miniatura (ou em "Copiar") copia a imagem como PNG para colar na IA.
(function () {
  const { aviso } = window.St;
  async function pngDe(url) {
    const img = new Image();
    img.src = url;
    await img.decode();
    const c = document.createElement('canvas');
    c.width = img.naturalWidth; c.height = img.naturalHeight;
    c.getContext('2d').drawImage(img, 0, 0);
    return new Promise((ok, erro) => c.toBlob((b) => (b ? ok(b) : erro(new Error('falhou'))), 'image/png'));
  }
  async function copiar(url) {
    if (!navigator.clipboard || !window.ClipboardItem) {
      aviso('Seu navegador não copia imagens. Use ⬇ para baixar.');
      return;
    }
    try {
      // Passar a Promise direto mantém o "gesto do usuário" no Safari.
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': pngDe(url) })]);
      aviso('Imagem copiada! Cole (Ctrl+V) no ChatGPT ou no Gemini.');
    } catch (e) {
      aviso('Não consegui copiar. Use ⬇ para baixar.');
    }
  }
  // Prompt e cena recolhidos: clicar (ou chegar com Tab) abre com altura fixa e rolagem; ao sair, recolhe.
  const RECOLHIDOS = 'textarea.prompt, textarea.recolhe';
  document.addEventListener('focusin', (ev) => {
    const ta = ev.target.matches && ev.target.matches(RECOLHIDOS) ? ev.target : null;
    if (ta && !ta.classList.contains('aberto')) { ta.classList.add('aberto'); ta.scrollTop = 0; }
  });
  document.addEventListener('focusout', (ev) => {
    if (ev.target.matches && ev.target.matches(RECOLHIDOS)) { ev.target.classList.remove('aberto'); ev.target.scrollTop = 0; }
  });
  document.addEventListener('click', async (ev) => {
    const bt = ev.target.closest('[data-copiar-texto]');
    if (bt) {
      const ta = document.getElementById(bt.dataset.copiarTexto);
      try { await navigator.clipboard.writeText(ta.value); } catch (_) { ta.select(); document.execCommand('copy'); }
      aviso('Prompt copiado! Cole no ChatGPT ou no Gemini.');
      return;
    }
    const img = ev.target.closest('img[data-copiar]');
    const btn = ev.target.closest('[data-copiar-de]');
    if (img) copiar(img.currentSrc || img.src);
    else if (btn) copiar(btn.dataset.copiarDe);
  });
})();

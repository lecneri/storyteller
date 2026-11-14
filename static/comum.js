// Helpers compartilhados pelas páginas (carregado antes dos outros scripts): window.St.
(function (raiz) {
  // Cria um elemento: el('button', { class: 'x', onclick: fn, disabled: false }, 'texto', filho...).
  // Atributos null/undefined/false são ignorados; true vira atributo vazio; "on…" vira evento.
  function el(tag, attrs = {}, ...filhos) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === 'class') e.className = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? '' : v);
    }
    for (const f of filhos.flat(Infinity)) if (f !== null && f !== undefined && f !== false) e.append(f.nodeType ? f : document.createTextNode(f));
    return e;
  }

  // Mensagem rápida no #aviso da página (some sozinha).
  function aviso(msg, ms = 2500) {
    const a = document.getElementById('aviso');
    if (!a) { console.log(msg); return; }
    a.textContent = msg; a.hidden = false;
    clearTimeout(a._t); a._t = setTimeout(() => (a.hidden = true), ms);
  }

  // POST com JSON; devolve o JSON da resposta ou lança Error com a mensagem do servidor ({erro}).
  async function postJson(url, corpo) {
    const r = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: corpo === undefined ? undefined : JSON.stringify(corpo) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.erro || 'erro ' + r.status);
    return j;
  }

  raiz.St = { el, aviso, postJson };
})(window);

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

  // <select> de voz: vozes grátis + as da conta ElevenLabs da família (dados vem de GET /vozes).
  // Muda `voz` (motor, id, nome, reserva) e chama depois() a cada troca.
  function seletorVoz(dados, voz, depois) {
    const eleven = [...(dados.eleven || [])];
    if (voz.motor === 'eleven' && !eleven.some((v) => v.id === voz.id)) {
      eleven.push({ id: voz.id, nome: (voz.nome || voz.id) + ' (não está mais na conta)', reserva: voz.reserva });
    }
    const opcao = (motor, v) => el('option', { value: motor + ':' + v.id, selected: (voz.motor === 'eleven') === (motor === 'eleven') && v.id === voz.id }, v.nome);
    const grupos = [el('optgroup', { label: 'Vozes grátis' }, dados.vozes.map((v) => opcao('edge', v)))];
    if (eleven.length) grupos.unshift(el('optgroup', { label: 'ElevenLabs (usa créditos)' }, eleven.map((v) => opcao('eleven', v))));
    return el('select', { onchange: (e) => {
      const [motor, id] = e.target.value.split(':');
      const v = eleven.find((x) => x.id === id);
      Object.assign(voz, motor === 'eleven' ? { motor, id, nome: v.nome, reserva: v.reserva, tom: 0 } : { motor, id, nome: '', reserva: '' });
      if (depois) depois();
    } }, grupos);
  }

  raiz.St = { el, aviso, postJson, seletorVoz };
})(window);

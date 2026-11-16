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

  // Área para colar (Ctrl+V), arrastar ou usar o botão "📋 Colar" junto de um <input type=file>.
  // Os arquivos colados entram no próprio input (respeitando accept e multiple) e disparam "change".
  function zonaColar(input, texto) {
    const aceita = (f) => {
      const tipos = (input.accept || '').split(',').map((t) => t.trim()).filter(Boolean);
      return !tipos.length || tipos.some((t) => (t.endsWith('/*') ? f.type.startsWith(t.slice(0, -1)) : f.type === t));
    };
    const lista = el('small', { class: 'suave' });
    // Prévia: miniatura de cada imagem escolhida/colada/arrastada, com ✕ para tirar.
    const previa = el('div', { class: 'previa-envio', hidden: '' });
    let urls = [];
    const mostrar = () => {
      const n = input.files.length;
      lista.textContent = n ? '✅ ' + (n === 1 ? input.files[0].name || '1 imagem' : n + ' imagens') + ' pronta(s)' : '';
      urls.forEach((u) => URL.revokeObjectURL(u));
      urls = [];
      const imagens = [...input.files].map((f, i) => ({ f, i })).filter(({ f }) => f.type.startsWith('image/'));
      previa.hidden = !imagens.length;
      previa.replaceChildren(...imagens.map(({ f, i }) => {
        const u = URL.createObjectURL(f);
        urls.push(u);
        return el('figure', { title: f.name }, el('img', { src: u, alt: f.name }),
          el('button', { type: 'button', class: 'tirar', title: 'Tirar esta imagem', onclick: () => tirar(i) }, '✕'));
      }));
    };
    function tirar(i) {
      const dt = new DataTransfer();
      [...input.files].forEach((f, j) => { if (j !== i) dt.items.add(f); });
      input.files = dt.files;
      input.dispatchEvent(new Event('change', { bubbles: true }));
    }
    function receber(arquivos) {
      const novos = [...arquivos].filter(aceita);
      if (!novos.length) { aviso('Não há imagem copiada para colar.'); return; }
      const dt = new DataTransfer();
      if (input.multiple) for (const f of input.files) dt.items.add(f);
      for (const f of input.multiple ? novos : novos.slice(0, 1)) {
        // Imagem colada vem como "image.png": dá um nome único para não sobrescrever.
        const nome = f.name && f.name !== 'image.png' ? f.name : 'colada-' + Date.now() + '-' + dt.items.length + '.' + (f.type.split('/')[1] || 'png');
        dt.items.add(new File([f], nome, { type: f.type }));
      }
      input.files = dt.files;
      input.dispatchEvent(new Event('change', { bubbles: true }));
      mostrar();
    }
    // Botão para celular (não há Ctrl+V): lê a área de transferência direto.
    async function colarDoBotao() {
      try {
        const itens = await navigator.clipboard.read();
        const arquivos = [];
        for (const item of itens) {
          const tipo = item.types.find((t) => t.startsWith('image/'));
          if (tipo) arquivos.push(new File([await item.getType(tipo)], '', { type: tipo }));
        }
        receber(arquivos);
      } catch (e) {
        aviso('O navegador não deixou ler a imagem copiada. Clique na área tracejada e aperte Ctrl+V.', 4000);
      }
    }
    const zona = el('div', { class: 'zona-envio', tabindex: 0, title: 'Clique aqui e cole com Ctrl+V',
      onclick: () => zona.focus(),
      ondragover: (e) => { e.preventDefault(); zona.classList.add('arrastando'); },
      ondragleave: () => zona.classList.remove('arrastando'),
      ondrop: (e) => { e.preventDefault(); zona.classList.remove('arrastando'); receber(e.dataTransfer.files); },
      onpaste: (e) => { e.preventDefault(); receber((e.clipboardData || {}).files || []); } },
      el('span', { class: 'zona-normal' }, texto || '📋 Clique aqui e cole (Ctrl+V) ou arraste a imagem'),
      el('span', { class: 'zona-pronta' }, '✅ Pronto! Agora aperte Ctrl+V'));
    input.addEventListener('change', mostrar);
    const caixa = el('div', { class: 'envio colar' }, zona, previa,
      el('span', { class: 'linha' }, lista,
        navigator.clipboard && navigator.clipboard.read ? el('button', { type: 'button', class: 'pequeno secundario', onclick: colarDoBotao }, '📋 Colar') : null));
    // Dentro de um <label>, clicar na área abriria a janela de arquivos: fica logo depois do label.
    const label = input.closest('label');
    if (label && label.classList.contains('inteiro')) caixa.classList.add('inteiro');
    (label || input).after(caixa);
    return caixa;
  }

  raiz.St = { el, aviso, postJson, seletorVoz, zonaColar };
})(window);

// Monta as folhas para imprimir. Vídeo e GIF viram um quadro (/midia/{id}/quadro).
(function () {
  const dados = JSON.parse(document.getElementById('dados').textContent);
  const livro = document.getElementById('livro');
  const modo = document.getElementById('modo');
  const folha = document.getElementById('folha');
  const estado = document.getElementById('estado');
  const sortear = document.getElementById('sortear');
  const el = (tag, attrs = {}, ...filhos) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) if (v != null) e.setAttribute(k, v);
    for (const f of filhos.flat()) if (f != null) e.append(f);
    return e;
  };
  let rodada = 0;
  // Toda mídia passa por /quadro: imagem parada volta como ela mesma; vídeo/GIF vira JPEG.
  const src = (url) => url + '/quadro?modo=' + modo.value + '&n=' + rodada;
  const imagem = (url) => (url ? el('div', { class: 'imagem' }, el('img', { src: src(url), alt: '' })) : null);

  function desenhar() {
    rodada++;
    const fala = (b) => b.quem !== 'narrador' && dados.narracao !== 'narrador';
    const primeira = (dados.paginas.find((p) => p.midia) || {}).midia;
    const folhas = [el('section', { class: 'folha capa' }, imagem(dados.capa || (primeira && primeira.url)), el('h1', {}, dados.titulo))];
    dados.paginas.forEach((p, i) => {
      folhas.push(el('section', { class: 'folha' },
        imagem(p.midia && p.midia.url),
        el('div', { class: 'texto' }, p.blocos.map((b) => fala(b)
          ? el('p', { class: 'fala' }, el('span', { class: 'quem' }, b.nome + ': '), '— ' + b.texto)
          : el('p', {}, b.texto))),
        el('div', { class: 'numero' }, String(i + 1))));
    });
    livro.replaceChildren(...folhas);
    esperar();
  }
  function imagensProntas() {
    return Promise.all([...livro.querySelectorAll('img')].map((img) => (img.complete ? null
      : new Promise((ok) => { img.onload = ok; img.onerror = ok; }))));
  }
  async function esperar() {
    estado.textContent = '⏳ Preparando as imagens…';
    await imagensProntas();
    estado.textContent = '✅ Pronto para imprimir (' + dados.paginas.length + ' páginas + capa)';
  }
  function aplicarFolha() {
    const deitada = folha.value === 'paisagem';
    document.body.classList.toggle('paisagem', deitada);
    document.getElementById('pagina-papel').textContent = '@page { size: A4 ' + (deitada ? 'landscape' : 'portrait') + '; margin: 12mm; }';
  }
  modo.addEventListener('change', () => { sortear.hidden = modo.value !== 'aleatorio'; desenhar(); });
  sortear.addEventListener('click', desenhar);
  folha.addEventListener('change', aplicarFolha);
  document.getElementById('imprimir').addEventListener('click', async () => {
    await imagensProntas();
    window.print();
  });
  aplicarFolha();
  desenhar();
})();

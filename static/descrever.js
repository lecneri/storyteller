// "📷 Descrever pela foto": envia as fotos para a IA e preenche o campo de aparência/descrição.
// Marcação: <span data-descrever data-alvo="ID do textarea" data-nome="..." data-personagem="slug (só em variante)"></span>
// Com data-fotos="/midia/a /midia/b" (fotos já guardadas na ficha), descreve elas, sem pedir arquivo.
document.querySelectorAll("[data-descrever]").forEach((caixa) => {
  const alvo = document.getElementById(caixa.dataset.alvo);
  if (!alvo) return;
  if (caixa.dataset.fotos) return guardadas(caixa, alvo);
  const entrada = Object.assign(document.createElement("input"), { type: "file", accept: "image/png,image/jpeg,image/webp", multiple: true, hidden: true });
  const botao = Object.assign(document.createElement("button"), { type: "button", className: "secundario", textContent: "📷 Descrever pela foto" });
  const aviso = Object.assign(document.createElement("small"), { className: "suave" });
  aviso.textContent = " A foto vai só para a IA e não fica guardada.";
  caixa.append(botao, entrada, aviso);
  // Com data-fonte, a imagem vem do campo do formulário (que já tem a área de colar): nada de segunda área.
  if (window.St && !caixa.dataset.fonte) window.St.zonaColar(entrada, '📋 Ou clique aqui e cole a foto (Ctrl+V) para a IA descrever');
  // data-fonte: um campo de imagem do próprio formulário; se já tiver imagem escolhida, descreve ela.
  const fonte = caixa.dataset.fonte ? document.getElementById(caixa.dataset.fonte) : null;
  if (fonte) {
    const legenda = () => { botao.textContent = fonte.files.length ? "📷 Descrever pela imagem escolhida" : "📷 Descrever pela foto"; };
    fonte.addEventListener("change", legenda);
    caixa.addEventListener("legenda", legenda);
  }
  botao.addEventListener("click", () => (fonte && fonte.files.length ? descrever(fonte.files) : entrada.click()));
  entrada.addEventListener("change", () => descrever(entrada.files));
  async function descrever(arquivos) {
    if (!arquivos.length) return;
    const dados = new FormData();
    for (const f of arquivos) dados.append("fotos", f, f.name);
    const form = alvo.form;
    const nome = caixa.dataset.nome || form?.querySelector("[name=nome]")?.value || "";
    const estilo = form?.querySelector("[name=estilo]")?.value || "";
    dados.append("nome", nome);
    dados.append("estilo", estilo);
    // Variante: a IA descreve só o que muda em relação à ficha do personagem.
    if (caixa.dataset.personagem) dados.append("personagem", caixa.dataset.personagem);
    botao.disabled = true;
    botao.textContent = "⏳ Olhando a foto…";
    aviso.textContent = "";
    try {
      const r = await fetch("/personagens/descrever-foto", { method: "POST", body: dados });
      const j = await r.json().catch(() => ({}));
      if (!r.ok || !j.aparencia) throw new Error(j.erro || "A IA não respondeu.");
      alvo.value = j.aparencia;
      alvo.rows = Math.max(alvo.rows, 4);
      aviso.textContent = " ✅ Pronto! Revise e salve.";
    } catch (e) {
      aviso.textContent = " ❌ " + e.message;
    } finally {
      botao.disabled = false;
      botao.textContent = "📷 Descrever pela foto";
      entrada.value = "";
      caixa.dispatchEvent(new Event("legenda"));
    }
  }
});

function guardadas(caixa, alvo) {
  const botao = Object.assign(document.createElement("button"), { type: "button", className: "secundario", textContent: "✨ Descrever pela foto" });
  const aviso = Object.assign(document.createElement("small"), { className: "suave" });
  caixa.append(botao, aviso);
  botao.addEventListener("click", async () => {
    if (alvo.value.trim() && !confirm("Trocar a aparência pela descrição da foto?")) return;
    botao.disabled = true;
    botao.textContent = "⏳ Olhando a foto…";
    aviso.textContent = "";
    try {
      const dados = new FormData();
      for (const url of caixa.dataset.fotos.split(" ").filter(Boolean).slice(0, 4)) {
        const r = await fetch(url);
        if (r.ok) dados.append("fotos", await r.blob(), "foto");
      }
      dados.append("nome", alvo.form?.querySelector("[name=nome]")?.value || "");
      dados.append("ficha", "1");
      const r = await fetch("/personagens/descrever-foto", { method: "POST", body: dados });
      const j = await r.json().catch(() => ({}));
      if (!r.ok || !j.aparencia) throw new Error(j.erro || "A IA não respondeu.");
      alvo.value = j.aparencia;
      aviso.textContent = " ✅ Pronto! Revise e salve a ficha.";
    } catch (e) {
      aviso.textContent = " ❌ " + e.message;
    } finally {
      botao.disabled = false;
      botao.textContent = "✨ Descrever pela foto";
    }
  });
}

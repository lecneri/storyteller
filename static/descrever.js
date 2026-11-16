// "📷 Descrever pela foto": envia as fotos para a IA e preenche o campo de aparência/descrição.
// Marcação: <span data-descrever data-alvo="ID do textarea" data-nome="..."></span>
document.querySelectorAll("[data-descrever]").forEach((caixa) => {
  const alvo = document.getElementById(caixa.dataset.alvo);
  if (!alvo) return;
  const entrada = Object.assign(document.createElement("input"), { type: "file", accept: "image/png,image/jpeg,image/webp", multiple: true, hidden: true });
  const botao = Object.assign(document.createElement("button"), { type: "button", className: "secundario", textContent: "📷 Descrever pela foto" });
  const aviso = Object.assign(document.createElement("small"), { className: "suave" });
  aviso.textContent = " A foto vai só para a IA e não fica guardada.";
  caixa.append(botao, entrada, aviso);
  if (window.St) window.St.zonaColar(entrada, '📋 Ou clique aqui e cole a foto (Ctrl+V) para a IA descrever');
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

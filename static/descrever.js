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
  botao.addEventListener("click", () => entrada.click());
  entrada.addEventListener("change", async () => {
    if (!entrada.files.length) return;
    const dados = new FormData();
    for (const f of entrada.files) dados.append("fotos", f, f.name);
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
    }
  });
});

//! CLIs de assinatura (Claude Code, Codex, Antigravity/agy). Todos rodam isolados: pasta temporária
//! vazia, ambiente limpo e sem ferramentas — o briefing vem das famílias, então nada de shell,
//! arquivos ou variáveis do servidor ao alcance da IA.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{anyhow, bail, Context};
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command};

use super::{api::conteudo_anthropic, extrair_json, resumo, PedidoTexto, TEMPO_MAXIMO};

/// Pasta temporária que se apaga sozinha (inclusive quando a IA estoura o tempo ou dá erro).
struct PastaTemp(PathBuf);

impl PastaTemp {
    async fn nova() -> anyhow::Result<Self> {
        let dir = std::env::temp_dir().join(format!("st-ia-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(PastaTemp(dir))
    }
    fn caminho(&self) -> &Path {
        &self.0
    }
}

impl Drop for PastaTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Comando com ambiente limpo (só as variáveis indicadas) rodando dentro da pasta temporária.
fn comando_isolado(programa: &str, pasta: &Path, variaveis: &[&str]) -> Command {
    let mut cmd = Command::new(programa);
    cmd.current_dir(pasta).env_clear().stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    for var in variaveis {
        if let Ok(v) = std::env::var(var) {
            cmd.env(var, v);
        }
    }
    cmd
}

const AMBIENTE: [&str; 3] = ["HOME", "PATH", "LANG"];

// ───────────── modelo e esforço por família ─────────────

/// Níveis de esforço que as CLIs entendem (claude e agy: --effort; codex: model_reasoning_effort).
pub const ESFORCOS: [(&str, &str); 5] = [("low", "Baixo"), ("medium", "Médio"), ("high", "Alto"), ("xhigh", "Muito alto"), ("max", "Máximo")];

/// Modelo e esforço escolhidos para uma CLI (por família). `None` = padrão da CLI.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct AjusteCli {
    pub modelo: Option<String>,
    pub esforco: Option<String>,
}

impl AjusteCli {
    /// Lê de `ia.config_familia.cli_ajustes` ({"claude": {"modelo": "opus", "esforco": "high"}}).
    pub fn de(ajustes: &Value, cli: &str) -> Self {
        let texto = |k: &str| ajustes[cli][k].as_str().map(str::trim).filter(|t| nome_seguro(t)).map(String::from);
        AjusteCli { modelo: texto("modelo"), esforco: texto("esforco").filter(|e| ESFORCOS.iter().any(|(k, _)| k == e)) }
    }
}

/// Nome de modelo que pode ir como argumento: letras, números e . _ - : / (nunca começando com "-").
pub fn nome_seguro(t: &str) -> bool {
    !t.is_empty() && t.len() <= 80 && !t.starts_with('-') && t.chars().all(|c| c.is_ascii_alphanumeric() || "._-:/".contains(c))
}

/// Modelo que a CLI usa quando ninguém escolhe (configuração dela no servidor), para mostrar no Sistema.
pub fn modelo_padrao(cli: &str) -> String {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/app".into()));
    let json = |p: &str| std::fs::read_to_string(home.join(p)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let achado = match cli {
        "claude" => json(".claude/settings.json").and_then(|v| v["model"].as_str().map(String::from)),
        "agy" => json(".gemini/antigravity-cli/settings.json").and_then(|v| v["model"].as_str().map(String::from)),
        "codex" => std::fs::read_to_string(home.join(".codex/config.toml")).ok().and_then(|t| {
            t.lines().find_map(|l| l.trim().strip_prefix("model").map(str::trim).and_then(|r| r.strip_prefix('=')).map(|r| r.trim().trim_matches('"').to_string()))
        }),
        _ => None,
    };
    achado.unwrap_or_else(|| format!("padrão do {}", if cli == "claude" { "Claude (da conta)" } else if cli == "codex" { "Codex" } else { "agy" }))
}

/// Modelos para sugerir na tela: (id, nome). Claude: apelidos; Codex: a lista que ele guarda em cache; agy: `agy models`.
pub async fn modelos(cli: &str) -> Vec<(String, String)> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/app".into()));
    match cli {
        "claude" => [("fable", "Fable (o mais novo)"), ("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku (rápido)")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
        "codex" => tokio::fs::read_to_string(home.join(".codex/models_cache.json"))
            .await
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| {
                v["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| m["visibility"] != "hide")
                    .filter_map(|m| Some((m["slug"].as_str()?.to_string(), m["display_name"].as_str().unwrap_or_default().to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        "agy" => modelos_agy().await,
        _ => Vec::new(),
    }
}

/// `agy models` (id<TAB>nome), guardado por 1 hora.
async fn modelos_agy() -> Vec<(String, String)> {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<(String, String)>)>> = std::sync::Mutex::new(None);
    if let Some((quando, v)) = CACHE.lock().ok().and_then(|c| c.clone()) {
        if quando.elapsed() < std::time::Duration::from_secs(3600) {
            return v;
        }
    }
    let Ok(pasta) = PastaTemp::nova().await else { return Vec::new() };
    let mut cmd = comando_isolado("agy", pasta.caminho(), &["HOME", "PATH", "LANG", "DBUS_SESSION_BUS_ADDRESS"]);
    cmd.arg("models").stdin(Stdio::null());
    if let Some(k) = crate::config::get("GEMINI_API_KEY") {
        cmd.env("GEMINI_API_KEY", k);
    }
    let Ok(Ok(saida)) = tokio::time::timeout(std::time::Duration::from_secs(40), cmd.output()).await else { return Vec::new() };
    let v: Vec<(String, String)> = String::from_utf8_lossy(&saida.stdout)
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))
        .filter(|(a, _)| nome_seguro(a))
        .collect();
    if !v.is_empty() {
        if let Ok(mut c) = CACHE.lock() {
            *c = Some((std::time::Instant::now(), v.clone()));
        }
    }
    v
}

/// Modelo que o Claude informa na resposta (modelUsage).
fn modelo_claude(stdout: &str) -> Option<String> {
    let linha = stdout.lines().rev().find(|l| l.contains("modelUsage")).unwrap_or(stdout);
    let v: Value = serde_json::from_str(linha.trim()).ok()?;
    v["modelUsage"].as_object()?.keys().next().cloned()
}

/// Modelo que o Codex escreve no cabeçalho ("model: gpt-...").
fn modelo_codex(saida: &std::process::Output) -> Option<String> {
    let texto = String::from_utf8_lossy(&saida.stderr).to_string() + &String::from_utf8_lossy(&saida.stdout);
    texto.lines().find_map(|l| l.trim().strip_prefix("model:").map(|m| m.trim().to_string())).filter(|m| !m.is_empty())
}

/// O agy não diz o modelo: vale o escolhido ou o padrão dele.
fn modelo_agy(a: &AjusteCli) -> Option<String> {
    Some(a.modelo.clone().unwrap_or_else(|| modelo_padrao("agy")))
}

fn estourou(_: tokio::time::error::Elapsed) -> anyhow::Error {
    anyhow!("a IA demorou mais de 5 minutos")
}

/// Pedido para CLIs que recebem tudo num texto só (sistema + pedido).
fn prompt_unico(p: &PedidoTexto) -> String {
    format!("{}\nNão use ferramentas nem crie arquivos: responda só com o JSON.\n\n{}", p.sistema, p.prompt)
}

// ───────────── Claude Code ─────────────

/// Roda `claude -p` sem nenhuma ferramenta. Com imagens, o pedido vai como mensagem stream-json.
/// Devolve a resposta e o modelo usado.
pub async fn claude_cli(p: &PedidoTexto, a: &AjusteCli) -> anyhow::Result<(Value, Option<String>)> {
    let pasta = PastaTemp::nova().await?;
    let com_imagens = !p.imagens.is_empty();
    let mut cmd = comando_isolado("claude", pasta.caminho(), &AMBIENTE);
    cmd.args(["-p", "--tools", "", "--no-session-persistence", "--strict-mcp-config"]);
    if let Some(m) = &a.modelo {
        cmd.arg("--model").arg(m);
    }
    if let Some(e) = &a.esforco {
        cmd.arg("--effort").arg(e);
    }
    if com_imagens {
        cmd.args(["--input-format", "stream-json", "--output-format", "stream-json", "--verbose"]);
    } else {
        cmd.args(["--output-format", "json"]);
    }
    cmd.arg("--system-prompt").arg(&p.sistema).arg("--json-schema").arg(p.schema.to_string()).stdin(Stdio::piped());
    if let Some(token) = crate::config::get("CLAUDE_CODE_OAUTH_TOKEN") {
        cmd.env("CLAUDE_CODE_OAUTH_TOKEN", token);
    }

    let mut filho = cmd.spawn().context("não consegui iniciar o claude (CLI instalado?)")?;
    if let Some(mut stdin) = filho.stdin.take() {
        let entrada = if com_imagens {
            let msg = serde_json::json!({ "type": "user", "message": { "role": "user", "content": conteudo_anthropic(p) } });
            format!("{msg}\n")
        } else {
            p.prompt.clone()
        };
        stdin.write_all(entrada.as_bytes()).await?;
    }
    let saida = tokio::time::timeout(TEMPO_MAXIMO, filho.wait_with_output()).await.map_err(estourou)??;
    let stdout = String::from_utf8_lossy(&saida.stdout);
    let v = ler_resposta_claude(&stdout, com_imagens).with_context(|| format!("stderr: {}", resumo(&String::from_utf8_lossy(&saida.stderr))))?;
    Ok((v, modelo_claude(&stdout).or_else(|| a.modelo.clone())))
}

/// Interpreta a saída do `claude -p`: um JSON (`--output-format json`) ou, em stream-json,
/// várias linhas em que a última com `"type":"result"` traz o resultado.
fn ler_resposta_claude(stdout: &str, stream: bool) -> anyhow::Result<Value> {
    let linha = if stream {
        stdout.lines().rev().find(|l| l.contains("\"type\":\"result\"")).unwrap_or_default()
    } else {
        stdout.trim()
    };
    let json: Value =
        serde_json::from_str(linha.trim()).with_context(|| format!("resposta inesperada do claude: {}", resumo(stdout)))?;
    if json.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        bail!("o claude devolveu erro: {}", resumo(json.get("result").and_then(Value::as_str).unwrap_or("?")));
    }
    if let Some(estruturado) = json.get("structured_output").filter(|v| !v.is_null()) {
        return Ok(estruturado.clone());
    }
    // Sem saída estruturada: tenta ler o texto como JSON (com ou sem ```json).
    let texto = json.get("result").and_then(Value::as_str).unwrap_or_default();
    extrair_json(texto).context("a IA não devolveu JSON válido")
}

// ───────────── Codex (ChatGPT) ─────────────

/// Codex CLI: somente leitura, sem pedir aprovação, resposta no formato do schema (gravada em arquivo).
/// Devolve a resposta e o modelo usado.
pub async fn codex_cli(p: &PedidoTexto, a: &AjusteCli) -> anyhow::Result<(Value, Option<String>)> {
    let pasta = PastaTemp::nova().await?;
    let dir = pasta.caminho();
    let schema = dir.join("schema.json");
    let resposta = dir.join("resposta.json");
    tokio::fs::write(&schema, p.schema.to_string()).await?;
    let mut anexos = Vec::new();
    for (i, (mime, b)) in p.imagens.iter().enumerate() {
        let ext = mime.rsplit('/').next().unwrap_or("png");
        let caminho = dir.join(format!("foto-{i}.{ext}"));
        tokio::fs::write(&caminho, b).await?;
        anexos.push(caminho);
    }
    let mut cmd = comando_isolado("codex", dir, &AMBIENTE);
    cmd.args(["exec", "--skip-git-repo-check", "--ephemeral", "--sandbox", "read-only", "-c", "approval_policy=\"never\""])
        .args(args_codex(a))
        .arg("--output-schema")
        .arg(&schema)
        .arg("-o")
        .arg(&resposta)
        .arg("-C")
        .arg(dir)
        // "--image=caminho" (com =): a opção aceita vários valores e engoliria o prompt.
        .args(anexos.iter().map(|a| format!("--image={}", a.display())))
        .arg(prompt_unico(p))
        .stdin(Stdio::null());
    let saida = tokio::time::timeout(TEMPO_MAXIMO, cmd.output()).await.map_err(estourou)??;
    match tokio::fs::read_to_string(&resposta).await {
        Ok(t) if !t.trim().is_empty() => Ok((extrair_json(&t).context("o Codex não devolveu JSON válido")?, modelo_codex(&saida).or_else(|| a.modelo.clone()))),
        _ => bail!("codex falhou: {}", resumo(ultima_linha(&String::from_utf8_lossy(&saida.stderr)).unwrap_or("sem resposta"))),
    }
}

/// Modelo (-m) e esforço (-c model_reasoning_effort) do Codex.
fn args_codex(a: &AjusteCli) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(m) = &a.modelo {
        v.extend(["-m".to_string(), m.clone()]);
    }
    if let Some(e) = &a.esforco {
        v.extend(["-c".to_string(), format!("model_reasoning_effort=\"{e}\"")]);
    }
    v
}

/// Modelo (--model) e esforço (--effort) do agy.
fn args_agy(a: &AjusteCli) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(m) = &a.modelo {
        v.extend(["--model".to_string(), m.clone()]);
    }
    if let Some(e) = &a.esforco {
        v.extend(["--effort".to_string(), e.clone()]);
    }
    v
}

fn ultima_linha(texto: &str) -> Option<&str> {
    texto.lines().rev().find(|l| !l.trim().is_empty())
}

// ───────────── Antigravity (agy, Gemini) ─────────────

/// Para o botão "Testar" do sistema.
pub async fn testar_agy(p: &PedidoTexto, a: &AjusteCli) -> anyhow::Result<(Value, Option<String>)> {
    agy_cli(p, a).await
}

/// Antigravity CLI (Gemini da assinatura Google). Nega todas as permissões (sem isso o agy fica
/// parado pedindo aprovação) e roda em modo plano. Não aceita imagens anexadas.
pub(super) async fn agy_cli(p: &PedidoTexto, a: &AjusteCli) -> anyhow::Result<(Value, Option<String>)> {
    let pasta = PastaTemp::nova().await?;
    let negar = r#"{"permissions":{"allow":[],"deny":["*","command(*)","write_file(*)","edit_file(*)"]}}"#;
    let mut cmd = comando_isolado("agy", pasta.caminho(), &["HOME", "PATH", "LANG", "DBUS_SESSION_BUS_ADDRESS"]);
    // O pedido vai logo depois do -p (o agy usa o argumento seguinte ao -p como prompt).
    cmd.args(["--output-format", "json", "--mode", "plan", "--print-timeout", "240s", "--settings", negar])
        .args(args_agy(a))
        .arg("--json-schema")
        .arg(p.schema.to_string())
        .arg("-p")
        .arg(prompt_unico(p));
    if let Some(k) = crate::config::get("GEMINI_API_KEY") {
        cmd.env("GEMINI_API_KEY", k);
    }
    let saida = tokio::time::timeout(TEMPO_MAXIMO, cmd.output()).await.map_err(estourou)??;
    let stdout = String::from_utf8_lossy(&saida.stdout);
    if !saida.status.success() {
        bail!("agy falhou: {} {}", resumo(&stdout), resumo(&String::from_utf8_lossy(&saida.stderr)));
    }
    Ok((ler_resposta_agy(&stdout)?, modelo_agy(a)))
}

/// A saída json do agy pode vir como envelope ({structured_output} ou {result: "..."}) ou direto como o objeto.
fn ler_resposta_agy(stdout: &str) -> anyhow::Result<Value> {
    let v: Value = serde_json::from_str(stdout.trim()).or_else(|_| extrair_json(stdout))?;
    if let Some(r) = v.get("structured_output").filter(|x| !x.is_null()) {
        return Ok(r.clone());
    }
    if v.get("status").and_then(Value::as_str).is_some_and(|s| s != "SUCCESS") {
        bail!("agy não concluiu: {}", resumo(&v.to_string()));
    }
    if let Some(r) = v.get("result").or_else(|| v.get("response")).and_then(Value::as_str) {
        return extrair_json(r);
    }
    Ok(v)
}

// ───────────── imagens pela assinatura (Codex e agy) ─────────────

/// CLIs que geram imagem.
pub const CLIS_IMAGEM: [&str; 2] = ["codex", "agy"];

/// Onde cada CLI guarda as imagens que gera (dentro do HOME). Único lugar que sabe disso.
const PASTAS_IMAGEM: [(&str, &str); 2] = [("codex", ".codex/generated_images"), ("agy", ".gemini/antigravity-cli/brain")];

fn pasta_imagens(nome: &str) -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/app".into()));
    PASTAS_IMAGEM.iter().find(|(n, _)| *n == nome).map(|(_, p)| home.join(p))
}

/// Imagens dentro de `raiz` (recursivo), com a data de modificação. Pula pastas ocultas: o agy guarda nelas
/// cópias das imagens que recebe (.tempmediaStorage, .user_uploaded, .system_generated), que não são a gerada.
fn imagens_em(raiz: &Path) -> HashMap<PathBuf, std::time::SystemTime> {
    let mut v = HashMap::new();
    let mut pilha = vec![raiz.to_path_buf()];
    while let Some(dir) = pilha.pop() {
        let Ok(itens) = std::fs::read_dir(&dir) else { continue };
        for item in itens.flatten() {
            let caminho = item.path();
            let Ok(meta) = item.metadata() else { continue };
            if meta.is_dir() {
                if !item.file_name().to_string_lossy().starts_with('.') {
                    pilha.push(caminho);
                }
                continue;
            }
            let ext = caminho.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
            if ["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
                if let Ok(quando) = meta.modified() {
                    v.insert(caminho, quando);
                }
            }
        }
    }
    v
}

/// A imagem que apareceu (ou mudou) entre as duas listagens; havendo mais de uma, a mais nova.
fn imagem_nova(
    antes: &HashMap<PathBuf, std::time::SystemTime>,
    depois: HashMap<PathBuf, std::time::SystemTime>,
) -> Option<PathBuf> {
    depois.into_iter().filter(|(c, q)| antes.get(c) != Some(q)).max_by_key(|(_, q)| *q).map(|(c, _)| c)
}

/// Move o arquivo (rename; entre discos diferentes, copia e apaga).
async fn mover(de: &Path, para: &Path) -> std::io::Result<()> {
    if tokio::fs::rename(de, para).await.is_ok() {
        return Ok(());
    }
    tokio::fs::copy(de, para).await?;
    tokio::fs::remove_file(de).await
}

/// Gera uma imagem pela assinatura (`codex` ou `agy`). `refs`: (mime, bytes) das referências.
/// A CLI salva a imagem na pasta dela; assim que termina, a imagem nova é movida para `destino`
/// (pasta do sistema) e a pasta da CLI fica limpa. Devolve o caminho em `destino`.
/// Devolve também o modelo usado.
pub async fn gerar_imagem(
    nome: &str,
    prompt: &str,
    refs: &[(String, Vec<u8>)],
    destino: &Path,
    a: &AjusteCli,
) -> anyhow::Result<(PathBuf, Option<String>)> {
    let Some(raiz) = pasta_imagens(nome) else { bail!("a CLI {nome} não gera imagem") };
    let pasta = PastaTemp::nova().await?;
    let dir = pasta.caminho();
    let mut anexos = Vec::new();
    for (i, (mime, b)) in refs.iter().enumerate() {
        let ext = mime.rsplit('/').next().unwrap_or("png");
        let caminho = dir.join(format!("referencia-{}.{ext}", i + 1));
        tokio::fs::write(&caminho, b).await?;
        anexos.push(caminho);
    }
    let pedido = format!(
        "Gere UMA imagem com a sua ferramenta de geração de imagens. Não rode comandos e não crie nem edite arquivos.\n{}{prompt}",
        if anexos.is_empty() {
            String::new()
        } else {
            format!(
                "Imagens de referência (na ordem citada no pedido: Imagem 1, Imagem 2...): {}\n",
                anexos.iter().map(|a| a.display().to_string()).collect::<Vec<_>>().join(", ")
            )
        }
    );
    // O que já estava na pasta da CLI antes (a imagem é o que aparecer de novo).
    let antes = imagens_em(&raiz);
    let saida = match nome {
        "codex" => {
            let mut cmd = comando_isolado("codex", dir, &AMBIENTE);
            cmd.args(["exec", "--skip-git-repo-check", "--ephemeral", "--sandbox", "read-only", "-c", "approval_policy=\"never\""])
                .args(args_codex(a))
                .arg("-C")
                .arg(dir)
                .args(anexos.iter().map(|x| format!("--image={}", x.display())))
                .arg(&pedido)
                .stdin(Stdio::null());
            tokio::time::timeout(TEMPO_MAXIMO, cmd.output()).await.map_err(estourou)??
        }
        "agy" => {
            // Pode ver as referências e gerar a imagem; comandos e escrita de arquivos ficam negados.
            let negar = r#"{"permissions":{"deny":["command(*)","write_file(*)","edit_file(*)"]}}"#;
            let mut cmd = comando_isolado("agy", dir, &["HOME", "PATH", "LANG", "DBUS_SESSION_BUS_ADDRESS"]);
            cmd.args(["--print-timeout", "270s", "--settings", negar]).args(args_agy(a)).arg("--add-dir").arg(dir).arg("-p").arg(&pedido);
            if let Some(k) = crate::config::get("GEMINI_API_KEY") {
                cmd.env("GEMINI_API_KEY", k);
            }
            tokio::time::timeout(TEMPO_MAXIMO, cmd.output()).await.map_err(estourou)??
        }
        outro => bail!("a CLI {outro} não gera imagem"),
    };
    let Some(arquivo) = imagem_nova(&antes, imagens_em(&raiz)) else {
        bail!(
            "{nome} não gerou imagem: {}",
            resumo(ultima_linha(&String::from_utf8_lossy(&saida.stdout)).or(ultima_linha(&String::from_utf8_lossy(&saida.stderr))).unwrap_or("sem resposta"))
        );
    };
    tokio::fs::create_dir_all(destino).await?;
    let ext = arquivo.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    let movida = destino.join(format!("{nome}-{}.{ext}", uuid::Uuid::new_v4()));
    mover(&arquivo, &movida).await.with_context(|| format!("não consegui mover a imagem de {}", arquivo.display()))?;
    // agy: a conversa (com as cópias das referências) fica numa pasta própria em brain/; apaga ela.
    if nome == "agy" {
        if let Some(conversa) = arquivo.strip_prefix(&raiz).ok().and_then(|r| r.components().next()).map(|c| raiz.join(c)) {
            if conversa != raiz && conversa.is_dir() {
                let _ = tokio::fs::remove_dir_all(&conversa).await;
            }
        }
    }
    let modelo = if nome == "codex" { modelo_codex(&saida).or_else(|| a.modelo.clone()) } else { modelo_agy(a) };
    Ok((movida, modelo))
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn ajuste_por_cli_vira_argumentos() {
        let aj = serde_json::json!({ "codex": { "modelo": "gpt-6-sol", "esforco": "max" }, "claude": { "modelo": "-x; rm", "esforco": "turbo" } });
        let codex = AjusteCli::de(&aj, "codex");
        assert_eq!(args_codex(&codex), vec!["-m", "gpt-6-sol", "-c", "model_reasoning_effort=\"max\""]);
        // Nome perigoso e esforço desconhecido são ignorados (fica o padrão da CLI).
        assert_eq!(AjusteCli::de(&aj, "claude"), AjusteCli::default());
        assert_eq!(args_agy(&AjusteCli::de(&aj, "agy")), Vec::<String>::new());
        assert!(nome_seguro("gemini-3.8-flash-medium") && !nome_seguro("--help") && !nome_seguro("a b"));
    }

    #[test]
    fn modelo_vem_da_resposta() {
        assert_eq!(modelo_claude(r#"{"result":"x","modelUsage":{"claude-opus-5-5":{}}}"#).as_deref(), Some("claude-opus-5-5"));
        let saida = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: b"provider: openai\nmodel: gpt-6-luna\nsandbox: read-only".to_vec(),
        };
        assert_eq!(modelo_codex(&saida).as_deref(), Some("gpt-6-luna"));
    }

    #[test]
    fn imagem_gerada_e_a_nova_e_nao_a_copia_da_referencia() {
        let raiz = std::env::temp_dir().join(format!("teste-brain-{}", std::process::id()));
        let conversa = raiz.join("conversa");
        std::fs::create_dir_all(conversa.join(".tempmediaStorage")).unwrap();
        std::fs::write(raiz.join("velha.png"), b"v").unwrap();
        let antes = imagens_em(&raiz);
        std::fs::write(conversa.join("cena_pokemon_123.jpg"), b"x").unwrap();
        std::fs::write(conversa.join(".tempmediaStorage/media_1.png"), b"y").unwrap();
        assert_eq!(imagem_nova(&antes, imagens_em(&raiz)), Some(conversa.join("cena_pokemon_123.jpg")));
        // Nada novo: nenhuma imagem (nem a velha).
        let agora = imagens_em(&raiz);
        assert_eq!(imagem_nova(&agora, imagens_em(&raiz)), None);
        std::fs::remove_dir_all(&raiz).unwrap();
    }

    #[test]
    fn le_resposta_do_claude() {
        let r = ler_resposta_claude(r#"{"is_error":false,"structured_output":{"titulo":"Oi"}}"#, false).unwrap();
        assert_eq!(r["titulo"], "Oi");
        // Sem structured_output: lê o JSON de dentro do texto.
        let r = ler_resposta_claude(r#"{"result":"```json\n{\"titulo\":\"Dois\"}\n```"}"#, false).unwrap();
        assert_eq!(r["titulo"], "Dois");
        // stream-json: só a linha do resultado conta.
        let stream = "{\"type\":\"system\"}\n{\"type\":\"assistant\"}\n{\"type\":\"result\",\"structured_output\":{\"x\":1}}\n";
        assert_eq!(ler_resposta_claude(stream, true).unwrap()["x"], 1);
        // Erro do CLI vira erro.
        let e = ler_resposta_claude(r#"{"is_error":true,"result":"Invalid API key"}"#, false).unwrap_err();
        assert!(format!("{e}").contains("Invalid API key"));
        assert!(ler_resposta_claude("não é json", false).is_err());
    }

    #[test]
    fn le_resposta_do_agy() {
        assert_eq!(ler_resposta_agy(r#"{"structured_output":{"a":1}}"#).unwrap()["a"], 1);
        assert_eq!(ler_resposta_agy(r#"{"status":"SUCCESS","result":"{\"a\":2}"}"#).unwrap()["a"], 2);
        assert_eq!(ler_resposta_agy(r#"{"a":3}"#).unwrap()["a"], 3);
        assert!(ler_resposta_agy(r#"{"status":"ERROR","result":"x"}"#).is_err());
    }

    #[test]
    fn prompt_unico_junta_sistema_e_pedido() {
        let p = PedidoTexto { sistema: "SIS".into(), prompt: "PED".into(), schema: Value::Null, imagens: vec![] };
        let t = prompt_unico(&p);
        assert!(t.starts_with("SIS") && t.ends_with("PED") && t.contains("Não use ferramentas"));
    }

    #[tokio::test]
    async fn pasta_temporaria_some_ao_sair() {
        let caminho = {
            let p = PastaTemp::nova().await.unwrap();
            assert!(p.caminho().exists());
            p.caminho().to_path_buf()
        };
        assert!(!caminho.exists());
    }

    #[test]
    fn ultima_linha_ignora_vazias() {
        assert_eq!(ultima_linha("a\nerro final\n\n  \n"), Some("erro final"));
        assert_eq!(ultima_linha(""), None);
    }
}

//! Provedores de IA. Por enquanto: Claude via CLI (assinatura), só para famílias com cli_habilitado.

use std::{process::Stdio, time::Duration};

use anyhow::{anyhow, bail, Context};
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command};

pub struct PedidoTexto {
    pub sistema: String,
    pub prompt: String,
    /// JSON Schema da resposta (saída estruturada).
    pub schema: Value,
}

const TEMPO_MAXIMO: Duration = Duration::from_secs(300);

/// Roda `claude -p` sem nenhuma ferramenta, num diretório vazio e com o ambiente limpo
/// (o briefing vem das famílias: nada de shell, arquivos ou variáveis do servidor ao alcance da IA).
pub async fn claude_cli(p: &PedidoTexto) -> anyhow::Result<Value> {
    let dir = std::env::temp_dir().join(format!("st-ia-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let resultado = rodar_claude(p, &dir).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    resultado
}

async fn rodar_claude(p: &PedidoTexto, dir: &std::path::Path) -> anyhow::Result<Value> {
    let mut cmd = Command::new("claude");
    cmd.args(["-p", "--output-format", "json", "--tools", "", "--no-session-persistence", "--strict-mcp-config"])
        .arg("--system-prompt")
        .arg(&p.sistema)
        .arg("--json-schema")
        .arg(p.schema.to_string())
        .current_dir(dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for var in ["HOME", "PATH", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            cmd.env(var, v);
        }
    }
    if let Some(token) = crate::config::get("CLAUDE_CODE_OAUTH_TOKEN") {
        cmd.env("CLAUDE_CODE_OAUTH_TOKEN", token);
    }

    let mut filho = cmd.spawn().context("não consegui iniciar o claude (CLI instalado?)")?;
    if let Some(mut stdin) = filho.stdin.take() {
        stdin.write_all(p.prompt.as_bytes()).await?;
    }
    let saida = tokio::time::timeout(TEMPO_MAXIMO, filho.wait_with_output())
        .await
        .map_err(|_| anyhow!("a IA demorou mais de 5 minutos"))??;

    let stdout = String::from_utf8_lossy(&saida.stdout);
    let json: Value = serde_json::from_str(stdout.trim()).with_context(|| {
        let stderr = String::from_utf8_lossy(&saida.stderr);
        format!("resposta inesperada do claude: {} {}", resumo(&stdout), resumo(&stderr))
    })?;
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

fn resumo(s: &str) -> String {
    s.chars().take(400).collect()
}

pub fn extrair_json(texto: &str) -> anyhow::Result<Value> {
    let t = texto.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let ini = t.find('{').ok_or_else(|| anyhow!("sem objeto JSON"))?;
    let fim = t.rfind('}').ok_or_else(|| anyhow!("sem objeto JSON"))?;
    Ok(serde_json::from_str(&t[ini..=fim])?)
}

// ───────────── provedores por API (chave da família, do sistema ou gratuita) ─────────────

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .unwrap_or_default()
}

/// Anthropic Messages API com saída estruturada (output_config.format).
async fn anthropic_api(chave: &str, modelo: Option<&str>, p: &PedidoTexto) -> anyhow::Result<Value> {
    let modelo = modelo.map(String::from).unwrap_or_else(|| crate::config::get_ou("ANTHROPIC_MODELO", "claude-opus-5-5"));
    let corpo = serde_json::json!({
        "model": modelo,
        "max_tokens": 16000,
        "system": p.sistema,
        "messages": [{ "role": "user", "content": p.prompt }],
        "output_config": { "effort": "medium", "format": { "type": "json_schema", "schema": p.schema } },
        // Se o classificador de segurança recusar, o próprio servidor tenta outro modelo.
        "fallbacks": "default",
    });
    let r = http()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", chave)
        .header("anthropic-version", "2026-10-21")
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .json(&corpo)
        .send()
        .await?;
    let status = r.status();
    let json: Value = r.json().await?;
    if !status.is_success() {
        bail!("Anthropic {status}: {}", resumo(&json["error"]["message"].to_string()));
    }
    match json["stop_reason"].as_str() {
        Some("refusal") => bail!("a IA recusou o pedido ({})", json["stop_details"]["category"]),
        Some("max_tokens") => bail!("a resposta da IA foi cortada (max_tokens)"),
        _ => {}
    }
    let texto: String = json["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    extrair_json(&texto).context("a Anthropic não devolveu JSON válido")
}

/// Gemini API (generateContent) pedindo JSON.
async fn gemini_api(chave: &str, modelo: Option<&str>, p: &PedidoTexto) -> anyhow::Result<Value> {
    let modelo = modelo.map(String::from).unwrap_or_else(|| crate::config::get_ou("GEMINI_MODELO", "gemini-2.5-flash"));
    let corpo = serde_json::json!({
        "systemInstruction": { "parts": [{ "text": p.sistema }] },
        "contents": [{ "role": "user", "parts": [{ "text": format!(
            "{}\n\nResponda SOMENTE com um JSON que siga este JSON Schema:\n{}", p.prompt, p.schema) }] }],
        "generationConfig": { "responseMimeType": "application/json" },
    });
    let r = http()
        .post(format!("https://generativelanguage.googleapis.com/v1beta/models/{modelo}:generateContent"))
        .header("x-goog-api-key", chave)
        .json(&corpo)
        .send()
        .await?;
    let status = r.status();
    let json: Value = r.json().await?;
    if !status.is_success() {
        bail!("Gemini {status}: {}", resumo(&json["error"]["message"].to_string()));
    }
    let texto = json["candidates"][0]["content"]["parts"][0]["text"].as_str().unwrap_or_default();
    extrair_json(texto).context("o Gemini não devolveu JSON válido")
}

/// Chat Completions compatível com OpenAI (OpenAI e Kimi/Moonshot) pedindo JSON.
async fn openai_api(chave: &str, modelo: Option<&str>, p: &PedidoTexto) -> anyhow::Result<Value> {
    compat_api("https://api.openai.com/v1", "OpenAI", chave, &modelo.map(String::from).unwrap_or_else(|| crate::config::get_ou("OPENAI_MODELO", "gpt-4.1-mini")), p).await
}

pub fn base_kimi() -> String {
    crate::config::get_ou("KIMI_BASE_URL", "https://api.moonshot.ai/v1").trim_end_matches('/').to_string()
}

async fn kimi_api(chave: &str, modelo: Option<&str>, p: &PedidoTexto) -> anyhow::Result<Value> {
    let modelo = modelo.map(String::from).or_else(|| crate::config::get("KIMI_MODELO"))
        .ok_or_else(|| anyhow!("escolha o modelo do Kimi na seção Chaves"))?;
    compat_api(&base_kimi(), "Kimi", chave, &modelo, p).await
}

async fn compat_api(base: &str, nome: &str, chave: &str, modelo: &str, p: &PedidoTexto) -> anyhow::Result<Value> {
    let corpo = serde_json::json!({
        "model": modelo,
        "messages": [
            { "role": "system", "content": p.sistema },
            { "role": "user", "content": format!("{}\n\nResponda SOMENTE com um JSON que siga este JSON Schema:\n{}", p.prompt, p.schema) },
        ],
        "response_format": { "type": "json_object" },
    });
    let r = http()
        .post(format!("{base}/chat/completions"))
        .bearer_auth(chave)
        .json(&corpo)
        .send()
        .await?;
    let status = r.status();
    let json: Value = r.json().await?;
    if !status.is_success() {
        bail!("{nome} {status}: {}", resumo(&json["error"]["message"].to_string()));
    }
    let texto = json["choices"][0]["message"]["content"].as_str().unwrap_or_default();
    extrair_json(texto).with_context(|| format!("{nome} não devolveu JSON válido"))
}

pub async fn por_api(provedor: &str, chave: &str, modelo: Option<&str>, p: &PedidoTexto) -> anyhow::Result<Value> {
    match provedor {
        "anthropic" => anthropic_api(chave, modelo, p).await,
        "gemini" => gemini_api(chave, modelo, p).await,
        "openai" => openai_api(chave, modelo, p).await,
        "kimi" => kimi_api(chave, modelo, p).await,
        outro => bail!("provedor desconhecido: {outro}"),
    }
}

/// Chaves cifradas disponíveis (da família, ou do sistema quando familia = None), na ordem de preferência.
async fn chaves(st: &crate::AppState, familia: Option<uuid::Uuid>) -> anyhow::Result<Vec<(String, String, Option<String>)>> {
    let linhas: Vec<(String, Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT provedor, cifrada, modelo FROM ia.chaves WHERE familia_id IS NOT DISTINCT FROM $1
          ORDER BY CASE provedor WHEN 'anthropic' THEN 1 WHEN 'gemini' THEN 2 WHEN 'openai' THEN 3 ELSE 4 END",
    )
    .bind(familia)
    .fetch_all(&st.db)
    .await?;
    let mut v = Vec::new();
    for (p, c, m) in linhas {
        match crate::segredos::decifrar(&c) {
            Ok(k) => v.push((p, k, m)),
            Err(e) => tracing::warn!("chave {p} não decifrou: {e}"),
        }
    }
    Ok(v)
}

async fn registrar_uso(st: &crate::AppState, familia: uuid::Uuid, via: &str, provedor: &str) {
    let _ = sqlx::query("INSERT INTO ia.uso (familia_id, tarefa, via, provedor) VALUES ($1, 'texto', $2, $3)")
        .bind(familia)
        .bind(via)
        .bind(provedor)
        .execute(&st.db)
        .await;
}

/// Gera texto pela melhor rota disponível para a família:
/// CLI (assinaturas) → chave da família → chave do sistema → chave gratuita do Gemini (GEMINI_CHAVE_GRATIS).
/// O limite semanal vale para o que o admin paga (CLI e chave do sistema).
pub async fn gerar_texto(st: &crate::AppState, familia: uuid::Uuid, p: &PedidoTexto) -> anyhow::Result<Value> {
    let (cli, sistema, limites): (bool, bool, Value) = sqlx::query_as(
        "SELECT COALESCE(c.cli_habilitado, false), COALESCE(c.usar_chave_sistema, false), COALESCE(c.limites, '{}')
           FROM acesso.familias f LEFT JOIN ia.config_familia c ON c.familia_id = f.id WHERE f.id = $1",
    )
    .bind(familia)
    .fetch_one(&st.db)
    .await?;
    let usado: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ia.uso WHERE familia_id = $1 AND criado_em > now() - interval '7 days' AND via IN ('cli', 'chave_sistema')",
    )
    .bind(familia)
    .fetch_one(&st.db)
    .await?;
    let dentro_do_limite = limites["historias_semana"].as_i64().is_none_or(|l| usado < l);
    let mut erros: Vec<String> = Vec::new();

    if cli && dentro_do_limite {
        let r = match crate::config::get("CLI_TEXTO").as_deref() {
            Some("agy") => agy_cli(p).await,
            _ => claude_cli(p).await,
        };
        match r {
            Ok(v) => {
                registrar_uso(st, familia, "cli", "assinatura").await;
                return Ok(v);
            }
            Err(e) => erros.push(format!("CLI: {e:#}")),
        }
    }
    for (prov, chave, modelo) in chaves(st, Some(familia)).await? {
        match por_api(&prov, &chave, modelo.as_deref(), p).await {
            Ok(v) => {
                registrar_uso(st, familia, "chave_familia", &prov).await;
                return Ok(v);
            }
            Err(e) => erros.push(format!("chave da família ({prov}): {e:#}")),
        }
    }
    if sistema && dentro_do_limite {
        for (prov, chave, modelo) in chaves(st, None).await? {
            match por_api(&prov, &chave, modelo.as_deref(), p).await {
                Ok(v) => {
                    registrar_uso(st, familia, "chave_sistema", &prov).await;
                    return Ok(v);
                }
                Err(e) => erros.push(format!("chave do sistema ({prov}): {e:#}")),
            }
        }
    }
    if let Some(chave) = crate::config::get("GEMINI_CHAVE_GRATIS") {
        if !chave.trim().is_empty() {
            match gemini_api(chave.trim(), None, p).await {
                Ok(v) => {
                    registrar_uso(st, familia, "gratis", "gemini").await;
                    return Ok(v);
                }
                Err(e) => erros.push(format!("Gemini gratuito: {e:#}")),
            }
        }
    }
    if erros.is_empty() {
        if !dentro_do_limite {
            bail!("O limite de histórias desta semana foi atingido. Tente de novo em alguns dias ou crie a história manualmente.");
        }
        bail!("Esta família ainda não tem IA de texto configurada (sem CLI e sem chave de API). Peça ao admin, ou crie a história manualmente.");
    }
    bail!("Nenhuma IA conseguiu gerar: {}", erros.join(" | "))
}

/// Para o botão "Testar" do sistema.
pub async fn testar_agy(p: &PedidoTexto) -> anyhow::Result<Value> {
    agy_cli(p).await
}

/// Antigravity CLI (Gemini da assinatura Google One). Mesmo isolamento do claude: sem ferramentas, ambiente limpo.
async fn agy_cli(p: &PedidoTexto) -> anyhow::Result<Value> {
    let dir = std::env::temp_dir().join(format!("st-ia-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let prompt = format!("{}\nNão use ferramentas nem crie arquivos: responda só com o JSON.\n\n{}", p.sistema, p.prompt);
    // Sem ferramentas: nega todas as permissões (sem isso o agy fica parado pedindo aprovação) e modo plano.
    // O pedido vai grudado no -p (o agy usa o argumento seguinte ao -p como prompt).
    let negar = r#"{"permissions":{"allow":[],"deny":["*","command(*)","write_file(*)","edit_file(*)"]}}"#;
    let mut cmd = Command::new("agy");
    cmd.args(["--output-format", "json", "--mode", "plan", "--print-timeout", "240s", "--settings", negar])
        .arg("--json-schema")
        .arg(p.schema.to_string())
        .arg("-p")
        .arg(&prompt)
        .current_dir(&dir)
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for var in ["HOME", "PATH", "LANG", "DBUS_SESSION_BUS_ADDRESS"] {
        if let Ok(v) = std::env::var(var) {
            cmd.env(var, v);
        }
    }
    if let Some(k) = crate::config::get("GEMINI_API_KEY") {
        cmd.env("GEMINI_API_KEY", k);
    }
    let saida = tokio::time::timeout(TEMPO_MAXIMO, cmd.output())
        .await
        .map_err(|_| anyhow!("a IA demorou mais de 5 minutos"))??;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let stdout = String::from_utf8_lossy(&saida.stdout);
    if !saida.status.success() {
        bail!("agy falhou: {} {}", resumo(&stdout), resumo(&String::from_utf8_lossy(&saida.stderr)));
    }
    // A saída json do agy pode vir como envelope {result: "..."} ou direto como o objeto.
    let v: Value = serde_json::from_str(stdout.trim()).or_else(|_| extrair_json(&stdout))?;
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

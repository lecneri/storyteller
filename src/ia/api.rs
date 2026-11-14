//! Provedores por API (chave da família, do sistema ou gratuita): Anthropic, Gemini e compatíveis com OpenAI (OpenAI, Kimi).

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::Value;

use super::{b64, extrair_json, resumo, PedidoTexto};

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .unwrap_or_default()
}

/// Conteúdo da mensagem no formato da Anthropic (imagens em base64 + texto). Usado também pelo Claude CLI.
pub(super) fn conteudo_anthropic(p: &PedidoTexto) -> Value {
    let mut partes: Vec<Value> = p
        .imagens
        .iter()
        .map(|(mime, b)| serde_json::json!({ "type": "image", "source": { "type": "base64", "media_type": mime, "data": b64(b) } }))
        .collect();
    partes.push(serde_json::json!({ "type": "text", "text": p.prompt }));
    Value::Array(partes)
}

/// Anthropic Messages API com saída estruturada (output_config.format).
async fn anthropic_api(chave: &str, modelo: Option<&str>, p: &PedidoTexto) -> anyhow::Result<Value> {
    let modelo = modelo.map(String::from).unwrap_or_else(|| crate::config::get_ou("ANTHROPIC_MODELO", "claude-opus-5-5"));
    let corpo = serde_json::json!({
        "model": modelo,
        "max_tokens": 16000,
        "system": p.sistema,
        "messages": [{ "role": "user", "content": conteudo_anthropic(p) }],
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
pub(super) async fn gemini_api(chave: &str, modelo: Option<&str>, p: &PedidoTexto) -> anyhow::Result<Value> {
    let modelo = modelo.map(String::from).unwrap_or_else(|| crate::config::get_ou("GEMINI_MODELO", "gemini-2.5-flash"));
    let mut partes: Vec<Value> = p
        .imagens
        .iter()
        .map(|(mime, b)| serde_json::json!({ "inline_data": { "mime_type": mime, "data": b64(b) } }))
        .collect();
    partes.push(serde_json::json!({ "text": format!(
        "{}\n\nResponda SOMENTE com um JSON que siga este JSON Schema:\n{}", p.prompt, p.schema) }));
    let corpo = serde_json::json!({
        "systemInstruction": { "parts": [{ "text": p.sistema }] },
        "contents": [{ "role": "user", "parts": partes }],
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
    let texto = format!("{}\n\nResponda SOMENTE com um JSON que siga este JSON Schema:\n{}", p.prompt, p.schema);
    let usuario = if p.imagens.is_empty() {
        Value::String(texto)
    } else {
        let mut partes: Vec<Value> = p
            .imagens
            .iter()
            .map(|(mime, b)| serde_json::json!({ "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{}", b64(b)) } }))
            .collect();
        partes.push(serde_json::json!({ "type": "text", "text": texto }));
        Value::Array(partes)
    };
    let corpo = serde_json::json!({
        "model": modelo,
        "messages": [
            { "role": "system", "content": p.sistema },
            { "role": "user", "content": usuario },
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


//! Geração de imagem por API (chave da família ou do sistema): OpenAI (gpt-image) ou Gemini (imagem).
//! Sem chave disponível, o app usa o modo manual (kit de prompt + referências).

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{midia, segredos, AppState};

pub struct Referencia {
    pub bytes: Vec<u8>,
    pub mime: String,
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(240)).build().unwrap_or_default()
}

async fn openai(chave: &str, prompt: &str, refs: &[Referencia]) -> anyhow::Result<Vec<u8>> {
    let modelo = crate::config::get_ou("OPENAI_MODELO_IMAGEM", "gpt-image-1");
    let r = if refs.is_empty() {
        http()
            .post("https://api.openai.com/v1/images/generations")
            .bearer_auth(chave)
            .json(&json!({ "model": modelo, "prompt": prompt, "size": "1024x1536", "n": 1 }))
            .send()
            .await?
    } else {
        let mut form = reqwest::multipart::Form::new()
            .text("model", modelo)
            .text("prompt", prompt.to_string())
            .text("size", "1024x1536");
        for (i, r) in refs.iter().enumerate() {
            let ext = r.mime.split('/').nth(1).unwrap_or("png");
            let parte = reqwest::multipart::Part::bytes(r.bytes.clone())
                .file_name(format!("ref{}.{ext}", i + 1))
                .mime_str(&r.mime)?;
            form = form.part("image[]", parte);
        }
        http().post("https://api.openai.com/v1/images/edits").bearer_auth(chave).multipart(form).send().await?
    };
    let status = r.status();
    let j: Value = r.json().await?;
    if !status.is_success() {
        bail!("OpenAI {status}: {}", j["error"]["message"]);
    }
    let b64 = j["data"][0]["b64_json"].as_str().ok_or_else(|| anyhow!("OpenAI não devolveu imagem"))?;
    Ok(STANDARD.decode(b64)?)
}

async fn gemini(chave: &str, prompt: &str, refs: &[Referencia]) -> anyhow::Result<Vec<u8>> {
    let modelo = crate::config::get_ou("GEMINI_MODELO_IMAGEM", "gemini-2.5-flash-image");
    let mut partes = vec![json!({ "text": prompt })];
    for r in refs {
        partes.push(json!({ "inline_data": { "mime_type": r.mime, "data": STANDARD.encode(&r.bytes) } }));
    }
    let corpo = json!({
        "contents": [{ "role": "user", "parts": partes }],
        "generationConfig": { "responseModalities": ["IMAGE"], "imageConfig": { "aspectRatio": "3:4" } },
    });
    let r = http()
        .post(format!("https://generativelanguage.googleapis.com/v1beta/models/{modelo}:generateContent"))
        .header("x-goog-api-key", chave)
        .json(&corpo)
        .send()
        .await?;
    let status = r.status();
    let j: Value = r.json().await?;
    if !status.is_success() {
        bail!("Gemini {status}: {}", j["error"]["message"]);
    }
    let partes = j["candidates"][0]["content"]["parts"].as_array().cloned().unwrap_or_default();
    let dado = partes
        .iter()
        .find_map(|p| p["inlineData"]["data"].as_str().or_else(|| p["inline_data"]["data"].as_str()))
        .ok_or_else(|| anyhow!("Gemini não devolveu imagem"))?;
    Ok(STANDARD.decode(dado)?)
}

/// Chaves de imagem disponíveis para a família, na ordem: família → sistema (se liberado e dentro do limite).
async fn chaves_imagem(st: &AppState, familia: Uuid) -> anyhow::Result<Vec<(String, String, &'static str)>> {
    let (sistema, limites): (bool, Value) = sqlx::query_as(
        "SELECT COALESCE(c.usar_chave_sistema, false), COALESCE(c.limites, '{}')
           FROM acesso.familias f LEFT JOIN ia.config_familia c ON c.familia_id = f.id WHERE f.id = $1",
    )
    .bind(familia)
    .fetch_one(&st.db)
    .await?;
    let usado: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ia.uso WHERE familia_id = $1 AND tarefa = 'imagem' AND via = 'chave_sistema'
            AND criado_em > now() - interval '7 days'",
    )
    .bind(familia)
    .fetch_one(&st.db)
    .await?;
    let limite_ok = limites["imagens_semana"].as_i64().is_none_or(|l| usado < l);
    let linhas: Vec<(Option<Uuid>, String, Vec<u8>)> = sqlx::query_as(
        "SELECT familia_id, provedor, cifrada FROM ia.chaves
          WHERE provedor IN ('openai', 'gemini') AND (familia_id = $1 OR (familia_id IS NULL AND $2))
          ORDER BY familia_id NULLS LAST, CASE provedor WHEN 'openai' THEN 1 ELSE 2 END",
    )
    .bind(familia)
    .bind(sistema && limite_ok)
    .fetch_all(&st.db)
    .await?;
    Ok(linhas
        .into_iter()
        .filter_map(|(f, p, c)| {
            segredos::decifrar(&c).ok().map(|k| (p, k, if f.is_some() { "chave_familia" } else { "chave_sistema" }))
        })
        .collect())
}

pub async fn disponivel(st: &AppState, familia: Uuid) -> bool {
    chaves_imagem(st, familia).await.map(|v| !v.is_empty()).unwrap_or(false)
}

/// Gera uma imagem 3:4 e grava como mídia da família. Devolve (id, mime).
pub async fn gerar(st: &AppState, familia: Uuid, familia_slug: &str, pasta: &str, prompt: &str, refs: &[Referencia]) -> anyhow::Result<(Uuid, String)> {
    let chaves = chaves_imagem(st, familia).await?;
    if chaves.is_empty() {
        bail!("Sem IA de imagem para esta família: use o modo manual (copie o prompt e as referências).");
    }
    let mut erros = Vec::new();
    for (prov, chave, via) in chaves {
        let r = match prov.as_str() {
            "openai" => openai(&chave, prompt, refs).await,
            _ => gemini(&chave, prompt, refs).await,
        };
        match r {
            Ok(bytes) => {
                let (mime, ext) = midia::tipo_imagem(&bytes).context("a IA devolveu um arquivo que não é imagem")?;
                let id = midia::salvar(st, familia, &format!("familias/{familia_slug}/{pasta}"), &bytes, mime, ext).await?;
                let _ = sqlx::query("INSERT INTO ia.uso (familia_id, tarefa, via, provedor) VALUES ($1, 'imagem', $2, $3)")
                    .bind(familia)
                    .bind(via)
                    .bind(&prov)
                    .execute(&st.db)
                    .await;
                return Ok((id, mime.to_string()));
            }
            Err(e) => erros.push(format!("{prov}: {e:#}")),
        }
    }
    bail!("Nenhuma IA de imagem conseguiu gerar: {}", erros.join(" | "))
}

/// Lê arquivos de mídia da família para usar como referência.
pub async fn carregar_refs(st: &AppState, familia: Uuid, ids: &[Uuid]) -> Vec<Referencia> {
    let mut v = Vec::new();
    for id in ids {
        let linha: Option<(String, String)> =
            sqlx::query_as("SELECT caminho, tipo_mime FROM midia.arquivos WHERE id = $1 AND familia_id = $2")
                .bind(id)
                .bind(familia)
                .fetch_optional(&st.db)
                .await
                .unwrap_or(None);
        if let Some((caminho, mime)) = linha.filter(|(_, m)| m.starts_with("image/") && m != "image/gif") {
            if let Ok(bytes) = tokio::fs::read(st.data_dir.join(caminho)).await {
                v.push(Referencia { bytes, mime });
            }
        }
    }
    v
}

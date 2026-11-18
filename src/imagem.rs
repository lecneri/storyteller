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

async fn openai(chave: &str, modelo: Option<&str>, prompt: &str, refs: &[Referencia]) -> anyhow::Result<Vec<u8>> {
    let modelo = modelo.map(String::from).unwrap_or_else(|| crate::config::get_ou("OPENAI_MODELO_IMAGEM", "gpt-image-1"));
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

async fn gemini(chave: &str, modelo: Option<&str>, prompt: &str, refs: &[Referencia]) -> anyhow::Result<Vec<u8>> {
    let padrao = crate::config::get_ou("GEMINI_MODELO_IMAGEM", "gemini-2.5-flash-image");
    let modelo = modelo.map(String::from).unwrap_or_else(|| padrao.clone());
    let mut partes = vec![json!({ "text": prompt })];
    for r in refs {
        partes.push(json!({ "inline_data": { "mime_type": r.mime, "data": STANDARD.encode(&r.bytes) } }));
    }
    let corpo = json!({
        "contents": [{ "role": "user", "parts": partes }],
        "generationConfig": { "responseModalities": ["IMAGE"], "imageConfig": { "aspectRatio": "3:4" } },
    });
    let j = crate::ia::gemini::gerar(chave, &modelo, &padrao, &corpo, Duration::from_secs(240)).await?;
    let partes = j["candidates"][0]["content"]["parts"].as_array().cloned().unwrap_or_default();
    let dado = partes
        .iter()
        .find_map(|p| p["inlineData"]["data"].as_str().or_else(|| p["inline_data"]["data"].as_str()))
        .ok_or_else(|| anyhow!("Gemini não devolveu imagem"))?;
    Ok(STANDARD.decode(dado)?)
}

/// Um jeito de gerar imagem: pela assinatura (CLI) ou por uma chave de API.
enum Fonte {
    Cli(String),
    Chave { provedor: String, chave: String, via: &'static str, modelo: Option<String> },
}

/// Fontes de imagem da família, na ordem dela (assinaturas, chave da família, chave do sistema se liberada e no limite).
async fn fontes_imagem(st: &AppState, familia: Uuid) -> anyhow::Result<Vec<Fonte>> {
    let (sistema, limites, clis, ordem): (bool, Value, Vec<String>, Vec<String>) = sqlx::query_as(
        "SELECT COALESCE(c.usar_chave_sistema, false), COALESCE(c.limites, '{}'), COALESCE(c.clis_imagem, '{}'),
                COALESCE(c.ordem_fontes, '{cli,familia,sistema}')
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
    let linhas: Vec<(Option<Uuid>, String, Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT familia_id, provedor, cifrada, modelo_imagem FROM ia.chaves
          WHERE provedor IN ('openai', 'gemini') AND (familia_id = $1 OR (familia_id IS NULL AND $2))
          ORDER BY familia_id NULLS LAST, CASE provedor WHEN 'openai' THEN 1 ELSE 2 END",
    )
    .bind(familia)
    .bind(sistema && limite_ok)
    .fetch_all(&st.db)
    .await?;
    let mut chaves: Vec<Fonte> = linhas
        .into_iter()
        .filter_map(|(f, p, c, m)| {
            segredos::decifrar(&c).ok().map(|k| Fonte::Chave {
                provedor: p,
                chave: k,
                via: if f.is_some() { "chave_familia" } else { "chave_sistema" },
                modelo: m,
            })
        })
        .collect();
    let mut v = Vec::new();
    for fonte in crate::ia::ordem_fontes(&ordem) {
        match fonte {
            "cli" => v.extend(clis.iter().filter(|c| crate::ia::CLIS_IMAGEM.contains(&c.as_str())).map(|c| Fonte::Cli(c.clone()))),
            via => {
                let alvo = if via == "familia" { "chave_familia" } else { "chave_sistema" };
                let (essas, resto): (Vec<Fonte>, Vec<Fonte>) =
                    chaves.into_iter().partition(|f| matches!(f, Fonte::Chave { via, .. } if *via == alvo));
                v.extend(essas);
                chaves = resto;
            }
        }
    }
    Ok(v)
}

pub async fn disponivel(st: &AppState, familia: Uuid) -> bool {
    fontes_imagem(st, familia).await.map(|v| !v.is_empty()).unwrap_or(false)
}

/// Fila de cada fonte: uma por CLI ("codex", "agy") e uma para as chaves de API ("api").
fn fila_da(f: &Fonte) -> &str {
    match f {
        Fonte::Cli(nome) => nome,
        Fonte::Chave { .. } => "api",
    }
}

/// Todas as filas que existem (um trabalhador para cada).
pub fn todas_filas() -> Vec<&'static str> {
    crate::ia::CLIS_IMAGEM.iter().copied().chain(["api"]).collect()
}

/// Nome para mostrar na tela.
pub fn nome_fila(fila: &str) -> &'static str {
    match fila {
        "codex" => "ChatGPT (Codex)",
        "agy" => "Gemini (agy)",
        "api" => "Chave de API",
        _ => "?",
    }
}

/// Filas da família, na ordem dela (sem repetir).
pub async fn filas(st: &AppState, familia: Uuid) -> anyhow::Result<Vec<String>> {
    let mut v: Vec<String> = Vec::new();
    for f in fontes_imagem(st, familia).await? {
        let q = fila_da(&f);
        if !v.iter().any(|x| x == q) {
            v.push(q.to_string());
        }
    }
    Ok(v)
}

/// Gera uma imagem 3:4 pelas fontes da família que são desta fila e grava como mídia. Devolve (id, mime).
pub async fn gerar_na_fila(
    st: &AppState,
    familia: Uuid,
    familia_slug: &str,
    pasta: &str,
    prompt: &str,
    refs: &[Referencia],
    fila: &str,
) -> anyhow::Result<(Uuid, String)> {
    let fontes: Vec<Fonte> = fontes_imagem(st, familia).await?.into_iter().filter(|f| fila_da(f) == fila).collect();
    if fontes.is_empty() {
        bail!("{} não está liberada para esta família.", nome_fila(fila));
    }
    let mut erros = Vec::new();
    for fonte in fontes {
        let (r, via, prov) = match &fonte {
            Fonte::Cli(nome) => {
                let anexos: Vec<(String, Vec<u8>)> = refs.iter().map(|r| (r.mime.clone(), r.bytes.clone())).collect();
                // A imagem sai da pasta da CLI direto para a pasta do sistema; vira mídia e o arquivo de passagem some.
                let r = match crate::ia::gerar_imagem_cli(nome, prompt, &anexos, &st.data_dir.join("tmp/cli-imagens")).await {
                    Ok(arquivo) => {
                        let b = tokio::fs::read(&arquivo).await.map_err(anyhow::Error::from);
                        let _ = tokio::fs::remove_file(&arquivo).await;
                        b
                    }
                    Err(e) => Err(e),
                };
                (r, "cli", nome.clone())
            }
            Fonte::Chave { provedor, chave, via, modelo } => {
                let r = match provedor.as_str() {
                    "openai" => openai(chave, modelo.as_deref(), prompt, refs).await,
                    _ => gemini(chave, modelo.as_deref(), prompt, refs).await,
                };
                (r, *via, provedor.clone())
            }
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
            Err(e) => {
                tracing::warn!("imagem por {prov} falhou: {e:#}");
                // A CLI já diz o nome dela no erro.
                erros.push(if via == "cli" { format!("{e:#}") } else { format!("{prov}: {e:#}") });
            }
        }
    }
    bail!("{}", erros.join(" | "))
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

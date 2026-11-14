//! Seção "Chaves": cada chave de API (da família ou do sistema) escolhe o seu modelo,
//! com a lista carregada do próprio provedor, e pode ser testada.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{auth::Sessao, segredos, AppState};

#[derive(Serialize)]
pub struct LinhaChave {
    pub provedor: String,
    pub nome: String,
    pub final_: String,
    pub modelo: String,
    pub modelo_imagem: String,
    pub faz_imagem: bool,
}

/// Linhas para a tela (familia = None → chaves do sistema).
pub async fn linhas(st: &AppState, familia: Option<Uuid>) -> Vec<LinhaChave> {
    let v: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT provedor, final, modelo, modelo_imagem FROM ia.chaves WHERE familia_id IS NOT DISTINCT FROM $1 ORDER BY provedor",
    )
    .bind(familia)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    v.into_iter()
        .map(|(p, f, m, mi)| LinhaChave {
            nome: crate::familia::PROVEDORES.iter().find(|(k, _)| *k == p).map(|(_, n)| n.to_string()).unwrap_or_else(|| p.clone()),
            faz_imagem: p == "openai" || p == "gemini",
            provedor: p,
            final_: f,
            modelo: m.unwrap_or_default(),
            modelo_imagem: mi.unwrap_or_default(),
        })
        .collect()
}

/// Quem pode mexer: "sistema" → admin do sistema; "familia" → admin da família.
fn escopo(s: &Sessao, escopo: &str) -> Option<Option<Uuid>> {
    match escopo {
        "sistema" if s.admin_sistema => Some(None),
        "familia" if s.admin_familia() => Some(Some(s.familia_id)),
        _ => None,
    }
}

async fn chave(st: &AppState, familia: Option<Uuid>, provedor: &str) -> Option<(String, Option<String>)> {
    let (c, m): (Vec<u8>, Option<String>) = sqlx::query_as(
        "SELECT cifrada, modelo FROM ia.chaves WHERE familia_id IS NOT DISTINCT FROM $1 AND provedor = $2",
    )
    .bind(familia)
    .bind(provedor)
    .fetch_optional(&st.db)
    .await
    .ok()??;
    segredos::decifrar(&c).ok().map(|k| (k, m))
}

fn erro(msg: impl Into<String>) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "erro": msg.into() }))).into_response()
}

/// POST /chaves/{escopo}/{provedor}/modelos: lista os modelos que a chave enxerga.
pub async fn modelos(State(st): State<AppState>, s: Sessao, Path((esc, provedor)): Path<(String, String)>) -> Response {
    let Some(familia) = escopo(&s, &esc) else { return StatusCode::FORBIDDEN.into_response() };
    let Some((k, _)) = chave(&st, familia, &provedor).await else { return erro("Chave não encontrada.") };
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().unwrap_or_default();
    let r = match provedor.as_str() {
        "anthropic" => http.get("https://api.anthropic.com/v1/models?limit=100").header("x-api-key", &k).header("anthropic-version", "2026-10-21").send().await,
        "openai" => http.get("https://api.openai.com/v1/models").bearer_auth(&k).send().await,
        "kimi" => http.get(format!("{}/models", crate::ia::base_kimi())).bearer_auth(&k).send().await,
        "gemini" => http.get("https://generativelanguage.googleapis.com/v1beta/models?pageSize=200").header("x-goog-api-key", &k).send().await,
        _ => return erro("Provedor desconhecido."),
    };
    let r = match r {
        Ok(r) => r,
        Err(e) => return erro(format!("Não consegui falar com o provedor: {e}")),
    };
    let status = r.status();
    let j: Value = r.json().await.unwrap_or_default();
    if !status.is_success() {
        return erro(format!("{status}: {}", j["error"]["message"].as_str().unwrap_or("a chave foi recusada")));
    }
    // Separa modelos de texto e de imagem (pelos nomes/métodos de cada provedor).
    let mut texto: Vec<String> = Vec::new();
    let mut imagem: Vec<String> = Vec::new();
    if provedor == "gemini" {
        for m in j["models"].as_array().into_iter().flatten() {
            let nome = m["name"].as_str().unwrap_or_default().trim_start_matches("models/").to_string();
            let gera = m["supportedGenerationMethods"].as_array().is_some_and(|a| a.iter().any(|x| x == "generateContent"));
            if !gera || nome.contains("embedding") || nome.contains("tts") || nome.contains("aqa") {
                continue;
            }
            if nome.contains("image") { imagem.push(nome) } else { texto.push(nome) }
        }
    } else {
        for m in j["data"].as_array().into_iter().flatten() {
            let id = m["id"].as_str().unwrap_or_default().to_string();
            if provedor == "openai" {
                if id.starts_with("gpt-image") || id.starts_with("dall-e") {
                    imagem.push(id);
                } else if (id.starts_with("gpt-") || id.starts_with('o')) && !id.contains("audio") && !id.contains("realtime")
                    && !id.contains("transcribe") && !id.contains("tts") && !id.contains("embedding") && !id.contains("search")
                {
                    texto.push(id);
                }
            } else {
                texto.push(id);
            }
        }
    }
    texto.sort();
    texto.dedup();
    imagem.sort();
    Json(json!({ "texto": texto, "imagem": imagem })).into_response()
}

#[derive(Deserialize)]
pub struct PedidoModelo {
    #[serde(default)]
    modelo: String,
    #[serde(default)]
    modelo_imagem: String,
}

/// POST /chaves/{escopo}/{provedor}/modelo
pub async fn salvar_modelo(
    State(st): State<AppState>,
    s: Sessao,
    Path((esc, provedor)): Path<(String, String)>,
    Json(p): Json<PedidoModelo>,
) -> Response {
    let Some(familia) = escopo(&s, &esc) else { return StatusCode::FORBIDDEN.into_response() };
    let vazio = |v: &str| Some(v.trim().to_string()).filter(|x| !x.is_empty());
    match sqlx::query(
        "UPDATE ia.chaves SET modelo = $3, modelo_imagem = $4 WHERE familia_id IS NOT DISTINCT FROM $1 AND provedor = $2",
    )
    .bind(familia)
    .bind(&provedor)
    .bind(vazio(&p.modelo))
    .bind(vazio(&p.modelo_imagem))
    .execute(&st.db)
    .await
    {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro(e.to_string()),
    }
}

/// POST /chaves/{escopo}/{provedor}/testar: um pedido curtinho com o modelo escolhido.
pub async fn testar(State(st): State<AppState>, s: Sessao, Path((esc, provedor)): Path<(String, String)>) -> Response {
    let Some(familia) = escopo(&s, &esc) else { return StatusCode::FORBIDDEN.into_response() };
    let Some((k, modelo)) = chave(&st, familia, &provedor).await else { return erro("Chave não encontrada.") };
    let pedido = crate::ia::PedidoTexto {
        sistema: "Responda só com o JSON pedido.".into(),
        imagens: Vec::new(),
        prompt: "Diga oi em português.".into(),
        schema: json!({ "type": "object", "additionalProperties": false, "required": ["resposta"],
                        "properties": { "resposta": { "type": "string" } } }),
    };
    match crate::ia::por_api(&provedor, &k, modelo.as_deref(), &pedido).await {
        Ok(v) => Json(json!({ "ok": true, "resposta": v["resposta"] })).into_response(),
        Err(e) => Json(json!({ "ok": false, "erro": format!("{e:#}") })).into_response(),
    }
}

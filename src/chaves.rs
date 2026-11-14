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
use crate::web::erro_json as erro;

#[derive(Serialize)]
pub struct LinhaChave {
    pub provedor: String,
    pub nome: String,
    pub final_: String,
    pub modelo: String,
    pub modelo_imagem: String,
    pub faz_imagem: bool,
    /// Chave de voz (ElevenLabs): o modelo é de voz e o teste mostra os créditos.
    pub e_voz: bool,
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
            e_voz: p == "elevenlabs",
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


/// Separa a lista de modelos do provedor em (texto, imagem), tirando os que não servem
/// (embeddings, áudio, tts, busca...). Cada provedor devolve a lista num formato.
fn separar_modelos(provedor: &str, j: &Value) -> (Vec<String>, Vec<String>) {
    let mut texto: Vec<String> = Vec::new();
    let mut imagem: Vec<String> = Vec::new();
    if provedor == "gemini" {
        for m in j["models"].as_array().into_iter().flatten() {
            let nome = m["name"].as_str().unwrap_or_default().trim_start_matches("models/").to_string();
            let gera = m["supportedGenerationMethods"].as_array().is_some_and(|a| a.iter().any(|x| x == "generateContent"));
            if !gera || ["embedding", "tts", "aqa"].iter().any(|x| nome.contains(x)) {
                continue;
            }
            if nome.contains("image") { imagem.push(nome) } else { texto.push(nome) }
        }
    } else {
        const FORA: [&str; 6] = ["audio", "realtime", "transcribe", "tts", "embedding", "search"];
        for m in j["data"].as_array().into_iter().flatten() {
            let id = m["id"].as_str().unwrap_or_default().to_string();
            match provedor {
                "openai" if id.starts_with("gpt-image") || id.starts_with("dall-e") => imagem.push(id),
                "openai" if (id.starts_with("gpt-") || id.starts_with('o')) && !FORA.iter().any(|x| id.contains(x)) => texto.push(id),
                "openai" => {}
                _ => texto.push(id),
            }
        }
    }
    texto.sort();
    texto.dedup();
    imagem.sort();
    (texto, imagem)
}

/// POST /chaves/{escopo}/{provedor}/modelos: lista os modelos que a chave enxerga.
pub async fn modelos(State(st): State<AppState>, s: Sessao, Path((esc, provedor)): Path<(String, String)>) -> Response {
    let Some(familia) = escopo(&s, &esc) else { return StatusCode::FORBIDDEN.into_response() };
    let Some((k, _)) = chave(&st, familia, &provedor).await else { return erro("Chave não encontrada.") };
    if provedor == "elevenlabs" {
        return match crate::elevenlabs::modelos(&k).await {
            Ok(v) => Json(json!({ "texto": v, "imagem": [] })).into_response(),
            Err(e) => erro(format!("{e:#}")),
        };
    }
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
    let (texto, imagem) = separar_modelos(&provedor, &j);
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
    if provedor == "elevenlabs" {
        return match crate::elevenlabs::creditos(&k).await {
            Ok(t) => Json(json!({ "ok": true, "resposta": t })).into_response(),
            Err(e) => Json(json!({ "ok": false, "erro": format!("{e:#}") })).into_response(),
        };
    }
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

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn separa_modelos_da_openai() {
        let j = json!({ "data": [
            { "id": "gpt-5" }, { "id": "gpt-image-1" }, { "id": "dall-e-3" }, { "id": "o4-mini" },
            { "id": "gpt-4o-audio-preview" }, { "id": "text-embedding-3" }, { "id": "whisper-1" }, { "id": "gpt-5" }
        ]});
        let (t, i) = separar_modelos("openai", &j);
        assert_eq!(t, vec!["gpt-5", "o4-mini"]);
        assert_eq!(i, vec!["dall-e-3", "gpt-image-1"]);
    }

    #[test]
    fn separa_modelos_do_gemini() {
        let j = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-2.5-flash-image", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/gemini-2.5-flash-preview-tts", "supportedGenerationMethods": ["generateContent"] }
        ]});
        let (t, i) = separar_modelos("gemini", &j);
        assert_eq!(t, vec!["gemini-2.5-flash"]);
        assert_eq!(i, vec!["gemini-2.5-flash-image"]);
    }

    #[test]
    fn outros_provedores_vao_todos_para_texto() {
        let (t, i) = separar_modelos("kimi", &json!({ "data": [{ "id": "kimi-k2" }, { "id": "moonshot-v1" }] }));
        assert_eq!((t.len(), i.len()), (2, 0));
    }
}

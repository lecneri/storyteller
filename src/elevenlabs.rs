//! ElevenLabs: vozes da conta da família, síntese (com o tempo de cada palavra) e créditos.
//! Cada família usa a própria chave (provedor "elevenlabs" em ia.chaves).

use anyhow::{bail, Context};
use base64::Engine;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::AppState;

const BASE: &str = "https://api.elevenlabs.io/v1";
pub const MODELO_PADRAO: &str = "eleven_multilingual_v2";

fn http(segundos: u64) -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(segundos)).build().unwrap_or_default()
}

/// Chave e modelo escolhido da família (None = a família não cadastrou chave).
pub async fn chave_da_familia(st: &AppState, familia: Uuid) -> Option<(String, String)> {
    let (c, m): (Vec<u8>, Option<String>) =
        sqlx::query_as("SELECT cifrada, modelo FROM ia.chaves WHERE familia_id = $1 AND provedor = 'elevenlabs'")
            .bind(familia)
            .fetch_optional(&st.db)
            .await
            .ok()??;
    let k = crate::segredos::decifrar(&c).ok()?;
    Some((k, m.filter(|m| !m.is_empty()).unwrap_or_else(|| MODELO_PADRAO.into())))
}

/// Id de voz da ElevenLabs: só letras e números.
pub fn id_valido(id: &str) -> bool {
    (8..=40).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Mensagem de erro legível a partir da resposta da API.
fn motivo(status: reqwest::StatusCode, j: &Value) -> String {
    let d = &j["detail"];
    let msg = d["message"].as_str().or_else(|| d.as_str()).unwrap_or("a chave foi recusada");
    if d["status"] == "quota_exceeded" {
        return "acabaram os créditos do mês na ElevenLabs".into();
    }
    format!("{status}: {msg}")
}

async fn get(chave: &str, caminho: &str) -> anyhow::Result<Value> {
    let r = http(20).get(format!("{BASE}{caminho}")).header("xi-api-key", chave).send().await.context("não consegui falar com a ElevenLabs")?;
    let status = r.status();
    let j: Value = r.json().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", motivo(status, &j));
    }
    Ok(j)
}

/// Uma voz da conta: (id, nome para a tela, voz grátis de reserva).
pub struct VozConta {
    pub id: String,
    pub nome: String,
    pub reserva: String,
}

/// Voz grátis (edge-tts) usada quando a ElevenLabs falha, pelo gênero da voz.
fn reserva_por_genero(genero: &str) -> &'static str {
    if genero.eq_ignore_ascii_case("female") { "pt-BR-ThalitaMultilingualNeural" } else { "pt-BR-AntonioNeural" }
}

pub fn ler_vozes(j: &Value) -> Vec<VozConta> {
    let mut v: Vec<VozConta> = j["voices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let id = x["voice_id"].as_str().filter(|i| id_valido(i))?.to_string();
            let nome = x["name"].as_str().unwrap_or("?").trim().to_string();
            let l = &x["labels"];
            let detalhes: Vec<&str> = ["gender", "age", "accent", "description"]
                .iter()
                .filter_map(|k| l[*k].as_str())
                .filter(|s| !s.is_empty())
                .collect();
            let nome = if detalhes.is_empty() { nome } else { format!("{nome} · {}", detalhes.join(", ")) };
            Some(VozConta { id, nome, reserva: reserva_por_genero(l["gender"].as_str().unwrap_or("")).into() })
        })
        .collect();
    v.sort_by(|a, b| a.nome.to_lowercase().cmp(&b.nome.to_lowercase()));
    v
}

/// Vozes da conta (as próprias, clonadas e as adicionadas da biblioteca).
pub async fn vozes(chave: &str) -> anyhow::Result<Vec<VozConta>> {
    Ok(ler_vozes(&get(chave, "/voices").await?))
}

/// Modelos que falam (para a escolha de modelo na chave).
pub async fn modelos(chave: &str) -> anyhow::Result<Vec<String>> {
    let j = get(chave, "/models").await?;
    let mut v: Vec<String> = j
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["can_do_text_to_speech"].as_bool().unwrap_or(true))
        .filter_map(|m| m["model_id"].as_str().map(String::from))
        .collect();
    v.sort();
    Ok(v)
}

/// Créditos do mês em texto ("restam 7.500 de 10.000 caracteres").
pub async fn creditos(chave: &str) -> anyhow::Result<String> {
    let j = get(chave, "/user/subscription").await?;
    let usados = j["character_count"].as_i64().unwrap_or(0);
    let limite = j["character_limit"].as_i64().unwrap_or(0);
    let mut t = format!("restam {} de {} caracteres", milhar(limite - usados), milhar(limite));
    if let Some(ts) = j["next_character_count_reset_unix"].as_i64().and_then(|s| chrono::DateTime::from_timestamp(s, 0)) {
        t.push_str(&format!(" (renova em {})", ts.format("%d/%m")));
    }
    Ok(t)
}

fn milhar(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Velocidade da tela (-40 a +40 %) → "speed" da ElevenLabs (0,7 a 1,2).
pub fn velocidade(v: i32) -> f64 {
    (1.0 + v as f64 / 100.0).clamp(0.7, 1.2)
}

/// Gera o MP3. Devolve (bytes, palavras no formato do player: [início, duração, texto]).
pub async fn sintetizar(chave: &str, modelo: &str, voz: &str, texto: &str, vel: i32) -> anyhow::Result<(Vec<u8>, Value)> {
    if !id_valido(voz) {
        bail!("voz da ElevenLabs inválida");
    }
    let corpo = json!({
        "text": texto,
        "model_id": modelo,
        "voice_settings": { "stability": 0.5, "similarity_boost": 0.75, "speed": velocidade(vel) },
    });
    let r = http(120)
        .post(format!("{BASE}/text-to-speech/{voz}/with-timestamps?output_format=mp3_44100_128"))
        .header("xi-api-key", chave)
        .json(&corpo)
        .send()
        .await
        .context("não consegui falar com a ElevenLabs")?;
    let status = r.status();
    let j: Value = r.json().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", motivo(status, &j));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(j["audio_base64"].as_str().unwrap_or_default())
        .context("áudio da ElevenLabs veio quebrado")?;
    if bytes.is_empty() {
        bail!("a ElevenLabs não devolveu áudio");
    }
    let a = if j["alignment"].is_object() { &j["alignment"] } else { &j["normalized_alignment"] };
    Ok((bytes, palavras_do_alinhamento(a)))
}

/// Junta o tempo de cada letra em palavras. Como no player, só conta pedaços com letra ou número.
pub fn palavras_do_alinhamento(a: &Value) -> Value {
    let letras = a["characters"].as_array().cloned().unwrap_or_default();
    let ini = a["character_start_times_seconds"].as_array().cloned().unwrap_or_default();
    let fim = a["character_end_times_seconds"].as_array().cloned().unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();
    let mut atual: Option<(f64, f64, String)> = None;
    let fechar = |p: Option<(f64, f64, String)>, out: &mut Vec<Value>| {
        if let Some((i, f, t)) = p {
            if t.chars().any(char::is_alphanumeric) {
                out.push(json!([round3(i), round3((f - i).max(0.0)), t]));
            }
        }
    };
    for (k, c) in letras.iter().enumerate() {
        let c = c.as_str().unwrap_or("");
        let (i, f) = (ini.get(k).and_then(Value::as_f64).unwrap_or(0.0), fim.get(k).and_then(Value::as_f64).unwrap_or(0.0));
        if c.chars().all(char::is_whitespace) {
            fechar(atual.take(), &mut out);
            continue;
        }
        match atual.as_mut() {
            Some(p) => {
                p.1 = f;
                p.2.push_str(c);
            }
            None => atual = Some((i, f, c.to_string())),
        }
    }
    fechar(atual.take(), &mut out);
    Value::Array(out)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn junta_letras_em_palavras() {
        let a = json!({
            "characters": ["O", "i", ",", " ", "P", "e", "d", "r", "o", "!", " ", "—", " ", "1"],
            "character_start_times_seconds": [0.0, 0.1, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.8],
            "character_end_times_seconds":   [0.1, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.8, 0.9]
        });
        // "—" sozinho não conta (o player também ignora).
        assert_eq!(palavras_do_alinhamento(&a), json!([[0.0, 0.25, "Oi,"], [0.3, 0.3, "Pedro!"], [0.8, 0.1, "1"]]));
        assert_eq!(palavras_do_alinhamento(&json!({})), json!([]));
    }

    #[test]
    fn le_vozes_da_conta() {
        let j = json!({ "voices": [
            { "voice_id": "21m00Tcm4TlvDq8ikWAM", "name": "Rachel", "labels": { "gender": "female", "accent": "american" } },
            { "voice_id": "abc", "name": "Inválida" },
            { "voice_id": "pNInz6obpgDQGcFmaJgB", "name": "adam", "labels": {} }
        ]});
        let v = ler_vozes(&j);
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].nome.as_str(), v[0].reserva.as_str()), ("adam", "pt-BR-AntonioNeural"));
        assert_eq!((v[1].nome.as_str(), v[1].reserva.as_str()), ("Rachel · female, american", "pt-BR-ThalitaMultilingualNeural"));
    }

    #[test]
    fn velocidade_e_numeros() {
        assert_eq!(velocidade(0), 1.0);
        assert_eq!(velocidade(-40), 0.7);
        assert_eq!(velocidade(40), 1.2);
        assert_eq!(milhar(10000), "10.000");
        assert_eq!(milhar(-1234), "-1.234");
        assert_eq!(milhar(999), "999");
        assert!(id_valido("21m00Tcm4TlvDq8ikWAM"));
        assert!(!id_valido("../../x"));
    }
}

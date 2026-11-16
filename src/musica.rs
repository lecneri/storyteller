//! Música de fundo da história (opcional): a família envia um arquivo de áudio, que vira mp3 sem o silêncio
//! das pontas (para o loop não ter buraco) e com volume padronizado. O leitor e o vídeo tocam em loop,
//! abaixando quando a narração fala.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::Sessao,
    web::{erro_interno, erro_json, historia_da_familia},
    AppState,
};

/// Só os primeiros 5 minutos entram (o resto é cortado): a música fica em loop mesmo.
const MAX_S: u32 = 300;

/// Volume "cheio" (sem narração) dentro do intervalo aceito.
pub fn volume_valido(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.05, 1.0) } else { 0.35 }
}

/// POST /historias/{id}/musica — multipart "arquivo" (mp3, ogg, wav, m4a...). Troca a música da história.
pub async fn enviar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, mut multipart: axum::extract::Multipart) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let mut bytes = None;
    while let Ok(Some(campo)) = multipart.next_field().await {
        if campo.name() == Some("arquivo") {
            match campo.bytes().await {
                Ok(b) => bytes = Some(b),
                Err(_) => return erro_json("O arquivo não chegou inteiro. Tente de novo."),
            }
        }
    }
    let Some(bytes) = bytes.filter(|b| !b.is_empty()) else { return erro_json("Arquivo vazio.") };
    if bytes.len() > crate::midia::LIMITE_BYTES {
        return erro_json("Arquivo grande demais (máximo 25 MB).");
    }
    let (mid, dur) = match converter(&st, s.familia_id, &bytes).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("música: {e:#}");
            return erro_json(format!("Não consegui usar essa música: {e}"));
        }
    };
    match sqlx::query("UPDATE conteudo.historias SET musica = $3 WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .bind(mid)
        .execute(&st.db)
        .await
    {
        Ok(_) => Json(json!({ "id": mid, "duracao": dur })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[derive(Deserialize)]
pub struct Ajuste {
    volume: Option<f32>,
    #[serde(default)]
    remover: bool,
}

/// POST /historias/{id}/musica/ajustar — {volume} muda o volume; {remover: true} tira a música.
pub async fn ajustar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(a): Json<Ajuste>) -> Response {
    let r = if a.remover {
        sqlx::query("UPDATE conteudo.historias SET musica = NULL WHERE id = $1 AND familia_id = $2").bind(id).bind(s.familia_id).execute(&st.db).await
    } else if let Some(v) = a.volume {
        sqlx::query("UPDATE conteudo.historias SET musica_volume = $3 WHERE id = $1 AND familia_id = $2")
            .bind(id)
            .bind(s.familia_id)
            .bind(volume_valido(v))
            .execute(&st.db)
            .await
    } else {
        return erro_json("Nada para ajustar.");
    };
    match r {
        Ok(r) if r.rows_affected() == 1 => Json(json!({ "ok": true })).into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

async fn converter(st: &AppState, familia_id: Uuid, bytes: &[u8]) -> anyhow::Result<(Uuid, f32)> {
    use anyhow::Context;
    let tmp = std::env::temp_dir();
    let nome = Uuid::new_v4();
    let entrada = tmp.join(format!("mus-{nome}.in"));
    let saida = tmp.join(format!("mus-{nome}.mp3"));
    tokio::fs::write(&entrada, bytes).await?;
    let r = async {
        let out = tokio::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-t", &MAX_S.to_string(), "-i"])
            .arg(&entrada)
            .args([
                "-vn", "-ac", "2", "-ar", "44100",
                // Silêncio do começo e do fim fora (loop sem buraco) e volume padronizado entre músicas.
                "-af",
                "silenceremove=start_periods=1:start_threshold=-50dB,areverse,silenceremove=start_periods=1:start_threshold=-50dB,areverse,loudnorm=I=-16:TP=-1.5",
                "-c:a", "libmp3lame", "-b:a", "128k",
            ])
            .arg(&saida)
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(std::time::Duration::from_secs(120), out).await.context("demorou demais")??;
        if !out.status.success() {
            anyhow::bail!("o arquivo de áudio não abriu");
        }
        let dur = crate::tts::duracao_arquivo(&saida).await.unwrap_or(0.0);
        if dur < 3.0 {
            anyhow::bail!("a música precisa ter pelo menos 3 segundos");
        }
        let mp3 = tokio::fs::read(&saida).await?;
        let slug: String = sqlx::query_scalar("SELECT slug FROM acesso.familias WHERE id = $1").bind(familia_id).fetch_one(&st.db).await?;
        let id = crate::midia::salvar(st, familia_id, &format!("familias/{slug}/musicas"), &mp3, "audio/mpeg", "mp3").await?;
        anyhow::Ok((id, (dur * 100.0).round() / 100.0))
    }
    .await;
    let _ = tokio::fs::remove_file(&entrada).await;
    let _ = tokio::fs::remove_file(&saida).await;
    r
}

#[cfg(test)]
mod testes {
    #[test]
    fn volume_fica_no_intervalo() {
        assert_eq!(super::volume_valido(2.0), 1.0);
        assert_eq!(super::volume_valido(0.0), 0.05);
        assert_eq!(super::volume_valido(f32::NAN), 0.35);
        assert_eq!(super::volume_valido(0.4), 0.4);
    }
}

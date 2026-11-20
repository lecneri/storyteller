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
    match sqlx::query("UPDATE conteudo.historias SET musica = $3, musica_credito = NULL WHERE id = $1 AND familia_id = $2")
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

/// POST /historias/{id}/musica/pagina — multipart "arquivo": prepara uma música para uma página
/// (o editor guarda o id na página ao salvar). Não muda a música geral.
pub async fn enviar_pagina(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, mut multipart: axum::extract::Multipart) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let (mut bytes, mut nome) = (None, None);
    while let Ok(Some(campo)) = multipart.next_field().await {
        if campo.name() == Some("arquivo") {
            nome = campo.file_name().map(|n| n.rsplit_once('.').map(|(a, _)| a).unwrap_or(n).chars().take(60).collect::<String>());
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
    match converter(&st, s.familia_id, &bytes).await {
        Ok((mid, dur)) => Json(json!({ "id": mid, "duracao": dur, "nome": nome.unwrap_or_else(|| "Música enviada".into()) })).into_response(),
        Err(e) => {
            tracing::warn!("música da página: {e:#}");
            erro_json(format!("Não consegui usar essa música: {e}"))
        }
    }
}

/// POST /historias/{id}/musica/pagina-modelo {modelo} — prepara uma música-modelo para uma página (com o crédito).
pub async fn modelo_pagina(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(e): Json<EscolherModelo>) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let r: Result<Option<(Uuid, String, Option<String>)>, sqlx::Error> = sqlx::query_as(
        "WITH m AS (SELECT * FROM midia.musicas_modelo WHERE id = $2),
              a AS (INSERT INTO midia.arquivos (familia_id, caminho, tipo_mime, bytes, sha256)
                    SELECT $1, caminho, 'audio/mpeg', bytes, sha256 FROM m RETURNING id)
         SELECT a.id, m.titulo, m.credito FROM a, m",
    )
    .bind(s.familia_id)
    .bind(e.modelo)
    .fetch_optional(&st.db)
    .await;
    match r {
        Ok(Some((mid, titulo, credito))) => Json(json!({ "id": mid, "nome": titulo, "credito": credito })).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
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
        sqlx::query("UPDATE conteudo.historias SET musica = NULL, musica_credito = NULL WHERE id = $1 AND familia_id = $2").bind(id).bind(s.familia_id).execute(&st.db).await
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

/// Converte o áudio enviado: mp3 128k estéreo, sem silêncio nas pontas, volume padronizado, até 5 min.
async fn preparar(bytes: &[u8]) -> anyhow::Result<(Vec<u8>, f32)> {
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
        anyhow::Ok((tokio::fs::read(&saida).await?, (dur * 100.0).round() / 100.0))
    }
    .await;
    let _ = tokio::fs::remove_file(&entrada).await;
    let _ = tokio::fs::remove_file(&saida).await;
    r
}

async fn converter(st: &AppState, familia_id: Uuid, bytes: &[u8]) -> anyhow::Result<(Uuid, f32)> {
    let (mp3, dur) = preparar(bytes).await?;
    let slug: String = sqlx::query_scalar("SELECT slug FROM acesso.familias WHERE id = $1").bind(familia_id).fetch_one(&st.db).await?;
    let id = crate::midia::salvar(st, familia_id, &format!("familias/{slug}/musicas"), &mp3, "audio/mpeg", "mp3").await?;
    Ok((id, dur))
}

// ───────────── músicas-modelo da plataforma ─────────────

/// Climas das músicas-modelo: (id, nome).
pub const CLIMAS: &[(&str, &str)] = &[
    ("aventura", "⚔️ Aventura"),
    ("alegre", "😊 Alegre"),
    ("engracada", "😂 Engraçada"),
    ("misterio", "🕵️ Mistério leve"),
    ("magica", "✨ Mágica"),
    ("calma", "🌙 Calma / ninar"),
];

/// [{id, titulo, clima, credito, duracao, idade_min, descricao}] — para o editor e a página do sistema.
pub async fn modelos(st: &AppState) -> anyhow::Result<serde_json::Value> {
    let linhas: Vec<(Uuid, String, String, Option<String>, f32, i32, Option<String>)> = sqlx::query_as(
        "SELECT id, titulo, clima, credito, duracao, idade_min, descricao FROM midia.musicas_modelo ORDER BY clima, idade_min, titulo",
    )
    .fetch_all(&st.db)
    .await?;
    Ok(linhas
        .into_iter()
        .map(|(id, titulo, clima, credito, duracao, idade_min, descricao)| {
            json!({ "id": id, "titulo": titulo, "clima": clima, "credito": credito, "duracao": duracao, "idade_min": idade_min, "descricao": descricao })
        })
        .collect())
}

/// GET /musicas-modelo/{id} — ouvir um modelo (qualquer pessoa logada).
pub async fn ouvir_modelo(State(st): State<AppState>, _s: Sessao, Path(id): Path<Uuid>) -> Response {
    let caminho: Option<String> = match sqlx::query_scalar("SELECT caminho FROM midia.musicas_modelo WHERE id = $1").bind(id).fetch_optional(&st.db).await {
        Ok(c) => c,
        Err(e) => return erro_interno(e.into()),
    };
    let Some(caminho) = caminho else { return StatusCode::NOT_FOUND.into_response() };
    match tokio::fs::read(st.data_dir.join(caminho)).await {
        Ok(b) => ([(axum::http::header::CONTENT_TYPE, "audio/mpeg"), (axum::http::header::CACHE_CONTROL, "private, max-age=86400")], b).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
pub struct EscolherModelo {
    modelo: Uuid,
}

/// POST /historias/{id}/musica/modelo — usa uma música-modelo na história (com o crédito dela).
pub async fn usar_modelo(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(e): Json<EscolherModelo>) -> Response {
    let r = async {
        let mut tx = st.db.begin().await?;
        let mid: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO midia.arquivos (familia_id, caminho, tipo_mime, bytes, sha256)
             SELECT $1, caminho, 'audio/mpeg', bytes, sha256 FROM midia.musicas_modelo WHERE id = $2
             RETURNING id",
        )
        .bind(s.familia_id)
        .bind(e.modelo)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(mid) = mid else { return Ok(None) };
        let n = sqlx::query(
            "UPDATE conteudo.historias SET musica = $3, musica_credito = (SELECT credito FROM midia.musicas_modelo WHERE id = $4)
              WHERE id = $1 AND familia_id = $2",
        )
        .bind(id)
        .bind(s.familia_id)
        .bind(mid)
        .bind(e.modelo)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n != 1 {
            return Ok(None);
        }
        tx.commit().await?;
        Ok::<_, sqlx::Error>(Some(mid))
    }
    .await;
    match r {
        Ok(Some(mid)) => Json(json!({ "id": mid })).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// GET /sistema/musicas — lista (só admin do sistema).
pub async fn listar_modelos(State(st): State<AppState>, s: Sessao) -> Response {
    if !s.admin_sistema {
        return StatusCode::FORBIDDEN.into_response();
    }
    match modelos(&st).await {
        Ok(m) => Json(json!({ "modelos": m, "climas": CLIMAS.iter().map(|(id, n)| json!({ "id": id, "nome": n })).collect::<Vec<_>>() })).into_response(),
        Err(e) => erro_interno(e),
    }
}

/// POST /sistema/musicas — multipart: arquivo, titulo, clima, credito, descricao, idade_min.
pub async fn criar_modelo(State(st): State<AppState>, s: Sessao, mut multipart: axum::extract::Multipart) -> Response {
    if !s.admin_sistema {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (mut bytes, mut titulo, mut clima, mut credito, mut descricao, mut idade) = (None, String::new(), String::new(), String::new(), String::new(), 0i32);
    while let Ok(Some(campo)) = multipart.next_field().await {
        let nome = campo.name().unwrap_or_default().to_string();
        if nome == "arquivo" {
            match campo.bytes().await {
                Ok(b) => bytes = Some(b),
                Err(_) => return erro_json("O arquivo não chegou inteiro. Tente de novo."),
            }
        } else {
            let v = campo.text().await.unwrap_or_default().trim().to_string();
            match nome.as_str() {
                "titulo" => titulo = v,
                "clima" => clima = v,
                "credito" => credito = v,
                "descricao" => descricao = v,
                "idade_min" => idade = v.parse().unwrap_or(0).clamp(0, 18),
                _ => {}
            }
        }
    }
    let Some(bytes) = bytes.filter(|b| !b.is_empty()) else { return erro_json("Arquivo vazio.") };
    if bytes.len() > crate::midia::LIMITE_BYTES {
        return erro_json("Arquivo grande demais (máximo 25 MB).");
    }
    if titulo.is_empty() {
        return erro_json("Dê um nome para a música.");
    }
    if !CLIMAS.iter().any(|(id, _)| *id == clima) {
        return erro_json("Clima inválido.");
    }
    let r = async {
        let (mp3, dur) = preparar(&bytes).await?;
        let id = Uuid::new_v4();
        let caminho = format!("modelos/musicas/{id}.mp3");
        let destino = st.data_dir.join(&caminho);
        if let Some(dir) = destino.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&destino, &mp3).await?;
        use sha2::Digest;
        sqlx::query(
            "INSERT INTO midia.musicas_modelo (id, titulo, clima, credito, caminho, bytes, sha256, duracao, idade_min, descricao)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(&titulo)
        .bind(&clima)
        .bind(Some(credito.as_str()).filter(|c| !c.is_empty()))
        .bind(&caminho)
        .bind(mp3.len() as i64)
        .bind(format!("{:x}", sha2::Sha256::digest(&mp3)))
        .bind(dur)
        .bind(idade)
        .bind(Some(descricao.as_str()).filter(|d| !d.is_empty()))
        .execute(&st.db)
        .await?;
        anyhow::Ok(id)
    }
    .await;
    match r {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(e) => {
            tracing::warn!("música-modelo: {e:#}");
            erro_json(format!("Não consegui usar essa música: {e}"))
        }
    }
}

/// POST /sistema/musicas/{id}/apagar — histórias que já usam continuam com a música (o arquivo só sai
/// do disco quando ninguém mais usa, pela limpeza).
pub async fn apagar_modelo(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if !s.admin_sistema {
        return StatusCode::FORBIDDEN.into_response();
    }
    let r = async {
        let caminho: Option<String> = sqlx::query_scalar("DELETE FROM midia.musicas_modelo WHERE id = $1 RETURNING caminho").bind(id).fetch_optional(&st.db).await?;
        if let Some(c) = caminho {
            let usado: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM midia.arquivos WHERE caminho = $1)").bind(&c).fetch_one(&st.db).await?;
            if !usado {
                let _ = tokio::fs::remove_file(st.data_dir.join(&c)).await;
            }
        }
        anyhow::Ok(())
    }
    .await;
    match r {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e),
    }
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

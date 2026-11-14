//! Arquivos de mídia: gravação no disco (DATA_DIR) + registro em midia.arquivos.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{auth::Sessao, web::erro_interno, AppState};

/// Tamanho máximo de um arquivo enviado.
pub const LIMITE_BYTES: usize = 25 * 1024 * 1024;

/// Imagem ou vídeo (para a mídia das páginas).
pub fn tipo_midia(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if let Some(t) = tipo_imagem(bytes) {
        return Some(t);
    }
    match bytes {
        [_, _, _, _, b'f', b't', b'y', b'p', b'q', b't', ..] => Some(("video/quicktime", "mov")),
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => Some(("video/mp4", "mp4")),
        [0x1A, 0x45, 0xDF, 0xA3, ..] => Some(("video/webm", "webm")),
        _ => None,
    }
}

/// Descobre o tipo pelo conteúdo (não confia no nome nem no navegador).
pub fn tipo_imagem(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(("image/png", "png")),
        [0xFF, 0xD8, 0xFF, ..] => Some(("image/jpeg", "jpg")),
        [b'G', b'I', b'F', b'8', ..] => Some(("image/gif", "gif")),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(("image/webp", "webp")),
        _ => None,
    }
}

/// Grava o arquivo em `DATA_DIR/<pasta>/<uuid>.<ext>` e registra no banco.
pub async fn salvar(
    st: &AppState,
    familia_id: Uuid,
    pasta: &str,
    bytes: &[u8],
    tipo_mime: &str,
    ext: &str,
) -> anyhow::Result<Uuid> {
    let id = Uuid::new_v4();
    let relativo = format!("{pasta}/{id}.{ext}");
    let destino = st.data_dir.join(&relativo);
    if let Some(dir) = destino.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(&destino, bytes).await?;
    let sha = format!("{:x}", Sha256::digest(bytes));
    sqlx::query(
        "INSERT INTO midia.arquivos (id, familia_id, caminho, tipo_mime, bytes, sha256)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(familia_id)
    .bind(&relativo)
    .bind(tipo_mime)
    .bind(bytes.len() as i64)
    .bind(sha)
    .execute(&st.db)
    .await?;
    Ok(id)
}

/// GET /midia/{id}: só a família dona vê (o compartilhamento entra depois).
pub async fn servir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let linha: Option<(String, String, Uuid)> = match sqlx::query_as(
        "SELECT caminho, tipo_mime, familia_id FROM midia.arquivos WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await
    {
        Ok(l) => l,
        Err(e) => return erro_interno(e.into()),
    };
    // A família dona vê; famílias conectadas também (para ler o que foi compartilhado).
    let Some((caminho, tipo, dona)) = linha else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if dona != s.familia_id && !crate::partilha::conectado(&st, dona, s.familia_id).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(st.data_dir.join(caminho)).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, tipo),
                (header::CACHE_CONTROL, "private, max-age=31536000, immutable".into()),
            ],
            Body::from(bytes),
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Prepara uma foto para mandar à IA: reduz para no máximo 1568 px no lado maior e devolve JPEG
/// (fotos de celular passam do limite de tamanho das APIs). Não é guardada em lugar nenhum.
pub fn foto_para_ia(bytes: &[u8]) -> Option<(String, Vec<u8>)> {
    tipo_imagem(bytes)?;
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width().max(img.height()) > 1568 { img.resize(1568, 1568, image::imageops::FilterType::Triangle) } else { img };
    let mut saida = std::io::Cursor::new(Vec::new());
    img.to_rgb8().write_to(&mut saida, image::ImageFormat::Jpeg).ok()?;
    Some(("image/jpeg".to_string(), saida.into_inner()))
}

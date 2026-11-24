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
    // A pasta às vezes leva um trecho vindo da URL (slug do personagem): nunca sai da pasta de dados.
    if !pasta_segura(pasta) {
        anyhow::bail!("pasta inválida para guardar arquivo");
    }
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
/// Caminho e tipo de um arquivo que a família da sessão pode ver: a dona vê; famílias
/// conectadas também (para ler o que foi compartilhado).
pub async fn permitido(st: &AppState, s: &Sessao, id: Uuid) -> anyhow::Result<Option<(String, String)>> {
    let linha: Option<(String, String, Uuid)> =
        sqlx::query_as("SELECT caminho, tipo_mime, familia_id FROM midia.arquivos WHERE id = $1")
            .bind(id)
            .fetch_optional(&st.db)
            .await?;
    let Some((caminho, tipo, dona)) = linha else { return Ok(None) };
    if dona != s.familia_id && !crate::partilha::conectado(st, dona, s.familia_id).await {
        return Ok(None);
    }
    Ok(Some((caminho, tipo)))
}

pub async fn servir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let (caminho, tipo) = match permitido(&st, &s, id).await {
        Ok(Some(l)) => l,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e),
    };
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

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn reconhece_tipo_pelo_conteudo() {
        assert_eq!(tipo_imagem(b"\x89PNG\r\n\x1a\n...").map(|t| t.0), Some("image/png"));
        assert_eq!(tipo_imagem(b"\xFF\xD8\xFF\xE0").map(|t| t.0), Some("image/jpeg"));
        assert_eq!(tipo_imagem(b"GIF89a").map(|t| t.0), Some("image/gif"));
        assert_eq!(tipo_imagem(b"RIFF\0\0\0\0WEBPVP8").map(|t| t.0), Some("image/webp"));
        assert_eq!(tipo_midia(b"\0\0\0\x18ftypmp42").map(|t| t.0), Some("video/mp4"));
        assert_eq!(tipo_midia(b"\0\0\0\x14ftypqt  ").map(|t| t.0), Some("video/quicktime"));
        assert_eq!(tipo_midia(b"\x1A\x45\xDF\xA3").map(|t| t.0), Some("video/webm"));
        // Vídeo não é imagem; texto não é nada.
        assert!(tipo_imagem(b"\0\0\0\x18ftypmp42").is_none());
        assert!(tipo_midia(b"<html>").is_none());
    }

    #[test]
    fn foto_para_ia_reduz_e_vira_jpeg() {
        let grande = image::RgbImage::from_pixel(3000, 2000, image::Rgb([200, 100, 50]));
        let mut png = std::io::Cursor::new(Vec::new());
        grande.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let (mime, bytes) = foto_para_ia(png.get_ref()).unwrap();
        assert_eq!(mime, "image/jpeg");
        let lida = image::load_from_memory(&bytes).unwrap();
        assert_eq!((lida.width(), lida.height()), (1568, 1045));
        // Não é imagem: recusa.
        assert!(foto_para_ia(b"nada").is_none());
    }
}

/// Caminho relativo simples: sem "..", sem barra invertida, sem começar na raiz e sem trecho vazio.
fn pasta_segura(pasta: &str) -> bool {
    !pasta.contains('\\') && !pasta.contains('\0') && pasta.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

#[cfg(test)]
mod tests {
    use super::pasta_segura;

    #[test]
    fn pasta_nao_sai_da_pasta_de_dados() {
        assert!(pasta_segura("familias/neri/personagens/pedro/refs"));
        assert!(!pasta_segura("familias/neri/personagens/../../../x/refs"));
        assert!(!pasta_segura("/etc/x"));
        assert!(!pasta_segura("familias//x"));
        assert!(!pasta_segura("familias\\..\\x"));
    }
}

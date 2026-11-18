//! Fotos reais do personagem (`ficha.fotos`: [uuid]): privadas, nunca vão para as histórias.
//! Servem para o "✨ Descrever pela foto" e, quando a família marca, de referência ao gerar o desenho de uma variante.

use axum::{
    extract::{Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{auth::Sessao, midia, web::erro_interno, AppState};

use super::paginas::detalhe_com;
use super::variantes::ler_multipart;

/// Fotos reais da ficha.
pub fn fotos(ficha: &Value) -> Vec<Uuid> {
    ficha["fotos"].as_array().into_iter().flatten().filter_map(|v| v.as_str()?.parse().ok()).collect()
}

/// Lê a ficha, muda e grava de volta.
async fn mudar_ficha(st: &AppState, familia: Uuid, slug: &str, f: impl FnOnce(&mut Value)) -> Result<(), sqlx::Error> {
    let mut ficha: Value = sqlx::query_scalar("SELECT COALESCE(ficha, '{}'::jsonb) FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2")
        .bind(familia)
        .bind(slug)
        .fetch_one(&st.db)
        .await?;
    f(&mut ficha);
    sqlx::query("UPDATE conteudo.personagens SET ficha = $3, atualizado_em = now() WHERE familia_id = $1 AND slug = $2")
        .bind(familia)
        .bind(slug)
        .bind(&ficha)
        .execute(&st.db)
        .await
        .map(|_| ())
}

fn ir_ficha(slug: &str) -> Response {
    Redirect::to(&format!("/personagens/{slug}#ficha")).into_response()
}

/// Guarda as fotos na pasta privada do personagem e soma na ficha.
pub(super) async fn guardar(st: &AppState, s: &Sessao, slug: &str, arquivos: &[Vec<u8>]) -> Result<(), String> {
    let mut novas = Vec::new();
    for bytes in arquivos {
        let (mime, ext) = midia::tipo_imagem(bytes).filter(|(m, _)| *m != "image/gif").ok_or("Só fotos PNG, JPG ou WebP.")?;
        let id = midia::salvar(st, s.familia_id, &format!("familias/{}/personagens/{slug}/fotos", s.familia_slug), bytes, mime, ext)
            .await
            .map_err(|e| {
                tracing::error!("foto do personagem: {e:#}");
                "Não consegui salvar a foto.".to_string()
            })?;
        novas.push(id);
    }
    mudar_ficha(st, s.familia_id, slug, |ficha| {
        let mut lista: Vec<Value> = ficha["fotos"].as_array().cloned().unwrap_or_default();
        lista.extend(novas.iter().map(|id| json!(id)));
        ficha["fotos"] = Value::Array(lista);
    })
    .await
    .map_err(|e| {
        tracing::error!("fotos na ficha: {e:#}");
        "Não consegui guardar a foto na ficha.".to_string()
    })
}

/// POST /personagens/{slug}/fotos (multipart "fotos")
pub async fn enviar_fotos(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, mp: Multipart) -> Response {
    let (_, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    match guardar(&st, &s, &slug, &arquivos).await {
        Ok(()) => ir_ficha(&slug),
        Err(msg) => detalhe_com(&st, s, &slug, Some(msg)).await,
    }
}

/// POST /personagens/{slug}/fotos/{foto}/tirar (o arquivo sai na limpeza, se nada mais usar).
pub async fn tirar_foto(State(st): State<AppState>, s: Sessao, Path((slug, foto)): Path<(String, Uuid)>) -> Response {
    let r = mudar_ficha(&st, s.familia_id, &slug, |ficha| {
        let lista: Vec<Value> = fotos(ficha).into_iter().filter(|f| *f != foto).map(|f| json!(f)).collect();
        ficha["fotos"] = Value::Array(lista);
    })
    .await;
    match r {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/fotos/usar/{ref}: a imagem de uma variante deste personagem vira também foto de referência da ficha.
pub async fn usar_da_variante(State(st): State<AppState>, s: Sessao, Path((slug, ref_id)): Path<(String, Uuid)>) -> Response {
    let da_variante: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM conteudo.variantes v JOIN conteudo.personagens p ON p.id = v.personagem_id
                         WHERE p.familia_id = $1 AND p.slug = $2 AND $3 = ANY(v.referencias))",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(ref_id)
    .fetch_one(&st.db)
    .await
    .unwrap_or(false);
    if !da_variante {
        return ir_ficha(&slug);
    }
    let r = mudar_ficha(&st, s.familia_id, &slug, |ficha| {
        let mut lista = fotos(ficha);
        if !lista.contains(&ref_id) {
            lista.push(ref_id);
        }
        ficha["fotos"] = Value::Array(lista.into_iter().map(|f| json!(f)).collect());
    })
    .await;
    match r {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

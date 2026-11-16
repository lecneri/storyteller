//! Imagem base do personagem (na ficha, `ficha.imagem`): o desenho principal, de onde as variantes derivam.
//! Vai na frente como referência de rosto, idade e proporções ao gerar o desenho de uma variante, e
//! substitui a imagem de uma variante que ainda não tem a dela (ilustrações das histórias, miniaturas).

use axum::{
    extract::{Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use uuid::Uuid;

use crate::{auth::Sessao, AppState};

use super::paginas::{buscar, detalhe_com};
use super::variantes::{ler_multipart, salvar_refs};

/// Volta para a ficha (âncora #ficha).
fn ir_ficha(slug: &str) -> Response {
    Redirect::to(&format!("/personagens/{slug}#ficha")).into_response()
}

/// Grava (ou tira, com None) a imagem base na ficha.
pub(super) async fn definir(st: &AppState, familia: Uuid, slug: &str, imagem: Option<Uuid>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE conteudo.personagens
            SET ficha = CASE WHEN $3::uuid IS NULL THEN COALESCE(ficha, '{}'::jsonb) - 'imagem'
                             ELSE jsonb_set(COALESCE(ficha, '{}'::jsonb), '{imagem}', to_jsonb($3::text)) END,
                atualizado_em = now()
          WHERE familia_id = $1 AND slug = $2",
    )
    .bind(familia)
    .bind(slug)
    .bind(imagem)
    .execute(&st.db)
    .await
    .map(|_| ())
}

/// Imagem base de uma ficha (JSON), se houver.
pub fn da_ficha(ficha: &serde_json::Value) -> Option<Uuid> {
    ficha["imagem"].as_str()?.parse().ok()
}

/// Personagem novo com imagem: a primeira vira a imagem base.
pub(super) async fn criar(st: &AppState, s: &Sessao, slug: &str, arquivos: &[Vec<u8>]) -> Result<(), String> {
    let ids = salvar_refs(st, s, slug, &arquivos[..1]).await?;
    definir(st, s.familia_id, slug, ids.first().copied()).await.map_err(|e| {
        tracing::error!("imagem base do personagem novo: {e:#}");
        "O personagem foi criado, mas não consegui guardar a imagem. Envie de novo na ficha.".to_string()
    })
}

/// POST /personagens/{slug}/imagem (multipart "imagem"): envia ou troca a imagem base.
pub async fn enviar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, mp: Multipart) -> Response {
    let (_, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    if arquivos.is_empty() {
        return detalhe_com(&st, s, &slug, Some("Escolha uma imagem.".into())).await;
    }
    match criar(&st, &s, &slug, &arquivos).await {
        Ok(()) => ir_ficha(&slug),
        Err(msg) => detalhe_com(&st, s, &slug, Some(msg)).await,
    }
}

/// POST /personagens/{slug}/imagem/usar/{ref}: usa a imagem de uma variante (ou uma foto) como imagem base.
pub async fn usar(State(st): State<AppState>, s: Sessao, Path((slug, ref_id)): Path<(String, Uuid)>) -> Response {
    let dele: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM conteudo.personagens p
                         WHERE p.familia_id = $1 AND p.slug = $2
                           AND (COALESCE(p.ficha->'fotos', '[]') ? $3::text
                                OR EXISTS (SELECT 1 FROM conteudo.variantes v WHERE v.personagem_id = p.id AND $3 = ANY(v.referencias))))",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(ref_id)
    .fetch_one(&st.db)
    .await
    .unwrap_or(false);
    if !dele {
        return ir_ficha(&slug);
    }
    match definir(&st, s.familia_id, &slug, Some(ref_id)).await {
        Ok(()) => ir_ficha(&slug),
        Err(e) => crate::web::erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/imagem/tirar: tira a imagem base (o arquivo sai na limpeza, se nada mais usar).
pub async fn tirar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    match definir(&st, s.familia_id, &slug, None).await {
        Ok(()) => ir_ficha(&slug),
        Err(e) => crate::web::erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/imagem/gerar: gera a imagem base pela API, só pela ficha (aparência e fotos).
pub async fn gerar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    let p = match buscar(&st, s.familia_id, &slug).await {
        Ok(Some(p)) => p,
        Ok(None) => return Redirect::to("/personagens").into_response(),
        Err(e) => return crate::web::erro_interno(e.into()),
    };
    // Referências: as fotos e, se já houver, a imagem base atual (para manter o rosto ao refazer).
    let mut ids: Vec<Uuid> = da_ficha(&p.ficha).into_iter().collect();
    ids.extend(p.ficha["fotos"].as_array().into_iter().flatten().filter_map(|v| v.as_str()?.parse::<Uuid>().ok()));
    let refs = crate::imagem::carregar_refs(&st, s.familia_id, &ids).await;
    let prompt = super::prompt_model_sheet(&p.nome, &p.campo("aparencia"), None, !refs.is_empty());
    match crate::imagem::gerar(&st, s.familia_id, &s.familia_slug, &format!("personagens/{slug}/refs"), &prompt, &refs).await {
        Ok((img, _)) => match definir(&st, s.familia_id, &slug, Some(img)).await {
            Ok(()) => ir_ficha(&slug),
            Err(e) => crate::web::erro_interno(e.into()),
        },
        Err(e) => detalhe_com(&st, s, &slug, Some(format!("{e:#}"))).await,
    }
}

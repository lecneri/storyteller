//! Imagens da história pela fila do servidor (crate::fila_imagens): o pedido volta na hora e o editor acompanha.
//! O editor monta o prompt e as referências; a imagem pronta vai direto para a página certa (pela `chave`
//! da página, que não muda se as páginas forem reordenadas) ou para a capa.

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
    fila_imagens::{self, Pedido as PedidoFila, AVULSO, LOTE},
    web::{erro_interno, erro_json},
    AppState,
};

#[derive(Deserialize)]
pub struct PedidoPagina {
    chave: String,
    prompt: String,
    #[serde(default)]
    refs: Vec<Uuid>,
}

#[derive(Deserialize)]
pub struct Pedido {
    paginas: Vec<PedidoPagina>,
}

async fn da_familia(st: &AppState, s: &Sessao, id: Uuid) -> Result<(), Response> {
    match crate::web::historia_da_familia(st, s.familia_id, id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(StatusCode::NOT_FOUND.into_response()),
        Err(e) => Err(erro_interno(e.into())),
    }
}

fn pedido_pagina(historia: Uuid, alvo: &'static str, chave: Option<String>, prompt: String, refs: Vec<Uuid>) -> PedidoFila {
    PedidoFila { alvo, historia: Some(historia), chave, variante: None, prompt, refs, pasta: format!("historias/{historia}/midia"), recortar: false }
}

/// Páginas que a fila ainda vai preencher ou acabou de preencher: o salvar do editor (que pode estar
/// desatualizado) não troca a imagem delas.
pub async fn chaves_protegidas(st: &AppState, historia: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT DISTINCT chave FROM jobs.imagens WHERE historia_id = $1 AND alvo = 'pagina' AND chave IS NOT NULL
            AND (estado IN ('pendente', 'rodando') OR (estado = 'feito' AND atualizado_em > now() - interval '2 minutes'))",
    )
    .bind(historia)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default()
}

/// POST /historias/{id}/imagens: "Gerar todas" — uma imagem por página, todas na fila.
pub async fn pedir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(p): Json<Pedido>) -> Response {
    if let Err(r) = da_familia(&st, &s, id).await {
        return r;
    }
    if fila_imagens::da_historia(&st, s.familia_id, id).await.iter().any(|i| i.ativo() && i.lote.is_some()) {
        return erro_json("As imagens desta história já estão sendo geradas.");
    }
    let pedidos: Vec<PedidoFila> = p
        .paginas
        .into_iter()
        .filter(|x| !x.chave.trim().is_empty() && !x.prompt.trim().is_empty())
        .map(|x| pedido_pagina(id, "pagina", Some(x.chave), x.prompt, x.refs))
        .collect();
    if pedidos.is_empty() {
        return erro_json("Nenhuma página para gerar.");
    }
    match fila_imagens::pedir(&st, s.familia_id, &s.familia_slug, pedidos, LOTE).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_json(format!("{e:#}")),
    }
}

#[derive(Deserialize)]
pub struct PedidoUma {
    prompt: String,
    #[serde(default)]
    refs: Vec<Uuid>,
    /// Página que recebe a imagem; sem chave e com `capa`, vai para a capa.
    #[serde(default)]
    chave: Option<String>,
    #[serde(default)]
    capa: bool,
}

/// POST /historias/{id}/imagem-ia: ✨ Gerar de uma página ou da capa (passa na frente dos lotes).
pub async fn pedir_uma(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(p): Json<PedidoUma>) -> Response {
    if let Err(r) = da_familia(&st, &s, id).await {
        return r;
    }
    let chave = p.chave.filter(|c| !c.trim().is_empty());
    let alvo = match (&chave, p.capa) {
        (_, true) => "capa",
        (Some(_), false) => "pagina",
        (None, false) => return erro_json("Salve a história antes de gerar a imagem."),
    };
    let ja = fila_imagens::da_historia(&st, s.familia_id, id)
        .await
        .into_iter()
        .any(|i| i.ativo() && i.alvo == alvo && (alvo == "capa" || i.chave == chave));
    if ja {
        return erro_json("Esta imagem já está na fila.");
    }
    let chave = if alvo == "capa" { None } else { chave };
    match fila_imagens::pedir(&st, s.familia_id, &s.familia_slug, vec![pedido_pagina(id, alvo, chave, p.prompt, p.refs)], AVULSO).await {
        Ok(ids) => Json(json!({ "id": ids.first() })).into_response(),
        Err(e) => erro_json(format!("{e:#}")),
    }
}

/// POST /historias/{id}/imagens/parar: tira da fila as páginas do lote que ainda não começaram.
pub async fn parar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if let Err(r) = da_familia(&st, &s, id).await {
        return r;
    }
    let _ = sqlx::query(
        "UPDATE jobs.imagens SET estado = 'cancelado', atualizado_em = now()
          WHERE historia_id = $1 AND familia_id = $2 AND lote IS NOT NULL AND estado = 'pendente'",
    )
    .bind(id)
    .bind(s.familia_id)
    .execute(&st.db)
    .await;
    Json(json!({ "ok": true })).into_response()
}

/// GET /historias/{id}/imagens: itens na fila (e os que terminaram há pouco) e a fonte da história.
pub async fn estado(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if let Err(r) = da_familia(&st, &s, id).await {
        return r;
    }
    let itens = fila_imagens::da_historia(&st, s.familia_id, id).await;
    let fonte: Option<String> = sqlx::query_scalar("SELECT fonte_imagem FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
        .flatten();
    let filas = crate::imagem::filas(&st, s.familia_id).await.unwrap_or_default();
    Json(json!({
        "fonte": fonte.filter(|f| filas.contains(f)),
        "filas": filas.iter().map(|f| json!({ "id": f, "nome": crate::imagem::nome_fila(f) })).collect::<Vec<_>>(),
        "itens": itens.iter().map(fila_imagens::Situacao::json).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct Fonte {
    fonte: Option<String>,
}

/// POST /historias/{id}/fonte-imagem: troca quem desenha esta história (vale para os próximos pedidos).
pub async fn definir_fonte(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(f): Json<Fonte>) -> Response {
    if let Err(r) = da_familia(&st, &s, id).await {
        return r;
    }
    let filas = crate::imagem::filas(&st, s.familia_id).await.unwrap_or_default();
    let fonte = f.fonte.filter(|x| filas.contains(x));
    match sqlx::query("UPDATE conteudo.historias SET fonte_imagem = $2 WHERE id = $1").bind(id).bind(&fonte).execute(&st.db).await {
        Ok(_) => Json(json!({ "ok": true, "fonte": fonte })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

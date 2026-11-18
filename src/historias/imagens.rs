//! "Gerar todas as imagens" na fila do servidor: a família pode fechar a aba.
//! O editor monta o prompt e as referências de cada página e manda a lista; o job gera uma por vez
//! e põe cada imagem na página certa (pela `chave` da página, que não muda se as páginas forem reordenadas).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    jobs,
    web::{erro_interno, erro_json},
    AppState,
};

pub const TIPO: &str = "gerar_imagens";

#[derive(Deserialize, serde::Serialize, Clone)]
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

/// Job em andamento (pendente ou rodando) desta história: (id, payload).
async fn em_andamento(st: &AppState, historia: Uuid) -> Option<(i64, Value)> {
    sqlx::query_as(
        "SELECT id, payload FROM jobs.fila WHERE tipo = $1 AND payload->>'historia_id' = $2
            AND estado IN ('pendente', 'rodando') ORDER BY id DESC LIMIT 1",
    )
    .bind(TIPO)
    .bind(historia.to_string())
    .fetch_optional(&st.db)
    .await
    .unwrap_or(None)
}

/// Chaves das páginas que o job ainda vai preencher (o salvar do editor não pode apagar a imagem delas).
pub async fn chaves_protegidas(st: &AppState, historia: Uuid) -> Vec<String> {
    em_andamento(st, historia)
        .await
        .map(|(_, p)| p["paginas"].as_array().into_iter().flatten().filter_map(|x| x["chave"].as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// POST /historias/{id}/imagens: põe na fila.
pub async fn pedir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(p): Json<Pedido>) -> Response {
    match crate::web::historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    if em_andamento(&st, id).await.is_some() {
        return erro_json("As imagens desta história já estão sendo geradas.");
    }
    let paginas: Vec<PedidoPagina> = p.paginas.into_iter().filter(|x| !x.chave.trim().is_empty() && !x.prompt.trim().is_empty()).collect();
    if paginas.is_empty() {
        return erro_json("Nenhuma página para gerar.");
    }
    let payload = json!({
        "historia_id": id, "familia_slug": s.familia_slug, "paginas": paginas,
        "feitas": [], "falhas": [], "parar": false,
    });
    match jobs::enfileirar(&st, TIPO, s.familia_id, payload).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e),
    }
}

/// POST /historias/{id}/imagens/parar: termina depois da imagem atual.
pub async fn parar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    match crate::web::historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    // Pendente ainda não começou: sai da fila na hora.
    let _ = sqlx::query(
        "UPDATE jobs.fila SET estado = CASE WHEN estado = 'pendente' THEN 'feito' ELSE estado END,
                payload = payload || '{\"parar\": true}', atualizado_em = now()
          WHERE tipo = $1 AND payload->>'historia_id' = $2 AND estado IN ('pendente', 'rodando')",
    )
    .bind(TIPO)
    .bind(id.to_string())
    .execute(&st.db)
    .await;
    Json(json!({ "ok": true })).into_response()
}

/// GET /historias/{id}/imagens: progresso do último pedido e as imagens das páginas já feitas.
pub async fn estado(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let linha: Option<(String, Value, Option<String>, Value)> = sqlx::query_as(
        "SELECT j.estado, j.payload, j.erro, h.conteudo FROM jobs.fila j JOIN conteudo.historias h ON h.id = $2
          WHERE j.tipo = $1 AND j.payload->>'historia_id' = $3 AND h.familia_id = $4 ORDER BY j.id DESC LIMIT 1",
    )
    .bind(TIPO)
    .bind(id)
    .bind(id.to_string())
    .bind(s.familia_id)
    .fetch_optional(&st.db)
    .await
    .unwrap_or(None);
    let Some((estado, payload, erro, conteudo)) = linha else {
        return Json(json!({ "job": null })).into_response();
    };
    let feitas: Vec<String> = payload["feitas"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    let imagens: Vec<Value> = conteudo["paginas"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["chave"].as_str().is_some_and(|c| feitas.iter().any(|f| f == c)))
        .map(|p| json!({ "chave": p["chave"], "midia": p["midia"] }))
        .collect();
    Json(json!({
        "job": estado,
        "total": payload["paginas"].as_array().map_or(0, Vec::len),
        "feitas": feitas.len(),
        "falhas": payload["falhas"],
        "parando": payload["parar"] == true,
        "erro": erro,
        "imagens": imagens,
    }))
    .into_response()
}

/// Põe a imagem na página (pela chave), sem mexer no resto da história.
async fn colocar(st: &AppState, historia: Uuid, chave: &str, midia: Uuid, tipo: &str) -> anyhow::Result<bool> {
    let mut tx = st.db.begin().await?;
    let mut conteudo: Value = sqlx::query_scalar("SELECT conteudo FROM conteudo.historias WHERE id = $1 FOR UPDATE")
        .bind(historia)
        .fetch_one(&mut *tx)
        .await?;
    let Some(pag) = conteudo["paginas"].as_array_mut().and_then(|ps| ps.iter_mut().find(|p| p["chave"] == chave)) else {
        return Ok(false); // a página foi apagada
    };
    pag["midia"] = json!({ "id": midia, "tipo": tipo });
    sqlx::query("UPDATE conteudo.historias SET conteudo = $2, atualizado_em = now() WHERE id = $1")
        .bind(historia)
        .bind(&conteudo)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// Executa o job: uma imagem por vez; anota o progresso no payload (sobrevive a reinício do app).
pub async fn executar(st: &AppState, job_id: i64, familia: Uuid, payload: &Value) -> anyhow::Result<Value> {
    let historia: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let familia_slug = payload["familia_slug"].as_str().unwrap_or_default().to_string();
    let paginas: Vec<PedidoPagina> = serde_json::from_value(payload["paginas"].clone())?;
    let pasta = format!("historias/{historia}/midia");
    for pag in &paginas {
        let atual: Value = sqlx::query_scalar("SELECT payload FROM jobs.fila WHERE id = $1").bind(job_id).fetch_one(&st.db).await?;
        if atual["parar"] == true {
            break;
        }
        let ja = |k: &str| atual[k].as_array().is_some_and(|a| a.iter().any(|x| x == pag.chave.as_str() || x["chave"] == pag.chave.as_str()));
        if ja("feitas") || ja("falhas") {
            continue; // reinício do app: já tentada
        }
        let refs = crate::imagem::carregar_refs(st, familia, &pag.refs).await;
        let r = match crate::imagem::gerar(st, familia, &familia_slug, &pasta, &pag.prompt, &refs).await {
            Ok((mid, tipo)) => colocar(st, historia, &pag.chave, mid, &tipo).await.map(|_| ()),
            Err(e) => Err(e),
        };
        let campo = if r.is_ok() { "feitas" } else { "falhas" };
        let item = match &r {
            Ok(()) => json!(pag.chave),
            Err(e) => json!({ "chave": pag.chave, "erro": format!("{e:#}") }),
        };
        sqlx::query(&format!(
            "UPDATE jobs.fila SET payload = jsonb_set(payload, '{{{campo}}}', COALESCE(payload->'{campo}', '[]') || jsonb_build_array($2::jsonb)),
                    atualizado_em = now() WHERE id = $1"
        ))
        .bind(job_id)
        .bind(&item)
        .execute(&st.db)
        .await?;
    }
    Ok(json!({ "ok": true }))
}

//! Rotas de voz e narração: prévia, voz de personagem/narrador, áudio por bloco e narração da história.

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
    historias::Conteudo,
    jobs,
    tts::{self, Voz},
    web::erro_interno,
    AppState,
};
use crate::web::erro_json as erro;


fn audio_json(a: &tts::Audio) -> Value {
    json!({ "audio": format!("/midia/{}", a.arquivo), "duracao": a.duracao, "palavras": a.palavras })
}

/// GET /vozes: lista de vozes + as vozes atuais da família.
pub async fn vozes(State(st): State<AppState>, s: Sessao) -> Response {
    match tts::vozes_da_familia(&st, s.familia_id).await {
        Ok(v) => Json(json!({
            "vozes": tts::VOZES.iter().map(|(id, nome)| json!({ "id": id, "nome": nome })).collect::<Vec<_>>(),
            "narrador": v.narrador,
            "personagens": v.personagens.into_iter().map(|(s, v)| json!({ "slug": s, "voz": v })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => erro_interno(e),
    }
}

#[derive(Deserialize)]
pub struct PedidoPrevia {
    voz: Voz,
    texto: String,
}

/// POST /tts/previa: gera (ou pega do cache) um áudio curto para ouvir a voz.
pub async fn previa(State(st): State<AppState>, s: Sessao, Json(p): Json<PedidoPrevia>) -> Response {
    let texto: String = p.texto.trim().chars().take(300).collect();
    if texto.is_empty() {
        return erro("Escreva um texto para ouvir.");
    }
    match tts::sintetizar(&st, s.familia_id, &s.familia_slug, &texto, &p.voz.validada()).await {
        Ok(a) => Json(audio_json(&a)).into_response(),
        Err(e) => {
            tracing::warn!("prévia de voz falhou: {e:#}");
            erro("Não consegui gerar a voz agora. Tente de novo.")
        }
    }
}

#[derive(Deserialize)]
pub struct PedidoVoz {
    /// null = volta para a voz padrão
    voz: Option<Voz>,
}

/// POST /personagens/{slug}/voz
pub async fn salvar_voz_personagem(
    State(st): State<AppState>,
    s: Sessao,
    Path(slug): Path<String>,
    Json(p): Json<PedidoVoz>,
) -> Response {
    let valor = p.voz.map(|v| json!(v.validada())).unwrap_or(json!({}));
    let r = sqlx::query("UPDATE conteudo.personagens SET voz_padrao = $3, atualizado_em = now() WHERE familia_id = $1 AND slug = $2")
        .bind(s.familia_id)
        .bind(&slug)
        .bind(valor)
        .execute(&st.db)
        .await;
    match r {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// GET /personagens/{slug}/vozes-extras: as vozes extras do personagem + as línguas oferecidas.
pub async fn vozes_extras(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    let v: Option<Value> = match sqlx::query_scalar("SELECT vozes_extras FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2")
        .bind(s.familia_id)
        .bind(&slug)
        .fetch_optional(&st.db)
        .await
    {
        Ok(v) => v,
        Err(e) => return erro_interno(e.into()),
    };
    let Some(v) = v else { return StatusCode::NOT_FOUND.into_response() };
    Json(json!({
        "vozes_extras": v,
        "linguas": tts::LINGUAS.iter().map(|(n, f)| json!({ "nome": n, "exemplo": f })).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// POST /personagens/{slug}/vozes-extras: grava a lista inteira ([{nome, lingua, voz}]).
pub async fn salvar_vozes_extras(
    State(st): State<AppState>,
    s: Sessao,
    Path(slug): Path<String>,
    Json(lista): Json<Vec<tts::VozExtra>>,
) -> Response {
    let mut limpas: Vec<tts::VozExtra> = Vec::new();
    for mut e in lista.into_iter().take(10) {
        e.nome = e.nome.trim().chars().take(40).collect();
        e.slug = crate::util::slug(&e.nome);
        if e.slug.is_empty() {
            return erro("Dê um nome para cada voz (ex.: Inglês).");
        }
        if limpas.iter().any(|x| x.slug == e.slug) {
            return erro("Duas vozes com o mesmo nome.");
        }
        if !tts::LINGUAS.iter().any(|(n, _)| *n == e.lingua) {
            e.lingua = "Português".into();
        }
        e.voz = e.voz.validada();
        limpas.push(e);
    }
    let r = sqlx::query("UPDATE conteudo.personagens SET vozes_extras = $3, atualizado_em = now() WHERE familia_id = $1 AND slug = $2")
        .bind(s.familia_id)
        .bind(&slug)
        .bind(json!(limpas))
        .execute(&st.db)
        .await;
    match r {
        Ok(_) => Json(json!({ "ok": true, "vozes_extras": limpas })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /familia/narrador
pub async fn salvar_narrador(State(st): State<AppState>, s: Sessao, Json(p): Json<PedidoVoz>) -> Response {
    let r = match p.voz {
        Some(v) => {
            sqlx::query("UPDATE acesso.familias SET config = jsonb_set(config, '{narrador}', $2) WHERE id = $1")
                .bind(s.familia_id)
                .bind(json!(v.validada()))
                .execute(&st.db)
                .await
        }
        None => {
            sqlx::query("UPDATE acesso.familias SET config = config - 'narrador' WHERE id = $1")
                .bind(s.familia_id)
                .execute(&st.db)
                .await
        }
    };
    match r {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[derive(Deserialize)]
pub struct PedidoBloco {
    quem: String,
    texto: String,
}

/// POST /historias/{id}/bloco-audio: ouvir um bloco no editor.
pub async fn audio_bloco(
    State(st): State<AppState>,
    s: Sessao,
    Path(id): Path<Uuid>,
    Json(p): Json<PedidoBloco>,
) -> Response {
    if p.texto.trim().is_empty() {
        return erro("Bloco vazio.");
    }
    if !crate::partilha::pode_ler_historia(&st, s.familia_id, id).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    let narracao: String = match sqlx::query_scalar("SELECT narracao FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_one(&st.db)
        .await
    {
        Ok(n) => n,
        Err(e) => return erro_interno(e.into()),
    };
    let quem = if narracao == "narrador" { "narrador" } else { p.quem.as_str() };
    let voz = match crate::player::voz_para(&st, s.familia_id, id, quem).await {
        Ok(v) => v,
        Err(e) => return erro_interno(e),
    };
    match tts::sintetizar(&st, s.familia_id, &s.familia_slug, &p.texto, &voz).await {
        Ok(a) => Json(audio_json(&a)).into_response(),
        Err(e) => {
            tracing::warn!("áudio do bloco falhou: {e:#}");
            erro("Não consegui gerar a voz agora. Tente de novo.")
        }
    }
}

#[derive(sqlx::FromRow)]
struct HistoriaNarrar {
    familia_id: Uuid,
    familia_slug: String,
    narracao: String,
    conteudo: Value,
}

async fn carregar(st: &AppState, id: Uuid) -> anyhow::Result<HistoriaNarrar> {
    Ok(sqlx::query_as::<_, HistoriaNarrar>(
        "SELECT h.familia_id, f.slug AS familia_slug, h.narracao, h.conteudo
           FROM conteudo.historias h JOIN acesso.familias f ON f.id = h.familia_id WHERE h.id = $1",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?)
}

/// Lista (página, bloco, quem, texto) dos blocos com texto.
fn blocos(h: &HistoriaNarrar) -> Vec<(usize, usize, String, String)> {
    let c: Conteudo = serde_json::from_value(h.conteudo.clone()).unwrap_or_default();
    let mut v = Vec::new();
    for (pi, p) in c.paginas.iter().enumerate() {
        for (bi, b) in p.blocos.iter().enumerate() {
            if !b.texto.trim().is_empty() {
                let quem = if h.narracao == "narrador" { "narrador".to_string() } else { b.chave_voz() };
                v.push((pi, bi, quem, b.texto.clone()));
            }
        }
    }
    v
}

/// Job: gera o áudio de todos os blocos (o que já está no cache é reaproveitado).
pub async fn executar_narracao(st: &AppState, payload: &Value) -> anyhow::Result<()> {
    let id: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let h = carregar(st, id).await?;
    let vozes = tts::vozes_da_familia(st, h.familia_id).await?;
    let mut falhas = 0;
    for (_, _, quem, texto) in blocos(&h) {
        if let Err(e) = tts::sintetizar(st, h.familia_id, &h.familia_slug, &texto, &vozes.de(&quem)).await {
            tracing::warn!("narração: bloco falhou: {e:#}");
            falhas += 1;
        }
    }
    if falhas > 0 {
        anyhow::bail!("{falhas} trecho(s) não foram gerados; tente de novo");
    }
    sqlx::query("UPDATE conteudo.historias SET estado = 'narracao', atualizado_em = now() WHERE id = $1 AND estado <> 'pronta'")
        .bind(id)
        .execute(&st.db)
        .await?;
    Ok(())
}

/// Job: gera o áudio de todos os trechos com as vozes genéricas (as do link público).
pub async fn executar_narracao_publica(st: &AppState, payload: &Value) -> anyhow::Result<()> {
    let id: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let h = carregar(st, id).await?;
    let tipos: Vec<(String, String)> = sqlx::query_as("SELECT slug, tipo FROM conteudo.personagens WHERE familia_id = $1")
        .bind(h.familia_id)
        .fetch_all(&st.db)
        .await?;
    let mut falhas = 0;
    for (_, _, quem, texto) in blocos(&h) {
        let tipo = tipos.iter().find(|(s, _)| s == tts::separar(&quem).0).map(|(_, t)| t.as_str()).unwrap_or("");
        if let Err(e) = tts::sintetizar(st, h.familia_id, &h.familia_slug, &texto, &tts::voz_generica(&quem, tipo)).await {
            tracing::warn!("narração pública: trecho falhou: {e:#}");
            falhas += 1;
        }
    }
    if falhas > 0 {
        anyhow::bail!("{falhas} trecho(s) não foram gerados");
    }
    Ok(())
}

/// POST /historias/{id}/narrar
pub async fn narrar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let ok = crate::web::historia_da_familia(&st, s.familia_id, id).await;
    match ok {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    match jobs::enfileirar(&st, "gerar_narracao", s.familia_id, json!({ "historia_id": id })).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e),
    }
}

/// GET /historias/{id}/narracao: quais blocos já têm áudio (com a voz atual) + estado do job.
pub async fn estado(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let h = match carregar(&st, id).await {
        Ok(h) if h.familia_id == s.familia_id => h,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e),
    };
    let vozes = match tts::vozes_da_familia(&st, h.familia_id).await {
        Ok(v) => v,
        Err(e) => return erro_interno(e),
    };
    let mut itens = Vec::new();
    let mut prontos = 0;
    let lista = blocos(&h);
    for (pi, bi, quem, texto) in &lista {
        let a = match tts::no_cache(&st, h.familia_id, texto, &vozes.de(quem)).await {
            Ok(a) => a,
            Err(e) => return erro_interno(e),
        };
        if a.is_some() {
            prontos += 1;
        }
        itens.push(json!({ "pagina": pi, "bloco": bi, "quem": quem, "audio": a.as_ref().map(audio_json) }));
    }
    let job: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT estado, erro FROM jobs.fila WHERE tipo = 'gerar_narracao' AND payload->>'historia_id' = $1
          ORDER BY id DESC LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(&st.db)
    .await
    .unwrap_or(None);
    Json(json!({
        "total": lista.len(),
        "prontos": prontos,
        "job": job.as_ref().map(|(e, _)| e.clone()),
        "erro": job.and_then(|(_, e)| e),
        "blocos": itens,
    }))
    .into_response()
}

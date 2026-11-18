//! Lugares da família: cenários fixos (o castelo, a casa da vovó) citados no briefing com `@lugar:slug`.
//! Têm uma descrição que nunca muda e imagens de referência; a luz, a hora e o clima vêm da cena de cada página.

use askama::Template;
use axum::{
    extract::{Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
    Form, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    fila_imagens, midia, util,
    web::{erro_interno, erro_json, render},
    AppState,
};

#[derive(sqlx::FromRow, Clone, serde::Serialize)]
pub struct Lugar {
    pub id: Uuid,
    pub slug: String,
    pub nome: String,
    pub descricao: String,
    pub referencias: Vec<Uuid>,
}

/// Todos os lugares da família (para o editor, o briefing e a geração).
pub async fn da_familia(st: &AppState, familia: Uuid) -> Vec<Lugar> {
    sqlx::query_as("SELECT id, slug, nome, descricao, referencias FROM conteudo.lugares WHERE familia_id = $1 ORDER BY nome")
        .bind(familia)
        .fetch_all(&st.db)
        .await
        .unwrap_or_default()
}

async fn buscar(st: &AppState, familia: Uuid, slug: &str) -> Option<Lugar> {
    sqlx::query_as("SELECT id, slug, nome, descricao, referencias FROM conteudo.lugares WHERE familia_id = $1 AND slug = $2")
        .bind(familia)
        .bind(slug)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
}

/// Prompt do desenho de referência do lugar (sem personagens).
pub fn prompt_lugar(nome: &str, descricao: &str, com_imagens: bool) -> String {
    let mut p = format!(
        "Ilustração de livro infantil, formato retrato 3:4: o cenário \"{nome}\", SEM nenhum personagem, pessoa ou animal.\n\
         Como é o lugar (sempre igual em todas as histórias): {}\n\
         Estilo: ilustração de livro infantil, cores vivas, traço limpo e amigável. Luz de dia, enquadramento amplo mostrando o lugar inteiro.\n\
         Não escreva nenhum texto, letra ou número na imagem.",
        if descricao.trim().is_empty() { "(invente um visual marcante e fácil de reconhecer)" } else { descricao.trim() }
    );
    if com_imagens {
        p.push_str("\nAs imagens anexadas já são este lugar: mantenha o formato, os objetos e as cores delas.");
    }
    p
}

// ───────────── telas ─────────────

#[derive(Template)]
#[template(path = "lugares.html")]
struct PaginaLugares {
    s: Sessao,
    lugares: Vec<Lugar>,
    erro: Option<String>,
}

#[derive(Template)]
#[template(path = "lugar.html")]
struct PaginaLugar {
    s: Sessao,
    l: Lugar,
    imagem_api: bool,
    fila: Vec<fila_imagens::Situacao>,
    erro: Option<String>,
}

impl PaginaLugar {
    fn prompt(&self) -> String {
        prompt_lugar(&self.l.nome, &self.l.descricao, !self.l.referencias.is_empty())
    }
    fn na_fila(&self) -> Vec<&fila_imagens::Situacao> {
        self.fila.iter().filter(|i| i.ativo()).collect()
    }
    fn falha(&self) -> Option<String> {
        if !self.na_fila().is_empty() {
            return None;
        }
        self.fila.iter().rev().find(|i| i.estado == "erro").map(|i| i.rotulo())
    }
}

async fn lista_com(st: &AppState, s: Sessao, erro: Option<String>) -> Response {
    let lugares = da_familia(st, s.familia_id).await;
    render(PaginaLugares { s, lugares, erro })
}

/// GET /lugares
pub async fn lista(State(st): State<AppState>, s: Sessao) -> Response {
    lista_com(&st, s, None).await
}

async fn detalhe_com(st: &AppState, s: Sessao, slug: &str, erro: Option<String>) -> Response {
    let Some(l) = buscar(st, s.familia_id, slug).await else { return Redirect::to("/lugares").into_response() };
    let imagem_api = crate::imagem::disponivel(st, s.familia_id).await;
    let fila = fila_imagens::do_lugar(st, s.familia_id, l.id).await;
    render(PaginaLugar { s, l, imagem_api, fila, erro })
}

/// GET /lugares/{slug}
pub async fn detalhe(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    detalhe_com(&st, s, &slug, None).await
}

/// Slug livre na família (castelo, castelo-2...).
async fn slug_livre(st: &AppState, familia: Uuid, base: &str, proprio: Option<Uuid>) -> anyhow::Result<String> {
    let base = Some(util::slug(base)).filter(|b| !b.is_empty()).unwrap_or_else(|| "lugar".into());
    for n in 1.. {
        let tentativa = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let existe: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM conteudo.lugares WHERE familia_id = $1 AND slug = $2 AND id IS DISTINCT FROM $3)",
        )
        .bind(familia)
        .bind(&tentativa)
        .bind(proprio)
        .fetch_one(&st.db)
        .await?;
        if !existe {
            return Ok(tentativa);
        }
    }
    unreachable!()
}

async fn inserir(st: &AppState, familia: Uuid, nome: &str, descricao: &str) -> anyhow::Result<String> {
    let slug = slug_livre(st, familia, nome, None).await?;
    sqlx::query("INSERT INTO conteudo.lugares (familia_id, slug, nome, descricao) VALUES ($1, $2, $3, $4)")
        .bind(familia)
        .bind(&slug)
        .bind(nome)
        .bind(descricao)
        .execute(&st.db)
        .await?;
    Ok(slug)
}

#[derive(Deserialize)]
pub struct FormNovo {
    nome: String,
    #[serde(default)]
    descricao: String,
    /// "1": já pede o desenho à fila de imagens.
    #[serde(default)]
    gerar: Option<String>,
}

/// POST /lugares
pub async fn criar(State(st): State<AppState>, s: Sessao, Form(f): Form<FormNovo>) -> Response {
    let nome = f.nome.trim();
    if nome.is_empty() {
        return lista_com(&st, s, Some("Dê um nome para o lugar.".into())).await;
    }
    let slug = match inserir(&st, s.familia_id, nome, f.descricao.trim()).await {
        Ok(sl) => sl,
        Err(e) => return erro_interno(e),
    };
    if f.gerar.is_some() && crate::imagem::disponivel(&st, s.familia_id).await {
        if let Err(msg) = pedir_desenho(&st, &s, &slug).await {
            return detalhe_com(&st, s, &slug, Some(format!("O lugar foi criado, mas o desenho não entrou na fila: {msg}"))).await;
        }
    }
    Redirect::to(&format!("/lugares/{slug}")).into_response()
}

#[derive(Deserialize)]
pub struct NovoDaHistoria {
    nome: String,
    #[serde(default)]
    descricao: String,
}

/// POST /lugares/da-historia (JSON): "Salvar como lugar" no editor. Devolve o slug.
pub async fn criar_da_historia(State(st): State<AppState>, s: Sessao, Json(n): Json<NovoDaHistoria>) -> Response {
    let nome = n.nome.trim();
    if nome.is_empty() {
        return erro_json("Lugar sem nome.");
    }
    match inserir(&st, s.familia_id, nome, n.descricao.trim()).await {
        Ok(slug) => Json(json!({ "slug": slug, "nome": nome, "descricao": n.descricao.trim(), "referencias": [] })).into_response(),
        Err(e) => erro_interno(e),
    }
}

#[derive(Deserialize)]
pub struct FormFicha {
    nome: String,
    #[serde(default)]
    arroba: String,
    #[serde(default)]
    descricao: String,
}

/// POST /lugares/{slug}: salva nome, @ e descrição. Trocar o @ não mexe nas histórias já escritas
/// (elas guardam o slug antigo; o editor mostra "lugar não encontrado" e dá para escolher de novo).
pub async fn salvar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, Form(f): Form<FormFicha>) -> Response {
    let Some(l) = buscar(&st, s.familia_id, &slug).await else { return Redirect::to("/lugares").into_response() };
    let nome = f.nome.trim();
    if nome.is_empty() {
        return detalhe_com(&st, s, &slug, Some("O nome não pode ficar vazio.".into())).await;
    }
    let pedido = util::slug(f.arroba.trim().trim_start_matches('@'));
    let novo = if pedido.is_empty() || pedido == l.slug {
        l.slug.clone()
    } else {
        match slug_livre(&st, s.familia_id, &pedido, Some(l.id)).await {
            Ok(x) => x,
            Err(e) => return erro_interno(e),
        }
    };
    let r = sqlx::query("UPDATE conteudo.lugares SET nome = $2, slug = $3, descricao = $4, atualizado_em = now() WHERE id = $1")
        .bind(l.id)
        .bind(nome)
        .bind(&novo)
        .bind(f.descricao.trim())
        .execute(&st.db)
        .await;
    match r {
        Ok(_) => Redirect::to(&format!("/lugares/{novo}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /lugares/{slug}/apagar
pub async fn apagar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    let _ = sqlx::query("DELETE FROM conteudo.lugares WHERE familia_id = $1 AND slug = $2").bind(s.familia_id).bind(&slug).execute(&st.db).await;
    Redirect::to("/lugares").into_response()
}

/// POST /lugares/{slug}/refs: envia imagens prontas.
pub async fn enviar_refs(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, mp: Multipart) -> Response {
    let Some(l) = buscar(&st, s.familia_id, &slug).await else { return Redirect::to("/lugares").into_response() };
    let (_, arquivos) = match crate::personagens::ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    let pasta = format!("familias/{}/lugares/{}", s.familia_slug, l.slug);
    let mut ids = Vec::new();
    for bytes in &arquivos {
        let Some((mime, ext)) = midia::tipo_imagem(bytes) else {
            return detalhe_com(&st, s, &slug, Some("Só imagens PNG, JPG, WebP ou GIF.".into())).await;
        };
        match midia::salvar(&st, s.familia_id, &pasta, bytes, mime, ext).await {
            Ok(id) => ids.push(id),
            Err(e) => return erro_interno(e),
        }
    }
    let _ = sqlx::query("UPDATE conteudo.lugares SET referencias = referencias || $2, atualizado_em = now() WHERE id = $1")
        .bind(l.id)
        .bind(&ids)
        .execute(&st.db)
        .await;
    Redirect::to(&format!("/lugares/{slug}")).into_response()
}

/// POST /lugares/{slug}/tirar-ref/{ref}
pub async fn tirar_ref(State(st): State<AppState>, s: Sessao, Path((slug, ref_id)): Path<(String, Uuid)>) -> Response {
    let _ = sqlx::query(
        "UPDATE conteudo.lugares SET referencias = array_remove(referencias, $3), atualizado_em = now() WHERE familia_id = $1 AND slug = $2",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(ref_id)
    .execute(&st.db)
    .await;
    Redirect::to(&format!("/lugares/{slug}")).into_response()
}

async fn pedir_desenho(st: &AppState, s: &Sessao, slug: &str) -> Result<(), String> {
    let l = buscar(st, s.familia_id, slug).await.ok_or("Lugar não encontrado.")?;
    let pedido = fila_imagens::Pedido {
        alvo: "lugar",
        historia: None,
        chave: None,
        variante: None,
        lugar: Some(l.id),
        prompt: prompt_lugar(&l.nome, &l.descricao, !l.referencias.is_empty()),
        refs: l.referencias.iter().take(2).copied().collect(),
        pasta: format!("lugares/{}", l.slug),
        recortar: false,
    };
    fila_imagens::pedir(st, s.familia_id, &s.familia_slug, vec![pedido], fila_imagens::AVULSO).await.map(|_| ()).map_err(|e| format!("{e:#}"))
}

/// POST /lugares/{slug}/gerar: pede um desenho do lugar à fila de imagens.
pub async fn gerar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    match pedir_desenho(&st, &s, &slug).await {
        Ok(()) => Redirect::to(&format!("/lugares/{slug}")).into_response(),
        Err(msg) => detalhe_com(&st, s, &slug, Some(msg)).await,
    }
}

/// Lugares para a IA: os citados no briefing e os que a história já usa.
pub fn para_ia(lugares: &[Lugar], slugs: &[String]) -> Vec<Value> {
    lugares
        .iter()
        .filter(|l| slugs.contains(&l.slug))
        .map(|l| json!({ "slug": l.slug, "nome": l.nome, "descricao": l.descricao }))
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn prompt_do_lugar_sem_personagens() {
        let p = prompt_lugar("Castelo", "castelo de areia com 3 torres e bandeira azul", false);
        assert!(p.contains("SEM nenhum personagem"));
        assert!(p.contains("3 torres"));
        assert!(!p.contains("anexadas"));
        assert!(prompt_lugar("Castelo", "", true).contains("anexadas"));
    }
}

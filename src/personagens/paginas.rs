//! Lista, ficha e página de detalhe do personagem.


use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::Sessao, util,
    web::{erro_interno, render},
    AppState,
};

use super::{nome_tipo, prompt_model_sheet, TIPOS};

// ───────────── lista ─────────────

#[derive(sqlx::FromRow)]
struct ItemLista {
    slug: String,
    nome: String,
    tipo: String,
    variante: Option<String>,
    miniatura: Option<Uuid>,
}

impl ItemLista {
    fn tipo_nome(&self) -> &'static str {
        nome_tipo(&self.tipo)
    }
}

#[derive(Template)]
#[template(path = "personagens.html")]
struct PaginaLista {
    s: Sessao,
    itens: Vec<ItemLista>,
    tipos: Vec<(&'static str, &'static str)>,
    erro: Option<String>,
}

pub(super) async fn lista_com(st: &AppState, s: Sessao, erro: Option<String>) -> Response {
    // Miniatura: primeira referência da variante atual (ou de qualquer variante).
    let itens = sqlx::query_as::<_, ItemLista>(
        "SELECT p.slug, p.nome, p.tipo, va.nome AS variante,
                COALESCE(va.referencias[1],
                         (SELECT v.referencias[1] FROM conteudo.variantes v
                           WHERE v.personagem_id = p.id AND cardinality(v.referencias) > 0
                           ORDER BY v.criado_em LIMIT 1)) AS miniatura
           FROM conteudo.personagens p
           LEFT JOIN conteudo.variantes va ON va.id = p.variante_atual
          WHERE p.familia_id = $1
          ORDER BY p.nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    match itens {
        Ok(itens) => render(PaginaLista { s, itens, tipos: TIPOS.to_vec(), erro }),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn lista(State(st): State<AppState>, s: Sessao) -> Response {
    lista_com(&st, s, None).await
}

// ───────────── criar / editar ficha ─────────────

#[derive(Deserialize)]
pub struct FormFicha {
    nome: String,
    tipo: String,
    #[serde(default)]
    apelidos: String,
    #[serde(default)]
    aparencia: String,
    #[serde(default)]
    personalidade: String,
    #[serde(default)]
    jeito_falar: String,
    /// Novo @ para citar (só na ficha de um personagem que já existe).
    #[serde(default)]
    arroba: String,
}

impl FormFicha {
    fn ficha(&self) -> serde_json::Value {
        let apelidos: Vec<&str> = self.apelidos.split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
        json!({
            "apelidos": apelidos,
            "aparencia": self.aparencia.trim(),
            "personalidade": self.personalidade.trim(),
            "jeito_falar": self.jeito_falar.trim(),
        })
    }
    fn tipo_valido(&self) -> bool {
        TIPOS.iter().any(|(k, _)| *k == self.tipo)
    }
}

/// POST /personagens (multipart): ficha + imagem opcional do personagem (vira a variante "Normal").
pub async fn criar(State(st): State<AppState>, s: Sessao, mp: axum::extract::Multipart) -> Response {
    let (campos, arquivos) = match super::variantes::ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return lista_com(&st, s, Some(msg)).await,
    };
    let c = |k: &str| campos.get(k).cloned().unwrap_or_default();
    let f = FormFicha {
        nome: c("nome"),
        tipo: c("tipo"),
        apelidos: c("apelidos"),
        aparencia: c("aparencia"),
        personalidade: c("personalidade"),
        jeito_falar: c("jeito_falar"),
        arroba: String::new(),
    };
    let nome = f.nome.trim().to_string();
    let slug = util::slug(&nome);
    if slug.is_empty() || !f.tipo_valido() {
        return lista_com(&st, s, Some("Preencha o nome e o tipo.".into())).await;
    }
    let r = sqlx::query(
        "INSERT INTO conteudo.personagens (familia_id, slug, nome, tipo, ficha) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&nome)
    .bind(&f.tipo)
    .bind(f.ficha())
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => {
            if !arquivos.is_empty() {
                if let Err(msg) = super::variantes::criar_normal(&st, &s, &slug, &arquivos).await {
                    return detalhe_com(&st, s, &slug, Some(msg)).await;
                }
            }
            Redirect::to(&format!("/personagens/{slug}")).into_response()
        }
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            lista_com(&st, s, Some(format!("Já existe um personagem chamado {nome}."))).await
        }
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn salvar_ficha(
    State(st): State<AppState>,
    s: Sessao,
    Path(slug): Path<String>,
    Form(f): Form<FormFicha>,
) -> Response {
    if f.nome.trim().is_empty() || !f.tipo_valido() {
        return detalhe_com(&st, s, &slug, Some("Preencha o nome e o tipo.".into())).await;
    }
    let r = sqlx::query(
        // `||` mescla: o formulário não traz tudo (ex.: "fotos" do personagem criado com IA) e o resto fica.
        "UPDATE conteudo.personagens SET nome = $3, tipo = $4, ficha = COALESCE(ficha, '{}'::jsonb) || $5, atualizado_em = now()
          WHERE familia_id = $1 AND slug = $2",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(f.nome.trim())
    .bind(&f.tipo)
    .bind(f.ficha())
    .execute(&st.db)
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    // Trocou o @: leva junto histórias, briefings e estantes.
    let novo = util::slug(f.arroba.trim_start_matches('@'));
    if !novo.is_empty() && novo != slug {
        if let Err(e) = super::renomear::renomear_personagem(&st, s.familia_id, &slug, &novo).await {
            return detalhe_com(&st, s, &slug, Some(format!("A ficha foi salva, mas não troquei o @: {e:#}"))).await;
        }
        return Redirect::to(&format!("/personagens/{novo}")).into_response();
    }
    Redirect::to(&format!("/personagens/{slug}")).into_response()
}

// ───────────── detalhe ─────────────

#[derive(sqlx::FromRow)]
pub(super) struct Personagem {
    pub(super) id: Uuid,
    slug: String,
    nome: String,
    tipo: String,
    ficha: serde_json::Value,
    pub(super) variante_atual: Option<Uuid>,
}

impl Personagem {
    fn campo(&self, k: &str) -> String {
        self.ficha.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    }
    fn apelidos(&self) -> String {
        self.ficha
            .get("apelidos")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    }
}

#[derive(sqlx::FromRow)]
struct Variante {
    id: Uuid,
    slug: String,
    nome: String,
    forma: Option<String>,
    estilo: Option<String>,
    descricao: Option<String>,
    referencias: Vec<Uuid>,
    arquivada: bool,
}

#[derive(Template)]
#[template(path = "personagem.html")]
struct PaginaPersonagem {
    s: Sessao,
    p: Personagem,
    variantes: Vec<Variante>,
    tipos: Vec<(&'static str, &'static str)>,
    erro: Option<String>,
    imagem_api: bool,
    /// Outros personagens da família (slug, nome), para "virar variante de".
    outros: Vec<(String, String)>,
}

impl PaginaPersonagem {
    fn fotos(&self) -> Vec<String> {
        self.p.ficha["fotos"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect()
    }
    fn prompt_kit(&self, v: &Variante) -> String {
        let aparencia = v.descricao.clone().unwrap_or_else(|| self.p.campo("aparencia"));
        prompt_model_sheet(&self.p.nome, &aparencia, v.estilo.as_deref(), !self.fotos().is_empty() || !v.referencias.is_empty())
    }
    fn foto_na_variante(&self, v: &Variante, foto: &str) -> bool {
        v.referencias.iter().any(|r| r.to_string() == foto)
    }
    fn eh_atual(&self, v: &Variante) -> bool {
        self.p.variante_atual == Some(v.id)
    }
}

pub(super) async fn buscar(st: &AppState, familia_id: Uuid, slug: &str) -> Result<Option<Personagem>, sqlx::Error> {
    sqlx::query_as::<_, Personagem>(
        "SELECT id, slug, nome, tipo, ficha, variante_atual
           FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2",
    )
    .bind(familia_id)
    .bind(slug)
    .fetch_optional(&st.db)
    .await
}

pub(super) async fn detalhe_com(st: &AppState, s: Sessao, slug: &str, erro: Option<String>) -> Response {
    let p = match buscar(st, s.familia_id, slug).await {
        Ok(Some(p)) => p,
        Ok(None) => return Redirect::to("/personagens").into_response(),
        Err(e) => return erro_interno(e.into()),
    };
    let variantes = sqlx::query_as::<_, Variante>(
        "SELECT id, slug, nome, forma, estilo, descricao, referencias, arquivada
           FROM conteudo.variantes WHERE personagem_id = $1 ORDER BY arquivada, criado_em",
    )
    .bind(p.id)
    .fetch_all(&st.db)
    .await;
    match variantes {
        Ok(variantes) => {
            let imagem_api = crate::imagem::disponivel(st, s.familia_id).await;
            let outros: Vec<(String, String)> =
                sqlx::query_as("SELECT slug, nome FROM conteudo.personagens WHERE familia_id = $1 AND id <> $2 ORDER BY nome")
                    .bind(s.familia_id)
                    .bind(p.id)
                    .fetch_all(&st.db)
                    .await
                    .unwrap_or_default();
            render(PaginaPersonagem { s, p, variantes, tipos: TIPOS.to_vec(), erro, imagem_api, outros })
        }
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn detalhe(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    detalhe_com(&st, s, &slug, None).await
}


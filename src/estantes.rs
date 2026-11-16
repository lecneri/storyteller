//! Estantes: coleções de histórias com padrões (personagens, variantes, estilo).

use askama::Template;
use axum::{
    extract::{Path, RawForm, State},
    response::{IntoResponse, Redirect, Response},
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    util,
    web::{erro_interno, render},
    AppState,
};

// ───────────── tela inicial: estantes com as histórias ─────────────

#[derive(sqlx::FromRow)]
pub struct EstanteResumo {
    pub id: Uuid,
    pub slug: String,
    pub nome: String,
    pub padroes: Value,
    /// [{id, titulo, faixa, paginas, capa}]
    pub historias: Value,
}

impl EstanteResumo {
    pub fn lista(&self) -> Vec<CartaoHistoria> {
        self.historias
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|h| CartaoHistoria {
                        id: h["id"].as_str().unwrap_or_default().to_string(),
                        titulo: h["titulo"].as_str().unwrap_or_default().to_string(),
                        faixa: h["faixa"].as_str().unwrap_or_default().to_string(),
                        paginas: h["paginas"].as_i64().unwrap_or(0),
                        capa: h["capa"].as_str().map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn estilo(&self) -> String {
        self.padroes["estilo"].as_str().unwrap_or_default().to_string()
    }
    pub fn instrucoes(&self) -> String {
        self.padroes["instrucoes"].as_str().unwrap_or_default().to_string()
    }
    pub fn resumo_personagens(&self) -> String {
        self.padroes["personagens"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| match p["variante"].as_str().filter(|v| !v.is_empty()) {
                        Some(v) => format!("@{}:{}", p["slug"].as_str().unwrap_or_default(), v),
                        None => format!("@{}", p["slug"].as_str().unwrap_or_default()),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    }
}

pub struct CartaoHistoria {
    pub id: String,
    pub titulo: String,
    pub faixa: String,
    pub paginas: i64,
    pub capa: Option<String>,
}

pub async fn estantes_da_familia(st: &AppState, familia_id: Uuid) -> Result<Vec<EstanteResumo>, sqlx::Error> {
    sqlx::query_as::<_, EstanteResumo>(
        "SELECT e.id, e.slug, e.nome, e.padroes,
                COALESCE((SELECT json_agg(json_build_object(
                            'id', h.id, 'titulo', h.titulo, 'faixa', h.faixa,
                            'paginas', jsonb_array_length(h.conteudo->'paginas'),
                            'capa', COALESCE(h.capa::text,
                                (SELECT pg->'midia'->>'id' FROM jsonb_array_elements(h.conteudo->'paginas') pg
                                  WHERE pg->'midia'->>'tipo' LIKE 'image/%' LIMIT 1)))
                            ORDER BY eh.ordem, h.criado_em)
                            FROM conteudo.estante_historias eh
                            JOIN conteudo.historias h ON h.id = eh.historia_id
                           WHERE eh.estante_id = e.id), '[]')::jsonb AS historias
           FROM conteudo.estantes e
          WHERE e.familia_id = $1
          ORDER BY e.ordem, e.nome",
    )
    .bind(familia_id)
    .fetch_all(&st.db)
    .await
}

#[derive(sqlx::FromRow)]
pub struct PersonagemOpcao {
    pub slug: String,
    pub nome: String,
    /// [{slug, nome}]
    pub variantes: Value,
}

impl PersonagemOpcao {
    pub fn opcoes(&self) -> Vec<(String, String)> {
        self.variantes
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| (v["slug"].as_str().unwrap_or_default().into(), v["nome"].as_str().unwrap_or_default().into()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

async fn personagens_opcoes(st: &AppState, familia_id: Uuid) -> Result<Vec<PersonagemOpcao>, sqlx::Error> {
    sqlx::query_as::<_, PersonagemOpcao>(
        "SELECT p.slug, p.nome,
                COALESCE((SELECT json_agg(json_build_object('slug', v.slug, 'nome', v.nome) ORDER BY v.criado_em)
                            FROM conteudo.variantes v WHERE v.personagem_id = p.id AND NOT v.arquivada), '[]')::jsonb AS variantes
           FROM conteudo.personagens p
          WHERE p.familia_id = $1 ORDER BY p.nome",
    )
    .bind(familia_id)
    .fetch_all(&st.db)
    .await
}

#[derive(sqlx::FromRow)]
struct SemEstante {
    id: Uuid,
    titulo: String,
}

#[derive(Template)]
#[template(path = "estantes.html")]
struct PaginaEstantes {
    s: Sessao,
    estantes: Vec<EstanteResumo>,
    soltas: Vec<SemEstante>,
    personagens: Vec<PersonagemOpcao>,
    erro: Option<String>,
    /// Histórias de famílias conectadas: (id, título, família, capa)
    compartilhadas: Vec<(Uuid, String, String, Option<String>)>,
}

impl PaginaEstantes {
    fn marcado(&self, _slug: &str) -> bool {
        false
    }
    fn variante_escolhida(&self, _slug: &str, _variante: &str) -> bool {
        false
    }
}

async fn inicio_com(st: &AppState, s: Sessao, erro: Option<String>) -> Response {
    let soltas = sqlx::query_as::<_, SemEstante>(
        "SELECT h.id, h.titulo FROM conteudo.historias h
          WHERE h.familia_id = $1
            AND NOT EXISTS (SELECT 1 FROM conteudo.estante_historias eh WHERE eh.historia_id = h.id)
          ORDER BY h.atualizado_em DESC",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    match (estantes_da_familia(st, s.familia_id).await, soltas, personagens_opcoes(st, s.familia_id).await) {
        (Ok(estantes), Ok(soltas), Ok(personagens)) => {
            let compartilhadas = crate::partilha::compartilhadas_comigo(st, s.familia_id).await;
            render(PaginaEstantes { s, estantes, soltas, personagens, erro, compartilhadas })
        }
        (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => erro_interno(e.into()),
    }
}

pub async fn inicio(State(st): State<AppState>, s: Sessao) -> Response {
    inicio_com(&st, s, None).await
}

// ───────────── criar / editar estante ─────────────

/// Lê o formulário (campos repetidos: personagem=slug e variante_<slug>=...).
fn ler_padroes(form: &[(String, String)]) -> (String, Value) {
    let campo = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
    let personagens: Vec<Value> = form
        .iter()
        .filter(|(n, _)| n == "personagem")
        .map(|(_, slug)| {
            let variante = campo(&format!("variante_{slug}"));
            json!({ "slug": slug, "variante": Some(variante).filter(|v| !v.is_empty()) })
        })
        .collect();
    (campo("nome"), json!({
            "estilo": Some(campo("estilo")).filter(|e| !e.is_empty()),
            "instrucoes": Some(campo("instrucoes")).filter(|e| !e.is_empty()),
            "personagens": personagens,
        }))
}

pub async fn criar(State(st): State<AppState>, s: Sessao, RawForm(corpo): RawForm) -> Response {
    let form: Vec<(String, String)> = serde_urlencoded::from_bytes(&corpo).unwrap_or_default();
    let (nome, padroes) = ler_padroes(&form);
    let slug = util::slug(&nome);
    if slug.is_empty() {
        return inicio_com(&st, s, Some("Dê um nome para a estante.".into())).await;
    }
    let r = sqlx::query(
        "INSERT INTO conteudo.estantes (familia_id, slug, nome, padroes, ordem)
         VALUES ($1, $2, $3, $4, (SELECT COALESCE(max(ordem), 0) + 1 FROM conteudo.estantes WHERE familia_id = $1))",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&nome)
    .bind(padroes)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to("/").into_response(),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            inicio_com(&st, s, Some(format!("Já existe a estante {nome}."))).await
        }
        Err(e) => erro_interno(e.into()),
    }
}

#[derive(Template)]
#[template(path = "estante.html")]
struct PaginaEstante {
    s: Sessao,
    e: EstanteResumo,
    personagens: Vec<PersonagemOpcao>,
    outras: Vec<SemEstante>,
}

impl PaginaEstante {
    fn marcado(&self, slug: &str) -> bool {
        self.e.padroes["personagens"].as_array().is_some_and(|a| a.iter().any(|p| p["slug"] == slug))
    }
    fn variante_escolhida(&self, slug: &str, variante: &str) -> bool {
        self.e.padroes["personagens"]
            .as_array()
            .and_then(|a| a.iter().find(|p| p["slug"] == slug))
            .is_some_and(|p| p["variante"] == variante)
    }
}

pub async fn abrir(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    let estantes = match estantes_da_familia(&st, s.familia_id).await {
        Ok(e) => e,
        Err(e) => return erro_interno(e.into()),
    };
    let Some(e) = estantes.into_iter().find(|e| e.slug == slug) else {
        return Redirect::to("/").into_response();
    };
    let outras = sqlx::query_as::<_, SemEstante>(
        "SELECT h.id, h.titulo FROM conteudo.historias h
          WHERE h.familia_id = $1
            AND NOT EXISTS (SELECT 1 FROM conteudo.estante_historias eh WHERE eh.historia_id = h.id AND eh.estante_id = $2)
          ORDER BY h.titulo",
    )
    .bind(s.familia_id)
    .bind(e.id)
    .fetch_all(&st.db)
    .await;
    match (personagens_opcoes(&st, s.familia_id).await, outras) {
        (Ok(personagens), Ok(outras)) => render(PaginaEstante { s, e, personagens, outras }),
        (Err(e), _) | (_, Err(e)) => erro_interno(e.into()),
    }
}

pub async fn salvar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, RawForm(corpo): RawForm) -> Response {
    let form: Vec<(String, String)> = serde_urlencoded::from_bytes(&corpo).unwrap_or_default();
    let (nome, padroes) = ler_padroes(&form);
    if nome.is_empty() {
        return Redirect::to(&format!("/estantes/{slug}")).into_response();
    }
    let r = sqlx::query("UPDATE conteudo.estantes SET nome = $3, padroes = $4 WHERE familia_id = $1 AND slug = $2")
        .bind(s.familia_id)
        .bind(&slug)
        .bind(&nome)
        .bind(padroes)
        .execute(&st.db)
        .await;
    match r {
        Ok(_) => Redirect::to(&format!("/estantes/{slug}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// Coloca uma história (que já existe) nesta estante.
pub async fn incluir(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, RawForm(corpo): RawForm) -> Response {
    let form: Vec<(String, String)> = serde_urlencoded::from_bytes(&corpo).unwrap_or_default();
    let Some(historia) = form.iter().find(|(k, _)| k == "historia").and_then(|(_, v)| v.parse::<Uuid>().ok()) else {
        return Redirect::to(&format!("/estantes/{slug}")).into_response();
    };
    let r = vincular(&st, s.familia_id, &slug, historia).await;
    match r {
        Ok(()) => Redirect::to(&format!("/estantes/{slug}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// Liga a história à estante (as duas precisam ser da família). Se a história ainda não tem
/// estante principal, esta vira a principal.
pub async fn vincular(st: &AppState, familia_id: Uuid, estante_slug: &str, historia: Uuid) -> Result<(), sqlx::Error> {
    let mut tx = st.db.begin().await?;
    let estante: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM conteudo.estantes WHERE familia_id = $1 AND slug = $2",
    )
    .bind(familia_id)
    .bind(estante_slug)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(estante) = estante else { return Ok(()) };
    let n = sqlx::query(
        "INSERT INTO conteudo.estante_historias (estante_id, historia_id, ordem)
         SELECT $1, h.id, (SELECT COALESCE(max(ordem), 0) + 1 FROM conteudo.estante_historias WHERE estante_id = $1)
           FROM conteudo.historias h WHERE h.id = $2 AND h.familia_id = $3
         ON CONFLICT DO NOTHING",
    )
    .bind(estante)
    .bind(historia)
    .bind(familia_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n > 0 {
        sqlx::query("UPDATE conteudo.historias SET estante_principal = COALESCE(estante_principal, $1) WHERE id = $2")
            .bind(estante)
            .bind(historia)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

pub async fn retirar(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, historia)): Path<(String, Uuid)>,
) -> Response {
    let r = async {
        let mut tx = st.db.begin().await?;
        let estante: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM conteudo.estantes WHERE familia_id = $1 AND slug = $2")
                .bind(s.familia_id)
                .bind(&slug)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(estante) = estante {
            sqlx::query("DELETE FROM conteudo.estante_historias WHERE estante_id = $1 AND historia_id = $2")
                .bind(estante)
                .bind(historia)
                .execute(&mut *tx)
                .await?;
            // Se era a principal, passa para outra estante em que a história esteja (ou nenhuma).
            sqlx::query(
                "UPDATE conteudo.historias SET estante_principal =
                   (SELECT estante_id FROM conteudo.estante_historias WHERE historia_id = $1 LIMIT 1)
                  WHERE id = $1 AND estante_principal = $2",
            )
            .bind(historia)
            .bind(estante)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
    .await;
    match r {
        Ok(()) => Redirect::to(&format!("/estantes/{slug}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

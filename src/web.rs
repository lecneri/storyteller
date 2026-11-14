//! Páginas e formulários.

use askama::Template;
use axum::{
    body::Body,
    extract::{Form, Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{self, Sessao},
    util, AppState,
};

pub fn erro_interno(e: anyhow::Error) -> Response {
    tracing::error!("erro interno: {e:#}");
    (StatusCode::INTERNAL_SERVER_ERROR, "Algo deu errado. Tente de novo.").into_response()
}

/// Erro que a pessoa pode corrigir (422 + {"erro": "..."}), para as rotas chamadas pelo JavaScript.
pub fn erro_json(msg: impl std::fmt::Display) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(serde_json::json!({ "erro": msg.to_string() }))).into_response()
}

/// A história existe e é desta família?
pub async fn historia_da_familia(st: &AppState, familia: Uuid, historia: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conteudo.historias WHERE id = $1 AND familia_id = $2)")
        .bind(historia)
        .bind(familia)
        .fetch_one(&st.db)
        .await
}

pub fn render<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// Proteção básica contra CSRF: um POST precisa vir do próprio site.
pub async fn checa_origem(req: Request<Body>, next: Next) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok());
        let origem = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|o| o.to_str().ok())
            .map(|o| o.split("://").nth(1).unwrap_or(o));
        if let (Some(h), Some(o)) = (host, origem) {
            if h != o {
                return (StatusCode::FORBIDDEN, "Origem não permitida").into_response();
            }
        }
    }
    next.run(req).await
}

// ───────────── entrar / sair ─────────────

#[derive(Template)]
#[template(path = "entrar.html")]
struct PaginaEntrar {
    erro: Option<String>,
    email: String,
}

pub async fn pagina_entrar(State(st): State<AppState>) -> Response {
    // Banco vazio: primeiro acesso vai para a instalação.
    if crate::auth::sem_usuarios(&st.db).await {
        return Redirect::to("/instalar").into_response();
    }
    render(PaginaEntrar { erro: None, email: String::new() })
}

#[derive(Deserialize)]
pub struct FormEntrar {
    email: String,
    senha: String,
}

pub async fn entrar(State(st): State<AppState>, jar: CookieJar, Form(f): Form<FormEntrar>) -> Response {
    let usuario: Option<(Uuid, String)> = match sqlx::query_as(
        "SELECT id, senha_hash FROM acesso.usuarios WHERE lower(email) = lower($1)",
    )
    .bind(f.email.trim())
    .fetch_optional(&st.db)
    .await
    {
        Ok(u) => u,
        Err(e) => return erro_interno(e.into()),
    };

    let ok = usuario.as_ref().is_some_and(|(_, hash)| auth::senha_confere(&f.senha, hash));
    let Some((usuario_id, _)) = usuario.filter(|_| ok) else {
        return render(PaginaEntrar { erro: Some("E-mail ou senha incorretos.".into()), email: f.email });
    };

    match auth::criar_sessao(&st.db, usuario_id, None).await {
        Ok(token) => (jar.add(auth::cookie_sessao(token, st.cookie_seguro)), Redirect::to("/")).into_response(),
        Err(e) => erro_interno(e),
    }
}

pub async fn sair(State(st): State<AppState>, jar: CookieJar) -> Response {
    if let Some(c) = jar.get(auth::COOKIE_SESSAO) {
        if let Err(e) = auth::apagar_sessao(&st.db, c.value()).await {
            return erro_interno(e);
        }
    }
    (jar.remove(Cookie::build(auth::COOKIE_SESSAO).path("/")), Redirect::to("/entrar")).into_response()
}

// ───────────── painel da família ─────────────

#[derive(sqlx::FromRow)]
struct Leitor {
    id: Uuid,
    nome: String,
    faixa: String,
    perfil: serde_json::Value,
}

impl Leitor {
    /// Sem lista = vê todas as estantes.
    fn ve_todas(&self) -> bool {
        !self.perfil["estantes"].is_array()
    }
    fn ve(&self, slug: &str) -> bool {
        self.perfil["estantes"].as_array().is_none_or(|a| a.iter().any(|v| v == slug))
    }
}

#[derive(Template)]
#[template(path = "painel.html")]
struct PaginaPainel {
    s: Sessao,
    leitores: Vec<Leitor>,
    estantes: Vec<(String, String)>,
    tem_pin: bool,
    erro: Option<String>,
    ok: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct AvisosPainel {
    erro: Option<String>,
    ok: Option<String>,
}

async fn painel_com(st: &AppState, s: Sessao, erro: Option<String>, ok: Option<String>) -> Response {
    let leitores = sqlx::query_as::<_, Leitor>(
        "SELECT id, nome, faixa, perfil FROM acesso.leitores WHERE familia_id = $1 ORDER BY nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    let estantes = sqlx::query_as::<_, (String, String)>(
        "SELECT slug, nome FROM conteudo.estantes WHERE familia_id = $1 ORDER BY ordem, nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    let tem_pin = sqlx::query_scalar::<_, bool>("SELECT pin_leitura_hash IS NOT NULL FROM acesso.familias WHERE id = $1")
        .bind(s.familia_id)
        .fetch_one(&st.db)
        .await;
    match (leitores, estantes, tem_pin) {
        (Ok(leitores), Ok(estantes), Ok(tem_pin)) => render(PaginaPainel { s, leitores, estantes, tem_pin, erro, ok }),
        (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => erro_interno(e.into()),
    }
}

pub async fn painel(
    State(st): State<AppState>,
    s: Sessao,
    axum::extract::Query(q): axum::extract::Query<AvisosPainel>,
) -> Response {
    let erro = q.erro.map(|e| match e.as_str() {
        "pin" => "Defina o PIN dos pais antes de entrar no modo leitura.".to_string(),
        "pin-formato" => "O PIN precisa ter de 4 a 8 números.".to_string(),
        _ => "Algo deu errado.".to_string(),
    });
    let ok = q.ok.map(|o| match o.as_str() {
        "pin" => "PIN salvo ✓".to_string(),
        _ => "Salvo ✓".to_string(),
    });
    painel_com(&st, s, erro, ok).await
}

#[derive(Deserialize)]
pub struct FormLeitor {
    nome: String,
    faixa: String,
}

const FAIXAS: [&str; 4] = ["2-3", "4-6", "7-9", "10-12"];

pub async fn criar_leitor(State(st): State<AppState>, s: Sessao, Form(f): Form<FormLeitor>) -> Response {
    let nome = f.nome.trim();
    let slug = util::slug(nome);
    if slug.is_empty() || !FAIXAS.contains(&f.faixa.as_str()) {
        return painel_com(&st, s, Some("Preencha o nome e escolha a faixa etária.".into()), None).await;
    }
    let r = sqlx::query("INSERT INTO acesso.leitores (familia_id, slug, nome, faixa) VALUES ($1, $2, $3, $4)")
        .bind(s.familia_id)
        .bind(&slug)
        .bind(nome)
        .bind(&f.faixa)
        .execute(&st.db)
        .await;
    match r {
        Ok(_) => Redirect::to("/leitores").into_response(),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            painel_com(&st, s, Some(format!("Já existe um leitor chamado {nome}.")), None).await
        }
        Err(e) => erro_interno(e.into()),
    }
}

/// Versão dos arquivos estáticos (muda a cada vez que o servidor sobe), usada como `?v=` nos
/// <script>/<link> para o navegador e a Cloudflare não ficarem com JS/CSS antigos depois de um deploy.
pub fn versao() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default().to_string()
    })
}

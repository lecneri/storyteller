//! Modo leitura (criança): só estantes e player; sair exige o PIN dos pais.

use askama::Template;
use axum::{
    body::Body,
    extract::{Form, Path, Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{self, Sessao},
    estantes,
    web::{erro_interno, render},
    AppState,
};

/// Caminhos liberados durante o modo leitura.
fn liberado(metodo: &Method, caminho: &str) -> bool {
    let get = metodo == Method::GET || metodo == Method::HEAD;
    (get && (caminho == "/crianca"
        || caminho.starts_with("/ler/")
        || caminho.starts_with("/midia/")
        || caminho == "/saude"
        || caminho.ends_with(".css")
        || caminho.ends_with(".js")
        || caminho.ends_with(".svg")))
        || caminho == "/sair-leitura"
        || caminho.starts_with("/p/")
        // áudio gerado na hora pelo player
        || (metodo == Method::POST && caminho.starts_with("/historias/") && caminho.ends_with("/bloco-audio"))
}

/// Middleware: com o modo leitura ativo, tudo fora da lista volta para /crianca.
pub async fn trava(axum::extract::State(st): axum::extract::State<AppState>, req: Request<Body>, next: Next) -> Response {
    let caminho = req.uri().path().to_string();
    if !liberado(req.method(), &caminho) {
        let jar = CookieJar::from_headers(req.headers());
        if let Some(token) = jar.get(auth::COOKIE_SESSAO).map(|c| c.value().to_owned()) {
            let leitor: Option<Option<Uuid>> =
                sqlx::query_scalar("SELECT leitor_id FROM acesso.sessoes WHERE token_hash = $1 AND expira_em > now()")
                    .bind(auth::hash_token(&token))
                    .fetch_optional(&st.db)
                    .await
                    .unwrap_or(None);
            if matches!(leitor, Some(Some(_))) {
                return Redirect::to("/crianca").into_response();
            }
        }
    }
    next.run(req).await
}

// ───────────── tela da criança ─────────────

#[derive(Template)]
#[template(path = "crianca.html")]
struct PaginaCrianca {
    nome: String,
    estantes: Vec<estantes::EstanteResumo>,
}

pub async fn inicio(State(st): State<AppState>, s: Sessao) -> Response {
    let Some((leitor_id, nome)) = s.leitor.clone() else {
        return Redirect::to("/").into_response();
    };
    let perfil: serde_json::Value = sqlx::query_scalar("SELECT perfil FROM acesso.leitores WHERE id = $1")
        .bind(leitor_id)
        .fetch_one(&st.db)
        .await
        .unwrap_or_default();
    // Estantes visíveis para este leitor (sem configuração = todas).
    let visiveis: Option<Vec<String>> = perfil["estantes"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
    match estantes::estantes_da_familia(&st, s.familia_id).await {
        Ok(lista) => render(PaginaCrianca {
            nome,
            estantes: lista
                .into_iter()
                .filter(|e| visiveis.as_ref().is_none_or(|v| v.contains(&e.slug)))
                .filter(|e| !e.lista().is_empty())
                .collect(),
        }),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── entrar / sair ─────────────

/// POST /leitores/{id}/modo-leitura: um adulto entrega o aparelho para a criança.
pub async fn entrar(State(st): State<AppState>, s: Sessao, jar: CookieJar, Path(leitor): Path<Uuid>) -> Response {
    let tem_pin: bool = match sqlx::query_scalar("SELECT pin_leitura_hash IS NOT NULL FROM acesso.familias WHERE id = $1")
        .bind(s.familia_id)
        .fetch_one(&st.db)
        .await
    {
        Ok(t) => t,
        Err(e) => return erro_interno(e.into()),
    };
    if !tem_pin {
        return Redirect::to("/leitores?erro=pin").into_response();
    }
    let Some(token) = jar.get(auth::COOKIE_SESSAO).map(|c| c.value().to_owned()) else {
        return Redirect::to("/entrar").into_response();
    };
    let r = sqlx::query(
        "UPDATE acesso.sessoes SET leitor_id = l.id
           FROM acesso.leitores l
          WHERE acesso.sessoes.token_hash = $1 AND l.id = $2 AND l.familia_id = $3",
    )
    .bind(auth::hash_token(&token))
    .bind(leitor)
    .bind(s.familia_id)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to("/crianca").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[derive(Template)]
#[template(path = "sair_leitura.html")]
struct PaginaSair {
    erro: Option<&'static str>,
}

pub async fn pagina_sair() -> Response {
    render(PaginaSair { erro: None })
}

#[derive(Deserialize)]
pub struct FormPin {
    pin: String,
}

pub async fn sair(State(st): State<AppState>, s: Sessao, jar: CookieJar, Form(f): Form<FormPin>) -> Response {
    let hash: Option<String> = sqlx::query_scalar("SELECT pin_leitura_hash FROM acesso.familias WHERE id = $1")
        .bind(s.familia_id)
        .fetch_one(&st.db)
        .await
        .unwrap_or(None);
    // No máximo 10 tentativas por família a cada 10 minutos (o PIN tem só 4 dígitos).
    let chave = format!("pin:{}", s.familia_id);
    if !crate::limite::tentar(&chave, 10, std::time::Duration::from_secs(600)) {
        return render(PaginaSair { erro: Some("Muitas tentativas. Espere uns 10 minutos.") });
    }
    let ok = hash.is_some_and(|h| auth::senha_confere(f.pin.trim(), &h));
    if ok {
        crate::limite::zerar(&chave);
    }
    if !ok {
        // Atrasa tentativas erradas (desestimula "chutar" o PIN).
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        return render(PaginaSair { erro: Some("PIN errado.") });
    }
    let Some(token) = jar.get(auth::COOKIE_SESSAO).map(|c| c.value().to_owned()) else {
        return Redirect::to("/entrar").into_response();
    };
    match sqlx::query("UPDATE acesso.sessoes SET leitor_id = NULL WHERE token_hash = $1")
        .bind(auth::hash_token(&token))
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/leitores").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── configurações (adultos) ─────────────

#[derive(Deserialize)]
pub struct FormDefinirPin {
    pin: String,
}

/// POST /familia/pin (só admin da família)
pub async fn definir_pin(State(st): State<AppState>, s: Sessao, Form(f): Form<FormDefinirPin>) -> Response {
    if !s.admin_familia() {
        return Redirect::to("/leitores").into_response();
    }
    let pin = f.pin.trim();
    if pin.len() < 4 || pin.len() > 8 || !pin.chars().all(|c| c.is_ascii_digit()) {
        return Redirect::to("/leitores?erro=pin-formato").into_response();
    }
    let hash = match auth::gerar_hash_senha(pin) {
        Ok(h) => h,
        Err(e) => return erro_interno(e),
    };
    match sqlx::query("UPDATE acesso.familias SET pin_leitura_hash = $2 WHERE id = $1")
        .bind(s.familia_id)
        .bind(hash)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/leitores?ok=pin").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /leitores/{id}/estantes: quais estantes o leitor vê (campos repetidos "estante").
pub async fn salvar_estantes(
    State(st): State<AppState>,
    s: Sessao,
    Path(leitor): Path<Uuid>,
    axum::extract::RawForm(corpo): axum::extract::RawForm,
) -> Response {
    let form: Vec<(String, String)> = serde_urlencoded::from_bytes(&corpo).unwrap_or_default();
    let todas = form.iter().any(|(k, _)| k == "todas");
    let lista: Vec<String> = form.into_iter().filter(|(k, _)| k == "estante").map(|(_, v)| v).collect();
    let r = if todas {
        sqlx::query("UPDATE acesso.leitores SET perfil = perfil - 'estantes' WHERE id = $1 AND familia_id = $2")
            .bind(leitor)
            .bind(s.familia_id)
            .execute(&st.db)
            .await
    } else {
        sqlx::query("UPDATE acesso.leitores SET perfil = jsonb_set(perfil, '{estantes}', $3) WHERE id = $1 AND familia_id = $2")
            .bind(leitor)
            .bind(s.familia_id)
            .bind(serde_json::json!(lista))
            .execute(&st.db)
            .await
    };
    match r {
        Ok(_) => Redirect::to("/leitores?ok=estantes").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

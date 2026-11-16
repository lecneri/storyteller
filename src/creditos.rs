//! Página pública de créditos: músicas da plataforma (com o crédito que a licença pede), vozes, IAs e software.

use askama::Template;
use axum::{extract::State, response::Response};

use crate::{
    web::{erro_interno, render},
    AppState,
};

#[derive(Template)]
#[template(path = "creditos.html")]
struct PaginaCreditos {
    /// (título, clima, crédito)
    musicas: Vec<(String, String, String)>,
}

impl PaginaCreditos {
    fn clima(&self, id: &str) -> &'static str {
        crate::musica::CLIMAS.iter().find(|(c, _)| *c == id).map(|(_, n)| *n).unwrap_or("")
    }
}

/// GET /creditos (sem login: vale para quem lê pelo link público também).
pub async fn ver(State(st): State<AppState>) -> Response {
    let musicas: Vec<(String, String, Option<String>)> =
        match sqlx::query_as("SELECT titulo, clima, credito FROM midia.musicas_modelo ORDER BY clima, titulo").fetch_all(&st.db).await {
            Ok(m) => m,
            Err(e) => return erro_interno(e.into()),
        };
    render(PaginaCreditos {
        musicas: musicas.into_iter().map(|(t, c, cr)| (t, c, cr.unwrap_or_default())).collect(),
    })
}

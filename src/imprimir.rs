//! Imprimir a história: uma página por folha (imagem em cima, texto embaixo).
//! Vídeo e GIF não imprimem: no lugar deles vai um quadro (o primeiro ou um aleatório).

use std::process::Stdio;

use askama::Template;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use rand::Rng;
use serde::Deserialize;
use tokio::process::Command;
use uuid::Uuid;

use crate::{
    auth::Sessao,
    web::{erro_interno, render},
    AppState,
};

#[derive(Template)]
#[template(path = "imprimir.html")]
struct PaginaImprimir {
    titulo: String,
    dados: String,
}

/// GET /imprimir/{id}
pub async fn pagina(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if !crate::partilha::pode_ler_historia(&st, s.familia_id, id).await {
        return Redirect::to("/").into_response();
    }
    match crate::player::manifest(&st, id, s.familia_id).await {
        Ok(Some((titulo, dados))) => render(PaginaImprimir { titulo, dados: dados.to_string().replace("</", "<\\/") }),
        Ok(None) => Redirect::to("/").into_response(),
        Err(e) => erro_interno(e),
    }
}

#[derive(Deserialize)]
pub struct PedidoQuadro {
    #[serde(default)]
    modo: String,
    /// Só para o navegador não reaproveitar o quadro aleatório anterior.
    #[serde(default)]
    #[allow(dead_code)]
    n: Option<String>,
}

/// Duração (s) de um vídeo ou GIF.
async fn duracao(caminho: &std::path::Path) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(caminho)
        .output()
        .await
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|d: &f64| d.is_finite() && *d > 0.0)
}

/// Momento do quadro: 0 no "primeiro"; no "aleatorio", um ponto qualquer longe do fim.
pub fn instante(modo: &str, duracao: Option<f64>, sorteio: f64) -> f64 {
    match (modo, duracao) {
        ("aleatorio", Some(d)) => (d * 0.9 * sorteio.clamp(0.0, 1.0) * 1000.0).round() / 1000.0,
        _ => 0.0,
    }
}

/// GET /midia/{id}/quadro?modo=primeiro|aleatorio — JPEG de um quadro do vídeo/GIF.
/// Imagem parada volta como ela mesma.
pub async fn quadro(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Query(q): Query<PedidoQuadro>) -> Response {
    let (caminho, tipo) = match crate::midia::permitido(&st, &s, id).await {
        Ok(Some(l)) => l,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e),
    };
    if !(tipo.starts_with("video/") || tipo == "image/gif") {
        return Redirect::to(&format!("/midia/{id}")).into_response();
    }
    let arquivo = st.data_dir.join(caminho);
    let t = instante(&q.modo, duracao(&arquivo).await, rand::thread_rng().gen());
    let saida = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&arquivo)
        .args(["-ss", &format!("{t:.3}"), "-frames:v", "1", "-f", "image2", "-c:v", "mjpeg", "-q:v", "2", "pipe:1"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(30), saida).await {
        Ok(Ok(o)) if o.status.success() && !o.stdout.is_empty() => (
            [(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "private, no-store")],
            Body::from(o.stdout),
        )
            .into_response(),
        Ok(Ok(o)) => {
            tracing::warn!("ffmpeg não tirou o quadro: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("?"));
            StatusCode::UNPROCESSABLE_ENTITY.into_response()
        }
        Ok(Err(e)) => erro_interno(e.into()),
        Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn instante_do_quadro() {
        assert_eq!(instante("primeiro", Some(10.0), 0.7), 0.0);
        assert_eq!(instante("aleatorio", Some(10.0), 0.5), 4.5);
        assert_eq!(instante("aleatorio", Some(10.0), 1.0), 9.0); // nunca no finalzinho
        assert_eq!(instante("aleatorio", None, 0.5), 0.0); // sem duração: o primeiro
    }
}

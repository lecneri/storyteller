//! Memória da história: resumo curto usado quando outra história a cita com `#`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::Conteudo;
use crate::{auth::Sessao, ia, web::erro_interno, AppState};

/// Gera (pela IA) e grava a memória de uma história: um resumo curto com os fatos importantes.
pub async fn gerar_memoria(st: &AppState, historia: Uuid) -> anyhow::Result<String> {
    let (familia, titulo, conteudo): (Uuid, String, Value) =
        sqlx::query_as("SELECT familia_id, titulo, conteudo FROM conteudo.historias WHERE id = $1")
            .bind(historia)
            .fetch_one(&st.db)
            .await?;
    let c: Conteudo = serde_json::from_value(conteudo).unwrap_or_default();
    let texto: Vec<String> = c
        .paginas
        .iter()
        .flat_map(|p| p.blocos.iter().map(|b| format!("[{}] {}", b.nome.as_deref().unwrap_or(&b.quem), b.texto)))
        .collect();
    let pedido = ia::PedidoTexto {
        sistema: "Você resume histórias infantis para servir de memória em histórias seguintes. Responda só com o JSON pedido.".into(),
        imagens: Vec::new(),
        prompt: format!(
            "Resuma a história \"{titulo}\" em 3 a 6 frases curtas com os fatos que importam para continuações: \
             quem participou, o que aconteceu, quem venceu ou o que foi descoberto, objetos e lugares novos, piadas internas.\n\n{}",
            texto.join("\n")
        ),
        schema: json!({ "type": "object", "additionalProperties": false, "required": ["memoria"],
                        "properties": { "memoria": { "type": "string" } } }),
    };
    let r = ia::gerar_texto(st, familia, &pedido).await?;
    let memoria = r["memoria"].as_str().unwrap_or_default().trim().to_string();
    if memoria.is_empty() {
        anyhow::bail!("a IA não devolveu a memória");
    }
    sqlx::query("UPDATE conteudo.historias SET memoria = $2 WHERE id = $1")
        .bind(historia)
        .bind(&memoria)
        .execute(&st.db)
        .await?;
    Ok(memoria)
}

/// POST /historias/{id}/memoria: gera de novo a memória pela IA.
pub async fn gerar_memoria_rota(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let dona: Option<Uuid> = sqlx::query_scalar("SELECT familia_id FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    if dona != Some(s.familia_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match gerar_memoria(&st, id).await {
        Ok(m) => Json(json!({ "memoria": m })).into_response(),
        Err(e) => crate::web::erro_json(format!("{e:#}")),
    }
}

#[derive(Deserialize)]
pub struct SalvarMemoria {
    memoria: String,
}

/// POST /historias/{id}/memoria/salvar: a família edita a memória.
pub async fn salvar_memoria(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(m): Json<SalvarMemoria>) -> Response {
    match sqlx::query("UPDATE conteudo.historias SET memoria = $3 WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .bind(m.memoria.trim())
        .execute(&st.db)
        .await
    {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}


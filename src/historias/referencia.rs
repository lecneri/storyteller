//! Referência da história (resumo curto usado quando outra história a cita com `#`) e memória (marcações, ex.: palavras aprendidas).

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

/// Gera (pela IA) e grava a referência de uma história: um resumo curto com os fatos importantes.
pub async fn gerar_referencia(st: &AppState, historia: Uuid) -> anyhow::Result<String> {
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
        sistema: "Você resume histórias para servir de referência em histórias seguintes. Responda só com o JSON pedido.".into(),
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
    let referencia = r["memoria"].as_str().unwrap_or_default().trim().to_string();
    if referencia.is_empty() {
        anyhow::bail!("a IA não devolveu a referência");
    }
    sqlx::query("UPDATE conteudo.historias SET referencia = $2 WHERE id = $1")
        .bind(historia)
        .bind(&referencia)
        .execute(&st.db)
        .await?;
    Ok(referencia)
}

/// POST /historias/{id}/referencia: gera de novo a referência pela IA.
pub async fn gerar_referencia_rota(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let dona: Option<Uuid> = sqlx::query_scalar("SELECT familia_id FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    if dona != Some(s.familia_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match gerar_referencia(&st, id).await {
        Ok(m) => Json(json!({ "referencia": m })).into_response(),
        Err(e) => crate::web::erro_json(format!("{e:#}")),
    }
}

#[derive(Deserialize)]
pub struct SalvarReferencia {
    referencia: String,
}

/// POST /historias/{id}/referencia/salvar: a família edita a referência.
pub async fn salvar_referencia(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(m): Json<SalvarReferencia>) -> Response {
    match sqlx::query("UPDATE conteudo.historias SET referencia = $3 WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .bind(m.referencia.trim())
        .execute(&st.db)
        .await
    {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}


/// Itens de memória limpos: sem espaços sobrando, sem vazios nem repetidos, até 200 itens de 200 letras.
pub fn itens_memoria(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in v.as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
        let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
        if !t.is_empty() && !out.iter().any(|o| o.to_lowercase() == t.to_lowercase()) && out.len() < 200 {
            out.push(t);
        }
    }
    out
}

#[derive(Deserialize)]
pub struct SalvarMemoria {
    itens: Value,
}

/// POST /historias/{id}/memoria/salvar: a família edita a memória (lista de marcações).
pub async fn salvar_memoria(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(m): Json<SalvarMemoria>) -> Response {
    let itens = itens_memoria(&m.itens);
    match sqlx::query("UPDATE conteudo.historias SET memoria = $3 WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .bind(json!(itens))
        .execute(&st.db)
        .await
    {
        Ok(_) => Json(json!({ "ok": true, "itens": itens })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn memoria_sem_vazios_nem_repetidos() {
        let v = json!(["  apple  (maçã) ", "", "Apple (maçã)", "red", 3]);
        assert_eq!(itens_memoria(&v), vec!["apple (maçã)", "red"]);
    }
}

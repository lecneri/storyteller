//! Ajuda da IA para editar uma história pronta: a família pede ("deixe a página 3 mais engraçada")
//! e a IA devolve a história inteira ajustada. Nada é gravado aqui: o editor mostra o resultado
//! e a família salva se gostar (ou desfaz).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::geracao::{preparar, prompt_para_copiar, Preparo};
use super::prompt::{normalizar, schema_historia};
use super::{Conteudo, Pagina};
use crate::{
    auth::Sessao,
    ia, jobs,
    web::{erro_interno, erro_json, historia_da_familia},
    AppState,
};

/// O que a IA vê de cada página (sem mídia, som etc.).
const CAMPOS_PAGINA: [&str; 5] = ["cena", "personagens", "secundarios", "animacao", "blocos"];

/// Schema da história com "origem" em cada página (para manter a imagem da página em que ela se baseia).
pub fn schema_edicao() -> Value {
    let mut s = schema_historia();
    let itens = &mut s["properties"]["paginas"]["items"];
    itens["properties"]["origem"] = json!({
        "type": "integer",
        "description": "número (1, 2, 3...) da página da HISTÓRIA ATUAL em que esta página se baseia; 0 se for uma página nova"
    });
    if let Some(r) = itens["required"].as_array_mut() {
        r.push(json!("origem"));
    }
    s
}

/// Pedido à IA: o mesmo contexto da geração (personagens, vozes, estante...) + a história atual + o ajuste pedido.
pub fn pedido_edicao(p: &Preparo, titulo: &str, conteudo: &Value, pedido: &str) -> ia::PedidoTexto {
    let paginas: Vec<Value> = conteudo["paginas"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, pg)| {
            let mut m = serde_json::Map::new();
            m.insert("numero".into(), json!(i + 1));
            for c in CAMPOS_PAGINA {
                if let Some(v) = pg.get(c) {
                    m.insert(c.into(), v.clone());
                }
            }
            Value::Object(m)
        })
        .collect();
    let atual = json!({ "titulo": titulo, "faixa": p.faixa, "secundarios": conteudo["secundarios"], "paginas": paginas });
    let prompt = format!(
        "{}\n\n\
         ATENÇÃO: esta história JÁ EXISTE. Não escreva outra: ajuste a HISTÓRIA ATUAL abaixo conforme o PEDIDO DE AJUSTE.\n\
         - Mude só o que o pedido exigir; o resto fica igual (mesmos textos, cenas, personagens, vozes e secundários).\n\
         - Devolva a história INTEIRA já ajustada, no mesmo formato. Em cada página, \"origem\" é o número da página atual \
           em que ela se baseia (0 se for uma página nova). Páginas que o pedido mandar tirar simplesmente não aparecem.\n\
         - Mantenha a faixa \"{}\", salvo se o pedido mudar a idade.\n\n\
         HISTÓRIA ATUAL:\n{}\n\n\
         PEDIDO DE AJUSTE:\n{}\n",
        p.pedido.prompt.trim_end(),
        p.faixa,
        serde_json::to_string_pretty(&atual).unwrap_or_default(),
        pedido.trim()
    );
    ia::PedidoTexto { sistema: p.pedido.sistema.clone(), prompt, schema: schema_edicao(), imagens: Vec::new() }
}

/// Resposta da IA → { titulo, conteudo } pronto para o editor, com a mídia de cada página de origem.
pub fn processar(p: &Preparo, atual: &Value, resposta: &Value) -> anyhow::Result<Value> {
    let mut c: Conteudo = serde_json::from_value(json!({ "paginas": resposta["paginas"], "secundarios": resposta["secundarios"] }))?;
    let antigas: Vec<Pagina> = serde_json::from_value(atual["paginas"].clone()).unwrap_or_default();
    let origens: Vec<i64> = resposta["paginas"].as_array().into_iter().flatten().map(|pg| pg["origem"].as_i64().unwrap_or(0)).collect();
    for (pg, o) in c.paginas.iter_mut().zip(origens) {
        if let Some(a) = usize::try_from(o).ok().filter(|o| *o >= 1).and_then(|o| antigas.get(o - 1)) {
            pg.midia = a.midia.clone();
            pg.som = a.som;
        }
    }
    let c = normalizar(c, &p.narracao, &p.citados, &p.todos);
    if c.paginas.is_empty() {
        anyhow::bail!("a IA devolveu uma história vazia");
    }
    let titulo = resposta["titulo"].as_str().map(str::trim).filter(|t| !t.is_empty());
    Ok(json!({ "titulo": titulo, "conteudo": c }))
}

async fn atual(st: &AppState, id: Uuid) -> anyhow::Result<(String, Value)> {
    Ok(sqlx::query_as("SELECT titulo, conteudo FROM conteudo.historias WHERE id = $1").bind(id).fetch_one(&st.db).await?)
}

/// Job "editar_historia": chama a IA e guarda o resultado no job (o editor busca).
pub async fn executar_ajuda(st: &AppState, payload: &Value) -> anyhow::Result<Value> {
    let id: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let pedido = payload["pedido"].as_str().unwrap_or_default();
    let p = preparar(st, id).await?;
    let (titulo, conteudo) = atual(st, id).await?;
    let resposta = ia::gerar_texto(st, p.familia_id, &pedido_edicao(&p, &titulo, &conteudo, pedido)).await?;
    processar(&p, &conteudo, &resposta)
}

#[derive(Deserialize)]
pub struct PedidoAjuda {
    pedido: String,
    /// "prompt": só devolve o prompt para copiar; senão a IA do app faz (na fila).
    #[serde(default)]
    modo: String,
}

/// POST /historias/{id}/ajuda-ia — salve a história antes (a IA lê a versão gravada).
pub async fn pedir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(f): Json<PedidoAjuda>) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let pedido = f.pedido.trim();
    if pedido.is_empty() {
        return erro_json("Escreva o que a IA deve mudar.");
    }
    if pedido.chars().count() > 2000 {
        return erro_json("Pedido grande demais (até 2000 letras).");
    }
    if f.modo == "prompt" {
        let r = async {
            let p = preparar(&st, id).await?;
            let (titulo, conteudo) = atual(&st, id).await?;
            anyhow::Ok(prompt_para_copiar(&pedido_edicao(&p, &titulo, &conteudo, pedido)))
        }
        .await;
        return match r {
            Ok(prompt) => Json(json!({ "prompt": prompt })).into_response(),
            Err(e) => erro_interno(e),
        };
    }
    match jobs::enfileirar(&st, "editar_historia", s.familia_id, json!({ "historia_id": id, "pedido": pedido })).await {
        Ok(job) => Json(json!({ "job": job })).into_response(),
        Err(e) => erro_interno(e),
    }
}

/// GET /historias/{id}/ajuda-ia/{job} — estado do pedido; quando pronto, traz { titulo, conteudo }.
pub async fn ver(State(st): State<AppState>, s: Sessao, Path((id, job)): Path<(Uuid, i64)>) -> Response {
    let linha: Result<Option<(String, Option<String>, Option<Value>)>, _> = sqlx::query_as(
        "SELECT estado, erro, resultado FROM jobs.fila
          WHERE id = $1 AND familia_id = $2 AND tipo = 'editar_historia' AND payload->>'historia_id' = $3",
    )
    .bind(job)
    .bind(s.familia_id)
    .bind(id.to_string())
    .fetch_optional(&st.db)
    .await;
    match linha {
        Ok(Some((estado, erro, resultado))) => Json(json!({ "estado": estado, "erro": erro, "resultado": resultado })).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[derive(Deserialize)]
pub struct ImportarAjuda {
    texto: String,
}

/// POST /historias/{id}/ajuda-ia/importar — resposta de outra IA (colada ou de arquivo) → { titulo, conteudo }.
pub async fn importar_ajuda(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(f): Json<ImportarAjuda>) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let resposta = match ia::extrair_json(&f.texto) {
        Ok(v) if v["paginas"].is_array() => v,
        Ok(_) => return erro_json("O JSON não tem \"paginas\". Copie a resposta inteira da IA."),
        Err(_) => return erro_json("Não achei um JSON na resposta. Copie a resposta inteira da IA (de { até })."),
    };
    let r = async {
        let p = preparar(&st, id).await?;
        let (_, conteudo) = atual(&st, id).await?;
        processar(&p, &conteudo, &resposta)
    }
    .await;
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => erro_json(format!("Não deu para usar a resposta: {e}")),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn schema_de_edicao_pede_a_origem() {
        let s = schema_edicao();
        let itens = &s["properties"]["paginas"]["items"];
        assert!(itens["properties"]["origem"].is_object());
        assert!(itens["required"].as_array().unwrap().contains(&json!("origem")));
        // O schema da geração continua sem "origem".
        assert!(schema_historia()["properties"]["paginas"]["items"]["properties"]["origem"].is_null());
    }
}

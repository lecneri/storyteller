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

use super::geracao::{preparar_com, prompt_para_copiar, Preparo};
use super::prompt::{normalizar, schema_historia};
use super::{Bloco, Conteudo, Pagina};
use crate::{
    auth::Sessao,
    ia, jobs,
    web::{erro_interno, erro_json, historia_da_familia},
    AppState,
};

/// O que a IA vê de cada página (sem mídia, som etc.).
const CAMPOS_PAGINA: [&str; 6] = ["cena", "personagens", "secundarios", "lugar", "animacao", "blocos"];

/// Schema da história com "origem" em cada página (para manter a imagem da página em que ela se baseia).
pub fn schema_edicao() -> Value {
    let mut s = schema_historia();
    // A ajuda não mexe na memória (ela é gravada na geração ou editada à mão).
    if let Some(p) = s["properties"].as_object_mut() {
        p.remove("memoria");
    }
    if let Some(r) = s["required"].as_array_mut() {
        r.retain(|x| x != "memoria");
    }
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
    let atual = json!({
        "titulo": titulo, "faixa": p.faixa, "secundarios": conteudo["secundarios"], "lugares": conteudo["lugares"], "paginas": paginas,
    });
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
    let mut c: Conteudo = serde_json::from_value(
        json!({ "paginas": resposta["paginas"], "secundarios": resposta["secundarios"], "lugares": resposta["lugares"] }),
    )?;
    let antigas: Vec<Pagina> = serde_json::from_value(atual["paginas"].clone()).unwrap_or_default();
    let origens: Vec<i64> = resposta["paginas"].as_array().into_iter().flatten().map(|pg| pg["origem"].as_i64().unwrap_or(0)).collect();
    herdar_da_origem(&mut c.paginas, &antigas, &origens);
    let mut c = normalizar(c, &p.narracao, &p.citados, &p.todos);
    super::prompt::ajustar_lugares(&mut c, &p.lugares);
    manter_gravacoes(&mut c, &antigas);
    if c.paginas.is_empty() {
        anyhow::bail!("a IA devolveu uma história vazia");
    }
    let titulo = resposta["titulo"].as_str().map(str::trim).filter(|t| !t.is_empty());
    Ok(json!({ "titulo": titulo, "conteudo": c }))
}

/// Cada página herda da página de origem o que não é texto: imagem/vídeo, som, chave e música.
/// Se a IA dividir uma página em várias, a troca de música fica só na primeira (a mídia vai para todas).
fn herdar_da_origem(paginas: &mut [Pagina], antigas: &[Pagina], origens: &[i64]) {
    let mut musica_usada: Vec<usize> = Vec::new();
    for (pg, o) in paginas.iter_mut().zip(origens) {
        let Some(i) = usize::try_from(*o).ok().filter(|o| *o >= 1).map(|o| o - 1) else { continue };
        let Some(a) = antigas.get(i) else { continue };
        pg.midia = a.midia.clone();
        pg.som = a.som;
        if !musica_usada.contains(&i) {
            pg.chave = a.chave.clone();
            pg.musica = a.musica.clone();
            musica_usada.push(i);
        }
    }
}

/// A fala que continua igual (mesmo personagem e mesmo texto) mantém a voz gravada.
pub fn manter_gravacoes(c: &mut Conteudo, antigas: &[Pagina]) {
    let gravadas: Vec<&Bloco> = antigas.iter().flat_map(|p| &p.blocos).filter(|b| b.gravacao.is_some()).collect();
    for b in c.paginas.iter_mut().flat_map(|p| &mut p.blocos) {
        if let Some(a) = gravadas.iter().find(|a| a.quem == b.quem && a.texto.trim() == b.texto.trim()) {
            b.gravacao = a.gravacao.clone();
        }
    }
}

async fn atual(st: &AppState, id: Uuid) -> anyhow::Result<(String, Value)> {
    Ok(sqlx::query_as("SELECT titulo, conteudo FROM conteudo.historias WHERE id = $1").bind(id).fetch_one(&st.db).await?)
}

/// Job "editar_historia": chama a IA e guarda o resultado no job (o editor busca).
pub async fn executar_ajuda(st: &AppState, payload: &Value) -> anyhow::Result<Value> {
    let id: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let pedido = payload["pedido"].as_str().unwrap_or_default();
    let p = preparar_com(st, id, pedido).await?;
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
            let p = preparar_com(&st, id, pedido).await?;
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
    /// O pedido de ajuda (para aceitar os personagens e variantes citados nele).
    #[serde(default)]
    pedido: String,
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
        Ok(_) => {
            let msg = "O JSON não tem \"paginas\". Copie a resposta inteira da IA.";
            crate::importacoes::falhou(&st, s.familia_id, Some(id), "ajuda", &f.texto, msg).await;
            return erro_json(msg);
        }
        Err(e) => {
            crate::importacoes::falhou(&st, s.familia_id, Some(id), "ajuda", &f.texto, &format!("{e:#}")).await;
            return erro_json(format!("Não achei um JSON válido na resposta ({e:#}). Copie a resposta inteira da IA (de {{ até }})."));
        }
    };
    let r = async {
        let p = preparar_com(&st, id, &f.pedido).await?;
        let (_, conteudo) = atual(&st, id).await?;
        processar(&p, &conteudo, &resposta)
    }
    .await;
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            crate::importacoes::falhou(&st, s.familia_id, Some(id), "ajuda", &f.texto, &format!("{e:#}")).await;
            erro_json(format!("Não deu para usar a resposta: {e}"))
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn pagina_herda_midia_e_musica_da_origem() {
        let antiga: Pagina = serde_json::from_value(json!({
            "chave": "k1", "som": false, "midia": { "id": "00000000-0000-0000-0000-000000000001", "tipo": "image/png" },
            "musica": { "modo": "silencio" }
        }))
        .unwrap();
        let mut novas = vec![Pagina::default(), Pagina::default(), Pagina::default()];
        herdar_da_origem(&mut novas, &[antiga], &[1, 1, 0]);
        assert_eq!(novas[0].musica.as_ref().map(|m| m.modo.as_str()), Some("silencio"));
        assert_eq!(novas[0].chave, "k1");
        assert_eq!(novas[0].som, Some(false));
        assert!(novas[1].midia.is_some() && novas[1].musica.is_none() && novas[1].chave.is_empty());
        assert!(novas[2].midia.is_none() && novas[2].musica.is_none());
    }

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

//! Fila de imagens: todo pedido de imagem (página, capa, desenho de variante) entra aqui e volta na hora.
//! Cada fonte (codex, agy, api) tem um trabalhador que desenha um item por vez, de qualquer família;
//! as fontes trabalham em paralelo. Pedido avulso passa na frente das páginas de um lote.
//! Imagem de história vai para a fonte da história (a mesma CLI desenha o livro todo); se ela falhar,
//! só aquele item tenta a próxima fonte da família.

use std::time::Duration;

use anyhow::bail;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{auth::Sessao, AppState};

pub const AVULSO: i32 = 1;
pub const LOTE: i32 = 0;

/// Um pedido de imagem.
pub struct Pedido {
    pub alvo: &'static str,
    pub historia: Option<Uuid>,
    pub chave: Option<String>,
    pub variante: Option<Uuid>,
    pub lugar: Option<Uuid>,
    pub prompt: String,
    pub refs: Vec<Uuid>,
    /// Pasta da mídia, dentro de familias/{slug}/.
    pub pasta: String,
    pub recortar: bool,
}

/// Fila onde o pedido entra: a da história (se ainda liberada), senão a primeira da família.
async fn escolher_fila(st: &AppState, familia: Uuid, historia: Option<Uuid>) -> anyhow::Result<String> {
    let filas = crate::imagem::filas(st, familia).await?;
    if filas.is_empty() {
        bail!("Sem IA de imagem para esta família: use o modo manual (copie o prompt e as referências).");
    }
    if let Some(h) = historia {
        let fonte: Option<String> = sqlx::query_scalar("SELECT fonte_imagem FROM conteudo.historias WHERE id = $1")
            .bind(h)
            .fetch_optional(&st.db)
            .await?
            .flatten();
        if let Some(f) = fonte.filter(|f| filas.contains(f)) {
            return Ok(f);
        }
    }
    Ok(filas[0].clone())
}

/// Põe os pedidos na fila (todos na mesma fila, com o mesmo lote se for mais de um). Devolve os ids.
pub async fn pedir(st: &AppState, familia: Uuid, familia_slug: &str, pedidos: Vec<Pedido>, prioridade: i32) -> anyhow::Result<Vec<i64>> {
    let Some(primeiro) = pedidos.first() else { return Ok(Vec::new()) };
    let fila = escolher_fila(st, familia, primeiro.historia).await?;
    let lote = (pedidos.len() > 1 || prioridade == LOTE).then(Uuid::new_v4);
    let mut tx = st.db.begin().await?;
    let mut ids = Vec::new();
    for p in pedidos {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO jobs.imagens (familia_id, familia_slug, alvo, historia_id, chave, variante_id, lote, prompt, refs, pasta,
                                       recortar, prioridade, fila, lugar_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) RETURNING id",
        )
        .bind(familia)
        .bind(familia_slug)
        .bind(p.alvo)
        .bind(p.historia)
        .bind(&p.chave)
        .bind(p.variante)
        .bind(lote)
        .bind(&p.prompt)
        .bind(&p.refs)
        .bind(&p.pasta)
        .bind(p.recortar)
        .bind(prioridade)
        .bind(&fila)
        .bind(p.lugar)
        .fetch_one(&mut *tx)
        .await?;
        ids.push(id);
    }
    tx.commit().await?;
    Ok(ids)
}

/// Itens que estavam desenhando quando o app caiu voltam para a fila (chamar antes dos trabalhadores).
pub async fn recuperar(st: &AppState) {
    if let Err(e) = sqlx::query("UPDATE jobs.imagens SET estado = 'pendente' WHERE estado = 'rodando'").execute(&st.db).await {
        tracing::error!("fila de imagens: falha ao recuperar: {e}");
    }
}

#[derive(sqlx::FromRow)]
struct Item {
    id: i64,
    familia_id: Uuid,
    familia_slug: String,
    alvo: String,
    historia_id: Option<Uuid>,
    chave: Option<String>,
    variante_id: Option<Uuid>,
    lugar_id: Option<Uuid>,
    prompt: String,
    refs: Vec<Uuid>,
    pasta: String,
    recortar: bool,
    tentadas: Vec<String>,
    erro: Option<String>,
    /// A família já disse "pode usar a API paga" para este item.
    pago_ok: bool,
}

/// Trabalhador de uma fila: um item por vez.
pub async fn trabalhador(st: AppState, fila: &'static str) {
    loop {
        let item = sqlx::query_as::<_, Item>(
            "UPDATE jobs.imagens SET estado = 'rodando', atualizado_em = now()
              WHERE id = (SELECT id FROM jobs.imagens WHERE estado = 'pendente' AND fila = $1
                           ORDER BY prioridade DESC, id FOR UPDATE SKIP LOCKED LIMIT 1)
              RETURNING id, familia_id, familia_slug, alvo, historia_id, chave, variante_id, lugar_id, prompt, refs, pasta, recortar,
                        tentadas, erro, pago_ok",
        )
        .bind(fila)
        .fetch_optional(&st.db)
        .await;
        match item {
            Ok(Some(item)) => executar(&st, fila, item).await,
            Ok(None) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(e) => {
                tracing::error!("fila de imagens {fila}: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn executar(st: &AppState, fila: &str, item: Item) {
    tracing::info!("imagem {} ({}) começou na fila {fila}", item.id, item.alvo);
    let refs = crate::imagem::carregar_refs(st, item.familia_id, &item.refs).await;
    let r = crate::imagem::gerar_na_fila(st, item.familia_id, &item.familia_slug, &item.pasta, &item.prompt, &refs, fila).await;
    let r = match r {
        Ok((mut midia, mut tipo)) => {
            if item.recortar {
                let pasta = format!("familias/{}/{}", item.familia_slug, item.pasta);
                let nova = crate::recorte::sem_fundo_se_der(st, item.familia_id, &pasta, midia).await;
                if nova != midia {
                    midia = nova;
                    tipo = "image/png".into();
                }
            }
            match aplicar(st, &item, fila, midia, &tipo).await {
                Ok(()) => sqlx::query(
                    "UPDATE jobs.imagens SET estado = 'feito', feita_por = $2, midia = $3, midia_tipo = $4, erro = NULL,
                            atualizado_em = now() WHERE id = $1",
                )
                .bind(item.id)
                .bind(fila)
                .bind(midia)
                .bind(&tipo)
                .execute(&st.db)
                .await
                .map(|_| ()),
                Err(e) => falhou(st, &item, fila, &format!("{e:#}")).await,
            }
        }
        Err(e) => falhou(st, &item, fila, &format!("{e:#}")).await,
    };
    if let Err(e) = r {
        tracing::error!("fila de imagens: não gravei o item {}: {e}", item.id);
    }
}

/// Falhou nesta fila: tenta a próxima fonte da família que ainda não tentou; sem mais nenhuma, fica com erro.
/// Se a próxima é a API paga (chaves), espera a família decidir ("aguardando"), salvo se ela já liberou.
async fn falhou(st: &AppState, item: &Item, fila: &str, erro: &str) -> Result<(), sqlx::Error> {
    tracing::warn!("imagem {} falhou na fila {fila}: {erro}", item.id);
    let mut tentadas = item.tentadas.clone();
    tentadas.push(fila.to_string());
    let erro = match &item.erro {
        Some(antes) => format!("{antes} | {}: {erro}", crate::imagem::nome_fila(fila)),
        None => format!("{}: {erro}", crate::imagem::nome_fila(fila)),
    };
    let filas = crate::imagem::filas(st, item.familia_id).await.unwrap_or_default();
    let proxima = filas.into_iter().find(|f| !tentadas.contains(f));
    let perguntar = proxima.as_deref() == Some("api") && !item.pago_ok && !pagar_sem_perguntar(st, item.familia_id).await;
    let novo_estado = match &proxima {
        None => "erro",
        Some(_) if perguntar => "aguardando",
        Some(_) => "pendente",
    };
    sqlx::query(
        "UPDATE jobs.imagens SET estado = $5, fila = COALESCE($2, fila), tentadas = $3, erro = $4, atualizado_em = now() WHERE id = $1",
    )
    .bind(item.id)
    .bind(proxima)
    .bind(&tentadas)
    .bind(&erro)
    .bind(novo_estado)
    .execute(&st.db)
    .await
    .map(|_| ())
}

/// A família liberou seguir para a API paga sem perguntar?
pub async fn pagar_sem_perguntar(st: &AppState, familia: Uuid) -> bool {
    sqlx::query_scalar("SELECT pagar_sem_perguntar FROM ia.config_familia WHERE familia_id = $1")
        .bind(familia)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

#[derive(Deserialize)]
pub struct Decisao {
    /// Itens (página do personagem/lugar) ou todos os da história.
    #[serde(default)]
    ids: Vec<i64>,
    #[serde(default)]
    historia: Option<Uuid>,
    /// "pagar" (segue para a API paga) ou "manual" (tira da fila).
    acao: String,
}

/// POST /imagens/decidir: o que fazer com as imagens que esperam a decisão sobre a API paga.
pub async fn decidir(State(st): State<AppState>, s: Sessao, Json(d): Json<Decisao>) -> Response {
    let sql = match d.acao.as_str() {
        "pagar" => "UPDATE jobs.imagens SET estado = 'pendente', pago_ok = true, atualizado_em = now()",
        "manual" => "UPDATE jobs.imagens SET estado = 'cancelado', atualizado_em = now()",
        _ => return crate::web::erro_json("Ação desconhecida."),
    };
    let r = sqlx::query(&format!(
        "{sql} WHERE familia_id = $1 AND estado = 'aguardando' AND (id = ANY($2) OR ($3::uuid IS NOT NULL AND historia_id = $3))"
    ))
    .bind(s.familia_id)
    .bind(&d.ids)
    .bind(d.historia)
    .execute(&st.db)
    .await;
    match r {
        Ok(r) => Json(json!({ "ok": true, "itens": r.rows_affected() })).into_response(),
        Err(e) => crate::web::erro_interno(e.into()),
    }
}

/// Põe a imagem pronta no lugar: na página (pela chave), na capa ou nas imagens da variante.
async fn aplicar(st: &AppState, item: &Item, fila: &str, midia: Uuid, tipo: &str) -> anyhow::Result<()> {
    match (item.alvo.as_str(), item.historia_id, item.variante_id) {
        ("pagina", Some(h), _) => {
            colocar_na_pagina(st, h, item.chave.as_deref().unwrap_or_default(), midia, tipo).await?;
        }
        ("capa", Some(h), _) => {
            sqlx::query("UPDATE conteudo.historias SET capa = $2 WHERE id = $1").bind(h).bind(midia).execute(&st.db).await?;
        }
        ("lugar", _, _) if item.lugar_id.is_some() => {
            sqlx::query("UPDATE conteudo.lugares SET referencias = array_append(referencias, $2), atualizado_em = now() WHERE id = $1")
                .bind(item.lugar_id)
                .bind(midia)
                .execute(&st.db)
                .await?;
        }
        ("variante", _, Some(v)) => {
            sqlx::query("UPDATE conteudo.variantes SET referencias = array_append(referencias, $2) WHERE id = $1")
                .bind(v)
                .bind(midia)
                .execute(&st.db)
                .await?;
        }
        _ => bail!("pedido de imagem sem destino"),
    }
    // A primeira fonte que desenha a história fica sendo a dela.
    if let Some(h) = item.historia_id {
        sqlx::query("UPDATE conteudo.historias SET fonte_imagem = $2 WHERE id = $1 AND fonte_imagem IS NULL")
            .bind(h)
            .bind(fila)
            .execute(&st.db)
            .await?;
    }
    Ok(())
}

/// Põe a imagem na página (pela chave), sem mexer no resto da história.
async fn colocar_na_pagina(st: &AppState, historia: Uuid, chave: &str, midia: Uuid, tipo: &str) -> anyhow::Result<()> {
    let mut tx = st.db.begin().await?;
    let mut conteudo: Value = sqlx::query_scalar("SELECT conteudo FROM conteudo.historias WHERE id = $1 FOR UPDATE")
        .bind(historia)
        .fetch_one(&mut *tx)
        .await?;
    let Some(pag) = conteudo["paginas"].as_array_mut().and_then(|ps| ps.iter_mut().find(|p| p["chave"] == chave)) else {
        return Ok(()); // a página foi apagada
    };
    pag["midia"] = json!({ "id": midia, "tipo": tipo });
    sqlx::query("UPDATE conteudo.historias SET conteudo = $2, atualizado_em = now() WHERE id = $1")
        .bind(historia)
        .bind(&conteudo)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Como um item aparece na tela.
#[derive(sqlx::FromRow)]
pub struct Situacao {
    pub id: i64,
    pub alvo: String,
    pub chave: Option<String>,
    pub variante_id: Option<Uuid>,
    pub lugar_id: Option<Uuid>,
    pub lote: Option<Uuid>,
    pub estado: String,
    pub fila: String,
    pub feita_por: Option<String>,
    pub midia: Option<Uuid>,
    pub midia_tipo: Option<String>,
    pub erro: Option<String>,
    /// Quantos estão na frente na mesma fila (contando o que está desenhando).
    pub na_frente: i64,
}

impl Situacao {
    pub fn ativo(&self) -> bool {
        self.estado == "pendente" || self.estado == "rodando" || self.estado == "aguardando"
    }
    /// "⏳ Na fila (2º)" / "🎨 Desenhando… (ChatGPT (Codex))".
    pub fn rotulo(&self) -> String {
        match self.estado.as_str() {
            "pendente" => format!("⏳ Na fila ({}º)", self.na_frente + 1),
            "rodando" => format!("🎨 Desenhando… ({})", crate::imagem::nome_fila(&self.fila)),
            "feito" => "✅ Pronta".into(),
            "cancelado" => "Cancelada".into(),
            "aguardando" => "💳 Esperando: usar a API paga?".into(),
            _ => {
                let e = self.erro.as_deref().unwrap_or("erro");
                let curto: String = e.chars().take(240).collect();
                format!("❌ Não saiu: {curto}{}", if curto.len() < e.len() { "…" } else { "" })
            }
        }
    }
    pub fn json(&self) -> Value {
        json!({
            "id": self.id, "alvo": self.alvo, "chave": self.chave, "variante": self.variante_id, "lugar": self.lugar_id, "lote": self.lote,
            "estado": self.estado, "fila": self.fila, "feita_por": self.feita_por,
            "midia": self.midia.map(|m| json!({ "id": m, "tipo": self.midia_tipo })),
            "erro": self.erro, "rotulo": self.rotulo(),
        })
    }
}

const SELECT_SITUACAO: &str = "SELECT i.id, i.alvo, i.chave, i.variante_id, i.lugar_id, i.lote, i.estado, i.fila, i.feita_por, i.midia, i.midia_tipo, i.erro,
        CASE WHEN i.estado = 'pendente' THEN (
          SELECT count(*) FROM jobs.imagens o WHERE o.fila = i.fila AND (o.estado = 'rodando' OR (o.estado = 'pendente'
             AND (o.prioridade > i.prioridade OR (o.prioridade = i.prioridade AND o.id < i.id))))) ELSE 0 END AS na_frente
   FROM jobs.imagens i";

/// Itens de uma história: os ativos, os que terminaram há pouco e todos de um lote que ainda está rodando
/// (senão, num lote longo, as primeiras prontas saíam da conta e o "x/30 prontas" diminuía).
pub async fn da_historia(st: &AppState, familia: Uuid, historia: Uuid) -> Vec<Situacao> {
    sqlx::query_as(&format!(
        "{SELECT_SITUACAO} WHERE i.familia_id = $1 AND i.historia_id = $2
            AND (i.estado IN ('pendente', 'rodando', 'aguardando') OR i.atualizado_em > now() - interval '30 minutes'
                 OR i.lote IN (SELECT a.lote FROM jobs.imagens a WHERE a.historia_id = $2 AND a.lote IS NOT NULL
                                  AND a.estado IN ('pendente', 'rodando', 'aguardando')))
          ORDER BY i.id"
    ))
    .bind(familia)
    .bind(historia)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default()
}

/// Itens das variantes de um personagem: os ativos e os que falharam há pouco.
pub async fn do_personagem(st: &AppState, familia: Uuid, personagem: Uuid) -> Vec<Situacao> {
    sqlx::query_as(&format!(
        "{SELECT_SITUACAO} JOIN conteudo.variantes v ON v.id = i.variante_id
          WHERE i.familia_id = $1 AND v.personagem_id = $2
            AND (i.estado IN ('pendente', 'rodando', 'aguardando') OR (i.estado = 'erro' AND i.atualizado_em > now() - interval '15 minutes'))
          ORDER BY i.id"
    ))
    .bind(familia)
    .bind(personagem)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default()
}

/// Desenhos de um lugar: os ativos e os que falharam há pouco.
pub async fn do_lugar(st: &AppState, familia: Uuid, lugar: Uuid) -> Vec<Situacao> {
    sqlx::query_as(&format!(
        "{SELECT_SITUACAO} WHERE i.familia_id = $1 AND i.lugar_id = $2
            AND (i.estado IN ('pendente', 'rodando', 'aguardando') OR (i.estado = 'erro' AND i.atualizado_em > now() - interval '15 minutes'))
          ORDER BY i.id"
    ))
    .bind(familia)
    .bind(lugar)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default()
}

#[derive(Deserialize)]
pub struct Ids {
    #[serde(default)]
    ids: String,
}

/// GET /imagens/fila?ids=1,2: situação de itens da família (a página do personagem acompanha por aqui).
pub async fn estado(State(st): State<AppState>, s: Sessao, Query(q): Query<Ids>) -> Response {
    let ids: Vec<i64> = q.ids.split(',').filter_map(|x| x.trim().parse().ok()).take(50).collect();
    let itens: Vec<Situacao> = sqlx::query_as(&format!("{SELECT_SITUACAO} WHERE i.familia_id = $1 AND i.id = ANY($2) ORDER BY i.id"))
        .bind(s.familia_id)
        .bind(&ids)
        .fetch_all(&st.db)
        .await
        .unwrap_or_default();
    Json(json!({ "itens": itens.iter().map(Situacao::json).collect::<Vec<_>>() })).into_response()
}

/// Resumo para o Sistema: quantos esperando e desenhando em cada fila.
pub async fn resumo(st: &AppState) -> Vec<(String, i64, i64)> {
    let linhas: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT fila, count(*) FILTER (WHERE estado = 'pendente'), count(*) FILTER (WHERE estado = 'rodando')
           FROM jobs.imagens WHERE estado IN ('pendente', 'rodando') GROUP BY fila",
    )
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    crate::imagem::todas_filas()
        .into_iter()
        .map(|f| {
            let (p, r) = linhas.iter().find(|l| l.0 == f).map_or((0, 0), |l| (l.1, l.2));
            (crate::imagem::nome_fila(f).to_string(), p, r)
        })
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn item(estado: &str, na_frente: i64) -> Situacao {
        Situacao {
            id: 1, alvo: "pagina".into(), chave: None, variante_id: None, lugar_id: None, lote: None, estado: estado.into(), fila: "codex".into(),
            feita_por: None, midia: None, midia_tipo: None, erro: Some("sem cota".into()), na_frente,
        }
    }

    #[test]
    fn rotulo_mostra_a_posicao_e_quem_desenha() {
        assert_eq!(item("pendente", 0).rotulo(), "⏳ Na fila (1º)");
        assert_eq!(item("pendente", 2).rotulo(), "⏳ Na fila (3º)");
        assert_eq!(item("rodando", 0).rotulo(), "🎨 Desenhando… (ChatGPT (Codex))");
        assert_eq!(item("erro", 0).rotulo(), "❌ Não saiu: sem cota");
        assert!(item("rodando", 0).ativo() && !item("feito", 0).ativo());
    }
}

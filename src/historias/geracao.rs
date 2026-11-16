//! Job "gerar_historia": monta o pedido, chama a IA e grava a história.

use serde_json::{json, Value};
use uuid::Uuid;

use super::deteccao::{citar_historias, detectar, personagens_da_familia};
use super::prompt::{montar_prompt, normalizar, schema_historia, SISTEMA};
use super::{gerar_memoria, Conteudo, NaCena, FAIXAS};
use crate::{ia, util, AppState};

#[derive(sqlx::FromRow)]
struct HistoriaGerar {
    familia_id: Uuid,
    briefing: Option<String>,
    faixa: String,
    narracao: String,
    padroes: Option<Value>,
}

/// Tudo o que a geração precisa antes de chamar a IA (e para gravar a resposta depois).
pub struct Preparo {
    pub familia_id: Uuid,
    faixa: String,
    narracao: String,
    citados: Vec<NaCena>,
    todos: Vec<super::deteccao::PersonagemIA>,
    continua_de: Vec<Uuid>,
    pub pedido: ia::PedidoTexto,
}

/// Monta o pedido à IA a partir do briefing, da estante, dos personagens e das histórias citadas.
pub async fn preparar(st: &AppState, id: Uuid) -> anyhow::Result<Preparo> {
    let h = sqlx::query_as::<_, HistoriaGerar>(
        "SELECT h.familia_id, h.briefing, h.faixa, h.narracao, e.padroes
           FROM conteudo.historias h
           LEFT JOIN conteudo.estantes e ON e.id = h.estante_principal
          WHERE h.id = $1",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;

    let todos = personagens_da_familia(st, h.familia_id).await?;
    let briefing = h.briefing.unwrap_or_default();
    let mut citados = detectar(&briefing, &todos);
    // Padrões da estante: seus personagens entram sempre, com a variante da estante
    // (a menos que o briefing peça outra com @slug:variante).
    let padroes = h.padroes.unwrap_or(Value::Null);
    for p in padroes["personagens"].as_array().into_iter().flatten() {
        let slug = p["slug"].as_str().unwrap_or_default();
        let variante = p["variante"].as_str().filter(|v| !v.is_empty()).map(String::from);
        if !todos.iter().any(|t| t.slug == slug) {
            continue;
        }
        match citados.iter_mut().find(|c| c.slug == slug) {
            Some(c) if c.variante.is_none() => c.variante = variante,
            Some(_) => {}
            None => citados.push(NaCena { slug: slug.into(), variante, voz: None, misto: false, tambem: Vec::new() }),
        }
    }
    if citados.is_empty() {
        // Sem ninguém citado nem estante: a IA pode usar todos os personagens da família.
        citados = todos.iter().map(|p| NaCena { slug: p.slug.clone(), variante: None, voz: None, misto: false, tambem: Vec::new() }).collect();
    }
    let estilo = padroes["estilo"].as_str().unwrap_or_default();
    let instrucoes = padroes["instrucoes"].as_str().unwrap_or_default();

    // Histórias citadas com #: entram pela memória (resumo), não pelo texto inteiro.
    let mut memorias = Vec::new();
    let mut continua_de = Vec::new();
    for slug in citar_historias(&briefing) {
        let achada: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
            "SELECT id, titulo, memoria FROM conteudo.historias WHERE familia_id = $1 AND slug = $2 AND id <> $3",
        )
        .bind(h.familia_id)
        .bind(&slug)
        .bind(id)
        .fetch_optional(&st.db)
        .await?;
        if let Some((hid, titulo, memoria)) = achada {
            let memoria = match memoria.filter(|m| !m.trim().is_empty()) {
                Some(m) => m,
                None => gerar_memoria(st, hid).await?,
            };
            memorias.push((titulo, memoria));
            continua_de.push(hid);
        }
    }

    let leitores: Vec<(String, String)> = sqlx::query_as("SELECT nome, faixa FROM acesso.leitores WHERE familia_id = $1 ORDER BY nome")
        .bind(h.familia_id)
        .fetch_all(&st.db)
        .await?;
    let vozes = crate::tts::vozes_da_familia(st, h.familia_id).await?;
    let mut vozes_usadas: Vec<String> = vec![vozes.narrador.id.clone()];
    vozes_usadas.extend(vozes.personagens.iter().chain(vozes.extras.iter()).map(|(_, v)| v.id.clone()));
    let pedido = ia::PedidoTexto {
        sistema: SISTEMA.into(),
        imagens: Vec::new(),
        prompt: montar_prompt(&briefing, &leitores, &h.narracao, &citados, &todos, estilo, instrucoes, &memorias, &vozes_usadas),
        schema: schema_historia(),
    };
    Ok(Preparo { familia_id: h.familia_id, faixa: h.faixa, narracao: h.narracao, citados, todos, continua_de, pedido })
}

pub async fn executar_geracao(st: &AppState, payload: &Value) -> anyhow::Result<()> {
    let id: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let p = preparar(st, id).await?;
    let resposta = ia::gerar_texto(st, p.familia_id, &p.pedido).await?;
    gravar(st, id, &p, &resposta).await
}

/// Texto único para colar em outra IA (ChatGPT, Gemini...): instruções, pedido e formato da resposta.
pub fn prompt_para_copiar(p: &ia::PedidoTexto) -> String {
    format!(
        "{}\n\n{}\n\nFORMATO DA RESPOSTA: responda SOMENTE com um objeto JSON válido (sem texto antes ou depois), \
         seguindo este JSON Schema:\n{}\n",
        p.sistema.trim(),
        p.prompt.trim(),
        serde_json::to_string_pretty(&p.schema).unwrap_or_default()
    )
}

/// Grava a resposta da IA (da fila ou colada pela família) como o texto da história.
pub async fn gravar(st: &AppState, id: Uuid, p: &Preparo, resposta: &Value) -> anyhow::Result<()> {
    let (h, citados, todos, continua_de) = (p, &p.citados, &p.todos, &p.continua_de);
    let titulo = resposta["titulo"].as_str().unwrap_or("Nova história").trim().to_string();
    let faixa = resposta["faixa"].as_str().filter(|f| FAIXAS.contains(f)).unwrap_or(&h.faixa).to_string();
    let conteudo: Conteudo = serde_json::from_value(json!({ "paginas": resposta["paginas"], "secundarios": resposta["secundarios"] }))?;
    let conteudo = normalizar(conteudo, &h.narracao, citados, todos);
    if conteudo.paginas.is_empty() {
        anyhow::bail!("a IA devolveu uma história vazia");
    }

    let slug = slug_livre(st, h.familia_id, &titulo, id).await?;
    let valor = serde_json::to_value(&conteudo)?;
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE conteudo.historias SET titulo = $2, slug = $3, conteudo = $4, estado = 'texto', continua_de = $5,
                faixa = $6, atualizado_em = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(&titulo)
    .bind(&slug)
    .bind(&valor)
    .bind(continua_de)
    .bind(&faixa)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO conteudo.versoes (historia_id, conteudo) VALUES ($1, $2)")
        .bind(id)
        .bind(&valor)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Avisa por e-mail quem pediu (se quiser receber).
    if let Ok(Some((email, nome))) = sqlx::query_as::<_, (String, String)>(
        "SELECT u.email, u.nome FROM conteudo.historias h JOIN acesso.usuarios u ON u.id = h.criado_por
          WHERE h.id = $1 AND u.notificacoes",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await
    {
        crate::email::enviar_em_segundo_plano(
            email,
            format!("História pronta: {titulo}"),
            format!("Oi, {nome}! A história \"{titulo}\" ficou pronta:\n{}/historias/{id}", crate::email::url_publica()),
        );
    }
    Ok(())
}

/// Slug do título, com sufixo -2, -3... se já existir outro igual na família.
pub async fn slug_livre(st: &AppState, familia_id: Uuid, titulo: &str, proprio: Uuid) -> anyhow::Result<String> {
    let base = Some(util::slug(titulo)).filter(|s| !s.is_empty()).unwrap_or_else(|| "historia".into());
    for n in 1..100 {
        let candidato = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let ocupado: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM conteudo.historias WHERE familia_id = $1 AND slug = $2 AND id <> $3)",
        )
        .bind(familia_id)
        .bind(&candidato)
        .bind(proprio)
        .fetch_one(&st.db)
        .await?;
        if !ocupado {
            return Ok(candidato);
        }
    }
    Ok(format!("{base}-{}", &proprio.to_string()[..8]))
}

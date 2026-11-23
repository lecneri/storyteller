//! Job "gerar_historia": monta o pedido, chama a IA e grava a história.

use serde_json::{json, Value};
use uuid::Uuid;

use super::deteccao::{citar_historias, citar_memoria, detectar, personagens_da_familia, Citacao};
use super::prompt::{montar_prompt, normalizar, schema_historia, SISTEMA};
use super::{faixa_limpa, gerar_referencia, itens_memoria, Conteudo, NaCena};
use crate::{ia, util, AppState};

#[derive(sqlx::FromRow)]
struct HistoriaGerar {
    familia_id: Uuid,
    briefing: Option<String>,
    faixa: String,
    narracao: String,
    padroes: Option<Value>,
    conteudo: Value,
    paginas_pedidas: Option<i32>,
    usar_memoria_estante: bool,
    estante_principal: Option<Uuid>,
    memoria_gravar: Option<String>,
    memoria_usar: Option<String>,
}

/// Tudo o que a geração precisa antes de chamar a IA (e para gravar a resposta depois).
pub struct Preparo {
    pub familia_id: Uuid,
    pub(super) faixa: String,
    pub(super) narracao: String,
    pub(super) citados: Vec<NaCena>,
    pub(super) todos: Vec<super::deteccao::PersonagemIA>,
    /// Slugs dos lugares da família (para validar o "lugar" das páginas).
    pub(super) lugares: Vec<String>,
    continua_de: Vec<Uuid>,
    pub pedido: ia::PedidoTexto,
}

/// Monta o pedido à IA a partir do briefing, da estante, dos personagens e das histórias citadas.
pub async fn preparar(st: &AppState, id: Uuid) -> anyhow::Result<Preparo> {
    preparar_com(st, id, "").await
}

/// Como `preparar`, mas também procura citações (@personagem:variante, @grupo:, @lugar:, #historia) em `extra`
/// (o pedido de ajuda da IA na edição).
pub async fn preparar_com(st: &AppState, id: Uuid, extra: &str) -> anyhow::Result<Preparo> {
    let h = sqlx::query_as::<_, HistoriaGerar>(
        "SELECT h.familia_id, h.briefing, h.faixa, h.narracao, e.padroes, h.conteudo, h.paginas_pedidas, h.usar_memoria_estante, h.estante_principal,
                h.memoria_gravar, h.memoria_usar
           FROM conteudo.historias h
           LEFT JOIN conteudo.estantes e ON e.id = h.estante_principal
          WHERE h.id = $1",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;

    let todos = personagens_da_familia(st, h.familia_id).await?;
    let mut briefing = h.briefing.unwrap_or_default();
    // Número de páginas escolhido no formulário (só na primeira geração): vale mais que a regra da faixa.
    if let Some(n) = h.paginas_pedidas.filter(|_| h.conteudo["paginas"].as_array().is_none_or(Vec::is_empty)) {
        briefing.push_str(&format!(
            "\n\n(Tamanho pedido: a história deve ter exatamente {n} páginas; isto vale mais que a quantidade de páginas da faixa etária. As palavras por página seguem a faixa.)"
        ));
    }
    let texto_citacoes = format!("{briefing}\n\n{extra}");
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
    // Citados no pedido de ajuda: entram; a variante pedida soma às da história (não troca a principal).
    for n in detectar(extra, &todos) {
        match citados.iter_mut().find(|c| c.slug == n.slug) {
            Some(c) => {
                for v in n.variante.into_iter().chain(n.tambem) {
                    if c.variante.is_none() {
                        c.variante = Some(v);
                    } else if c.variante.as_deref() != Some(v.as_str()) && !c.tambem.contains(&v) {
                        c.tambem.push(v);
                    }
                }
                if c.voz.is_none() && n.voz.is_some() {
                    c.voz = n.voz;
                    c.misto = n.misto;
                }
            }
            None => citados.push(n),
        }
    }
    // História já escrita (ajuda da IA na edição): quem já está nas páginas também entra.
    for pg in h.conteudo["paginas"].as_array().into_iter().flatten() {
        for n in pg["personagens"].as_array().into_iter().flatten() {
            let slug = n["slug"].as_str().unwrap_or_default();
            if todos.iter().any(|t| t.slug == slug) && !citados.iter().any(|c| c.slug == slug) {
                citados.push(NaCena { slug: slug.into(), variante: None, voz: None, misto: false, tambem: Vec::new() });
            }
        }
        for b in pg["blocos"].as_array().into_iter().flatten() {
            let slug = b["quem"].as_str().unwrap_or_default();
            if todos.iter().any(|t| t.slug == slug) && !citados.iter().any(|c| c.slug == slug) {
                citados.push(NaCena { slug: slug.into(), variante: None, voz: None, misto: false, tambem: Vec::new() });
            }
        }
    }
    // Ninguém citado (nem @, nem pelo nome) e estante sem personagens fixos: a IA não recebe os personagens da família
    // e cria os da própria história (secundários: alunos, professora...).
    let estilo = padroes["estilo"].as_str().unwrap_or_default();
    let instrucoes = padroes["instrucoes"].as_str().unwrap_or_default();

    // Histórias citadas com #: entram pela referência (resumo), não pelo texto inteiro;
    // #historia:memoria e #memoria trazem as marcações da memória.
    let mut referencias = Vec::new();
    let mut continua_de = Vec::new();
    let mut memoria: Vec<(String, Vec<String>)> = Vec::new();
    let mut somar = |titulo: String, itens: Vec<String>| {
        if !itens.is_empty() && !memoria.iter().any(|(t, _)| *t == titulo) {
            memoria.push((titulo, itens));
        }
    };
    // Do briefing (#historia, #historia:memoria, #memoria) e do campo "usar da memória" (#estante, #historia, #todas).
    let mut citacoes = citar_historias(&texto_citacoes);
    citacoes.extend(citar_memoria(h.memoria_usar.as_deref().unwrap_or_default()));
    if h.usar_memoria_estante {
        citacoes.push(Citacao::MemoriaEstante(None));
    }
    let mut estantes: Vec<Uuid> = Vec::new();
    for c in citacoes {
        match c {
            Citacao::MemoriaEstante(None) => estantes.extend(h.estante_principal),
            Citacao::MemoriaEstante(Some(slug)) => {
                let e: Option<Uuid> = sqlx::query_scalar("SELECT id FROM conteudo.estantes WHERE familia_id = $1 AND slug = $2")
                    .bind(h.familia_id)
                    .bind(&slug)
                    .fetch_optional(&st.db)
                    .await?;
                estantes.extend(e);
            }
            Citacao::Referencia(slug) => {
                let achada: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
                    "SELECT id, titulo, referencia FROM conteudo.historias WHERE familia_id = $1 AND slug = $2 AND id <> $3",
                )
                .bind(h.familia_id)
                .bind(&slug)
                .bind(id)
                .fetch_optional(&st.db)
                .await?;
                if let Some((hid, titulo, referencia)) = achada {
                    let referencia = match referencia.filter(|m| !m.trim().is_empty()) {
                        Some(m) => m,
                        None => gerar_referencia(st, hid).await?,
                    };
                    referencias.push((titulo, referencia));
                    continua_de.push(hid);
                }
            }
            Citacao::Memoria(slug) => {
                let achada: Option<(String, Value)> = sqlx::query_as(
                    "SELECT titulo, memoria FROM conteudo.historias WHERE familia_id = $1 AND slug = $2 AND id <> $3",
                )
                .bind(h.familia_id)
                .bind(&slug)
                .bind(id)
                .fetch_optional(&st.db)
                .await?;
                if let Some((titulo, m)) = achada {
                    somar(titulo, itens_memoria(&m));
                }
            }
            Citacao::TodaMemoria => {
                let todas: Vec<(String, Value)> = sqlx::query_as(
                    "SELECT titulo, memoria FROM conteudo.historias
                      WHERE familia_id = $1 AND id <> $2 AND jsonb_array_length(memoria) > 0 ORDER BY criado_em",
                )
                .bind(h.familia_id)
                .bind(id)
                .fetch_all(&st.db)
                .await?;
                for (titulo, m) in todas {
                    somar(titulo, itens_memoria(&m));
                }
            }
        }
    }
    // #estante (a atual, ou a antiga caixa "usar a memória da estante") e #estante:slug: a memória das outras histórias delas.
    estantes.dedup();
    for estante in estantes {
        let da_estante: Vec<(String, Value)> = sqlx::query_as(
            "SELECT h.titulo, h.memoria FROM conteudo.historias h JOIN conteudo.estante_historias eh ON eh.historia_id = h.id
              WHERE eh.estante_id = $1 AND h.id <> $2 AND jsonb_array_length(h.memoria) > 0 ORDER BY h.criado_em",
        )
        .bind(estante)
        .bind(id)
        .fetch_all(&st.db)
        .await?;
        for (titulo, m) in da_estante {
            somar(titulo, itens_memoria(&m));
        }
    }

    let vozes = crate::tts::vozes_da_familia(st, h.familia_id).await?;
    let mut vozes_usadas: Vec<String> = vec![vozes.narrador.id.clone()];
    vozes_usadas.extend(vozes.personagens.iter().chain(vozes.extras.iter()).map(|(_, v)| v.id.clone()));
    // Lugares: os citados com @lugar:slug e os que a história já usa (na ajuda da IA).
    let lugares_familia = crate::lugares::da_familia(st, h.familia_id).await;
    let mut lugares_citados = super::deteccao::lugares_citados(&texto_citacoes);
    for pg in h.conteudo["paginas"].as_array().into_iter().flatten() {
        if let Some(l) = pg["lugar"].as_str().filter(|l| !l.is_empty()) {
            if !lugares_citados.iter().any(|x| x == l) {
                lugares_citados.push(l.to_string());
            }
        }
    }
    let mut prompt = montar_prompt(&briefing, &h.faixa, &h.narracao, &citados, &todos, estilo, instrucoes, &referencias, &vozes_usadas);
    prompt.push_str(&super::prompt::bloco_memoria(&memoria));
    prompt.push_str(&super::prompt::bloco_gravar(h.memoria_gravar.as_deref().unwrap_or_default()));
    prompt.push_str(&super::prompt::bloco_lugares(&crate::lugares::para_ia(&lugares_familia, &lugares_citados)));
    let pedido = ia::PedidoTexto { sistema: SISTEMA.into(), imagens: Vec::new(), prompt, schema: schema_historia() };
    let lugares = lugares_familia.into_iter().map(|l| l.slug).collect();
    Ok(Preparo { familia_id: h.familia_id, faixa: h.faixa, narracao: h.narracao, citados, todos, lugares, continua_de, pedido })
}

pub async fn executar_geracao(st: &AppState, payload: &Value) -> anyhow::Result<()> {
    let id: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let p = preparar(st, id).await?;
    // "pago": a família escolheu seguir para a API paga depois que as assinaturas falharam.
    let resposta = ia::gerar_texto_com(st, p.familia_id, &p.pedido, payload["pago"].as_bool() == Some(true)).await?;
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
    // A faixa escolhida pela família vale; vazia, fica a que a IA escolheu.
    let faixa = if h.faixa.trim().is_empty() { faixa_limpa(resposta["faixa"].as_str().unwrap_or_default()) } else { h.faixa.clone() };
    let conteudo: Conteudo = serde_json::from_value(
        json!({ "paginas": resposta["paginas"], "secundarios": resposta["secundarios"], "lugares": resposta["lugares"] }),
    )?;
    let mut conteudo = normalizar(conteudo, &h.narracao, citados, todos);
    super::prompt::ajustar_lugares(&mut conteudo, &p.lugares);
    if conteudo.paginas.is_empty() {
        anyhow::bail!("a IA devolveu uma história vazia");
    }

    let slug = slug_livre(st, h.familia_id, &titulo, id).await?;
    let valor = serde_json::to_value(&conteudo)?;
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE conteudo.historias SET titulo = $2, slug = $3, conteudo = $4, estado = 'texto', continua_de = $5,
                faixa = $6, memoria = $7, atualizado_em = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(&titulo)
    .bind(&slug)
    .bind(&valor)
    .bind(continua_de)
    .bind(&faixa)
    .bind(json!(itens_memoria(&resposta["memoria"])))
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

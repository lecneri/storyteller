//! Rotas das histórias: lista, criação, editor, salvar, mídia, imagem pela IA, menções e apagar.

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::deteccao::personagens_da_familia;
use super::geracao::slug_livre;
use super::{faixa_limpa, Conteudo, ANIMACOES};
use crate::{
    auth::Sessao,
    jobs, midia, util,
    web::{erro_interno, erro_json, historia_da_familia, render},
    AppState,
};

// ───────────── páginas ─────────────

#[derive(sqlx::FromRow)]
struct ItemHistoria {
    id: Uuid,
    titulo: String,
    faixa: String,
    estado: String,
    paginas: i32,
    /// Admin da família ou quem criou.
    pode_apagar: bool,
    /// Para a busca: título, briefing e texto das páginas.
    busca: String,
    /// Slugs dos personagens que aparecem (cenas, falas e @ do briefing), separados por espaço.
    pers: String,
    /// Slugs das estantes onde ela está.
    est: String,
}

#[derive(Template)]
#[template(path = "historias.html")]
struct PaginaHistorias {
    s: Sessao,
    itens: Vec<ItemHistoria>,
    personagens: Vec<(String, String)>,
    estantes: Vec<(String, String)>,
    /// Faixa da última história criada (o campo da nova história já vem com ela).
    ultima_faixa: String,
}

/// Faixa da última história criada pela família (vazia se nenhuma).
pub async fn ultima_faixa(st: &AppState, familia: Uuid) -> String {
    sqlx::query_scalar("SELECT faixa FROM conteudo.historias WHERE familia_id = $1 AND faixa <> '' ORDER BY criado_em DESC LIMIT 1")
        .bind(familia)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub async fn lista(State(st): State<AppState>, s: Sessao) -> Response {
    let itens = sqlx::query_as::<_, ItemHistoria>(
        "SELECT h.id, h.titulo, h.faixa, h.estado, jsonb_array_length(h.conteudo->'paginas') AS paginas,
                ($2 OR h.criado_por = $3) AS pode_apagar,
                concat_ws(' ', h.titulo, h.briefing,
                    (SELECT string_agg(b->>'texto', ' ') FROM jsonb_array_elements(h.conteudo->'paginas') pg,
                            jsonb_array_elements(COALESCE(pg->'blocos', '[]')) b)) AS busca,
                COALESCE((SELECT string_agg(DISTINCT x, ' ') FROM (
                    SELECT n->>'slug' AS x FROM jsonb_array_elements(h.conteudo->'paginas') pg,
                           jsonb_array_elements(COALESCE(pg->'personagens', '[]')) n
                    UNION SELECT b->>'quem' FROM jsonb_array_elements(h.conteudo->'paginas') pg,
                           jsonb_array_elements(COALESCE(pg->'blocos', '[]')) b
                    UNION SELECT (regexp_matches(COALESCE(h.briefing, ''), '@([A-Za-z0-9-]+)', 'g'))[1]
                  ) t WHERE x IS NOT NULL AND x <> 'narrador'), '') AS pers,
                COALESCE((SELECT string_agg(e.slug, ' ') FROM conteudo.estante_historias eh
                            JOIN conteudo.estantes e ON e.id = eh.estante_id WHERE eh.historia_id = h.id), '') AS est
           FROM conteudo.historias h WHERE h.familia_id = $1 ORDER BY h.atualizado_em DESC",
    )
    .bind(s.familia_id)
    .bind(s.admin_familia())
    .bind(s.usuario_id)
    .fetch_all(&st.db)
    .await;
    let personagens = personagens_da_familia(&st, s.familia_id).await;
    let estantes = sqlx::query_as::<_, (String, String)>(
        "SELECT slug, nome FROM conteudo.estantes WHERE familia_id = $1 ORDER BY ordem, nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    let ultima_faixa = ultima_faixa(&st, s.familia_id).await;
    match (itens, personagens, estantes) {
        (Ok(itens), Ok(p), Ok(estantes)) => render(PaginaHistorias {
            ultima_faixa,
            s,
            itens,
            personagens: p.into_iter().map(|p| (p.slug, p.nome)).collect(),
            estantes,
        }),
        (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => erro_interno(e.into()),
    }
}

#[derive(Deserialize)]
pub struct FormNova {
    #[serde(default)]
    briefing: String,
    /// Opcional, texto livre ("5 anos", "8-10", "adulto"): sem ela, a IA escolhe.
    #[serde(default)]
    faixa: String,
    narracao: String,
    /// "ia", "manual" ou "externo" (copiar o prompt para outra IA e colar a resposta)
    modo: String,
    /// slug da estante onde a história nasce (opcional)
    #[serde(default)]
    estante: String,
    #[serde(default)]
    visibilidade: String,
    /// Número de páginas (4 a 30); vazio = a IA escolhe pela idade.
    #[serde(default)]
    paginas: String,
    /// "1": a geração recebe a memória das histórias da mesma estante.
    #[serde(default)]
    usar_memoria_estante: Option<String>,
}

/// Páginas pedidas no formulário, dentro do limite (4 a 30); vazio ou inválido = automático.
fn paginas_pedidas(t: &str) -> Option<i32> {
    t.trim().parse::<i32>().ok().map(|n| n.clamp(4, 30))
}

pub async fn criar(State(st): State<AppState>, s: Sessao, Form(f): Form<FormNova>) -> Response {
    // Faixa escolhida pela família (texto livre); vazia = a IA escolhe ao escrever.
    let faixa = faixa_limpa(&f.faixa);
    let narracao = if f.narracao == "narrador" { "narrador" } else { "por-personagem" };
    let manual = f.modo == "manual";
    let id = Uuid::new_v4();
    let conteudo = if manual {
        json!({ "paginas": [ { "cena": "", "personagens": [], "animacao": "nenhuma",
                               "blocos": [ { "quem": "narrador", "texto": "" } ] } ] })
    } else {
        json!({ "paginas": [] })
    };
    let r = sqlx::query(
        "INSERT INTO conteudo.historias (id, familia_id, slug, titulo, faixa, narracao, briefing, estado, conteudo, criado_por, visibilidade,
                                         paginas_pedidas, usar_memoria_estante)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(id)
    .bind(s.familia_id)
    .bind(format!("rascunho-{}", &id.to_string()[..8]))
    .bind(if manual { "História sem título" } else if f.modo == "externo" { "Aguardando a resposta da IA" } else { "Gerando…" })
    .bind(&faixa)
    .bind(narracao)
    .bind(Some(f.briefing.trim()).filter(|b| !b.is_empty()))
    .bind(if manual { "texto" } else { "rascunho" })
    .bind(conteudo)
    .bind(s.usuario_id)
    .bind(if ["privado", "compartilhado", "publico"].contains(&f.visibilidade.as_str()) { f.visibilidade.as_str() } else { "herdar" })
    .bind(paginas_pedidas(&f.paginas))
    .bind(f.usar_memoria_estante.is_some())
    .execute(&st.db)
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    if !f.estante.is_empty() {
        if let Err(e) = crate::estantes::vincular(&st, s.familia_id, &f.estante, id).await {
            return erro_interno(e.into());
        }
    }
    if f.modo == "ia" {
        if let Err(e) = jobs::enfileirar(&st, "gerar_historia", s.familia_id, json!({ "historia_id": id })).await {
            return erro_interno(e);
        }
    }
    Redirect::to(&format!("/historias/{id}")).into_response()
}

#[derive(sqlx::FromRow)]
struct Historia {
    id: Uuid,
    titulo: String,
    faixa: String,
    narracao: String,
    briefing: Option<String>,
    conteudo: Value,
    estilo: Option<String>,
    referencia: Option<String>,
    memoria: Value,
    continua: Option<Value>,
    capa: Option<Uuid>,
    musica: Option<Uuid>,
    musica_volume: f32,
    musica_credito: Option<String>,
}

#[derive(Template)]
#[template(path = "historia_gerando.html")]
struct PaginaGerando {
    s: Sessao,
    id: Uuid,
    briefing: String,
    erro: Option<String>,
    /// Sem geração na fila: a família vai usar outra IA (copiar prompt, colar resposta).
    externo: bool,
    /// Prompt para copiar (quando é externo ou a geração falhou).
    prompt: Option<String>,
    /// Admin da família ou quem criou.
    pode_apagar: bool,
}

#[derive(Template)]
#[template(path = "historia_editor.html")]
struct PaginaEditor {
    s: Sessao,
    titulo: String,
    dados: String,
    id: Uuid,
}

pub async fn abrir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let h = match sqlx::query_as::<_, Historia>(
        "SELECT h.id, h.titulo, h.faixa, h.narracao, h.briefing, h.conteudo, e.padroes->>'estilo' AS estilo, h.referencia, h.memoria, h.capa, h.musica, h.musica_volume, h.musica_credito,
                (SELECT json_agg(json_build_object('id', o.id, 'titulo', o.titulo))::jsonb
                   FROM conteudo.historias o WHERE o.id = ANY(h.continua_de)) AS continua
           FROM conteudo.historias h LEFT JOIN conteudo.estantes e ON e.id = h.estante_principal
          WHERE h.id = $1 AND h.familia_id = $2",
    )
    .bind(id)
    .bind(s.familia_id)
    .fetch_optional(&st.db)
    .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return Redirect::to("/historias").into_response(),
        Err(e) => return erro_interno(e.into()),
    };

    let vazia = h.conteudo["paginas"].as_array().is_none_or(|a| a.is_empty());
    if vazia {
        let job: Option<(String, Option<String>)> = match sqlx::query_as(
            "SELECT estado, erro FROM jobs.fila WHERE tipo = 'gerar_historia' AND payload->>'historia_id' = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(id.to_string())
        .fetch_optional(&st.db)
        .await
        {
            Ok(j) => j,
            Err(e) => return erro_interno(e.into()),
        };
        let externo = job.is_none();
        let erro = match job {
            Some((estado, erro)) if estado == "erro" => Some(erro.unwrap_or_else(|| "erro desconhecido".into())),
            _ => None,
        };
        let prompt = if externo || erro.is_some() {
            match super::geracao::preparar(&st, id).await {
                Ok(p) => Some(super::geracao::prompt_para_copiar(&p.pedido)),
                Err(e) => return erro_interno(e),
            }
        } else {
            None
        };
        let criador: Option<Uuid> = sqlx::query_scalar("SELECT criado_por FROM conteudo.historias WHERE id = $1")
            .bind(id)
            .fetch_one(&st.db)
            .await
            .unwrap_or(None);
        let pode_apagar = s.admin_familia() || criador == Some(s.usuario_id);
        return render(PaginaGerando { s, id, briefing: h.briefing.unwrap_or_default(), erro, externo, prompt, pode_apagar });
    }

    let personagens = match personagens_da_familia(&st, s.familia_id).await {
        Ok(p) => p,
        Err(e) => return erro_interno(e.into()),
    };
    let dados = json!({
        "id": h.id, "titulo": h.titulo, "faixa": h.faixa, "narracao": h.narracao,
        "briefing": h.briefing, "conteudo": h.conteudo,
        "personagens": personagens,
        "lugares": crate::lugares::da_familia(&st, s.familia_id).await,
        "tipos": crate::personagens::TIPOS.iter().map(|(k, n)| json!({ "id": k, "nome": n })).collect::<Vec<_>>(),
        "vozes_sistema": crate::tts::VOZES.iter().map(|(id, n)| json!({ "id": id, "nome": n })).collect::<Vec<_>>(),
        "animacoes": ANIMACOES, "estilo": h.estilo,
        "referencia": h.referencia, "memoria": h.memoria, "continua": h.continua, "capa": h.capa,
        "musica": h.musica.map(|m| json!({ "id": m, "volume": h.musica_volume, "credito": h.musica_credito })),
        "musicas_modelo": crate::musica::modelos(&st).await.unwrap_or_default(),
        "climas": crate::musica::CLIMAS.iter().map(|(id, n)| json!({ "id": id, "nome": n })).collect::<Vec<_>>(),
        "imagem_api": crate::imagem::disponivel(&st, s.familia_id).await,
    });
    // JSON dentro de <script>: impede que um texto "</script>" feche a tag.
    let dados = dados.to_string().replace("</", "<\\/");
    render(PaginaEditor { s, titulo: h.titulo, dados, id })
}

pub async fn regerar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let existe = historia_da_familia(&st, s.familia_id, id).await;
    match existe {
        Ok(true) => {}
        Ok(false) => return Redirect::to("/historias").into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    if let Err(e) = jobs::enfileirar(&st, "gerar_historia", s.familia_id, json!({ "historia_id": id })).await {
        return erro_interno(e);
    }
    Redirect::to(&format!("/historias/{id}")).into_response()
}

#[derive(Deserialize)]
pub struct Importar {
    texto: String,
}

/// POST /historias/{id}/importar — resposta de outra IA (colada ou de arquivo) vira o texto da história.
pub async fn importar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(f): Json<Importar>) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let resposta = match crate::ia::extrair_json(&f.texto) {
        Ok(v) if v["paginas"].is_array() => v,
        Ok(_) => return erro_json("O JSON não tem \"paginas\". Copie a resposta inteira da IA."),
        Err(_) => return erro_json("Não achei um JSON na resposta. Copie a resposta inteira da IA (de { até })."),
    };
    let p = match super::geracao::preparar(&st, id).await {
        Ok(p) => p,
        Err(e) => return erro_interno(e),
    };
    match super::geracao::gravar(&st, id, &p, &resposta).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_json(format!("Não deu para usar a resposta: {e}")),
    }
}

#[derive(Deserialize)]
pub struct SalvarConteudo {
    titulo: String,
    narracao: String,
    conteudo: Conteudo,
    /// Faixa etária escolhida no editor (vazio = não muda).
    #[serde(default)]
    faixa: String,
}

pub async fn salvar(
    State(st): State<AppState>,
    s: Sessao,
    Path(id): Path<Uuid>,
    Json(mut c): Json<SalvarConteudo>,
) -> Response {
    let titulo = c.titulo.trim().to_string();
    let titulo = titulo.as_str();
    if titulo.is_empty() {
        return erro_json("Dê um título para a história.");
    }
    let narracao = if c.narracao == "narrador" { "narrador" } else { "por-personagem" };
    // Imagens sendo geradas na fila: o editor pode estar desatualizado; não apaga nem troca a que a fila pôs.
    let protegidas = super::imagens::chaves_protegidas(&st, id).await;
    if !protegidas.is_empty() {
        let no_banco: serde_json::Value = sqlx::query_scalar("SELECT conteudo FROM conteudo.historias WHERE id = $1 AND familia_id = $2")
            .bind(id)
            .bind(s.familia_id)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        // Nessas páginas vale a imagem que a fila pôs (o editor pode não ter recebido ainda).
        for pag in c.conteudo.paginas.iter_mut().filter(|p| protegidas.contains(&p.chave)) {
            let atual = no_banco["paginas"].as_array().into_iter().flatten().find(|x| x["chave"] == pag.chave.as_str());
            if let Some(m) = atual.and_then(|x| serde_json::from_value(x["midia"].clone()).ok()) {
                pag.midia = Some(m);
            }
        }
    }
    let valor = match serde_json::to_value(&c.conteudo) {
        Ok(v) => v,
        Err(e) => return erro_interno(e.into()),
    };
    let slug = match slug_livre(&st, s.familia_id, titulo, id).await {
        Ok(sl) => sl,
        Err(e) => return erro_interno(e),
    };
    let r = async {
        let mut tx = st.db.begin().await?;
        let n = sqlx::query(
            "UPDATE conteudo.historias SET titulo = $3, slug = $4, narracao = $5, conteudo = $6, faixa = COALESCE($7, faixa),
                    atualizado_em = now()
              WHERE id = $1 AND familia_id = $2",
        )
        .bind(id)
        .bind(s.familia_id)
        .bind(titulo)
        .bind(&slug)
        .bind(narracao)
        .bind(&valor)
        .bind(Some(faixa_limpa(&c.faixa)).filter(|f| !f.is_empty()))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 1 {
            sqlx::query("INSERT INTO conteudo.versoes (historia_id, conteudo, criado_por) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(&valor)
                .bind(s.usuario_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok::<u64, sqlx::Error>(n)
    }
    .await;
    match r {
        Ok(1) => Json(json!({ "ok": true })).into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /historias/{id}/midia: recebe a imagem/GIF/vídeo de uma página. Devolve {id, tipo};
/// o editor coloca na página e salva (assim não sobrescreve edições não salvas).
pub async fn enviar_midia(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, mut mp: Multipart) -> Response {
    let existe = historia_da_familia(&st, s.familia_id, id).await;
    match existe {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let erro = |m: &str| erro_json(m);
    let bytes = loop {
        match mp.next_field().await {
            Ok(Some(campo)) if campo.file_name().is_some() => match campo.bytes().await {
                Ok(b) => break b,
                Err(_) => return erro("Arquivo grande demais (máximo 25 MB)."),
            },
            Ok(Some(_)) => continue,
            Ok(None) => return erro("Nenhum arquivo enviado."),
            Err(_) => return erro("Envio inválido."),
        }
    };
    let Some((mime, ext)) = midia::tipo_midia(&bytes) else {
        return erro("Formato não aceito. Use PNG, JPG, WebP, GIF, MP4, WebM ou MOV.");
    };
    let pasta = format!("familias/{}/historias/{id}/midia", s.familia_slug);
    match midia::salvar(&st, s.familia_id, &pasta, &bytes, mime, ext).await {
        Ok(mid) => Json(json!({ "id": mid, "tipo": mime })).into_response(),
        Err(e) => erro_interno(e),
    }
}

#[derive(Deserialize)]
pub struct PedidoSecundario {
    nome: String,
    tipo: String,
    #[serde(default)]
    descricao: String,
    /// Voz usada nas falas dele na história (vira a voz padrão do personagem).
    #[serde(default)]
    voz: Option<crate::tts::Voz>,
}

/// POST /personagens/de-secundario: transforma um personagem só da história em personagem da família
/// (ficha com a aparência, variante "Original" e voz). Devolve o personagem no formato do editor.
pub async fn salvar_secundario(State(st): State<AppState>, s: Sessao, Json(p): Json<PedidoSecundario>) -> Response {
    let erro = |m: &str| erro_json(m);
    let nome = p.nome.trim().to_string();
    let slug = util::slug(&nome);
    if slug.is_empty() || !crate::personagens::TIPOS.iter().any(|(k, _)| *k == p.tipo) {
        return erro("Dê um nome e escolha o tipo.");
    }
    let descricao = p.descricao.trim().to_string();
    let voz = p.voz.map(|v| json!(v.validada())).unwrap_or(json!({}));
    let r = async {
        let mut tx = st.db.begin().await?;
        let pid: Uuid = sqlx::query_scalar(
            "INSERT INTO conteudo.personagens (familia_id, slug, nome, tipo, ficha, voz_padrao) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(s.familia_id)
        .bind(&slug)
        .bind(&nome)
        .bind(&p.tipo)
        .bind(json!({ "apelidos": [], "aparencia": descricao }))
        .bind(&voz)
        .fetch_one(&mut *tx)
        .await?;
        crate::personagens::variantes::criar_original(&mut tx, pid, None, None, &descricao, &[]).await?;
        tx.commit().await
    }
    .await;
    match r {
        Ok(()) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return erro(&format!("Já existe um personagem chamado {nome}.")),
        Err(e) => return erro_interno(e.into()),
    }
    match personagens_da_familia(&st, s.familia_id).await {
        Ok(todos) => Json(json!({ "personagem": todos.into_iter().find(|x| x.slug == slug) })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// GET /mencoes: personagens (para @) e histórias (para #) da família, usados pelo autocompletar do briefing.
/// Grupos de personagens para o autocompletar: [{slug, nome, membros: [slug], variantes: {slug: variante}}].
fn grupos_json(ps: &[super::deteccao::PersonagemIA]) -> Vec<serde_json::Value> {
    let mut grupos: Vec<(String, String, Vec<String>, serde_json::Map<String, Value>)> = Vec::new();
    for p in ps {
        for g in super::deteccao::grupos(&p.ficha) {
            let gs = crate::util::slug(&g);
            let i = match grupos.iter().position(|(s, ..)| *s == gs) {
                Some(i) => i,
                None => {
                    grupos.push((gs.clone(), g.clone(), Vec::new(), serde_json::Map::new()));
                    grupos.len() - 1
                }
            };
            grupos[i].2.push(p.slug.clone());
            if let Some(v) = super::deteccao::variante_no_grupo(&p.ficha, &gs) {
                grupos[i].3.insert(p.slug.clone(), json!(v));
            }
        }
    }
    grupos.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    grupos.into_iter().map(|(slug, nome, membros, vs)| json!({ "slug": slug, "nome": nome, "membros": membros, "variantes": vs })).collect()
}

pub async fn mencoes(State(st): State<AppState>, s: Sessao) -> Response {
    let personagens = personagens_da_familia(&st, s.familia_id).await;
    let historias: Result<Vec<(String, String)>, _> = sqlx::query_as(
        "SELECT slug, titulo FROM conteudo.historias
          WHERE familia_id = $1 AND jsonb_array_length(conteudo->'paginas') > 0 ORDER BY atualizado_em DESC",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    match (personagens, historias) {
        (Ok(p), Ok(h)) => Json(json!({
            "personagens": p.iter().map(|p| json!({
                "slug": p.slug, "nome": p.nome, "apelidos": p.ficha["apelidos"],
                "variantes": p.variantes.as_array().map(|a| a.iter().filter(|v| v["arquivada"] != true)
                    .map(|v| json!({ "slug": v["slug"], "nome": v["nome"] })).collect::<Vec<_>>()),
                "vozes": p.vozes_extras.as_array().map(|a| a.iter()
                    .map(|v| json!({ "slug": v["slug"], "nome": v["nome"], "lingua": v["lingua"] })).collect::<Vec<_>>()),
            })).collect::<Vec<_>>(),
            "grupos": grupos_json(&p),
            "lugares": crate::lugares::da_familia(&st, s.familia_id).await.into_iter()
                .map(|l| json!({ "slug": l.slug, "nome": l.nome })).collect::<Vec<_>>(),
            "historias": h.into_iter().map(|(slug, titulo)| json!({ "slug": slug, "titulo": titulo })).collect::<Vec<_>>(),
        }))
        .into_response(),
        (Err(e), _) | (_, Err(e)) => erro_interno(e.into()),
    }
}

/// POST /historias/{id}/apagar: apaga a história (admin da família ou quem criou). As mídias que ficarem
/// sem uso são apagadas pela limpeza (cópias em outras famílias continuam com os arquivos).
pub async fn apagar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let dados: Option<(Uuid, Option<Uuid>)> = sqlx::query_as("SELECT familia_id, criado_por FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    let Some((familia, criador)) = dados.filter(|(f, _)| *f == s.familia_id) else {
        return Redirect::to("/historias").into_response();
    };
    if !s.admin_familia() && criador != Some(s.usuario_id) {
        return Redirect::to(&format!("/historias/{id}")).into_response();
    }
    let r = async {
        let mut tx = st.db.begin().await?;
        sqlx::query("DELETE FROM partilha.destinos WHERE item_tipo = 'historia' AND item_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM partilha.links WHERE item_tipo = 'historia' AND item_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE conteudo.historias SET continua_de = array_remove(continua_de, $1) WHERE $1 = ANY(continua_de)")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM conteudo.historias WHERE id = $1 AND familia_id = $2").bind(id).bind(familia).execute(&mut *tx).await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    let st2 = st.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::limpeza::limpar(&st2).await {
            tracing::warn!("limpeza após apagar falhou: {e:#}");
        }
    });
    crate::video::apagar_video(&st, id).await;
    Redirect::to("/historias").into_response()
}


#[derive(Deserialize)]
pub struct DefinirCapa {
    /// Imagem já enviada (POST /historias/{id}/midia ou imagem-ia); null volta para a 1ª imagem do livro.
    midia: Option<Uuid>,
}

/// POST /historias/{id}/capa — escolhe a imagem de capa (ou volta a usar a da página 1).
pub async fn definir_capa(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(d): Json<DefinirCapa>) -> Response {
    if let Some(m) = d.midia {
        let ok: Option<String> = match sqlx::query_scalar("SELECT tipo_mime FROM midia.arquivos WHERE id = $1 AND familia_id = $2")
            .bind(m)
            .bind(s.familia_id)
            .fetch_optional(&st.db)
            .await
        {
            Ok(t) => t,
            Err(e) => return erro_interno(e.into()),
        };
        match ok {
            Some(t) if t.starts_with("image/") => {}
            Some(_) => return erro_json("A capa precisa ser uma imagem."),
            None => return erro_json("Imagem não encontrada."),
        }
    }
    match sqlx::query("UPDATE conteudo.historias SET capa = $3 WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .bind(d.midia)
        .execute(&st.db)
        .await
    {
        Ok(r) if r.rows_affected() == 1 => Json(json!({ "ok": true })).into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[cfg(test)]
mod testes_paginas {
    #[test]
    fn paginas_pedidas_ficam_no_limite() {
        assert_eq!(super::paginas_pedidas(""), None);
        assert_eq!(super::paginas_pedidas("abc"), None);
        assert_eq!(super::paginas_pedidas("12"), Some(12));
        assert_eq!(super::paginas_pedidas("200"), Some(30));
        assert_eq!(super::paginas_pedidas("1"), Some(4));
    }
}

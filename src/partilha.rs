//! Compartilhamento: conexões entre famílias, visibilidade (item vence grupo), cópia e links públicos.

use askama::Template;
use axum::{
    body::Body,
    extract::{Path, RawForm, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::{self, Sessao},
    player, tts, util,
    web::{erro_interno, render},
    AppState,
};

// ───────────── conexões ─────────────

pub async fn conectado(st: &AppState, a: Uuid, b: Uuid) -> bool {
    a == b
        || sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM partilha.conexoes WHERE estado = 'aceita'
                              AND ((de = $1 AND para = $2) OR (de = $2 AND para = $1)))",
        )
        .bind(a)
        .bind(b)
        .fetch_one(&st.db)
        .await
        .unwrap_or(false)
}

/// Famílias conectadas: (id, nome, slug).
pub async fn conectadas(st: &AppState, familia: Uuid) -> Result<Vec<(Uuid, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT f.id, f.nome, f.slug FROM partilha.conexoes c
           JOIN acesso.familias f ON f.id = CASE WHEN c.de = $1 THEN c.para ELSE c.de END
          WHERE c.estado = 'aceita' AND (c.de = $1 OR c.para = $1) ORDER BY f.nome",
    )
    .bind(familia)
    .fetch_all(&st.db)
    .await
}

/// Pedidos de conexão recebidos (id, nome, slug).
pub async fn pedidos_recebidos(st: &AppState, familia: Uuid) -> Result<Vec<(Uuid, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT f.id, f.nome, f.slug FROM partilha.conexoes c JOIN acesso.familias f ON f.id = c.de
          WHERE c.para = $1 AND c.estado = 'pendente' ORDER BY c.criado_em",
    )
    .bind(familia)
    .fetch_all(&st.db)
    .await
}

fn campos(corpo: &[u8]) -> Vec<(String, String)> {
    serde_urlencoded::from_bytes(corpo).unwrap_or_default()
}

/// POST /familia/conexoes (slug=@nome-unico)
pub async fn conectar(State(st): State<AppState>, s: Sessao, RawForm(corpo): RawForm) -> Response {
    if !s.admin_familia() {
        return Redirect::to("/familia").into_response();
    }
    let form = campos(&corpo);
    let slug = util::slug(form.iter().find(|(k, _)| k == "slug").map(|(_, v)| v.as_str()).unwrap_or_default());
    let outra: Option<Uuid> = sqlx::query_scalar("SELECT id FROM acesso.familias WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    let Some(outra) = outra.filter(|o| *o != s.familia_id) else {
        return Redirect::to("/familia?conexao=nao-encontrada").into_response();
    };
    // Se a outra família já tinha pedido, aceita; senão cria o pedido.
    let r = async {
        let aceitou = sqlx::query("UPDATE partilha.conexoes SET estado = 'aceita' WHERE de = $2 AND para = $1")
            .bind(s.familia_id)
            .bind(outra)
            .execute(&st.db)
            .await?
            .rows_affected();
        if aceitou == 0 {
            sqlx::query("INSERT INTO partilha.conexoes (de, para) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(s.familia_id)
                .bind(outra)
                .execute(&st.db)
                .await?;
        }
        Ok::<_, sqlx::Error>(())
    }
    .await;
    match r {
        Ok(()) => Redirect::to("/familia?conexao=enviada").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn aceitar(State(st): State<AppState>, s: Sessao, Path(de): Path<Uuid>) -> Response {
    if !s.admin_familia() {
        return Redirect::to("/familia").into_response();
    }
    match sqlx::query("UPDATE partilha.conexoes SET estado = 'aceita' WHERE de = $1 AND para = $2")
        .bind(de)
        .bind(s.familia_id)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/familia").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn desconectar(State(st): State<AppState>, s: Sessao, Path(outra): Path<Uuid>) -> Response {
    if !s.admin_familia() {
        return Redirect::to("/familia").into_response();
    }
    match sqlx::query("DELETE FROM partilha.conexoes WHERE (de = $1 AND para = $2) OR (de = $2 AND para = $1)")
        .bind(s.familia_id)
        .bind(outra)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/familia").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── regra de visibilidade (item → estante → família) ─────────────

async fn destinos(st: &AppState, tipo: &str, id: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT familia_id FROM partilha.destinos WHERE item_tipo = $1 AND item_id = $2")
        .bind(tipo)
        .bind(id)
        .fetch_all(&st.db)
        .await
        .unwrap_or_default()
}

/// Um nível explícito libera a leitora? (compartilhado/público com lista vazia = todas as conectadas)
async fn nivel_libera(st: &AppState, nivel: &str, tipo: &str, id: Uuid, leitora: Uuid) -> Option<bool> {
    match nivel {
        "privado" => Some(false),
        "compartilhado" | "publico" => {
            let d = destinos(st, tipo, id).await;
            Some(d.is_empty() || d.contains(&leitora))
        }
        _ => None, // herdar
    }
}

async fn padrao_familia(st: &AppState, familia: Uuid) -> bool {
    sqlx::query_scalar::<_, String>("SELECT visibilidade_padrao FROM acesso.familias WHERE id = $1")
        .bind(familia)
        .fetch_one(&st.db)
        .await
        .map(|v| v == "compartilhado")
        .unwrap_or(false)
}

/// A família `leitora` pode ler a história?
pub async fn pode_ler_historia(st: &AppState, leitora: Uuid, historia: Uuid) -> bool {
    let Ok(Some((dona, nivel))) = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT familia_id, visibilidade FROM conteudo.historias WHERE id = $1",
    )
    .bind(historia)
    .fetch_optional(&st.db)
    .await
    else {
        return false;
    };
    if dona == leitora {
        return true;
    }
    if !conectado(st, dona, leitora).await {
        return false;
    }
    if let Some(r) = nivel_libera(st, &nivel, "historia", historia, leitora).await {
        return r; // o item vence o grupo
    }
    // Herdar: basta uma estante liberar.
    let estantes: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT e.id, e.visibilidade FROM conteudo.estante_historias eh
           JOIN conteudo.estantes e ON e.id = eh.estante_id WHERE eh.historia_id = $1",
    )
    .bind(historia)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    if estantes.is_empty() {
        return padrao_familia(st, dona).await;
    }
    for (eid, ev) in estantes {
        let libera = match nivel_libera(st, &ev, "estante", eid, leitora).await {
            Some(r) => r,
            None => padrao_familia(st, dona).await,
        };
        if libera {
            return true;
        }
    }
    false
}

/// Histórias de outras famílias que a leitora pode ler: (id, titulo, familia_nome, capa).
pub async fn compartilhadas_comigo(st: &AppState, leitora: Uuid) -> Vec<(Uuid, String, String, Option<String>)> {
    let candidatas: Vec<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT h.id, h.titulo, f.nome,
                COALESCE(h.capa::text, (SELECT pg->'midia'->>'id' FROM jsonb_array_elements(h.conteudo->'paginas') pg
                                         WHERE pg->'midia'->>'tipo' LIKE 'image/%' LIMIT 1))
           FROM conteudo.historias h JOIN acesso.familias f ON f.id = h.familia_id
          WHERE h.familia_id IN (SELECT CASE WHEN de = $1 THEN para ELSE de END FROM partilha.conexoes
                                  WHERE estado = 'aceita' AND (de = $1 OR para = $1))
            AND jsonb_array_length(h.conteudo->'paginas') > 0
          ORDER BY h.atualizado_em DESC",
    )
    .bind(leitora)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    let mut v = Vec::new();
    for c in candidatas {
        if pode_ler_historia(st, leitora, c.0).await {
            v.push(c);
        }
    }
    v
}

// ───────────── tela "Compartilhar" (história, estante, personagem) ─────────────

const TIPOS: [&str; 3] = ["historia", "estante", "personagem"];

fn tabela(tipo: &str) -> &'static str {
    match tipo {
        "historia" => "conteudo.historias",
        "estante" => "conteudo.estantes",
        _ => "conteudo.personagens",
    }
}

#[derive(Template)]
#[template(path = "compartilhar.html")]
struct PaginaCompartilhar {
    s: Sessao,
    tipo: String,
    id: Uuid,
    nome: String,
    voltar: String,
    visibilidade: String,
    conectadas: Vec<(Uuid, String, String)>,
    escolhidas: Vec<Uuid>,
    links: Vec<(String, String)>,
    novo_link: Option<String>,
    pode_link: bool,
}

impl PaginaCompartilhar {
    fn marcada(&self, f: &Uuid) -> bool {
        self.escolhidas.contains(f)
    }
}

async fn carregar_item(st: &AppState, familia: Uuid, tipo: &str, id: Uuid) -> Option<(String, String, String)> {
    if !TIPOS.contains(&tipo) {
        return None;
    }
    let (col_nome, voltar) = match tipo {
        "historia" => ("titulo", format!("/historias/{id}")),
        "estante" => ("nome", String::new()),
        _ => ("nome", String::new()),
    };
    let sql = format!("SELECT {col_nome}, visibilidade, slug FROM {} WHERE id = $1 AND familia_id = $2", tabela(tipo));
    let (nome, vis, slug): (String, String, String) =
        sqlx::query_as(&sql).bind(id).bind(familia).fetch_optional(&st.db).await.ok()??;
    let voltar = match tipo {
        "estante" => format!("/estantes/{slug}"),
        "personagem" => format!("/personagens/{slug}"),
        _ => voltar,
    };
    Some((nome, vis, voltar))
}

async fn pagina(st: &AppState, s: Sessao, tipo: String, id: Uuid, novo_link: Option<String>) -> Response {
    let Some((nome, visibilidade, voltar)) = carregar_item(st, s.familia_id, &tipo, id).await else {
        return Redirect::to("/").into_response();
    };
    let conectadas = match conectadas(st, s.familia_id).await {
        Ok(c) => c,
        Err(e) => return erro_interno(e.into()),
    };
    let escolhidas = destinos(st, &tipo, id).await;
    let links: Vec<(String, String)> = sqlx::query_as(
        "SELECT token_hash, COALESCE('vence ' || to_char(expira_em, 'DD/MM/YYYY'), 'sem prazo')
           FROM partilha.links WHERE item_tipo = $1 AND item_id = $2 AND NOT revogado
            AND (expira_em IS NULL OR expira_em > now()) ORDER BY criado_em DESC",
    )
    .bind(&tipo)
    .bind(id)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    let pode_link = tipo != "personagem";
    render(PaginaCompartilhar { s, tipo, id, nome, voltar, visibilidade, conectadas, escolhidas, links, novo_link, pode_link })
}

pub async fn ver(State(st): State<AppState>, s: Sessao, Path((tipo, id)): Path<(String, Uuid)>) -> Response {
    pagina(&st, s, tipo, id, None).await
}

pub async fn salvar(
    State(st): State<AppState>,
    s: Sessao,
    Path((tipo, id)): Path<(String, Uuid)>,
    RawForm(corpo): RawForm,
) -> Response {
    if carregar_item(&st, s.familia_id, &tipo, id).await.is_none() {
        return Redirect::to("/").into_response();
    }
    let form = campos(&corpo);
    let vis = form.iter().find(|(k, _)| k == "visibilidade").map(|(_, v)| v.as_str()).unwrap_or("herdar");
    let vis = if ["herdar", "privado", "compartilhado", "publico"].contains(&vis) { vis } else { "herdar" };
    let todas = form.iter().any(|(k, v)| k == "destino" && v == "todas");
    let escolhidas: Vec<Uuid> = form.iter().filter(|(k, _)| k == "destino").filter_map(|(_, v)| v.parse().ok()).collect();
    let r = async {
        let mut tx = st.db.begin().await?;
        sqlx::query(&format!("UPDATE {} SET visibilidade = $3 WHERE id = $1 AND familia_id = $2", tabela(&tipo)))
            .bind(id)
            .bind(s.familia_id)
            .bind(vis)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM partilha.destinos WHERE item_tipo = $1 AND item_id = $2")
            .bind(&tipo)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if !todas {
            for f in escolhidas {
                sqlx::query("INSERT INTO partilha.destinos (item_tipo, item_id, familia_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
                    .bind(&tipo)
                    .bind(id)
                    .bind(f)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await
    }
    .await;
    match r {
        Ok(()) => Redirect::to(&format!("/compartilhar/{tipo}/{id}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── links públicos ─────────────

#[derive(Deserialize)]
pub struct FormLink {
    #[serde(default)]
    dias: String,
}

pub async fn criar_link(
    State(st): State<AppState>,
    s: Sessao,
    Path((tipo, id)): Path<(String, Uuid)>,
    axum::extract::Form(f): axum::extract::Form<FormLink>,
) -> Response {
    if tipo == "personagem" || carregar_item(&st, s.familia_id, &tipo, id).await.is_none() {
        return Redirect::to("/").into_response();
    }
    let dias: Option<i32> = f.dias.parse().ok().filter(|d| *d > 0);
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let r = sqlx::query(
        "INSERT INTO partilha.links (token_hash, item_tipo, item_id, familia_id, expira_em)
         VALUES ($1, $2, $3, $4, CASE WHEN $5::int IS NULL THEN NULL ELSE now() + make_interval(days => $5::int) END)",
    )
    .bind(auth::hash_token(&token))
    .bind(&tipo)
    .bind(id)
    .bind(s.familia_id)
    .bind(dias)
    .execute(&st.db)
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    let link = format!("{}/p/{token}", crate::email::url_publica());
    pagina(&st, s, tipo, id, Some(link)).await
}

pub async fn revogar_link(
    State(st): State<AppState>,
    s: Sessao,
    Path((tipo, id, hash)): Path<(String, Uuid, String)>,
) -> Response {
    match sqlx::query("UPDATE partilha.links SET revogado = true WHERE token_hash = $1 AND familia_id = $2")
        .bind(hash)
        .bind(s.familia_id)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to(&format!("/compartilhar/{tipo}/{id}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[derive(sqlx::FromRow)]
struct LinkValido {
    item_tipo: String,
    item_id: Uuid,
    familia_id: Uuid,
}

async fn link_valido(st: &AppState, token: &str) -> Option<LinkValido> {
    sqlx::query_as::<_, LinkValido>(
        "SELECT item_tipo, item_id, familia_id FROM partilha.links
          WHERE token_hash = $1 AND NOT revogado AND (expira_em IS NULL OR expira_em > now())",
    )
    .bind(auth::hash_token(token))
    .fetch_optional(&st.db)
    .await
    .ok()
    .flatten()
}

/// Histórias liberadas por um link: a própria história, ou as da estante que não estiverem privadas.
async fn historia_no_link(st: &AppState, l: &LinkValido, historia: Uuid) -> bool {
    if l.item_tipo == "historia" {
        return l.item_id == historia;
    }
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM conteudo.estante_historias eh JOIN conteudo.historias h ON h.id = eh.historia_id
                         WHERE eh.estante_id = $1 AND h.id = $2 AND h.visibilidade <> 'privado')",
    )
    .bind(l.item_id)
    .bind(historia)
    .fetch_one(&st.db)
    .await
    .unwrap_or(false)
}

#[derive(Template)]
#[template(path = "publico_estante.html")]
struct PaginaPublicaEstante {
    token: String,
    nome: String,
    historias: Vec<(Uuid, String, Option<String>)>,
}

/// GET /p/{token}: abre o livro (link de história) ou a estante pública.
pub async fn publico(State(st): State<AppState>, Path(token): Path<String>) -> Response {
    let Some(l) = link_valido(&st, &token).await else {
        return (StatusCode::NOT_FOUND, "Link inválido ou vencido.").into_response();
    };
    if l.item_tipo == "historia" {
        return publico_historia(State(st), Path((token, l.item_id))).await;
    }
    let nome: String = sqlx::query_scalar("SELECT nome FROM conteudo.estantes WHERE id = $1")
        .bind(l.item_id)
        .fetch_one(&st.db)
        .await
        .unwrap_or_default();
    let historias: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT h.id, h.titulo,
                COALESCE(h.capa::text, (SELECT pg->'midia'->>'id' FROM jsonb_array_elements(h.conteudo->'paginas') pg
                                         WHERE pg->'midia'->>'tipo' LIKE 'image/%' LIMIT 1))
           FROM conteudo.estante_historias eh JOIN conteudo.historias h ON h.id = eh.historia_id
          WHERE eh.estante_id = $1 AND h.visibilidade <> 'privado' AND jsonb_array_length(h.conteudo->'paginas') > 0
          ORDER BY eh.ordem",
    )
    .bind(l.item_id)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    render(PaginaPublicaEstante { token, nome, historias })
}

/// GET /p/{token}/h/{historia}: player público (só vozes genéricas de IA).
pub async fn publico_historia(State(st): State<AppState>, Path((token, historia)): Path<(String, Uuid)>) -> Response {
    let Some(l) = link_valido(&st, &token).await else {
        return (StatusCode::NOT_FOUND, "Link inválido ou vencido.").into_response();
    };
    if !historia_no_link(&st, &l, historia).await {
        return (StatusCode::NOT_FOUND, "História não disponível neste link.").into_response();
    }
    match player::manifest_publico(&st, historia, l.familia_id, &token).await {
        Ok(Some((titulo, dados))) => render(player::PaginaPlayer {
            titulo,
            dados: dados.to_string().replace("</", "<\\/"),
            voltar: if l.item_tipo == "estante" { format!("/p/{token}") } else { "about:blank".into() },
        }),
        Ok(None) => (StatusCode::NOT_FOUND, "História não encontrada.").into_response(),
        Err(e) => erro_interno(e),
    }
}

/// GET /p/{token}/midia/{id}: mídia da família dona do link.
pub async fn publico_midia(State(st): State<AppState>, Path((token, id)): Path<(String, Uuid)>) -> Response {
    let Some(l) = link_valido(&st, &token).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let linha: Option<(String, String)> =
        sqlx::query_as("SELECT caminho, tipo_mime FROM midia.arquivos WHERE id = $1 AND familia_id = $2")
            .bind(id)
            .bind(l.familia_id)
            .fetch_optional(&st.db)
            .await
            .unwrap_or(None);
    let Some((caminho, tipo)) = linha else { return StatusCode::NOT_FOUND.into_response() };
    match tokio::fs::read(st.data_dir.join(caminho)).await {
        Ok(b) => ([(header::CONTENT_TYPE, tipo), (header::CACHE_CONTROL, "public, max-age=86400".into())], Body::from(b)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
pub struct PedidoBloco {
    historia: Uuid,
    quem: String,
    texto: String,
}

/// POST /p/{token}/bloco-audio: gera áudio com voz genérica (nunca clonada).
pub async fn publico_audio(State(st): State<AppState>, Path(token): Path<String>, Json(p): Json<PedidoBloco>) -> Response {
    let Some(l) = link_valido(&st, &token).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !historia_no_link(&st, &l, p.historia).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    // O texto precisa existir na história (não é um TTS aberto para qualquer texto).
    let existe: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM conteudo.historias h, jsonb_array_elements(h.conteudo->'paginas') pg,
                               jsonb_array_elements(pg->'blocos') b
                         WHERE h.id = $1 AND trim(b->>'texto') = trim($2))",
    )
    .bind(p.historia)
    .bind(&p.texto)
    .fetch_one(&st.db)
    .await
    .unwrap_or(false);
    if !existe {
        return StatusCode::NOT_FOUND.into_response();
    }
    let tipo: String = sqlx::query_scalar("SELECT tipo FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2")
        .bind(l.familia_id)
        .bind(tts::separar(&p.quem).0)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let voz = if p.quem == "narrador" || tipo.is_empty() {
        tts::narrador_padrao()
    } else if tts::separar(&p.quem).1.is_some() {
        tts::generica_extra(&tipo)
    } else {
        tts::voz_por_tipo(&tipo)
    };
    let slug: String = sqlx::query_scalar("SELECT slug FROM acesso.familias WHERE id = $1")
        .bind(l.familia_id)
        .fetch_one(&st.db)
        .await
        .unwrap_or_default();
    match tts::sintetizar(&st, l.familia_id, &slug, &p.texto, &voz).await {
        Ok(a) => Json(json!({ "audio": format!("/p/{token}/midia/{}", a.arquivo), "duracao": a.duracao, "palavras": a.palavras }))
            .into_response(),
        Err(e) => {
            tracing::warn!("áudio público falhou: {e:#}");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

// ───────────── copiar para minha família ─────────────

/// POST /historias/{id}/copiar: a cópia passa a ser da família de quem copiou.
pub async fn copiar_historia(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if !pode_ler_historia(&st, s.familia_id, id).await {
        return Redirect::to("/").into_response();
    }
    let r = async {
        let mut tx = st.db.begin().await?;
        let (titulo, faixa, narracao, conteudo, dona): (String, String, String, Value, Uuid) = sqlx::query_as(
            "SELECT titulo, faixa, narracao, conteudo, familia_id FROM conteudo.historias WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        // Mídia: novas linhas apontando para os mesmos arquivos, agora da família que copiou.
        let mut conteudo = conteudo;
        if let Some(paginas) = conteudo["paginas"].as_array_mut() {
            for p in paginas {
                if let Some(mid) = p["midia"]["id"].as_str().and_then(|m| m.parse::<Uuid>().ok()) {
                    let novo: Option<Uuid> = sqlx::query_scalar(
                        "INSERT INTO midia.arquivos (familia_id, caminho, tipo_mime, bytes, sha256)
                         SELECT $1, caminho, tipo_mime, bytes, sha256 FROM midia.arquivos WHERE id = $2 AND familia_id = $3
                         RETURNING id",
                    )
                    .bind(s.familia_id)
                    .bind(mid)
                    .bind(dona)
                    .fetch_optional(&mut *tx)
                    .await?;
                    match novo {
                        Some(n) => p["midia"]["id"] = json!(n),
                        None => p["midia"] = Value::Null,
                    }
                }
            }
        }
        let novo = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO conteudo.historias (id, familia_id, slug, titulo, faixa, narracao, estado, conteudo, criado_por, visibilidade)
             VALUES ($1, $2, $3, $4, $5, $6, 'texto', $7, $8, 'privado')",
        )
        .bind(novo)
        .bind(s.familia_id)
        .bind(format!("copia-{}", &novo.to_string()[..8]))
        .bind(format!("{titulo} (cópia)"))
        .bind(faixa)
        .bind(narracao)
        .bind(conteudo)
        .bind(s.usuario_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok::<Uuid, sqlx::Error>(novo)
    }
    .await;
    match r {
        Ok(novo) => Redirect::to(&format!("/historias/{novo}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

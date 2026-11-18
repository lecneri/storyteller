//! Configurações do sistema (só admin do sistema): famílias, flags de CLI e chave do sistema, limites e chaves.

use askama::Template;
use axum::{
    extract::{Path, Query, RawForm, State},
    response::{IntoResponse, Redirect, Response},
};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    familia::{self, PROVEDORES},
    segredos,
    web::{erro_interno, render},
    AppState,
};

#[derive(sqlx::FromRow)]
struct LinhaFamilia {
    id: Uuid,
    nome: String,
    slug: String,
    /// "Nome · e-mail" do(s) admin(s) da família.
    admins: Option<String>,
    membros: i64,
    historias: i64,
    clis: Vec<String>,
    modo_cli: String,
    clis_imagem: Vec<String>,
    ordem_fontes: Vec<String>,
    /// Modelo e esforço por CLI: {"claude": {"modelo": "...", "esforco": "..."}}.
    cli_ajustes: serde_json::Value,
    usar_chave_sistema: bool,
    limite_semana: Option<i64>,
    uso_semana: i64,
    espaco_mb: f64,
}

/// Abas da página do sistema: (chave, rótulo).
const ABAS: [(&str, &str); 5] = [
    ("familias", "👨‍👩‍👧 Famílias"),
    ("assinaturas", "🤖 Assinaturas"),
    ("config", "⚙️ Configurações"),
    ("musicas", "🎵 Músicas"),
    ("manutencao", "🧹 Manutenção"),
];
const POR_PAGINA: i64 = 20;

/// Nome amigável de cada CLI.
fn nome_cli(c: &str) -> &'static str {
    match c {
        "claude" => "Claude",
        "codex" => "ChatGPT (Codex)",
        "agy" => "Gemini (agy)",
        _ => "?",
    }
}

#[derive(Template)]
#[template(path = "sistema.html")]
struct PaginaSistema {
    s: Sessao,
    aba: String,
    abas: Vec<(&'static str, &'static str)>,
    familias: Vec<LinhaFamilia>,
    busca: String,
    pagina: i64,
    paginas: i64,
    total: i64,
    provedores: Vec<(&'static str, &'static str)>,
    linhas_chaves: Vec<crate::chaves::LinhaChave>,
    cripto_ok: bool,
    aviso: Option<String>,
    clis: Vec<(String, String)>,
    /// Fila de imagens: (fonte, esperando, desenhando).
    fila_imagens: Vec<(String, i64, i64)>,
}

impl PaginaSistema {
    fn cfg(&self, k: &str) -> String {
        if crate::config::secreta(k) { String::new() } else { crate::config::get(k).unwrap_or_default() }
    }
    fn tem(&self, k: &str) -> bool {
        crate::config::get(k).is_some()
    }
    fn origem(&self, k: &str) -> &'static str {
        match crate::config::origem(k) {
            "site" => "salvo aqui",
            "env" => "vem do .env",
            _ => "",
        }
    }
    fn eh_aba(&self, k: &str) -> bool {
        self.aba == k
    }
    fn nome_cli(&self, c: &str) -> &'static str {
        nome_cli(c)
    }
    /// Link de outra página da lista, mantendo a busca.
    fn link_pagina(&self, n: &i64) -> String {
        let mut u = format!("/sistema?aba=familias&p={n}");
        if !self.busca.is_empty() {
            u.push_str("&q=");
            u.push_str(&urlencoding(&self.busca));
        }
        u
    }
    fn link_anterior(&self) -> String {
        self.link_pagina(&(self.pagina - 1))
    }
    fn link_proxima(&self) -> String {
        self.link_pagina(&(self.pagina + 1))
    }
    fn lista_paginas(&self) -> Vec<i64> {
        (1..=self.paginas).collect()
    }
}

fn urlencoding(t: &str) -> String {
    serde_urlencoded::to_string([("", t)]).unwrap_or_default().trim_start_matches('=').to_string()
}

fn so_admin_sistema(s: &Sessao) -> Option<Response> {
    (!s.admin_sistema).then(|| Redirect::to("/").into_response())
}

const SELECT_FAMILIA: &str = "SELECT f.id, f.nome, f.slug,
        (SELECT string_agg(u.nome || ' · ' || u.email, ', ' ORDER BY m.criado_em) FROM acesso.membros m
           JOIN acesso.usuarios u ON u.id = m.usuario_id WHERE m.familia_id = f.id AND m.papel = 'admin') AS admins,
        (SELECT count(*) FROM acesso.membros m WHERE m.familia_id = f.id) AS membros,
        (SELECT count(*) FROM conteudo.historias h WHERE h.familia_id = f.id) AS historias,
        COALESCE(c.clis, '{}') AS clis, COALESCE(c.modo_cli, 'ordem') AS modo_cli,
        COALESCE(c.clis_imagem, '{}') AS clis_imagem, COALESCE(c.ordem_fontes, '{cli,familia,sistema}') AS ordem_fontes,
        COALESCE(c.cli_ajustes, '{}') AS cli_ajustes,
        COALESCE(c.usar_chave_sistema, false) AS usar_chave_sistema,
        (c.limites->>'historias_semana')::bigint AS limite_semana,
        (SELECT count(*) FROM ia.uso u WHERE u.familia_id = f.id AND u.criado_em > now() - interval '7 days'
            AND u.via IN ('cli', 'chave_sistema')) AS uso_semana,
        COALESCE((SELECT sum(a.bytes) FROM midia.arquivos a WHERE a.familia_id = f.id), 0)::float8 / 1048576.0 AS espaco_mb
   FROM acesso.familias f LEFT JOIN ia.config_familia c ON c.familia_id = f.id";

/// Filtro da busca: nome ou @ da família, nome ou e-mail de qualquer membro.
const FILTRO_BUSCA: &str = "($1 = '' OR f.nome ILIKE $1 OR f.slug ILIKE $1 OR EXISTS (
        SELECT 1 FROM acesso.membros m JOIN acesso.usuarios u ON u.id = m.usuario_id
         WHERE m.familia_id = f.id AND (u.nome ILIKE $1 OR u.email ILIKE $1)))";

#[derive(serde::Deserialize, Default)]
pub struct Consulta {
    #[serde(default)]
    aba: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    p: Option<i64>,
}

async fn pagina(st: &AppState, s: Sessao, c: Consulta, aviso: Option<String>) -> Response {
    let aba = if ABAS.iter().any(|(k, _)| *k == c.aba) { c.aba.clone() } else { "familias".to_string() };
    let busca = c.q.trim().to_string();
    let padrao = if busca.is_empty() { String::new() } else { format!("%{}%", busca.replace('%', "").replace('_', "")) };
    let (mut familias, mut total, mut pag, mut paginas) = (Vec::new(), 0, 1, 1);
    if aba == "familias" {
        total = match sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM acesso.familias f WHERE {FILTRO_BUSCA}"))
            .bind(&padrao)
            .fetch_one(&st.db)
            .await
        {
            Ok(n) => n,
            Err(e) => return erro_interno(e.into()),
        };
        paginas = ((total + POR_PAGINA - 1) / POR_PAGINA).max(1);
        pag = c.p.unwrap_or(1).clamp(1, paginas);
        familias = match sqlx::query_as::<_, LinhaFamilia>(&format!(
            "{SELECT_FAMILIA} WHERE {FILTRO_BUSCA} ORDER BY lower(f.nome), f.slug LIMIT $2 OFFSET $3"
        ))
        .bind(&padrao)
        .bind(POR_PAGINA)
        .bind((pag - 1) * POR_PAGINA)
        .fetch_all(&st.db)
        .await
        {
            Ok(f) => f,
            Err(e) => return erro_interno(e.into()),
        };
    }
    let clis = if aba == "assinaturas" { crate::cli_login::situacao().await } else { Vec::new() };
    let fila_imagens = if aba == "assinaturas" { crate::fila_imagens::resumo(st).await } else { Vec::new() };
    render(PaginaSistema {
        clis,
        fila_imagens,
        s,
        aba,
        abas: ABAS.to_vec(),
        familias,
        busca,
        pagina: pag,
        paginas,
        total,
        // A ElevenLabs é por família: cada uma usa a própria chave.
        provedores: PROVEDORES.iter().copied().filter(|(p, _)| *p != "elevenlabs").collect(),
        linhas_chaves: crate::chaves::linhas(st, None).await,
        cripto_ok: segredos::disponivel(),
        aviso,
    })
}

fn aba(nome: &str) -> Consulta {
    Consulta { aba: nome.into(), ..Default::default() }
}

pub async fn ver(State(st): State<AppState>, s: Sessao, Query(c): Query<Consulta>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    pagina(&st, s, c, None).await
}

fn campos(corpo: &[u8]) -> Vec<(String, String)> {
    serde_urlencoded::from_bytes(corpo).unwrap_or_default()
}
fn campo(form: &[(String, String)], k: &str) -> String {
    form.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_string()).unwrap_or_default()
}

// ───────────── página de uma família ─────────────

/// Um CLI na página da família: chave, nome, situação no servidor, marcado para a família.
struct OpcaoCli {
    chave: String,
    nome: &'static str,
    situacao: String,
    marcado: bool,
}

/// Modelo e esforço de uma CLI na família (tela).
struct AjusteTela {
    chave: String,
    nome: &'static str,
    modelo: String,
    esforco: String,
    /// Sugestões: (id, nome).
    modelos: Vec<(String, String)>,
    padrao: String,
    /// Último modelo usado de verdade por esta família nesta CLI.
    ultimo: Option<String>,
}

#[derive(Template)]
#[template(path = "sistema_familia.html")]
struct PaginaFamiliaSistema {
    s: Sessao,
    f: LinhaFamilia,
    membros: Vec<(String, String, String)>,
    uso: Vec<(String, String, i64)>,
    clis: Vec<OpcaoCli>,
    /// CLIs que geram imagem (codex, agy), na ordem da família.
    clis_imagem: Vec<OpcaoCli>,
    /// Fontes de IA na ordem da família: (chave, rótulo).
    fontes: Vec<(&'static str, &'static str)>,
    /// Modelo e esforço de cada CLI conectada.
    ajustes: Vec<AjusteTela>,
    esforcos: Vec<(&'static str, &'static str)>,
    novo_link: Option<String>,
    aviso: Option<String>,
}

fn conectado(situacao: &str) -> bool {
    !situacao.starts_with("não conectado") && situacao != "indisponível"
}

/// CLIs para marcar: os da família na ordem dela e depois os outros conectados na plataforma.
/// Um CLI desconectado só aparece se a família já tinha (para poder desmarcar).
fn opcoes_cli(da_familia: &[String], situacao: &[(String, String)]) -> Vec<OpcaoCli> {
    let sit = |c: &str| situacao.iter().find(|(n, _)| n == c).map(|(_, s)| s.clone()).unwrap_or_else(|| "não conectado".into());
    let mut v: Vec<OpcaoCli> = da_familia
        .iter()
        .filter(|c| ["claude", "codex", "agy"].contains(&c.as_str()))
        .map(|c| OpcaoCli { chave: c.clone(), nome: nome_cli(c), situacao: sit(c), marcado: true })
        .collect();
    for (c, st) in situacao {
        if !v.iter().any(|o| o.chave == *c) && conectado(st) {
            v.push(OpcaoCli { chave: c.clone(), nome: nome_cli(c), situacao: st.clone(), marcado: false });
        }
    }
    v
}

async fn pagina_familia(st: &AppState, s: Sessao, id: Uuid, novo_link: Option<String>, aviso: Option<String>) -> Response {
    let f = match sqlx::query_as::<_, LinhaFamilia>(&format!("{SELECT_FAMILIA} WHERE f.id = $1")).bind(id).fetch_optional(&st.db).await {
        Ok(Some(f)) => f,
        Ok(None) => return Redirect::to("/sistema").into_response(),
        Err(e) => return erro_interno(e.into()),
    };
    let membros: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT u.nome, u.email, m.papel FROM acesso.membros m JOIN acesso.usuarios u ON u.id = m.usuario_id
          WHERE m.familia_id = $1 ORDER BY m.papel, u.nome",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    let uso: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT via, provedor, count(*) FROM ia.uso WHERE familia_id = $1 AND criado_em > now() - interval '7 days'
          GROUP BY via, provedor ORDER BY count(*) DESC",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    let situacao = crate::cli_login::situacao().await;
    let clis = opcoes_cli(&f.clis, &situacao);
    let so_imagem: Vec<(String, String)> = situacao.iter().filter(|(c, _)| crate::ia::CLIS_IMAGEM.contains(&c.as_str())).cloned().collect();
    let clis_imagem = opcoes_cli(&f.clis_imagem, &so_imagem).into_iter().filter(|o| crate::ia::CLIS_IMAGEM.contains(&o.chave.as_str())).collect();
    let fontes = crate::ia::ordem_fontes(&f.ordem_fontes)
        .into_iter()
        .map(|x| (x, match x {
            "cli" => "🤖 Assinaturas (CLI)",
            "familia" => "🔑 Chave de API da própria família",
            _ => "🏢 Chave de API do sistema (se liberada abaixo)",
        }))
        .collect();
    let mut ajustes = Vec::new();
    for (c, sit) in &situacao {
        if !conectado(sit) {
            continue;
        }
        let a = crate::ia::AjusteCli::de(&f.cli_ajustes, c);
        let ultimo: Option<String> = sqlx::query_scalar(
            "SELECT modelo FROM ia.uso WHERE familia_id = $1 AND via = 'cli' AND provedor = $2 AND modelo IS NOT NULL ORDER BY id DESC LIMIT 1",
        )
        .bind(id)
        .bind(c)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
        ajustes.push(AjusteTela {
            chave: c.clone(),
            nome: nome_cli(c),
            modelo: a.modelo.unwrap_or_default(),
            esforco: a.esforco.unwrap_or_default(),
            modelos: crate::ia::modelos(c).await,
            padrao: crate::ia::modelo_padrao(c),
            ultimo,
        });
    }
    render(PaginaFamiliaSistema {
        s, f, membros, uso, clis, clis_imagem, fontes, ajustes, esforcos: crate::ia::ESFORCOS.to_vec(), novo_link, aviso,
    })
}

/// GET /sistema/familias/{id}
pub async fn ver_familia(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let aviso = q.contains_key("salvo").then(|| "Salvo ✓".to_string());
    pagina_familia(&st, s, id, None, aviso).await
}

/// POST /sistema/familias: cria a família e gera o link de convite para o admin dela.
/// O nome pode repetir (o @ ganha -2, -3...); o admin aparece na lista para diferenciar.
pub async fn criar_familia(State(st): State<AppState>, s: Sessao, RawForm(corpo): RawForm) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let form = campos(&corpo);
    let nome = campo(&form, "nome");
    if nome.is_empty() {
        return pagina(&st, s, aba("familias"), Some("Dê um nome para a família.".into())).await;
    }
    let id = match familia::criar_familia(&st, &nome).await {
        Ok(id) => id,
        Err(e) => return erro_interno(e),
    };
    match familia::criar_convite_para(&st, id, "admin", s.usuario_id).await {
        Ok(link) => pagina_familia(&st, s, id, Some(link), Some(format!("Família {nome} criada ✓"))).await,
        Err(e) => erro_interno(e),
    }
}

/// POST /sistema/familias/{id}: CLIs (em ordem), chave do sistema e limite.
pub async fn salvar_familia(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, RawForm(corpo): RawForm) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let form = campos(&corpo);
    // Os "cli" marcados chegam na ordem da tela.
    let mut clis: Vec<String> = Vec::new();
    for (k, v) in &form {
        if k == "cli" && ["claude", "codex", "agy"].contains(&v.as_str()) && !clis.contains(v) {
            clis.push(v.clone());
        }
    }
    let mut clis_imagem: Vec<String> = Vec::new();
    for (k, v) in &form {
        if k == "cli_imagem" && crate::ia::CLIS_IMAGEM.contains(&v.as_str()) && !clis_imagem.contains(v) {
            clis_imagem.push(v.clone());
        }
    }
    let fontes: Vec<String> = crate::ia::ordem_fontes(
        &form.iter().filter(|(k, _)| k == "fonte").map(|(_, v)| v.clone()).collect::<Vec<_>>(),
    )
    .into_iter()
    .map(String::from)
    .collect();
    // Modelo e esforço de cada CLI (vazio = padrão dela).
    let mut ajustes = serde_json::Map::new();
    for c in ["claude", "codex", "agy"] {
        let modelo = campo(&form, &format!("modelo_{c}"));
        let esforco = campo(&form, &format!("esforco_{c}"));
        let mut o = serde_json::Map::new();
        if crate::ia::cli_nome_seguro(modelo.trim()) {
            o.insert("modelo".into(), modelo.trim().into());
        }
        if crate::ia::ESFORCOS.iter().any(|(k, _)| *k == esforco) {
            o.insert("esforco".into(), esforco.into());
        }
        if !o.is_empty() {
            ajustes.insert(c.into(), o.into());
        }
    }
    let modo = Some(campo(&form, "modo_cli")).filter(|m| ["ordem", "rodizio", "fixo"].contains(&m.as_str())).unwrap_or_else(|| "ordem".into());
    let chave = form.iter().any(|(k, _)| k == "chave_sistema");
    let limite: Option<i64> = campo(&form, "limite").parse().ok().filter(|n: &i64| *n > 0);
    let limites = match limite {
        Some(n) => serde_json::json!({ "historias_semana": n }),
        None => serde_json::json!({}),
    };
    let r = sqlx::query(
        "INSERT INTO ia.config_familia (familia_id, cli_habilitado, clis, usar_chave_sistema, limites, modo_cli, clis_imagem, ordem_fontes, cli_ajustes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (familia_id) DO UPDATE SET cli_habilitado = $2, clis = $3, usar_chave_sistema = $4, limites = $5, modo_cli = $6,
                                                clis_imagem = $7, ordem_fontes = $8, cli_ajustes = $9",
    )
    .bind(id)
    .bind(!clis.is_empty())
    .bind(&clis)
    .bind(chave)
    .bind(limites)
    .bind(&modo)
    .bind(&clis_imagem)
    .bind(&fontes)
    .bind(serde_json::Value::Object(ajustes))
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to(&format!("/sistema/familias/{id}?salvo=1")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn convite_admin(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    match familia::criar_convite_para(&st, id, "admin", s.usuario_id).await {
        Ok(link) => pagina_familia(&st, s, id, Some(link), None).await,
        Err(e) => erro_interno(e),
    }
}

pub async fn salvar_chave(State(st): State<AppState>, s: Sessao, RawForm(corpo): RawForm) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let form = campos(&corpo);
    let (provedor, chave) = (campo(&form, "provedor"), campo(&form, "chave"));
    if !PROVEDORES.iter().any(|(p, _)| *p == provedor) || provedor == "elevenlabs" || chave.len() < 10 {
        return pagina(&st, s, aba("config"), Some("Chave inválida.".into())).await;
    }
    match familia::gravar_chave(&st, None, &provedor, &chave).await {
        Ok(()) => pagina(&st, s, aba("config"), Some("Chave do sistema salva ✓".into())).await,
        Err(e) => erro_interno(e),
    }
}

pub async fn remover_chave(State(st): State<AppState>, s: Sessao, Path(provedor): Path<String>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    match sqlx::query("DELETE FROM ia.chaves WHERE familia_id IS NULL AND provedor = $1")
        .bind(provedor)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/sistema?aba=config").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── configurações gerais (o que antes ficava no .env) ─────────────

/// POST /sistema/config: salva as configurações. Segredos em branco mantêm o valor atual.
pub async fn salvar_config(State(st): State<AppState>, s: Sessao, RawForm(corpo): RawForm) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let form = campos(&corpo);
    for (chave, secreta) in crate::config::CHAVES {
        let apagar = form.iter().any(|(k, _)| k == &format!("apagar_{chave}"));
        let Some((_, valor)) = form.iter().find(|(k, _)| k == chave) else {
            if apagar {
                let _ = crate::config::salvar(&st, chave, "").await;
            }
            continue;
        };
        if secreta && valor.trim().is_empty() && !apagar {
            continue; // segredo em branco = manter
        }
        if let Err(e) = crate::config::salvar(&st, chave, if apagar { "" } else { valor }).await {
            return erro_interno(e);
        }
    }
    pagina(&st, s, aba("config"), Some("Configurações salvas ✓".into())).await
}

/// POST /sistema/email-teste: manda um e-mail de teste para o admin.
pub async fn email_teste(State(st): State<AppState>, s: Sessao) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let email: String = sqlx::query_scalar("SELECT email FROM acesso.usuarios WHERE id = $1")
        .bind(s.usuario_id)
        .fetch_one(&st.db)
        .await
        .unwrap_or_default();
    let aviso = if !crate::email::configurado() {
        "Preencha usuário e senha do e-mail primeiro.".to_string()
    } else {
        match crate::email::enviar(&email, "Storyteller: e-mail de teste", "Se você recebeu isto, o envio de e-mails está funcionando.").await {
            Ok(()) => format!("E-mail de teste enviado para {email} ✓"),
            Err(e) => format!("Falhou: {e:#}"),
        }
    };
    pagina(&st, s, aba("config"), Some(aviso)).await
}

// ───────────── login dos CLIs pelo site ─────────────

pub async fn cli_iniciar(State(st): State<AppState>, s: Sessao, Path(nome): Path<String>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    match crate::cli_login::iniciar(&st, &nome).await {
        Ok(e) => axum::Json(e).into_response(),
        Err(e) => crate::web::erro_json(format!("{e:#}")),
    }
}

pub async fn cli_estado(s: Sessao, Path(nome): Path<String>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    axum::Json(crate::cli_login::estado(&nome).await).into_response()
}

#[derive(serde::Deserialize)]
pub struct PedidoCodigo {
    codigo: String,
}

pub async fn cli_codigo(s: Sessao, Path(nome): Path<String>, axum::Json(p): axum::Json<PedidoCodigo>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    match crate::cli_login::enviar_codigo(&nome, &p.codigo).await {
        Ok(e) => axum::Json(e).into_response(),
        Err(e) => crate::web::erro_json(format!("{e:#}")),
    }
}

/// POST /sistema/cli/{nome}/testar: um pedido curtinho para confirmar que o CLI responde.
pub async fn cli_testar(s: Sessao, Path(nome): Path<String>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let pedido = crate::ia::PedidoTexto {
        sistema: "Responda só com o JSON pedido.".into(),
        imagens: Vec::new(),
        prompt: "Diga oi em português.".into(),
        schema: serde_json::json!({ "type": "object", "additionalProperties": false, "required": ["resposta"],
                                    "properties": { "resposta": { "type": "string" } } }),
    };
    // Esforço baixo: o teste só confere se responde (com o padrão o agy leva mais de 1 minuto).
    let a = crate::ia::AjusteCli { modelo: None, esforco: Some("low".into()) };
    let r = match nome.as_str() {
        "claude" => crate::ia::claude_cli(&pedido, &a).await,
        "agy" => crate::ia::testar_agy(&pedido, &a).await,
        "codex" => crate::ia::codex_cli(&pedido, &a).await,
        _ => Err(anyhow::anyhow!("CLI desconhecido")),
    };
    crate::cli_login::anotar_teste(&nome, r.as_ref().map(|(_, m)| m.clone()).map_err(|e| format!("{e:#}")));
    let situacao = crate::cli_login::situacao().await.into_iter().find(|(n, _)| *n == nome).map(|(_, s)| s);
    match r {
        Ok((v, modelo)) => {
            axum::Json(serde_json::json!({ "ok": true, "resposta": v["resposta"], "modelo": modelo, "situacao": situacao })).into_response()
        }
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "erro": format!("{e:#}"), "situacao": situacao })).into_response(),
    }
}

/// POST /sistema/limpar: roda a limpeza de arquivos agora.
pub async fn limpar_agora(State(st): State<AppState>, s: Sessao) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let r = crate::limpeza::limpar(&st).await;
    let ok = r.is_ok();
    let texto = crate::limpeza::registrar(&st, r).await;
    let aviso = if ok { format!("Limpeza feita ✓ {texto} (arquivos com menos de 24 h ficam).") } else { format!("A limpeza {texto}") };
    pagina(&st, s, aba("manutencao"), Some(aviso)).await
}

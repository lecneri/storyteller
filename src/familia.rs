//! Família: configurações, membros e papéis, convites por link, chaves de API próprias e troca de família.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{self, Sessao},
    segredos, util,
    web::{erro_interno, render},
    AppState,
};

pub const PROVEDORES: [(&str, &str); 4] = [
    ("anthropic", "Anthropic (Claude)"),
    ("openai", "OpenAI (GPT / imagens)"),
    ("gemini", "Google Gemini (texto / imagens)"),
    ("kimi", "Kimi (Moonshot)"),
];

#[derive(sqlx::FromRow)]
struct Membro {
    usuario_id: Uuid,
    nome: String,
    email: String,
    papel: String,
}

#[derive(sqlx::FromRow)]
struct Convite {
    token_hash: String,
    papel: String,
    expira: String,
}

#[derive(sqlx::FromRow)]
struct Config {
    nome: String,
    slug: String,
    visibilidade_padrao: String,
    cli_habilitado: Option<bool>,
    usar_chave_sistema: Option<bool>,
}

#[derive(Template)]
#[template(path = "familia.html")]
struct PaginaFamilia {
    s: Sessao,
    cfg: Config,
    membros: Vec<Membro>,
    convites: Vec<Convite>,
    chaves: Vec<(String, String)>,
    provedores: Vec<(&'static str, &'static str)>,
    linhas_chaves: Vec<crate::chaves::LinhaChave>,
    cripto_ok: bool,
    novo_link: Option<String>,
    aviso: Option<String>,
    conectadas: Vec<(Uuid, String, String)>,
    pedidos: Vec<(Uuid, String, String)>,
}

impl PaginaFamilia {
}

async fn pagina(st: &AppState, s: Sessao, novo_link: Option<String>, aviso: Option<String>) -> Response {
    let cfg = sqlx::query_as::<_, Config>(
        "SELECT f.nome, f.slug, f.visibilidade_padrao, c.cli_habilitado, c.usar_chave_sistema
           FROM acesso.familias f LEFT JOIN ia.config_familia c ON c.familia_id = f.id WHERE f.id = $1",
    )
    .bind(s.familia_id)
    .fetch_one(&st.db)
    .await;
    let membros = sqlx::query_as::<_, Membro>(
        "SELECT u.id AS usuario_id, u.nome, u.email, m.papel
           FROM acesso.membros m JOIN acesso.usuarios u ON u.id = m.usuario_id
          WHERE m.familia_id = $1 ORDER BY m.papel, u.nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    let convites = sqlx::query_as::<_, Convite>(
        "SELECT token_hash, papel, to_char(expira_em, 'DD/MM HH24:MI') AS expira
           FROM acesso.convites WHERE familia_id = $1 AND usado_em IS NULL AND expira_em > now() ORDER BY criado_em DESC",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    let chaves = sqlx::query_as::<_, (String, String)>("SELECT provedor, final FROM ia.chaves WHERE familia_id = $1")
        .bind(s.familia_id)
        .fetch_all(&st.db)
        .await;
    let linhas_chaves = crate::chaves::linhas(st, Some(s.familia_id)).await;
    let conectadas = crate::partilha::conectadas(st, s.familia_id).await.unwrap_or_default();
    let pedidos = crate::partilha::pedidos_recebidos(st, s.familia_id).await.unwrap_or_default();
    match (cfg, membros, convites, chaves) {
        (Ok(cfg), Ok(membros), Ok(convites), Ok(chaves)) => render(PaginaFamilia {
            conectadas,
            pedidos,
            s,
            cfg,
            membros,
            convites,
            chaves,
            provedores: PROVEDORES.to_vec(),
            linhas_chaves,
            cripto_ok: segredos::disponivel(),
            novo_link,
            aviso,
        }),
        (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (.., Err(e)) => erro_interno(e.into()),
    }
}

#[derive(Deserialize, Default)]
pub struct AvisoFamilia {
    conexao: Option<String>,
}

pub async fn ver(State(st): State<AppState>, s: Sessao, axum::extract::Query(q): axum::extract::Query<AvisoFamilia>) -> Response {
    let aviso = q.conexao.map(|c| match c.as_str() {
        "nao-encontrada" => "Não encontrei uma família com esse nome único.".to_string(),
        _ => "Pedido de conexão enviado ✓ (a outra família precisa aceitar)".to_string(),
    });
    pagina(&st, s, None, aviso).await
}

fn so_admin(s: &Sessao) -> Option<Response> {
    (!s.admin_familia()).then(|| Redirect::to("/familia").into_response())
}

#[derive(Deserialize)]
pub struct FormConfig {
    nome: String,
    visibilidade_padrao: String,
}

pub async fn salvar_config(State(st): State<AppState>, s: Sessao, Form(f): Form<FormConfig>) -> Response {
    if let Some(r) = so_admin(&s) {
        return r;
    }
    let vis = if f.visibilidade_padrao == "compartilhado" { "compartilhado" } else { "privado" };
    let nome = f.nome.trim();
    if nome.is_empty() {
        return pagina(&st, s, None, Some("Dê um nome para a família.".into())).await;
    }
    match sqlx::query("UPDATE acesso.familias SET nome = $2, visibilidade_padrao = $3 WHERE id = $1")
        .bind(s.familia_id)
        .bind(nome)
        .bind(vis)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/familia").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── convites ─────────────

fn url_publica() -> String {
    crate::email::url_publica()
}

/// Cria um convite e devolve o link (o token só existe nesse momento; no banco fica o hash).
pub async fn criar_convite_para(st: &AppState, familia_id: Uuid, papel: &str, criado_por: Uuid) -> anyhow::Result<String> {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    sqlx::query(
        "INSERT INTO acesso.convites (token_hash, familia_id, papel, criado_por, expira_em)
         VALUES ($1, $2, $3, $4, now() + interval '7 days')",
    )
    .bind(auth::hash_token(&token))
    .bind(familia_id)
    .bind(papel)
    .bind(criado_por)
    .execute(&st.db)
    .await?;
    Ok(format!("{}/convite/{token}", url_publica()))
}

#[derive(Deserialize)]
pub struct FormConvite {
    papel: String,
    #[serde(default)]
    email: String,
}

pub async fn criar_convite(State(st): State<AppState>, s: Sessao, Form(f): Form<FormConvite>) -> Response {
    if let Some(r) = so_admin(&s) {
        return r;
    }
    let papel = if f.papel == "admin" { "admin" } else { "membro" };
    match criar_convite_para(&st, s.familia_id, papel, s.usuario_id).await {
        Ok(link) => {
            let para = f.email.trim();
            let aviso = if para.contains('@') && crate::email::configurado() {
                crate::email::enviar_em_segundo_plano(
                    para.to_string(),
                    format!("Convite para a {} no Storyteller", s.familia_nome),
                    format!("{} convidou você para a {} no Storyteller (histórias infantis).\n\nAbra o link para aceitar (vale 7 dias):\n{link}", s.usuario_nome, s.familia_nome),
                );
                Some(format!("Convite enviado para {para} ✓"))
            } else {
                None
            };
            pagina(&st, s, Some(link), aviso).await
        }
        Err(e) => erro_interno(e),
    }
}

pub async fn revogar_convite(State(st): State<AppState>, s: Sessao, Path(hash): Path<String>) -> Response {
    if let Some(r) = so_admin(&s) {
        return r;
    }
    match sqlx::query("DELETE FROM acesso.convites WHERE token_hash = $1 AND familia_id = $2 AND usado_em IS NULL")
        .bind(hash)
        .bind(s.familia_id)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/familia").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[derive(sqlx::FromRow)]
struct ConviteValido {
    familia_id: Uuid,
    familia_nome: String,
    papel: String,
}

async fn convite_valido(st: &AppState, token: &str) -> Result<Option<ConviteValido>, sqlx::Error> {
    sqlx::query_as::<_, ConviteValido>(
        "SELECT c.familia_id, f.nome AS familia_nome, c.papel
           FROM acesso.convites c JOIN acesso.familias f ON f.id = c.familia_id
          WHERE c.token_hash = $1 AND c.usado_em IS NULL AND c.expira_em > now()",
    )
    .bind(auth::hash_token(token))
    .fetch_optional(&st.db)
    .await
}

#[derive(Template)]
#[template(path = "convite.html")]
struct PaginaConvite {
    familia: String,
    papel: String,
    logado: Option<String>,
    erro: Option<String>,
}

/// GET /convite/{token}: aceitar (se já tem conta) ou criar conta.
pub async fn abrir_convite(State(st): State<AppState>, jar: CookieJar, Path(token): Path<String>) -> Response {
    let c = match convite_valido(&st, &token).await {
        Ok(Some(c)) => c,
        Ok(None) => return render(PaginaConvite { familia: String::new(), papel: String::new(), logado: None, erro: Some("Convite inválido ou vencido. Peça um novo link.".into()) }),
        Err(e) => return erro_interno(e.into()),
    };
    let logado = usuario_logado(&st, &jar).await.map(|(_, nome)| nome);
    render(PaginaConvite { familia: c.familia_nome, papel: c.papel, logado, erro: None })
}

async fn usuario_logado(st: &AppState, jar: &CookieJar) -> Option<(Uuid, String)> {
    let token = jar.get(auth::COOKIE_SESSAO)?.value().to_owned();
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT u.id, u.nome FROM acesso.sessoes s JOIN acesso.usuarios u ON u.id = s.usuario_id
          WHERE s.token_hash = $1 AND s.expira_em > now()",
    )
    .bind(auth::hash_token(&token))
    .fetch_optional(&st.db)
    .await
    .ok()
    .flatten()
}

#[derive(Deserialize)]
pub struct FormAceitar {
    #[serde(default)]
    nome: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    senha: String,
}

pub async fn aceitar_convite(
    State(st): State<AppState>,
    jar: CookieJar,
    Path(token): Path<String>,
    Form(f): Form<FormAceitar>,
) -> Response {
    let c = match convite_valido(&st, &token).await {
        Ok(Some(c)) => c,
        Ok(None) => return Redirect::to(&format!("/convite/{token}")).into_response(),
        Err(e) => return erro_interno(e.into()),
    };
    let erro_form = |msg: &str| {
        render(PaginaConvite { familia: c.familia_nome.clone(), papel: c.papel.clone(), logado: None, erro: Some(msg.into()) })
    };

    // Já logado: só entra na família. Senão: cria a conta.
    let (usuario_id, nova_sessao) = match usuario_logado(&st, &jar).await {
        Some((id, _)) => (id, false),
        None => {
            let (nome, email) = (f.nome.trim(), f.email.trim());
            if nome.is_empty() || !email.contains('@') || f.senha.len() < 8 {
                return erro_form("Preencha nome, e-mail e uma senha com pelo menos 8 caracteres.");
            }
            let existe: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM acesso.usuarios WHERE lower(email) = lower($1))")
                .bind(email)
                .fetch_one(&st.db)
                .await
                .unwrap_or(false);
            if existe {
                return erro_form("Este e-mail já tem conta. Entre primeiro e depois abra o link de novo.");
            }
            let hash = match auth::gerar_hash_senha(&f.senha) {
                Ok(h) => h,
                Err(e) => return erro_interno(e),
            };
            match sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO acesso.usuarios (email, nome, senha_hash) VALUES ($1, $2, $3) RETURNING id",
            )
            .bind(email)
            .bind(nome)
            .bind(hash)
            .fetch_one(&st.db)
            .await
            {
                Ok(id) => (id, true),
                Err(e) => return erro_interno(e.into()),
            }
        }
    };

    let r = async {
        let mut tx = st.db.begin().await?;
        // Marca como usado só se ainda estiver livre (evita uso duplo simultâneo).
        let usado = sqlx::query(
            "UPDATE acesso.convites SET usado_em = now(), usado_por = $2 WHERE token_hash = $1 AND usado_em IS NULL",
        )
        .bind(auth::hash_token(&token))
        .bind(usuario_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if usado == 1 {
            sqlx::query(
                "INSERT INTO acesso.membros (familia_id, usuario_id, papel) VALUES ($1, $2, $3)
                 ON CONFLICT (familia_id, usuario_id) DO NOTHING",
            )
            .bind(c.familia_id)
            .bind(usuario_id)
            .bind(&c.papel)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    if nova_sessao {
        match auth::criar_sessao(&st.db, usuario_id, Some(c.familia_id)).await {
            Ok(t) => (jar.add(auth::cookie_sessao(t, st.cookie_seguro)), Redirect::to("/")).into_response(),
            Err(e) => erro_interno(e),
        }
    } else {
        trocar_para(&st, &jar, usuario_id, c.familia_id).await
    }
}

// ───────────── membros ─────────────

#[derive(Deserialize)]
pub struct FormPapel {
    papel: String,
}

pub async fn mudar_papel(State(st): State<AppState>, s: Sessao, Path(usuario): Path<Uuid>, Form(f): Form<FormPapel>) -> Response {
    if let Some(r) = so_admin(&s) {
        return r;
    }
    let papel = if f.papel == "admin" { "admin" } else { "membro" };
    if usuario == s.usuario_id && papel != "admin" && admins(&st, s.familia_id).await <= 1 {
        return pagina(&st, s, None, Some("A família precisa de pelo menos um admin.".into())).await;
    }
    match sqlx::query("UPDATE acesso.membros SET papel = $3 WHERE familia_id = $1 AND usuario_id = $2")
        .bind(s.familia_id)
        .bind(usuario)
        .bind(papel)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/familia").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

async fn admins(st: &AppState, familia: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM acesso.membros WHERE familia_id = $1 AND papel = 'admin'")
        .bind(familia)
        .fetch_one(&st.db)
        .await
        .unwrap_or(0)
}

pub async fn remover_membro(State(st): State<AppState>, s: Sessao, Path(usuario): Path<Uuid>) -> Response {
    if let Some(r) = so_admin(&s) {
        return r;
    }
    if usuario == s.usuario_id {
        return pagina(&st, s, None, Some("Você não pode remover a si mesmo.".into())).await;
    }
    match sqlx::query("DELETE FROM acesso.membros WHERE familia_id = $1 AND usuario_id = $2")
        .bind(s.familia_id)
        .bind(usuario)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/familia").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── chaves de API ─────────────

#[derive(Deserialize)]
pub struct FormChave {
    provedor: String,
    chave: String,
}

/// Grava (ou troca) uma chave cifrada. familia = None → chave do sistema.
pub async fn gravar_chave(st: &AppState, familia: Option<Uuid>, provedor: &str, chave: &str) -> anyhow::Result<()> {
    let chave = chave.trim();
    let cifrada = segredos::cifrar(chave)?;
    let final_: String = chave.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let mut tx = st.db.begin().await?;
    sqlx::query("DELETE FROM ia.chaves WHERE familia_id IS NOT DISTINCT FROM $1 AND provedor = $2")
        .bind(familia)
        .bind(provedor)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO ia.chaves (familia_id, provedor, cifrada, final) VALUES ($1, $2, $3, $4)")
        .bind(familia)
        .bind(provedor)
        .bind(cifrada)
        .bind(final_)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn salvar_chave(State(st): State<AppState>, s: Sessao, Form(f): Form<FormChave>) -> Response {
    if let Some(r) = so_admin(&s) {
        return r;
    }
    if !PROVEDORES.iter().any(|(p, _)| *p == f.provedor) || f.chave.trim().len() < 10 {
        return pagina(&st, s, None, Some("Chave inválida.".into())).await;
    }
    match gravar_chave(&st, Some(s.familia_id), &f.provedor, &f.chave).await {
        Ok(()) => pagina(&st, s, None, Some("Chave salva ✓".into())).await,
        Err(e) => erro_interno(e),
    }
}

pub async fn remover_chave(State(st): State<AppState>, s: Sessao, Path(provedor): Path<String>) -> Response {
    if let Some(r) = so_admin(&s) {
        return r;
    }
    match sqlx::query("DELETE FROM ia.chaves WHERE familia_id = $1 AND provedor = $2")
        .bind(s.familia_id)
        .bind(provedor)
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/familia").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── troca de família ─────────────

#[derive(Deserialize)]
pub struct FormTrocar {
    familia: Uuid,
}

pub async fn trocar(State(st): State<AppState>, s: Sessao, jar: CookieJar, Form(f): Form<FormTrocar>) -> Response {
    trocar_para(&st, &jar, s.usuario_id, f.familia).await
}

async fn trocar_para(st: &AppState, jar: &CookieJar, usuario: Uuid, familia: Uuid) -> Response {
    let Some(token) = jar.get(auth::COOKIE_SESSAO).map(|c| c.value().to_owned()) else {
        return Redirect::to("/entrar").into_response();
    };
    let r = sqlx::query(
        "UPDATE acesso.sessoes SET familia_id = $3
          WHERE token_hash = $1 AND usuario_id = $2
            AND EXISTS (SELECT 1 FROM acesso.membros WHERE usuario_id = $2 AND familia_id = $3)",
    )
    .bind(auth::hash_token(&token))
    .bind(usuario)
    .bind(familia)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to("/").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// Cria uma família nova (usado pelo admin do sistema). Devolve o id.
pub async fn criar_familia(st: &AppState, nome: &str) -> anyhow::Result<Uuid> {
    let base = util::slug(nome);
    let mut slug = base.clone();
    for n in 2..100 {
        let existe: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM acesso.familias WHERE slug = $1)")
            .bind(&slug)
            .fetch_one(&st.db)
            .await?;
        if !existe {
            break;
        }
        slug = format!("{base}-{n}");
    }
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO acesso.familias (slug, nome) VALUES ($1, $2) RETURNING id")
        .bind(&slug)
        .bind(nome)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO ia.config_familia (familia_id) VALUES ($1)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}

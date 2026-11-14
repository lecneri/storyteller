//! Configurações do sistema (só admin do sistema): famílias, flags de CLI e chave do sistema, limites e chaves.

use askama::Template;
use axum::{
    extract::{Path, RawForm, State},
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
    membros: i64,
    historias: i64,
    cli_habilitado: bool,
    usar_chave_sistema: bool,
    limite_semana: Option<i64>,
    uso_semana: i64,
    espaco_mb: f64,
}

#[derive(Template)]
#[template(path = "sistema.html")]
struct PaginaSistema {
    s: Sessao,
    familias: Vec<LinhaFamilia>,
    chaves: Vec<(String, String)>,
    provedores: Vec<(&'static str, &'static str)>,
    linhas_chaves: Vec<crate::chaves::LinhaChave>,
    cripto_ok: bool,
    novo_link: Option<(String, String)>,
    aviso: Option<String>,
    clis: Vec<(String, String)>,
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
}

fn so_admin_sistema(s: &Sessao) -> Option<Response> {
    (!s.admin_sistema).then(|| Redirect::to("/").into_response())
}

async fn pagina(st: &AppState, s: Sessao, novo_link: Option<(String, String)>, aviso: Option<String>) -> Response {
    let familias = sqlx::query_as::<_, LinhaFamilia>(
        "SELECT f.id, f.nome, f.slug,
                (SELECT count(*) FROM acesso.membros m WHERE m.familia_id = f.id) AS membros,
                (SELECT count(*) FROM conteudo.historias h WHERE h.familia_id = f.id) AS historias,
                COALESCE(c.cli_habilitado, false) AS cli_habilitado,
                COALESCE(c.usar_chave_sistema, false) AS usar_chave_sistema,
                (c.limites->>'historias_semana')::bigint AS limite_semana,
                (SELECT count(*) FROM ia.uso u WHERE u.familia_id = f.id AND u.criado_em > now() - interval '7 days'
                    AND u.via IN ('cli', 'chave_sistema')) AS uso_semana,
                COALESCE((SELECT sum(a.bytes) FROM midia.arquivos a WHERE a.familia_id = f.id), 0)::float8 / 1048576.0 AS espaco_mb
           FROM acesso.familias f LEFT JOIN ia.config_familia c ON c.familia_id = f.id
          ORDER BY f.nome",
    )
    .fetch_all(&st.db)
    .await;
    let chaves = sqlx::query_as::<_, (String, String)>("SELECT provedor, final FROM ia.chaves WHERE familia_id IS NULL")
        .fetch_all(&st.db)
        .await;
    let clis = crate::cli_login::situacao().await;
    match (familias, chaves) {
        (Ok(familias), Ok(chaves)) => render(PaginaSistema {
            clis,
            s,
            familias,
            chaves,
            provedores: PROVEDORES.to_vec(),
            linhas_chaves: crate::chaves::linhas(st, None).await,
            cripto_ok: segredos::disponivel(),
            novo_link,
            aviso,
        }),
        (Err(e), _) | (_, Err(e)) => erro_interno(e.into()),
    }
}

pub async fn ver(State(st): State<AppState>, s: Sessao) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    pagina(&st, s, None, None).await
}

fn campos(corpo: &[u8]) -> Vec<(String, String)> {
    serde_urlencoded::from_bytes(corpo).unwrap_or_default()
}
fn campo(form: &[(String, String)], k: &str) -> String {
    form.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_string()).unwrap_or_default()
}

/// POST /sistema/familias: cria a família e gera o link de convite para o admin dela.
pub async fn criar_familia(State(st): State<AppState>, s: Sessao, RawForm(corpo): RawForm) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let form = campos(&corpo);
    let nome = campo(&form, "nome");
    if nome.is_empty() {
        return pagina(&st, s, None, Some("Dê um nome para a família.".into())).await;
    }
    let id = match familia::criar_familia(&st, &nome).await {
        Ok(id) => id,
        Err(e) => return erro_interno(e),
    };
    match familia::criar_convite_para(&st, id, "admin", s.usuario_id).await {
        Ok(link) => pagina(&st, s, Some((nome, link)), None).await,
        Err(e) => erro_interno(e),
    }
}

/// POST /sistema/familias/{id}: flags e limite.
pub async fn salvar_familia(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, RawForm(corpo): RawForm) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let form = campos(&corpo);
    let cli = form.iter().any(|(k, _)| k == "cli");
    let chave = form.iter().any(|(k, _)| k == "chave_sistema");
    let limite: Option<i64> = campo(&form, "limite").parse().ok().filter(|n: &i64| *n > 0);
    let limites = match limite {
        Some(n) => serde_json::json!({ "historias_semana": n }),
        None => serde_json::json!({}),
    };
    let r = sqlx::query(
        "INSERT INTO ia.config_familia (familia_id, cli_habilitado, usar_chave_sistema, limites) VALUES ($1, $2, $3, $4)
         ON CONFLICT (familia_id) DO UPDATE SET cli_habilitado = $2, usar_chave_sistema = $3, limites = $4",
    )
    .bind(id)
    .bind(cli)
    .bind(chave)
    .bind(limites)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to("/sistema").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn convite_admin(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let nome: String = match sqlx::query_scalar("SELECT nome FROM acesso.familias WHERE id = $1").bind(id).fetch_one(&st.db).await {
        Ok(n) => n,
        Err(e) => return erro_interno(e.into()),
    };
    match familia::criar_convite_para(&st, id, "admin", s.usuario_id).await {
        Ok(link) => pagina(&st, s, Some((nome, link)), None).await,
        Err(e) => erro_interno(e),
    }
}

pub async fn salvar_chave(State(st): State<AppState>, s: Sessao, RawForm(corpo): RawForm) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let form = campos(&corpo);
    let (provedor, chave) = (campo(&form, "provedor"), campo(&form, "chave"));
    if !PROVEDORES.iter().any(|(p, _)| *p == provedor) || chave.len() < 10 {
        return pagina(&st, s, None, Some("Chave inválida.".into())).await;
    }
    match familia::gravar_chave(&st, None, &provedor, &chave).await {
        Ok(()) => pagina(&st, s, None, Some("Chave do sistema salva ✓".into())).await,
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
        Ok(_) => Redirect::to("/sistema").into_response(),
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
    pagina(&st, s, None, Some("Configurações salvas ✓".into())).await
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
    pagina(&st, s, None, Some(aviso)).await
}

// ───────────── login dos CLIs pelo site ─────────────

pub async fn cli_iniciar(State(st): State<AppState>, s: Sessao, Path(nome): Path<String>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    match crate::cli_login::iniciar(&st, &nome).await {
        Ok(e) => axum::Json(e).into_response(),
        Err(e) => (axum::http::StatusCode::UNPROCESSABLE_ENTITY, axum::Json(serde_json::json!({ "erro": format!("{e:#}") }))).into_response(),
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
        Err(e) => (axum::http::StatusCode::UNPROCESSABLE_ENTITY, axum::Json(serde_json::json!({ "erro": format!("{e:#}") }))).into_response(),
    }
}

/// POST /sistema/cli/{nome}/testar: um pedido curtinho para confirmar que o CLI responde.
pub async fn cli_testar(s: Sessao, Path(nome): Path<String>) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let pedido = crate::ia::PedidoTexto {
        sistema: "Responda só com o JSON pedido.".into(),
        prompt: "Diga oi em português.".into(),
        schema: serde_json::json!({ "type": "object", "additionalProperties": false, "required": ["resposta"],
                                    "properties": { "resposta": { "type": "string" } } }),
    };
    let r = match nome.as_str() {
        "claude" => crate::ia::claude_cli(&pedido).await,
        "agy" => crate::ia::testar_agy(&pedido).await,
        "codex" => crate::ia::codex_cli(&pedido).await,
        _ => Err(anyhow::anyhow!("CLI desconhecido")),
    };
    match r {
        Ok(v) => axum::Json(serde_json::json!({ "ok": true, "resposta": v["resposta"] })).into_response(),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "erro": format!("{e:#}") })).into_response(),
    }
}

/// POST /sistema/limpar: roda a limpeza de arquivos agora.
pub async fn limpar_agora(State(st): State<AppState>, s: Sessao) -> Response {
    if let Some(r) = so_admin_sistema(&s) {
        return r;
    }
    let aviso = match crate::limpeza::limpar(&st).await {
        Ok((n, b)) => format!("Limpeza feita ✓ {n} arquivo(s) apagado(s), {:.1} MB liberados (arquivos com menos de 24 h ficam).", b as f64 / 1048576.0),
        Err(e) => format!("A limpeza falhou: {e:#}"),
    };
    pagina(&st, s, None, Some(aviso)).await
}

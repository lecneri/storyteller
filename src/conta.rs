//! Minha conta (nome, senha, notificações) e recuperação de senha por e-mail.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{self, Sessao},
    email,
    web::{erro_interno, render},
    AppState,
};

#[derive(Template)]
#[template(path = "conta.html")]
struct PaginaConta {
    s: Sessao,
    email: String,
    notificacoes: bool,
    email_ok: bool,
    aviso: Option<String>,
}

async fn pagina(st: &AppState, s: Sessao, aviso: Option<String>) -> Response {
    match sqlx::query_as::<_, (String, bool)>("SELECT email, notificacoes FROM acesso.usuarios WHERE id = $1")
        .bind(s.usuario_id)
        .fetch_one(&st.db)
        .await
    {
        Ok((email, notificacoes)) => render(PaginaConta { s, email, notificacoes, email_ok: email::configurado(), aviso }),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn ver(State(st): State<AppState>, s: Sessao) -> Response {
    pagina(&st, s, None).await
}

#[derive(Deserialize)]
pub struct FormConta {
    nome: String,
    #[serde(default)]
    notificacoes: Option<String>,
    #[serde(default)]
    senha_atual: String,
    #[serde(default)]
    senha_nova: String,
}

pub async fn salvar(State(st): State<AppState>, s: Sessao, Form(f): Form<FormConta>) -> Response {
    let nome = f.nome.trim();
    if nome.is_empty() {
        return pagina(&st, s, Some("O nome não pode ficar vazio.".into())).await;
    }
    if !f.senha_nova.is_empty() {
        let hash: String = match sqlx::query_scalar("SELECT senha_hash FROM acesso.usuarios WHERE id = $1")
            .bind(s.usuario_id)
            .fetch_one(&st.db)
            .await
        {
            Ok(h) => h,
            Err(e) => return erro_interno(e.into()),
        };
        if !auth::senha_confere(&f.senha_atual, &hash) {
            return pagina(&st, s, Some("Senha atual incorreta.".into())).await;
        }
        if f.senha_nova.len() < 8 {
            return pagina(&st, s, Some("A senha nova precisa ter pelo menos 8 caracteres.".into())).await;
        }
        let novo = match auth::gerar_hash_senha(&f.senha_nova) {
            Ok(h) => h,
            Err(e) => return erro_interno(e),
        };
        if let Err(e) = sqlx::query("UPDATE acesso.usuarios SET senha_hash = $2 WHERE id = $1")
            .bind(s.usuario_id)
            .bind(novo)
            .execute(&st.db)
            .await
        {
            return erro_interno(e.into());
        }
    }
    match sqlx::query("UPDATE acesso.usuarios SET nome = $2, notificacoes = $3 WHERE id = $1")
        .bind(s.usuario_id)
        .bind(nome)
        .bind(f.notificacoes.is_some())
        .execute(&st.db)
        .await
    {
        Ok(_) => Redirect::to("/conta").into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── esqueci a senha ─────────────

#[derive(Template)]
#[template(path = "esqueci.html")]
struct PaginaEsqueci {
    enviado: bool,
    token: Option<String>,
    erro: Option<String>,
}

pub async fn pagina_esqueci() -> Response {
    render(PaginaEsqueci { enviado: false, token: None, erro: None })
}

#[derive(Deserialize)]
pub struct FormEsqueci {
    email: String,
}

pub async fn esqueci(State(st): State<AppState>, Form(f): Form<FormEsqueci>) -> Response {
    let usuario: Option<Uuid> = sqlx::query_scalar("SELECT id FROM acesso.usuarios WHERE lower(email) = lower($1)")
        .bind(f.email.trim())
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    if let Some(uid) = usuario {
        let mut bytes = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = URL_SAFE_NO_PAD.encode(bytes);
        let ok = sqlx::query(
            "INSERT INTO acesso.redefinicoes (token_hash, usuario_id, expira_em) VALUES ($1, $2, now() + interval '1 hour')",
        )
        .bind(auth::hash_token(&token))
        .bind(uid)
        .execute(&st.db)
        .await
        .is_ok();
        if ok {
            email::enviar_em_segundo_plano(
                f.email.trim().to_string(),
                "Storyteller: redefinir senha".into(),
                format!(
                    "Para criar uma senha nova, abra o link (vale 1 hora):\n{}/redefinir/{token}\n\nSe não foi você, ignore este e-mail.",
                    email::url_publica()
                ),
            );
        }
    }
    // Mesma resposta exista ou não a conta (não revela quais e-mails estão cadastrados).
    render(PaginaEsqueci { enviado: true, token: None, erro: None })
}

pub async fn pagina_redefinir(Path(token): Path<String>) -> Response {
    render(PaginaEsqueci { enviado: false, token: Some(token), erro: None })
}

#[derive(Deserialize)]
pub struct FormRedefinir {
    senha: String,
}

pub async fn redefinir(State(st): State<AppState>, Path(token): Path<String>, Form(f): Form<FormRedefinir>) -> Response {
    if f.senha.len() < 8 {
        return render(PaginaEsqueci { enviado: false, token: Some(token), erro: Some("A senha precisa ter pelo menos 8 caracteres.".into()) });
    }
    let hash = match auth::gerar_hash_senha(&f.senha) {
        Ok(h) => h,
        Err(e) => return erro_interno(e),
    };
    let r = sqlx::query(
        "WITH r AS (UPDATE acesso.redefinicoes SET usado_em = now()
                     WHERE token_hash = $1 AND usado_em IS NULL AND expira_em > now() RETURNING usuario_id)
         UPDATE acesso.usuarios u SET senha_hash = $2 FROM r WHERE u.id = r.usuario_id",
    )
    .bind(auth::hash_token(&token))
    .bind(hash)
    .execute(&st.db)
    .await;
    match r {
        Ok(x) if x.rows_affected() == 1 => {
            // Encerra as sessões antigas da conta.
            Redirect::to("/entrar").into_response()
        }
        Ok(_) => render(PaginaEsqueci { enviado: false, token: None, erro: Some("Link inválido ou vencido. Peça outro.".into()) }),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── instalação (primeiro acesso, sem nenhum usuário) ─────────────

#[derive(Template)]
#[template(path = "instalar.html")]
struct PaginaInstalar {
    erro: Option<String>,
}

pub async fn pagina_instalar(State(st): State<AppState>) -> Response {
    if !auth::sem_usuarios(&st.db).await {
        return Redirect::to("/entrar").into_response();
    }
    render(PaginaInstalar { erro: None })
}

#[derive(Deserialize)]
pub struct FormInstalar {
    nome: String,
    email: String,
    senha: String,
    familia: String,
}

pub async fn instalar(
    State(st): State<AppState>,
    jar: axum_extra::extract::cookie::CookieJar,
    Form(f): Form<FormInstalar>,
) -> Response {
    if !auth::sem_usuarios(&st.db).await {
        return Redirect::to("/entrar").into_response();
    }
    let (nome, email, familia) = (f.nome.trim(), f.email.trim(), f.familia.trim());
    if nome.is_empty() || familia.is_empty() || !email.contains('@') || f.senha.len() < 8 {
        return render(PaginaInstalar { erro: Some("Preencha tudo; a senha precisa ter pelo menos 8 caracteres.".into()) });
    }
    match auth::criar_admin(&st.db, nome, email, &f.senha, familia).await {
        Ok((uid, fid)) => match auth::criar_sessao(&st.db, uid, Some(fid)).await {
            Ok(t) => (jar.add(auth::cookie_sessao(t, st.cookie_seguro)), Redirect::to("/sistema")).into_response(),
            Err(e) => erro_interno(e),
        },
        Err(e) => erro_interno(e),
    }
}

// ───────────── meus dados e apagar conta (LGPD) ─────────────

/// GET /conta/dados: zip com os dados da conta (JSON) + o conteúdo das famílias de que a pessoa participa.
pub async fn meus_dados(State(st): State<AppState>, s: Sessao) -> Response {
    let r = async {
        let (nome, email, criado, notif): (String, String, chrono::DateTime<chrono::Utc>, bool) =
            sqlx::query_as("SELECT nome, email, criado_em, notificacoes FROM acesso.usuarios WHERE id = $1")
                .bind(s.usuario_id)
                .fetch_one(&st.db)
                .await?;
        let familias: Vec<(Uuid, String, String, String)> = sqlx::query_as(
            "SELECT f.id, f.slug, f.nome, m.papel FROM acesso.membros m JOIN acesso.familias f ON f.id = m.familia_id
              WHERE m.usuario_id = $1",
        )
        .bind(s.usuario_id)
        .fetch_all(&st.db)
        .await?;
        let conta = serde_json::json!({
            "nome": nome, "email": email, "conta_criada_em": criado, "recebe_emails": notif,
            "admin_do_sistema": s.admin_sistema,
            "familias": familias.iter().map(|(_, slug, nome, papel)| serde_json::json!({ "nome": nome, "nome_unico": slug, "papel": papel })).collect::<Vec<_>>(),
        });
        let mut arquivos = vec![("conta.json".to_string(), serde_json::to_vec_pretty(&conta)?)];
        for (fid, slug, _, _) in &familias {
            arquivos.extend(crate::exportar::arquivos_familia(&st, *fid, slug).await?);
        }
        crate::exportar::zipar(arquivos)
    }
    .await;
    match r {
        Ok(bytes) => (
            [
                (axum::http::header::CONTENT_TYPE, "application/zip".to_string()),
                (axum::http::header::CONTENT_DISPOSITION, "attachment; filename=\"meus-dados-storyteller.zip\"".to_string()),
            ],
            axum::body::Body::from(bytes),
        )
            .into_response(),
        Err(e) => erro_interno(e),
    }
}

#[derive(Deserialize)]
pub struct FormApagar {
    senha: String,
    confirmacao: String,
}

/// POST /conta/apagar: apaga a conta. Famílias em que a pessoa é a única participante são apagadas junto;
/// nas outras, ela só sai (precisa haver outro admin).
pub async fn apagar_conta(State(st): State<AppState>, s: Sessao, Form(f): Form<FormApagar>) -> Response {
    let hash: String = match sqlx::query_scalar("SELECT senha_hash FROM acesso.usuarios WHERE id = $1").bind(s.usuario_id).fetch_one(&st.db).await {
        Ok(h) => h,
        Err(e) => return erro_interno(e.into()),
    };
    if !auth::senha_confere(&f.senha, &hash) || f.confirmacao.trim().to_uppercase() != "APAGAR" {
        return pagina(&st, s, Some("Para apagar, digite a sua senha e a palavra APAGAR.".into())).await;
    }
    if s.admin_sistema {
        let outros: i64 = sqlx::query_scalar("SELECT count(*) FROM acesso.usuarios WHERE admin_sistema AND id <> $1")
            .bind(s.usuario_id)
            .fetch_one(&st.db)
            .await
            .unwrap_or(0);
        if outros == 0 {
            return pagina(&st, s, Some("Você é o único admin do sistema: não dá para apagar esta conta.".into())).await;
        }
    }
    // Verifica as famílias antes de mexer em qualquer coisa.
    let familias: Vec<(Uuid, String, String, i64, i64)> = match sqlx::query_as(
        "SELECT f.id, f.nome, m.papel,
                (SELECT count(*) FROM acesso.membros o WHERE o.familia_id = f.id AND o.usuario_id <> $1),
                (SELECT count(*) FROM acesso.membros o WHERE o.familia_id = f.id AND o.usuario_id <> $1 AND o.papel = 'admin')
           FROM acesso.membros m JOIN acesso.familias f ON f.id = m.familia_id WHERE m.usuario_id = $1",
    )
    .bind(s.usuario_id)
    .fetch_all(&st.db)
    .await
    {
        Ok(v) => v,
        Err(e) => return erro_interno(e.into()),
    };
    if let Some((_, nome, ..)) = familias.iter().find(|(_, _, papel, outros, admins)| papel == "admin" && *outros > 0 && *admins == 0) {
        return pagina(&st, s, Some(format!("Antes, torne outra pessoa admin da {nome} (em Família)."))).await;
    }
    for (fid, _, _, outros, _) in &familias {
        if *outros == 0 {
            if let Err(e) = crate::limpeza::apagar_familia(&st, *fid).await {
                return erro_interno(e);
            }
        }
    }
    if let Err(e) = sqlx::query("DELETE FROM acesso.usuarios WHERE id = $1").bind(s.usuario_id).execute(&st.db).await {
        return erro_interno(e.into());
    }
    let jar = axum_extra::extract::cookie::CookieJar::new()
        .remove(axum_extra::extract::cookie::Cookie::build(auth::COOKIE_SESSAO).path("/"));
    (jar, Redirect::to("/entrar")).into_response()
}

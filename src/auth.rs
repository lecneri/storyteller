//! Login, senhas e sessões.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::FromRequestParts,
    http::request::Parts,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::AppState;

pub const COOKIE_SESSAO: &str = "st_sessao";
const DIAS_SESSAO: i64 = 30;

pub fn gerar_hash_senha(senha: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(senha.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("falha ao gerar hash: {e}"))
}

pub fn senha_confere(senha: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(senha.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

pub fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    format!("{:x}", h.finalize())
}

/// Cria uma sessão e devolve o token (só o hash vai para o banco).
pub async fn criar_sessao(db: &PgPool, usuario_id: Uuid, familia_id: Option<Uuid>) -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    sqlx::query(
        "INSERT INTO acesso.sessoes (token_hash, usuario_id, familia_id, expira_em)
         VALUES ($1, $2, $3, now() + make_interval(days => $4))",
    )
    .bind(hash_token(&token))
    .bind(usuario_id)
    .bind(familia_id)
    .bind(DIAS_SESSAO as i32)
    .execute(db)
    .await?;
    Ok(token)
}

pub async fn apagar_sessao(db: &PgPool, token: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM acesso.sessoes WHERE token_hash = $1")
        .bind(hash_token(token))
        .execute(db)
        .await?;
    Ok(())
}

pub fn cookie_sessao(token: String, seguro: bool) -> Cookie<'static> {
    Cookie::build((COOKIE_SESSAO, token))
        .path("/")
        .http_only(true)
        .secure(seguro)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(DIAS_SESSAO))
        .build()
}

/// Quem está logado, em qual família e com qual papel.
#[derive(Clone, Debug)]
pub struct Sessao {
    pub usuario_id: Uuid,
    pub usuario_nome: String,
    pub admin_sistema: bool,
    pub familia_id: Uuid,
    pub familia_nome: String,
    pub familia_slug: String,
    pub papel: String,
    /// Modo leitura ativo: o leitor (criança) e o nome dele.
    pub leitor: Option<(Uuid, String)>,
    /// Famílias de que a pessoa participa (para trocar de família).
    pub familias: Vec<(Uuid, String)>,
}

impl Sessao {
    pub fn admin_familia(&self) -> bool {
        self.papel == "admin"
    }
    pub fn eh_atual(&self, familia: &Uuid) -> bool {
        *familia == self.familia_id
    }
}

#[derive(sqlx::FromRow)]
struct LinhaSessao {
    usuario_id: Uuid,
    usuario_nome: String,
    admin_sistema: bool,
    familia_id: Uuid,
    familia_nome: String,
    familia_slug: String,
    papel: String,
    leitor_id: Option<Uuid>,
    leitor_nome: Option<String>,
}

/// Sem sessão válida → redireciona para /entrar.
impl FromRequestParts<AppState> for Sessao {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let Some(token) = jar.get(COOKIE_SESSAO).map(|c| c.value().to_owned()) else {
            return Err(Redirect::to("/entrar").into_response());
        };
        // Se a sessão não tiver família escolhida, usa a primeira em que a pessoa é membro.
        let linha = sqlx::query_as::<_, LinhaSessao>(
            "SELECT u.id AS usuario_id, u.nome AS usuario_nome, u.admin_sistema,
                    f.id AS familia_id, f.nome AS familia_nome, f.slug AS familia_slug, m.papel,
                    s.leitor_id, l.nome AS leitor_nome
               FROM acesso.sessoes s
               JOIN acesso.usuarios u ON u.id = s.usuario_id
               JOIN acesso.membros  m ON m.usuario_id = u.id
                                     AND (s.familia_id IS NULL OR m.familia_id = s.familia_id)
               JOIN acesso.familias f ON f.id = m.familia_id
               LEFT JOIN acesso.leitores l ON l.id = s.leitor_id
              WHERE s.token_hash = $1 AND s.expira_em > now()
              ORDER BY m.criado_em
              LIMIT 1",
        )
        .bind(hash_token(&token))
        .fetch_optional(&st.db)
        .await
        .map_err(|e| crate::web::erro_interno(e.into()))?;

        let familias = match &linha {
            Some(l) => sqlx::query_as::<_, (Uuid, String)>(
                "SELECT f.id, f.nome FROM acesso.membros m JOIN acesso.familias f ON f.id = m.familia_id
                  WHERE m.usuario_id = $1 ORDER BY f.nome",
            )
            .bind(l.usuario_id)
            .fetch_all(&st.db)
            .await
            .map_err(|e| crate::web::erro_interno(e.into()))?,
            None => Vec::new(),
        };
        match linha {
            Some(l) => Ok(Sessao {
                familias,
                usuario_id: l.usuario_id,
                usuario_nome: l.usuario_nome,
                admin_sistema: l.admin_sistema,
                familia_id: l.familia_id,
                familia_nome: l.familia_nome,
                familia_slug: l.familia_slug,
                papel: l.papel,
                leitor: l.leitor_id.zip(l.leitor_nome),
            }),
            None => Err(Redirect::to("/entrar").into_response()),
        }
    }
}

/// Na primeira execução, cria o admin do sistema e a família dele a partir das variáveis de ambiente.
pub async fn bootstrap_admin(db: &PgPool) -> anyhow::Result<()> {
    let existe: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM acesso.usuarios)")
        .fetch_one(db)
        .await?;
    if existe {
        return Ok(());
    }
    let (Ok(email), Ok(senha)) = (std::env::var("ADMIN_EMAIL"), std::env::var("ADMIN_SENHA")) else {
        tracing::info!("nenhum usuário no banco: abra /instalar no navegador para criar o admin");
        return Ok(());
    };
    let nome = std::env::var("ADMIN_NOME").unwrap_or_else(|_| "Admin".into());
    let familia = std::env::var("ADMIN_FAMILIA").unwrap_or_else(|_| format!("Família {nome}"));
    criar_admin(db, &nome, email.trim(), &senha, &familia).await?;
    tracing::info!("admin do sistema e família \"{familia}\" criados");
    Ok(())
}

/// Cria o admin do sistema e a família dele (CLI liberado). Devolve (usuario, familia).
pub async fn criar_admin(db: &PgPool, nome: &str, email: &str, senha: &str, familia: &str) -> anyhow::Result<(Uuid, Uuid)> {
    let mut tx = db.begin().await?;
    let usuario_id: Uuid = sqlx::query_scalar(
        "INSERT INTO acesso.usuarios (email, nome, senha_hash, admin_sistema)
         VALUES ($1, $2, $3, true) RETURNING id",
    )
    .bind(email)
    .bind(nome)
    .bind(gerar_hash_senha(senha)?)
    .fetch_one(&mut *tx)
    .await?;
    let familia_id: Uuid = sqlx::query_scalar(
        "INSERT INTO acesso.familias (slug, nome) VALUES ($1, $2) RETURNING id",
    )
    .bind(crate::util::slug(familia))
    .bind(familia)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO acesso.membros (familia_id, usuario_id, papel) VALUES ($1, $2, 'admin')")
        .bind(familia_id)
        .bind(usuario_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO ia.config_familia (familia_id, cli_habilitado) VALUES ($1, true)")
        .bind(familia_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((usuario_id, familia_id))
}

pub async fn sem_usuarios(db: &PgPool) -> bool {
    !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM acesso.usuarios)").fetch_one(db).await.unwrap_or(true)
}

//! Configurações do sistema: ficam no banco (editadas em /sistema) com cache em memória.
//! Ordem de leitura: valor salvo no site → variável de ambiente (.env) → padrão do código.

use std::{
    collections::HashMap,
    sync::{OnceLock, RwLock},
};

use crate::{segredos, AppState};

fn cache() -> &'static RwLock<HashMap<String, String>> {
    static C: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Chaves editáveis pelo site: (chave, secreta?).
pub const CHAVES: [(&str, bool); 14] = [
    ("URL_PUBLICA", false),
    ("CLI_TEXTO", false),
    ("ANTHROPIC_MODELO", false),
    ("GEMINI_MODELO", false),
    ("OPENAI_MODELO", false),
    ("OPENAI_MODELO_IMAGEM", false),
    ("GEMINI_MODELO_IMAGEM", false),
    ("SMTP_HOST", false),
    ("SMTP_USUARIO", false),
    ("SMTP_REMETENTE", false),
    ("SMTP_SENHA", true),
    ("GEMINI_CHAVE_GRATIS", true),
    ("CLAUDE_CODE_OAUTH_TOKEN", true),
    ("GEMINI_API_KEY", true),
];

pub fn secreta(chave: &str) -> bool {
    CHAVES.iter().any(|(k, s)| *k == chave && *s)
}

/// Valor efetivo (site → ambiente). Vazio conta como ausente.
pub fn get(chave: &str) -> Option<String> {
    let doc = cache().read().ok().and_then(|c| c.get(chave).cloned());
    doc.or_else(|| std::env::var(chave).ok()).filter(|v| !v.trim().is_empty())
}

pub fn get_ou(chave: &str, padrao: &str) -> String {
    get(chave).unwrap_or_else(|| padrao.to_string())
}

/// Onde o valor está vindo: "site", "env" ou "".
pub fn origem(chave: &str) -> &'static str {
    if cache().read().ok().is_some_and(|c| c.contains_key(chave)) {
        "site"
    } else if std::env::var(chave).is_ok_and(|v| !v.trim().is_empty()) {
        "env"
    } else {
        ""
    }
}

/// Carrega tudo do banco para o cache (na subida e depois de salvar).
pub async fn carregar(st: &AppState) -> anyhow::Result<()> {
    let linhas: Vec<(String, Option<String>, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT chave, valor, cifrado FROM ia.config_sistema").fetch_all(&st.db).await?;
    let mut mapa = HashMap::new();
    for (k, valor, cifrado) in linhas {
        let v = match cifrado {
            Some(c) => match segredos::decifrar(&c) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("config {k} não decifrou: {e}");
                    continue;
                }
            },
            None => valor.unwrap_or_default(),
        };
        if !v.trim().is_empty() {
            mapa.insert(k, v);
        }
    }
    if let Ok(mut c) = cache().write() {
        *c = mapa;
    }
    Ok(())
}

/// Salva (ou apaga, se vazio) uma configuração.
pub async fn salvar(st: &AppState, chave: &str, valor: &str) -> anyhow::Result<()> {
    let valor = valor.trim();
    if valor.is_empty() {
        sqlx::query("DELETE FROM ia.config_sistema WHERE chave = $1").bind(chave).execute(&st.db).await?;
    } else if secreta(chave) {
        sqlx::query(
            "INSERT INTO ia.config_sistema (chave, cifrado) VALUES ($1, $2)
             ON CONFLICT (chave) DO UPDATE SET cifrado = $2, valor = NULL, atualizado_em = now()",
        )
        .bind(chave)
        .bind(segredos::cifrar(valor)?)
        .execute(&st.db)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO ia.config_sistema (chave, valor) VALUES ($1, $2)
             ON CONFLICT (chave) DO UPDATE SET valor = $2, cifrado = NULL, atualizado_em = now()",
        )
        .bind(chave)
        .bind(valor)
        .execute(&st.db)
        .await?;
    }
    carregar(st).await
}

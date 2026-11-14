//! Envio de e-mails (SMTP; por padrão o Gmail com senha de app). Sem configuração, o envio é ignorado.

use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials, AsyncSmtpTransport, AsyncTransport,
    Message, Tokio1Executor,
};

pub fn configurado() -> bool {
    crate::config::get("SMTP_USUARIO").is_some() && crate::config::get("SMTP_SENHA").is_some()
}

pub fn url_publica() -> String {
    crate::config::get_ou("URL_PUBLICA", "http://localhost:8090").trim_end_matches('/').to_string()
}

pub async fn enviar(para: &str, assunto: &str, texto: &str) -> anyhow::Result<()> {
    if !configurado() {
        tracing::info!("e-mail não configurado; não enviei \"{assunto}\"");
        return Ok(());
    }
    let usuario = crate::config::get("SMTP_USUARIO").unwrap_or_default();
    let senha = crate::config::get("SMTP_SENHA").unwrap_or_default();
    let host = crate::config::get_ou("SMTP_HOST", "smtp.gmail.com");
    let remetente = crate::config::get("SMTP_REMETENTE").unwrap_or_else(|| format!("Storyteller <{usuario}>"));
    let msg = Message::builder()
        .from(remetente.parse()?)
        .to(para.parse()?)
        .subject(assunto)
        .header(ContentType::TEXT_PLAIN)
        .body(format!("{texto}\n\n— Storyteller 📚"))?;
    let smtp = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)?
        .credentials(Credentials::new(usuario, senha))
        .build();
    smtp.send(msg).await?;
    Ok(())
}

/// Envia sem travar quem chamou (falhas só vão para o log).
pub fn enviar_em_segundo_plano(para: String, assunto: String, texto: String) {
    tokio::spawn(async move {
        if let Err(e) = enviar(&para, &assunto, &texto).await {
            tracing::warn!("falha ao enviar e-mail para {para}: {e:#}");
        }
    });
}

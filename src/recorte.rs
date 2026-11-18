//! Tirar o fundo das imagens dos personagens: o serviço "recorte" (rembg, rede interna) devolve um PNG transparente.
//! Sem RECORTE_URL, o recurso some da tela e os desenhos ficam como vieram.

use std::time::Duration;

use anyhow::{bail, Context};
use uuid::Uuid;

use crate::{midia, AppState};

fn url() -> Option<String> {
    std::env::var("RECORTE_URL").ok().filter(|u| !u.trim().is_empty())
}

pub fn disponivel() -> bool {
    url().is_some()
}

async fn tirar_fundo(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let url = url().context("recorte não configurado")?;
    let r = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()?
        .post(url)
        .body(bytes)
        .send()
        .await
        .context("o serviço de recorte não respondeu")?;
    if !r.status().is_success() {
        bail!("o recorte falhou: {}", r.text().await.unwrap_or_default());
    }
    let png = r.bytes().await?.to_vec();
    if midia::tipo_imagem(&png).map(|(m, _)| m) != Some("image/png") {
        bail!("o recorte não devolveu um PNG");
    }
    Ok(png)
}

/// Recorta a imagem `id` da família e guarda o resultado (PNG) na `pasta`. Devolve o id novo.
pub async fn recortar(st: &AppState, familia: Uuid, pasta: &str, id: Uuid) -> anyhow::Result<Uuid> {
    let img = crate::imagem::carregar_refs(st, familia, &[id]).await.pop().context("imagem não encontrada")?;
    let png = tirar_fundo(img.bytes).await?;
    midia::salvar(st, familia, pasta, &png, "image/png", "png").await
}

/// Depois de gerar um desenho de personagem: tenta tirar o fundo; se não der, fica a original.
pub async fn sem_fundo_se_der(st: &AppState, familia: Uuid, pasta: &str, id: Uuid) -> Uuid {
    if !disponivel() {
        return id;
    }
    match recortar(st, familia, pasta, id).await {
        Ok(novo) => novo,
        Err(e) => {
            tracing::warn!("recorte automático falhou: {e:#}");
            id
        }
    }
}

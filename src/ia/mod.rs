//! IA de texto: escolhe a rota (CLI de assinatura → chave da família → chave do sistema → Gemini grátis)
//! e chama o provedor. Os provedores ficam em `cli` (assinaturas) e `api` (chaves).

mod api;
mod cli;
pub mod gemini;

use anyhow::{anyhow, bail};
use serde_json::Value;

pub use api::{base_kimi, por_api};
pub use cli::{
    gerar_imagem as gerar_imagem_cli, nome_seguro as cli_nome_seguro, situacao as situacao_clis, testar as testar_cli, AjusteCli, CLIS_IMAGEM, ESFORCOS,
};
use api::gemini_api;

pub struct PedidoTexto {
    pub sistema: String,
    pub prompt: String,
    /// JSON Schema da resposta (saída estruturada).
    pub schema: Value,
    /// Imagens anexadas (mime, bytes), ex.: fotos para a IA descrever o personagem.
    pub imagens: Vec<(String, Vec<u8>)>,
}

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Corta mensagens longas (erros de CLI/API) para caberem num aviso.
fn resumo(s: &str) -> String {
    s.chars().take(400).collect()
}

/// Lê um objeto JSON de um texto da IA (com ou sem ```json, com texto antes/depois).
pub fn extrair_json(texto: &str) -> anyhow::Result<Value> {
    let t = texto.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let ini = t.find('{').ok_or_else(|| anyhow!("sem objeto JSON"))?;
    let fim = t.rfind('}').ok_or_else(|| anyhow!("sem objeto JSON"))?;
    Ok(serde_json::from_str(&t[ini..=fim])?)
}

/// Chaves cifradas disponíveis (da família, ou do sistema quando familia = None), na ordem de preferência.
async fn chaves(st: &crate::AppState, familia: Option<uuid::Uuid>) -> anyhow::Result<Vec<(String, String, Option<String>)>> {
    let linhas: Vec<(String, Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT provedor, cifrada, modelo FROM ia.chaves WHERE familia_id IS NOT DISTINCT FROM $1 AND provedor <> 'elevenlabs'
          ORDER BY CASE provedor WHEN 'anthropic' THEN 1 WHEN 'gemini' THEN 2 WHEN 'openai' THEN 3 ELSE 4 END",
    )
    .bind(familia)
    .fetch_all(&st.db)
    .await?;
    let mut v = Vec::new();
    for (p, c, m) in linhas {
        match crate::segredos::decifrar(&c) {
            Ok(k) => v.push((p, k, m)),
            Err(e) => tracing::warn!("chave {p} não decifrou: {e}"),
        }
    }
    Ok(v)
}

async fn registrar_uso(st: &crate::AppState, familia: uuid::Uuid, via: &str, provedor: &str, modelo: Option<&str>) {
    let _ = sqlx::query("INSERT INTO ia.uso (familia_id, tarefa, via, provedor, modelo) VALUES ($1, 'texto', $2, $3, $4)")
        .bind(familia)
        .bind(via)
        .bind(provedor)
        .bind(modelo)
        .execute(&st.db)
        .await;
}

/// Fontes de IA na ordem da família, sem repetir e sem nomes estranhos; as que faltam vão no fim.
pub fn ordem_fontes(fontes: &[String]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for f in fontes.iter().map(String::as_str).chain(["cli", "familia", "sistema"]) {
        if let Some(x) = ["cli", "familia", "sistema"].into_iter().find(|x| *x == f) {
            if !v.contains(&x) {
                v.push(x);
            }
        }
    }
    v
}

/// Em que ordem tentar as CLIs da família: "ordem" = como estão; "rodizio" = começa por uma diferente a cada
/// pedido (pelo número de usos) e depois as outras; "fixo" = só a primeira.
fn ordem_clis(clis: &[String], modo: &str, vezes: i64) -> Vec<String> {
    if clis.is_empty() {
        return Vec::new();
    }
    match modo {
        "fixo" => clis[..1].to_vec(),
        "rodizio" => {
            let i = vezes.rem_euclid(clis.len() as i64) as usize;
            clis[i..].iter().chain(&clis[..i]).cloned().collect()
        }
        _ => clis.to_vec(),
    }
}

/// Gera texto pela melhor rota disponível para a família:
/// CLIs liberados para a família (na ordem dela) → chave da família → chave do sistema → chave gratuita do Gemini (GEMINI_CHAVE_GRATIS).
/// O limite semanal vale para o que o admin paga (CLI e chave do sistema).
/// Começo da mensagem quando as assinaturas falharam e a API paga ficou esperando a família decidir.
pub const PRECISA_PAGAR: &str = "💳 As assinaturas falharam";

pub async fn gerar_texto(st: &crate::AppState, familia: uuid::Uuid, p: &PedidoTexto) -> anyhow::Result<Value> {
    gerar_texto_com(st, familia, p, false).await
}

/// Como `gerar_texto`; `pago_ok` = a família já disse "pode usar a API paga". Sem isso (e sem "pagar sem perguntar"),
/// se as assinaturas falharem, as chaves pagas não são usadas e o erro começa com PRECISA_PAGAR.
pub async fn gerar_texto_com(st: &crate::AppState, familia: uuid::Uuid, p: &PedidoTexto, pago_ok: bool) -> anyhow::Result<Value> {
    let (clis, modo, sistema, limites, fontes, ajustes, sem_perguntar): (Vec<String>, String, bool, Value, Vec<String>, Value, bool) = sqlx::query_as(
        "SELECT COALESCE(c.clis, '{}'), COALESCE(c.modo_cli, 'ordem'), COALESCE(c.usar_chave_sistema, false), COALESCE(c.limites, '{}'),
                COALESCE(c.ordem_fontes, '{cli,familia,sistema}'), COALESCE(c.cli_ajustes, '{}'), COALESCE(c.pagar_sem_perguntar, false)
           FROM acesso.familias f LEFT JOIN ia.config_familia c ON c.familia_id = f.id WHERE f.id = $1",
    )
    .bind(familia)
    .fetch_one(&st.db)
    .await?;
    let usado: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ia.uso WHERE familia_id = $1 AND criado_em > now() - interval '7 days' AND via IN ('cli', 'chave_sistema')",
    )
    .bind(familia)
    .fetch_one(&st.db)
    .await?;
    let dentro_do_limite = limites["historias_semana"].as_i64().is_none_or(|l| usado < l);
    let mut erros: Vec<String> = Vec::new();
    // Alguma assinatura foi tentada e falhou: as chaves pagas só entram com o "pode pagar" da família.
    let mut cli_falhou = false;
    let mut pagas_puladas = false;

    // Fontes na ordem da família (padrão: assinaturas → chave da família → chave do sistema).
    for fonte in ordem_fontes(&fontes) {
        match fonte {
            "cli" => {
                // As assinaturas liberadas para a família, na ordem dela; se uma falhar, tenta a próxima.
                if dentro_do_limite {
                    let vezes = if modo == "rodizio" {
                        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ia.uso WHERE familia_id = $1 AND via = 'cli'")
                            .bind(familia)
                            .fetch_one(&st.db)
                            .await
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    for nome in &ordem_clis(&clis, &modo, vezes) {
                        // Modelo e esforço escolhidos para esta CLI na família (vazio = padrão dela).
                        let a = AjusteCli::de(&ajustes, nome);
                        let r = match nome.as_str() {
                            // O agy não aceita imagens anexadas: pula para a próxima.
                            "agy" if !p.imagens.is_empty() => continue,
                            "agy" | "codex" | "claude" => cli::texto(nome, p, &a).await,
                            _ => continue,
                        };
                        match r {
                            Ok((v, modelo)) => {
                                registrar_uso(st, familia, "cli", nome, modelo.as_deref()).await;
                                return Ok(v);
                            }
                            Err(e) => {
                                cli_falhou = true;
                                erros.push(format!("CLI {e:#}"));
                            }
                        }
                    }
                }
            }
            "familia" if cli_falhou && !pago_ok && !sem_perguntar => pagas_puladas |= !chaves(st, Some(familia)).await?.is_empty(),
            "sistema" if cli_falhou && !pago_ok && !sem_perguntar => pagas_puladas |= sistema && dentro_do_limite && !chaves(st, None).await?.is_empty(),
            "familia" => {
                for (prov, chave, modelo) in chaves(st, Some(familia)).await? {
                    match por_api(&prov, &chave, modelo.as_deref(), p).await {
                        Ok(v) => {
                            registrar_uso(st, familia, "chave_familia", &prov, modelo.as_deref()).await;
                            return Ok(v);
                        }
                        Err(e) => erros.push(format!("chave da família ({prov}): {e:#}")),
                    }
                }
            }
            _ => {
                if sistema && dentro_do_limite {
                    for (prov, chave, modelo) in chaves(st, None).await? {
                        match por_api(&prov, &chave, modelo.as_deref(), p).await {
                            Ok(v) => {
                                registrar_uso(st, familia, "chave_sistema", &prov, modelo.as_deref()).await;
                                return Ok(v);
                            }
                            Err(e) => erros.push(format!("chave do sistema ({prov}): {e:#}")),
                        }
                    }
                }
            }
        }
    }
    if let Some(chave) = crate::config::get("GEMINI_CHAVE_GRATIS") {
        if !chave.trim().is_empty() {
            match gemini_api(chave.trim(), None, p).await {
                Ok(v) => {
                    registrar_uso(st, familia, "gratis", "gemini", None).await;
                    return Ok(v);
                }
                Err(e) => erros.push(format!("Gemini gratuito: {e:#}")),
            }
        }
    }
    if pagas_puladas {
        bail!("{PRECISA_PAGAR} ({}). A API paga não foi usada sem a sua confirmação.", erros.join(" | "));
    }
    if erros.is_empty() {
        if !dentro_do_limite {
            bail!("O limite de histórias desta semana foi atingido. Tente de novo em alguns dias ou crie a história manualmente.");
        }
        bail!("Esta família ainda não tem IA de texto configurada (sem CLI e sem chave de API). Peça ao admin, ou crie a história manualmente.");
    }
    bail!("Nenhuma IA conseguiu gerar: {}", erros.join(" | "))
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn extrai_json_de_textos_da_ia() {
        assert_eq!(extrair_json("{\"a\": 1}").unwrap()["a"], 1);
        assert_eq!(extrair_json("```json\n{\"a\": 2}\n```").unwrap()["a"], 2);
        assert_eq!(extrair_json("Claro! Aqui está: {\"a\": {\"b\": 3}} Espero ter ajudado.").unwrap()["a"]["b"], 3);
        assert!(extrair_json("sem json aqui").is_err());
        assert!(extrair_json("{ quebrado").is_err());
    }

    #[test]
    fn ordem_das_fontes() {
        let f = |v: &[&str]| ordem_fontes(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(f(&["familia", "cli"]), vec!["familia", "cli", "sistema"]);
        assert_eq!(f(&[]), vec!["cli", "familia", "sistema"]);
        assert_eq!(f(&["x", "sistema", "sistema"]), vec!["sistema", "cli", "familia"]);
    }

    #[test]
    fn ordem_das_clis_por_modo() {
        let c: Vec<String> = ["claude", "codex", "agy"].map(String::from).to_vec();
        assert_eq!(ordem_clis(&c, "ordem", 5), c);
        assert_eq!(ordem_clis(&c, "fixo", 5), vec!["claude".to_string()]);
        assert_eq!(ordem_clis(&c, "rodizio", 0), c);
        assert_eq!(ordem_clis(&c, "rodizio", 4), ["codex", "agy", "claude"].map(String::from).to_vec());
        assert!(ordem_clis(&[], "rodizio", 3).is_empty());
    }

    #[test]
    fn resumo_corta_em_400_caracteres() {
        assert_eq!(resumo(&"á".repeat(1000)).chars().count(), 400);
        assert_eq!(resumo("curto"), "curto");
    }
}

//! Chamada à API do Gemini com retentativas, pensada para a chave gratuita, que falha bastante:
//! - 503/500/408/504 e erro de rede: passageiros, repete;
//! - 429 por minuto: repete, esperando o que o servidor pede (Retry-After ou retryDelay do corpo);
//! - 429 da cota do dia ("PerDay"): não adianta repetir, vai para o próximo provedor;
//! - 400/404 com modelo escolhido na chave: tenta uma vez com o modelo padrão.
//! Usada pelo texto (ia::api) e pelas imagens (imagem.rs).

use std::time::Duration;

use anyhow::bail;
use serde_json::Value;

use super::resumo;

#[derive(Debug, PartialEq)]
pub enum Falha {
    /// Vale repetir; espera mínima pedida pelo servidor (ms), se houver.
    Passageira(Option<u64>),
    /// Cota do dia acabou: repetir só queima tempo.
    CotaDoDia,
    /// Modelo inexistente ou pedido recusado (400/404).
    ModeloOuPedido,
    Outra,
}

/// Classifica uma resposta de erro do Gemini.
pub fn classificar(status: u16, corpo: &Value, retry_after: Option<&str>) -> Falha {
    let detalhes = corpo["error"]["details"].as_array().cloned().unwrap_or_default();
    match status {
        429 => {
            let do_dia = detalhes.iter().flat_map(|d| d["violations"].as_array().cloned().unwrap_or_default())
                .any(|v| v["quotaId"].as_str().is_some_and(|q| q.contains("PerDay")));
            if do_dia {
                return Falha::CotaDoDia;
            }
            let pedido = retry_after.and_then(|s| s.trim().parse::<f64>().ok()).map(|s| (s * 1000.0) as u64).or_else(|| {
                detalhes.iter().find_map(|d| {
                    let t = d["retryDelay"].as_str()?.trim().strip_suffix('s')?;
                    t.parse::<f64>().ok().map(|s| (s * 1000.0).round() as u64)
                })
            });
            Falha::Passageira(pedido)
        }
        402 => Falha::CotaDoDia,
        408 | 500 | 502 | 503 | 504 => Falha::Passageira(None),
        400 | 404 => Falha::ModeloOuPedido,
        _ => Falha::Outra,
    }
}

/// Espera antes da próxima tentativa: backoff exponencial (250 ms × 2^(n-1), até 10 s),
/// nunca menos de 1 s e nunca menos do que o servidor pediu (até 65 s).
pub fn espera_ms(tentativa: u32, pedido: Option<u64>) -> u64 {
    let backoff = (250u64 << tentativa.saturating_sub(1).min(10)).min(10_000);
    backoff.max(1000).max(pedido.unwrap_or(0).min(65_000))
}

fn tentativas() -> u32 {
    crate::config::get("GEMINI_TENTATIVAS").and_then(|v| v.parse().ok()).unwrap_or(3).clamp(1, 6)
}

/// POST generateContent com retentativas. Devolve o JSON da resposta (status 2xx).
/// `modelo_padrao` é usado quando o modelo escolhido não existe (400/404).
pub async fn gerar(chave: &str, modelo: &str, modelo_padrao: &str, corpo: &Value, timeout: Duration) -> anyhow::Result<Value> {
    let http = reqwest::Client::builder().timeout(timeout).build().unwrap_or_default();
    let mut modelo = modelo.to_string();
    let mut trocou = false;
    let max = tentativas();
    let mut n = 1;
    loop {
        let r = http
            .post(format!("https://generativelanguage.googleapis.com/v1beta/models/{modelo}:generateContent"))
            .header("x-goog-api-key", chave)
            .json(corpo)
            .send()
            .await;
        let (falha, msg) = match r {
            Ok(r) => {
                let status = r.status();
                let retry_after = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(String::from);
                let j: Value = r.json().await.unwrap_or_default();
                if status.is_success() {
                    return Ok(j);
                }
                (classificar(status.as_u16(), &j, retry_after.as_deref()), format!("Gemini {status}: {}", resumo(j["error"]["message"].as_str().unwrap_or("?"))))
            }
            // Timeout e erro de rede: passageiros.
            Err(e) => (Falha::Passageira(None), format!("Gemini sem resposta: {e}")),
        };
        match falha {
            Falha::Passageira(pedido) if n < max => {
                let ms = espera_ms(n, pedido);
                tracing::warn!("{msg} (modelo {modelo}); tentativa {n}/{max}, nova em {ms} ms");
                tokio::time::sleep(Duration::from_millis(ms)).await;
                n += 1;
            }
            Falha::ModeloOuPedido if !trocou && modelo != modelo_padrao => {
                tracing::warn!("{msg}; tentando o modelo padrão {modelo_padrao} no lugar de {modelo}");
                modelo = modelo_padrao.to_string();
                trocou = true;
                n = 1;
            }
            Falha::CotaDoDia => bail!("{msg} (acabou a cota do dia)"),
            _ => bail!("{msg}"),
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use serde_json::json;

    #[test]
    fn separa_cota_do_dia_da_cota_por_minuto() {
        let dia = json!({ "error": { "details": [{ "violations": [{ "quotaId": "GenerateRequestsPerDayPerProjectPerModel-FreeTier" }] }] } });
        assert_eq!(classificar(429, &dia, None), Falha::CotaDoDia);
        let minuto = json!({ "error": { "details": [
            { "violations": [{ "quotaId": "GenerateRequestsPerMinutePerProjectPerModel-FreeTier" }] },
            { "@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "33s" }
        ] } });
        assert_eq!(classificar(429, &minuto, None), Falha::Passageira(Some(33_000)));
        assert_eq!(classificar(429, &json!({}), Some("2")), Falha::Passageira(Some(2_000)));
        assert_eq!(classificar(429, &json!({}), None), Falha::Passageira(None));
        assert_eq!(classificar(503, &json!({}), None), Falha::Passageira(None));
        assert_eq!(classificar(404, &json!({}), None), Falha::ModeloOuPedido);
        assert_eq!(classificar(403, &json!({}), None), Falha::Outra);
    }

    #[test]
    fn espera_respeita_piso_teto_e_pedido_do_servidor() {
        assert_eq!(espera_ms(1, None), 1000);
        assert_eq!(espera_ms(4, None), 2000);
        assert_eq!(espera_ms(20, None), 10_000);
        assert_eq!(espera_ms(1, Some(33_000)), 33_000);
        assert_eq!(espera_ms(1, Some(500_000)), 65_000);
    }
}

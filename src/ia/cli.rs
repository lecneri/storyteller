//! Assinaturas (Claude Code, Codex, Antigravity/agy) pelo serviço ia-cli: os CLIs rodam lá (isolados, com os
//! logins) e o storyteller só faz pedidos HTTP. Endereço e chave: IA_CLI_URL e IA_CLI_CHAVE (site ou .env).
//! Cada pedido vai para UMA assinatura: a ordem, o rodízio e o registro de uso continuam aqui.

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use super::{resumo, PedidoTexto};

/// Os três CLIs, na ordem em que aparecem na tela.
pub const CLIS: [&str; 3] = ["claude", "codex", "agy"];

/// CLIs que geram imagem.
pub const CLIS_IMAGEM: [&str; 2] = ["codex", "agy"];

/// Níveis de esforço que as CLIs entendem.
pub const ESFORCOS: [(&str, &str); 5] = [("low", "Baixo"), ("medium", "Médio"), ("high", "Alto"), ("xhigh", "Muito alto"), ("max", "Máximo")];

/// Prazo de um pedido (a espera na fila do ia-cli conta dentro dele).
const PRAZO_S: u64 = 300;

/// Modelo e esforço escolhidos para uma CLI (por família). `None` = padrão da CLI.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct AjusteCli {
    pub modelo: Option<String>,
    pub esforco: Option<String>,
}

impl AjusteCli {
    /// Lê de `ia.config_familia.cli_ajustes` ({"claude": {"modelo": "opus", "esforco": "high"}}).
    pub fn de(ajustes: &Value, cli: &str) -> Self {
        let texto = |k: &str| ajustes[cli][k].as_str().map(str::trim).filter(|t| nome_seguro(t)).map(String::from);
        AjusteCli { modelo: texto("modelo"), esforco: texto("esforco").filter(|e| ESFORCOS.iter().any(|(k, _)| k == e)) }
    }

    fn json(&self) -> Value {
        let mut v = json!({});
        if let Some(m) = &self.modelo {
            v["modelo"] = json!(m);
        }
        if let Some(e) = &self.esforco {
            v["esforco"] = json!(e);
        }
        v
    }
}

/// Nome de modelo aceitável: letras, números e . _ - : / (nunca começando com "-").
pub fn nome_seguro(t: &str) -> bool {
    !t.is_empty() && t.len() <= 80 && !t.starts_with('-') && t.chars().all(|c| c.is_ascii_alphanumeric() || "._-:/".contains(c))
}

/// Endereço e chave do ia-cli; sem a chave, as assinaturas ficam indisponíveis.
fn destino() -> anyhow::Result<(String, String)> {
    let chave = crate::config::get("IA_CLI_CHAVE").ok_or_else(|| anyhow!("o ia-cli não está configurado (falta IA_CLI_CHAVE em Sistema › Configurações)"))?;
    let url = crate::config::get_ou("IA_CLI_URL", "http://ia-cli:8080");
    Ok((url.trim_end_matches('/').to_string(), chave.trim().to_string()))
}

fn http(segundos: u64) -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(segundos)).build().unwrap_or_default()
}

/// Falha devolvida pelo ia-cli: o tipo (limite | login | tempo | falha) vem junto na mensagem.
fn erro_da_resposta(cli: &str, status: reqwest::StatusCode, j: &Value) -> anyhow::Error {
    let tipo = match j["tipo"].as_str().unwrap_or_default() {
        "limite" => "limite de uso da assinatura",
        "login" => "precisa conectar de novo",
        "tempo" => "demorou demais",
        _ => "falhou",
    };
    let msg = j["tentativas"]
        .as_array()
        .and_then(|t| t.last())
        .and_then(|t| t["erro"].as_str())
        .or_else(|| j["erro"].as_str())
        .unwrap_or("sem detalhe");
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return anyhow!("{cli}: a chave do ia-cli foi recusada (confira IA_CLI_CHAVE)");
    }
    anyhow!("{cli} ({tipo}): {}", resumo(msg))
}

async fn post(caminho: &str, corpo: &Value, cli: &str) -> anyhow::Result<Value> {
    let (url, chave) = destino()?;
    let r = http(PRAZO_S + 30)
        .post(format!("{url}{caminho}"))
        .bearer_auth(chave)
        .json(corpo)
        .send()
        .await
        .map_err(|e| anyhow!("{cli}: não consegui falar com o ia-cli ({e})"))?;
    let status = r.status();
    let j: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(erro_da_resposta(cli, status, &j));
    }
    Ok(j)
}

/// Texto por uma assinatura. Devolve a resposta (JSON no formato do schema) e o modelo usado.
pub async fn texto(cli: &str, p: &PedidoTexto, a: &AjusteCli) -> anyhow::Result<(Value, Option<String>)> {
    let imagens: Vec<Value> = p.imagens.iter().map(|(mime, b)| json!({ "mime": mime, "base64": STANDARD.encode(b) })).collect();
    let corpo = json!({
        "sistema": p.sistema,
        "prompt": p.prompt,
        "schema": p.schema,
        "imagens": imagens,
        "clis": [cli],
        "ajustes": { cli: a.json() },
        "tempo_max_s": PRAZO_S,
    });
    let j = post("/v1/texto", &corpo, cli).await?;
    let modelo = j["modelo"].as_str().filter(|m| !m.is_empty()).map(String::from).or_else(|| a.modelo.clone());
    let v = match j.get("json").filter(|v| v.is_object()) {
        Some(v) => v.clone(),
        None => super::extrair_json(j["texto"].as_str().unwrap_or_default()).with_context(|| format!("{cli} não devolveu JSON válido"))?,
    };
    Ok((v, modelo))
}

/// Imagem por uma assinatura (`codex` ou `agy`). `refs`: (mime, bytes). Devolve os bytes e o modelo usado.
pub async fn gerar_imagem(cli: &str, prompt: &str, refs: &[(String, Vec<u8>)], a: &AjusteCli) -> anyhow::Result<(Vec<u8>, Option<String>)> {
    if !CLIS_IMAGEM.contains(&cli) {
        bail!("a CLI {cli} não gera imagem");
    }
    let referencias: Vec<Value> = refs.iter().map(|(mime, b)| json!({ "mime": mime, "base64": STANDARD.encode(b) })).collect();
    let corpo = json!({ "prompt": prompt, "referencias": referencias, "clis": [cli], "ajustes": { cli: a.json() }, "tempo_max_s": PRAZO_S });
    let j = post("/v1/imagem", &corpo, cli).await?;
    let bytes = STANDARD.decode(j["base64"].as_str().unwrap_or_default()).with_context(|| format!("{cli}: imagem inválida"))?;
    if bytes.is_empty() {
        bail!("{cli} não devolveu imagem");
    }
    let modelo = j["modelo"].as_str().filter(|m| !m.is_empty()).map(String::from).or_else(|| a.modelo.clone());
    Ok((bytes, modelo))
}

/// Situação de um CLI no ia-cli (para a tela Sistema).
#[derive(Clone, Debug, Default)]
pub struct SituacaoCli {
    pub nome: String,
    /// Texto para mostrar ("conectado", "✅ funcionando (testado …)", "indisponível"…).
    pub situacao: String,
    pub conectado: bool,
    pub modelo_padrao: String,
    pub modelos: Vec<(String, String)>,
}

fn hora_local(quando: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(quando)
        .map(|d| d.with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).expect("fuso")).format("%d/%m %H:%M").to_string())
        .unwrap_or_default()
}

/// Lê um item de /v1/clis.
fn ler_situacao(c: &Value) -> SituacaoCli {
    let nome = c["nome"].as_str().unwrap_or_default().to_string();
    let login = c["situacao"].as_str().unwrap_or("?").to_string();
    let conectado = !login.starts_with("não conectado") && login != "indisponível";
    let t = &c["ultimo_teste"];
    let situacao = if t.is_object() {
        let hora = hora_local(t["quando"].as_str().unwrap_or_default());
        match (t["ok"].as_bool().unwrap_or(false), t["modelo"].as_str().filter(|m| !m.is_empty())) {
            (true, Some(m)) => format!("✅ funcionando (testado {hora}, modelo {m})"),
            (true, None) => format!("✅ funcionando (testado {hora})"),
            (false, _) => format!("❌ falhou no teste de {hora}: {}", t["erro"].as_str().unwrap_or("?").chars().take(120).collect::<String>()),
        }
    } else {
        login
    };
    SituacaoCli {
        nome,
        situacao,
        conectado,
        modelo_padrao: c["modelo_padrao"].as_str().unwrap_or_default().to_string(),
        modelos: c["modelos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| Some((m["id"].as_str()?.to_string(), m["nome"].as_str().unwrap_or_default().to_string())))
            .filter(|(id, _)| nome_seguro(id))
            .collect(),
    }
}

/// Situação dos três CLIs. Sem ia-cli (ou fora do ar), todos aparecem como indisponíveis.
pub async fn situacao() -> Vec<SituacaoCli> {
    let indisponivel = |motivo: &str| {
        CLIS.iter().map(|c| SituacaoCli { nome: c.to_string(), situacao: motivo.to_string(), ..Default::default() }).collect()
    };
    let Ok((url, chave)) = destino() else { return indisponivel("indisponível (ia-cli não configurado)") };
    let r = match http(15).get(format!("{url}/v1/clis")).bearer_auth(chave).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => return indisponivel(&format!("indisponível (ia-cli respondeu {})", r.status().as_u16())),
        Err(_) => return indisponivel("indisponível (ia-cli fora do ar)"),
    };
    let j: Value = r.json().await.unwrap_or(Value::Null);
    let lidos: Vec<SituacaoCli> = j["clis"].as_array().into_iter().flatten().map(ler_situacao).collect();
    CLIS.iter()
        .map(|c| lidos.iter().find(|s| s.nome == *c).cloned().unwrap_or_else(|| SituacaoCli { nome: c.to_string(), situacao: "indisponível".into(), ..Default::default() }))
        .collect()
}

/// Botão "Testar": pedido mínimo no ia-cli. Devolve o modelo usado.
pub async fn testar(cli: &str) -> anyhow::Result<Option<String>> {
    if !CLIS.contains(&cli) {
        bail!("CLI desconhecido");
    }
    let j = post("/v1/testar", &json!({ "cli": cli }), cli).await?;
    if j["ok"].as_bool() != Some(true) {
        bail!("{}", resumo(j["erro"].as_str().unwrap_or("falhou")));
    }
    Ok(j["modelo"].as_str().filter(|m| !m.is_empty()).map(String::from))
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn ajuste_por_cli_vai_no_pedido() {
        let aj = json!({ "codex": { "modelo": "gpt-6-sol", "esforco": "max" }, "claude": { "modelo": "-x; rm", "esforco": "turbo" } });
        assert_eq!(AjusteCli::de(&aj, "codex").json(), json!({ "modelo": "gpt-6-sol", "esforco": "max" }));
        // Nome perigoso e esforço desconhecido são ignorados (fica o padrão da CLI).
        assert_eq!(AjusteCli::de(&aj, "claude"), AjusteCli::default());
        assert_eq!(AjusteCli::de(&aj, "agy").json(), json!({}));
        assert!(nome_seguro("gemini-3.8-flash-medium") && !nome_seguro("--help") && !nome_seguro("a b"));
    }

    #[test]
    fn erro_do_ia_cli_diz_o_tipo() {
        let j = json!({ "erro": "codex: x", "tipo": "limite", "tentativas": [{ "cli": "codex", "tipo": "limite", "erro": "usage limit reached" }] });
        let e = erro_da_resposta("codex", reqwest::StatusCode::BAD_GATEWAY, &j).to_string();
        assert!(e.contains("limite de uso") && e.contains("usage limit reached"), "{e}");
        let e = erro_da_resposta("claude", reqwest::StatusCode::UNAUTHORIZED, &json!({ "erro": "chave inválida" })).to_string();
        assert!(e.contains("IA_CLI_CHAVE"));
    }

    #[test]
    fn situacao_mostra_o_ultimo_teste() {
        let c = json!({ "nome": "codex", "situacao": "conectado", "modelo_padrao": "gpt-6", "gera_imagem": true,
            "modelos": [{ "id": "gpt-6", "nome": "GPT-6" }, { "id": "--mal", "nome": "x" }],
            "ultimo_teste": { "quando": "2026-11-18T18:30:00Z", "ok": true, "modelo": "gpt-6" } });
        let s = ler_situacao(&c);
        assert!(s.conectado);
        assert_eq!(s.situacao, "✅ funcionando (testado 18/11 15:30, modelo gpt-6)");
        assert_eq!(s.modelos, vec![("gpt-6".to_string(), "GPT-6".to_string())]);
        let s = ler_situacao(&json!({ "nome": "agy", "situacao": "não conectado" }));
        assert!(!s.conectado && s.situacao == "não conectado");
    }
}

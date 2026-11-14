//! CLIs de assinatura (Claude Code, Codex, Antigravity/agy). Todos rodam isolados: pasta temporária
//! vazia, ambiente limpo e sem ferramentas — o briefing vem das famílias, então nada de shell,
//! arquivos ou variáveis do servidor ao alcance da IA.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{anyhow, bail, Context};
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command};

use super::{api::conteudo_anthropic, extrair_json, resumo, PedidoTexto, TEMPO_MAXIMO};

/// Pasta temporária que se apaga sozinha (inclusive quando a IA estoura o tempo ou dá erro).
struct PastaTemp(PathBuf);

impl PastaTemp {
    async fn nova() -> anyhow::Result<Self> {
        let dir = std::env::temp_dir().join(format!("st-ia-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(PastaTemp(dir))
    }
    fn caminho(&self) -> &Path {
        &self.0
    }
}

impl Drop for PastaTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Comando com ambiente limpo (só as variáveis indicadas) rodando dentro da pasta temporária.
fn comando_isolado(programa: &str, pasta: &Path, variaveis: &[&str]) -> Command {
    let mut cmd = Command::new(programa);
    cmd.current_dir(pasta).env_clear().stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    for var in variaveis {
        if let Ok(v) = std::env::var(var) {
            cmd.env(var, v);
        }
    }
    cmd
}

const AMBIENTE: [&str; 3] = ["HOME", "PATH", "LANG"];

fn estourou(_: tokio::time::error::Elapsed) -> anyhow::Error {
    anyhow!("a IA demorou mais de 5 minutos")
}

/// Pedido para CLIs que recebem tudo num texto só (sistema + pedido).
fn prompt_unico(p: &PedidoTexto) -> String {
    format!("{}\nNão use ferramentas nem crie arquivos: responda só com o JSON.\n\n{}", p.sistema, p.prompt)
}

// ───────────── Claude Code ─────────────

/// Roda `claude -p` sem nenhuma ferramenta. Com imagens, o pedido vai como mensagem stream-json.
pub async fn claude_cli(p: &PedidoTexto) -> anyhow::Result<Value> {
    let pasta = PastaTemp::nova().await?;
    let com_imagens = !p.imagens.is_empty();
    let mut cmd = comando_isolado("claude", pasta.caminho(), &AMBIENTE);
    cmd.args(["-p", "--tools", "", "--no-session-persistence", "--strict-mcp-config"]);
    if com_imagens {
        cmd.args(["--input-format", "stream-json", "--output-format", "stream-json", "--verbose"]);
    } else {
        cmd.args(["--output-format", "json"]);
    }
    cmd.arg("--system-prompt").arg(&p.sistema).arg("--json-schema").arg(p.schema.to_string()).stdin(Stdio::piped());
    if let Some(token) = crate::config::get("CLAUDE_CODE_OAUTH_TOKEN") {
        cmd.env("CLAUDE_CODE_OAUTH_TOKEN", token);
    }

    let mut filho = cmd.spawn().context("não consegui iniciar o claude (CLI instalado?)")?;
    if let Some(mut stdin) = filho.stdin.take() {
        let entrada = if com_imagens {
            let msg = serde_json::json!({ "type": "user", "message": { "role": "user", "content": conteudo_anthropic(p) } });
            format!("{msg}\n")
        } else {
            p.prompt.clone()
        };
        stdin.write_all(entrada.as_bytes()).await?;
    }
    let saida = tokio::time::timeout(TEMPO_MAXIMO, filho.wait_with_output()).await.map_err(estourou)??;
    ler_resposta_claude(&String::from_utf8_lossy(&saida.stdout), com_imagens)
        .with_context(|| format!("stderr: {}", resumo(&String::from_utf8_lossy(&saida.stderr))))
}

/// Interpreta a saída do `claude -p`: um JSON (`--output-format json`) ou, em stream-json,
/// várias linhas em que a última com `"type":"result"` traz o resultado.
fn ler_resposta_claude(stdout: &str, stream: bool) -> anyhow::Result<Value> {
    let linha = if stream {
        stdout.lines().rev().find(|l| l.contains("\"type\":\"result\"")).unwrap_or_default()
    } else {
        stdout.trim()
    };
    let json: Value =
        serde_json::from_str(linha.trim()).with_context(|| format!("resposta inesperada do claude: {}", resumo(stdout)))?;
    if json.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        bail!("o claude devolveu erro: {}", resumo(json.get("result").and_then(Value::as_str).unwrap_or("?")));
    }
    if let Some(estruturado) = json.get("structured_output").filter(|v| !v.is_null()) {
        return Ok(estruturado.clone());
    }
    // Sem saída estruturada: tenta ler o texto como JSON (com ou sem ```json).
    let texto = json.get("result").and_then(Value::as_str).unwrap_or_default();
    extrair_json(texto).context("a IA não devolveu JSON válido")
}

// ───────────── Codex (ChatGPT) ─────────────

/// Codex CLI: somente leitura, sem pedir aprovação, resposta no formato do schema (gravada em arquivo).
pub async fn codex_cli(p: &PedidoTexto) -> anyhow::Result<Value> {
    let pasta = PastaTemp::nova().await?;
    let dir = pasta.caminho();
    let schema = dir.join("schema.json");
    let resposta = dir.join("resposta.json");
    tokio::fs::write(&schema, p.schema.to_string()).await?;
    let mut anexos = Vec::new();
    for (i, (mime, b)) in p.imagens.iter().enumerate() {
        let ext = mime.rsplit('/').next().unwrap_or("png");
        let caminho = dir.join(format!("foto-{i}.{ext}"));
        tokio::fs::write(&caminho, b).await?;
        anexos.push(caminho);
    }
    let mut cmd = comando_isolado("codex", dir, &AMBIENTE);
    cmd.args(["exec", "--skip-git-repo-check", "--ephemeral", "--sandbox", "read-only", "-c", "approval_policy=\"never\""])
        .arg("--output-schema")
        .arg(&schema)
        .arg("-o")
        .arg(&resposta)
        .arg("-C")
        .arg(dir)
        // "--image=caminho" (com =): a opção aceita vários valores e engoliria o prompt.
        .args(anexos.iter().map(|a| format!("--image={}", a.display())))
        .arg(prompt_unico(p))
        .stdin(Stdio::null());
    let saida = tokio::time::timeout(TEMPO_MAXIMO, cmd.output()).await.map_err(estourou)??;
    match tokio::fs::read_to_string(&resposta).await {
        Ok(t) if !t.trim().is_empty() => extrair_json(&t).context("o Codex não devolveu JSON válido"),
        _ => bail!("codex falhou: {}", resumo(ultima_linha(&String::from_utf8_lossy(&saida.stderr)).unwrap_or("sem resposta"))),
    }
}

fn ultima_linha(texto: &str) -> Option<&str> {
    texto.lines().rev().find(|l| !l.trim().is_empty())
}

// ───────────── Antigravity (agy, Gemini) ─────────────

/// Para o botão "Testar" do sistema.
pub async fn testar_agy(p: &PedidoTexto) -> anyhow::Result<Value> {
    agy_cli(p).await
}

/// Antigravity CLI (Gemini da assinatura Google). Nega todas as permissões (sem isso o agy fica
/// parado pedindo aprovação) e roda em modo plano. Não aceita imagens anexadas.
pub(super) async fn agy_cli(p: &PedidoTexto) -> anyhow::Result<Value> {
    let pasta = PastaTemp::nova().await?;
    let negar = r#"{"permissions":{"allow":[],"deny":["*","command(*)","write_file(*)","edit_file(*)"]}}"#;
    let mut cmd = comando_isolado("agy", pasta.caminho(), &["HOME", "PATH", "LANG", "DBUS_SESSION_BUS_ADDRESS"]);
    // O pedido vai logo depois do -p (o agy usa o argumento seguinte ao -p como prompt).
    cmd.args(["--output-format", "json", "--mode", "plan", "--print-timeout", "240s", "--settings", negar])
        .arg("--json-schema")
        .arg(p.schema.to_string())
        .arg("-p")
        .arg(prompt_unico(p));
    if let Some(k) = crate::config::get("GEMINI_API_KEY") {
        cmd.env("GEMINI_API_KEY", k);
    }
    let saida = tokio::time::timeout(TEMPO_MAXIMO, cmd.output()).await.map_err(estourou)??;
    let stdout = String::from_utf8_lossy(&saida.stdout);
    if !saida.status.success() {
        bail!("agy falhou: {} {}", resumo(&stdout), resumo(&String::from_utf8_lossy(&saida.stderr)));
    }
    ler_resposta_agy(&stdout)
}

/// A saída json do agy pode vir como envelope ({structured_output} ou {result: "..."}) ou direto como o objeto.
fn ler_resposta_agy(stdout: &str) -> anyhow::Result<Value> {
    let v: Value = serde_json::from_str(stdout.trim()).or_else(|_| extrair_json(stdout))?;
    if let Some(r) = v.get("structured_output").filter(|x| !x.is_null()) {
        return Ok(r.clone());
    }
    if v.get("status").and_then(Value::as_str).is_some_and(|s| s != "SUCCESS") {
        bail!("agy não concluiu: {}", resumo(&v.to_string()));
    }
    if let Some(r) = v.get("result").or_else(|| v.get("response")).and_then(Value::as_str) {
        return extrair_json(r);
    }
    Ok(v)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn le_resposta_do_claude() {
        let r = ler_resposta_claude(r#"{"is_error":false,"structured_output":{"titulo":"Oi"}}"#, false).unwrap();
        assert_eq!(r["titulo"], "Oi");
        // Sem structured_output: lê o JSON de dentro do texto.
        let r = ler_resposta_claude(r#"{"result":"```json\n{\"titulo\":\"Dois\"}\n```"}"#, false).unwrap();
        assert_eq!(r["titulo"], "Dois");
        // stream-json: só a linha do resultado conta.
        let stream = "{\"type\":\"system\"}\n{\"type\":\"assistant\"}\n{\"type\":\"result\",\"structured_output\":{\"x\":1}}\n";
        assert_eq!(ler_resposta_claude(stream, true).unwrap()["x"], 1);
        // Erro do CLI vira erro.
        let e = ler_resposta_claude(r#"{"is_error":true,"result":"Invalid API key"}"#, false).unwrap_err();
        assert!(format!("{e}").contains("Invalid API key"));
        assert!(ler_resposta_claude("não é json", false).is_err());
    }

    #[test]
    fn le_resposta_do_agy() {
        assert_eq!(ler_resposta_agy(r#"{"structured_output":{"a":1}}"#).unwrap()["a"], 1);
        assert_eq!(ler_resposta_agy(r#"{"status":"SUCCESS","result":"{\"a\":2}"}"#).unwrap()["a"], 2);
        assert_eq!(ler_resposta_agy(r#"{"a":3}"#).unwrap()["a"], 3);
        assert!(ler_resposta_agy(r#"{"status":"ERROR","result":"x"}"#).is_err());
    }

    #[test]
    fn prompt_unico_junta_sistema_e_pedido() {
        let p = PedidoTexto { sistema: "SIS".into(), prompt: "PED".into(), schema: Value::Null, imagens: vec![] };
        let t = prompt_unico(&p);
        assert!(t.starts_with("SIS") && t.ends_with("PED") && t.contains("Não use ferramentas"));
    }

    #[tokio::test]
    async fn pasta_temporaria_some_ao_sair() {
        let caminho = {
            let p = PastaTemp::nova().await.unwrap();
            assert!(p.caminho().exists());
            p.caminho().to_path_buf()
        };
        assert!(!caminho.exists());
    }

    #[test]
    fn ultima_linha_ignora_vazias() {
        assert_eq!(ultima_linha("a\nerro final\n\n  \n"), Some("erro final"));
        assert_eq!(ultima_linha(""), None);
    }
}

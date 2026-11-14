//! Login dos CLIs pelo site (sem SSH): o servidor roda o login num terminal simulado,
//! mostra o link/código para o admin e recebe o código de volta quando o CLI pede.

use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Arc, Mutex, OnceLock},
};

use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, Command},
};

use crate::{config, AppState};

pub const CLIS: [&str; 3] = ["claude", "codex", "agy"];

#[derive(Clone, Serialize, Default)]
pub struct Estado {
    /// iniciando | aguardando | concluido | erro
    pub fase: String,
    pub url: Option<String>,
    pub codigo: Option<String>,
    pub mensagem: String,
    /// O CLI espera um código colado de volta (claude e agy).
    pub pede_codigo: bool,
}

struct Sessao {
    estado: Estado,
    saida: String,
    stdin: Option<ChildStdin>,
    parar: Arc<tokio::sync::Notify>,
}

fn sessoes() -> &'static Mutex<HashMap<String, Arc<tokio::sync::Mutex<Sessao>>>> {
    static S: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<Sessao>>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remove sequências de escape do terminal (cores, links OSC 8) e junta quebras de linha.
fn limpar(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            match it.peek() {
                Some('[') => {
                    it.next();
                    for d in it.by_ref() {
                        if d.is_ascii_alphabetic() || d == '~' {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: termina em BEL ou ESC \
                    it.next();
                    while let Some(d) = it.next() {
                        if d == '\u{7}' {
                            break;
                        }
                        if d == '\u{1b}' && it.peek() == Some(&'\\') {
                            it.next();
                            break;
                        }
                        out.push(d); // mantém o conteúdo (o link aparece aqui)
                    }
                    out.push(' ');
                }
                Some('(') | Some(')') => {
                    it.next();
                    it.next();
                }
                _ => {}
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

fn achar_url(texto: &str, prefixos: &[&str]) -> Option<String> {
    for p in prefixos {
        if let Some(i) = texto.find(p) {
            let url: String = texto[i..].chars().take_while(|c| !c.is_whitespace() && *c != ';').collect();
            return Some(url);
        }
    }
    None
}

fn achar_codigo_device(texto: &str) -> Option<String> {
    texto.split_whitespace().find(|t| {
        let partes: Vec<&str> = t.split('-').collect();
        partes.len() == 2
            && partes.iter().all(|p| (4..=6).contains(&p.len()) && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
    }).map(String::from)
}

fn achar_token_claude(texto: &str) -> Option<String> {
    let i = texto.find("sk-ant-oat")?;
    // O token pode quebrar de linha no terminal: junta só os caracteres válidos.
    let token: String = texto[i..]
        .chars()
        .filter(|c| !c.is_whitespace())
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    // O terminal gruda o texto seguinte no token (sem espaços): corta na primeira palavra conhecida.
    let mut fim = token.len();
    for palavra in ["Store", "Use", "Your", "This", "Token", "Save", "You", "Copy", "Keep", "Export", "Set", "Press"] {
        if let Some(j) = token[20.min(token.len())..].find(palavra) {
            fim = fim.min(j + 20);
        }
    }
    let token = token[..fim].to_string();
    (token.len() > 40).then_some(token)
}

/// Confere se o token funciona com um pedido mínimo ao Claude.
async fn token_funciona(token: &str) -> bool {
    let saida = Command::new("claude")
        .args(["-p", "--output-format", "json", "--tools", "", "--no-session-persistence", "--strict-mcp-config"])
        .arg("Responda só: ok")
        .env("CLAUDE_CODE_OAUTH_TOKEN", token)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(60), saida).await {
        Ok(Ok(o)) => serde_json::from_slice::<serde_json::Value>(&o.stdout)
            .map(|v| v["is_error"] == false)
            .unwrap_or(false),
        _ => false,
    }
}

pub async fn estado(nome: &str) -> Estado {
    let s = sessoes().lock().ok().and_then(|m| m.get(nome).cloned());
    match s {
        Some(s) => s.lock().await.estado.clone(),
        None => Estado { fase: "parado".into(), ..Default::default() },
    }
}

/// Inicia o login de um CLI. Devolve quando aparecer o link (ou depois de ~15 s).
pub async fn iniciar(st: &AppState, nome: &str) -> anyhow::Result<Estado> {
    if !CLIS.contains(&nome) {
        anyhow::bail!("CLI desconhecido");
    }
    let comando = match nome {
        "claude" => "stty cols 2000; claude setup-token",
        "codex" => "stty cols 2000; codex login --device-auth",
        _ => "stty cols 2000; agy -p ok --print-timeout 600s",
    };
    let mut cmd = Command::new("script");
    cmd.args(["-qfec", comando, "/dev/null"])
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env("COLUMNS", "2000")
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Um login novo encerra o anterior (que poderia ficar esperando para sempre).
    let anterior = sessoes().lock().ok().and_then(|m| m.get(nome).cloned());
    if let Some(a) = anterior {
        a.lock().await.parar.notify_one();
    }
    let mut filho = cmd.spawn()?;
    let parar = Arc::new(tokio::sync::Notify::new());
    let sessao = Arc::new(tokio::sync::Mutex::new(Sessao {
        parar: parar.clone(),
        estado: Estado {
            fase: "iniciando".into(),
            pede_codigo: nome != "codex",
            mensagem: "Abrindo o login…".into(),
            ..Default::default()
        },
        saida: String::new(),
        stdin: filho.stdin.take(),
    }));
    if let Ok(mut m) = sessoes().lock() {
        m.insert(nome.to_string(), sessao.clone());
    }

    // Lê a saída e atualiza o estado; ao terminar, verifica o resultado.
    let mut stdout = filho.stdout.take().expect("stdout");
    let nome_t = nome.to_string();
    let st_t = st.clone();
    let s_t = sessao.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        let mut token_visto: Option<String> = None;
        loop {
            let lido = tokio::select! {
                r = stdout.read(&mut buf) => r,
                _ = parar.notified() => {
                    let _ = filho.kill().await;
                    return;
                }
            };
            match lido {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut s = s_t.lock().await;
                    s.saida.push_str(&limpar(&String::from_utf8_lossy(&buf[..n])));
                    let texto = s.saida.clone();
                    if s.estado.url.is_none() {
                        s.estado.url = achar_url(&texto, &[
                            "https://claude.com/cai/oauth/", "https://claude.ai/oauth/", "https://auth.openai.com/",
                            "https://accounts.google.com/",
                        ]);
                    }
                    if nome_t == "codex" && s.estado.codigo.is_none() {
                        s.estado.codigo = achar_codigo_device(&texto);
                    }
                    if s.estado.url.is_some() && s.estado.fase == "iniciando" {
                        s.estado.fase = "aguardando".into();
                        s.estado.mensagem = match nome_t.as_str() {
                            "codex" => "Abra o link, entre na conta do ChatGPT e digite o código.".into(),
                            _ => "Abra o link, autorize e cole aqui o código que aparecer.".into(),
                        };
                    }
                    // Código recusado ou vencido: avisa e encerra (o CLI ficaria esperando um Enter).
                    let baixo = texto.to_lowercase();
                    if s.estado.fase == "aguardando"
                        && (baixo.contains("oauth error") || baixo.contains("wasn't accepted") || baixo.contains("invalid code")
                            || baixo.contains("authentication failed") || baixo.contains("expired"))
                    {
                        s.estado.fase = "erro".into();
                        s.estado.mensagem = "O código não foi aceito (incompleto, vencido ou de uma tentativa anterior). Clique em Conectar de novo e use o código novo.".into();
                        drop(s);
                        let _ = filho.kill().await;
                        return;
                    }
                    if nome_t == "claude" && token_visto.is_none() {
                        token_visto = achar_token_claude(&texto);
                        if token_visto.is_some() {
                            s.estado.mensagem = "Token recebido, conferindo…".into();
                        }
                    }
                }
            }
        }
        let status = filho.wait().await;
        // Claude: confere o token antes de salvar (o terminal pode ter grudado texto nele).
        if let Some(t) = token_visto {
            let mut candidatos = vec![t.clone()];
            if t.len() > 108 {
                candidatos.push(t[..108].to_string());
            }
            let mut ok = None;
            for c in candidatos {
                if token_funciona(&c).await {
                    ok = Some(c);
                    break;
                }
            }
            let mut s = s_t.lock().await;
            match ok {
                Some(c) if config::salvar(&st_t, "CLAUDE_CODE_OAUTH_TOKEN", &c).await.is_ok() => {
                    s.estado.fase = "concluido".into();
                    s.estado.mensagem = "Assinatura do Claude conectada ✓".into();
                }
                _ => {
                    s.estado.fase = "erro".into();
                    s.estado.mensagem = "Recebi o token, mas ele não funcionou. Tente Conectar de novo.".into();
                }
            }
            s.stdin = None;
            return;
        }
        let mut s = s_t.lock().await;
        if s.estado.fase != "concluido" {
            let ok = status.map(|x| x.success()).unwrap_or(false);
            // codex/agy guardam o login no volume do container; o claude precisa do token capturado.
            if ok && nome_t != "claude" {
                s.estado.fase = "concluido".into();
                s.estado.mensagem = "Conectado ✓".into();
            } else {
                s.estado.fase = "erro".into();
                let fim: String = s.saida.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(200).collect();
                s.estado.mensagem = format!("O login não terminou. {fim}");
            }
        }
        s.stdin = None;
    });

    // Espera o link aparecer (até ~15 s).
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let e = sessao.lock().await.estado.clone();
        if e.url.is_some() || e.fase == "erro" || e.fase == "concluido" {
            return Ok(e);
        }
    }
    let e = sessao.lock().await.estado.clone();
    Ok(e)
}

/// Envia o código colado pelo admin para o CLI que está esperando.
pub async fn enviar_codigo(nome: &str, codigo: &str) -> anyhow::Result<Estado> {
    let s = sessoes().lock().ok().and_then(|m| m.get(nome).cloned()).ok_or_else(|| anyhow::anyhow!("login não iniciado"))?;
    {
        let mut g = s.lock().await;
        let stdin = g.stdin.as_mut().ok_or_else(|| anyhow::anyhow!("o login já terminou; comece de novo"))?;
        stdin.write_all(format!("{}\r", codigo.trim()).as_bytes()).await?;
        stdin.flush().await?;
        g.estado.mensagem = "Código enviado, verificando…".into();
    }
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let e = s.lock().await.estado.clone();
        if e.fase == "concluido" || e.fase == "erro" {
            return Ok(e);
        }
    }
    let e = s.lock().await.estado.clone();
    Ok(e)
}

/// Situação atual de cada CLI (sem gastar uso): token salvo / `codex login status` / arquivos do agy.
pub async fn situacao() -> Vec<(String, String)> {
    let claude = if config::get("CLAUDE_CODE_OAUTH_TOKEN").is_some() { "conectado (token salvo)" } else { "não conectado" };
    let codex = Command::new("codex")
        .args(["login", "status"])
        .output()
        .await
        .map(|o| {
            let t = String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr);
            if t.to_lowercase().contains("logged in") && !t.to_lowercase().contains("not logged") { "conectado" } else { "não conectado" }
        })
        .unwrap_or("indisponível");
    let agy = if config::get("GEMINI_API_KEY").is_some() { "chave de API configurada" } else { "use “Conectar” e depois “Testar”" };
    vec![("claude".into(), claude.into()), ("codex".into(), codex.into()), ("agy".into(), agy.into())]
}

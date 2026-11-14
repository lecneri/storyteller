//! Narração: vozes, síntese (edge-tts via script Python) e cache por (texto + voz).

use std::process::Stdio;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;

use crate::{midia, AppState};

/// Vozes disponíveis (edge-tts). As "Multilingual" também falam português.
pub const VOZES: [(&str, &str); 21] = [
    ("pt-BR-FranciscaNeural", "Francisca · mulher (BR)"),
    ("pt-BR-AntonioNeural", "Antonio · homem (BR)"),
    ("pt-BR-ThalitaMultilingualNeural", "Thalita · mulher jovem (BR)"),
    ("en-US-AvaMultilingualNeural", "Ava · mulher, expressiva"),
    ("en-US-EmmaMultilingualNeural", "Emma · mulher, alegre"),
    ("en-US-AndrewMultilingualNeural", "Andrew · homem, caloroso"),
    ("en-US-BrianMultilingualNeural", "Brian · homem, descontraído"),
    ("fr-FR-VivienneMultilingualNeural", "Vivienne · mulher, sotaque leve"),
    ("fr-FR-RemyMultilingualNeural", "Remy · homem, sotaque leve"),
    ("it-IT-GiuseppeMultilingualNeural", "Giuseppe · homem, sotaque leve"),
    ("en-AU-WilliamMultilingualNeural", "William · homem, sotaque leve"),
    ("ko-KR-HyunsuMultilingualNeural", "Hyunsu · homem jovem, sotaque leve"),
    ("en-US-AnaNeural", "Ana · menina (só inglês)"),
    ("en-GB-MaisieNeural", "Maisie · menina (só inglês britânico)"),
    ("en-US-GuyNeural", "Guy · homem, animado (só inglês)"),
    ("en-US-RogerNeural", "Roger · homem, alegre (só inglês)"),
    ("en-US-ChristopherNeural", "Christopher · homem, voz de narrador (só inglês)"),
    ("en-US-EricNeural", "Eric · homem, calmo (só inglês)"),
    ("en-US-SteffanNeural", "Steffan · homem, sereno (só inglês)"),
    ("en-GB-RyanNeural", "Ryan · homem (só inglês britânico)"),
    ("en-GB-ThomasNeural", "Thomas · homem (só inglês britânico)"),
];

/// Línguas oferecidas para as vozes extras de um personagem (nome, frase de teste).
pub const LINGUAS: [(&str, &str); 9] = [
    ("Português", "Oi! Vamos brincar?"),
    ("Inglês", "Hello! Let's play together!"),
    ("Espanhol", "¡Hola! ¿Vamos a jugar?"),
    ("Francês", "Bonjour ! On joue ensemble ?"),
    ("Italiano", "Ciao! Giochiamo insieme?"),
    ("Alemão", "Hallo! Spielen wir zusammen?"),
    ("Japonês", "こんにちは！いっしょにあそぼう！"),
    ("Coreano", "안녕! 같이 놀자!"),
    ("Chinês", "你好！我们一起玩吧！"),
];

/// Uma voz extra do personagem (ex.: "Inglês").
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VozExtra {
    #[serde(default)]
    pub slug: String,
    pub nome: String,
    #[serde(default = "lingua_padrao")]
    pub lingua: String,
    pub voz: Voz,
}

fn lingua_padrao() -> String {
    "Português".into()
}

/// Separa a chave de voz de um trecho: "pedro+ingles" → ("pedro", Some("ingles")).
pub fn separar(chave: &str) -> (&str, Option<&str>) {
    match chave.split_once('+') {
        Some((q, v)) if !v.is_empty() => (q, Some(v)),
        Some((q, _)) => (q, None),
        None => (chave, None),
    }
}

/// Voz genérica (links públicos) de um trecho: `chave` é "narrador", o slug ou "slug+voz-extra".
pub fn voz_generica(chave: &str, tipo: &str) -> Voz {
    if chave == "narrador" || tipo.is_empty() || tipo == "narrador" {
        narrador_padrao()
    } else if separar(chave).1.is_some() {
        generica_extra(tipo)
    } else {
        voz_por_tipo(tipo)
    }
}

/// Voz genérica (links públicos) para uma voz extra: precisa ser multilíngue.
pub fn generica_extra(tipo: &str) -> Voz {
    let v = voz_por_tipo(tipo);
    if v.id.contains("Multilingual") { v } else { Voz { id: "en-US-AndrewMultilingualNeural".into(), ..v } }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Voz {
    #[serde(default = "motor_padrao")]
    pub motor: String,
    pub id: String,
    /// Ajuste de tom em Hz (-60 a +60)
    #[serde(default)]
    pub tom: i32,
    /// Ajuste de velocidade em % (-40 a +40)
    #[serde(default)]
    pub velocidade: i32,
}

fn motor_padrao() -> String {
    "edge".into()
}

impl Voz {
    fn nova(id: &str, tom: i32, velocidade: i32) -> Self {
        Voz { motor: "edge".into(), id: id.into(), tom, velocidade }
    }
    /// Garante valores aceitos (voz conhecida e ajustes dentro dos limites).
    pub fn validada(mut self) -> Self {
        if !VOZES.iter().any(|(id, _)| *id == self.id) {
            self.id = VOZES[0].0.into();
        }
        self.motor = "edge".into();
        self.tom = self.tom.clamp(-60, 60);
        self.velocidade = self.velocidade.clamp(-40, 40);
        self
    }
    pub fn de_json(v: &Value) -> Option<Voz> {
        serde_json::from_value::<Voz>(v.clone()).ok().map(Voz::validada)
    }
}

pub fn narrador_padrao() -> Voz {
    Voz::nova("pt-BR-FranciscaNeural", 0, 0)
}

/// Voz padrão de um personagem sem voz configurada, pelo tipo.
pub fn voz_por_tipo(tipo: &str) -> Voz {
    match tipo {
        "crianca" => Voz::nova("pt-BR-ThalitaMultilingualNeural", 25, 5),
        "animal" => Voz::nova("pt-BR-AntonioNeural", 35, 10),
        "vilao" => Voz::nova("pt-BR-AntonioNeural", -20, -10),
        "robo" => Voz::nova("en-US-AndrewMultilingualNeural", -15, -5),
        "criatura" => Voz::nova("en-US-BrianMultilingualNeural", -25, -5),
        _ => Voz::nova("pt-BR-AntonioNeural", 0, 0),
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Audio {
    pub arquivo: Uuid,
    pub duracao: f32,
    pub palavras: Value,
}

fn hash(voz: &Voz, texto: &str) -> String {
    let chave = json!([voz.motor, voz.id, voz.tom, voz.velocidade, texto]).to_string();
    format!("{:x}", Sha256::digest(chave.as_bytes()))
}

/// Hash do cache de áudio (usado também pela limpeza).
pub fn hash_publico(voz: &Voz, texto: &str) -> String {
    hash(voz, texto.trim())
}

/// Devolve o áudio do cache ou gera com o edge-tts.
pub async fn sintetizar(st: &AppState, familia_id: Uuid, familia_slug: &str, texto: &str, voz: &Voz) -> anyhow::Result<Audio> {
    let texto = texto.trim();
    if texto.is_empty() {
        bail!("texto vazio");
    }
    let h = hash(voz, texto);
    let em_cache: Option<(Uuid, f32, Value)> = sqlx::query_as(
        "SELECT arquivo, duracao, palavras FROM midia.audios WHERE familia_id = $1 AND hash = $2",
    )
    .bind(familia_id)
    .bind(&h)
    .fetch_optional(&st.db)
    .await?;
    if let Some((arquivo, duracao, palavras)) = em_cache {
        return Ok(Audio { arquivo, duracao, palavras });
    }

    // O serviço de voz às vezes falha por instabilidade de rede: tenta até 3 vezes.
    let mut ultima = None;
    for tentativa in 0..3u64 {
        if tentativa > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(800 * tentativa)).await;
        }
        match gerar(st, familia_id, familia_slug, texto, voz, &h).await {
            Ok(a) => return Ok(a),
            Err(e) => {
                tracing::warn!("TTS tentativa {} falhou: {e:#}", tentativa + 1);
                ultima = Some(e);
            }
        }
    }
    Err(ultima.unwrap_or_else(|| anyhow::anyhow!("falha no TTS")))
}

async fn gerar(st: &AppState, familia_id: Uuid, familia_slug: &str, texto: &str, voz: &Voz, h: &str) -> anyhow::Result<Audio> {
    let tmp = std::env::temp_dir().join(format!("st-tts-{h}-{}.mp3", Uuid::new_v4()));
    let entrada = json!({
        "texto": texto,
        "voz": voz.id,
        "tom": format!("{:+}Hz", voz.tom),
        "velocidade": format!("{:+}%", voz.velocidade),
        "saida": tmp,
    });
    let mut filho = Command::new("python3")
        .arg("/usr/local/bin/st-tts.py")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("não consegui iniciar o TTS")?;
    if let Some(mut stdin) = filho.stdin.take() {
        stdin.write_all(entrada.to_string().as_bytes()).await?;
    }
    let saida = tokio::time::timeout(std::time::Duration::from_secs(90), filho.wait_with_output())
        .await
        .context("o TTS demorou demais")??;
    if !saida.status.success() {
        let _ = tokio::fs::remove_file(&tmp).await;
        // A última linha do traceback é a que explica o erro.
        let stderr = String::from_utf8_lossy(&saida.stderr);
        bail!("falha no TTS: {}", stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("?"));
    }
    let palavras = serde_json::from_slice::<Value>(&saida.stdout)
        .ok()
        .and_then(|v| v.get("palavras").cloned())
        .unwrap_or(json!([]));
    let bytes = tokio::fs::read(&tmp).await.context("o TTS não gerou o arquivo")?;
    let duracao = duracao_mp3(&tmp).await.unwrap_or(0.0);
    let _ = tokio::fs::remove_file(&tmp).await;

    let pasta = format!("familias/{familia_slug}/audio");
    let arquivo = midia::salvar(st, familia_id, &pasta, &bytes, "audio/mpeg", "mp3").await?;
    sqlx::query(
        "INSERT INTO midia.audios (familia_id, hash, arquivo, duracao, palavras) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (familia_id, hash) DO NOTHING",
    )
    .bind(familia_id)
    .bind(h)
    .bind(arquivo)
    .bind(duracao)
    .bind(&palavras)
    .execute(&st.db)
    .await?;
    Ok(Audio { arquivo, duracao, palavras })
}

async fn duracao_mp3(caminho: &std::path::Path) -> anyhow::Result<f32> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(caminho)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().parse()?)
}

/// Só consulta o cache (não gera).
pub async fn no_cache(st: &AppState, familia_id: Uuid, texto: &str, voz: &Voz) -> anyhow::Result<Option<Audio>> {
    let r: Option<(Uuid, f32, Value)> = sqlx::query_as(
        "SELECT arquivo, duracao, palavras FROM midia.audios WHERE familia_id = $1 AND hash = $2",
    )
    .bind(familia_id)
    .bind(hash(voz, texto.trim()))
    .fetch_optional(&st.db)
    .await?;
    Ok(r.map(|(arquivo, duracao, palavras)| Audio { arquivo, duracao, palavras }))
}

/// Vozes de uma família: narrador + cada personagem (configurada ou padrão pelo tipo) + vozes extras.
pub struct VozesFamilia {
    pub narrador: Voz,
    pub personagens: Vec<(String, Voz)>,
    /// ("pedro+ingles", voz)
    pub extras: Vec<(String, Voz)>,
}

impl VozesFamilia {
    /// `quem` é "narrador", o slug do personagem ou a chave de uma voz extra ("pedro+ingles").
    pub fn de(&self, quem: &str) -> Voz {
        if quem == "narrador" {
            return self.narrador.clone();
        }
        if let Some((_, v)) = self.extras.iter().find(|(k, _)| k == quem) {
            return v.clone();
        }
        let (base, _) = separar(quem);
        self.personagens
            .iter()
            .find(|(s, _)| s == base)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| self.narrador.clone())
    }
}

pub async fn vozes_da_familia(st: &AppState, familia_id: Uuid) -> anyhow::Result<VozesFamilia> {
    let config: Value = sqlx::query_scalar("SELECT config FROM acesso.familias WHERE id = $1")
        .bind(familia_id)
        .fetch_one(&st.db)
        .await?;
    let narrador = Voz::de_json(&config["narrador"]).unwrap_or_else(narrador_padrao);
    let linhas: Vec<(String, String, Value, Value)> =
        sqlx::query_as("SELECT slug, tipo, voz_padrao, vozes_extras FROM conteudo.personagens WHERE familia_id = $1")
            .bind(familia_id)
            .fetch_all(&st.db)
            .await?;
    let mut personagens = Vec::new();
    let mut extras = Vec::new();
    for (slug, tipo, v, ex) in linhas {
        for e in serde_json::from_value::<Vec<VozExtra>>(ex).unwrap_or_default() {
            extras.push((format!("{slug}+{}", e.slug), e.voz.validada()));
        }
        personagens.push((slug, Voz::de_json(&v).unwrap_or_else(|| voz_por_tipo(&tipo))));
    }
    Ok(VozesFamilia { narrador, personagens, extras })
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn valida_voz() {
        let v = Voz::nova("voz-inexistente", 500, -500).validada();
        assert_eq!(v.id, VOZES[0].0);
        assert_eq!((v.tom, v.velocidade), (60, -40));
        assert_ne!(hash(&narrador_padrao(), "oi"), hash(&narrador_padrao(), "olá"));
        assert_eq!(separar("pedro+ingles"), ("pedro", Some("ingles")));
        assert_eq!(separar("pedro"), ("pedro", None));
    }
}

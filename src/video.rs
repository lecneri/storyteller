//! Exportar a história em vídeo MP4 (H.264 + AAC, 1280×720), que abre em qualquer celular, TV e no WhatsApp:
//! imagem da página à esquerda, texto à direita (o trecho que está sendo lido em destaque) e a narração já gerada.
//!
//! Cuidado com o disco: um vídeo por história (o novo substitui o antigo), apagado depois de 24 h;
//! temporários apagados sempre; um vídeo por vez; não começa com menos de 2 GB livres.

use std::{
    collections::HashMap,
    path::{Path as CaminhoFs, PathBuf},
    process::Stdio,
    sync::{LazyLock, Mutex},
    time::{Duration, SystemTime},
};

use askama::Template;
use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::{process::Command, sync::Semaphore};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    web::{erro_interno, erro_json, historia_da_familia, render},
    AppState,
};

const LARGURA: u32 = 1280;
const ALTURA: u32 = 720;
/// Pausa depois de cada trecho e no fim de cada página (s).
const PAUSA: f64 = 0.6;
const FIM_PAGINA: f64 = 1.0;
const CAPA: f64 = 4.0;
const MIN_LIVRE_MB: u64 = 2048;
const VALIDADE: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone, Serialize)]
pub struct Estado {
    /// "fila", "fazendo", "pronto" ou "erro"
    estado: &'static str,
    progresso: String,
    erro: Option<String>,
    aviso: Option<String>,
}

static ESTADOS: LazyLock<Mutex<HashMap<Uuid, Estado>>> = LazyLock::new(Default::default);
/// Um vídeo por vez (o servidor tem 2 núcleos).
static UM_POR_VEZ: Semaphore = Semaphore::const_new(1);

fn marcar(id: Uuid, e: Estado) {
    if let Ok(mut m) = ESTADOS.lock() {
        m.insert(id, e);
    }
}

fn estado_atual(id: Uuid) -> Option<Estado> {
    ESTADOS.lock().ok()?.get(&id).cloned()
}

fn pasta(st: &AppState) -> PathBuf {
    st.data_dir.join("exportacoes")
}

fn arquivo(st: &AppState, id: Uuid) -> PathBuf {
    pasta(st).join(format!("{id}.mp4"))
}

/// Espaço livre (MB) no disco da pasta.
async fn livre_mb(dir: &CaminhoFs) -> Option<u64> {
    let out = Command::new("df").arg("-Pk").arg(dir).output().await.ok()?;
    let texto = String::from_utf8_lossy(&out.stdout);
    let kb: u64 = texto.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb / 1024)
}

/// Apaga vídeos com mais de 24 h e temporários esquecidos (de um vídeo que caiu no meio).
pub async fn limpar_antigos(st: &AppState) -> u64 {
    let mut liberado = 0;
    let Ok(mut dir) = tokio::fs::read_dir(pasta(st)).await else { return 0 };
    let ocupado = UM_POR_VEZ.available_permits() == 0;
    while let Ok(Some(e)) = dir.next_entry().await {
        let Ok(meta) = e.metadata().await else { continue };
        let idade = meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).unwrap_or_default();
        let nome = e.file_name().to_string_lossy().to_string();
        if meta.is_dir() && nome.starts_with("tmp-") && !ocupado {
            let _ = tokio::fs::remove_dir_all(e.path()).await;
        } else if meta.is_file() && idade > VALIDADE && tokio::fs::remove_file(e.path()).await.is_ok() {
            liberado += meta.len();
        }
    }
    liberado
}

// ───────────── legendas (ASS) ─────────────

fn tempo_ass(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

/// Texto seguro para o ASS ({ } e \ são comandos dele).
fn ass_seguro(s: &str) -> String {
    s.trim().replace('\\', "/").replace('{', "(").replace('}', ")").replace(['\r', '\n'], " ")
}

fn cabecalho_ass() -> String {
    format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {LARGURA}\nPlayResY: {ALTURA}\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, \
         ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Texto,DejaVu Sans,34,&H002B2B2B,&H002B2B2B,&H00F2FAFF,&H00000000,0,0,0,0,100,100,0,0,1,0,0,4,{},48,36,1\n\
         Style: Titulo,DejaVu Sans,58,&H002B61C0,&H002B61C0,&H00F2FAFF,&H00000000,1,0,0,0,100,100,0,0,1,0,0,5,{},48,36,1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        LARGURA / 2 + 36,
        LARGURA / 2 + 36,
    )
}

/// Um trecho lido: texto, quem fala (só nas falas), áudio e duração.
pub struct Trecho {
    pub texto: String,
    pub nome: Option<String>,
    pub audio: Option<PathBuf>,
    pub dur: f64,
}

/// Tamanho da letra pelo tamanho do texto da página (texto longo, letra menor, para caber).
fn fonte_para(letras: usize) -> u32 {
    match letras {
        0..=260 => 36,
        261..=420 => 31,
        421..=620 => 26,
        _ => 22,
    }
}

/// Legenda da página: o texto todo, com o trecho que está sendo lido em destaque e os outros apagadinhos.
pub fn ass_pagina(trechos: &[Trecho]) -> String {
    let letras: usize = trechos.iter().map(|t| t.texto.chars().count() + t.nome.as_ref().map_or(0, |n| n.len())).sum();
    let fs = fonte_para(letras);
    let mut s = cabecalho_ass();
    let mut t0 = 0.0;
    let total: f64 = trechos.iter().map(|t| t.dur + PAUSA).sum::<f64>() + FIM_PAGINA;
    for (k, _) in trechos.iter().enumerate() {
        let t1 = if k + 1 == trechos.len() { total } else { t0 + trechos[k].dur + PAUSA };
        let corpo: Vec<String> = trechos
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let cor = match (i == k, t.nome.is_some()) {
                    (true, true) => "&H894E1D&",
                    (true, false) => "&H2B2B2B&",
                    (false, _) => "&HA8B4BE&",
                };
                let nome = t.nome.as_ref().map(|n| format!("{{\\c&H2B61C0&\\fs{}\\b1}}{}{{\\b0\\fs{fs}}}\\N", fs * 6 / 10, ass_seguro(&n.to_uppercase())));
                format!("{}{{\\c{cor}}}{}{}", nome.unwrap_or_default(), if t.nome.is_some() { "— " } else { "" }, ass_seguro(&t.texto))
            })
            .collect();
        s.push_str(&format!("Dialogue: 0,{},{},Texto,,0,0,0,,{{\\fs{fs}}}{}\n", tempo_ass(t0), tempo_ass(t1), corpo.join("\\N\\N")));
        t0 = t1;
    }
    s
}

/// Duração da página: cada trecho + pausa, e um respiro no fim (página sem texto: 3 s).
pub fn duracao_pagina(trechos: &[Trecho]) -> f64 {
    if trechos.is_empty() {
        return 3.0;
    }
    trechos.iter().map(|t| t.dur + PAUSA).sum::<f64>() + FIM_PAGINA
}

/// Sem áudio: tempo de leitura (~2,5 palavras por segundo, no mínimo 2 s).
fn tempo_de_leitura(texto: &str) -> f64 {
    (texto.split_whitespace().count() as f64 / 2.5).max(2.0)
}

// ───────────── ffmpeg ─────────────

/// Argumentos do ffmpeg de uma página (ou da capa): imagem/vídeo à esquerda, legenda à direita, narração.
pub fn args_pagina(visual: Option<(&CaminhoFs, &str)>, trechos: &[Trecho], dur: f64, ass: &str, saida: &str) -> Vec<String> {
    let mut a: Vec<String> = ["-y", "-v", "error"].iter().map(|s| s.to_string()).collect();
    let mut entrada = 0;
    let tem_visual = if let Some((caminho, tipo)) = visual {
        if tipo == "image/gif" {
            a.extend(["-ignore_loop", "0"].map(String::from));
        } else if tipo.starts_with("video/") {
            a.extend(["-stream_loop", "-1"].map(String::from));
        } else {
            a.extend(["-loop", "1", "-framerate", "24"].map(String::from));
        }
        a.extend(["-i".into(), caminho.to_string_lossy().to_string()]);
        entrada += 1;
        true
    } else {
        false
    };
    let mut filtro = format!("color=c=0xFFFAF2:s={LARGURA}x{ALTURA}:r=24:d={dur:.3}[bg];");
    if tem_visual {
        filtro.push_str(&format!(
            "[0:v]scale={m}:{ALTURA}:force_original_aspect_ratio=increase,crop={m}:{ALTURA},setsar=1,fps=24,format=yuv420p[img];\
             [bg][img]overlay=x=0:y=0:eof_action=repeat[v0];",
            m = LARGURA / 2
        ));
    } else {
        filtro.push_str("[bg]null[v0];");
    }
    filtro.push_str(&format!("[v0]subtitles=f={ass}[v];"));
    let mut partes = Vec::new();
    for (k, t) in trechos.iter().enumerate() {
        let d = t.dur + PAUSA;
        match &t.audio {
            Some(p) => {
                a.extend(["-i".into(), p.to_string_lossy().to_string()]);
                filtro.push_str(&format!(
                    "[{entrada}:a]aresample=44100,aformat=sample_fmts=fltp:channel_layouts=stereo,apad,atrim=duration={d:.3}[a{k}];"
                ));
                entrada += 1;
            }
            None => filtro.push_str(&format!("anullsrc=r=44100:cl=stereo,atrim=duration={d:.3}[a{k}];")),
        }
        partes.push(format!("[a{k}]"));
    }
    let fim = if trechos.is_empty() { dur } else { FIM_PAGINA };
    filtro.push_str(&format!("anullsrc=r=44100:cl=stereo,atrim=duration={fim:.3}[af];"));
    partes.push("[af]".into());
    filtro.push_str(&format!("{}concat=n={}:v=0:a=1[a]", partes.join(""), partes.len()));
    a.extend(["-filter_complex".into(), filtro]);
    a.extend(
        ["-map", "[v]", "-map", "[a]", "-t", &format!("{dur:.3}"), "-r", "24", "-c:v", "libx264", "-preset", "veryfast", "-crf", "24",
         "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "128k", "-ar", "44100", "-ac", "2", saida]
            .map(String::from),
    );
    a
}

async fn ffmpeg(dir: &CaminhoFs, args: &[String]) -> anyhow::Result<()> {
    let r = Command::new("ffmpeg").args(args).current_dir(dir).stdin(Stdio::null()).kill_on_drop(true).output();
    let o = tokio::time::timeout(Duration::from_secs(15 * 60), r).await.map_err(|_| anyhow::anyhow!("o ffmpeg demorou demais"))??;
    if !o.status.success() {
        let erro = String::from_utf8_lossy(&o.stderr);
        anyhow::bail!("ffmpeg: {}", erro.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("falhou"));
    }
    Ok(())
}

/// Caminho no disco de uma URL do manifest ("/midia/<id>").
async fn caminho_midia(st: &AppState, url: &str) -> Option<PathBuf> {
    let id: Uuid = url.rsplit('/').next()?.parse().ok()?;
    let c: String = sqlx::query_scalar("SELECT caminho FROM midia.arquivos WHERE id = $1").bind(id).fetch_optional(&st.db).await.ok()??;
    Some(st.data_dir.join(c))
}

async fn tipo_midia(st: &AppState, url: &str) -> String {
    let Some(id) = url.rsplit('/').next().and_then(|s| s.parse::<Uuid>().ok()) else { return String::new() };
    sqlx::query_scalar("SELECT tipo_mime FROM midia.arquivos WHERE id = $1").bind(id).fetch_optional(&st.db).await.ok().flatten().unwrap_or_default()
}

/// Trechos de música do vídeo: junta os segmentos seguidos com a mesma música → (url, início, duração).
/// Segmento sem música (silêncio) não vira trecho.
pub fn trechos_musica(segmentos: &[(Option<String>, f64)]) -> Vec<(String, f64, f64)> {
    let mut v: Vec<(String, f64, f64)> = Vec::new();
    let (mut t, mut anterior): (f64, Option<&str>) = (0.0, None);
    for (url, dur) in segmentos {
        match (url.as_deref(), v.last_mut()) {
            (Some(u), Some(ultimo)) if anterior == Some(u) => ultimo.2 += dur,
            (Some(u), _) => v.push((u.to_string(), t, *dur)),
            (None, _) => {}
        }
        anterior = url.as_deref();
        t += dur;
    }
    v
}

/// Mistura as músicas no vídeo pronto: cada trecho entra e sai aos poucos (1,5 s) no seu tempo, em loop se for curta;
/// volume "cheio" sem narração e abaixa sozinho quando há voz (a própria narração comanda o compressor); fade no fim.
pub fn args_musica(video: &str, trechos: &[(PathBuf, f64, f64)], volume: f64, dur: f64, saida: &str) -> Vec<String> {
    let fade = if dur > 4.0 { format!(",afade=t=out:st={:.2}:d=3", dur - 3.0) } else { String::new() };
    let mut filtro = String::from("[0:a]asplit=2[voz][chave];");
    for (k, (_, ini, d)) in trechos.iter().enumerate() {
        let fd = (d / 2.0).min(1.5);
        let ms = (ini * 1000.0).round() as u64;
        filtro.push_str(&format!(
            "[{n}:a]aresample=44100,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={volume:.3},\
             atrim=0:{d:.3},asetpts=PTS-STARTPTS,afade=t=in:d={fd:.2},afade=t=out:st={fo:.3}:d={fd:.2},adelay={ms}|{ms}[t{k}];",
            n = k + 1,
            fo = (d - fd).max(0.0),
        ));
    }
    if trechos.len() == 1 {
        filtro.push_str("[t0]anull[m];");
    } else {
        let ent: String = (0..trechos.len()).map(|k| format!("[t{k}]")).collect();
        filtro.push_str(&format!("{ent}amix=inputs={}:duration=longest:normalize=0[m];", trechos.len()));
    }
    filtro.push_str(&format!(
        "[m][chave]sidechaincompress=threshold=0.01:ratio=12:attack=60:release=900[md];\
         [voz][md]amix=inputs=2:duration=first:normalize=0{fade}[a]"
    ));
    let mut a: Vec<String> = ["-y", "-v", "error", "-i", video].map(String::from).to_vec();
    for (caminho, ..) in trechos {
        a.extend(["-stream_loop", "-1", "-i"].map(String::from));
        a.push(caminho.to_string_lossy().to_string());
    }
    a.extend(["-filter_complex", &filtro, "-map", "0:v", "-map", "[a]", "-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-shortest", "-movflags", "+faststart", saida].map(String::from));
    a
}

/// Monta o vídeo em `tmp` e devolve o aviso (trechos sem narração), se houver.
async fn montar(st: &AppState, id: Uuid, familia: Uuid, tmp: &CaminhoFs) -> anyhow::Result<Option<String>> {
    let (titulo, dados) = crate::player::manifest(st, id, familia).await?.ok_or_else(|| anyhow::anyhow!("história não encontrada"))?;
    let paginas = dados["paginas"].as_array().cloned().unwrap_or_default();
    if paginas.is_empty() {
        anyhow::bail!("a história não tem páginas");
    }
    let mut lista = String::new();
    let mut mudos = 0;

    // Capa: imagem + título.
    let capa = match dados["capa"].as_str() {
        Some(u) => caminho_midia(st, u).await.map(|p| (p, "image/png".to_string())),
        None => None,
    };
    let ass = format!(
        "{}Dialogue: 0,{},{},Titulo,,0,0,0,,{}\n",
        cabecalho_ass(),
        tempo_ass(0.0),
        tempo_ass(CAPA),
        ass_seguro(&titulo)
    );
    tokio::fs::write(tmp.join("capa.ass"), ass).await?;
    ffmpeg(tmp, &args_pagina(capa.as_ref().map(|(p, t)| (p.as_path(), t.as_str())), &[], CAPA, "capa.ass", "capa.mp4")).await?;
    lista.push_str("file 'capa.mp4'\n");
    // Linha do tempo da música: (música, duração) de cada pedaço do vídeo; a capa usa a da primeira página.
    let mus = |p: &Value| p["musica"].as_str().map(String::from);
    let mut segmentos: Vec<(Option<String>, f64)> = vec![(paginas.first().and_then(mus), CAPA)];

    for (i, p) in paginas.iter().enumerate() {
        marcar(id, Estado { estado: "fazendo", progresso: format!("página {} de {}", i + 1, paginas.len()), erro: None, aviso: None });
        let mut trechos = Vec::new();
        for b in p["blocos"].as_array().into_iter().flatten() {
            let texto = b["texto"].as_str().unwrap_or_default().to_string();
            let audio = match b["audio"].as_str() {
                Some(u) => caminho_midia(st, u).await.filter(|p| p.exists()),
                None => None,
            };
            let dur = match (&audio, b["duracao"].as_f64()) {
                (Some(_), Some(d)) if d > 0.0 => d,
                _ => {
                    mudos += 1;
                    tempo_de_leitura(&texto)
                }
            };
            let fala = b["quem"].as_str().is_some_and(|q| q != "narrador") && dados["narracao"] != "narrador";
            trechos.push(Trecho { nome: fala.then(|| b["nome"].as_str().unwrap_or_default().to_string()), texto, audio: audio.filter(|_| dur > 0.0), dur });
        }
        let visual = match p["midia"]["url"].as_str() {
            Some(u) => match caminho_midia(st, u).await {
                Some(c) => Some((c, tipo_midia(st, u).await)),
                None => None,
            },
            None => None,
        };
        let nome_ass = format!("p{i:03}.ass");
        let nome_mp4 = format!("p{i:03}.mp4");
        tokio::fs::write(tmp.join(&nome_ass), ass_pagina(&trechos)).await?;
        let dur = duracao_pagina(&trechos);
        segmentos.push((mus(p), dur));
        ffmpeg(tmp, &args_pagina(visual.as_ref().map(|(c, t)| (c.as_path(), t.as_str())), &trechos, dur, &nome_ass, &nome_mp4)).await?;
        lista.push_str(&format!("file '{nome_mp4}'\n"));
    }
    // Músicas com licença que pede crédito: cartão "Fim" com os créditos (o fim segue a música da última página).
    let creditos: Vec<String> = dados["creditos_musica"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect();
    if !creditos.is_empty() {
        segmentos.push((paginas.last().and_then(mus), CAPA));
        let credito = creditos.join(" · ");
        let credito = credito.as_str();
        let ass = format!(
            "{}Dialogue: 0,{},{},Titulo,,0,0,0,,Fim!\\N\\N{{\\fs26\\b0}}Música: {}\n",
            cabecalho_ass(),
            tempo_ass(0.0),
            tempo_ass(CAPA),
            ass_seguro(credito)
        );
        tokio::fs::write(tmp.join("fim.ass"), ass).await?;
        ffmpeg(tmp, &args_pagina(capa.as_ref().map(|(p, t)| (p.as_path(), t.as_str())), &[], CAPA, "fim.ass", "fim.mp4")).await?;
        lista.push_str("file 'fim.mp4'\n");
    }
    marcar(id, Estado { estado: "fazendo", progresso: "juntando as páginas".into(), erro: None, aviso: None });
    tokio::fs::write(tmp.join("lista.txt"), lista).await?;
    // Música de fundo (opcional, pode mudar entre as páginas): entra por último, por cima do vídeo já pronto.
    let mut musica: Vec<(PathBuf, f64, f64)> = Vec::new();
    for (u, ini, d) in trechos_musica(&segmentos) {
        if let Some(c) = caminho_midia(st, &u).await.filter(|p| p.exists()) {
            musica.push((c, ini, d));
        }
    }
    let juntado = if musica.is_empty() { "final.mp4" } else { "sem-musica.mp4" };
    let args: Vec<String> = ["-y", "-v", "error", "-f", "concat", "-safe", "0", "-i", "lista.txt", "-c", "copy", "-movflags", "+faststart", juntado]
        .map(String::from)
        .to_vec();
    ffmpeg(tmp, &args).await?;
    if !musica.is_empty() {
        marcar(id, Estado { estado: "fazendo", progresso: "pondo a música de fundo".into(), erro: None, aviso: None });
        let dur = crate::tts::duracao_arquivo(&tmp.join(juntado)).await.unwrap_or(0.0) as f64;
        let volume = dados["musica_volume"].as_f64().unwrap_or(0.35);
        ffmpeg(tmp, &args_musica(juntado, &musica, volume, dur, "final.mp4")).await?;
    }
    Ok((mudos > 0).then(|| format!("{mudos} trecho(s) sem narração ficaram em silêncio. Gere a narração no editor e faça o vídeo de novo.")))
}

async fn exportar(st: AppState, id: Uuid, familia: Uuid) {
    let _vez = UM_POR_VEZ.acquire().await;
    let tmp = pasta(&st).join(format!("tmp-{id}"));
    let r = async {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        tokio::fs::create_dir_all(&tmp).await?;
        let aviso = montar(&st, id, familia, &tmp).await?;
        tokio::fs::rename(tmp.join("final.mp4"), arquivo(&st, id)).await?;
        anyhow::Ok(aviso)
    }
    .await;
    // Temporários sempre saem do disco.
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    match r {
        Ok(aviso) => marcar(id, Estado { estado: "pronto", progresso: String::new(), erro: None, aviso }),
        Err(e) => {
            tracing::warn!("vídeo da história {id} falhou: {e:#}");
            let _ = tokio::fs::remove_file(arquivo(&st, id)).await;
            marcar(id, Estado { estado: "erro", progresso: String::new(), erro: Some(format!("{e:#}")), aviso: None });
        }
    }
}

/// A história foi apagada: o vídeo dela sai junto.
pub async fn apagar_video(st: &AppState, id: Uuid) {
    let _ = tokio::fs::remove_file(arquivo(st, id)).await;
    if let Ok(mut m) = ESTADOS.lock() {
        m.remove(&id);
    }
}

// ───────────── rotas ─────────────

async fn info(st: &AppState, id: Uuid) -> Value {
    let e = estado_atual(id);
    let meta = tokio::fs::metadata(arquivo(st, id)).await.ok();
    let pronto = meta.as_ref().map(|m| {
        let feito: chrono::DateTime<chrono::Local> = m.modified().unwrap_or(SystemTime::now()).into();
        json!({ "mb": (m.len() as f64 / 1048576.0 * 10.0).round() / 10.0, "feito": feito.format("%d/%m %H:%M").to_string() })
    });
    let estado = match &e {
        Some(e) => json!(e),
        None if pronto.is_some() => json!({ "estado": "pronto" }),
        None => json!({ "estado": "nada" }),
    };
    json!({ "video": estado, "arquivo": pronto })
}

#[derive(Template)]
#[template(path = "exportar.html")]
struct PaginaExportar {
    s: Sessao,
    id: Uuid,
    titulo: String,
    info: String,
}

/// GET /historias/{id}/exportar
pub async fn pagina(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let titulo: Option<String> = match sqlx::query_scalar("SELECT titulo FROM conteudo.historias WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .fetch_optional(&st.db)
        .await
    {
        Ok(t) => t,
        Err(e) => return erro_interno(e.into()),
    };
    let Some(titulo) = titulo else { return Redirect::to("/historias").into_response() };
    let info = info(&st, id).await.to_string().replace("</", "<\\/");
    render(PaginaExportar { s, id, titulo, info })
}

/// POST /historias/{id}/video — começa (ou refaz) o vídeo.
pub async fn fazer(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    if estado_atual(id).is_some_and(|e| e.estado == "fila" || e.estado == "fazendo") {
        return Json(info(&st, id).await).into_response();
    }
    if let Err(e) = tokio::fs::create_dir_all(pasta(&st)).await {
        return erro_interno(e.into());
    }
    limpar_antigos(&st).await;
    if livre_mb(&pasta(&st)).await.is_some_and(|mb| mb < MIN_LIVRE_MB) {
        return erro_json("O servidor está com pouco espaço em disco: não dá para fazer o vídeo agora.");
    }
    // O vídeo antigo sai antes (não guarda dois).
    let _ = tokio::fs::remove_file(arquivo(&st, id)).await;
    let ocupado = UM_POR_VEZ.available_permits() == 0;
    marcar(id, Estado {
        estado: if ocupado { "fila" } else { "fazendo" },
        progresso: if ocupado { "esperando outro vídeo terminar".into() } else { "começando".into() },
        erro: None,
        aviso: None,
    });
    tokio::spawn(exportar(st.clone(), id, s.familia_id));
    Json(info(&st, id).await).into_response()
}

/// GET /historias/{id}/video/estado
pub async fn estado(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    match historia_da_familia(&st, s.familia_id, id).await {
        Ok(true) => Json(info(&st, id).await).into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// GET /historias/{id}/video/baixar e /video/ver — o vídeo pronto, para baixar ou assistir.
/// A URL não termina em .mp4 e toda resposta é "no-store": o Cloudflare guarda em cache arquivos .mp4
/// (sem olhar o login) e chegou a servir um 404 antigo depois de o vídeo ficar pronto.
pub async fn baixar(st: State<AppState>, s: Sessao, id: Path<Uuid>, req: Request) -> Response {
    sem_cache(servir(st, s, id, req, true).await)
}

pub async fn ver(st: State<AppState>, s: Sessao, id: Path<Uuid>, req: Request) -> Response {
    sem_cache(servir(st, s, id, req, false).await)
}

fn sem_cache(mut r: Response) -> Response {
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    r
}

async fn servir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, req: Request, anexo: bool) -> Response {
    let slug: Option<String> = match sqlx::query_scalar("SELECT slug FROM conteudo.historias WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .fetch_optional(&st.db)
        .await
    {
        Ok(t) => t,
        Err(e) => return erro_interno(e.into()),
    };
    let Some(slug) = slug else { return StatusCode::NOT_FOUND.into_response() };
    let caminho = arquivo(&st, id);
    if !caminho.exists() {
        // Ainda não fez (ou já passou das 24 h): volta para a página de exportar.
        return Redirect::to(&format!("/historias/{id}/exportar")).into_response();
    }
    match tower_http::services::ServeFile::new(caminho).try_call(req).await {
        Ok(r) => {
            let mut r = r.map(Body::new);
            let nome = if slug.is_empty() { "historia" } else { &slug };
            let disp = format!("{}; filename=\"{nome}.mp4\"", if anexo { "attachment" } else { "inline" });
            if let Ok(v) = HeaderValue::from_str(&disp) {
                r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
            }
            r
        }
        Err(e) => erro_interno(e.into()),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn musica_do_video_em_trechos() {
        let s = |u: Option<&str>, d: f64| (u.map(String::from), d);
        let v = trechos_musica(&[s(Some("g"), 4.0), s(Some("g"), 10.0), s(Some("a"), 8.0), s(None, 5.0), s(Some("g"), 6.0)]);
        assert_eq!(v, vec![("g".into(), 0.0, 14.0), ("a".into(), 14.0, 8.0), ("g".into(), 27.0, 6.0)]);
        let a = args_musica("v.mp4", &[(PathBuf::from("g.mp3"), 0.0, 14.0), (PathBuf::from("a.mp3"), 14.0, 8.0)], 0.35, 22.0, "f.mp4");
        let f = &a[a.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert!(f.contains("adelay=14000|14000[t1]"));
        assert!(f.contains("amix=inputs=2:duration=longest"));
        assert_eq!(a.iter().filter(|x| *x == "-stream_loop").count(), 2);
    }

    fn t(texto: &str, nome: Option<&str>, dur: f64) -> Trecho {
        Trecho { texto: texto.into(), nome: nome.map(String::from), audio: None, dur }
    }

    #[test]
    fn tempos_e_texto_seguro() {
        assert_eq!(tempo_ass(0.0), "0:00:00.00");
        assert_eq!(tempo_ass(83.456), "0:01:23.46");
        assert_eq!(ass_seguro("a {b}\\c\nd"), "a (b)/c d");
    }

    #[test]
    fn legenda_destaca_um_trecho_por_vez() {
        let tr = vec![t("Era uma vez.", None, 2.0), t("Oi!", Some("Pedro"), 1.0)];
        let ass = ass_pagina(&tr);
        let linhas: Vec<&str> = ass.lines().filter(|l| l.starts_with("Dialogue:")).collect();
        assert_eq!(linhas.len(), 2);
        assert!(linhas[0].starts_with("Dialogue: 0,0:00:00.00,0:00:02.60,"));
        // A última vai até o fim da página (com o respiro).
        assert!(linhas[1].starts_with("Dialogue: 0,0:00:02.60,0:00:05.20,"));
        assert!(linhas[1].contains("PEDRO") && linhas[1].contains("— Oi!"));
        assert_eq!(duracao_pagina(&tr), 5.2);
        assert_eq!(duracao_pagina(&[]), 3.0);
    }

    #[test]
    fn ffmpeg_junta_audio_e_silencio() {
        let tr = vec![t("Sem áudio", None, 2.0)];
        let a = args_pagina(Some((CaminhoFs::new("/x/img.png"), "image/png")), &tr, 3.6, "p000.ass", "p000.mp4");
        let filtro = &a[a.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert!(filtro.contains("anullsrc=r=44100:cl=stereo,atrim=duration=2.600[a0]"));
        assert!(filtro.contains("concat=n=2:v=0:a=1[a]"));
        assert!(filtro.contains("subtitles=f=p000.ass"));
        assert!(a.contains(&"-loop".to_string()));
        assert_eq!(a.last().unwrap(), "p000.mp4");
    }
}

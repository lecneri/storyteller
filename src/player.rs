//! Player: o livro (imagem | texto) com leitura livre ou narração.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    historias::Conteudo,
    tts,
    web::{erro_interno, render},
    AppState,
};

#[derive(sqlx::FromRow)]
struct HistoriaPlayer {
    id: Uuid,
    titulo: String,
    familia_id: Uuid,
    narracao: String,
    conteudo: Value,
    capa: Option<Uuid>,
}

/// Como escolher as vozes de cada trecho.
enum Vozes {
    /// Vozes da família que está lendo; personagem que ela não tem usa a voz padrão do tipo.
    Familia(tts::VozesFamilia),
    /// Link público: só vozes genéricas de IA.
    Genericas,
}

impl Vozes {
    /// `quem`: "narrador", slug ou chave de voz extra ("pedro+ingles").
    fn de(&self, quem: &str, tipo: &str) -> tts::Voz {
        let extra = tts::separar(quem).1.is_some();
        if let Some(v) = tts::voz_direta(quem) {
            return v;
        }
        match self {
            Vozes::Familia(v) if quem == "narrador" => v.narrador.clone(),
            Vozes::Familia(v) => v
                .extras
                .iter()
                .chain(v.personagens.iter())
                .find(|(s, _)| s == quem)
                .map(|(_, voz)| voz.clone())
                .unwrap_or_else(|| if extra { tts::generica_extra(tipo) } else { tts::voz_por_tipo(tipo) }),
            Vozes::Genericas if quem == "narrador" || tipo == "narrador" => tts::narrador_padrao(),
            Vozes::Genericas if extra => tts::generica_extra(tipo),
            Vozes::Genericas => tts::voz_por_tipo(tipo),
        }
    }
}

/// Monta o "manifest" do livro: páginas, mídia, efeitos e o áudio já gerado de cada trecho.
async fn montar(
    st: &AppState,
    historia: Uuid,
    familia_cache: Uuid,
    vozes: Vozes,
    prefixo_midia: &str,
    audio_url: &str,
) -> anyhow::Result<Option<(String, Value)>> {
    let Some(h) = sqlx::query_as::<_, HistoriaPlayer>(
        "SELECT id, titulo, familia_id, narracao, conteudo, capa FROM conteudo.historias WHERE id = $1",
    )
    .bind(historia)
    .fetch_optional(&st.db)
    .await?
    else {
        return Ok(None);
    };
    let nomes: Vec<(String, String, String)> =
        sqlx::query_as("SELECT slug, nome, tipo FROM conteudo.personagens WHERE familia_id = $1")
            .bind(h.familia_id)
            .fetch_all(&st.db)
            .await?;
    let c: Conteudo = serde_json::from_value(h.conteudo.clone()).unwrap_or_default();
    let url = |id: &Uuid| format!("{prefixo_midia}{id}");

    let mut paginas = Vec::new();
    for p in &c.paginas {
        let mut blocos = Vec::new();
        for b in p.blocos.iter().filter(|b| !b.texto.trim().is_empty()) {
            let quem = if h.narracao == "narrador" { "narrador".to_string() } else { b.chave_voz() };
            let (nome, tipo) = match b.quem.strip_prefix("voz:").map(|r| r.split('|').next().unwrap_or(r)) {
                // Voz do sistema sem personagem: o nome dado no editor (ou o da voz).
                Some(id) => (b.nome.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| tts::nome_voz(id)), "voz".to_string()),
                None => nomes
                    .iter()
                    .find(|(s, _, _)| *s == b.quem)
                    .map(|(_, n, t)| (n.clone(), t.clone()))
                    .unwrap_or_else(|| ("Narrador".into(), "narrador".into())),
            };
            let tipo_voz = if quem == "narrador" { "narrador" } else { tipo.as_str() };
            let audio = tts::no_cache(st, familia_cache, &b.texto, &vozes.de(&quem, tipo_voz)).await?;
            blocos.push(json!({
                "quem": b.quem, "voz": quem, "nome": nome, "tipo": tipo, "texto": b.texto.trim(),
                "audio": audio.as_ref().map(|a| url(&a.arquivo)),
                "duracao": audio.as_ref().map(|a| a.duracao),
                "palavras": audio.as_ref().map(|a| a.palavras.clone()),
            }));
        }
        paginas.push(json!({
            "midia": p.midia.as_ref().map(|m| json!({ "url": url(&m.id), "tipo": m.tipo })),
            "animacao": p.animacao.clone().unwrap_or_else(|| "nenhuma".into()),
            "som": p.som.unwrap_or(true),
            "blocos": blocos,
        }));
    }
    // Capa: a definida, ou a primeira imagem do livro.
    let capa = h.capa.map(|c| url(&c)).or_else(|| {
        c.paginas
            .iter()
            .filter_map(|p| p.midia.as_ref())
            .find(|m| m.tipo.starts_with("image/"))
            .map(|m| url(&m.id))
    });
    Ok(Some((
        h.titulo.clone(),
        json!({ "id": h.id, "titulo": h.titulo, "capa": capa, "narracao": h.narracao,
                "audio_url": audio_url, "paginas": paginas }),
    )))
}

/// Manifest para uma família logada (dona ou com quem a história foi compartilhada).
pub async fn manifest(st: &AppState, historia: Uuid, familia_leitora: Uuid) -> anyhow::Result<Option<(String, Value)>> {
    let vozes = Vozes::Familia(tts::vozes_da_familia(st, familia_leitora).await?);
    montar(st, historia, familia_leitora, vozes, "/midia/", &format!("/historias/{historia}/bloco-audio")).await
}

/// Manifest de link público: vozes genéricas e mídia servida pelo link.
pub async fn manifest_publico(st: &AppState, historia: Uuid, dona: Uuid, token: &str, vozes_familia: bool) -> anyhow::Result<Option<(String, Value)>> {
    let vozes = if vozes_familia { Vozes::Familia(tts::vozes_da_familia(st, dona).await?) } else { Vozes::Genericas };
    montar(st, historia, dona, vozes, &format!("/p/{token}/midia/"), &format!("/p/{token}/bloco-audio")).await
}

/// Voz de um trecho para a família leitora (usada também pelo áudio sob demanda).
pub async fn voz_para(st: &AppState, familia_leitora: Uuid, historia: Uuid, quem: &str) -> anyhow::Result<tts::Voz> {
    let vozes = tts::vozes_da_familia(st, familia_leitora).await?;
    let tipo: String = sqlx::query_scalar(
        "SELECT p.tipo FROM conteudo.personagens p JOIN conteudo.historias h ON h.familia_id = p.familia_id
          WHERE h.id = $1 AND p.slug = $2",
    )
    .bind(historia)
    .bind(tts::separar(quem).0)
    .fetch_optional(&st.db)
    .await?
    .unwrap_or_else(|| "narrador".into());
    Ok(Vozes::Familia(vozes).de(quem, &tipo))
}

#[derive(Template)]
#[template(path = "player.html")]
pub struct PaginaPlayer {
    pub titulo: String,
    pub dados: String,
    /// Para onde o botão "voltar" leva.
    pub voltar: String,
}

/// GET /ler/{id}
pub async fn ler(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    if !crate::partilha::pode_ler_historia(&st, s.familia_id, id).await {
        return Redirect::to("/").into_response();
    }
    match manifest(&st, id, s.familia_id).await {
        Ok(Some((titulo, dados))) => render(PaginaPlayer {
            titulo,
            dados: dados.to_string().replace("</", "<\\/"),
            voltar: if s.leitor.is_some() { "/crianca".into() } else { "/".into() },
        }),
        Ok(None) => Redirect::to("/").into_response(),
        Err(e) => erro_interno(e),
    }
}

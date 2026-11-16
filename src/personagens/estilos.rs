//! Ficha do personagem: fotos da pessoa (privadas) e estilos do personagem.
//!
//! - `ficha.fotos`: [uuid] — fotos reais; só servem para criar os estilos (nunca vão para as ilustrações).
//! - `ficha.estilos`: [{slug, nome, imagem}] — o mesmo personagem em traços diferentes (Pixar, anime, anos 90…).
//!   O primeiro é o principal.
//!
//! A variante (roupa/forma) escolhe um estilo da ficha pelo campo "estilo": ao gerar o desenho dela, a imagem
//! desse estilo vai na frente como rosto, idade e traço; e é usada quando a variante ainda não tem imagem.

use axum::{
    extract::{Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{auth::Sessao, midia, util, web::erro_interno, AppState};

use super::paginas::{buscar, detalhe_com};
use super::variantes::{ler_multipart, salvar_refs};

/// Sugestões de estilo no formulário.
pub const SUGESTOES: [&str; 8] = ["Pixar", "Anime", "Desenho anos 90", "Cartoon", "Aquarela", "Mangá", "Massinha", "Livro infantil clássico"];

#[derive(Clone)]
pub struct Estilo {
    pub slug: String,
    pub nome: String,
    pub imagem: Uuid,
}

/// Estilos da ficha (o primeiro é o principal).
pub fn estilos(ficha: &Value) -> Vec<Estilo> {
    ficha["estilos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some(Estilo {
                slug: e["slug"].as_str()?.to_string(),
                nome: e["nome"].as_str()?.to_string(),
                imagem: e["imagem"].as_str()?.parse().ok()?,
            })
        })
        .collect()
}

/// Imagem do estilo da ficha que a variante pede (pelo nome do estilo dela); senão a do estilo principal.
pub fn imagem_do_estilo(ficha: &Value, estilo_variante: Option<&str>) -> Option<(Uuid, String)> {
    let todos = estilos(ficha);
    let pedido = estilo_variante.map(util::slug).filter(|s| !s.is_empty());
    pedido
        .and_then(|p| todos.iter().find(|e| e.slug == p))
        .or(todos.first())
        .map(|e| (e.imagem, e.nome.clone()))
}

fn gravar_estilos(ficha: &mut Value, lista: &[Estilo]) {
    ficha["estilos"] = Value::Array(lista.iter().map(|e| json!({ "slug": e.slug, "nome": e.nome, "imagem": e.imagem })).collect());
}

/// Lê a ficha, muda e grava de volta.
async fn mudar_ficha(st: &AppState, familia: Uuid, slug: &str, f: impl FnOnce(&mut Value)) -> Result<(), sqlx::Error> {
    let mut ficha: Value = sqlx::query_scalar("SELECT COALESCE(ficha, '{}'::jsonb) FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2")
        .bind(familia)
        .bind(slug)
        .fetch_one(&st.db)
        .await?;
    f(&mut ficha);
    sqlx::query("UPDATE conteudo.personagens SET ficha = $3, atualizado_em = now() WHERE familia_id = $1 AND slug = $2")
        .bind(familia)
        .bind(slug)
        .bind(&ficha)
        .execute(&st.db)
        .await
        .map(|_| ())
}

/// Põe (ou troca) o estilo `nome` com esta imagem. O primeiro estilo criado vira o principal.
pub(super) async fn definir(st: &AppState, familia: Uuid, slug: &str, nome: &str, imagem: Uuid) -> Result<(), sqlx::Error> {
    let nome = nome.trim();
    let nome = if nome.is_empty() { "Principal" } else { nome };
    let eslug = util::slug(nome);
    mudar_ficha(st, familia, slug, |ficha| {
        let mut lista = estilos(ficha);
        match lista.iter_mut().find(|e| e.slug == eslug) {
            Some(e) => e.imagem = imagem,
            None => lista.push(Estilo { slug: eslug, nome: nome.to_string(), imagem }),
        }
        gravar_estilos(ficha, &lista);
    })
    .await
}

fn ir_ficha(slug: &str) -> Response {
    Redirect::to(&format!("/personagens/{slug}#ficha")).into_response()
}

/// Personagem novo com imagem: vira o estilo principal.
pub(super) async fn criar(st: &AppState, s: &Sessao, slug: &str, nome_estilo: &str, arquivos: &[Vec<u8>]) -> Result<(), String> {
    let ids = salvar_refs(st, s, slug, &arquivos[..1]).await?;
    let Some(id) = ids.first() else { return Ok(()) };
    definir(st, s.familia_id, slug, nome_estilo, *id).await.map_err(|e| {
        tracing::error!("estilo do personagem novo: {e:#}");
        "O personagem foi criado, mas não consegui guardar a imagem. Envie de novo na ficha.".to_string()
    })
}

/// Gera a imagem de um estilo pela API: pelas fotos e pela aparência; com outro estilo pronto, copia o rosto dele.
async fn gerar_estilo(st: &AppState, s: &Sessao, slug: &str, nome_estilo: &str) -> anyhow::Result<Uuid> {
    let p = buscar(st, s.familia_id, slug).await?.ok_or_else(|| anyhow::anyhow!("Personagem não encontrado."))?;
    let eslug = util::slug(nome_estilo);
    let todos = estilos(&p.ficha);
    // Rosto: o próprio estilo (ao refazer) ou o principal; depois as fotos.
    let rosto = todos.iter().find(|e| e.slug == eslug).or(todos.first()).map(|e| e.imagem);
    let mut ids: Vec<Uuid> = rosto.into_iter().collect();
    ids.extend(p.ficha["fotos"].as_array().into_iter().flatten().filter_map(|v| v.as_str()?.parse::<Uuid>().ok()));
    let refs = crate::imagem::carregar_refs(st, s.familia_id, &ids).await;
    let mut prompt = super::prompt_model_sheet(&p.nome, &p.campo("aparencia"), Some(nome_estilo), !refs.is_empty());
    if rosto.is_some() {
        prompt.push_str(&format!(
            "\nA primeira imagem anexada é o mesmo personagem: mantenha o rosto, o cabelo, a idade e as proporções, \
             mudando só o traço para o estilo {nome_estilo}."
        ));
    }
    let (img, _) = crate::imagem::gerar(st, s.familia_id, &s.familia_slug, &format!("personagens/{slug}/refs"), &prompt, &refs).await?;
    Ok(img)
}

/// POST /personagens/{slug}/estilos (multipart: nome, imagem opcional, gerar=1): novo estilo (ou troca a imagem
/// de um que já existe). Com imagem, usa ela; sem imagem, gera pela IA.
pub async fn adicionar(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, mp: Multipart) -> Response {
    let (campos, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    let nome = campos.get("nome").map(|n| n.trim().to_string()).unwrap_or_default();
    if util::slug(&nome).is_empty() {
        return detalhe_com(&st, s, &slug, Some("Dê um nome para o estilo (ex.: Pixar, Anime).".into())).await;
    }
    let imagem = if arquivos.is_empty() {
        match gerar_estilo(&st, &s, &slug, &nome).await {
            Ok(i) => i,
            Err(e) => return detalhe_com(&st, s, &slug, Some(format!("{e:#}"))).await,
        }
    } else {
        match salvar_refs(&st, &s, &slug, &arquivos[..1]).await {
            Ok(ids) => ids[0],
            Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
        }
    };
    match definir(&st, s.familia_id, &slug, &nome, imagem).await {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/estilos/{eslug}/gerar: refaz a imagem do estilo pela IA.
pub async fn refazer(State(st): State<AppState>, s: Sessao, Path((slug, eslug)): Path<(String, String)>) -> Response {
    let nome = match buscar(&st, s.familia_id, &slug).await {
        Ok(Some(p)) => estilos(&p.ficha).into_iter().find(|e| e.slug == eslug).map(|e| e.nome),
        Ok(None) => None,
        Err(e) => return erro_interno(e.into()),
    };
    let Some(nome) = nome else { return ir_ficha(&slug) };
    match gerar_estilo(&st, &s, &slug, &nome).await {
        Ok(img) => match definir(&st, s.familia_id, &slug, &nome, img).await {
            Ok(()) => ir_ficha(&slug),
            Err(e) => erro_interno(e.into()),
        },
        Err(e) => detalhe_com(&st, s, &slug, Some(format!("{e:#}"))).await,
    }
}

/// POST /personagens/{slug}/estilos/{eslug}/principal: passa o estilo para o começo da lista.
pub async fn principal(State(st): State<AppState>, s: Sessao, Path((slug, eslug)): Path<(String, String)>) -> Response {
    let r = mudar_ficha(&st, s.familia_id, &slug, |ficha| {
        let mut lista = estilos(ficha);
        if let Some(i) = lista.iter().position(|e| e.slug == eslug) {
            let e = lista.remove(i);
            lista.insert(0, e);
        }
        gravar_estilos(ficha, &lista);
    })
    .await;
    match r {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/estilos/{eslug}/tirar: tira o estilo (o arquivo sai na limpeza, se nada mais usar).
pub async fn tirar(State(st): State<AppState>, s: Sessao, Path((slug, eslug)): Path<(String, String)>) -> Response {
    let r = mudar_ficha(&st, s.familia_id, &slug, |ficha| {
        let lista: Vec<Estilo> = estilos(ficha).into_iter().filter(|e| e.slug != eslug).collect();
        gravar_estilos(ficha, &lista);
    })
    .await;
    match r {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/estilos/usar/{ref}: a imagem de uma variante vira um estilo da ficha
/// (com o nome do estilo da variante, ou "Principal").
pub async fn usar(State(st): State<AppState>, s: Sessao, Path((slug, ref_id)): Path<(String, Uuid)>) -> Response {
    let estilo: Option<Option<String>> = sqlx::query_scalar(
        "SELECT v.estilo FROM conteudo.variantes v JOIN conteudo.personagens p ON p.id = v.personagem_id
          WHERE p.familia_id = $1 AND p.slug = $2 AND $3 = ANY(v.referencias) LIMIT 1",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(ref_id)
    .fetch_optional(&st.db)
    .await
    .unwrap_or(None);
    let Some(estilo) = estilo else { return ir_ficha(&slug) };
    match definir(&st, s.familia_id, &slug, estilo.as_deref().unwrap_or("Principal"), ref_id).await {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/fotos (multipart "fotos"): fotos da pessoa (só no servidor; servem para criar os estilos).
pub async fn enviar_fotos(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>, mp: Multipart) -> Response {
    let (_, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    let mut novas = Vec::new();
    for bytes in &arquivos {
        let Some((mime, ext)) = midia::tipo_imagem(bytes).filter(|(m, _)| *m != "image/gif") else {
            return detalhe_com(&st, s, &slug, Some("Só fotos PNG, JPG ou WebP.".into())).await;
        };
        match midia::salvar(&st, s.familia_id, &format!("familias/{}/personagens/{slug}/fotos", s.familia_slug), bytes, mime, ext).await {
            Ok(id) => novas.push(id),
            Err(e) => return erro_interno(e),
        }
    }
    let r = mudar_ficha(&st, s.familia_id, &slug, |ficha| {
        let mut fotos: Vec<Value> = ficha["fotos"].as_array().cloned().unwrap_or_default();
        fotos.extend(novas.iter().map(|id| json!(id)));
        ficha["fotos"] = Value::Array(fotos);
    })
    .await;
    match r {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/fotos/{foto}/tirar
pub async fn tirar_foto(State(st): State<AppState>, s: Sessao, Path((slug, foto)): Path<(String, Uuid)>) -> Response {
    let r = mudar_ficha(&st, s.familia_id, &slug, |ficha| {
        let fotos: Vec<Value> = ficha["fotos"].as_array().cloned().unwrap_or_default().into_iter().filter(|v| v.as_str() != Some(&foto.to_string())).collect();
        ficha["fotos"] = Value::Array(fotos);
    })
    .await;
    match r {
        Ok(()) => ir_ficha(&slug),
        Err(e) => erro_interno(e.into()),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn variante_escolhe_o_estilo_pelo_nome() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let ficha = json!({ "estilos": [
            { "slug": "pixar", "nome": "Pixar", "imagem": a },
            { "slug": "anime", "nome": "Anime", "imagem": b },
        ] });
        assert_eq!(imagem_do_estilo(&ficha, Some("Anime")).unwrap().0, b);
        assert_eq!(imagem_do_estilo(&ficha, Some(" anime ")).unwrap().0, b);
        // Estilo que a ficha não tem (ou nenhum): o principal.
        assert_eq!(imagem_do_estilo(&ficha, Some("Power Rangers")).unwrap().0, a);
        assert_eq!(imagem_do_estilo(&ficha, None).unwrap().0, a);
        assert!(imagem_do_estilo(&json!({}), Some("Anime")).is_none());
    }
}

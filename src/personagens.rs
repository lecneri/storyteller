//! Personagens e variantes (forma × estilo).

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::Sessao,
    midia, util,
    web::{erro_interno, render},
    AppState,
};

pub const TIPOS: [(&str, &str); 6] = [
    ("crianca", "Criança / herói"),
    ("animal", "Animal"),
    ("robo", "Robô / veículo"),
    ("vilao", "Vilão"),
    ("adulto", "Adulto"),
    ("criatura", "Criatura"),
];

fn nome_tipo(t: &str) -> &'static str {
    TIPOS.iter().find(|(k, _)| *k == t).map(|(_, v)| *v).unwrap_or("Outro")
}

// ───────────── lista ─────────────

#[derive(sqlx::FromRow)]
struct ItemLista {
    slug: String,
    nome: String,
    tipo: String,
    variante: Option<String>,
    miniatura: Option<Uuid>,
}

impl ItemLista {
    fn tipo_nome(&self) -> &'static str {
        nome_tipo(&self.tipo)
    }
}

#[derive(Template)]
#[template(path = "personagens.html")]
struct PaginaLista {
    s: Sessao,
    itens: Vec<ItemLista>,
    tipos: Vec<(&'static str, &'static str)>,
    erro: Option<String>,
}

async fn lista_com(st: &AppState, s: Sessao, erro: Option<String>) -> Response {
    // Miniatura: primeira referência da variante atual (ou de qualquer variante).
    let itens = sqlx::query_as::<_, ItemLista>(
        "SELECT p.slug, p.nome, p.tipo, va.nome AS variante,
                COALESCE(va.referencias[1],
                         (SELECT v.referencias[1] FROM conteudo.variantes v
                           WHERE v.personagem_id = p.id AND cardinality(v.referencias) > 0
                           ORDER BY v.criado_em LIMIT 1)) AS miniatura
           FROM conteudo.personagens p
           LEFT JOIN conteudo.variantes va ON va.id = p.variante_atual
          WHERE p.familia_id = $1
          ORDER BY p.nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    match itens {
        Ok(itens) => render(PaginaLista { s, itens, tipos: TIPOS.to_vec(), erro }),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn lista(State(st): State<AppState>, s: Sessao) -> Response {
    lista_com(&st, s, None).await
}

// ───────────── criar / editar ficha ─────────────

#[derive(Deserialize)]
pub struct FormFicha {
    nome: String,
    tipo: String,
    #[serde(default)]
    apelidos: String,
    #[serde(default)]
    aparencia: String,
    #[serde(default)]
    personalidade: String,
    #[serde(default)]
    jeito_falar: String,
}

impl FormFicha {
    fn ficha(&self) -> serde_json::Value {
        let apelidos: Vec<&str> = self.apelidos.split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
        json!({
            "apelidos": apelidos,
            "aparencia": self.aparencia.trim(),
            "personalidade": self.personalidade.trim(),
            "jeito_falar": self.jeito_falar.trim(),
        })
    }
    fn tipo_valido(&self) -> bool {
        TIPOS.iter().any(|(k, _)| *k == self.tipo)
    }
}

pub async fn criar(State(st): State<AppState>, s: Sessao, Form(f): Form<FormFicha>) -> Response {
    let nome = f.nome.trim().to_string();
    let slug = util::slug(&nome);
    if slug.is_empty() || !f.tipo_valido() {
        return lista_com(&st, s, Some("Preencha o nome e o tipo.".into())).await;
    }
    let r = sqlx::query(
        "INSERT INTO conteudo.personagens (familia_id, slug, nome, tipo, ficha) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&nome)
    .bind(&f.tipo)
    .bind(f.ficha())
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to(&format!("/personagens/{slug}")).into_response(),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            lista_com(&st, s, Some(format!("Já existe um personagem chamado {nome}."))).await
        }
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn salvar_ficha(
    State(st): State<AppState>,
    s: Sessao,
    Path(slug): Path<String>,
    Form(f): Form<FormFicha>,
) -> Response {
    if f.nome.trim().is_empty() || !f.tipo_valido() {
        return detalhe_com(&st, s, &slug, Some("Preencha o nome e o tipo.".into())).await;
    }
    let r = sqlx::query(
        "UPDATE conteudo.personagens SET nome = $3, tipo = $4, ficha = $5, atualizado_em = now()
          WHERE familia_id = $1 AND slug = $2",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(f.nome.trim())
    .bind(&f.tipo)
    .bind(f.ficha())
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to(&format!("/personagens/{slug}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── detalhe ─────────────

#[derive(sqlx::FromRow)]
struct Personagem {
    id: Uuid,
    slug: String,
    nome: String,
    tipo: String,
    ficha: serde_json::Value,
    variante_atual: Option<Uuid>,
}

impl Personagem {
    fn campo(&self, k: &str) -> String {
        self.ficha.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    }
    fn apelidos(&self) -> String {
        self.ficha
            .get("apelidos")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    }
}

#[derive(sqlx::FromRow)]
struct Variante {
    id: Uuid,
    slug: String,
    nome: String,
    forma: Option<String>,
    estilo: Option<String>,
    descricao: Option<String>,
    referencias: Vec<Uuid>,
    arquivada: bool,
}

#[derive(Template)]
#[template(path = "personagem.html")]
struct PaginaPersonagem {
    s: Sessao,
    p: Personagem,
    variantes: Vec<Variante>,
    tipos: Vec<(&'static str, &'static str)>,
    erro: Option<String>,
    imagem_api: bool,
}

impl PaginaPersonagem {
    fn fotos(&self) -> Vec<String> {
        self.p.ficha["fotos"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect()
    }
    fn prompt_kit(&self, v: &Variante) -> String {
        let aparencia = v.descricao.clone().unwrap_or_else(|| self.p.campo("aparencia"));
        prompt_model_sheet(&self.p.nome, &aparencia, v.estilo.as_deref(), !self.fotos().is_empty() || !v.referencias.is_empty())
    }
    fn eh_atual(&self, v: &Variante) -> bool {
        self.p.variante_atual == Some(v.id)
    }
}

async fn buscar(st: &AppState, familia_id: Uuid, slug: &str) -> Result<Option<Personagem>, sqlx::Error> {
    sqlx::query_as::<_, Personagem>(
        "SELECT id, slug, nome, tipo, ficha, variante_atual
           FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2",
    )
    .bind(familia_id)
    .bind(slug)
    .fetch_optional(&st.db)
    .await
}

async fn detalhe_com(st: &AppState, s: Sessao, slug: &str, erro: Option<String>) -> Response {
    let p = match buscar(st, s.familia_id, slug).await {
        Ok(Some(p)) => p,
        Ok(None) => return Redirect::to("/personagens").into_response(),
        Err(e) => return erro_interno(e.into()),
    };
    let variantes = sqlx::query_as::<_, Variante>(
        "SELECT id, slug, nome, forma, estilo, descricao, referencias, arquivada
           FROM conteudo.variantes WHERE personagem_id = $1 ORDER BY arquivada, criado_em",
    )
    .bind(p.id)
    .fetch_all(&st.db)
    .await;
    match variantes {
        Ok(variantes) => {
            let imagem_api = crate::imagem::disponivel(st, s.familia_id).await;
            render(PaginaPersonagem { s, p, variantes, tipos: TIPOS.to_vec(), erro, imagem_api })
        }
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn detalhe(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    detalhe_com(&st, s, &slug, None).await
}

// ───────────── variantes ─────────────

/// Lê um formulário multipart: campos de texto + imagens de referência.
async fn ler_multipart(mut mp: Multipart) -> Result<(HashMap<String, String>, Vec<Vec<u8>>), String> {
    let mut campos = HashMap::new();
    let mut arquivos = Vec::new();
    while let Some(campo) = mp.next_field().await.map_err(|e| format!("Envio inválido: {e}"))? {
        let nome = campo.name().unwrap_or_default().to_string();
        if campo.file_name().is_some() {
            let bytes = campo.bytes().await.map_err(|_| "Arquivo grande demais (máximo 25 MB).".to_string())?;
            if !bytes.is_empty() {
                arquivos.push(bytes.to_vec());
            }
        } else {
            campos.insert(nome, campo.text().await.unwrap_or_default());
        }
    }
    Ok((campos, arquivos))
}

/// Grava as imagens como referências e devolve os IDs. Recusa o que não for imagem.
async fn salvar_refs(st: &AppState, s: &Sessao, p_slug: &str, arquivos: &[Vec<u8>]) -> Result<Vec<Uuid>, String> {
    let pasta = format!("familias/{}/personagens/{p_slug}/refs", s.familia_slug);
    let mut ids = Vec::new();
    for bytes in arquivos {
        let (mime, ext) = midia::tipo_imagem(bytes).ok_or("Só imagens PNG, JPG, WebP ou GIF.")?;
        let id = midia::salvar(st, s.familia_id, &pasta, bytes, mime, ext)
            .await
            .map_err(|e| {
                tracing::error!("falha ao salvar referência: {e:#}");
                "Não consegui salvar a imagem.".to_string()
            })?;
        ids.push(id);
    }
    Ok(ids)
}

pub async fn criar_variante(
    State(st): State<AppState>,
    s: Sessao,
    Path(slug): Path<String>,
    mp: Multipart,
) -> Response {
    let p = match buscar(&st, s.familia_id, &slug).await {
        Ok(Some(p)) => p,
        Ok(None) => return Redirect::to("/personagens").into_response(),
        Err(e) => return erro_interno(e.into()),
    };
    let (campos, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    let texto = |k: &str| campos.get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let Some(nome) = texto("nome") else {
        return detalhe_com(&st, s, &slug, Some("Dê um nome para a variante.".into())).await;
    };
    let refs = match salvar_refs(&st, &s, &slug, &arquivos).await {
        Ok(r) => r,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    let r = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO conteudo.variantes (personagem_id, slug, nome, forma, estilo, descricao, referencias)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(p.id)
    .bind(util::slug(&nome))
    .bind(&nome)
    .bind(texto("forma"))
    .bind(texto("estilo"))
    .bind(texto("descricao"))
    .bind(&refs)
    .fetch_one(&st.db)
    .await;
    let vid = match r {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return detalhe_com(&st, s, &slug, Some(format!("Já existe a variante {nome}."))).await
        }
        Err(e) => return erro_interno(e.into()),
    };
    // A primeira variante vira a atual automaticamente.
    if p.variante_atual.is_none() || campos.get("atual").is_some() {
        if let Err(e) = definir_atual(&st, p.id, vid).await {
            return erro_interno(e.into());
        }
    }
    Redirect::to(&format!("/personagens/{slug}")).into_response()
}

async fn definir_atual(st: &AppState, personagem_id: Uuid, variante_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE conteudo.personagens SET variante_atual = $2, atualizado_em = now() WHERE id = $1")
        .bind(personagem_id)
        .bind(variante_id)
        .execute(&st.db)
        .await
        .map(|_| ())
}

pub async fn marcar_atual(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, vslug)): Path<(String, String)>,
) -> Response {
    let r = sqlx::query(
        "UPDATE conteudo.personagens p SET variante_atual = v.id, atualizado_em = now()
           FROM conteudo.variantes v
          WHERE v.personagem_id = p.id AND p.familia_id = $1 AND p.slug = $2 AND v.slug = $3",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&vslug)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to(&format!("/personagens/{slug}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn adicionar_refs(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, vslug)): Path<(String, String)>,
    mp: Multipart,
) -> Response {
    let (_, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    let refs = match salvar_refs(&st, &s, &slug, &arquivos).await {
        Ok(r) => r,
        Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
    };
    let r = sqlx::query(
        "UPDATE conteudo.variantes v SET referencias = v.referencias || $4
           FROM conteudo.personagens p
          WHERE v.personagem_id = p.id AND p.familia_id = $1 AND p.slug = $2 AND v.slug = $3",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&vslug)
    .bind(&refs)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => Redirect::to(&format!("/personagens/{slug}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/variantes/{vslug}/arquivar: arquiva ou restaura (alterna).
/// Arquivada não aparece para histórias novas; as antigas continuam usando.
pub async fn alternar_arquivo(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, vslug)): Path<(String, String)>,
) -> Response {
    let r = async {
        let mut tx = st.db.begin().await?;
        let linha: Option<(Uuid, Uuid, bool)> = sqlx::query_as(
            "UPDATE conteudo.variantes v SET arquivada = NOT v.arquivada
               FROM conteudo.personagens p
              WHERE v.personagem_id = p.id AND p.familia_id = $1 AND p.slug = $2 AND v.slug = $3
             RETURNING p.id, v.id, v.arquivada",
        )
        .bind(s.familia_id)
        .bind(&slug)
        .bind(&vslug)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((pid, vid, true)) = linha {
            // Se era a atual, passa para outra variante ativa (ou nenhuma).
            sqlx::query(
                "UPDATE conteudo.personagens SET variante_atual =
                   (SELECT id FROM conteudo.variantes WHERE personagem_id = $1 AND NOT arquivada ORDER BY criado_em LIMIT 1)
                  WHERE id = $1 AND variante_atual = $2",
            )
            .bind(pid)
            .bind(vid)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
    .await;
    match r {
        Ok(()) => Redirect::to(&format!("/personagens/{slug}")).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

// ───────────── criar personagem com IA (descrição e/ou fotos) ─────────────

/// Prompt do "model sheet" (folha de referência) de uma variante.
pub fn prompt_model_sheet(nome: &str, aparencia: &str, estilo: Option<&str>, com_fotos: bool) -> String {
    let estilo = estilo.filter(|e| !e.trim().is_empty()).map(|e| format!("{e}, ")).unwrap_or_default();
    let fotos = if com_fotos {
        "\nUse as imagens anexadas só como referência de traços (rosto, cabelo, cores, roupa) e transforme em desenho; não faça foto realista."
    } else {
        ""
    };
    format!(
        "Folha de personagem (model sheet) para livro infantil: {nome}. {aparencia}\n\
         Estilo: {estilo}cores vivas, traço limpo e amigável.\n\
         Corpo inteiro, de frente, pose neutra e simpática, fundo branco liso, formato retrato 3:4.\n\
         Não escreva nenhum texto, letra ou número na imagem.{fotos}"
    )
}

pub async fn criar_com_ia(State(st): State<AppState>, s: Sessao, mp: Multipart) -> Response {
    let (campos, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return lista_com(&st, s, Some(msg)).await,
    };
    let texto = |k: &str| campos.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let (nome, tipo, descricao, estilo) = (texto("nome"), texto("tipo"), texto("descricao"), texto("estilo"));
    let slug = util::slug(&nome);
    if slug.is_empty() || !TIPOS.iter().any(|(k, _)| *k == tipo) || (descricao.is_empty() && arquivos.is_empty()) {
        return lista_com(&st, s, Some("Dê um nome, o tipo e uma descrição ou foto.".into())).await;
    }
    if !arquivos.is_empty() && campos.get("consentimento").is_none() {
        return lista_com(&st, s, Some("Para usar fotos, marque a autorização.".into())).await;
    }

    // 1) Ficha pela IA de texto, olhando a descrição e as fotos.
    let mut imagens = Vec::new();
    for bytes in &arquivos {
        match midia::foto_para_ia(bytes) {
            Some(f) => imagens.push(f),
            None => return lista_com(&st, s, Some("Só imagens PNG, JPG ou WebP.".into())).await,
        }
    }
    let pedido = crate::ia::PedidoTexto {
        sistema: "Você cria personagens para livros infantis em português do Brasil. Responda só com o JSON pedido.".into(),
        imagens,
        prompt: format!(
            "Crie a ficha do personagem \"{nome}\" (tipo: {tipo}). Estilo visual de referência: {}.\n\
             Descrição da família: {}\n{}\
             Aparência: descrição visual detalhada e fixa (cores, roupa, traços) para manter consistência nas ilustrações.\n\
             Personalidade e jeito de falar: curtos, adequados para crianças. Apelidos: até 3.\n\
             nome_variante: nome curto da aparência atual (ex.: \"Super Léo Rangers\"); forma: normal, super, gigante...",
            if estilo.is_empty() { "livre" } else { &estilo },
            if descricao.is_empty() { "(sem descrição: a família enviou fotos)" } else { &descricao },
            if arquivos.is_empty() { "" } else { "As fotos anexadas mostram o personagem: descreva a aparência a partir delas (cabelo, olhos, pele, roupa, cores, traços marcantes), coerente com a descrição. Não cite que é uma foto.\n" },
        ),
        schema: serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["aparencia", "personalidade", "jeito_falar", "apelidos", "nome_variante", "forma"],
            "properties": {
                "aparencia": { "type": "string" }, "personalidade": { "type": "string" },
                "jeito_falar": { "type": "string" }, "apelidos": { "type": "array", "items": { "type": "string" } },
                "nome_variante": { "type": "string" }, "forma": { "type": "string" }
            }
        }),
    };
    let ficha = match crate::ia::gerar_texto(&st, s.familia_id, &pedido).await {
        Ok(f) => f,
        Err(e) => return lista_com(&st, s, Some(format!("A IA não conseguiu criar a ficha: {e:#}"))).await,
    };
    let aparencia = ficha["aparencia"].as_str().unwrap_or(&descricao).to_string();

    // 2) Fotos: guardadas só no servidor (pasta "fotos"), usadas apenas para gerar o desenho.
    let mut fotos = Vec::new();
    for bytes in &arquivos {
        let Some((mime, ext)) = midia::tipo_imagem(bytes) else {
            return lista_com(&st, s, Some("Só imagens PNG, JPG ou WebP.".into())).await;
        };
        match midia::salvar(&st, s.familia_id, &format!("familias/{}/personagens/{slug}/fotos", s.familia_slug), bytes, mime, ext).await {
            Ok(id) => fotos.push(id),
            Err(e) => return erro_interno(e),
        }
    }

    // 3) Personagem + variante.
    let ficha_json = serde_json::json!({
        "apelidos": ficha["apelidos"], "aparencia": aparencia,
        "personalidade": ficha["personalidade"], "jeito_falar": ficha["jeito_falar"], "fotos": fotos,
    });
    let nome_var = ficha["nome_variante"].as_str().filter(|v| !v.trim().is_empty()).unwrap_or("Principal").to_string();
    let r = async {
        let mut tx = st.db.begin().await?;
        let pid: Uuid = sqlx::query_scalar(
            "INSERT INTO conteudo.personagens (familia_id, slug, nome, tipo, ficha) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(s.familia_id)
        .bind(&slug)
        .bind(&nome)
        .bind(&tipo)
        .bind(&ficha_json)
        .fetch_one(&mut *tx)
        .await?;
        let vid: Uuid = sqlx::query_scalar(
            "INSERT INTO conteudo.variantes (personagem_id, slug, nome, forma, estilo, descricao)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(pid)
        .bind(util::slug(&nome_var))
        .bind(&nome_var)
        .bind(ficha["forma"].as_str())
        .bind(Some(estilo.as_str()).filter(|e| !e.is_empty()))
        .bind(&aparencia)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE conteudo.personagens SET variante_atual = $2 WHERE id = $1").bind(pid).bind(vid).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok::<_, sqlx::Error>(vid)
    }
    .await;
    let vid = match r {
        Ok(v) => v,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return lista_com(&st, s, Some(format!("Já existe um personagem chamado {nome}."))).await
        }
        Err(e) => return erro_interno(e.into()),
    };

    // 4) Desenho de referência pela API de imagem (se houver); senão fica o kit manual na página do personagem.
    let mut destino = format!("/personagens/{slug}");
    if crate::imagem::disponivel(&st, s.familia_id).await {
        let refs = crate::imagem::carregar_refs(&st, s.familia_id, &fotos).await;
        let prompt = prompt_model_sheet(&nome, &aparencia, Some(&estilo), !refs.is_empty());
        match crate::imagem::gerar(&st, s.familia_id, &s.familia_slug, &format!("personagens/{slug}/refs"), &prompt, &refs).await {
            Ok((img, _)) => {
                let _ = sqlx::query("UPDATE conteudo.variantes SET referencias = array_append(referencias, $2) WHERE id = $1")
                    .bind(vid)
                    .bind(img)
                    .execute(&st.db)
                    .await;
            }
            Err(e) => {
                tracing::warn!("model sheet pela API falhou: {e:#}");
                destino.push_str("?kit=erro");
            }
        }
    }
    Redirect::to(&destino).into_response()
}

/// POST /personagens/{slug}/variantes/{vslug}/gerar-ref: gera mais um desenho de referência pela API.
pub async fn gerar_ref(State(st): State<AppState>, s: Sessao, Path((slug, vslug)): Path<(String, String)>) -> Response {
    let linha: Option<(Uuid, String, serde_json::Value, Option<String>, Option<String>, Vec<Uuid>)> = sqlx::query_as(
        "SELECT v.id, p.nome, p.ficha, v.descricao, v.estilo, v.referencias
           FROM conteudo.variantes v JOIN conteudo.personagens p ON p.id = v.personagem_id
          WHERE p.familia_id = $1 AND p.slug = $2 AND v.slug = $3",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&vslug)
    .fetch_optional(&st.db)
    .await
    .unwrap_or(None);
    let Some((vid, nome, ficha, descricao, estilo, refs_atuais)) = linha else {
        return Redirect::to("/personagens").into_response();
    };
    let fotos: Vec<Uuid> = ficha["fotos"].as_array().into_iter().flatten().filter_map(|v| v.as_str()?.parse().ok()).collect();
    let mut ids = refs_atuais.clone();
    ids.extend(fotos);
    let refs = crate::imagem::carregar_refs(&st, s.familia_id, &ids).await;
    let aparencia = descricao.unwrap_or_else(|| ficha["aparencia"].as_str().unwrap_or_default().to_string());
    let prompt = prompt_model_sheet(&nome, &aparencia, estilo.as_deref(), !refs.is_empty());
    match crate::imagem::gerar(&st, s.familia_id, &s.familia_slug, &format!("personagens/{slug}/refs"), &prompt, &refs).await {
        Ok((img, _)) => {
            let _ = sqlx::query("UPDATE conteudo.variantes SET referencias = array_append(referencias, $2) WHERE id = $1")
                .bind(vid)
                .bind(img)
                .execute(&st.db)
                .await;
            Redirect::to(&format!("/personagens/{slug}")).into_response()
        }
        Err(e) => detalhe_com(&st, s, &slug, Some(format!("{e:#}"))).await,
    }
}

/// POST /personagens/descrever-foto (multipart: fotos, nome, estilo): a IA olha as fotos e devolve
/// a descrição da aparência para preencher o campo. As fotos não são guardadas.
pub async fn descrever_foto(State(st): State<AppState>, s: Sessao, mp: Multipart) -> Response {
    let erro = |m: String| (axum::http::StatusCode::UNPROCESSABLE_ENTITY, axum::Json(serde_json::json!({ "erro": m }))).into_response();
    let (campos, arquivos) = match ler_multipart(mp).await {
        Ok(x) => x,
        Err(m) => return erro(m),
    };
    if arquivos.is_empty() {
        return erro("Escolha pelo menos uma foto.".into());
    }
    let mut imagens = Vec::new();
    for b in arquivos.iter().take(4) {
        match midia::foto_para_ia(b) {
            Some(f) => imagens.push(f),
            None => return erro("Só imagens PNG, JPG ou WebP.".into()),
        }
    }
    let texto = |k: &str| campos.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let (nome, estilo) = (texto("nome"), texto("estilo"));
    let pedido = crate::ia::PedidoTexto {
        sistema: "Você descreve personagens para ilustradores de livros infantis, em português do Brasil. Responda só com o JSON pedido.".into(),
        imagens,
        prompt: format!(
            "Olhe as imagens anexadas{} e descreva a aparência para um ilustrador manter o personagem igual em todas as páginas: \
             idade aparente, cabelo, olhos, pele, corpo, roupa e cores, acessórios e traços marcantes.{} \
             Um parágrafo curto e objetivo (até 70 palavras), sem citar que é uma foto e sem nomes de pessoas reais.",
            if nome.is_empty() { String::new() } else { format!(" (personagem \"{nome}\")") },
            if estilo.is_empty() { String::new() } else { format!(" Estilo desejado: {estilo}.") },
        ),
        schema: serde_json::json!({ "type": "object", "additionalProperties": false, "required": ["aparencia"],
                                    "properties": { "aparencia": { "type": "string" } } }),
    };
    match crate::ia::gerar_texto(&st, s.familia_id, &pedido).await {
        Ok(v) => axum::Json(serde_json::json!({ "aparencia": v["aparencia"] })).into_response(),
        Err(e) => erro(format!("{e:#}")),
    }
}

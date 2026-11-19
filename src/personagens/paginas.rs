//! Lista, ficha e página de detalhe do personagem.


use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::Sessao, util,
    web::{erro_interno, render},
    AppState,
};

use super::{nome_tipo, prompt_model_sheet, TIPOS};

// ───────────── lista ─────────────

#[derive(sqlx::FromRow)]
struct ItemLista {
    slug: String,
    nome: String,
    tipo: String,
    variante: Option<String>,
    miniatura: Option<Uuid>,
    /// Para a busca: @, apelidos, nomes e estilos das variantes, relações.
    busca: String,
    grupos: Vec<String>,
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
    /// Seções da lista: (nome do grupo, índices dos itens). Sem nenhum grupo, uma seção só sem nome.
    secoes: Vec<(String, Vec<usize>)>,
    tipos: Vec<(&'static str, &'static str)>,
    erro: Option<String>,
    /// Grupos de verdade (sem o "Sem grupo"), para a aba 👥 Grupos e o campo de grupos ao criar.
    grupos: Vec<(String, Vec<usize>)>,
}

impl PaginaLista {
    fn item(&self, i: &usize) -> &ItemLista {
        &self.itens[*i]
    }
    fn membro(&self, idx: &[usize], i: &usize) -> bool {
        idx.contains(i)
    }
    fn arroba_grupo(&self, g: &str) -> String {
        format!("@grupo:{}", util::slug(g))
    }
}

pub(super) async fn lista_com(st: &AppState, s: Sessao, erro: Option<String>) -> Response {
    // Miniatura: primeira referência da variante atual (senão a de qualquer variante). Nunca a foto real.
    let itens = sqlx::query_as::<_, ItemLista>(
        "SELECT p.slug, p.nome, p.tipo, va.nome AS variante,
                COALESCE(va.referencias[1],
                         (SELECT v.referencias[1] FROM conteudo.variantes v
                           WHERE v.personagem_id = p.id AND cardinality(v.referencias) > 0
                           ORDER BY v.criado_em LIMIT 1)) AS miniatura
                , concat_ws(' ', '@' || p.slug, p.ficha->>'relacoes',
                    (SELECT string_agg(x, ' ') FROM jsonb_array_elements_text(COALESCE(p.ficha->'apelidos', '[]')) x),
                    (SELECT string_agg(concat_ws(' ', v.nome, v.estilo), ' ') FROM conteudo.variantes v WHERE v.personagem_id = p.id)) AS busca,
                ARRAY(SELECT trim(g) FROM jsonb_array_elements_text(COALESCE(p.ficha->'grupos', '[]')) g WHERE trim(g) <> '') AS grupos
           FROM conteudo.personagens p
           LEFT JOIN conteudo.variantes va ON va.id = p.variante_atual
          WHERE p.familia_id = $1
          ORDER BY p.nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    match itens {
        Ok(itens) => {
            let secoes = secoes(&itens);
            let mut grupos: Vec<(String, Vec<usize>)> = Vec::new();
            for (i, it) in itens.iter().enumerate() {
                for g in &it.grupos {
                    match grupos.iter_mut().find(|(n, _)| util::slug(n) == util::slug(g)) {
                        Some((_, l)) => l.push(i),
                        None => grupos.push((g.clone(), vec![i])),
                    }
                }
            }
            grupos.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
            render(PaginaLista { s, itens, secoes, tipos: TIPOS.to_vec(), erro, grupos })
        }
        Err(e) => erro_interno(e.into()),
    }
}

/// Agrupa a lista: um personagem aparece em cada grupo dele; os sem grupo vão para "Sem grupo" no fim.
fn secoes(itens: &[ItemLista]) -> Vec<(String, Vec<usize>)> {
    if itens.iter().all(|i| i.grupos.is_empty()) {
        return vec![(String::new(), (0..itens.len()).collect())];
    }
    let mut v: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, it) in itens.iter().enumerate() {
        for g in &it.grupos {
            match v.iter_mut().find(|(n, _)| util::slug(n) == util::slug(g)) {
                Some((_, l)) => l.push(i),
                None => v.push((g.clone(), vec![i])),
            }
        }
    }
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    let sem: Vec<usize> = itens.iter().enumerate().filter(|(_, i)| i.grupos.is_empty()).map(|(i, _)| i).collect();
    if !sem.is_empty() {
        v.push(("Sem grupo".into(), sem));
    }
    v
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
    /// Relações com outros personagens, em texto livre (vai para a IA): "irmão mais velho do @pedro".
    #[serde(default)]
    relacoes: String,
    /// Novo @ para citar (só na ficha de um personagem que já existe).
    #[serde(default)]
    arroba: String,
    /// Grupos separados por vírgula ("Família, Pokémon").
    #[serde(default)]
    grupos: String,
}

impl FormFicha {
    fn ficha(&self) -> serde_json::Value {
        let apelidos: Vec<&str> = self.apelidos.split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
        let grupos = super::grupos::do_texto(&self.grupos);
        json!({
            "apelidos": apelidos,
            "grupos": grupos,
            "aparencia": self.aparencia.trim(),
            "personalidade": self.personalidade.trim(),
            "jeito_falar": self.jeito_falar.trim(),
            "relacoes": self.relacoes.trim(),
        })
    }
    fn tipo_valido(&self) -> bool {
        TIPOS.iter().any(|(k, _)| *k == self.tipo)
    }
}

/// POST /personagens (multipart): ficha + foto real opcional (privada, fica na ficha só como referência).
pub async fn criar(State(st): State<AppState>, s: Sessao, mp: axum::extract::Multipart) -> Response {
    let (campos, arquivos) = match super::variantes::ler_multipart(mp).await {
        Ok(x) => x,
        Err(msg) => return lista_com(&st, s, Some(msg)).await,
    };
    let c = |k: &str| campos.get(k).cloned().unwrap_or_default();
    let f = FormFicha {
        nome: c("nome"),
        tipo: c("tipo"),
        apelidos: c("apelidos"),
        aparencia: c("aparencia"),
        personalidade: c("personalidade"),
        jeito_falar: c("jeito_falar"),
        relacoes: c("relacoes"),
        arroba: String::new(),
        grupos: c("grupos"),
    };
    let nome = f.nome.trim().to_string();
    let slug = util::slug(&nome);
    if slug.is_empty() || !f.tipo_valido() {
        return lista_com(&st, s, Some("Preencha o nome e o tipo.".into())).await;
    }
    // Personagem + variante Original (a aparência da ficha; a foto enviada vira referência dela).
    let aparencia = f.aparencia.trim().to_string();
    let r = async {
        let mut tx = st.db.begin().await?;
        let pid: Uuid = sqlx::query_scalar(
            "INSERT INTO conteudo.personagens (familia_id, slug, nome, tipo, ficha) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(s.familia_id)
        .bind(&slug)
        .bind(&nome)
        .bind(&f.tipo)
        .bind(f.ficha())
        .fetch_one(&mut *tx)
        .await?;
        let vid = super::variantes::criar_original(&mut tx, pid, None, None, &aparencia, &[]).await?;
        tx.commit().await?;
        Ok::<_, sqlx::Error>(vid)
    }
    .await;
    match r {
        Ok(vid) => {
            if !arquivos.is_empty() {
                match super::fotos::guardar(&st, &s, &slug, &arquivos).await {
                    Ok(fotos) => {
                        if let Err(e) = sqlx::query("UPDATE conteudo.variantes SET referencias = referencias || $2 WHERE id = $1")
                            .bind(vid)
                            .bind(&fotos)
                            .execute(&st.db)
                            .await
                        {
                            tracing::error!("foto na variante Original: {e:#}");
                        }
                    }
                    Err(msg) => return detalhe_com(&st, s, &slug, Some(msg)).await,
                }
            }
            Redirect::to(&format!("/personagens/{slug}")).into_response()
        }
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
        // `||` mescla: o formulário não traz tudo (ex.: "fotos" do personagem criado com IA) e o resto fica.
        "UPDATE conteudo.personagens SET nome = $3, tipo = $4, ficha = COALESCE(ficha, '{}'::jsonb) || $5, atualizado_em = now()
          WHERE familia_id = $1 AND slug = $2",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(f.nome.trim())
    .bind(&f.tipo)
    .bind(f.ficha())
    .execute(&st.db)
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    // Trocou o @: leva junto histórias, briefings e estantes.
    let novo = util::slug(f.arroba.trim_start_matches('@'));
    if !novo.is_empty() && novo != slug {
        if let Err(e) = super::renomear::renomear_personagem(&st, s.familia_id, &slug, &novo).await {
            return detalhe_com(&st, s, &slug, Some(format!("A ficha foi salva, mas não troquei o @: {e:#}"))).await;
        }
        return Redirect::to(&format!("/personagens/{novo}")).into_response();
    }
    Redirect::to(&format!("/personagens/{slug}")).into_response()
}

// ───────────── detalhe ─────────────

#[derive(sqlx::FromRow)]
pub(super) struct Personagem {
    pub(super) id: Uuid,
    slug: String,
    pub(super) nome: String,
    tipo: String,
    pub(super) ficha: serde_json::Value,
    pub(super) variante_atual: Option<Uuid>,
}

impl Personagem {
    pub(super) fn campo(&self, k: &str) -> String {
        self.ficha.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    }
    fn grupos(&self) -> String {
        crate::historias::grupos_da_ficha(&self.ficha).join(", ")
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
    /// Serviço de tirar o fundo das imagens disponível.
    recorte: bool,
    /// Outros personagens da família (slug, nome), para "virar variante de".
    outros: Vec<(String, String)>,
    /// Grupos que já existem na família (sugestões).
    grupos_familia: Vec<String>,
    /// Desenhos das variantes na fila de imagens (e os que falharam há pouco).
    fila: Vec<crate::fila_imagens::Situacao>,
}

impl PaginaPersonagem {
    /// Fotos reais da ficha (privadas).
    fn fotos(&self) -> Vec<String> {
        super::fotos::fotos(&self.p.ficha).iter().map(Uuid::to_string).collect()
    }
    fn fotos_urls(&self) -> String {
        self.fotos().iter().map(|f| format!("/midia/{f}")).collect::<Vec<_>>().join(" ")
    }
    /// Prompt para fazer o desenho da variante no ChatGPT/Gemini.
    fn prompt_kit(&self, v: &Variante) -> String {
        let aparencia = super::com_ia::aparencia_variante(&self.p.campo("aparencia"), &v.nome, v.descricao.as_deref());
        prompt_model_sheet(&self.p.nome, &aparencia, v.estilo.as_deref(), false)
            + &super::com_ia::nota_referencias(!v.referencias.is_empty(), !self.fotos().is_empty())
    }
    /// Modelo do prompt da variante nova: o navegador troca §NOME§, §ESTILO§ e §MUDA§ pelo que foi digitado.
    fn prompt_modelo(&self) -> String {
        let aparencia = super::com_ia::aparencia_variante(&self.p.campo("aparencia"), "§NOME§", Some("§MUDA§"));
        prompt_model_sheet(&self.p.nome, &aparencia, Some("§ESTILO§"), false) + &super::com_ia::nota_referencias(false, !self.fotos().is_empty())
    }
    /// Desenhos desta variante na fila agora.
    fn na_fila(&self, v: &Variante) -> Vec<&crate::fila_imagens::Situacao> {
        self.fila.iter().filter(|i| i.variante_id == Some(v.id) && i.ativo()).collect()
    }
    /// Último desenho desta variante que não saiu (há pouco).
    fn falha(&self, v: &Variante) -> Option<String> {
        self.fila.iter().rev().find(|i| i.variante_id == Some(v.id) && i.estado == "erro").map(|i| i.rotulo())
    }
    fn eh_atual(&self, v: &Variante) -> bool {
        self.p.variante_atual == Some(v.id)
    }
}

pub(super) async fn buscar(st: &AppState, familia_id: Uuid, slug: &str) -> Result<Option<Personagem>, sqlx::Error> {
    sqlx::query_as::<_, Personagem>(
        "SELECT id, slug, nome, tipo, ficha, variante_atual
           FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2",
    )
    .bind(familia_id)
    .bind(slug)
    .fetch_optional(&st.db)
    .await
}

pub(super) async fn detalhe_com(st: &AppState, s: Sessao, slug: &str, erro: Option<String>) -> Response {
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
            let outros: Vec<(String, String)> =
                sqlx::query_as("SELECT slug, nome FROM conteudo.personagens WHERE familia_id = $1 AND id <> $2 ORDER BY nome")
                    .bind(s.familia_id)
                    .bind(p.id)
                    .fetch_all(&st.db)
                    .await
                    .unwrap_or_default();
            let grupos_familia: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT trim(g) FROM conteudo.personagens p, jsonb_array_elements_text(COALESCE(p.ficha->'grupos', '[]')) g
                  WHERE p.familia_id = $1 AND trim(g) <> '' ORDER BY 1",
            )
            .bind(s.familia_id)
            .fetch_all(&st.db)
            .await
            .unwrap_or_default();
            let fila = crate::fila_imagens::do_personagem(st, s.familia_id, p.id).await;
            render(PaginaPersonagem {
                s, p, variantes, tipos: TIPOS.to_vec(), erro, imagem_api, recorte: crate::recorte::disponivel(), outros, grupos_familia, fila,
            })
        }
        Err(e) => erro_interno(e.into()),
    }
}

pub async fn detalhe(State(st): State<AppState>, s: Sessao, Path(slug): Path<String>) -> Response {
    detalhe_com(&st, s, &slug, None).await
}


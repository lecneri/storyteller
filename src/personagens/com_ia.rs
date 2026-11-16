//! Personagem com IA: ficha a partir de descrição/fotos, desenho de referência e "descrever pela foto".


use axum::{
    extract::{Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    midia, util,
    web::erro_interno,
    AppState,
};

use super::paginas::{detalhe_com, lista_com};
use super::variantes::ler_multipart;
use super::TIPOS;

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
    let mut emprestou = false;
    if ids.is_empty() {
        // Variante nova sem imagem: a imagem de outra variante (a atual, se tiver) vai como referência de rosto e traços.
        let emprestada: Option<Uuid> = sqlx::query_scalar(
            "SELECT v.referencias[1] FROM conteudo.variantes v JOIN conteudo.personagens p ON p.id = v.personagem_id
              WHERE p.familia_id = $1 AND p.slug = $2 AND cardinality(v.referencias) > 0
              ORDER BY (v.id = p.variante_atual) DESC, v.criado_em LIMIT 1",
        )
        .bind(s.familia_id)
        .bind(&slug)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None)
        .flatten();
        emprestou = emprestada.is_some();
        ids.extend(emprestada);
    }
    ids.extend(fotos);
    let refs = crate::imagem::carregar_refs(&st, s.familia_id, &ids).await;
    let aparencia = descricao.unwrap_or_else(|| ficha["aparencia"].as_str().unwrap_or_default().to_string());
    let mut prompt = prompt_model_sheet(&nome, &aparencia, estilo.as_deref(), !refs.is_empty());
    if emprestou {
        prompt.push_str(
            "\nA primeira imagem anexada é o mesmo personagem em outra versão: copie dela só o rosto, o cabelo e os traços. \
             A roupa, a forma e o estilo são os descritos acima, não os da imagem.",
        );
    }
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
    let erro = |m: String| crate::web::erro_json(m);
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


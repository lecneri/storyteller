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

/// Aparência de uma variante para o desenho: a da ficha (o personagem) mais só o que a variante muda.
/// A variante não precisa repetir a ficha; se repetir (variantes antigas), não duplica.
pub fn aparencia_variante(base: &str, variante: &str, descricao: Option<&str>) -> String {
    let (base, desc) = (base.trim(), descricao.unwrap_or_default().trim());
    if desc.is_empty() || desc == base {
        base.to_string()
    } else if base.is_empty() {
        desc.to_string()
    } else {
        format!("{base}\nNesta versão ({variante}), o que muda (vale sobre a descrição acima): {desc}")
    }
}

/// Prompt do "model sheet" (folha de referência) de uma variante.
pub fn prompt_model_sheet(nome: &str, aparencia: &str, estilo: Option<&str>, com_fotos: bool) -> String {
    // O estilo da variante manda no traço: vem destacado e sem o "traço limpo" padrão misturado.
    let estilo = match estilo.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => format!(
            "ESTILO DE DESENHO: {e}. Desenhe exatamente nesse estilo (traço, formato dos olhos e do rosto, proporções, \
             cores e sombreamento típicos de {e}), adaptado para livro infantil."
        ),
        None => "Estilo: ilustração de livro infantil, cores vivas, traço limpo e amigável.".to_string(),
    };
    let fotos = if com_fotos {
        "\nUse as imagens anexadas só como referência de traços (rosto, cabelo, cores, roupa) e transforme em desenho; não faça foto realista."
    } else {
        ""
    };
    format!(
        "Folha de personagem (model sheet) para livro infantil: {nome}. {aparencia}\n\
         {estilo}\n\
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

    // 4) Desenho de referência na fila de imagens (se houver IA de imagem); senão fica o kit manual na página do personagem.
    let destino = format!("/personagens/{slug}");
    if crate::imagem::disponivel(&st, s.familia_id).await {
        let prompt = prompt_model_sheet(&nome, &aparencia, Some(&estilo), !fotos.is_empty());
        let pedido = crate::fila_imagens::Pedido {
            alvo: "variante",
            historia: None,
            chave: None,
            variante: Some(vid),
            lugar: None,
            prompt,
            refs: fotos.clone(),
            pasta: format!("personagens/{slug}/refs"),
            recortar: true,
        };
        if let Err(e) = crate::fila_imagens::pedir(&st, s.familia_id, &s.familia_slug, vec![pedido], crate::fila_imagens::AVULSO).await {
            tracing::warn!("model sheet não entrou na fila: {e:#}");
        }
    }
    Redirect::to(&destino).into_response()
}

#[derive(serde::Deserialize)]
pub struct FormGerar {
    /// "1": manda as fotos reais da ficha como referência de rosto e idade.
    #[serde(default)]
    foto: Option<String>,
}

/// POST /personagens/{slug}/variantes/{vslug}/gerar-ref: pede mais um desenho da variante (vai para a fila).
pub async fn gerar_ref(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, vslug)): Path<(String, String)>,
    axum::extract::Form(f): axum::extract::Form<FormGerar>,
) -> Response {
    match gerar_desenho_variante(&st, &s, &slug, &vslug, f.foto.is_some()).await {
        Ok(()) => super::variantes::ir_para(&slug, &vslug),
        Err(msg) => detalhe_com(&st, s, &slug, Some(msg)).await,
    }
}

/// Pede um desenho da variante à fila de imagens (✨ Gerar e ao criar a variante): aparência da ficha + estilo + o que muda.
/// Vão como referência as imagens que a variante já tem e, se `usar_foto`, as fotos reais da ficha.
pub(super) async fn gerar_desenho_variante(st: &AppState, s: &Sessao, slug: &str, vslug: &str, usar_foto: bool) -> Result<(), String> {
    let linha: Option<(Uuid, String, serde_json::Value, Option<String>, Option<String>, Vec<Uuid>, String)> = sqlx::query_as(
        "SELECT v.id, p.nome, p.ficha, v.descricao, v.estilo, v.referencias, v.nome
           FROM conteudo.variantes v JOIN conteudo.personagens p ON p.id = v.personagem_id
          WHERE p.familia_id = $1 AND p.slug = $2 AND v.slug = $3",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&vslug)
    .fetch_optional(&st.db)
    .await
    .unwrap_or(None);
    let Some((vid, nome, ficha, descricao, estilo, refs_atuais, vnome)) = linha else {
        return Err("Variante não encontrada.".into());
    };
    let fotos = if usar_foto { super::fotos::fotos(&ficha) } else { Vec::new() };
    let mut ids = refs_atuais.clone();
    ids.extend(fotos.iter().copied());
    let aparencia = aparencia_variante(ficha["aparencia"].as_str().unwrap_or_default(), &vnome, descricao.as_deref());
    let mut prompt = prompt_model_sheet(&nome, &aparencia, estilo.as_deref(), false);
    prompt.push_str(&nota_referencias(!refs_atuais.is_empty(), !fotos.is_empty()));
    let pedido = crate::fila_imagens::Pedido {
        alvo: "variante",
        historia: None,
        chave: None,
        variante: Some(vid),
            lugar: None,
        prompt,
        refs: ids,
        pasta: format!("personagens/{slug}/refs"),
        recortar: true,
    };
    crate::fila_imagens::pedir(st, s.familia_id, &s.familia_slug, vec![pedido], crate::fila_imagens::AVULSO)
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

/// O que dizer à IA sobre as imagens anexadas: primeiro as desta variante, depois as fotos reais.
pub fn nota_referencias(com_variante: bool, com_fotos: bool) -> String {
    let mut t = String::new();
    if com_variante {
        t.push_str("\nAs primeiras imagens anexadas já são esta versão do personagem: mantenha o rosto e a roupa delas, mas o traço é sempre o do ESTILO DE DESENHO acima.");
    }
    if com_fotos {
        t.push_str(
            "\nA última imagem anexada é a referência do personagem (foto real ou desenho): use só para o rosto, o cabelo, a pele, \
             a idade e as proporções, e desenhe no ESTILO DE DESENHO pedido. Não copie a roupa nem o fundo dela; se for foto, não faça foto realista.",
        );
    }
    t
}

/// POST /personagens/descrever-foto (multipart: fotos, nome, estilo, personagem): a IA olha as fotos e devolve
/// a descrição da aparência para preencher o campo. As fotos não são guardadas.
/// Com "personagem" (slug), é uma variante: descreve só o que muda em relação à ficha dele.
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
    let base: Option<String> = if texto("personagem").is_empty() {
        None
    } else {
        sqlx::query_scalar("SELECT ficha->>'aparencia' FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2")
            .bind(s.familia_id)
            .bind(texto("personagem"))
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten()
            .flatten()
            .filter(|a: &String| !a.trim().is_empty())
    };
    let pedido_texto = match &base {
        Some(b) => format!(
            "Olhe as imagens anexadas{}. Elas mostram uma versão (variante) de um personagem cuja aparência normal já está descrita assim:\n\"{}\"\n\
             Descreva SÓ o que muda nesta versão em relação à aparência normal: roupa, armadura, acessórios, cores, forma, poderes. \
             Não repita o que é igual (rosto, cabelo, idade, pele) a não ser que tenha mudado.{} \
             Um parágrafo curto e objetivo (até 50 palavras), sem citar que é uma foto e sem nomes de pessoas reais.",
            if nome.is_empty() { String::new() } else { format!(" (\"{nome}\")") },
            b.trim(),
            if estilo.is_empty() { String::new() } else { format!(" Estilo desejado: {estilo}.") },
        ),
        None if !texto("ficha").is_empty() => format!(
            "Olhe as imagens anexadas{} e descreva só a aparência que não muda, para um ilustrador desenhar o personagem igual \
             em qualquer roupa: idade aparente, rosto, cabelo, olhos, pele, corpo e traços marcantes. Não descreva roupa, acessórios nem fundo. \
             Um parágrafo curto e objetivo (até 60 palavras), sem citar que é uma foto e sem nomes de pessoas reais.",
            if nome.is_empty() { String::new() } else { format!(" (personagem \"{nome}\")") },
        ),
        None => format!(
            "Olhe as imagens anexadas{} e descreva a aparência para um ilustrador manter o personagem igual em todas as páginas: \
             idade aparente, cabelo, olhos, pele, corpo, roupa e cores, acessórios e traços marcantes.{} \
             Um parágrafo curto e objetivo (até 70 palavras), sem citar que é uma foto e sem nomes de pessoas reais.",
            if nome.is_empty() { String::new() } else { format!(" (personagem \"{nome}\")") },
            if estilo.is_empty() { String::new() } else { format!(" Estilo desejado: {estilo}.") },
        ),
    };
    let pedido = crate::ia::PedidoTexto {
        sistema: "Você descreve personagens para ilustradores de livros infantis, em português do Brasil. Responda só com o JSON pedido.".into(),
        imagens,
        prompt: pedido_texto,
        schema: serde_json::json!({ "type": "object", "additionalProperties": false, "required": ["aparencia"],
                                    "properties": { "aparencia": { "type": "string" } } }),
    };
    match crate::ia::gerar_texto(&st, s.familia_id, &pedido).await {
        Ok(v) => axum::Json(serde_json::json!({ "aparencia": v["aparencia"] })).into_response(),
        Err(e) => erro(format!("{e:#}")),
    }
}


#[cfg(test)]
mod testes {
    use super::aparencia_variante;

    #[test]
    fn estilo_vai_destacado_no_prompt() {
        let p = super::prompt_model_sheet("Pedro", "Menino de 4 anos", Some("anime do Pokémon"), false);
        assert!(p.contains("ESTILO DE DESENHO: anime do Pokémon."));
        assert!(!p.contains("traço limpo"));
        let sem = super::prompt_model_sheet("Pedro", "Menino de 4 anos", Some("  "), false);
        assert!(sem.contains("cores vivas, traço limpo"));
    }

    #[test]
    fn variante_soma_so_o_que_muda() {
        assert_eq!(aparencia_variante("Menino de 4 anos", "Pirata", None), "Menino de 4 anos");
        assert_eq!(aparencia_variante("Menino de 4 anos", "Pirata", Some(" Menino de 4 anos ")), "Menino de 4 anos");
        assert_eq!(aparencia_variante("", "Pirata", Some("tapa-olho")), "tapa-olho");
        let a = aparencia_variante("Menino de 4 anos", "Pirata", Some("tapa-olho e chapéu"));
        assert!(a.starts_with("Menino de 4 anos\n") && a.contains("(Pirata)") && a.ends_with("tapa-olho e chapéu"));
    }
}

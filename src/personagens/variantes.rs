//! Variantes (forma × estilo): criar, marcar atual, imagens de referência, arquivar.

use std::collections::HashMap;

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

use super::paginas::{buscar, detalhe_com};

// ───────────── variantes ─────────────

/// Lê um formulário multipart: campos de texto + imagens de referência.
pub(super) async fn ler_multipart(mut mp: Multipart) -> Result<(HashMap<String, String>, Vec<Vec<u8>>), String> {
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
pub(super) async fn salvar_refs(st: &AppState, s: &Sessao, p_slug: &str, arquivos: &[Vec<u8>]) -> Result<Vec<Uuid>, String> {
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
    ir_para(&slug, &util::slug(&nome))
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
        Ok(_) => ir_para(&slug, &vslug),
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
        Ok(_) => ir_para(&slug, &vslug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/variantes/{vslug}/usar-foto/{foto}: a foto enviada na criação (ex.: já é um desenho)
/// passa a ser também imagem de referência da variante. Só fotos deste personagem.
pub async fn usar_foto(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, vslug, foto)): Path<(String, String, Uuid)>,
) -> Response {
    let r = sqlx::query(
        "UPDATE conteudo.variantes v SET referencias = array_append(v.referencias, $4)
           FROM conteudo.personagens p
          WHERE v.personagem_id = p.id AND p.familia_id = $1 AND p.slug = $2 AND v.slug = $3
            AND COALESCE(p.ficha->'fotos', '[]') ? $4::text AND NOT ($4 = ANY(v.referencias))",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&vslug)
    .bind(foto)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => ir_para(&slug, &vslug),
        Err(e) => erro_interno(e.into()),
    }
}

/// POST /personagens/{slug}/variantes/{vslug}/tirar-ref/{ref}: tira uma imagem de referência da variante
/// (ex.: um desenho que saiu errado). O arquivo sai do disco na limpeza, se nada mais usar.
pub async fn tirar_ref(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, vslug, ref_id)): Path<(String, String, Uuid)>,
) -> Response {
    let r = sqlx::query(
        "UPDATE conteudo.variantes v SET referencias = array_remove(v.referencias, $4)
           FROM conteudo.personagens p
          WHERE v.personagem_id = p.id AND p.familia_id = $1 AND p.slug = $2 AND v.slug = $3",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&vslug)
    .bind(ref_id)
    .execute(&st.db)
    .await;
    match r {
        Ok(_) => ir_para(&slug, &vslug),
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
        Ok(()) => ir_para(&slug, &vslug),
        Err(e) => erro_interno(e.into()),
    }
}


#[derive(serde::Deserialize)]
pub struct FormEditarVariante {
    nome: String,
    #[serde(default)]
    arroba: String,
    #[serde(default)]
    forma: String,
    #[serde(default)]
    estilo: String,
    #[serde(default)]
    descricao: String,
}

/// POST /personagens/{slug}/variantes/{vslug}/editar: nome, @, forma, estilo e descrição da variante.
pub async fn editar_variante(
    State(st): State<AppState>,
    s: Sessao,
    Path((slug, vslug)): Path<(String, String)>,
    axum::extract::Form(f): axum::extract::Form<FormEditarVariante>,
) -> Response {
    let nome = f.nome.trim();
    if nome.is_empty() {
        return detalhe_com(&st, s, &slug, Some("Dê um nome para a variante.".into())).await;
    }
    let texto = |t: &str| Some(t.trim().to_string()).filter(|t| !t.is_empty());
    let r = sqlx::query(
        "UPDATE conteudo.variantes v SET nome = $4, forma = $5, estilo = $6, descricao = $7
           FROM conteudo.personagens p
          WHERE v.personagem_id = p.id AND p.familia_id = $1 AND p.slug = $2 AND v.slug = $3",
    )
    .bind(s.familia_id)
    .bind(&slug)
    .bind(&vslug)
    .bind(nome)
    .bind(texto(&f.forma))
    .bind(texto(&f.estilo))
    .bind(texto(&f.descricao))
    .execute(&st.db)
    .await;
    match r {
        Ok(r) if r.rows_affected() == 0 => return detalhe_com(&st, s, &slug, Some("Variante não encontrada.".into())).await,
        Ok(_) => {}
        Err(e) => return erro_interno(e.into()),
    }
    let novo = util::slug(f.arroba.trim_start_matches('@').rsplit(':').next().unwrap_or_default());
    if !novo.is_empty() && novo != vslug {
        if let Err(e) = super::renomear::renomear_variante(&st, s.familia_id, &slug, &vslug, &novo).await {
            return detalhe_com(&st, s, &slug, Some(format!("A variante foi salva, mas não troquei o @: {e:#}"))).await;
        }
    }
    ir_para(&slug, if !novo.is_empty() { &novo } else { &vslug })
}

/// Volta para a página do personagem já na variante (âncora #v-slug: rola até ela e, no carrossel, abre ela).
pub(super) fn ir_para(slug: &str, vslug: &str) -> Response {
    Redirect::to(&format!("/personagens/{slug}#v-{vslug}")).into_response()
}

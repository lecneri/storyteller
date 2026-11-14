//! Um personagem vira variante de outro (ex.: "Pedro Pirata", salvo de uma história, vira variante do Pedro).


use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::Sessao, util,
    AppState,
};

use super::paginas::detalhe_com;

// ───────────── virar variante de outro personagem ─────────────

#[derive(Deserialize)]
pub struct FormVirarVariante {
    destino: String,
}

/// POST /personagens/{slug}/virar-variante: o personagem vira variante(s) de outro (ex.: "Pedro Pirata",
/// salvo de uma história, vira variante do Pedro). Move variantes e referências, troca nas histórias
/// (falas e cenas) e nas estantes, leva vozes extras e fotos, e apaga o personagem antigo.
pub async fn virar_variante(
    State(st): State<AppState>,
    s: Sessao,
    Path(slug): Path<String>,
    Form(f): Form<FormVirarVariante>,
) -> Response {
    if f.destino == slug {
        return detalhe_com(&st, s, &slug, Some("Escolha outro personagem.".into())).await;
    }
    match juntar(&st, s.familia_id, &slug, &f.destino).await {
        Ok(()) => Redirect::to(&format!("/personagens/{}", f.destino)).into_response(),
        Err(e) => detalhe_com(&st, s, &slug, Some(format!("Não consegui transformar: {e:#}"))).await,
    }
}

async fn juntar(st: &AppState, familia: Uuid, origem: &str, destino: &str) -> anyhow::Result<()> {
    type Linha = (Uuid, String, serde_json::Value, serde_json::Value, Option<Uuid>);
    let carregar = |slug: &str| {
        sqlx::query_as::<_, Linha>(
            "SELECT id, nome, ficha, vozes_extras, variante_atual FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2",
        )
        .bind(familia)
        .bind(slug.to_string())
        .fetch_optional(&st.db)
    };
    let (oid, onome, oficha, oextras, oatual) = carregar(origem).await?.ok_or_else(|| anyhow::anyhow!("personagem não encontrado"))?;
    let (did, _, dficha, dextras, _) = carregar(destino).await?.ok_or_else(|| anyhow::anyhow!("destino não encontrado"))?;

    let mut tx = st.db.begin().await?;
    // 1) Variantes: mudam de dono, com slug/nome que não colidam ("principal" vira o nome do personagem).
    let variantes: Vec<(Uuid, String, String, Option<String>)> =
        sqlx::query_as("SELECT id, slug, nome, descricao FROM conteudo.variantes WHERE personagem_id = $1 ORDER BY criado_em")
            .bind(oid)
            .fetch_all(&mut *tx)
            .await?;
    let mut existentes: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM conteudo.variantes WHERE personagem_id = $1").bind(did).fetch_all(&mut *tx).await?;
    let aparencia = oficha["aparencia"].as_str().unwrap_or_default().to_string();
    let mut mapa: Vec<(String, String)> = Vec::new(); // (slug antigo, slug novo)
    let mut slug_atual = None;
    for (vid, vslug, vnome, vdesc) in &variantes {
        let generica = vslug == "principal" || variantes.len() == 1;
        let nome = if generica { onome.clone() } else { vnome.clone() };
        let base = if generica { util::slug(&onome) } else { vslug.clone() };
        let mut novo = base.clone();
        let mut n = 2;
        while existentes.contains(&novo) {
            novo = format!("{base}-{n}");
            n += 1;
        }
        existentes.push(novo.clone());
        let desc = vdesc.clone().filter(|d| !d.trim().is_empty()).or_else(|| Some(aparencia.clone()).filter(|a| !a.is_empty()));
        sqlx::query("UPDATE conteudo.variantes SET personagem_id = $2, slug = $3, nome = $4, descricao = $5 WHERE id = $1")
            .bind(vid)
            .bind(did)
            .bind(&novo)
            .bind(&nome)
            .bind(desc)
            .execute(&mut *tx)
            .await?;
        if Some(*vid) == oatual {
            slug_atual = Some(novo.clone());
        }
        mapa.push((vslug.clone(), novo));
    }
    // Sem nenhuma variante: cria uma com a aparência dele.
    if mapa.is_empty() {
        let mut novo = util::slug(&onome);
        if existentes.contains(&novo) {
            novo = format!("{novo}-2");
        }
        sqlx::query("INSERT INTO conteudo.variantes (personagem_id, slug, nome, descricao) VALUES ($1, $2, $3, $4)")
            .bind(did)
            .bind(&novo)
            .bind(&onome)
            .bind(Some(aparencia.clone()).filter(|a| !a.is_empty()))
            .execute(&mut *tx)
            .await?;
        slug_atual = Some(novo);
    }
    let atual = slug_atual.or_else(|| mapa.first().map(|(_, n)| n.clone()));
    let trocar_var = |v: Option<&str>| -> Option<String> {
        match v {
            Some(v) => mapa.iter().find(|(a, _)| a == v).map(|(_, n)| n.clone()).or_else(|| atual.clone()),
            None => atual.clone(),
        }
    };

    // 2) Vozes extras e fotos vão para o destino.
    let mut extras: Vec<serde_json::Value> = dextras.as_array().cloned().unwrap_or_default();
    for e in oextras.as_array().into_iter().flatten() {
        if !extras.iter().any(|x| x["slug"] == e["slug"]) {
            extras.push(e.clone());
        }
    }
    let mut ficha = dficha.clone();
    let mut fotos: Vec<serde_json::Value> = ficha["fotos"].as_array().cloned().unwrap_or_default();
    fotos.extend(oficha["fotos"].as_array().cloned().unwrap_or_default());
    if !fotos.is_empty() {
        ficha["fotos"] = serde_json::Value::Array(fotos);
    }
    sqlx::query("UPDATE conteudo.personagens SET vozes_extras = $2, ficha = $3, atualizado_em = now() WHERE id = $1")
        .bind(did)
        .bind(serde_json::Value::Array(extras))
        .bind(&ficha)
        .execute(&mut *tx)
        .await?;

    // 3) Histórias: falas e cenas passam para o destino (com a variante certa).
    let historias: Vec<(Uuid, serde_json::Value)> =
        sqlx::query_as("SELECT id, conteudo FROM conteudo.historias WHERE familia_id = $1").bind(familia).fetch_all(&mut *tx).await?;
    for (hid, mut c) in historias {
        let mut mudou = false;
        for p in c["paginas"].as_array_mut().into_iter().flatten() {
            for b in p["blocos"].as_array_mut().into_iter().flatten() {
                if b["quem"] == origem {
                    b["quem"] = destino.into();
                    mudou = true;
                }
            }
            if let Some(lista) = p["personagens"].as_array_mut() {
                let ja_tem = lista.iter().any(|n| n["slug"] == destino);
                let mut novos = Vec::new();
                for n in lista.iter() {
                    if n["slug"] == origem {
                        mudou = true;
                        if !ja_tem {
                            novos.push(serde_json::json!({ "slug": destino, "variante": trocar_var(n["variante"].as_str()) }));
                        }
                    } else {
                        novos.push(n.clone());
                    }
                }
                *lista = novos;
            }
        }
        if mudou {
            sqlx::query("UPDATE conteudo.historias SET conteudo = $2, atualizado_em = now() WHERE id = $1").bind(hid).bind(&c).execute(&mut *tx).await?;
        }
    }

    // 4) Estantes: o personagem padrão passa a ser o destino com a variante correspondente.
    let estantes: Vec<(Uuid, serde_json::Value)> =
        sqlx::query_as("SELECT id, padroes FROM conteudo.estantes WHERE familia_id = $1").bind(familia).fetch_all(&mut *tx).await?;
    for (eid, mut padroes) in estantes {
        let Some(lista) = padroes["personagens"].as_array_mut() else { continue };
        if !lista.iter().any(|n| n["slug"] == origem) {
            continue;
        }
        let ja_tem = lista.iter().any(|n| n["slug"] == destino);
        let mut novos = Vec::new();
        for n in lista.iter() {
            if n["slug"] == origem {
                if !ja_tem {
                    novos.push(serde_json::json!({ "slug": destino, "variante": trocar_var(n["variante"].as_str().filter(|v| !v.is_empty())) }));
                }
            } else {
                novos.push(n.clone());
            }
        }
        *lista = novos;
        sqlx::query("UPDATE conteudo.estantes SET padroes = $2 WHERE id = $1").bind(eid).bind(&padroes).execute(&mut *tx).await?;
    }

    // 5) O personagem antigo sai (as variantes já foram para o destino).
    sqlx::query("DELETE FROM partilha.destinos WHERE item_tipo = 'personagem' AND item_id = $1").bind(oid).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM conteudo.personagens WHERE id = $1").bind(oid).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}


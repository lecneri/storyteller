//! Um personagem vira variante de outro (ex.: "Pedro Pirata", salvo de uma história, vira variante do Pedro).
//!
//! As regras (nomes das variantes, troca nas histórias e estantes) são funções puras, testadas abaixo;
//! `juntar` só lê, aplica as regras e grava, numa transação.

use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::paginas::detalhe_com;
use crate::{auth::Sessao, util, AppState};

#[derive(Deserialize)]
pub struct FormVirarVariante {
    destino: String,
}

/// POST /personagens/{slug}/virar-variante: o personagem vira variante(s) de outro. Move variantes e
/// referências, troca nas histórias (falas e cenas) e nas estantes, leva vozes extras e fotos,
/// e apaga o personagem antigo.
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

// ───────────── regras (puras) ─────────────

/// Variante do personagem de origem, como está no banco.
struct VarianteOrigem {
    id: Uuid,
    slug: String,
    nome: String,
    descricao: Option<String>,
}

/// Como cada variante antiga se chama no destino; `atual` é a usada quando a cena não dizia qual.
#[derive(Debug, Default, PartialEq)]
struct MapaVariantes {
    pares: Vec<(String, String)>,
    atual: Option<String>,
}

impl MapaVariantes {
    fn trocar(&self, antiga: Option<&str>) -> Option<String> {
        antiga
            .filter(|v| !v.is_empty())
            .and_then(|v| self.pares.iter().find(|(a, _)| a == v).map(|(_, n)| n.clone()))
            .or_else(|| self.atual.clone())
    }
}

/// O que fazer com as variantes: renomear e mover as existentes, ou criar uma (se não havia nenhuma).
#[derive(Debug, PartialEq)]
struct Plano {
    /// (id, slug novo, nome novo, descrição)
    mover: Vec<(Uuid, String, String, Option<String>)>,
    /// (slug, nome, descrição) da variante criada quando a origem não tinha nenhuma
    criar: Option<(String, String, Option<String>)>,
    mapa: MapaVariantes,
}

/// Slug livre: `base`, `base-2`, `base-3`...
fn slug_livre(base: &str, existentes: &[String]) -> String {
    let mut novo = base.to_string();
    let mut n = 2;
    while existentes.contains(&novo) {
        novo = format!("{base}-{n}");
        n += 1;
    }
    novo
}

/// Variantes "genéricas" (a "Principal", ou a única) ganham o nome do personagem: "Pedro Pirata".
fn planejar(nome_origem: &str, aparencia: &str, variantes: &[VarianteOrigem], atual: Option<Uuid>, existentes: &[String]) -> Plano {
    let mut ocupados = existentes.to_vec();
    let aparencia = Some(aparencia.trim().to_string()).filter(|a| !a.is_empty());
    let mut mover = Vec::new();
    let mut mapa = MapaVariantes::default();
    for v in variantes {
        let generica = v.slug == "principal" || variantes.len() == 1;
        let nome = if generica { nome_origem.to_string() } else { v.nome.clone() };
        let novo = slug_livre(&if generica { util::slug(nome_origem) } else { v.slug.clone() }, &ocupados);
        ocupados.push(novo.clone());
        if Some(v.id) == atual {
            mapa.atual = Some(novo.clone());
        }
        let descricao = v.descricao.clone().filter(|d| !d.trim().is_empty()).or_else(|| aparencia.clone());
        mapa.pares.push((v.slug.clone(), novo.clone()));
        mover.push((v.id, novo, nome, descricao));
    }
    let criar = if variantes.is_empty() {
        let novo = slug_livre(&util::slug(nome_origem), &ocupados);
        mapa.atual = Some(novo.clone());
        Some((novo, nome_origem.to_string(), aparencia))
    } else {
        None
    };
    if mapa.atual.is_none() {
        mapa.atual = mapa.pares.first().map(|(_, n)| n.clone());
    }
    Plano { mover, criar, mapa }
}

/// Lista de [{slug, variante}] (personagens na cena ou padrões da estante): a origem vira o destino.
/// Se o destino já estava na lista, a origem só sai (fica a escolha que já existia para o destino).
fn trocar_na_lista(lista: &mut Vec<Value>, origem: &str, destino: &str, mapa: &MapaVariantes) -> bool {
    if !lista.iter().any(|n| n["slug"] == origem) {
        return false;
    }
    let ja_tem = lista.iter().any(|n| n["slug"] == destino);
    let antiga = std::mem::take(lista);
    for n in antiga {
        if n["slug"] != origem {
            lista.push(n);
        } else if !ja_tem {
            lista.push(json!({ "slug": destino, "variante": mapa.trocar(n["variante"].as_str()) }));
        }
    }
    true
}

/// Conteúdo de uma história: falas (`quem`) e personagens das cenas passam para o destino.
fn trocar_no_conteudo(c: &mut Value, origem: &str, destino: &str, mapa: &MapaVariantes) -> bool {
    let mut mudou = false;
    for p in c["paginas"].as_array_mut().into_iter().flatten() {
        for b in p["blocos"].as_array_mut().into_iter().flatten() {
            if b["quem"] == origem {
                b["quem"] = destino.into();
                mudou = true;
            }
        }
        if let Some(lista) = p["personagens"].as_array_mut() {
            mudou |= trocar_na_lista(lista, origem, destino, mapa);
        }
    }
    mudou
}

/// Vozes extras do destino + as da origem que ele ainda não tem (pelo slug).
fn juntar_vozes_extras(destino: &Value, origem: &Value) -> Value {
    let mut extras: Vec<Value> = destino.as_array().cloned().unwrap_or_default();
    for e in origem.as_array().into_iter().flatten() {
        if !extras.iter().any(|x| x["slug"] == e["slug"]) {
            extras.push(e.clone());
        }
    }
    Value::Array(extras)
}

/// Ficha do destino com as fotos da origem somadas (as fotos só ficam no servidor).
fn juntar_fotos(ficha_destino: &Value, ficha_origem: &Value) -> Value {
    let mut ficha = ficha_destino.clone();
    let mut fotos: Vec<Value> = ficha["fotos"].as_array().cloned().unwrap_or_default();
    fotos.extend(ficha_origem["fotos"].as_array().cloned().unwrap_or_default());
    if !fotos.is_empty() {
        ficha["fotos"] = Value::Array(fotos);
    }
    ficha
}

// ───────────── aplicação no banco ─────────────

async fn juntar(st: &AppState, familia: Uuid, origem: &str, destino: &str) -> anyhow::Result<()> {
    type Linha = (Uuid, String, Value, Value, Option<Uuid>);
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
    let variantes: Vec<VarianteOrigem> = sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
        "SELECT id, slug, nome, descricao FROM conteudo.variantes WHERE personagem_id = $1 ORDER BY criado_em",
    )
    .bind(oid)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(id, slug, nome, descricao)| VarianteOrigem { id, slug, nome, descricao })
    .collect();
    let existentes: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM conteudo.variantes WHERE personagem_id = $1").bind(did).fetch_all(&mut *tx).await?;
    let plano = planejar(&onome, oficha["aparencia"].as_str().unwrap_or_default(), &variantes, oatual, &existentes);

    // 1) Variantes mudam de dono (ou nasce uma, se não havia).
    for (vid, slug, nome, descricao) in &plano.mover {
        sqlx::query("UPDATE conteudo.variantes SET personagem_id = $2, slug = $3, nome = $4, descricao = $5 WHERE id = $1")
            .bind(vid)
            .bind(did)
            .bind(slug)
            .bind(nome)
            .bind(descricao)
            .execute(&mut *tx)
            .await?;
    }
    if let Some((slug, nome, descricao)) = &plano.criar {
        sqlx::query("INSERT INTO conteudo.variantes (personagem_id, slug, nome, descricao) VALUES ($1, $2, $3, $4)")
            .bind(did)
            .bind(slug)
            .bind(nome)
            .bind(descricao)
            .execute(&mut *tx)
            .await?;
    }

    // 2) Vozes extras e fotos vão para o destino.
    sqlx::query("UPDATE conteudo.personagens SET vozes_extras = $2, ficha = $3, atualizado_em = now() WHERE id = $1")
        .bind(did)
        .bind(juntar_vozes_extras(&dextras, &oextras))
        .bind(juntar_fotos(&dficha, &oficha))
        .execute(&mut *tx)
        .await?;

    // 3) Histórias e 4) estantes passam a usar o destino.
    let historias: Vec<(Uuid, Value)> =
        sqlx::query_as("SELECT id, conteudo FROM conteudo.historias WHERE familia_id = $1").bind(familia).fetch_all(&mut *tx).await?;
    for (hid, mut c) in historias {
        if trocar_no_conteudo(&mut c, origem, destino, &plano.mapa) {
            sqlx::query("UPDATE conteudo.historias SET conteudo = $2, atualizado_em = now() WHERE id = $1")
                .bind(hid)
                .bind(&c)
                .execute(&mut *tx)
                .await?;
        }
    }
    let estantes: Vec<(Uuid, Value)> =
        sqlx::query_as("SELECT id, padroes FROM conteudo.estantes WHERE familia_id = $1").bind(familia).fetch_all(&mut *tx).await?;
    for (eid, mut padroes) in estantes {
        let mudou = padroes["personagens"].as_array_mut().is_some_and(|l| trocar_na_lista(l, origem, destino, &plano.mapa));
        if mudou {
            sqlx::query("UPDATE conteudo.estantes SET padroes = $2 WHERE id = $1").bind(eid).bind(&padroes).execute(&mut *tx).await?;
        }
    }

    // 5) O personagem antigo sai (as variantes já foram para o destino).
    sqlx::query("DELETE FROM partilha.destinos WHERE item_tipo = 'personagem' AND item_id = $1").bind(oid).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM conteudo.personagens WHERE id = $1").bind(oid).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;

    fn var(slug: &str, nome: &str, desc: Option<&str>) -> VarianteOrigem {
        VarianteOrigem { id: Uuid::new_v4(), slug: slug.into(), nome: nome.into(), descricao: desc.map(String::from) }
    }

    #[test]
    fn slug_livre_numera_quando_ocupado() {
        let ex = vec!["pirata".to_string(), "pirata-2".to_string()];
        assert_eq!(slug_livre("pirata", &ex), "pirata-3");
        assert_eq!(slug_livre("ninja", &ex), "ninja");
    }

    #[test]
    fn variante_principal_ganha_o_nome_do_personagem() {
        let vs = vec![var("principal", "Principal", None)];
        let p = planejar("Pedro Pirata", "chapéu de pirata", &vs, Some(vs[0].id), &["normal".into()]);
        assert_eq!(p.mover[0].1, "pedro-pirata");
        assert_eq!(p.mover[0].2, "Pedro Pirata");
        // Sem descrição na variante: usa a aparência da ficha.
        assert_eq!(p.mover[0].3.as_deref(), Some("chapéu de pirata"));
        assert_eq!(p.mapa.atual.as_deref(), Some("pedro-pirata"));
        assert!(p.criar.is_none());
    }

    #[test]
    fn varias_variantes_mantem_os_nomes_e_evitam_colisao() {
        let vs = vec![var("normal", "Normal", Some("casaco")), var("gigante", "Gigante", None)];
        let p = planejar("Zorgo", "", &vs, Some(vs[1].id), &["normal".into()]);
        let slugs: Vec<_> = p.mover.iter().map(|m| m.1.as_str()).collect();
        assert_eq!(slugs, vec!["normal-2", "gigante"]);
        assert_eq!(p.mover[1].3, None);
        assert_eq!(p.mapa.trocar(Some("normal")).as_deref(), Some("normal-2"));
        // Variante desconhecida ou vazia → a atual.
        assert_eq!(p.mapa.trocar(Some("sumiu")).as_deref(), Some("gigante"));
        assert_eq!(p.mapa.trocar(Some("")).as_deref(), Some("gigante"));
        assert_eq!(p.mapa.trocar(None).as_deref(), Some("gigante"));
    }

    #[test]
    fn sem_variantes_cria_uma() {
        let p = planejar("Pedro Ninja", "roupa preta", &[], None, &["pedro-ninja".into()]);
        assert!(p.mover.is_empty());
        assert_eq!(p.criar, Some(("pedro-ninja-2".into(), "Pedro Ninja".into(), Some("roupa preta".into()))));
        assert_eq!(p.mapa.trocar(None).as_deref(), Some("pedro-ninja-2"));
    }

    fn mapa() -> MapaVariantes {
        MapaVariantes { pares: vec![("principal".into(), "pedro-pirata".into())], atual: Some("pedro-pirata".into()) }
    }

    #[test]
    fn troca_falas_e_cenas_da_historia() {
        let mut c = json!({ "paginas": [
            { "personagens": [{ "slug": "noob", "variante": null }, { "slug": "pirata", "variante": "principal" }],
              "blocos": [{ "quem": "narrador", "texto": "a" }, { "quem": "pirata", "texto": "Arrr" }] },
            { "personagens": [{ "slug": "noob" }], "blocos": [{ "quem": "noob", "texto": "Au" }] }
        ]});
        assert!(trocar_no_conteudo(&mut c, "pirata", "pedro", &mapa()));
        assert_eq!(c["paginas"][0]["blocos"][1]["quem"], "pedro");
        assert_eq!(c["paginas"][0]["personagens"][1], json!({ "slug": "pedro", "variante": "pedro-pirata" }));
        assert_eq!(c["paginas"][1]["personagens"], json!([{ "slug": "noob" }]));
        // Nada a trocar: não mexe.
        assert!(!trocar_no_conteudo(&mut c, "pirata", "pedro", &mapa()));
    }

    #[test]
    fn destino_ja_na_cena_nao_duplica() {
        let mut lista = vec![json!({ "slug": "pedro", "variante": "normal" }), json!({ "slug": "pirata", "variante": null })];
        assert!(trocar_na_lista(&mut lista, "pirata", "pedro", &mapa()));
        assert_eq!(lista, vec![json!({ "slug": "pedro", "variante": "normal" })]);
    }

    #[test]
    fn junta_vozes_extras_e_fotos_sem_repetir() {
        let v = juntar_vozes_extras(&json!([{ "slug": "ingles" }]), &json!([{ "slug": "ingles" }, { "slug": "espanhol" }]));
        assert_eq!(v, json!([{ "slug": "ingles" }, { "slug": "espanhol" }]));
        assert_eq!(juntar_vozes_extras(&Value::Null, &Value::Null), json!([]));
        let f = juntar_fotos(&json!({ "aparencia": "x", "fotos": ["a"] }), &json!({ "fotos": ["b"] }));
        assert_eq!(f, json!({ "aparencia": "x", "fotos": ["a", "b"] }));
        // Sem fotos dos dois lados: a ficha fica igual (sem "fotos": []).
        assert_eq!(juntar_fotos(&json!({ "aparencia": "x" }), &json!({})), json!({ "aparencia": "x" }));
    }
}

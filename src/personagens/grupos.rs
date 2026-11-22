//! Grupos de personagens ("Família", "Pokémon"): ficam na ficha de cada um (ficha.grupos). A aba 👥 Grupos da lista
//! cria, renomeia, escolhe os membros (cada um com uma variante, ou a atual) e apaga um grupo de uma vez
//! (apagar só tira o grupo; os personagens ficam). A variante de cada grupo fica em ficha.grupos_variantes: {grupo: variante}.

use axum::{
    body::Bytes,
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::{auth::Sessao, util, web::erro_interno, AppState};

/// Grupos de um texto separado por vírgula, sem repetir (comparando sem acento e maiúsculas).
pub fn do_texto(texto: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for g in texto.split(',').map(str::trim).filter(|g| !util::slug(g).is_empty()) {
        if !v.iter().any(|x| util::slug(x) == util::slug(g)) {
            v.push(g.to_string());
        }
    }
    v
}

/// Novos grupos de um personagem: tira o grupo antigo (e o de mesmo nome que o novo) e, se for membro, põe o novo.
fn aplicar(atuais: &[String], antigo: &str, novo: &str, membro: bool) -> Vec<String> {
    let (a, n) = (util::slug(antigo), util::slug(novo));
    let mut v: Vec<String> = atuais.iter().filter(|g| { let s = util::slug(g); s.is_empty() || (s != a && s != n) }).cloned().collect();
    v.retain(|g| !util::slug(g).is_empty());
    if membro && !n.is_empty() {
        v.push(novo.trim().to_string());
    }
    v
}

/// Variantes por grupo depois de salvar: tira a do grupo antigo e a do novo nome; se for membro com variante, põe a nova.
fn aplicar_variante(atual: &Value, antigo: &str, novo: &str, variante: Option<&str>) -> Map<String, Value> {
    let mut m = atual.as_object().cloned().unwrap_or_default();
    m.remove(&util::slug(antigo));
    let n = util::slug(novo);
    m.remove(&n);
    if let Some(v) = variante.map(str::trim).filter(|v| !v.is_empty()) {
        if !n.is_empty() {
            m.insert(n, json!(v));
        }
    }
    m
}

/// Ficha depois que a variante `de` some (arquivada; `para` = None) ou troca de @ (`para` = Some):
/// sumiu, o personagem sai dos grupos em que estava com ela; trocou, os grupos passam a usar o @ novo.
fn ficha_sem_variante(ficha: &Value, de: &str, para: Option<&str>) -> Value {
    let mut f = ficha.clone();
    let vs = ficha["grupos_variantes"].as_object().cloned().unwrap_or_default();
    let afetados: Vec<String> = vs.iter().filter(|(_, v)| v.as_str() == Some(de)).map(|(g, _)| g.clone()).collect();
    if afetados.is_empty() {
        return f;
    }
    let mut novas = vs.clone();
    for g in &afetados {
        match para {
            Some(p) => {
                novas.insert(g.clone(), json!(p));
            }
            None => {
                novas.remove(g);
            }
        }
    }
    f["grupos_variantes"] = Value::Object(novas);
    if para.is_none() {
        let grupos: Vec<Value> = ficha["grupos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|g| !afetados.contains(&util::slug(g.as_str().unwrap_or_default())))
            .cloned()
            .collect();
        f["grupos"] = Value::Array(grupos);
    }
    f
}

/// Aplica `ficha_sem_variante` no personagem (dentro da transação de quem arquiva ou renomeia a variante).
pub async fn variante_mudou(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    personagem: Uuid,
    de: &str,
    para: Option<&str>,
) -> Result<(), sqlx::Error> {
    let ficha: Value = sqlx::query_scalar("SELECT COALESCE(ficha, '{}'::jsonb) FROM conteudo.personagens WHERE id = $1")
        .bind(personagem)
        .fetch_one(&mut **tx)
        .await?;
    let nova = ficha_sem_variante(&ficha, de, para);
    if nova != ficha {
        sqlx::query("UPDATE conteudo.personagens SET ficha = $2 WHERE id = $1").bind(personagem).bind(&nova).execute(&mut **tx).await?;
    }
    Ok(())
}

/// Ficha depois de escolher, na variante `vslug`, os grupos em que o personagem entra com ela: entra nos marcados
/// (com esta variante, no lugar de outra que tivesse) e sai dos que tinham esta variante e foram desmarcados.
fn ficha_com_grupos_da_variante(ficha: &Value, vslug: &str, marcados: &[String]) -> Value {
    let mut f = ficha.clone();
    let mut vs = ficha["grupos_variantes"].as_object().cloned().unwrap_or_default();
    let marc: Vec<String> = marcados.iter().map(|g| util::slug(g)).collect();
    let sair: Vec<String> = vs.iter().filter(|(g, v)| v.as_str() == Some(vslug) && !marc.contains(g)).map(|(g, _)| g.clone()).collect();
    let mut grupos: Vec<String> = ficha["grupos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| g.as_str())
        .filter(|g| !sair.contains(&util::slug(g)))
        .map(str::to_string)
        .collect();
    for g in &sair {
        vs.remove(g);
    }
    for g in marcados {
        let gs = util::slug(g);
        if !grupos.iter().any(|x| util::slug(x) == gs) {
            grupos.push(g.trim().to_string());
        }
        vs.insert(gs, json!(vslug));
    }
    f["grupos"] = json!(grupos);
    f["grupos_variantes"] = Value::Object(vs);
    f
}

/// Aplica `ficha_com_grupos_da_variante` no personagem.
pub async fn variante_nos_grupos(db: &sqlx::PgPool, personagem: Uuid, vslug: &str, marcados: &[String]) -> Result<(), sqlx::Error> {
    let ficha: Value = sqlx::query_scalar("SELECT COALESCE(ficha, '{}'::jsonb) FROM conteudo.personagens WHERE id = $1")
        .bind(personagem)
        .fetch_one(db)
        .await?;
    let nova = ficha_com_grupos_da_variante(&ficha, vslug, marcados);
    if nova != ficha {
        sqlx::query("UPDATE conteudo.personagens SET ficha = $2 WHERE id = $1").bind(personagem).bind(&nova).execute(db).await?;
    }
    Ok(())
}

/// POST /personagens/grupos: cria ou salva um grupo (antigo = nome atual, vazio para novo; nome; membros = slugs)
/// com variante-{slug} = slug da variante (vazio = a atual), ou apaga (acao=apagar).
pub async fn salvar(State(st): State<AppState>, s: Sessao, corpo: Bytes) -> Response {
    let form: Vec<(String, String)> = serde_urlencoded::from_bytes(&corpo).unwrap_or_default();
    let campo = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
    let membros: Vec<&str> = form.iter().filter(|(n, _)| n == "membros").map(|(_, v)| v.as_str()).collect();
    let apagar = campo("acao") == "apagar";
    let (antigo, nome) = (campo("antigo"), if apagar { String::new() } else { campo("nome") });
    if util::slug(&nome).is_empty() && !apagar {
        return Redirect::to("/personagens#grupos").into_response();
    }
    let r = async {
        let mut tx = st.db.begin().await?;
        let todos: Vec<(Uuid, String, Vec<String>, Value)> = sqlx::query_as(
            "SELECT id, slug, ARRAY(SELECT trim(g) FROM jsonb_array_elements_text(COALESCE(ficha->'grupos', '[]')) g),
                    COALESCE(ficha->'grupos_variantes', '{}')
               FROM conteudo.personagens WHERE familia_id = $1",
        )
        .bind(s.familia_id)
        .fetch_all(&mut *tx)
        .await?;
        for (id, slug, atuais, vs) in todos {
            let membro = membros.contains(&slug.as_str());
            let novos = aplicar(&atuais, &antigo, &nome, membro);
            let variante = Some(campo(&format!("variante-{slug}"))).filter(|_| membro);
            let novas_vs = Value::Object(aplicar_variante(&vs, &antigo, &nome, variante.as_deref()));
            if novos != atuais || novas_vs != vs {
                sqlx::query(
                    "UPDATE conteudo.personagens SET ficha = jsonb_set(jsonb_set(ficha, '{grupos}', $2), '{grupos_variantes}', $3) WHERE id = $1",
                )
                .bind(id)
                .bind(json!(novos))
                .bind(&novas_vs)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        anyhow::Ok(())
    }
    .await;
    match r {
        Ok(()) => Redirect::to("/personagens#grupos").into_response(),
        Err(e) => erro_interno(e),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn v(x: &[&str]) -> Vec<String> {
        x.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn grupos_do_texto_sem_repetir() {
        assert_eq!(do_texto(" Família, Pokémon , familia,, !! "), v(&["Família", "Pokémon"]));
    }

    #[test]
    fn renomear_entrar_sair_e_apagar() {
        let g = v(&["Família", "Pokémon"]);
        // Renomeia (continua membro): troca no lugar do antigo, no fim.
        assert_eq!(aplicar(&g, "Pokémon", "Pokémons", true), v(&["Família", "Pokémons"]));
        // Desmarcado: sai do grupo.
        assert_eq!(aplicar(&g, "Pokémon", "Pokémon", false), v(&["Família"]));
        // Grupo novo: entra só quem foi marcado, sem duplicar.
        assert_eq!(aplicar(&g, "", "família", true), v(&["Pokémon", "família"]));
        assert_eq!(aplicar(&g, "", "Escola", false), g);
        // Apagar: sai de todos.
        assert_eq!(aplicar(&g, "Família", "", false), v(&["Pokémon"]));
    }

    #[test]
    fn variante_arquivada_tira_do_grupo_e_renomeada_acompanha() {
        let f = json!({ "grupos": ["Rangers", "Família"], "grupos_variantes": { "rangers": "ranger" }, "aparencia": "x" });
        assert_eq!(
            ficha_sem_variante(&f, "ranger", None),
            json!({ "grupos": ["Família"], "grupos_variantes": {}, "aparencia": "x" })
        );
        assert_eq!(
            ficha_sem_variante(&f, "ranger", Some("power-ranger")),
            json!({ "grupos": ["Rangers", "Família"], "grupos_variantes": { "rangers": "power-ranger" }, "aparencia": "x" })
        );
        // Outra variante: nada muda.
        assert_eq!(ficha_sem_variante(&f, "gigante", None), f);
    }

    #[test]
    fn variante_do_grupo_troca_renomeia_e_sai() {
        let vs = json!({ "rangers": "ranger", "familia": "normal" });
        // Escolhe outra variante.
        assert_eq!(Value::Object(aplicar_variante(&vs, "Rangers", "Rangers", Some("gigante"))), json!({ "rangers": "gigante", "familia": "normal" }));
        // Renomeia mantendo a variante; "atual" (vazio) tira a escolha.
        assert_eq!(Value::Object(aplicar_variante(&vs, "Rangers", "Super Rangers", Some("ranger"))), json!({ "super-rangers": "ranger", "familia": "normal" }));
        assert_eq!(Value::Object(aplicar_variante(&vs, "Rangers", "Rangers", Some(""))), json!({ "familia": "normal" }));
        // Saiu do grupo ou o grupo foi apagado.
        assert_eq!(Value::Object(aplicar_variante(&vs, "Rangers", "", None)), json!({ "familia": "normal" }));
    }

    #[test]
    fn grupos_pela_variante() {
        let f = json!({ "grupos": ["Família", "Rangers"], "grupos_variantes": { "rangers": "normal", "escola": "ranger" } });
        let n = ficha_com_grupos_da_variante(&f, "ranger", &v(&["Rangers", "Heróis"]));
        assert_eq!(n["grupos"], json!(["Família", "Rangers", "Heróis"]));
        assert_eq!(n["grupos_variantes"], json!({ "rangers": "ranger", "herois": "ranger" }));
        // Desmarcar tira do grupo que usava esta variante; os outros ficam.
        let n2 = ficha_com_grupos_da_variante(&n, "ranger", &[]);
        assert_eq!(n2["grupos"], json!(["Família"]));
        assert_eq!(n2["grupos_variantes"], json!({}));
    }
}

//! Grupos de personagens ("Família", "Pokémon"): ficam na ficha de cada um (ficha.grupos). A aba 👥 Grupos da lista
//! cria, renomeia, escolhe os membros e apaga um grupo de uma vez (apagar só tira o grupo; os personagens ficam).

use axum::{
    body::Bytes,
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
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

/// POST /personagens/grupos: cria ou salva um grupo (antigo = nome atual, vazio para novo; nome; membros = slugs)
/// ou apaga (acao=apagar).
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
        let todos: Vec<(Uuid, String, Vec<String>)> = sqlx::query_as(
            "SELECT id, slug, ARRAY(SELECT trim(g) FROM jsonb_array_elements_text(COALESCE(ficha->'grupos', '[]')) g) FROM conteudo.personagens WHERE familia_id = $1",
        )
        .bind(s.familia_id)
        .fetch_all(&mut *tx)
        .await?;
        for (id, slug, atuais) in todos {
            let novos = aplicar(&atuais, &antigo, &nome, membros.contains(&slug.as_str()));
            if novos != atuais {
                sqlx::query("UPDATE conteudo.personagens SET ficha = jsonb_set(ficha, '{grupos}', $2) WHERE id = $1")
                    .bind(id)
                    .bind(serde_json::json!(novos))
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
}

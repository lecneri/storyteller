//! Personagens da família vistos pela IA e detecção de menções no briefing (`@personagem`, `#história`).

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use super::NaCena;
use crate::{util, AppState};

#[derive(sqlx::FromRow, Clone, Serialize)]
pub struct PersonagemIA {
    pub slug: String,
    pub nome: String,
    pub tipo: String,
    pub ficha: Value,
    pub variante_atual: Option<String>,
    /// [{slug, nome, forma, estilo, descricao, referencias}]
    pub variantes: Value,
    /// [{slug, nome, lingua, voz}]
    pub vozes_extras: Value,
}

impl PersonagemIA {
    pub fn tem_voz(&self, v: &str) -> bool {
        self.vozes_extras.as_array().is_some_and(|a| a.iter().any(|x| x["slug"] == v))
    }
}

pub async fn personagens_da_familia(st: &AppState, familia_id: Uuid) -> Result<Vec<PersonagemIA>, sqlx::Error> {
    sqlx::query_as::<_, PersonagemIA>(
        "SELECT p.slug, p.nome, p.tipo, p.ficha, va.slug AS variante_atual, p.vozes_extras,
                COALESCE((SELECT json_agg(json_build_object(
                            'slug', v.slug, 'nome', v.nome, 'forma', v.forma, 'estilo', v.estilo,
                            'descricao', v.descricao, 'referencias', v.referencias, 'arquivada', v.arquivada)
                            ORDER BY v.criado_em)
                            FROM conteudo.variantes v WHERE v.personagem_id = p.id), '[]')::jsonb AS variantes
           FROM conteudo.personagens p
           LEFT JOIN conteudo.variantes va ON va.id = p.variante_atual
          WHERE p.familia_id = $1
          ORDER BY p.nome",
    )
    .bind(familia_id)
    .fetch_all(&st.db)
    .await
}

/// Grupos do personagem (ficha.grupos: ["Família", "Pokémon"]).
pub fn grupos(ficha: &Value) -> Vec<String> {
    ficha["grupos"].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(str::trim)).filter(|g| !g.is_empty()).map(String::from).collect()
}

/// Personagens citados no briefing (`@grupo:familia` cita todos do grupo): `@slug`, `@slug:variante`, ou o nome/apelido escrito normalmente.
pub fn detectar(briefing: &str, todos: &[PersonagemIA]) -> Vec<NaCena> {
    let mut achados: Vec<NaCena> = Vec::new();
    let mut add = |slug: &str, variante: Option<String>, voz: Option<(String, bool)>| {
        if let Some(a) = achados.iter_mut().find(|a| a.slug == slug) {
            match (&a.variante, variante) {
                (None, v) => a.variante = v,
                (Some(atual), Some(v)) if *atual != v && !a.tambem.contains(&v) => a.tambem.push(v),
                _ => {}
            }
            if a.voz.is_none() {
                if let Some((v, m)) = voz {
                    a.voz = Some(v);
                    a.misto = m;
                }
            }
        } else {
            let (voz, misto) = voz.map(|(v, m)| (Some(v), m)).unwrap_or((None, false));
            achados.push(NaCena { slug: slug.to_string(), variante, voz, misto, tambem: Vec::new() });
        }
    };

    // Menções diretas com @
    for parte in briefing.split('@').skip(1) {
        let token: String = parte
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == ':' || *c == '_' || *c == '!')
            .collect();
        // "@pedro:variante!ingles" ou "@pedro!misto-ingles"
        let (token, voz) = match token.split_once('!') {
            Some((t, v)) => (t.to_string(), Some(util::slug(v))),
            None => (token, None),
        };
        let (s, v) = match token.split_once(':') {
            Some((s, v)) => (util::slug(s), Some(util::slug(v))),
            None => (util::slug(&token), None),
        };
        // "@grupo:familia": todos os personagens do grupo (se não houver um personagem chamado "grupo").
        if s == "grupo" && !todos.iter().any(|p| p.slug == "grupo") {
            if let Some(g) = v.filter(|g| !g.is_empty()) {
                for p in todos.iter().filter(|p| grupos(&p.ficha).iter().any(|x| util::slug(x) == g)) {
                    add(&p.slug, None, None);
                }
            }
            continue;
        }
        if let Some(p) = todos.iter().find(|p| p.slug == s) {
            let v = v.filter(|v| {
                p.variantes.as_array().is_some_and(|a| a.iter().any(|x| x["slug"] == *v && x["arquivada"] != true))
            });
            let voz = voz.and_then(|v| match v.strip_prefix("misto-") {
                Some(x) => Some((x.to_string(), true)),
                None => Some((v, false)),
            });
            let voz = voz.filter(|(v, _)| p.tem_voz(v));
            add(&p.slug, v, voz);
        }
    }

    // Menções indiretas: nome ou apelido como palavra inteira
    let texto = format!("-{}-", util::slug(briefing));
    for p in todos {
        let mut nomes = vec![p.nome.clone()];
        if let Some(a) = p.ficha.get("apelidos").and_then(Value::as_array) {
            nomes.extend(a.iter().filter_map(|x| x.as_str().map(String::from)));
        }
        if nomes.iter().map(|n| util::slug(n)).any(|n| !n.is_empty() && texto.contains(&format!("-{n}-"))) {
            add(&p.slug, None, None);
        }
    }
    achados
}

/// Slugs citados com `#` no briefing.
pub fn citar_historias(briefing: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for parte in briefing.split('#').skip(1) {
        let token: String = parte.chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
        let s = util::slug(&token);
        if !s.is_empty() && !v.contains(&s) {
            v.push(s);
        }
    }
    v
}


/// Personagem de teste (com uma voz extra "ingles"), usado pelos testes de detecção e de prompt.
#[cfg(test)]
pub(crate) fn personagem_teste(slug: &str, nome: &str, apelidos: &[&str], variantes: &[&str]) -> PersonagemIA {
    use serde_json::json;
    PersonagemIA {
        slug: slug.into(),
        nome: nome.into(),
        tipo: "crianca".into(),
        ficha: json!({ "apelidos": apelidos }),
        variante_atual: None,
        variantes: json!(variantes.iter().map(|v| json!({ "slug": v })).collect::<Vec<_>>()),
        vozes_extras: json!([{ "slug": "ingles", "nome": "Inglês", "lingua": "Inglês" }]),
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use super::personagem_teste as p;


    #[test]
    fn grupo_cita_todos() {
        let mut a = p("pedro", "Pedro", &[], &[]);
        a.ficha["grupos"] = serde_json::json!(["Família", "Pokémon"]);
        let mut b = p("henrique", "Henrique", &[], &[]);
        b.ficha["grupos"] = serde_json::json!(["Pokémon"]);
        let c = p("noob", "Noob", &[], &[]);
        let todos = vec![a, b, c];
        let slugs = |t: &str| detectar(t, &todos).into_iter().map(|n| n.slug).collect::<Vec<_>>();
        assert_eq!(slugs("Os @grupo:pokemon vão ao parque"), vec!["pedro", "henrique"]);
        assert_eq!(slugs("A @grupo:familia e o @noob"), vec!["pedro", "noob"]);
        assert!(slugs("o @grupo:nada").is_empty());
    }

    #[test]
    fn detecta_mencoes() {
        let todos = vec![
            p("leo", "Léo", &["Super Léo"], &["normal", "super-rangers"]),
            p("tito", "Tito", &[], &[]),
            p("zorgo", "Zorgo", &[], &["gigante"]),
        ];
        let r = detectar("O @leo:super-rangers e o Tito enfrentam o Zorgo!", &todos);
        let slugs: Vec<_> = r.iter().map(|n| (n.slug.as_str(), n.variante.as_deref())).collect();
        assert_eq!(slugs, vec![("leo", Some("super-rangers")), ("tito", None), ("zorgo", None)]);
        // Mesmo personagem com duas variantes (transformação)
        let r = detectar("O @leo:normal vira o @leo:super-rangers", &todos);
        assert_eq!((r.len(), r[0].variante.as_deref(), r[0].tambem.clone()), (1, Some("normal"), vec!["super-rangers".to_string()]));
        // "Leonel" não é "Léo"
        assert!(detectar("A dona Leonel", &todos).is_empty());
        // Voz extra: "!ingles" (só inglês) e "!misto-ingles"; voz que não existe é ignorada.
        let r = detectar("@leo:normal!misto-ingles e @tito!ingles e @zorgo!frances", &todos);
        let vozes: Vec<_> = r.iter().map(|n| (n.slug.as_str(), n.variante.as_deref(), n.voz.as_deref(), n.misto)).collect();
        assert_eq!(vozes, vec![("leo", Some("normal"), Some("ingles"), true), ("tito", None, Some("ingles"), false), ("zorgo", None, None, false)]);
    }

    #[test]
    fn cita_historias_com_hashtag() {
        assert_eq!(citar_historias("Continua #a-lua e #o-mar, e de novo #a-lua!"), vec!["a-lua", "o-mar"]);
        assert!(citar_historias("sem citação # solta").is_empty());
    }
}

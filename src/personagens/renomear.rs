//! Trocar o @ de um personagem (ex.: "super-pedro" → "pedro") ou de uma variante, levando junto tudo o que
//! usa esse @: falas e cenas das histórias, briefings ("@super-pedro:ranger"), estantes e suas instruções.
//!
//! As trocas são funções puras (testadas abaixo); `renomear_*` aplica no banco numa transação.

use serde_json::Value;
use uuid::Uuid;

use crate::{util, AppState};

/// O que trocar: o @ do personagem, ou o @ de uma variante dele.
#[derive(Clone, Copy)]
pub enum Troca<'a> {
    Personagem { de: &'a str, para: &'a str },
    Variante { personagem: &'a str, de: &'a str, para: &'a str },
}

/// Troca as menções num texto livre: "@super-pedro", "@super-pedro:ranger!ingles"...
pub fn trocar_mencoes(texto: &str, troca: Troca) -> String {
    let mut out = String::with_capacity(texto.len());
    let mut partes = texto.split('@');
    out.push_str(partes.next().unwrap_or_default());
    for parte in partes {
        out.push('@');
        let fim = parte
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '-' | ':' | '_' | '!')))
            .map(|(i, _)| i)
            .unwrap_or(parte.len());
        let (token, resto) = parte.split_at(fim);
        let (base, voz) = match token.split_once('!') {
            Some((b, v)) => (b, Some(v)),
            None => (token, None),
        };
        let (p, v) = match base.split_once(':') {
            Some((p, v)) => (p, Some(v)),
            None => (base, None),
        };
        let (p_novo, v_novo) = match troca {
            Troca::Personagem { de, para } if util::slug(p) == de => (para.to_string(), v.map(String::from)),
            Troca::Variante { personagem, de, para } if util::slug(p) == personagem && v.is_some_and(|v| util::slug(v) == de) => {
                (p.to_string(), Some(para.to_string()))
            }
            _ => (p.to_string(), v.map(String::from)),
        };
        out.push_str(&p_novo);
        if let Some(v) = v_novo {
            out.push(':');
            out.push_str(&v);
        }
        if let Some(voz) = voz {
            out.push('!');
            out.push_str(voz);
        }
        out.push_str(resto);
    }
    out
}

/// Lista [{slug, variante}] (personagens na cena ou padrões da estante).
pub fn trocar_na_lista(lista: &mut [Value], troca: Troca) -> bool {
    let mut mudou = false;
    for n in lista.iter_mut() {
        match troca {
            Troca::Personagem { de, para } if n["slug"] == de => {
                n["slug"] = para.into();
                mudou = true;
            }
            Troca::Variante { personagem, de, para } if n["slug"] == personagem && n["variante"] == de => {
                n["variante"] = para.into();
                mudou = true;
            }
            _ => {}
        }
    }
    mudou
}

/// Conteúdo de uma história: falas (`quem`) e personagens das cenas.
pub fn trocar_no_conteudo(c: &mut Value, troca: Troca) -> bool {
    let mut mudou = false;
    for p in c["paginas"].as_array_mut().into_iter().flatten() {
        if let Troca::Personagem { de, para } = troca {
            for b in p["blocos"].as_array_mut().into_iter().flatten() {
                if b["quem"] == de {
                    b["quem"] = para.into();
                    mudou = true;
                }
            }
        }
        if let Some(lista) = p["personagens"].as_array_mut() {
            mudou |= trocar_na_lista(lista, troca);
        }
    }
    mudou
}

/// Aplica a troca em todas as histórias e estantes da família (dentro da transação).
async fn aplicar(tx: &mut sqlx::PgConnection, familia: Uuid, troca: Troca<'_>) -> anyhow::Result<()> {
    let historias: Vec<(Uuid, Value, Option<String>)> =
        sqlx::query_as("SELECT id, conteudo, briefing FROM conteudo.historias WHERE familia_id = $1").bind(familia).fetch_all(&mut *tx).await?;
    for (id, mut c, briefing) in historias {
        let mudou = trocar_no_conteudo(&mut c, troca);
        let novo_briefing = briefing.as_deref().map(|b| trocar_mencoes(b, troca));
        if mudou || novo_briefing != briefing {
            sqlx::query("UPDATE conteudo.historias SET conteudo = $2, briefing = $3, versao = versao + 1 WHERE id = $1")
                .bind(id)
                .bind(&c)
                .bind(&novo_briefing)
                .execute(&mut *tx)
                .await?;
        }
    }
    // Relações escritas nas fichas ("irmão do @super-pedro").
    let fichas: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, ficha->>'relacoes' FROM conteudo.personagens WHERE familia_id = $1 AND COALESCE(ficha->>'relacoes', '') LIKE '%@%'",
    )
    .bind(familia)
    .fetch_all(&mut *tx)
    .await?;
    for (id, rel) in fichas {
        let nova = trocar_mencoes(&rel, troca);
        if nova != rel {
            sqlx::query("UPDATE conteudo.personagens SET ficha = jsonb_set(ficha, '{relacoes}', to_jsonb($2::text)) WHERE id = $1")
                .bind(id)
                .bind(&nova)
                .execute(&mut *tx)
                .await?;
        }
    }
    let estantes: Vec<(Uuid, Value)> =
        sqlx::query_as("SELECT id, padroes FROM conteudo.estantes WHERE familia_id = $1").bind(familia).fetch_all(&mut *tx).await?;
    for (id, mut padroes) in estantes {
        let mut mudou = padroes["personagens"].as_array_mut().is_some_and(|l| trocar_na_lista(l, troca));
        if let Some(i) = padroes["instrucoes"].as_str() {
            let nova = trocar_mencoes(i, troca);
            if nova != i {
                padroes["instrucoes"] = nova.into();
                mudou = true;
            }
        }
        if mudou {
            sqlx::query("UPDATE conteudo.estantes SET padroes = $2 WHERE id = $1").bind(id).bind(&padroes).execute(&mut *tx).await?;
        }
    }
    Ok(())
}

/// Troca o @ do personagem. Erro legível se o novo @ já for de outro personagem.
pub async fn renomear_personagem(st: &AppState, familia: Uuid, de: &str, para: &str) -> anyhow::Result<()> {
    let mut tx = st.db.begin().await?;
    let ocupado: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2)")
        .bind(familia)
        .bind(para)
        .fetch_one(&mut *tx)
        .await?;
    if ocupado {
        anyhow::bail!("já existe um personagem com @{para}");
    }
    let n = sqlx::query("UPDATE conteudo.personagens SET slug = $3, atualizado_em = now() WHERE familia_id = $1 AND slug = $2")
        .bind(familia)
        .bind(de)
        .bind(para)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n != 1 {
        anyhow::bail!("personagem não encontrado");
    }
    aplicar(&mut tx, familia, Troca::Personagem { de, para }).await?;
    tx.commit().await?;
    Ok(())
}

/// Troca o @ de uma variante. Erro legível se o personagem já tiver outra variante com esse @.
pub async fn renomear_variante(st: &AppState, familia: Uuid, personagem: &str, de: &str, para: &str) -> anyhow::Result<()> {
    let mut tx = st.db.begin().await?;
    let pid: Uuid = sqlx::query_scalar("SELECT id FROM conteudo.personagens WHERE familia_id = $1 AND slug = $2")
        .bind(familia)
        .bind(personagem)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("personagem não encontrado"))?;
    let ocupado: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conteudo.variantes WHERE personagem_id = $1 AND slug = $2)")
        .bind(pid)
        .bind(para)
        .fetch_one(&mut *tx)
        .await?;
    if ocupado {
        anyhow::bail!("{personagem} já tem uma variante com @{personagem}:{para}");
    }
    sqlx::query("UPDATE conteudo.variantes SET slug = $3 WHERE personagem_id = $1 AND slug = $2")
        .bind(pid)
        .bind(de)
        .bind(para)
        .execute(&mut *tx)
        .await?;
    aplicar(&mut tx, familia, Troca::Variante { personagem, de, para }).await?;
    super::grupos::variante_mudou(&mut tx, pid, de, Some(para)).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;
    use serde_json::json;

    const P: Troca = Troca::Personagem { de: "super-pedro", para: "pedro" };
    const V: Troca = Troca::Variante { personagem: "pedro", de: "super-pedro-ranger-vermelho", para: "super-pedro" };

    #[test]
    fn troca_mencoes_no_texto() {
        assert_eq!(trocar_mencoes("O @super-pedro e o @noob brincam.", P), "O @pedro e o @noob brincam.");
        assert_eq!(
            trocar_mencoes("@super-pedro:super-pedro-ranger-vermelho!ingles vira", P),
            "@pedro:super-pedro-ranger-vermelho!ingles vira"
        );
        // Só o @ inteiro: "@super-pedroca" é outro.
        assert_eq!(trocar_mencoes("@super-pedroca e e-mail a@b", P), "@super-pedroca e e-mail a@b");
        assert_eq!(trocar_mencoes("@pedro:super-pedro-ranger-vermelho, @pedro e @pedro:normal", V), "@pedro:super-pedro, @pedro e @pedro:normal");
        assert_eq!(trocar_mencoes("sem menção", P), "sem menção");
    }

    #[test]
    fn troca_falas_e_cenas() {
        let mut c = json!({ "paginas": [
            { "personagens": [{ "slug": "super-pedro", "variante": "super-pedro-ranger-vermelho" }, { "slug": "noob", "variante": null }],
              "blocos": [{ "quem": "super-pedro", "texto": "Oi" }, { "quem": "narrador", "texto": "a" }] }
        ]});
        assert!(trocar_no_conteudo(&mut c, P));
        assert_eq!(c["paginas"][0]["blocos"][0]["quem"], "pedro");
        assert_eq!(c["paginas"][0]["personagens"][0]["slug"], "pedro");
        assert!(trocar_no_conteudo(&mut c, V));
        assert_eq!(c["paginas"][0]["personagens"][0]["variante"], "super-pedro");
        // Nada a trocar: não mexe.
        assert!(!trocar_no_conteudo(&mut c, P));
        assert!(!trocar_no_conteudo(&mut c, V));
    }
}

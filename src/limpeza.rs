//! Limpeza de arquivos: apaga mídias e áudios que nada mais usa (histórias apagadas, imagens trocadas,
//! trechos editados). Roda uma vez por dia e logo depois de apagar uma história.

use std::{collections::HashSet, time::Duration};

use serde_json::Value;
use uuid::Uuid;

use crate::{historias::Conteudo, tts, AppState};

/// Arquivos mais novos que isto nunca são apagados (um envio pode ainda não ter sido salvo na página).
const CARENCIA_HORAS: i32 = 24;

/// Hashes de áudio ainda úteis para uma família: todos os trechos das histórias dela com as vozes
/// atuais da família, e com as vozes genéricas só nas histórias que têm link público ativo.
async fn audios_em_uso(st: &AppState, familia: Uuid) -> anyhow::Result<HashSet<String>> {
    let vozes = tts::vozes_da_familia(st, familia).await?;
    let historias: Vec<(bool, String, Value)> = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM partilha.links l
                         WHERE NOT l.revogado AND NOT l.vozes_familia AND (l.expira_em IS NULL OR l.expira_em > now())
                           AND ((l.item_tipo = 'historia' AND l.item_id = h.id)
                             OR (l.item_tipo = 'estante' AND h.visibilidade <> 'privado' AND EXISTS (
                                   SELECT 1 FROM conteudo.estante_historias eh WHERE eh.estante_id = l.item_id AND eh.historia_id = h.id)))),
                h.narracao, h.conteudo
           FROM conteudo.historias h WHERE h.familia_id = $1",
    )
            .bind(familia)
            .fetch_all(&st.db)
            .await?;
    let tipos: Vec<(String, String)> = sqlx::query_as("SELECT slug, tipo FROM conteudo.personagens WHERE familia_id = $1")
        .bind(familia)
        .fetch_all(&st.db)
        .await?;
    let mut usados = HashSet::new();
    for (publica, narracao, conteudo) in historias {
        let c: Conteudo = serde_json::from_value(conteudo).unwrap_or_default();
        for b in c.paginas.iter().flat_map(|p| p.blocos.iter()) {
            let texto = b.texto.trim();
            if texto.is_empty() {
                continue;
            }
            let quem = if narracao == "narrador" { "narrador".to_string() } else { b.chave_voz() };
            let (base, _) = tts::separar(&quem);
            let tipo = tipos.iter().find(|(s, _)| s == base).map(|(_, t)| t.as_str()).unwrap_or("narrador");
            usados.insert(tts::hash_publico(&vozes.de(&quem), texto));
            if publica {
                usados.insert(tts::hash_publico(&tts::voz_generica(&quem, tipo), texto));
            }
        }
    }
    Ok(usados)
}

/// Faz a limpeza completa. Devolve (arquivos apagados, bytes liberados).
pub async fn limpar(st: &AppState) -> anyhow::Result<(u64, i64)> {
    // 1) Áudios de narração que nenhum trecho atual usa.
    let familias: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM acesso.familias").fetch_all(&st.db).await?;
    for f in familias {
        let usados: Vec<String> = audios_em_uso(st, f).await?.into_iter().collect();
        sqlx::query(
            "DELETE FROM midia.audios WHERE familia_id = $1 AND NOT (hash = ANY($2))
               AND criado_em < now() - make_interval(hours => $3)",
        )
        .bind(f)
        .bind(&usados)
        .bind(CARENCIA_HORAS)
        .execute(&st.db)
        .await?;
    }

    // 2) Arquivos sem nenhuma referência (páginas, capas, referências de variantes, fotos, áudios).
    let orfaos: Vec<(Uuid, String, i64)> = sqlx::query_as(
        "WITH usados AS (
            SELECT (pg->'midia'->>'id')::uuid AS id FROM conteudo.historias h, jsonb_array_elements(h.conteudo->'paginas') pg
             WHERE pg->'midia'->>'id' IS NOT NULL
            UNION SELECT capa FROM conteudo.historias WHERE capa IS NOT NULL
            UNION SELECT musica FROM conteudo.historias WHERE musica IS NOT NULL
            UNION SELECT (b->'gravacao'->>'id')::uuid FROM conteudo.historias h, jsonb_array_elements(h.conteudo->'paginas') pg,
                         jsonb_array_elements(pg->'blocos') b WHERE b->'gravacao'->>'id' IS NOT NULL
            UNION SELECT capa FROM conteudo.estantes WHERE capa IS NOT NULL
            UNION SELECT unnest(referencias) FROM conteudo.variantes
            UNION SELECT (f)::uuid FROM conteudo.personagens p, jsonb_array_elements_text(COALESCE(p.ficha->'fotos', '[]')) f
            UNION SELECT (ficha->>'imagem')::uuid FROM conteudo.personagens WHERE ficha->>'imagem' IS NOT NULL
            UNION SELECT arquivo FROM midia.audios
         )
         SELECT a.id, a.caminho, a.bytes FROM midia.arquivos a
          WHERE a.id NOT IN (SELECT id FROM usados WHERE id IS NOT NULL)
            AND a.criado_em < now() - make_interval(hours => $1)",
    )
    .bind(CARENCIA_HORAS)
    .fetch_all(&st.db)
    .await?;

    let mut apagados = 0u64;
    // Vídeos exportados com mais de 24 h (e temporários esquecidos).
    let mut liberado = crate::video::limpar_antigos(st).await as i64;
    for (id, caminho, bytes) in orfaos {
        sqlx::query("DELETE FROM midia.arquivos WHERE id = $1").bind(id).execute(&st.db).await?;
        // O mesmo arquivo pode estar em outra família (cópia de história): só apaga do disco se ninguém mais usa.
        let ainda_usado: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM midia.arquivos WHERE caminho = $1)
                OR EXISTS (SELECT 1 FROM midia.musicas_modelo WHERE caminho = $1)")
            .bind(&caminho)
            .fetch_one(&st.db)
            .await?;
        if !ainda_usado && tokio::fs::remove_file(st.data_dir.join(&caminho)).await.is_ok() {
            apagados += 1;
            liberado += bytes;
        }
    }
    if apagados > 0 {
        tracing::info!("limpeza: {apagados} arquivo(s) apagado(s), {} KB liberados", liberado / 1024);
    }
    Ok((apagados, liberado))
}

/// Worker: confere a cada hora e limpa se a última limpeza tem mais de 24 h. A data fica no banco
/// (LIMPEZA_ULTIMA), então reiniciar o servidor (deploy) não atrasa a limpeza.
pub async fn agendar(st: AppState) {
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        let agora = chrono::Utc::now().timestamp();
        let ultima: i64 = crate::config::get("LIMPEZA_ULTIMA").and_then(|v| v.parse().ok()).unwrap_or(0);
        if agora - ultima >= 24 * 3600 {
            registrar(&st, limpar(&st).await).await;
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

/// Guarda quando foi a última limpeza e o resultado (mostrado em ⚙️ Sistema).
pub async fn registrar(st: &AppState, r: anyhow::Result<(u64, i64)>) -> String {
    let texto = match &r {
        Ok((n, b)) => format!("{n} arquivo(s) apagado(s), {:.1} MB liberados", *b as f64 / 1048576.0),
        Err(e) => format!("falhou: {e:#}"),
    };
    if let Err(e) = &r {
        tracing::warn!("limpeza falhou: {e:#}");
    }
    let quando = chrono::Utc::now();
    let _ = crate::config::salvar(st, "LIMPEZA_ULTIMA", &quando.timestamp().to_string()).await;
    let _ = crate::config::salvar(st, "LIMPEZA_RESULTADO", &format!("{} — {texto}", quando.with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).unwrap()).format("%d/%m %H:%M"))).await;
    texto
}

/// Apaga uma família inteira: banco (em cascata) e arquivos do disco que nenhuma outra família usa.
pub async fn apagar_familia(st: &AppState, familia: Uuid) -> anyhow::Result<()> {
    let slug: String = sqlx::query_scalar("SELECT slug FROM acesso.familias WHERE id = $1").bind(familia).fetch_one(&st.db).await?;
    let caminhos: Vec<String> = sqlx::query_scalar("SELECT DISTINCT caminho FROM midia.arquivos WHERE familia_id = $1")
        .bind(familia)
        .fetch_all(&st.db)
        .await?;
    let mut tx = st.db.begin().await?;
    // Destinos e links apontam para a família sem cascata: saem antes.
    sqlx::query("DELETE FROM partilha.destinos WHERE familia_id = $1").bind(familia).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM partilha.links WHERE familia_id = $1").bind(familia).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM acesso.familias WHERE id = $1").bind(familia).execute(&mut *tx).await?;
    tx.commit().await?;
    // Arquivos: só sai do disco o que nenhuma cópia em outra família ainda usa.
    for c in caminhos {
        let usado: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM midia.arquivos WHERE caminho = $1)
                OR EXISTS (SELECT 1 FROM midia.musicas_modelo WHERE caminho = $1)")
            .bind(&c)
            .fetch_one(&st.db)
            .await?;
        if !usado {
            let _ = tokio::fs::remove_file(st.data_dir.join(&c)).await;
        }
    }
    remover_pastas_vazias(&st.data_dir.join("familias").join(&slug));
    tracing::info!("família {slug} apagada");
    Ok(())
}

fn remover_pastas_vazias(dir: &std::path::Path) -> bool {
    let Ok(entradas) = std::fs::read_dir(dir) else { return false };
    let mut vazia = true;
    for e in entradas.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !remover_pastas_vazias(&p) {
                vazia = false;
            }
        } else {
            vazia = false;
        }
    }
    vazia && std::fs::remove_dir(dir).is_ok()
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn remove_so_pastas_vazias() {
        let raiz = std::env::temp_dir().join(format!("st-teste-{}", Uuid::new_v4()));
        std::fs::create_dir_all(raiz.join("a/b/c")).unwrap();
        std::fs::create_dir_all(raiz.join("d")).unwrap();
        std::fs::write(raiz.join("d/arquivo.png"), b"x").unwrap();
        // "d" tem arquivo: a raiz não pode sumir, mas a/b/c (vazias) somem.
        assert!(!remover_pastas_vazias(&raiz));
        assert!(!raiz.join("a").exists());
        assert!(raiz.join("d/arquivo.png").exists());
        std::fs::remove_dir_all(&raiz).unwrap();
    }
}

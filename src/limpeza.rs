//! Limpeza de arquivos: apaga mídias e áudios que nada mais usa (histórias apagadas, imagens trocadas,
//! trechos editados). Roda uma vez por dia e logo depois de apagar uma história.

use std::{collections::HashSet, time::Duration};

use serde_json::Value;
use uuid::Uuid;

use crate::{historias::Conteudo, tts, AppState};

/// Arquivos mais novos que isto nunca são apagados (um envio pode ainda não ter sido salvo na página).
const CARENCIA_HORAS: i32 = 24;

/// Hashes de áudio ainda úteis para uma família: todos os trechos das histórias que ela pode tocar,
/// com as vozes atuais dela e com as vozes genéricas (links públicos).
async fn audios_em_uso(st: &AppState, familia: Uuid) -> anyhow::Result<HashSet<String>> {
    let vozes = tts::vozes_da_familia(st, familia).await?;
    let historias: Vec<(Uuid, String, Value)> =
        sqlx::query_as("SELECT familia_id, narracao, conteudo FROM conteudo.historias WHERE familia_id = $1")
            .bind(familia)
            .fetch_all(&st.db)
            .await?;
    let tipos: Vec<(String, String)> = sqlx::query_as("SELECT slug, tipo FROM conteudo.personagens WHERE familia_id = $1")
        .bind(familia)
        .fetch_all(&st.db)
        .await?;
    let mut usados = HashSet::new();
    for (_, narracao, conteudo) in historias {
        let c: Conteudo = serde_json::from_value(conteudo).unwrap_or_default();
        for b in c.paginas.iter().flat_map(|p| p.blocos.iter()) {
            let texto = b.texto.trim();
            if texto.is_empty() {
                continue;
            }
            let quem = if narracao == "narrador" { "narrador".to_string() } else { b.chave_voz() };
            let (base, extra) = tts::separar(&quem);
            let tipo = tipos.iter().find(|(s, _)| s == base).map(|(_, t)| t.as_str()).unwrap_or("narrador");
            usados.insert(tts::hash_publico(&vozes.de(&quem), texto));
            let generica = if quem == "narrador" {
                tts::narrador_padrao()
            } else if extra.is_some() {
                tts::generica_extra(tipo)
            } else {
                tts::voz_por_tipo(tipo)
            };
            usados.insert(tts::hash_publico(&generica, texto));
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
            UNION SELECT capa FROM conteudo.estantes WHERE capa IS NOT NULL
            UNION SELECT unnest(referencias) FROM conteudo.variantes
            UNION SELECT (f)::uuid FROM conteudo.personagens p, jsonb_array_elements_text(COALESCE(p.ficha->'fotos', '[]')) f
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
    let mut liberado = 0i64;
    for (id, caminho, bytes) in orfaos {
        sqlx::query("DELETE FROM midia.arquivos WHERE id = $1").bind(id).execute(&st.db).await?;
        // O mesmo arquivo pode estar em outra família (cópia de história): só apaga do disco se ninguém mais usa.
        let ainda_usado: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM midia.arquivos WHERE caminho = $1)")
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

/// Roda a limpeza uma vez por dia (a primeira, 10 minutos depois de subir).
pub async fn agendar(st: AppState) {
    tokio::time::sleep(Duration::from_secs(600)).await;
    loop {
        if let Err(e) = limpar(&st).await {
            tracing::warn!("limpeza falhou: {e:#}");
        }
        tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
    }
}

/// Apaga uma família inteira: banco (em cascata) e arquivos do disco que nenhuma outra família usa.
pub async fn apagar_familia(st: &AppState, familia: Uuid) -> anyhow::Result<()> {
    let slug: String = sqlx::query_scalar("SELECT slug FROM acesso.familias WHERE id = $1").bind(familia).fetch_one(&st.db).await?;
    let caminhos: Vec<String> = sqlx::query_scalar("SELECT DISTINCT caminho FROM midia.arquivos WHERE familia_id = $1")
        .bind(familia)
        .fetch_all(&st.db)
        .await?;
    let mut tx = st.db.begin().await?;
    for tabela in ["partilha.destinos", "partilha.links"] {
        let col = if tabela == "partilha.links" { "familia_id" } else { "familia_id" };
        sqlx::query(&format!("DELETE FROM {tabela} WHERE {col} = $1")).bind(familia).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM acesso.familias WHERE id = $1").bind(familia).execute(&mut *tx).await?;
    tx.commit().await?;
    // Arquivos: só sai do disco o que nenhuma cópia em outra família ainda usa.
    for c in caminhos {
        let usado: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM midia.arquivos WHERE caminho = $1)")
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

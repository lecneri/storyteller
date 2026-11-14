//! Exportar a família em Markdown (zip com a estrutura de pastas do PLANO.md, seção 2.0).

use std::io::Write;

use axum::{
    body::Body,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use serde_json::Value;
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::{auth::Sessao, historias::Conteudo, web::erro_interno, AppState};

fn yaml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// historia.md: frontmatter + "# Página N" com cena/animação + blocos ("[slug] — fala" ou parágrafo do narrador).
pub fn historia_md(titulo: &str, faixa: &str, narracao: &str, briefing: Option<&str>, c: &Conteudo, midia: &dyn Fn(&Uuid) -> String) -> String {
    let mut s = format!(
        "---\ntitulo: {}\nfaixa: {faixa}\nnarracao: {narracao}\nlayout: livro\n{}---\n",
        yaml_str(titulo),
        briefing.map(|b| format!("briefing: {}\n", yaml_str(b))).unwrap_or_default()
    );
    for (i, p) in c.paginas.iter().enumerate() {
        s.push_str(&format!("\n# Página {}\n", i + 1));
        if !p.cena.is_empty() {
            s.push_str(&format!("cena: {}\n", p.cena.replace('\n', " ")));
        }
        if !p.personagens.is_empty() {
            let lista: Vec<String> = p
                .personagens
                .iter()
                .map(|n| match &n.variante {
                    Some(v) => format!("{}:{v}", n.slug),
                    None => n.slug.clone(),
                })
                .collect();
            s.push_str(&format!("personagens_na_cena: [{}]\n", lista.join(", ")));
        }
        if let Some(a) = p.animacao.as_deref().filter(|a| *a != "nenhuma") {
            s.push_str(&format!("animacao: {a}\n"));
        }
        if let Some(m) = &p.midia {
            s.push_str(&format!("midia: {}\n", midia(&m.id)));
        }
        for b in &p.blocos {
            if b.quem == "narrador" {
                s.push_str(&format!("\n{}\n", b.texto.trim()));
            } else {
                s.push_str(&format!("\n[{}] — {}\n", b.chave_voz(), b.texto.trim()));
            }
        }
    }
    s
}

fn ext_de(mime: &str) -> &str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "audio/mpeg" => "mp3",
        _ => "bin",
    }
}

/// GET /exportar: baixa um .zip com tudo da família em Markdown + mídias (sem áudios gerados e sem fotos privadas).
pub async fn exportar(State(st): State<AppState>, s: Sessao) -> Response {
    let arquivos = match arquivos_familia(&st, s.familia_id, &s.familia_slug).await {
        Ok(a) => a,
        Err(e) => return erro_interno(e),
    };
    match zipar(arquivos) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/zip".to_string()),
                (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.zip\"", s.familia_slug)),
            ],
            Body::from(bytes),
        )
            .into_response(),
        Err(e) => erro_interno(e),
    }
}

/// Todos os arquivos (Markdown + mídias) de uma família, com o caminho dentro do zip.
pub async fn arquivos_familia(st: &AppState, familia_id: Uuid, familia_slug: &str) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let base = format!("familias/{familia_slug}");
    let mut arquivos: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |caminho: String, conteudo: Vec<u8>| arquivos.push((caminho, conteudo));

    // Mídia da família, para copiar os arquivos referenciados.
    let midias: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, caminho, tipo_mime FROM midia.arquivos WHERE familia_id = $1")
            .bind(familia_id)
            .fetch_all(&st.db)
            .await?;
    let ler = |id: &Uuid| midias.iter().find(|(m, _, _)| m == id).cloned();

    // familia.md
    let (nome, vis, config): (String, String, Value) =
        sqlx::query_as("SELECT nome, visibilidade_padrao, config FROM acesso.familias WHERE id = $1")
            .bind(familia_id)
            .fetch_one(&st.db)
            .await?;
    add(
        format!("{base}/familia.md"),
        format!(
            "---\nid: {}\nnome: {}\nvisibilidade_padrao: {vis}\nnarrador: {}\n---\n",
            familia_slug,
            yaml_str(&nome),
            config.get("narrador").map(|v| v.to_string()).unwrap_or_else(|| "padrão".into())
        )
        .into_bytes(),
    );

    // leitores
    let leitores: Vec<(String, String, String)> =
        sqlx::query_as("SELECT slug, nome, faixa FROM acesso.leitores WHERE familia_id = $1")
            .bind(familia_id)
            .fetch_all(&st.db)
            .await?;
    for (slug, nome, faixa) in leitores {
        add(format!("{base}/leitores/{slug}.md"), format!("---\nid: {slug}\nnome: {}\nfaixa: {faixa}\n---\n", yaml_str(&nome)).into_bytes());
    }

    // personagens + variantes + refs
    let personagens: Vec<(Uuid, String, String, String, Value, Value)> = sqlx::query_as(
        "SELECT id, slug, nome, tipo, ficha, voz_padrao FROM conteudo.personagens WHERE familia_id = $1",
    )
    .bind(familia_id)
    .fetch_all(&st.db)
    .await?;
    let mut pendentes: Vec<(String, String)> = Vec::new(); // (caminho no zip, caminho no disco)
    for (pid, slug, nome, tipo, ficha, voz) in &personagens {
        let campo = |k: &str| ficha[k].as_str().unwrap_or_default().to_string();
        add(
            format!("{base}/personagens/{slug}/main.md"),
            format!(
                "---\nid: {slug}\nnome: {}\ntipo: {tipo}\napelidos: {}\nvoz: {}\naparencia: {}\n---\n\n## Personalidade\n{}\n\n## Jeito de falar\n{}\n",
                yaml_str(nome),
                ficha["apelidos"],
                voz,
                yaml_str(&campo("aparencia")),
                campo("personalidade"),
                campo("jeito_falar")
            )
            .into_bytes(),
        );
        let variantes: Vec<(String, String, Option<String>, Option<String>, Option<String>, Vec<Uuid>, bool)> = sqlx::query_as(
            "SELECT slug, nome, forma, estilo, descricao, referencias, arquivada FROM conteudo.variantes WHERE personagem_id = $1",
        )
        .bind(pid)
        .fetch_all(&st.db)
        .await?;
        for (vslug, vnome, forma, estilo, desc, refs, arquivada) in variantes {
            let mut lista = Vec::new();
            for (i, r) in refs.iter().enumerate() {
                if let Some((_, caminho, mime)) = ler(r) {
                    let destino = format!("refs/{vslug}-{}.{}", i + 1, ext_de(&mime));
                    pendentes.push((format!("{base}/personagens/{slug}/{destino}"), caminho));
                    lista.push(destino);
                }
            }
            add(
                format!("{base}/personagens/{slug}/{vslug}.md"),
                format!(
                    "---\nid: {vslug}\nnome: {}\nforma: {}\nestilo: {}\narquivada: {arquivada}\nreferencias: [{}]\n---\n\n{}\n",
                    yaml_str(&vnome),
                    forma.unwrap_or_default(),
                    yaml_str(&estilo.unwrap_or_default()),
                    lista.join(", "),
                    desc.unwrap_or_default()
                )
                .into_bytes(),
            );
        }
    }

    // estantes + histórias (história fica na estante principal; sem estante → pasta "sem-estante")
    let estantes: Vec<(Uuid, String, String, Value)> =
        sqlx::query_as("SELECT id, slug, nome, padroes FROM conteudo.estantes WHERE familia_id = $1")
            .bind(familia_id)
            .fetch_all(&st.db)
            .await?;
    let historias: Vec<(Uuid, String, String, String, String, Option<String>, Value, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, slug, titulo, faixa, narracao, briefing, conteudo, memoria, estante_principal
           FROM conteudo.historias WHERE familia_id = $1",
    )
    .bind(familia_id)
    .fetch_all(&st.db)
    .await?;
    for (eid, eslug, enome, padroes) in &estantes {
        let membros: Vec<String> = historias.iter().filter(|h| h.8 == Some(*eid)).map(|h| h.1.clone()).collect();
        add(
            format!("{base}/estantes/{eslug}/estante.md"),
            format!("---\nid: {eslug}\nnome: {}\npadroes: {}\nhistorias: [{}]\n---\n", yaml_str(enome), padroes, membros.join(", ")).into_bytes(),
        );
    }
    for (_, hslug, titulo, faixa, narracao, briefing, conteudo, memoria, principal) in &historias {
        let pasta_estante = principal
            .and_then(|p| estantes.iter().find(|e| e.0 == p))
            .map(|e| e.1.clone())
            .unwrap_or_else(|| "sem-estante".into());
        let dir = format!("{base}/estantes/{pasta_estante}/{hslug}");
        let c: Conteudo = serde_json::from_value(conteudo.clone()).unwrap_or_default();
        let mut nomes_midia: Vec<(Uuid, String)> = Vec::new();
        for (i, p) in c.paginas.iter().enumerate() {
            if let Some(m) = &p.midia {
                if let Some((_, caminho, mime)) = ler(&m.id) {
                    let nome = format!("midia/p{:02}.{}", i + 1, ext_de(&mime));
                    pendentes.push((format!("{dir}/{nome}"), caminho));
                    nomes_midia.push((m.id, nome));
                }
            }
        }
        let md = historia_md(titulo, faixa, narracao, briefing.as_deref(), &c, &|id| {
            nomes_midia.iter().find(|(m, _)| m == id).map(|(_, n)| n.clone()).unwrap_or_default()
        });
        add(format!("{dir}/historia.md"), md.into_bytes());
        if let Some(m) = memoria {
            add(format!("{dir}/memoria.md"), m.clone().into_bytes());
        }
    }

    for (destino, caminho) in pendentes {
        if let Ok(b) = tokio::fs::read(st.data_dir.join(caminho)).await {
            arquivos.push((destino, b));
        }
    }

    Ok(arquivos)
}

/// Monta o zip em memória (o acervo de uma família é pequeno).
pub fn zipar(arquivos: Vec<(String, Vec<u8>)>) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (caminho, conteudo) in arquivos {
            z.start_file(caminho, opts)?;
            z.write_all(&conteudo)?;
        }
        z.finish()?;
    }
    Ok(buf.into_inner())
}

#[cfg(test)]
mod testes {
    use super::*;
    use serde_json::json;

    #[test]
    fn historia_em_markdown() {
        let id = Uuid::new_v4();
        let c: Conteudo = serde_json::from_value(json!({ "paginas": [{
            "cena": "Navio\npirata", "personagens": [{ "slug": "pedro", "variante": "pirata" }, { "slug": "noob" }],
            "animacao": "chuva", "midia": { "id": id, "tipo": "image/png" },
            "blocos": [{ "quem": "narrador", "texto": " Era uma vez. " }, { "quem": "pedro", "voz": "ingles", "texto": "Ahoy!" }]
        }]})).unwrap();
        let md = historia_md("O \"Pirata\"", "4-6", "por-personagem", Some("briefing"), &c, &|_| "midia/p01.png".into());
        assert!(md.starts_with("---\ntitulo: \"O \\\"Pirata\\\"\"\n"));
        assert!(md.contains("briefing: \"briefing\"\n"));
        assert!(md.contains("# Página 1\ncena: Navio pirata\n"));
        assert!(md.contains("personagens_na_cena: [pedro:pirata, noob]\n"));
        assert!(md.contains("animacao: chuva\nmidia: midia/p01.png\n"));
        assert!(md.contains("\nEra uma vez.\n"));
        assert!(md.contains("[pedro+ingles] — Ahoy!"));
    }

    #[test]
    fn zip_abre_com_os_arquivos() {
        let bytes = zipar(vec![("a/b.md".into(), b"oi".to_vec()), ("c.txt".into(), b"tchau".to_vec())]).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(z.len(), 2);
        let mut s = String::new();
        std::io::Read::read_to_string(&mut z.by_name("a/b.md").unwrap(), &mut s).unwrap();
        assert_eq!(s, "oi");
    }
}

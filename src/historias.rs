//! Histórias: criação (briefing → IA, ou manual), fila de geração e editor de blocos.

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::Sessao,
    ia, jobs, midia, util,
    web::{erro_interno, render},
    AppState,
};

pub const ANIMACOES: [&str; 10] = [
    "nenhuma", "estrelas-piscando", "chuva", "neve", "brilhos", "folhas", "bolhas", "tremor", "onomatopeia-pop",
    "explosao",
];
const FAIXAS: [&str; 4] = ["2-3", "4-6", "7-9", "10-12"];

// ───────────── modelo do conteúdo (páginas e blocos) ─────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Conteudo {
    #[serde(default)]
    pub paginas: Vec<Pagina>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Pagina {
    #[serde(default)]
    pub cena: String,
    #[serde(default)]
    pub personagens: Vec<NaCena>,
    #[serde(default)]
    pub animacao: Option<String>,
    #[serde(default)]
    pub blocos: Vec<Bloco>,
    /// Imagem, GIF ou vídeo da página (arquivo em midia.arquivos).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midia: Option<MidiaPagina>,
    /// Som da animação ligado (padrão: sim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub som: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MidiaPagina {
    pub id: Uuid,
    /// tipo MIME: image/png, image/gif, video/mp4...
    pub tipo: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NaCena {
    pub slug: String,
    #[serde(default)]
    pub variante: Option<String>,
    /// Voz extra pedida no briefing (`@pedro!ingles`) e se mistura com português (`@pedro!misto-ingles`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voz: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub misto: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bloco {
    /// "narrador" ou o slug do personagem.
    pub quem: String,
    pub texto: String,
    /// Voz extra do personagem usada neste trecho (slug, ex.: "ingles").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voz: Option<String>,
}

impl Bloco {
    /// Chave da voz do trecho: "pedro" ou "pedro+ingles".
    pub fn chave_voz(&self) -> String {
        match &self.voz {
            Some(v) if self.quem != "narrador" && !v.is_empty() => format!("{}+{v}", self.quem),
            _ => self.quem.clone(),
        }
    }
}

/// Limites de cada faixa etária: (páginas mín, páginas máx, palavras mín por página, palavras máx).
fn regras_faixa(faixa: &str) -> (u32, u32, u32, u32) {
    match faixa {
        "2-3" => (6, 8, 15, 30),
        "7-9" => (10, 12, 60, 110),
        "10-12" => (12, 14, 100, 160),
        _ => (8, 10, 30, 60),
    }
}

// ───────────── personagens disponíveis e detecção no briefing ─────────────

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
    fn tem_voz(&self, v: &str) -> bool {
        self.vozes_extras.as_array().is_some_and(|a| a.iter().any(|x| x["slug"] == v))
    }
}

async fn personagens_da_familia(st: &AppState, familia_id: Uuid) -> Result<Vec<PersonagemIA>, sqlx::Error> {
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

/// Personagens citados no briefing: `@slug`, `@slug:variante`, ou o nome/apelido escrito normalmente.
pub fn detectar(briefing: &str, todos: &[PersonagemIA]) -> Vec<NaCena> {
    let mut achados: Vec<NaCena> = Vec::new();
    let mut add = |slug: &str, variante: Option<String>, voz: Option<(String, bool)>| {
        if let Some(a) = achados.iter_mut().find(|a| a.slug == slug) {
            if a.variante.is_none() {
                a.variante = variante;
            }
            if a.voz.is_none() {
                if let Some((v, m)) = voz {
                    a.voz = Some(v);
                    a.misto = m;
                }
            }
        } else {
            let (voz, misto) = voz.map(|(v, m)| (Some(v), m)).unwrap_or((None, false));
            achados.push(NaCena { slug: slug.to_string(), variante, voz, misto });
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

// ───────────── prompt ─────────────

fn schema_historia() -> Value {
    json!({
        "type": "object",
                    "additionalProperties": false,
        "required": ["titulo", "paginas"],
        "properties": {
            "titulo": { "type": "string" },
            "paginas": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["cena", "personagens", "animacao", "blocos"],
                    "properties": {
                        "cena": { "type": "string", "description": "Descrição visual da ilustração da página (para o gerador de imagem)" },
                        "personagens": {
                            "type": "array",
                            "items": {
                                "type": "object",
                    "additionalProperties": false,
                                "required": ["slug", "variante"],
                                "properties": { "slug": { "type": "string" }, "variante": { "type": "string" } }
                            }
                        },
                        "animacao": { "type": "string", "enum": ANIMACOES },
                        "blocos": {
                            "type": "array",
                            "items": {
                                "type": "object",
                    "additionalProperties": false,
                                "required": ["quem", "texto", "voz"],
                                "properties": { "quem": { "type": "string" }, "texto": { "type": "string" },
                                                "voz": { "type": "string", "description": "\"\" (voz normal) ou o slug da voz extra do personagem" } }
                            }
                        }
                    }
                }
            }
        }
    })
}

const SISTEMA: &str = "Você é um autor de histórias infantis em português do Brasil. \
Escreve livros ilustrados: cada página tem uma ilustração e um texto curto. \
Regras fixas: conteúdo adequado para crianças; nada de violência gráfica, medo excessivo, linguagem imprópria ou temas adultos; \
vilões são engraçados e atrapalhados, nunca cruéis; lutas são divertidas, sem ferimentos; finais positivos. \
Respeite a ficha de cada personagem (aparência, personalidade, jeito de falar). \
Responda apenas com o JSON pedido.";

fn montar_prompt(
    briefing: &str,
    faixa: &str,
    narracao: &str,
    citados: &[NaCena],
    todos: &[PersonagemIA],
    estilo: &str,
    memorias: &[(String, String)],
) -> String {
    let (pmin, pmax, wmin, wmax) = regras_faixa(faixa);
    let fichas: Vec<Value> = citados
        .iter()
        .filter_map(|c| todos.iter().find(|p| p.slug == c.slug).map(|p| (c, p)))
        .map(|(c, p)| {
            let var_slug = c.variante.clone().or_else(|| p.variante_atual.clone());
            let variante = p
                .variantes
                .as_array()
                .and_then(|a| a.iter().find(|v| Some(v["slug"].as_str().unwrap_or_default().to_string()) == var_slug))
                .cloned();
            // Voz extra pedida no briefing: como o personagem fala nesta história.
            let extra = c.voz.as_ref().and_then(|v| {
                p.vozes_extras.as_array()?.iter().find(|x| x["slug"] == *v).cloned()
            });
            let falas = extra.map(|x| {
                let (vs, lingua) = (x["slug"].as_str().unwrap_or_default(), x["lingua"].as_str().unwrap_or("Português"));
                let em = if lingua == "Português" { String::new() } else { format!(", escritas em {lingua} (frases curtas e palavras simples, para uma criança que está aprendendo)") };
                if c.misto {
                    format!("{} MISTURA as línguas: algumas falas curtas usam voz = \"{vs}\"{em}; logo depois, em outro bloco com voz = \"\", ele mesmo repete ou explica em português, para a criança aprender.", p.nome)
                } else {
                    format!("TODAS as falas de {} usam voz = \"{vs}\"{em}. O narrador pode contar em português o que ele disse.", p.nome)
                }
            });
            json!({
                "slug": p.slug, "nome": p.nome, "tipo": p.tipo,
                "ficha": p.ficha,
                "falas": falas,
                "variante_a_usar": variante.map(|v| json!({
                    "slug": v["slug"], "nome": v["nome"], "forma": v["forma"], "estilo": v["estilo"], "descricao": v["descricao"]
                })),
                "outras_variantes": p.variantes.as_array().map(|a| {
                    a.iter().filter(|v| v["arquivada"] != true).map(|v| v["slug"].clone()).collect::<Vec<_>>()
                }),
            })
        })
        .collect();

    let modo = if narracao == "narrador" {
        "Narração SÓ COM NARRADOR: cada página tem exatamente UM bloco, com quem = \"narrador\". As falas dos personagens entram no texto do narrador com travessão (— Vamos lá! — disse Léo.)."
    } else {
        "Narração POR PERSONAGEM: cada página tem vários blocos em sequência. Trechos narrados usam quem = \"narrador\"; cada fala usa quem = slug do personagem que fala, com o texto só da fala (sem \"disse fulano\"). Alterne narrador e falas de forma natural. Em cada bloco, voz = \"\", salvo quando o campo \"falas\" do personagem pedir outra."
    };
    let briefing = if briefing.trim().is_empty() {
        "(sem briefing: invente uma história divertida com os personagens disponíveis)".to_string()
    } else {
        briefing.trim().to_string()
    };

    let estilo = if estilo.trim().is_empty() {
        String::new()
    } else {
        format!("- Universo/estilo desta coleção: {}. Mantenha o clima desse universo no texto e nas cenas.\n", estilo.trim())
    };
    let anteriores = if memorias.is_empty() {
        String::new()
    } else {
        let lista: Vec<String> = memorias.iter().map(|(t, m)| format!("- \"{t}\": {m}")).collect();
        format!(
            "HISTÓRIAS ANTERIORES (esta história continua a partir delas; respeite o que aconteceu e pode citar):\n{}\n\n",
            lista.join("\n")
        )
    };
    format!(
        "Escreva uma história para a faixa de {faixa} anos.\n{estilo}\
         - Entre {pmin} e {pmax} páginas; cada página com {wmin} a {wmax} palavras no total.\n\
         - {modo}\n\
         - Em cada página: \"cena\" descreve a ilustração (lugar, ação, expressões, enquadramento), sem texto escrito na imagem; \
           \"personagens\" lista quem aparece na ilustração com slug e variante (use a variante_a_usar, salvo se o briefing pedir outra das outras_variantes); \
           \"animacao\" é um efeito leve que combine com a cena, ou \"nenhuma\".\n\
         - Use SOMENTE os personagens abaixo (pelos slugs). Personagens secundários sem ficha podem aparecer só na narração.\n\n\
         PERSONAGENS:\n{}\n\n\
         {anteriores}\
         BRIEFING DA FAMÍLIA:\n{briefing}\n",
        serde_json::to_string_pretty(&fichas).unwrap_or_default()
    )
}

/// Ajusta a resposta da IA às regras do app (quem inválido vira narrador, modo narrador = 1 bloco).
fn normalizar(mut c: Conteudo, narracao: &str, citados: &[NaCena], todos: &[PersonagemIA]) -> Conteudo {
    for p in &mut c.paginas {
        p.personagens.retain(|n| citados.iter().any(|c| c.slug == n.slug));
        for n in &mut p.personagens {
            if n.variante.as_deref().is_some_and(|v| v.trim().is_empty()) {
                n.variante = None;
            }
        }
        if p.animacao.as_deref().is_some_and(|a| !ANIMACOES.contains(&a)) {
            p.animacao = Some("nenhuma".into());
        }
        for b in &mut p.blocos {
            if b.quem != "narrador" && !citados.iter().any(|c| c.slug == b.quem) {
                b.quem = "narrador".into();
            }
            b.texto = b.texto.trim().to_string();
            // Só vale voz extra que o personagem tem.
            let valida = b.voz.as_deref().is_some_and(|v| todos.iter().any(|p| p.slug == b.quem && p.tem_voz(v)));
            if !valida {
                b.voz = None;
            }
        }
        p.blocos.retain(|b| !b.texto.is_empty());
        if narracao == "narrador" && p.blocos.len() > 1 {
            let texto = p.blocos.iter().map(|b| b.texto.as_str()).collect::<Vec<_>>().join(" ");
            p.blocos = vec![Bloco { quem: "narrador".into(), texto, voz: None }];
        }
    }
    c.paginas.retain(|p| !p.blocos.is_empty());
    c
}

// ───────────── job: gerar história ─────────────

#[derive(sqlx::FromRow)]
struct HistoriaGerar {
    familia_id: Uuid,
    briefing: Option<String>,
    faixa: String,
    narracao: String,
    padroes: Option<Value>,
}

pub async fn executar_geracao(st: &AppState, payload: &Value) -> anyhow::Result<()> {
    let id: Uuid = payload["historia_id"].as_str().unwrap_or_default().parse()?;
    let h = sqlx::query_as::<_, HistoriaGerar>(
        "SELECT h.familia_id, h.briefing, h.faixa, h.narracao, e.padroes
           FROM conteudo.historias h
           LEFT JOIN conteudo.estantes e ON e.id = h.estante_principal
          WHERE h.id = $1",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;

    let todos = personagens_da_familia(st, h.familia_id).await?;
    let briefing = h.briefing.unwrap_or_default();
    let mut citados = detectar(&briefing, &todos);
    // Padrões da estante: seus personagens entram sempre, com a variante da estante
    // (a menos que o briefing peça outra com @slug:variante).
    let padroes = h.padroes.unwrap_or(Value::Null);
    for p in padroes["personagens"].as_array().into_iter().flatten() {
        let slug = p["slug"].as_str().unwrap_or_default();
        let variante = p["variante"].as_str().filter(|v| !v.is_empty()).map(String::from);
        if !todos.iter().any(|t| t.slug == slug) {
            continue;
        }
        match citados.iter_mut().find(|c| c.slug == slug) {
            Some(c) if c.variante.is_none() => c.variante = variante,
            Some(_) => {}
            None => citados.push(NaCena { slug: slug.into(), variante, voz: None, misto: false }),
        }
    }
    if citados.is_empty() {
        // Sem ninguém citado nem estante: a IA pode usar todos os personagens da família.
        citados = todos.iter().map(|p| NaCena { slug: p.slug.clone(), variante: None, voz: None, misto: false }).collect();
    }
    let estilo = padroes["estilo"].as_str().unwrap_or_default();

    // Histórias citadas com #: entram pela memória (resumo), não pelo texto inteiro.
    let mut memorias = Vec::new();
    let mut continua_de = Vec::new();
    for slug in citar_historias(&briefing) {
        let achada: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
            "SELECT id, titulo, memoria FROM conteudo.historias WHERE familia_id = $1 AND slug = $2 AND id <> $3",
        )
        .bind(h.familia_id)
        .bind(&slug)
        .bind(id)
        .fetch_optional(&st.db)
        .await?;
        if let Some((hid, titulo, memoria)) = achada {
            let memoria = match memoria.filter(|m| !m.trim().is_empty()) {
                Some(m) => m,
                None => gerar_memoria(st, hid).await?,
            };
            memorias.push((titulo, memoria));
            continua_de.push(hid);
        }
    }

    let pedido = ia::PedidoTexto {
        sistema: SISTEMA.into(),
        imagens: Vec::new(),
        prompt: montar_prompt(&briefing, &h.faixa, &h.narracao, &citados, &todos, estilo, &memorias),
        schema: schema_historia(),
    };
    let resposta = ia::gerar_texto(st, h.familia_id, &pedido).await?;
    let titulo = resposta["titulo"].as_str().unwrap_or("Nova história").trim().to_string();
    let conteudo: Conteudo = serde_json::from_value(json!({ "paginas": resposta["paginas"] }))?;
    let conteudo = normalizar(conteudo, &h.narracao, &citados, &todos);
    if conteudo.paginas.is_empty() {
        anyhow::bail!("a IA devolveu uma história vazia");
    }

    let slug = slug_livre(st, h.familia_id, &titulo, id).await?;
    let valor = serde_json::to_value(&conteudo)?;
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE conteudo.historias SET titulo = $2, slug = $3, conteudo = $4, estado = 'texto', continua_de = $5,
                atualizado_em = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(&titulo)
    .bind(&slug)
    .bind(&valor)
    .bind(&continua_de)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO conteudo.versoes (historia_id, conteudo) VALUES ($1, $2)")
        .bind(id)
        .bind(&valor)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Avisa por e-mail quem pediu (se quiser receber).
    if let Ok(Some((email, nome))) = sqlx::query_as::<_, (String, String)>(
        "SELECT u.email, u.nome FROM conteudo.historias h JOIN acesso.usuarios u ON u.id = h.criado_por
          WHERE h.id = $1 AND u.notificacoes",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await
    {
        crate::email::enviar_em_segundo_plano(
            email,
            format!("História pronta: {titulo}"),
            format!("Oi, {nome}! A história \"{titulo}\" ficou pronta:\n{}/historias/{id}", crate::email::url_publica()),
        );
    }
    Ok(())
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

/// Gera (pela IA) e grava a memória de uma história: um resumo curto com os fatos importantes.
pub async fn gerar_memoria(st: &AppState, historia: Uuid) -> anyhow::Result<String> {
    let (familia, titulo, conteudo): (Uuid, String, Value) =
        sqlx::query_as("SELECT familia_id, titulo, conteudo FROM conteudo.historias WHERE id = $1")
            .bind(historia)
            .fetch_one(&st.db)
            .await?;
    let c: Conteudo = serde_json::from_value(conteudo).unwrap_or_default();
    let texto: Vec<String> = c
        .paginas
        .iter()
        .flat_map(|p| p.blocos.iter().map(|b| format!("[{}] {}", b.quem, b.texto)))
        .collect();
    let pedido = ia::PedidoTexto {
        sistema: "Você resume histórias infantis para servir de memória em histórias seguintes. Responda só com o JSON pedido.".into(),
        imagens: Vec::new(),
        prompt: format!(
            "Resuma a história \"{titulo}\" em 3 a 6 frases curtas com os fatos que importam para continuações: \
             quem participou, o que aconteceu, quem venceu ou o que foi descoberto, objetos e lugares novos, piadas internas.\n\n{}",
            texto.join("\n")
        ),
        schema: json!({ "type": "object", "additionalProperties": false, "required": ["memoria"],
                        "properties": { "memoria": { "type": "string" } } }),
    };
    let r = ia::gerar_texto(st, familia, &pedido).await?;
    let memoria = r["memoria"].as_str().unwrap_or_default().trim().to_string();
    if memoria.is_empty() {
        anyhow::bail!("a IA não devolveu a memória");
    }
    sqlx::query("UPDATE conteudo.historias SET memoria = $2 WHERE id = $1")
        .bind(historia)
        .bind(&memoria)
        .execute(&st.db)
        .await?;
    Ok(memoria)
}

/// POST /historias/{id}/memoria: gera de novo a memória pela IA.
pub async fn gerar_memoria_rota(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let dona: Option<Uuid> = sqlx::query_scalar("SELECT familia_id FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    if dona != Some(s.familia_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match gerar_memoria(&st, id).await {
        Ok(m) => Json(json!({ "memoria": m })).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "erro": format!("{e:#}") }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct SalvarMemoria {
    memoria: String,
}

/// POST /historias/{id}/memoria/salvar: a família edita a memória.
pub async fn salvar_memoria(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(m): Json<SalvarMemoria>) -> Response {
    match sqlx::query("UPDATE conteudo.historias SET memoria = $3 WHERE id = $1 AND familia_id = $2")
        .bind(id)
        .bind(s.familia_id)
        .bind(m.memoria.trim())
        .execute(&st.db)
        .await
    {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

/// Slug do título, com sufixo -2, -3... se já existir outro igual na família.
async fn slug_livre(st: &AppState, familia_id: Uuid, titulo: &str, proprio: Uuid) -> anyhow::Result<String> {
    let base = Some(util::slug(titulo)).filter(|s| !s.is_empty()).unwrap_or_else(|| "historia".into());
    for n in 1..100 {
        let candidato = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let ocupado: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM conteudo.historias WHERE familia_id = $1 AND slug = $2 AND id <> $3)",
        )
        .bind(familia_id)
        .bind(&candidato)
        .bind(proprio)
        .fetch_one(&st.db)
        .await?;
        if !ocupado {
            return Ok(candidato);
        }
    }
    Ok(format!("{base}-{}", &proprio.to_string()[..8]))
}

// ───────────── páginas ─────────────

#[derive(sqlx::FromRow)]
struct ItemHistoria {
    id: Uuid,
    titulo: String,
    faixa: String,
    estado: String,
    paginas: i32,
}

#[derive(sqlx::FromRow)]
struct LeitorOpcao {
    nome: String,
    faixa: String,
}

#[derive(Template)]
#[template(path = "historias.html")]
struct PaginaHistorias {
    s: Sessao,
    itens: Vec<ItemHistoria>,
    leitores: Vec<LeitorOpcao>,
    personagens: Vec<(String, String)>,
    estantes: Vec<(String, String)>,
}

pub async fn lista(State(st): State<AppState>, s: Sessao) -> Response {
    let itens = sqlx::query_as::<_, ItemHistoria>(
        "SELECT id, titulo, faixa, estado, jsonb_array_length(conteudo->'paginas') AS paginas
           FROM conteudo.historias WHERE familia_id = $1 ORDER BY atualizado_em DESC",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    let leitores = sqlx::query_as::<_, LeitorOpcao>(
        "SELECT nome, faixa FROM acesso.leitores WHERE familia_id = $1 ORDER BY nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    let personagens = personagens_da_familia(&st, s.familia_id).await;
    let estantes = sqlx::query_as::<_, (String, String)>(
        "SELECT slug, nome FROM conteudo.estantes WHERE familia_id = $1 ORDER BY ordem, nome",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    match (itens, leitores, personagens, estantes) {
        (Ok(itens), Ok(leitores), Ok(p), Ok(estantes)) => render(PaginaHistorias {
            s,
            itens,
            leitores,
            personagens: p.into_iter().map(|p| (p.slug, p.nome)).collect(),
            estantes,
        }),
        (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (.., Err(e)) => erro_interno(e.into()),
    }
}

#[derive(Deserialize)]
pub struct FormNova {
    #[serde(default)]
    briefing: String,
    faixa: String,
    narracao: String,
    /// "ia" ou "manual"
    modo: String,
    /// slug da estante onde a história nasce (opcional)
    #[serde(default)]
    estante: String,
    #[serde(default)]
    visibilidade: String,
}

pub async fn criar(State(st): State<AppState>, s: Sessao, Form(f): Form<FormNova>) -> Response {
    let faixa = if FAIXAS.contains(&f.faixa.as_str()) { f.faixa } else { "4-6".into() };
    let narracao = if f.narracao == "narrador" { "narrador" } else { "por-personagem" };
    let manual = f.modo == "manual";
    let id = Uuid::new_v4();
    let conteudo = if manual {
        json!({ "paginas": [ { "cena": "", "personagens": [], "animacao": "nenhuma",
                               "blocos": [ { "quem": "narrador", "texto": "" } ] } ] })
    } else {
        json!({ "paginas": [] })
    };
    let r = sqlx::query(
        "INSERT INTO conteudo.historias (id, familia_id, slug, titulo, faixa, narracao, briefing, estado, conteudo, criado_por, visibilidade)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(s.familia_id)
    .bind(format!("rascunho-{}", &id.to_string()[..8]))
    .bind(if manual { "História sem título" } else { "Gerando…" })
    .bind(&faixa)
    .bind(narracao)
    .bind(Some(f.briefing.trim()).filter(|b| !b.is_empty()))
    .bind(if manual { "texto" } else { "rascunho" })
    .bind(conteudo)
    .bind(s.usuario_id)
    .bind(if ["privado", "compartilhado", "publico"].contains(&f.visibilidade.as_str()) { f.visibilidade.as_str() } else { "herdar" })
    .execute(&st.db)
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    if !f.estante.is_empty() {
        if let Err(e) = crate::estantes::vincular(&st, s.familia_id, &f.estante, id).await {
            return erro_interno(e.into());
        }
    }
    if !manual {
        if let Err(e) = jobs::enfileirar(&st, "gerar_historia", s.familia_id, json!({ "historia_id": id })).await {
            return erro_interno(e);
        }
    }
    Redirect::to(&format!("/historias/{id}")).into_response()
}

#[derive(sqlx::FromRow)]
struct Historia {
    id: Uuid,
    titulo: String,
    faixa: String,
    narracao: String,
    briefing: Option<String>,
    conteudo: Value,
    estilo: Option<String>,
    memoria: Option<String>,
    continua: Option<Value>,
}

#[derive(Template)]
#[template(path = "historia_gerando.html")]
struct PaginaGerando {
    s: Sessao,
    id: Uuid,
    briefing: String,
    erro: Option<String>,
}

#[derive(Template)]
#[template(path = "historia_editor.html")]
struct PaginaEditor {
    s: Sessao,
    titulo: String,
    dados: String,
}

pub async fn abrir(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let h = match sqlx::query_as::<_, Historia>(
        "SELECT h.id, h.titulo, h.faixa, h.narracao, h.briefing, h.conteudo, e.padroes->>'estilo' AS estilo, h.memoria,
                (SELECT json_agg(json_build_object('id', o.id, 'titulo', o.titulo))::jsonb
                   FROM conteudo.historias o WHERE o.id = ANY(h.continua_de)) AS continua
           FROM conteudo.historias h LEFT JOIN conteudo.estantes e ON e.id = h.estante_principal
          WHERE h.id = $1 AND h.familia_id = $2",
    )
    .bind(id)
    .bind(s.familia_id)
    .fetch_optional(&st.db)
    .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return Redirect::to("/historias").into_response(),
        Err(e) => return erro_interno(e.into()),
    };

    let vazia = h.conteudo["paginas"].as_array().is_none_or(|a| a.is_empty());
    if vazia {
        let job: Option<(String, Option<String>)> = match sqlx::query_as(
            "SELECT estado, erro FROM jobs.fila WHERE payload->>'historia_id' = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(id.to_string())
        .fetch_optional(&st.db)
        .await
        {
            Ok(j) => j,
            Err(e) => return erro_interno(e.into()),
        };
        let erro = match job {
            Some((estado, erro)) if estado == "erro" => Some(erro.unwrap_or_else(|| "erro desconhecido".into())),
            _ => None,
        };
        return render(PaginaGerando { s, id, briefing: h.briefing.unwrap_or_default(), erro });
    }

    let personagens = match personagens_da_familia(&st, s.familia_id).await {
        Ok(p) => p,
        Err(e) => return erro_interno(e.into()),
    };
    let dados = json!({
        "id": h.id, "titulo": h.titulo, "faixa": h.faixa, "narracao": h.narracao,
        "briefing": h.briefing, "conteudo": h.conteudo,
        "personagens": personagens, "animacoes": ANIMACOES, "estilo": h.estilo,
        "memoria": h.memoria, "continua": h.continua,
        "imagem_api": crate::imagem::disponivel(&st, s.familia_id).await,
    });
    // JSON dentro de <script>: impede que um texto "</script>" feche a tag.
    let dados = dados.to_string().replace("</", "<\\/");
    render(PaginaEditor { s, titulo: h.titulo, dados })
}

pub async fn regerar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let existe: Result<bool, _> =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conteudo.historias WHERE id = $1 AND familia_id = $2)")
            .bind(id)
            .bind(s.familia_id)
            .fetch_one(&st.db)
            .await;
    match existe {
        Ok(true) => {}
        Ok(false) => return Redirect::to("/historias").into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    if let Err(e) = jobs::enfileirar(&st, "gerar_historia", s.familia_id, json!({ "historia_id": id })).await {
        return erro_interno(e);
    }
    Redirect::to(&format!("/historias/{id}")).into_response()
}

#[derive(Deserialize)]
pub struct SalvarConteudo {
    titulo: String,
    narracao: String,
    conteudo: Conteudo,
}

pub async fn salvar(
    State(st): State<AppState>,
    s: Sessao,
    Path(id): Path<Uuid>,
    Json(c): Json<SalvarConteudo>,
) -> Response {
    let titulo = c.titulo.trim();
    if titulo.is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "erro": "Dê um título para a história." })))
            .into_response();
    }
    let narracao = if c.narracao == "narrador" { "narrador" } else { "por-personagem" };
    let valor = match serde_json::to_value(&c.conteudo) {
        Ok(v) => v,
        Err(e) => return erro_interno(e.into()),
    };
    let slug = match slug_livre(&st, s.familia_id, titulo, id).await {
        Ok(sl) => sl,
        Err(e) => return erro_interno(e),
    };
    let r = async {
        let mut tx = st.db.begin().await?;
        let n = sqlx::query(
            "UPDATE conteudo.historias SET titulo = $3, slug = $4, narracao = $5, conteudo = $6, atualizado_em = now()
              WHERE id = $1 AND familia_id = $2",
        )
        .bind(id)
        .bind(s.familia_id)
        .bind(titulo)
        .bind(&slug)
        .bind(narracao)
        .bind(&valor)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 1 {
            sqlx::query("INSERT INTO conteudo.versoes (historia_id, conteudo, criado_por) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(&valor)
                .bind(s.usuario_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok::<u64, sqlx::Error>(n)
    }
    .await;
    match r {
        Ok(1) => Json(json!({ "ok": true })).into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => erro_interno(e.into()),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn p(slug: &str, nome: &str, apelidos: &[&str], variantes: &[&str]) -> PersonagemIA {
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
        // "Leonel" não é "Léo"
        assert!(detectar("A dona Leonel", &todos).is_empty());
        // Voz extra: "!ingles" (só inglês) e "!misto-ingles"; voz que não existe é ignorada.
        let r = detectar("@leo:normal!misto-ingles e @tito!ingles e @zorgo!frances", &todos);
        let vozes: Vec<_> = r.iter().map(|n| (n.slug.as_str(), n.variante.as_deref(), n.voz.as_deref(), n.misto)).collect();
        assert_eq!(vozes, vec![("leo", Some("normal"), Some("ingles"), true), ("tito", None, Some("ingles"), false), ("zorgo", None, None, false)]);
    }
}

/// POST /historias/{id}/midia: recebe a imagem/GIF/vídeo de uma página. Devolve {id, tipo};
/// o editor coloca na página e salva (assim não sobrescreve edições não salvas).
pub async fn enviar_midia(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, mut mp: Multipart) -> Response {
    let existe: Result<bool, _> =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conteudo.historias WHERE id = $1 AND familia_id = $2)")
            .bind(id)
            .bind(s.familia_id)
            .fetch_one(&st.db)
            .await;
    match existe {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return erro_interno(e.into()),
    }
    let erro = |msg: &str| (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "erro": msg }))).into_response();
    let bytes = loop {
        match mp.next_field().await {
            Ok(Some(campo)) if campo.file_name().is_some() => match campo.bytes().await {
                Ok(b) => break b,
                Err(_) => return erro("Arquivo grande demais (máximo 25 MB)."),
            },
            Ok(Some(_)) => continue,
            Ok(None) => return erro("Nenhum arquivo enviado."),
            Err(_) => return erro("Envio inválido."),
        }
    };
    let Some((mime, ext)) = midia::tipo_midia(&bytes) else {
        return erro("Formato não aceito. Use PNG, JPG, WebP, GIF, MP4, WebM ou MOV.");
    };
    let pasta = format!("familias/{}/historias/{id}/midia", s.familia_slug);
    match midia::salvar(&st, s.familia_id, &pasta, &bytes, mime, ext).await {
        Ok(mid) => Json(json!({ "id": mid, "tipo": mime })).into_response(),
        Err(e) => erro_interno(e),
    }
}

/// GET /mencoes: personagens (para @) e histórias (para #) da família, usados pelo autocompletar do briefing.
pub async fn mencoes(State(st): State<AppState>, s: Sessao) -> Response {
    let personagens = personagens_da_familia(&st, s.familia_id).await;
    let historias: Result<Vec<(String, String)>, _> = sqlx::query_as(
        "SELECT slug, titulo FROM conteudo.historias
          WHERE familia_id = $1 AND jsonb_array_length(conteudo->'paginas') > 0 ORDER BY atualizado_em DESC",
    )
    .bind(s.familia_id)
    .fetch_all(&st.db)
    .await;
    match (personagens, historias) {
        (Ok(p), Ok(h)) => Json(json!({
            "personagens": p.iter().map(|p| json!({
                "slug": p.slug, "nome": p.nome, "apelidos": p.ficha["apelidos"],
                "variantes": p.variantes.as_array().map(|a| a.iter().filter(|v| v["arquivada"] != true)
                    .map(|v| json!({ "slug": v["slug"], "nome": v["nome"] })).collect::<Vec<_>>()),
                "vozes": p.vozes_extras.as_array().map(|a| a.iter()
                    .map(|v| json!({ "slug": v["slug"], "nome": v["nome"], "lingua": v["lingua"] })).collect::<Vec<_>>()),
            })).collect::<Vec<_>>(),
            "historias": h.into_iter().map(|(slug, titulo)| json!({ "slug": slug, "titulo": titulo })).collect::<Vec<_>>(),
        }))
        .into_response(),
        (Err(e), _) | (_, Err(e)) => erro_interno(e.into()),
    }
}

#[derive(Deserialize)]
pub struct PedidoImagem {
    prompt: String,
    #[serde(default)]
    refs: Vec<Uuid>,
}

/// POST /historias/{id}/imagem-ia: gera a imagem da página pela API (o editor coloca na página e salva).
pub async fn gerar_imagem(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>, Json(p): Json<PedidoImagem>) -> Response {
    let dona: Option<Uuid> = sqlx::query_scalar("SELECT familia_id FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    if dona != Some(s.familia_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let refs = crate::imagem::carregar_refs(&st, s.familia_id, &p.refs).await;
    let pasta = format!("historias/{id}/midia");
    match crate::imagem::gerar(&st, s.familia_id, &s.familia_slug, &pasta, &p.prompt, &refs).await {
        Ok((mid, tipo)) => Json(json!({ "id": mid, "tipo": tipo })).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "erro": format!("{e:#}") }))).into_response(),
    }
}

/// POST /historias/{id}/apagar: apaga a história (admin da família ou quem criou). As mídias que ficarem
/// sem uso são apagadas pela limpeza (cópias em outras famílias continuam com os arquivos).
pub async fn apagar(State(st): State<AppState>, s: Sessao, Path(id): Path<Uuid>) -> Response {
    let dados: Option<(Uuid, Option<Uuid>)> = sqlx::query_as("SELECT familia_id, criado_por FROM conteudo.historias WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
    let Some((familia, criador)) = dados.filter(|(f, _)| *f == s.familia_id) else {
        return Redirect::to("/historias").into_response();
    };
    if !s.admin_familia() && criador != Some(s.usuario_id) {
        return Redirect::to(&format!("/historias/{id}")).into_response();
    }
    let r = async {
        let mut tx = st.db.begin().await?;
        sqlx::query("DELETE FROM partilha.destinos WHERE item_tipo = 'historia' AND item_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM partilha.links WHERE item_tipo = 'historia' AND item_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE conteudo.historias SET continua_de = array_remove(continua_de, $1) WHERE $1 = ANY(continua_de)")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM conteudo.historias WHERE id = $1 AND familia_id = $2").bind(id).bind(familia).execute(&mut *tx).await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = r {
        return erro_interno(e.into());
    }
    let st2 = st.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::limpeza::limpar(&st2).await {
            tracing::warn!("limpeza após apagar falhou: {e:#}");
        }
    });
    Redirect::to("/historias").into_response()
}

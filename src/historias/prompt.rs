//! Pedido à IA: schema da resposta, prompt da história e ajuste da resposta às regras do app.

use serde_json::{json, Value};

use super::deteccao::PersonagemIA;
use super::{regras_faixa, Bloco, Conteudo, NaCena, Secundario, ANIMACOES, FAIXAS};

pub fn schema_historia() -> Value {
    json!({
        "type": "object",
                    "additionalProperties": false,
        "required": ["titulo", "faixa", "secundarios", "paginas"],
        "properties": {
            "titulo": { "type": "string" },
            "secundarios": {
                "type": "array",
                "description": "personagens sem ficha que aparecem nas ilustrações, com a aparência fixa",
                "items": {
                    "type": "object", "additionalProperties": false, "required": ["nome", "descricao"],
                    "properties": { "nome": { "type": "string" }, "descricao": { "type": "string" } }
                }
            },
            "faixa": { "type": "string", "enum": FAIXAS, "description": "faixa etária escolhida para esta história" },
            "paginas": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["cena", "personagens", "secundarios", "animacao", "blocos"],
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
                        "secundarios": { "type": "array", "items": { "type": "string" },
                                         "description": "nomes dos secundários (da lista secundarios) que aparecem nesta ilustração" },
                        "animacao": { "type": "string", "enum": ANIMACOES },
                        "blocos": {
                            "type": "array",
                            "items": {
                                "type": "object",
                    "additionalProperties": false,
                                "required": ["quem", "texto", "voz", "nome", "tom", "velocidade"],
                                "properties": { "quem": { "type": "string" }, "texto": { "type": "string" },
                                                "voz": { "type": "string", "description": "\"\" (voz normal) ou o slug da voz extra do personagem" },
                                                "nome": { "type": "string", "description": "só com quem = \"voz:<id>\": nome do personagem secundário; senão \"\"" },
                                                "tom": { "type": "integer", "description": "só com quem = \"voz:<id>\": -60 (grave) a 60 (agudo); senão 0" },
                                                "velocidade": { "type": "integer", "description": "só com quem = \"voz:<id>\": -40 a 40 (%); senão 0" } }
                            }
                        }
                    }
                }
            }
        }
    })
}

pub const SISTEMA: &str = "Você é um autor de histórias infantis em português do Brasil. \
Escreve livros ilustrados: cada página tem uma ilustração e um texto curto. \
Regras fixas: conteúdo adequado para crianças; nada de violência gráfica, medo excessivo, linguagem imprópria ou temas adultos; \
vilões são engraçados e atrapalhados, nunca cruéis; lutas são divertidas, sem ferimentos; finais positivos. \
Respeite a ficha de cada personagem (aparência, personalidade, jeito de falar). \
Responda apenas com o JSON pedido.";

#[allow(clippy::too_many_arguments)]
pub fn montar_prompt(
    briefing: &str,
    leitores: &[(String, String)],
    narracao: &str,
    citados: &[NaCena],
    todos: &[PersonagemIA],
    estilo: &str,
    memorias: &[(String, String)],
    vozes_usadas: &[String],
) -> String {
    // Faixa etária: a IA escolhe pelo briefing, pelas fichas (idade dos personagens) e pelas crianças da família.
    let regras: Vec<String> = FAIXAS
        .iter()
        .map(|f| {
            let (pmin, pmax, wmin, wmax) = regras_faixa(f);
            format!("  {f} anos: {pmin} a {pmax} páginas, {wmin} a {wmax} palavras por página")
        })
        .collect();
    let criancas = if leitores.is_empty() {
        String::new()
    } else {
        let l: Vec<String> = leitores.iter().map(|(n, f)| format!("{n} ({f} anos)")).collect();
        format!(" Crianças que leem nesta família: {}.", l.join(", "))
    };
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
                // Variantes também citadas no briefing (transformação etc.): a IA usa cada uma onde fizer sentido.
                "tambem_citadas": c.tambem.iter().filter_map(|t| {
                    p.variantes.as_array()?.iter().find(|v| v["slug"] == *t).map(|v| json!({
                        "slug": v["slug"], "nome": v["nome"], "forma": v["forma"], "estilo": v["estilo"], "descricao": v["descricao"]
                    }))
                }).collect::<Vec<_>>(),
                "outras_variantes": p.variantes.as_array().map(|a| {
                    a.iter().filter(|v| v["arquivada"] != true).map(|v| v["slug"].clone()).collect::<Vec<_>>()
                }),
            })
        })
        .collect();

    let modo = if narracao == "narrador" {
        "Narração SÓ COM NARRADOR: cada página tem exatamente UM bloco, com quem = \"narrador\". As falas dos personagens entram no texto do narrador com travessão (— Vamos lá! — disse Léo.)."
    } else {
        "Narração POR PERSONAGEM: cada página tem vários blocos em sequência. Trechos narrados usam quem = \"narrador\"; cada fala usa quem = slug do personagem que fala, com o texto só da fala (sem \"disse fulano\"). Alterne narrador e falas de forma natural. Em cada bloco, voz = \"\", salvo quando o campo \"falas\" do personagem pedir outra. \
         Personagens SECUNDÁRIOS sem ficha (ex.: um vilão só desta história, um bichinho, a professora) também podem falar: \
         use quem = \"voz:<id>\" com uma das VOZES DO SISTEMA abaixo, \"nome\" com o nome dele, e \"tom\"/\"velocidade\" para dar personalidade \
         (vilão: tom grave e mais lento; criaturinha: tom agudo e rápido). Mantenha a MESMA voz, nome, tom e velocidade para o mesmo secundário na história toda, \
         e não use para ele a mesma voz de outro secundário. Vozes marcadas \"só inglês\" apenas para falas em inglês. \
         Nos blocos dos personagens com ficha e do narrador: nome = \"\", tom = 0, velocidade = 0."
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
    let secundarios = if narracao == "narrador" {
        "Personagens secundários sem ficha podem aparecer só na narração."
    } else {
        "Personagens secundários sem ficha podem aparecer e falar com as VOZES DO SISTEMA (quem = \"voz:<id>\")."
    };
    let vozes_sistema = if narracao == "narrador" {
        String::new()
    } else {
        // Sem as vozes que o narrador e os personagens da família já usam (para não confundir quem fala).
        let lista: Vec<String> = crate::tts::VOZES
            .iter()
            .filter(|(id, _)| !vozes_usadas.iter().any(|u| u == id))
            .map(|(id, n)| format!("- voz:{id} — {n}"))
            .collect();
        format!("VOZES DO SISTEMA (para secundários sem ficha):\n{}\n\n", lista.join("\n"))
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
        "Escreva uma história infantil.\n{estilo}\
         - Faixa etária: escolha em \"faixa\" a que combina com o briefing (se ele citar idade ou série, siga) e com as fichas \
           (ex.: protagonista \"menino de 4 anos\" → 4-6).{criancas} Na dúvida, 4-6. Siga o tamanho da faixa escolhida:\n{}\n\
         - {modo}\n\
         - Em cada página: \"cena\" descreve a ilustração (lugar, ação, expressões, enquadramento), sem texto escrito na imagem; \
           \"personagens\" lista quem aparece na ilustração com slug e variante (use a variante_a_usar, salvo se o briefing pedir outra; as variantes em tambem_citadas também foram pedidas: use cada uma nas páginas em que fizer sentido, por exemplo depois de uma transformação, e escreva a cena com a aparência da variante daquela página); \
           \"animacao\" é um efeito leve que combine com a cena, ou \"nenhuma\".\n\
         - Personagens SECUNDÁRIOS (sem ficha) que aparecem nas ilustrações: descreva cada um uma vez em \"secundarios\" \
           (nome + aparência FIXA e detalhada, como uma ficha para o ilustrador: espécie/idade, corpo, rosto, roupa, cores, acessórios), \
           e em cada página liste em \"secundarios\" os nomes dos que aparecem na ilustração. A aparência não muda entre as páginas.\n\
         - Use SOMENTE os personagens abaixo (pelos slugs). {secundarios}\n\n\
         PERSONAGENS:\n{}\n\n\
         {vozes_sistema}\
         {anteriores}\
         BRIEFING DA FAMÍLIA:\n{briefing}\n",
        regras.join("\n"),
        serde_json::to_string_pretty(&fichas).unwrap_or_default()
    )
}

/// Ajusta a resposta da IA às regras do app: secundários com nome conhecido, personagens só os citados,
/// animação válida, quem inválido vira narrador, voz extra só se existir, modo narrador = 1 bloco por página.
pub fn normalizar(mut c: Conteudo, narracao: &str, citados: &[NaCena], todos: &[PersonagemIA]) -> Conteudo {
    let nomes = normalizar_secundarios(&mut c.secundarios);
    for p in &mut c.paginas {
        p.secundarios = secundarios_na_cena(&p.secundarios, &nomes);
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
            normalizar_bloco(b, narracao, citados, todos);
        }
        p.blocos.retain(|b| !b.texto.is_empty());
        if narracao == "narrador" && p.blocos.len() > 1 {
            p.blocos = vec![juntar_em_narrador(&p.blocos)];
        }
    }
    c.paginas.retain(|p| !p.blocos.is_empty());
    c
}

/// Limpa a lista de secundários (sem nome vazio) e devolve os nomes.
fn normalizar_secundarios(lista: &mut Vec<Secundario>) -> Vec<String> {
    for sec in lista.iter_mut() {
        sec.nome = sec.nome.trim().to_string();
        sec.descricao = sec.descricao.trim().to_string();
    }
    lista.retain(|s| !s.nome.is_empty());
    lista.iter().map(|s| s.nome.clone()).collect()
}

/// Secundários de uma página: só nomes da lista (com a grafia dela, sem diferenciar maiúsculas), sem repetir.
fn secundarios_na_cena(pedidos: &[String], nomes: &[String]) -> Vec<String> {
    let mut na_cena: Vec<String> = Vec::new();
    for n in pedidos {
        if let Some(x) = nomes.iter().find(|x| x.to_lowercase() == n.trim().to_lowercase()) {
            if !na_cena.contains(x) {
                na_cena.push(x.clone());
            }
        }
    }
    na_cena
}

/// Um bloco: voz do sistema (secundário) mantém nome/tom/velocidade dentro dos limites;
/// os demais perdem esses campos, `quem` desconhecido vira narrador e voz extra inexistente sai.
fn normalizar_bloco(b: &mut Bloco, narracao: &str, citados: &[NaCena], todos: &[PersonagemIA]) {
    b.texto = b.texto.trim().to_string();
    let sistema = b.quem.strip_prefix("voz:").is_some_and(|id| crate::tts::VOZES.iter().any(|(v, _)| *v == id));
    if sistema && narracao != "narrador" {
        b.nome = b.nome.take().map(|n| n.trim().chars().take(40).collect::<String>()).filter(|n| !n.is_empty());
        b.tom = b.tom.map(|t| t.clamp(-60, 60)).filter(|t| *t != 0);
        b.velocidade = b.velocidade.map(|v| v.clamp(-40, 40)).filter(|v| *v != 0);
        b.voz = None;
        return;
    }
    b.nome = None;
    b.tom = None;
    b.velocidade = None;
    if b.quem != "narrador" && !citados.iter().any(|c| c.slug == b.quem) {
        b.quem = "narrador".into();
    }
    let valida = b.voz.as_deref().is_some_and(|v| todos.iter().any(|p| p.slug == b.quem && p.tem_voz(v)));
    if !valida {
        b.voz = None;
    }
}

/// Modo "só narrador": a página vira um bloco único com todo o texto.
fn juntar_em_narrador(blocos: &[Bloco]) -> Bloco {
    let texto = blocos.iter().map(|b| b.texto.as_str()).collect::<Vec<_>>().join(" ");
    Bloco { quem: "narrador".into(), texto, voz: None, nome: None, tom: None, velocidade: None }
}


#[cfg(test)]
mod testes {
    use super::*;
    use crate::historias::deteccao::personagem_teste as p;

    /// Saída estruturada estrita (Anthropic/OpenAI): todo objeto precisa de additionalProperties=false
    /// e de todas as propriedades em "required". Se um campo novo esquecer disso, a IA recusa o pedido.
    fn conferir_estrito(v: &Value, caminho: &str) {
        if v["type"] == "object" {
            assert_eq!(v["additionalProperties"], false, "{caminho}: falta additionalProperties=false");
            let props: Vec<&String> = v["properties"].as_object().map(|o| o.keys().collect()).unwrap_or_default();
            let req: Vec<&str> = v["required"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            for k in &props {
                assert!(req.contains(&k.as_str()), "{caminho}: \"{k}\" não está em required");
            }
            for (k, filho) in v["properties"].as_object().into_iter().flatten() {
                conferir_estrito(filho, &format!("{caminho}.{k}"));
            }
        }
        if v["type"] == "array" {
            conferir_estrito(&v["items"], &format!("{caminho}[]"));
        }
    }

    #[test]
    fn schema_da_historia_e_estrito() {
        conferir_estrito(&schema_historia(), "historia");
    }

    fn bloco(quem: &str, texto: &str) -> Bloco {
        Bloco { quem: quem.into(), texto: texto.into(), voz: None, nome: None, tom: None, velocidade: None }
    }

    fn cena(slug: &str) -> NaCena {
        NaCena { slug: slug.into(), variante: None, voz: None, misto: false, tambem: vec![] }
    }

    fn conteudo(blocos: Vec<Bloco>) -> Conteudo {
        serde_json::from_value(json!({ "paginas": [{ "cena": "", "blocos": blocos }] })).unwrap()
    }

    #[test]
    fn quem_desconhecido_vira_narrador_e_vazios_somem() {
        let c = conteudo(vec![bloco("leo", " Oi! "), bloco("intruso", "Olá"), bloco("leo", "   ")]);
        let c = normalizar(c, "por-personagem", &[cena("leo")], &[p("leo", "Léo", &[], &[])]);
        let b = &c.paginas[0].blocos;
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].quem.as_str(), b[0].texto.as_str()), ("leo", "Oi!"));
        assert_eq!(b[1].quem, "narrador");
    }

    #[test]
    fn voz_extra_so_se_o_personagem_tiver() {
        let mut ok = bloco("leo", "Hello");
        ok.voz = Some("ingles".into());
        let mut ruim = bloco("leo", "Bonjour");
        ruim.voz = Some("frances".into());
        let c = normalizar(conteudo(vec![ok, ruim]), "por-personagem", &[cena("leo")], &[p("leo", "Léo", &[], &[])]);
        assert_eq!(c.paginas[0].blocos[0].voz.as_deref(), Some("ingles"));
        assert_eq!(c.paginas[0].blocos[1].voz, None);
    }

    #[test]
    fn secundario_com_voz_do_sistema_mantem_nome_e_limita_ajustes() {
        let mut b = bloco("voz:en-US-GuyNeural", "Arrr!");
        b.nome = Some("  Capitão  ".into());
        b.tom = Some(-200);
        b.velocidade = Some(0);
        let c = normalizar(conteudo(vec![b, bloco("voz:voz-inventada", "x")]), "por-personagem", &[], &[]);
        let b = &c.paginas[0].blocos;
        assert_eq!(b[0].quem, "voz:en-US-GuyNeural");
        assert_eq!((b[0].nome.as_deref(), b[0].tom, b[0].velocidade), (Some("Capitão"), Some(-60), None));
        // Voz que não existe: vira narrador.
        assert_eq!(b[1].quem, "narrador");
    }

    #[test]
    fn modo_narrador_junta_tudo_num_bloco() {
        let c = conteudo(vec![bloco("narrador", "Era uma vez."), bloco("leo", "— Oi! — disse Léo.")]);
        let c = normalizar(c, "narrador", &[cena("leo")], &[]);
        assert_eq!(c.paginas[0].blocos.len(), 1);
        assert_eq!(c.paginas[0].blocos[0].texto, "Era uma vez. — Oi! — disse Léo.");
    }

    #[test]
    fn secundarios_usam_a_grafia_da_lista_e_paginas_vazias_somem() {
        let c: Conteudo = serde_json::from_value(json!({
            "secundarios": [{ "nome": " Pirata Caveira ", "descricao": " esqueleto " }, { "nome": "  ", "descricao": "x" }],
            "paginas": [
                { "secundarios": ["pirata caveira", "Pirata Caveira", "Fantasma"], "personagens": [{ "slug": "leo", "variante": "" }, { "slug": "zorgo" }],
                  "animacao": "fogos", "blocos": [{ "quem": "narrador", "texto": "a" }] },
                { "blocos": [{ "quem": "narrador", "texto": " " }] }
            ]
        })).unwrap();
        let c = normalizar(c, "por-personagem", &[cena("leo")], &[]);
        assert_eq!(c.secundarios.len(), 1);
        assert_eq!(c.secundarios[0].descricao, "esqueleto");
        assert_eq!(c.paginas.len(), 1);
        let p0 = &c.paginas[0];
        assert_eq!(p0.secundarios, vec!["Pirata Caveira".to_string()]);
        assert_eq!(p0.personagens.len(), 1);
        assert_eq!(p0.personagens[0].variante, None);
        assert_eq!(p0.animacao.as_deref(), Some("nenhuma"));
    }

    #[test]
    fn prompt_traz_faixas_e_tira_vozes_ja_usadas() {
        let todos = vec![p("leo", "Léo", &[], &[])];
        let usadas = vec!["pt-BR-FranciscaNeural".to_string()];
        let t = montar_prompt("O @leo na lua", &[("Pedro".into(), "4-6".into())], "por-personagem", &[cena("leo")], &todos, "", &[], &usadas);
        for f in FAIXAS {
            assert!(t.contains(&format!("{f} anos:")), "falta a regra da faixa {f}");
        }
        assert!(t.contains("Pedro (4-6 anos)"));
        assert!(t.contains("voz:en-US-GuyNeural"));
        assert!(!t.contains("voz:pt-BR-FranciscaNeural"), "voz do narrador não pode ir para secundários");
        // Só narrador: sem lista de vozes do sistema.
        let t = montar_prompt("", &[], "narrador", &[], &todos, "", &[], &[]);
        assert!(!t.contains("VOZES DO SISTEMA"));
        assert!(t.contains("sem briefing"));
    }
}

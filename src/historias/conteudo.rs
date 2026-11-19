//! Modelo do conteúdo de uma história: páginas, blocos (falas), secundários e faixas etárias.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ANIMACOES: [&str; 10] = [
    "nenhuma", "estrelas-piscando", "chuva", "neve", "brilhos", "folhas", "bolhas", "tremor", "onomatopeia-pop",
    "explosao",
];
/// Faixas de exemplo (sugestões na tela e referência de tamanho para a IA). A faixa da história é texto livre.
pub const FAIXAS: [&str; 6] = ["2-3", "4-6", "7-9", "10-12", "13-17", "adulto"];

/// Faixa digitada pela família ou devolvida pela IA: sem espaços sobrando, até 40 letras.
pub fn faixa_limpa(f: &str) -> String {
    f.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect()
}

/// Faixa para mostrar: "4-6" → "4-6 anos"; "adulto" e "5 anos" ficam como estão; vazia → "idade livre".
pub fn faixa_texto(f: &str) -> String {
    let f = f.trim();
    if f.is_empty() {
        "idade livre".into()
    } else if f.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ' ') {
        format!("{f} anos")
    } else {
        f.into()
    }
}

// ───────────── modelo do conteúdo (páginas e blocos) ─────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Conteudo {
    #[serde(default)]
    pub paginas: Vec<Pagina>,
    /// Personagens só desta história (sem ficha), com a aparência fixa usada nos prompts de imagem.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secundarios: Vec<Secundario>,
    /// Lugares só desta história (inventados pela IA ou à mão), com a descrição fixa para as ilustrações.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lugares: Vec<LugarHistoria>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LugarHistoria {
    pub nome: String,
    #[serde(default)]
    pub descricao: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Secundario {
    pub nome: String,
    #[serde(default)]
    pub descricao: String,
    /// Tipo sugerido (crianca, animal, robo, vilao, adulto, criatura), para "Salvar como personagem".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tipo: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Pagina {
    /// Identificador estável da página (o editor cria quando precisa: ex. imagens geradas na fila).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chave: String,
    #[serde(default)]
    pub cena: String,
    #[serde(default)]
    pub personagens: Vec<NaCena>,
    /// Nomes dos secundários (Conteudo.secundarios) que aparecem na ilustração desta página.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secundarios: Vec<String>,
    /// Onde a cena acontece: slug de um lugar da família ou nome de um lugar só desta história ("" = nenhum).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lugar: String,
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
    /// Outras variantes citadas no mesmo briefing (ex.: "@pedro:normal ... vira o @pedro:ranger").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tambem: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bloco {
    /// "narrador" ou o slug do personagem.
    pub quem: String,
    pub texto: String,
    /// Voz extra do personagem usada neste trecho (slug, ex.: "ingles").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voz: Option<String>,
    /// Nome de quem fala quando quem = "voz:<id>" (voz do sistema sem personagem, ex.: "Vilão").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nome: Option<String>,
    /// Ajustes da voz do sistema neste trecho (só com quem = "voz:<id>").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tom: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocidade: Option<i32>,
    /// Voz gravada pela família para esta fala ("dublagem"): toca no lugar da voz automática.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravacao: Option<Gravacao>,
}

/// Áudio gravado no editor (mp3 em midia.arquivos) e a duração em segundos.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Gravacao {
    pub id: Uuid,
    pub duracao: f32,
}

impl Bloco {
    /// Chave da voz do trecho: "pedro" ou "pedro+ingles".
    pub fn chave_voz(&self) -> String {
        if self.quem.starts_with("voz:") {
            let (t, v) = (self.tom.unwrap_or(0), self.velocidade.unwrap_or(0));
            return if t == 0 && v == 0 { self.quem.clone() } else { format!("{}|{t}|{v}", self.quem) };
        }
        match &self.voz {
            Some(v) if self.quem != "narrador" && !v.is_empty() => format!("{}+{v}", self.quem),
            _ => self.quem.clone(),
        }
    }
}

/// Limites de cada faixa etária: (páginas mín, páginas máx, palavras mín por página, palavras máx).
pub fn regras_faixa(faixa: &str) -> (u32, u32, u32, u32) {
    match faixa {
        "2-3" => (6, 8, 15, 30),
        "7-9" => (10, 12, 60, 110),
        "10-12" => (12, 14, 100, 160),
        "13-17" => (12, 16, 140, 220),
        "adulto" => (14, 20, 180, 300),
        _ => (8, 10, 30, 60),
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use serde_json::json;

    fn bloco(v: serde_json::Value) -> Bloco {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn chave_da_voz_de_cada_fala() {
        assert_eq!(bloco(json!({ "quem": "narrador", "texto": "" })).chave_voz(), "narrador");
        assert_eq!(bloco(json!({ "quem": "pedro", "texto": "" })).chave_voz(), "pedro");
        assert_eq!(bloco(json!({ "quem": "pedro", "voz": "ingles", "texto": "" })).chave_voz(), "pedro+ingles");
        assert_eq!(bloco(json!({ "quem": "pedro", "voz": "", "texto": "" })).chave_voz(), "pedro");
        // Narrador nunca usa voz extra.
        assert_eq!(bloco(json!({ "quem": "narrador", "voz": "ingles", "texto": "" })).chave_voz(), "narrador");
        assert_eq!(bloco(json!({ "quem": "voz:en-US-GuyNeural", "texto": "" })).chave_voz(), "voz:en-US-GuyNeural");
        assert_eq!(bloco(json!({ "quem": "voz:en-US-GuyNeural", "tom": -30, "texto": "" })).chave_voz(), "voz:en-US-GuyNeural|-30|0");
    }

    #[test]
    fn historias_antigas_continuam_abrindo() {
        // JSON de antes das vozes extras, secundários e mídia: os campos novos ficam vazios.
        let c: Conteudo = serde_json::from_value(json!({ "paginas": [
            { "cena": "Quintal", "personagens": [{ "slug": "noob" }], "animacao": "chuva",
              "blocos": [{ "quem": "noob", "texto": "Au!" }] }
        ]})).unwrap();
        assert!(c.secundarios.is_empty());
        assert!(c.paginas[0].midia.is_none());
        // E os campos vazios não vão para o JSON gravado.
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("secundarios").is_none());
        assert!(v["paginas"][0]["blocos"][0].get("voz").is_none());
    }

    #[test]
    fn faixa_livre_limpa_e_mostra() {
        assert_eq!(faixa_limpa("  11 e   13 anos "), "11 e 13 anos");
        assert_eq!(faixa_limpa(&"x".repeat(60)).len(), 40);
        assert_eq!(faixa_texto("4-6"), "4-6 anos");
        assert_eq!(faixa_texto("adulto"), "adulto");
        assert_eq!(faixa_texto(""), "idade livre");
    }

    #[test]
    fn regras_das_faixas_crescem_com_a_idade() {
        let mut anterior = (0, 0, 0, 0);
        for f in FAIXAS {
            let r = regras_faixa(f);
            assert!(r.0 <= r.1 && r.2 <= r.3, "{f}: mínimo maior que máximo");
            assert!(r.1 >= anterior.1 && r.3 >= anterior.3, "{f}: menor que a faixa anterior");
            anterior = r;
        }
    }
}

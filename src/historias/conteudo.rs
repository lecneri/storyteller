//! Modelo do conteúdo de uma história: páginas, blocos (falas), secundários e faixas etárias.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ANIMACOES: [&str; 10] = [
    "nenhuma", "estrelas-piscando", "chuva", "neve", "brilhos", "folhas", "bolhas", "tremor", "onomatopeia-pop",
    "explosao",
];
pub const FAIXAS: [&str; 4] = ["2-3", "4-6", "7-9", "10-12"];

// ───────────── modelo do conteúdo (páginas e blocos) ─────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Conteudo {
    #[serde(default)]
    pub paginas: Vec<Pagina>,
    /// Personagens só desta história (sem ficha), com a aparência fixa usada nos prompts de imagem.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secundarios: Vec<Secundario>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Secundario {
    pub nome: String,
    #[serde(default)]
    pub descricao: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Pagina {
    #[serde(default)]
    pub cena: String,
    #[serde(default)]
    pub personagens: Vec<NaCena>,
    /// Nomes dos secundários (Conteudo.secundarios) que aparecem na ilustração desta página.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secundarios: Vec<String>,
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
        _ => (8, 10, 30, 60),
    }
}

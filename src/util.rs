/// Normaliza um nome: minúsculas, sem acento, espaços e símbolos viram "-".
/// "Família Silva" → "familia-silva", "Super Léo" → "super-leo".
pub fn slug(texto: &str) -> String {
    let ascii = deunicode::deunicode(texto).to_lowercase();
    let mut out = String::with_capacity(ascii.len());
    for c in ascii.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod testes {
    use super::slug;

    #[test]
    fn normaliza_nomes() {
        assert_eq!(slug("Família Silva"), "familia-silva");
        assert_eq!(slug("  Super Léo!! "), "super-leo");
        assert_eq!(slug("Léo Mestre Pokémon"), "leo-mestre-pokemon");
    }
}

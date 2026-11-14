//! Personagens e variantes (forma × estilo).

mod com_ia;
mod juntar;
mod paginas;
mod variantes;

pub use com_ia::*;
pub use juntar::*;
pub use paginas::*;
pub use variantes::*;

pub const TIPOS: [(&str, &str); 6] = [
    ("crianca", "Criança / herói"),
    ("animal", "Animal"),
    ("robo", "Robô / veículo"),
    ("vilao", "Vilão"),
    ("adulto", "Adulto"),
    ("criatura", "Criatura"),
];

fn nome_tipo(t: &str) -> &'static str {
    TIPOS.iter().find(|(k, _)| *k == t).map(|(_, v)| *v).unwrap_or("Outro")
}


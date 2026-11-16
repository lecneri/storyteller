//! Histórias: criação (briefing → IA, ou manual), fila de geração e editor de blocos.

mod ajuda;
mod conteudo;
mod deteccao;
mod geracao;
mod memoria;
mod prompt;
mod rotas;

pub use ajuda::{executar_ajuda, importar_ajuda, pedir as pedir_ajuda, ver as ver_ajuda};
pub use conteudo::*;
pub use geracao::executar_geracao;
pub use memoria::*;
pub use rotas::*;

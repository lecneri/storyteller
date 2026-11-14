//! Histórias: criação (briefing → IA, ou manual), fila de geração e editor de blocos.

mod conteudo;
mod deteccao;
mod geracao;
mod memoria;
mod prompt;
mod rotas;

pub use conteudo::*;
pub use geracao::executar_geracao;
pub use memoria::*;
pub use rotas::*;

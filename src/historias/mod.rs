//! Histórias: criação (briefing → IA, ou manual), fila de geração e editor de blocos.

mod ajuda;
mod conteudo;
mod deteccao;
mod geracao;
pub mod imagens;
mod referencia;
mod prompt;
mod rotas;

pub use ajuda::{executar_ajuda, importar_ajuda, pedir as pedir_ajuda, ver as ver_ajuda};
pub use conteudo::*;
pub use geracao::executar_geracao;
pub use referencia::*;
pub use rotas::*;
pub use deteccao::{grupos as grupos_da_ficha, variante_no_grupo};

//! Limite de tentativas em memória (login, "esqueci a senha", PIN): poucas por janela de tempo, por chave.
//! Fica só no processo: reiniciar o app zera, o que basta para frear quem tenta adivinhar senha ou PIN.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

fn mapa() -> &'static Mutex<HashMap<String, (u32, Instant)>> {
    static M: OnceLock<Mutex<HashMap<String, (u32, Instant)>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// Conta uma tentativa para a chave; false = passou do máximo dentro da janela.
pub fn tentar(chave: &str, max: u32, janela: Duration) -> bool {
    tentar_em(chave, max, janela, Instant::now())
}

fn tentar_em(chave: &str, max: u32, janela: Duration, agora: Instant) -> bool {
    let mut m = mapa().lock().unwrap_or_else(|e| e.into_inner());
    if m.len() > 10_000 {
        m.retain(|_, (_, inicio)| agora.duration_since(*inicio) < Duration::from_secs(3600));
    }
    let e = m.entry(chave.to_string()).or_insert((0, agora));
    if agora.duration_since(e.1) >= janela {
        *e = (0, agora);
    }
    e.0 += 1;
    e.0 <= max
}

/// Esquece as tentativas da chave (ex.: depois de entrar com a senha certa).
pub fn zerar(chave: &str) {
    mapa().lock().unwrap_or_else(|e| e.into_inner()).remove(chave);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limita_e_libera_depois_da_janela() {
        let (j, t0) = (Duration::from_secs(60), Instant::now());
        assert!(tentar_em("teste:a", 2, j, t0));
        assert!(tentar_em("teste:a", 2, j, t0));
        assert!(!tentar_em("teste:a", 2, j, t0));
        assert!(tentar_em("teste:b", 2, j, t0), "cada chave conta sozinha");
        assert!(tentar_em("teste:a", 2, j, t0 + j), "passada a janela, volta a valer");
        assert!(tentar_em("teste:a", 2, j, t0 + j));
        zerar("teste:a");
        assert!(tentar_em("teste:a", 2, j, t0 + j));
    }
}

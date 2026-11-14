//! Criptografia das chaves de API (AES-256-GCM com a CHAVE_MESTRA do ambiente).

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::STANDARD, Engine};

/// A chave-mestra vem do ambiente; se não houver, é gerada uma vez e guardada em DATA_DIR/segredos/chave-mestra.
fn chave_mestra() -> anyhow::Result<String> {
    if let Ok(k) = std::env::var("CHAVE_MESTRA") {
        if !k.trim().is_empty() {
            return Ok(k);
        }
    }
    let dir = std::path::PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "/data".into())).join("segredos");
    let arq = dir.join("chave-mestra");
    if let Ok(k) = std::fs::read_to_string(&arq) {
        return Ok(k);
    }
    use aes_gcm::aead::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let k = STANDARD.encode(bytes);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&arq, &k)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&arq, std::fs::Permissions::from_mode(0o600));
    }
    tracing::info!("chave-mestra criada em {}", arq.display());
    Ok(k)
}

fn cifra() -> anyhow::Result<Aes256Gcm> {
    let b64 = chave_mestra()?;
    let bytes = STANDARD.decode(b64.trim()).context("CHAVE_MESTRA inválida (base64)")?;
    if bytes.len() != 32 {
        return Err(anyhow!("CHAVE_MESTRA precisa ter 32 bytes"));
    }
    Ok(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&bytes)))
}

pub fn disponivel() -> bool {
    cifra().is_ok()
}

pub fn cifrar(texto: &str) -> anyhow::Result<Vec<u8>> {
    let c = cifra()?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut saida = nonce.to_vec();
    saida.extend(c.encrypt(&nonce, texto.as_bytes()).map_err(|_| anyhow!("falha ao cifrar"))?);
    Ok(saida)
}

pub fn decifrar(dados: &[u8]) -> anyhow::Result<String> {
    if dados.len() < 13 {
        return Err(anyhow!("dado cifrado curto demais"));
    }
    let (nonce, resto) = dados.split_at(12);
    let claro = cifra()?.decrypt(Nonce::from_slice(nonce), resto).map_err(|_| anyhow!("falha ao decifrar"))?;
    Ok(String::from_utf8(claro)?)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn ida_e_volta() {
        std::env::set_var("CHAVE_MESTRA", STANDARD.encode([7u8; 32]));
        let c = cifrar("sk-teste-123").unwrap();
        assert_ne!(&c[12..], b"sk-teste-123");
        assert_eq!(decifrar(&c).unwrap(), "sk-teste-123");
    }
}

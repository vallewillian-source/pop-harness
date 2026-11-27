use std::collections::BTreeMap;

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::rand::{SecureRandom, SystemRandom};

pub const KEY_VAR: &str = "POPH_CHAVE_CIFRA";

const SAL: &[u8] = b"poph/integracao/segredos/v1";
const INFO: &[u8] = b"aes-256-gcm";
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CipherFailure {
    MissingKey,
    InvalidBlob,
    CannotDecipher,
}

impl std::fmt::Display for CipherFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingKey => write!(f, "`{KEY_VAR}` não está definida neste processo"),
            Self::InvalidBlob => write!(f, "o segredo gravado não tem o formato esperado"),
            Self::CannotDecipher => write!(
                f,
                "o segredo gravado não abre com a chave atual (`{KEY_VAR}` mudou?)"
            ),
        }
    }
}

impl std::error::Error for CipherFailure {}

struct TrintaEDois;

impl KeyType for TrintaEDois {
    fn len(&self) -> usize {
        32
    }
}

#[derive(Clone)]
pub struct CipherKey {
    chave: LessSafeKey,
}

impl std::fmt::Debug for CipherKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChaveDeCifra(…)")
    }
}

impl CipherKey {
    pub fn derive(value: &str) -> Result<Self, CipherFailure> {
        if value.trim().is_empty() {
            return Err(CipherFailure::MissingKey);
        }
        let salt = Salt::new(HKDF_SHA256, SAL);
        let prk = salt.extract(value.as_bytes());
        let okm = prk
            .expand(&[INFO], TrintaEDois)
            .map_err(|_| CipherFailure::MissingKey)?;
        let mut bytes = [0u8; 32];
        okm.fill(&mut bytes)
            .map_err(|_| CipherFailure::MissingKey)?;
        let without_key =
            UnboundKey::new(&AES_256_GCM, &bytes).map_err(|_| CipherFailure::MissingKey)?;
        Ok(Self {
            chave: LessSafeKey::new(without_key),
        })
    }

    pub fn from_environment() -> Option<Self> {
        let value = std::env::var(KEY_VAR).ok()?;
        match Self::derive(&value) {
            Ok(chave) => Some(chave),
            Err(failure) => {
                tracing::warn!("falha" = %failure, "chave de cifra ignorada: segredos de integração não poderão ser gravados");
                None
            }
        }
    }

    pub fn cifrar(&self, segredos: &BTreeMap<String, String>) -> Result<Vec<u8>, CipherFailure> {
        let claro = serde_json::to_vec(segredos).map_err(|_| CipherFailure::CannotDecipher)?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| CipherFailure::CannotDecipher)?;
        let mut em_buffer = claro;
        self.chave
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::empty(),
                &mut em_buffer,
            )
            .map_err(|_| CipherFailure::CannotDecipher)?;
        let mut blob = Vec::with_capacity(NONCE_LEN + em_buffer.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&em_buffer);
        Ok(blob)
    }

    pub fn decifrar(&self, blob: &[u8]) -> Result<BTreeMap<String, String>, CipherFailure> {
        if blob.len() <= NONCE_LEN {
            return Err(CipherFailure::InvalidBlob);
        }
        let (nonce_bytes, cifrado) = blob.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce_bytes
            .try_into()
            .map_err(|_| CipherFailure::InvalidBlob)?;
        let mut em_buffer = cifrado.to_vec();
        let claro = self
            .chave
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut em_buffer,
            )
            .map_err(|_| CipherFailure::CannotDecipher)?;
        serde_json::from_slice(claro).map_err(|_| CipherFailure::InvalidBlob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(nome, value)| ((*nome).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn cipher_round_trip_keeps_map_without_plaintext() {
        let chave = CipherKey::derive("chave-de-teste-da-hr44c").expect("chave");
        let original = map(&[("token", "SEGREDO-QUE-NAO-PODE-VAZAR")]);
        let blob = chave.cifrar(&original).expect("cifra");
        assert!(
            !String::from_utf8_lossy(&blob).contains("SEGREDO-QUE-NAO-PODE-VAZAR"),
            "o blob não pode conter o texto em claro"
        );
        assert_eq!(chave.decifrar(&blob).expect("decifra"), original);
    }

    #[test]
    fn nonce_differs_per_write() {
        let chave = CipherKey::derive("chave-de-teste-da-hr44c").expect("chave");
        let segredos = map(&[("token", "mesmo-valor")]);
        let um = chave.cifrar(&segredos).expect("primeira");
        let two = chave.cifrar(&segredos).expect("segunda");
        assert_ne!(um, two, "dois blobs iguais revelariam valor repetido");
        assert_eq!(chave.decifrar(&um).unwrap(), chave.decifrar(&two).unwrap());
    }

    #[test]
    fn other_key_cannot_open() {
        let um = CipherKey::derive("chave-um").expect("chave");
        let other = CipherKey::derive("chave-dois").expect("chave");
        let blob = um.cifrar(&map(&[("token", "x")])).expect("cifra");
        assert_eq!(other.decifrar(&blob), Err(CipherFailure::CannotDecipher));
    }

    #[test]
    fn empty_key_and_short_blob_are_refused() {
        assert!(matches!(
            CipherKey::derive("   "),
            Err(CipherFailure::MissingKey)
        ));
        let chave = CipherKey::derive("chave").expect("chave");
        assert_eq!(chave.decifrar(&[0u8; 3]), Err(CipherFailure::InvalidBlob));
    }

    #[test]
    fn debug_of_key_shows_nothing() {
        let chave = CipherKey::derive("SENTINELA-CHAVE-123").expect("chave");
        assert!(!format!("{chave:?}").contains("SENTINELA"));
    }
}

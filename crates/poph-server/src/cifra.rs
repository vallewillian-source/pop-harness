//! Cifra dos segredos de integração em repouso (HR-44c).
//!
//! O contrato do plugin já diz que segredo só viaja no handshake; este módulo cuida do outro lado:
//! o que fica gravado em `integrations.secrets_enc`. O conteúdo é um objeto JSON
//! (`{"api_key":"..."}`) cifrado com **AES-256-GCM** (`ring`), com nonce aleatório por gravação.
//!
//! A chave vem de `POPH_CHAVE_CIFRA` (só do ambiente, como todos os segredos) e é derivada com
//! **HKDF-SHA256** para exatamente 32 bytes: assim o operador pode usar um texto longo ou uma chave
//! já aleatória, sem combinar formato. O sal e o `info` são fixos do produto — mudá-los tornaria
//! ilegível o que já está gravado.
//!
//! Formato do blob: `nonce (12 bytes) || ciphertext || tag (16 bytes)`.
//!
//! Consequências, registradas em `docs/API.md`:
//! - sem a variável, gravar segredo devolve `503 cifra_indisponivel` (não se grava em claro);
//! - trocar o valor da variável torna os segredos antigos ilegíveis — eles continuam guardados, e a
//!   API informa `tem_segredo: true` sem conseguir listar quais campos estão preenchidos;
//! - o valor do segredo nunca sai daqui: nem em log, nem em erro, nem em resposta.

use std::collections::BTreeMap;

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::rand::{SecureRandom, SystemRandom};

/// Variável de ambiente com a chave de cifra (a mesma do `poph doctor`).
pub const VAR_CHAVE: &str = "POPH_CHAVE_CIFRA";

/// Sal fixo da derivação (não é segredo; separa esta chave de qualquer outro uso do mesmo valor).
const SAL: &[u8] = b"poph/integracao/segredos/v1";
/// `info` da derivação.
const INFO: &[u8] = b"aes-256-gcm";
/// Tamanho do nonce do AES-GCM.
const TAMANHO_NONCE: usize = 12;

/// Por que uma cifra ou decifra falhou. Nunca carrega o valor do segredo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalhaCifra {
    /// `POPH_CHAVE_CIFRA` ausente ou vazia neste processo.
    SemChave,
    /// Blob gravado não tem o formato esperado (curto demais).
    BlobInvalido,
    /// A chave atual não abre este blob (variável trocada ou conteúdo adulterado).
    NaoDecifra,
}

impl std::fmt::Display for FalhaCifra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SemChave => write!(f, "`{VAR_CHAVE}` não está definida neste processo"),
            Self::BlobInvalido => write!(f, "o segredo gravado não tem o formato esperado"),
            Self::NaoDecifra => write!(
                f,
                "o segredo gravado não abre com a chave atual (`{VAR_CHAVE}` mudou?)"
            ),
        }
    }
}

impl std::error::Error for FalhaCifra {}

/// Comprimento fixo de uma chave AES-256.
struct TrintaEDois;

impl KeyType for TrintaEDois {
    fn len(&self) -> usize {
        32
    }
}

/// Chave de cifra pronta para uso (derivada uma vez na subida do servidor).
#[derive(Clone)]
pub struct ChaveDeCifra {
    chave: LessSafeKey,
}

impl std::fmt::Debug for ChaveDeCifra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nunca imprime a chave: só o fato de existir.
        f.write_str("ChaveDeCifra(…)")
    }
}

impl ChaveDeCifra {
    /// Deriva a chave do valor da variável de ambiente (HKDF-SHA256 → 32 bytes).
    ///
    /// O valor em si não é guardado: só a chave derivada, em memória.
    pub fn derivar(valor: &str) -> Result<Self, FalhaCifra> {
        if valor.trim().is_empty() {
            return Err(FalhaCifra::SemChave);
        }
        let salt = Salt::new(HKDF_SHA256, SAL);
        let prk = salt.extract(valor.as_bytes());
        let okm = prk
            .expand(&[INFO], TrintaEDois)
            .map_err(|_| FalhaCifra::SemChave)?;
        let mut bytes = [0u8; 32];
        okm.fill(&mut bytes).map_err(|_| FalhaCifra::SemChave)?;
        let sem_chave = UnboundKey::new(&AES_256_GCM, &bytes).map_err(|_| FalhaCifra::SemChave)?;
        Ok(Self {
            chave: LessSafeKey::new(sem_chave),
        })
    }

    /// Lê `POPH_CHAVE_CIFRA` do ambiente. `None` = não configurada (ou vazia).
    ///
    /// Valor inválido não impede o servidor de subir: vira `None` com um aviso, e quem precisar
    /// gravar segredo recebe `503 cifra_indisponivel` com a orientação.
    pub fn do_ambiente() -> Option<Self> {
        let valor = std::env::var(VAR_CHAVE).ok()?;
        match Self::derivar(&valor) {
            Ok(chave) => Some(chave),
            Err(falha) => {
                tracing::warn!(%falha, "chave de cifra ignorada: segredos de integração não poderão ser gravados");
                None
            }
        }
    }

    /// Cifra o mapa de segredos. O JSON em claro existe só na memória desta chamada.
    pub fn cifrar(&self, segredos: &BTreeMap<String, String>) -> Result<Vec<u8>, FalhaCifra> {
        let claro = serde_json::to_vec(segredos).map_err(|_| FalhaCifra::NaoDecifra)?;
        let mut nonce_bytes = [0u8; TAMANHO_NONCE];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| FalhaCifra::NaoDecifra)?;
        let mut em_buffer = claro;
        self.chave
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::empty(),
                &mut em_buffer,
            )
            .map_err(|_| FalhaCifra::NaoDecifra)?;
        let mut blob = Vec::with_capacity(TAMANHO_NONCE + em_buffer.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&em_buffer);
        Ok(blob)
    }

    /// Decifra o blob gravado. Falha sem expor o conteúdo.
    pub fn decifrar(&self, blob: &[u8]) -> Result<BTreeMap<String, String>, FalhaCifra> {
        if blob.len() <= TAMANHO_NONCE {
            return Err(FalhaCifra::BlobInvalido);
        }
        let (nonce_bytes, cifrado) = blob.split_at(TAMANHO_NONCE);
        let nonce: [u8; TAMANHO_NONCE] = nonce_bytes
            .try_into()
            .map_err(|_| FalhaCifra::BlobInvalido)?;
        let mut em_buffer = cifrado.to_vec();
        let claro = self
            .chave
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut em_buffer,
            )
            .map_err(|_| FalhaCifra::NaoDecifra)?;
        serde_json::from_slice(claro).map_err(|_| FalhaCifra::BlobInvalido)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn mapa(pares: &[(&str, &str)]) -> BTreeMap<String, String> {
        pares
            .iter()
            .map(|(nome, valor)| ((*nome).to_owned(), (*valor).to_owned()))
            .collect()
    }

    #[test]
    fn cifra_e_decifra_volta_o_mesmo_mapa_sem_texto_em_claro() {
        let chave = ChaveDeCifra::derivar("chave-de-teste-da-hr44c").expect("chave");
        let original = mapa(&[("token", "SEGREDO-QUE-NAO-PODE-VAZAR")]);
        let blob = chave.cifrar(&original).expect("cifra");
        assert!(
            !String::from_utf8_lossy(&blob).contains("SEGREDO-QUE-NAO-PODE-VAZAR"),
            "o blob não pode conter o texto em claro"
        );
        assert_eq!(chave.decifrar(&blob).expect("decifra"), original);
    }

    #[test]
    fn nonce_diferente_a_cada_gravacao() {
        let chave = ChaveDeCifra::derivar("chave-de-teste-da-hr44c").expect("chave");
        let segredos = mapa(&[("token", "mesmo-valor")]);
        let um = chave.cifrar(&segredos).expect("primeira");
        let dois = chave.cifrar(&segredos).expect("segunda");
        assert_ne!(um, dois, "dois blobs iguais revelariam valor repetido");
        assert_eq!(chave.decifrar(&um).unwrap(), chave.decifrar(&dois).unwrap());
    }

    #[test]
    fn outra_chave_nao_abre() {
        let um = ChaveDeCifra::derivar("chave-um").expect("chave");
        let outro = ChaveDeCifra::derivar("chave-dois").expect("chave");
        let blob = um.cifrar(&mapa(&[("token", "x")])).expect("cifra");
        assert_eq!(outro.decifrar(&blob), Err(FalhaCifra::NaoDecifra));
    }

    #[test]
    fn chave_vazia_e_recusada_e_blob_curto_tambem() {
        assert!(matches!(
            ChaveDeCifra::derivar("   "),
            Err(FalhaCifra::SemChave)
        ));
        let chave = ChaveDeCifra::derivar("chave").expect("chave");
        assert_eq!(chave.decifrar(&[0u8; 3]), Err(FalhaCifra::BlobInvalido));
    }

    #[test]
    fn debug_da_chave_nao_mostra_nada() {
        let chave = ChaveDeCifra::derivar("SENTINELA-CHAVE-123").expect("chave");
        assert!(!format!("{chave:?}").contains("SENTINELA"));
    }
}

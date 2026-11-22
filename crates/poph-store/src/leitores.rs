//! Pool de leitores: várias conexões de leitura, uma por vez em uso.
//!
//! Com WAL, leitura não bloqueia escrita. O pool evita abrir conexão a cada consulta e limita o
//! número de conexões abertas (`MAX_LEITORES`). Se todas estiverem ocupadas, a chamada abre uma
//! conexão extra e a descarta ao devolver — prefere-se uma conexão a mais a uma fila de espera.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::conexao::abrir_leitura;
use crate::erro::{ErroStore, Resultado};

/// Quantas conexões o pool guarda para reuso.
const MAX_LEITORES: usize = 8;

/// Pool de conexões de leitura.
pub(crate) struct PoolLeitores {
    caminho: PathBuf,
    livres: Mutex<Vec<Connection>>,
}

impl PoolLeitores {
    /// Cria o pool apontando para o banco (não abre conexão agora).
    pub fn novo(caminho: &Path) -> Self {
        Self {
            caminho: caminho.to_path_buf(),
            livres: Mutex::new(Vec::new()),
        }
    }

    /// Roda a consulta numa conexão de leitura do pool.
    pub fn com<T>(&self, consulta: impl FnOnce(&Connection) -> Resultado<T>) -> Resultado<T> {
        let mut conexao = {
            let mut livres = self.livres.lock().unwrap_or_else(|e| e.into_inner());
            livres.pop()
        };
        let conexao = match conexao.take() {
            Some(conexao) => conexao,
            None => abrir_leitura(&self.caminho)?,
        };
        let resultado = consulta(&conexao);
        let mut livres = self.livres.lock().unwrap_or_else(|e| e.into_inner());
        if livres.len() < MAX_LEITORES {
            livres.push(conexao);
        }
        resultado
    }
}

impl std::fmt::Debug for PoolLeitores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let quantidade = self.livres.lock().map(|livres| livres.len()).unwrap_or(0);
        f.debug_struct("PoolLeitores")
            .field("caminho", &self.caminho)
            .field("livres", &quantidade)
            .finish()
    }
}

/// Converte um `Option` vazio de consulta em "não encontrado" com entidade e id.
pub(crate) fn exigir<T>(valor: Option<T>, entidade: &'static str, id: &str) -> Resultado<T> {
    valor.ok_or_else(|| {
        ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
            entidade,
            id: id.to_owned(),
        })
    })
}

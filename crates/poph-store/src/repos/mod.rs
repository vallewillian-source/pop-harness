//! Repositórios por entidade.
//!
//! Regra que vale para todos: **`org_id` é obrigatório na assinatura de leitura e escrita**.
//! Não existe método que consulte uma tabela de negócio sem o escopo da org; o teste de
//! isolamento do crate passa por todos os repositórios.
//!
//! Escrita passa pelo escritor dedicado (`Store::escrita`), leitura pelo pool (`Store::leitura`).
//! Toda entidade editável tem `versao`, exigida no `If-Match` do `atualizar`.

use poph_types::DomainError;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row};

use crate::erro::ErroStore;
use crate::modelo::Status;

mod agentes;
mod biblioteca;
mod caixas;
mod carrinho;
mod categorias;
mod conhecimento;
mod conversas;
mod departamentos;
mod fichas;
mod fluxos;
mod instrucoes;
mod matrix;
mod matrix_midia;
mod mensagens;
mod orgaos;
mod produtos;
mod retencao;
mod tokens;
mod turnos;

pub use agentes::Agentes;
pub use biblioteca::{Biblioteca, LIMITE_DESCRICAO_ARQUIVO, LIMITE_NOME_ARQUIVO};
pub use caixas::{CaixaEntrada, CaixaSaida};
pub use carrinho::{Carrinhos, ItemCarrinho, LIMITE_QUANTIDADE, ResumoCarrinho};
pub use categorias::Categorias;
pub use conhecimento::Conhecimento;
pub use conversas::{
    Conversas, FiltroConversas, LIMITE_NOME_CLIENTE, OrdemConversas, normalizar_nome_cliente,
};
pub use departamentos::Departamentos;
pub use fichas::{Fichas, MOTIVO_ABANDONO_ATENDIMENTO};
pub use fluxos::{Fluxos, MOTIVO_EXCLUSAO_FLUXO, TETO_FLUXOS_POR_AGENTE};
pub use instrucoes::InstrucoesRepo;
pub use matrix::{
    AtrasoMatrix, CursorConversas, CursoresMatrix, DesfechoAviso, EspacoDesligado, EspacoMatrix,
    EstadoMatrixOrg, EstadoSala, FalhaDeRespostaHumana, FalhaMatrix, LIMITE_FALHAS,
    LIMITE_MOTIVO_FALHA, Matrix, OrigemEspaco, SalaMatrix,
};
pub use matrix_midia::{
    CategoriaMidia, LIMITE_ERRO_MIDIA, LIMITE_PENDENTES, MatrixMidia, MidiaDaSala, NovaMidia,
};
pub use mensagens::{LIMITE_DESDE, MensagemDesde, Mensagens};
pub use orgaos::Orgaos;
pub use produtos::{Produtos, ReferenciasProduto};
pub use retencao::{Retencao, VarreduraRetencao};
pub use tokens::Tokens;
pub use turnos::{Auditoria, Turnos};

pub(crate) use crate::leitores::exigir;

/// Lê um id tipado de uma coluna de texto.
pub(crate) fn id_da_coluna<T>(linha: &Row<'_>, indice: usize) -> rusqlite::Result<T>
where
    T: std::str::FromStr<Err = DomainError>,
{
    let texto: String = linha.get(indice)?;
    texto.parse().map_err(|erro: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(indice, Type::Text, Box::new(erro))
    })
}

/// Lê um id opcional.
pub(crate) fn id_opcional<T>(linha: &Row<'_>, indice: usize) -> rusqlite::Result<Option<T>>
where
    T: std::str::FromStr<Err = DomainError>,
{
    let texto: Option<String> = linha.get(indice)?;
    match texto {
        Some(texto) => texto.parse().map(Some).map_err(|erro: DomainError| {
            rusqlite::Error::FromSqlConversionFailure(indice, Type::Text, Box::new(erro))
        }),
        None => Ok(None),
    }
}

/// Lê o campo `status`.
pub(crate) fn status_da_coluna(linha: &Row<'_>, indice: usize) -> rusqlite::Result<Status> {
    let texto: String = linha.get(indice)?;
    Status::de_texto(&texto).map_err(|erro| {
        rusqlite::Error::FromSqlConversionFailure(indice, Type::Text, Box::new(erro))
    })
}

/// Converte INTEGER em `u64` (versões e contadores nunca são negativos).
pub(crate) fn u64_da_coluna(linha: &Row<'_>, indice: usize) -> rusqlite::Result<u64> {
    let valor: i64 = linha.get(indice)?;
    Ok(valor.max(0) as u64)
}

/// Confere que o registro referenciado existe **na mesma org**. Usado nas referências opcionais,
/// que a chave estrangeira simples não consegue proteger por org.
pub(crate) fn pertence(
    conn: &Connection,
    tabela: &'static str,
    entidade: &'static str,
    org_id: &str,
    id: &str,
) -> crate::erro::Resultado<()> {
    let existe: Option<i64> = conn
        .query_row(
            &format!("SELECT 1 FROM {tabela} WHERE org_id = ?1 AND id = ?2"),
            rusqlite::params![org_id, id],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(ErroStore::from)?;
    match existe {
        Some(_) => Ok(()),
        None => Err(ErroStore::Dominio(DomainError::NaoEncontrado {
            entidade,
            id: id.to_owned(),
        })),
    }
}

/// Converte INTEGER em `u32`.
pub(crate) fn u32_da_coluna(linha: &Row<'_>, indice: usize) -> rusqlite::Result<u32> {
    let valor: i64 = linha.get(indice)?;
    Ok(valor.clamp(0, i64::from(u32::MAX)) as u32)
}

/// Monta o erro certo quando o `UPDATE ... WHERE version = ?` não pegou nenhuma linha:
/// conflito de versão se o registro existe, "não encontrado" se não existe (ou é de outra org).
pub(crate) fn conflito_ou_ausente(
    conn: &Connection,
    tabela: &'static str,
    entidade: &'static str,
    id: &str,
    org_id: &str,
    esperada: u64,
) -> ErroStore {
    let atual: Result<Option<i64>, ErroStore> = conn
        .query_row(
            &format!("SELECT version FROM {tabela} WHERE id = ?1 AND org_id = ?2"),
            rusqlite::params![id, org_id],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(ErroStore::from);
    match atual {
        Ok(Some(atual)) => ErroStore::Dominio(DomainError::ConflitoDeVersao {
            entidade,
            id: id.to_owned(),
            esperada,
            atual: atual.max(0) as u64,
        }),
        Ok(None) | Err(_) => ErroStore::Dominio(DomainError::NaoEncontrado {
            entidade,
            id: id.to_owned(),
        }),
    }
}

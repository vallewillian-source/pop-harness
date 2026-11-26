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

use crate::error::StoreError;
use crate::model::Status;

mod agentes;
mod analytics;
mod biblioteca;
mod caixas;
mod carrinho;
mod categorias;
mod conhecimento;
mod conversas;
mod departamentos;
mod efeitos;
mod execucoes;
mod fichas;
mod fluxos;
mod instrucoes;
mod matrix;
mod matrix_midia;
mod mensagens;
mod orgaos;
mod produtos;
mod retencao;
mod revisoes;
mod tokens;
mod turnos;
mod usage;
pub use usage::{AgentUsage, DayUsage, TokenUsage, Usage};

pub use agentes::{Agentes, agentes_por_provedor_em};
pub use analytics::{Analytics, NodeCount, OperationalSummary};
pub use biblioteca::{Biblioteca, LIMITE_DESCRICAO_ARQUIVO, LIMITE_NOME_ARQUIVO};
pub use caixas::{CaixaEntrada, CaixaSaida};
pub use carrinho::{Carrinhos, ItemCarrinho, LIMITE_QUANTIDADE, ResumoCarrinho};
pub use categorias::Categorias;
pub use conhecimento::Conhecimento;
pub use conversas::{
    Conversas, FiltroConversas, LIMITE_NOME_CLIENTE, OrdemConversas, normalizar_nome_cliente,
};
pub use departamentos::Departamentos;
pub use efeitos::Efeitos;
pub use execucoes::{Execucoes, LIMITE_EXECUCOES};
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
pub use revisoes::{ACAO_PUBLICAR, ATORES_DE_AUDITORIA, LIMITE_REVISOES, Revisoes};
pub use tokens::Tokens;
pub use turnos::{Auditoria, Turnos};

pub(crate) use crate::readers::require;

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
    Status::from_text(&texto).map_err(|erro| {
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
) -> crate::error::Resultado<()> {
    let existe: Option<i64> = conn
        .query_row(
            &format!("SELECT 1 FROM {tabela} WHERE org_id = ?1 AND id = ?2"),
            rusqlite::params![org_id, id],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    match existe {
        Some(_) => Ok(()),
        None => Err(StoreError::Domain(DomainError::NotFound {
            entity: entidade,
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
) -> StoreError {
    let atual: Result<Option<i64>, StoreError> = conn
        .query_row(
            &format!("SELECT version FROM {tabela} WHERE id = ?1 AND org_id = ?2"),
            rusqlite::params![id, org_id],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(StoreError::from);
    match atual {
        Ok(Some(atual)) => StoreError::Domain(DomainError::VersionConflict {
            entity: entidade,
            id: id.to_owned(),
            expected: esperada,
            current: atual.max(0) as u64,
        }),
        Ok(None) | Err(_) => StoreError::Domain(DomainError::NotFound {
            entity: entidade,
            id: id.to_owned(),
        }),
    }
}

mod conversation_analytics;
pub use conversation_analytics::{
    ConversationAgent, ConversationAnalytics, ConversationChannel, ConversationCounts,
    ConversationDay, ConversationReport, ConversationSummary,
};

mod sla_analytics;
pub use sla_analytics::{SlaAnalytics, SlaItem, SlaReport};

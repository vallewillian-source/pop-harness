//! Interface com o armazenamento. A retenção não conhece SQL: quem implementa é o adaptador sobre
//! `poph-store`, e as operações de apagar ficam dentro do banco, numa transação só por org.
//!
//! Contrato da operação [`Armazem::varrer`]:
//! - apaga (ou, com `simular`, só conta e desfaz) apenas o que venceu até `corte_ms`;
//! - devolve as contagens e, em `hashes_referenciados`, os sha256 que **continuam** com dono
//!   depois da varredura (anexos vivos e biblioteca de arquivos), para a varredura de arquivos;
//! - o banco é o último a mudar: se falhar, nenhum arquivo é apagado.

use std::path::PathBuf;

use poph_types::OrgId;
use serde::Serialize;

use crate::erro::ErroRetencao;

/// Dados mínimos de uma org para a retenção.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgRetencao {
    /// Id da org.
    pub id: OrgId,
    /// Slug (para mensagens legíveis).
    pub slug: String,
    /// Configuração JSON da org (`orgs.config_json`).
    pub config_json: String,
}

/// O que a varredura apagou (ou apagaria, na simulação) numa org.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct VarreduraBanco {
    /// Mensagens além da retenção.
    pub mensagens: u64,
    /// Anexos que saem junto com as mensagens.
    pub anexos: u64,
    /// Turnos encerrados antes do corte.
    pub turnos: u64,
    /// Entradas já processadas antes do corte.
    pub inbox: u64,
    /// Saídas já enviadas antes do corte.
    pub outbox: u64,
    /// Conversas vazias e antigas.
    pub conversas: u64,
    /// Ids das conversas afetadas (mensagem apagada ou conversa removida).
    pub ids_conversas: Vec<String>,
    /// sha256 que ainda têm dono depois da varredura. Não vai para o relatório.
    #[serde(skip)]
    pub hashes_referenciados: Vec<String>,
}

/// Acesso que a retenção precisa. Implementado sobre `poph-store` (ver docs/RETENCAO.md).
pub trait Armazem {
    /// Todas as orgs, para a varredura geral.
    fn orgs(&self) -> Result<Vec<OrgRetencao>, ErroRetencao>;
    /// Uma org pelo id.
    fn org(&self, id: OrgId) -> Result<OrgRetencao, ErroRetencao>;
    /// Varre o banco da org até `corte_ms`. Com `simular`, nada é gravado.
    fn varrer(
        &self,
        org: OrgId,
        corte_ms: i64,
        simular: bool,
    ) -> Result<VarreduraBanco, ErroRetencao>;
    /// Pasta de blobs da org (`<blobs>/<org>`).
    fn pasta_blobs(&self, org: OrgId) -> PathBuf;
    /// Grava uma linha de `audit_log` para a org.
    fn registrar_auditoria(
        &self,
        org: OrgId,
        ator_tipo: &str,
        detalhe_json: &str,
    ) -> Result<(), ErroRetencao>;
}

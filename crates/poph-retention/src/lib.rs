//! Retenção do histórico do POP Harness (HR-51).
//!
//! - [`config`]: lê `retencao.dias` de cada org (padrão 30; `0` = nunca apagar; valor inválido é erro).
//! - [`armazem`]: a interface com o banco. Quem implementa é o adaptador sobre `poph-store`.
//! - [`blobs`]: arquivos em `data/blobs/<org>/` sem dono, e temporários `.tmp` abandonados.
//! - [`execucao`]: varre uma org ou todas, com relógio injetado, simulação (dry-run) e auditoria.
//!
//! O que nunca é apagado: base de conhecimento, produtos, biblioteca de arquivos, orgs, agentes,
//! tokens, departamentos, categorias e `audit_log`. Pendente de inbox/outbox nunca é apagado.

pub mod armazem;
pub mod banco;
pub mod blobs;
pub mod config;
pub mod erro;
pub mod execucao;

pub use armazem::{Armazem, OrgRetencao, VarreduraBanco};
pub use banco::BancoHarness;
pub use erro::ErroRetencao;
pub use execucao::{
    CARENCIA_BLOBS_MS, Opcoes, RelatorioExecucao, RelatorioOrg, Situacao, executar_org,
    executar_todas,
};

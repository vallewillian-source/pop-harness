//! Execução da retenção: uma org por vez, com relógio injetado, simulação e auditoria.
//!
//! Ordem por org: lê a retenção → varre o banco → lista os arquivos sem dono → apaga os arquivos
//! (só depois de o banco ter sido varrido com sucesso) → grava a auditoria.
//! Falha numa org não para as outras.

use std::collections::HashSet;

use poph_types::OrgId;
use serde::Serialize;
use serde_json::json;

use crate::armazem::{Armazem, OrgRetencao, VarreduraBanco};
use crate::blobs::{encontrar_orfaos, remover};
use crate::config::{corte_ms, dias_da_org};
use crate::erro::ErroRetencao;

/// Carência para blobs sem dono: arquivos mais novos que isso não são apagados (um pedido pode
/// ter acabado de gravar o arquivo e ainda não ter registrado o anexo).
pub const CARENCIA_BLOBS_MS: i64 = 3_600_000;

/// Parâmetros de uma execução.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opcoes {
    /// Instante "agora", em ms desde a época Unix (injetado: os testes simulam o futuro).
    pub agora_ms: i64,
    /// Retenção padrão, quando a org não define a sua (`retencao.dias` da configuração).
    pub padrao_dias: u32,
    /// Carência dos arquivos sem dono, em ms.
    pub carencia_blobs_ms: i64,
    /// `true` = simulação (dry-run): conta e lista, sem apagar nada.
    pub simular: bool,
    /// Tipo de ator para a auditoria (`cli` ou `system`).
    pub ator_tipo: String,
}

impl Opcoes {
    /// Opções com a carência padrão.
    pub fn novas(agora_ms: i64, padrao_dias: u32, simular: bool, ator_tipo: &str) -> Self {
        Self {
            agora_ms,
            padrao_dias,
            carencia_blobs_ms: CARENCIA_BLOBS_MS,
            simular,
            ator_tipo: ator_tipo.to_owned(),
        }
    }
}

/// Como terminou a org.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Situacao {
    /// Varrida sem problema.
    Varrida,
    /// Pulada de propósito (retenção `0`): nada foi apagado.
    Ignorada,
    /// Algo falhou. Veja `motivo`; o que já foi apagado no banco continua apagado.
    Erro,
}

/// Resultado de uma org.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatorioOrg {
    /// Id da org.
    pub org_id: String,
    /// Slug da org.
    pub slug: String,
    /// Situação final.
    pub situacao: Situacao,
    /// Dias de retenção usados.
    pub dias: Option<u32>,
    /// Instante de corte usado.
    pub corte_ms: Option<i64>,
    /// Contagens do banco (ou o que seriam, na simulação).
    pub banco: Option<VarreduraBanco>,
    /// Arquivos sem dono encontrados.
    pub arquivos_sem_dono: Vec<String>,
    /// Arquivos efetivamente apagados.
    pub arquivos_apagados: u64,
    /// Motivo, quando não foi "varrida" limpa.
    pub motivo: Option<String>,
}

/// Resultado de uma execução inteira.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatorioExecucao {
    /// Instante usado.
    pub agora_ms: i64,
    /// Se foi simulação.
    pub simulacao: bool,
    /// Uma entrada por org.
    pub orgs: Vec<RelatorioOrg>,
}

/// Varre uma org pelo id.
pub fn executar_org(
    armazem: &dyn Armazem,
    id: OrgId,
    opcoes: &Opcoes,
) -> Result<RelatorioOrg, ErroRetencao> {
    let org = armazem.org(id)?;
    Ok(varrer_uma(armazem, &org, opcoes))
}

/// Varre todas as orgs. Uma org com problema não impede as outras.
pub fn executar_todas(
    armazem: &dyn Armazem,
    opcoes: &Opcoes,
) -> Result<RelatorioExecucao, ErroRetencao> {
    let orgs = armazem.orgs()?;
    let orgs = orgs
        .iter()
        .map(|org| varrer_uma(armazem, org, opcoes))
        .collect();
    Ok(RelatorioExecucao {
        agora_ms: opcoes.agora_ms,
        simulacao: opcoes.simular,
        orgs,
    })
}

fn varrer_uma(armazem: &dyn Armazem, org: &OrgRetencao, opcoes: &Opcoes) -> RelatorioOrg {
    let base = |situacao: Situacao, motivo: Option<String>| RelatorioOrg {
        org_id: org.id.to_string(),
        slug: org.slug.clone(),
        situacao,
        dias: None,
        corte_ms: None,
        banco: None,
        arquivos_sem_dono: Vec::new(),
        arquivos_apagados: 0,
        motivo,
    };

    let dias = match dias_da_org(&org.slug, &org.config_json, opcoes.padrao_dias) {
        Ok(dias) => dias,
        Err(erro) => return base(Situacao::Erro, Some(erro.to_string())),
    };
    let Some(corte) = corte_ms(opcoes.agora_ms, dias) else {
        let mut relatorio = base(
            Situacao::Ignorada,
            Some("retenção 0: nada é apagado nesta org".to_owned()),
        );
        relatorio.dias = Some(dias);
        return relatorio;
    };

    let banco = match armazem.varrer(org.id, corte, opcoes.simular) {
        Ok(banco) => banco,
        Err(erro) => {
            // Banco falhou: nenhum arquivo é tocado.
            let mut relatorio = base(Situacao::Erro, Some(erro.to_string()));
            relatorio.dias = Some(dias);
            relatorio.corte_ms = Some(corte);
            return relatorio;
        }
    };

    let mut motivos = Vec::new();
    let referenciados: HashSet<String> = banco.hashes_referenciados.iter().cloned().collect();
    let pasta = armazem.pasta_blobs(org.id);
    let corte_arquivos = opcoes.agora_ms.saturating_sub(opcoes.carencia_blobs_ms);
    let orfaos = match encontrar_orfaos(&pasta, &referenciados, corte_arquivos) {
        Ok(orfaos) => orfaos,
        Err(erro) => {
            motivos.push(erro.to_string());
            Vec::new()
        }
    };

    let mut apagados = 0_u64;
    if !opcoes.simular {
        for orfao in &orfaos {
            match remover(orfao) {
                Ok(()) => apagados += 1,
                Err(erro) => motivos.push(erro.to_string()),
            }
        }
    }

    let detalhe = json!({
        "simulacao": opcoes.simular,
        "dias": dias,
        "corte_ms": corte,
        "mensagens": banco.mensagens,
        "anexos": banco.anexos,
        "turnos": banco.turnos,
        "inbox": banco.inbox,
        "outbox": banco.outbox,
        "conversas": banco.conversas,
        "arquivos_sem_dono": orfaos.len(),
        "arquivos_apagados": apagados,
    });
    if let Err(erro) = armazem.registrar_auditoria(org.id, &opcoes.ator_tipo, &detalhe.to_string())
    {
        motivos.push(format!("auditoria não gravada: {erro}"));
    }

    let situacao = if motivos.is_empty() {
        Situacao::Varrida
    } else {
        Situacao::Erro
    };
    RelatorioOrg {
        org_id: org.id.to_string(),
        slug: org.slug.clone(),
        situacao,
        dias: Some(dias),
        corte_ms: Some(corte),
        banco: Some(banco),
        arquivos_sem_dono: orfaos.into_iter().map(|o| o.nome).collect(),
        arquivos_apagados: apagados,
        motivo: if motivos.is_empty() {
            None
        } else {
            Some(motivos.join("; "))
        },
    }
}

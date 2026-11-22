//! Departamento que recebe a transferência para humano (G4, HR-32).
//!
//! O gate **decide um nome**; aqui o nome vira **efeito**, e o efeito nunca depende da confiança do
//! modelo:
//!
//! 1. a escolha do Jev é casada com um departamento **que existe agora** no banco (org da conversa e
//!    situação ativa), não com a lista que foi enviada na pergunta;
//! 2. departamento de outra org, apagado, inativo ou com nome ambíguo é **recusado**, com o motivo
//!    gravado no registro do turno — e a conversa vai para o padrão do agente;
//! 3. sem escolha confiável (gate desligado, abstenção, erro, prazo, confiança baixa), vale o
//!    departamento de transferência do agente — exatamente o comportamento de antes do G4;
//! 4. agente sem departamento configurado: a conversa é transferida **sem** departamento, como hoje.
//!
//! O padrão do agente é o campo `handoff_departamento` (o destino humano), e não o
//! `departamento_padrao` (o departamento em que o agente atende): são coisas diferentes e trocar uma
//! pela outra mudaria para onde a conversa humana vai.
//!
//! **O padrão não é validado**, de propósito: ele é o comportamento de hoje e validá-lo mudaria o
//! que acontece com o G4 desligado. Fica registrado como limite conhecido na HR-32.

use poph_jev::casar_candidato;
use poph_store::Departamento;
use poph_types::DepartmentId;

/// Um departamento que pode receber a conversa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidato {
    /// Id (o que a conversa grava).
    pub id: DepartmentId,
    /// Nome (o que o Jev vê e responde).
    pub nome: String,
}

impl Candidato {
    /// A partir do registro do banco.
    pub fn de(departamento: &Departamento) -> Self {
        Self {
            id: departamento.id,
            nome: departamento.nome.clone(),
        }
    }
}

/// De onde veio o departamento gravado na transferência.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origem {
    /// O G4 escolheu e a escolha passou na validação do código.
    G4,
    /// Departamento de transferência do agente (gate desligado, abstenção ou escolha recusada).
    Padrao,
}

impl Origem {
    /// Texto usado na auditoria e no registro do turno.
    pub fn como_texto(self) -> &'static str {
        match self {
            Origem::G4 => "g4",
            Origem::Padrao => "padrao",
        }
    }
}

/// O que a transferência vai gravar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escolha {
    /// Departamento que recebe a conversa (`None` = transfere sem departamento).
    pub departamento: Option<DepartmentId>,
    /// De onde veio.
    pub origem: Origem,
    /// Por que não foi a escolha do Jev (quando não foi).
    pub motivo: Option<String>,
}

impl Escolha {
    /// Transferência para o padrão do agente, com o motivo.
    fn padrao(padrao: Option<DepartmentId>, motivo: Option<String>) -> Self {
        let motivo = match (padrao, motivo) {
            (_, Some(motivo)) => Some(motivo),
            (None, None) => Some("agente sem departamento de transferência configurado".to_owned()),
            (Some(_), None) => None,
        };
        Self {
            departamento: padrao,
            origem: Origem::Padrao,
            motivo,
        }
    }
}

/// Resolve o departamento da transferência.
///
/// - `escolha_do_jev`: escolha **confiável** do G4 (`None` quando o gate está desligado, não se
///   aplicou, absteve ou a chamada falhou);
/// - `candidatos`: departamentos ativos da org **lidos no momento da decisão**;
/// - `padrao`: `handoff_departamento` do agente;
/// - `motivo_da_abstencao`: por que não houve escolha confiável (vai para o registro).
pub fn resolver(
    escolha_do_jev: Option<&str>,
    candidatos: &[Candidato],
    padrao: Option<DepartmentId>,
    motivo_da_abstencao: Option<&str>,
) -> Escolha {
    let Some(escolha) = escolha_do_jev else {
        return Escolha::padrao(padrao, motivo_da_abstencao.map(str::to_owned));
    };
    let nomes: Vec<String> = candidatos
        .iter()
        .map(|candidato| candidato.nome.clone())
        .collect();
    let canonico = match casar_candidato(escolha, &nomes) {
        Ok(canonico) => canonico,
        Err(motivo) => return Escolha::padrao(padrao, Some(motivo)),
    };
    let encontrados: Vec<&Candidato> = candidatos
        .iter()
        .filter(|candidato| candidato.nome.trim() == canonico)
        .collect();
    match encontrados.as_slice() {
        [unico] => Escolha {
            departamento: Some(unico.id),
            origem: Origem::G4,
            motivo: None,
        },
        [] => Escolha::padrao(
            padrao,
            Some(format!(
                "o departamento \"{canonico}\" não é um departamento ativo desta empresa"
            )),
        ),
        varios => Escolha::padrao(
            padrao,
            Some(format!(
                "o departamento \"{canonico}\" é ambíguo: {} departamentos ativos têm esse nome",
                varios.len()
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidato(nome: &str) -> Candidato {
        Candidato {
            id: DepartmentId::generate(),
            nome: nome.to_owned(),
        }
    }

    fn ids(candidatos: &[Candidato]) -> Vec<DepartmentId> {
        candidatos.iter().map(|candidato| candidato.id).collect()
    }

    #[test]
    fn escolha_valida_e_do_jev() {
        let candidatos = vec![candidato("Comercial"), candidato("Financeiro")];
        let padrao = DepartmentId::generate();
        let escolha = resolver(Some("Financeiro"), &candidatos, Some(padrao), None);
        assert_eq!(escolha.departamento, Some(candidatos[1].id));
        assert_eq!(escolha.origem, Origem::G4);
        assert_eq!(escolha.motivo, None);
    }

    #[test]
    fn nome_com_caixa_diferente_casa_com_o_canonico() {
        let candidatos = vec![candidato("Comercial"), candidato("Financeiro")];
        let escolha = resolver(Some(" financeiro "), &candidatos, None, None);
        assert_eq!(escolha.departamento, Some(candidatos[1].id));
        assert_eq!(escolha.origem, Origem::G4);
    }

    #[test]
    fn departamento_de_outra_org_ou_inexistente_cai_no_padrao() {
        let candidatos = vec![candidato("Comercial"), candidato("Suporte")];
        let padrao = DepartmentId::generate();
        for escolha in ["Jurídico", "Comercial de outra org", ""] {
            let resultado = resolver(Some(escolha), &candidatos, Some(padrao), None);
            assert_eq!(
                resultado.departamento,
                Some(padrao),
                "escolha {escolha:?} devia cair no padrão"
            );
            assert_eq!(resultado.origem, Origem::Padrao);
            assert!(
                resultado.motivo.is_some(),
                "escolha recusada precisa de motivo registrado"
            );
        }
    }

    #[test]
    fn sem_escolha_do_jev_usa_o_padrao_com_o_motivo_da_abstencao() {
        let candidatos = vec![candidato("Comercial"), candidato("Suporte")];
        let padrao = DepartmentId::generate();
        let escolha = resolver(
            None,
            &candidatos,
            Some(padrao),
            Some("confiança 0.40 abaixo"),
        );
        assert_eq!(escolha.departamento, Some(padrao));
        assert_eq!(escolha.origem, Origem::Padrao);
        assert_eq!(escolha.motivo.as_deref(), Some("confiança 0.40 abaixo"));
    }

    #[test]
    fn sem_padrao_transfere_sem_departamento() {
        let candidatos = vec![candidato("Comercial"), candidato("Suporte")];
        for escolha in [None, Some("Jurídico")] {
            let resultado = resolver(escolha, &candidatos, None, None);
            assert_eq!(resultado.departamento, None);
            assert_eq!(resultado.origem, Origem::Padrao);
            assert!(resultado.motivo.is_some());
        }
    }

    #[test]
    fn nome_ambiguo_cai_no_padrao() {
        let candidatos = vec![candidato("Financeiro"), candidato("Financeiro")];
        let padrao = DepartmentId::generate();
        let escolha = resolver(Some("Financeiro"), &candidatos, Some(padrao), None);
        assert_eq!(escolha.departamento, Some(padrao));
        assert!(
            escolha
                .motivo
                .as_deref()
                .unwrap_or_default()
                .contains("ambíguo")
        );
    }

    #[test]
    fn candidato_fora_da_lista_de_ids_nao_entra() {
        // O id escolhido é sempre um dos candidatos lidos do banco.
        let candidatos = vec![candidato("Comercial"), candidato("Suporte")];
        let escolha = resolver(Some("Suporte"), &candidatos, None, None);
        let validos = ids(&candidatos);
        assert!(validos.contains(&escolha.departamento.expect("escolheu")));
    }
}

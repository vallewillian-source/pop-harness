use poph_jev::match_candidate;
use poph_store::Departamento;
use poph_types::DepartmentId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: DepartmentId,
    pub nome: String,
}

impl Candidate {
    pub fn de(departamento: &Departamento) -> Self {
        Self {
            id: departamento.id,
            nome: departamento.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    G4,
    Default,
}

impl Origin {
    pub fn as_text(self) -> &'static str {
        match self {
            Origin::G4 => "g4",
            Origin::Default => "padrao",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub departamento: Option<DepartmentId>,
    pub origin: Origin,
    pub motivo: Option<String>,
}

impl Choice {
    fn default(default: Option<DepartmentId>, motivo: Option<String>) -> Self {
        let motivo = match (default, motivo) {
            (_, Some(motivo)) => Some(motivo),
            (None, None) => Some("agente sem departamento de transferência configurado".to_owned()),
            (Some(_), None) => None,
        };
        Self {
            departamento: default,
            origin: Origin::Default,
            motivo,
        }
    }
}

pub fn resolver(
    choice_jev: Option<&str>,
    candidates: &[Candidate],
    default: Option<DepartmentId>,
    reason_of_abstention: Option<&str>,
) -> Choice {
    let Some(choice) = choice_jev else {
        return Choice::default(default, reason_of_abstention.map(str::to_owned));
    };
    let names: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.nome.clone())
        .collect();
    let canonical = match match_candidate(choice, &names) {
        Ok(canonical) => canonical,
        Err(motivo) => return Choice::default(default, Some(motivo)),
    };
    let found: Vec<&Candidate> = candidates
        .iter()
        .filter(|candidate| candidate.nome.trim() == canonical)
        .collect();
    match found.as_slice() {
        [single] => Choice {
            departamento: Some(single.id),
            origin: Origin::G4,
            motivo: None,
        },
        [] => Choice::default(
            default,
            Some(format!(
                "o departamento \"{canonical}\" não é um departamento ativo desta empresa"
            )),
        ),
        varios => Choice::default(
            default,
            Some(format!(
                "o departamento \"{canonical}\" é ambíguo: {} departamentos ativos têm esse nome",
                varios.len()
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(nome: &str) -> Candidate {
        Candidate {
            id: DepartmentId::generate(),
            nome: nome.to_owned(),
        }
    }

    fn ids(candidates: &[Candidate]) -> Vec<DepartmentId> {
        candidates.iter().map(|candidate| candidate.id).collect()
    }

    #[test]
    fn choice_validates_jev() {
        let candidates = vec![candidate("Comercial"), candidate("Financeiro")];
        let default = DepartmentId::generate();
        let choice = resolver(Some("Financeiro"), &candidates, Some(default), None);
        assert_eq!(choice.departamento, Some(candidates[1].id));
        assert_eq!(choice.origin, Origin::G4);
        assert_eq!(choice.motivo, None);
    }

    #[test]
    fn name_with_box_different_case_with_o_canonical() {
        let candidates = vec![candidate("Comercial"), candidate("Financeiro")];
        let choice = resolver(Some(" financeiro "), &candidates, None, None);
        assert_eq!(choice.departamento, Some(candidates[1].id));
        assert_eq!(choice.origin, Origin::G4);
    }

    #[test]
    fn department_of_other_org_or_missing_falls_default() {
        let candidates = vec![candidate("Comercial"), candidate("Suporte")];
        let default = DepartmentId::generate();
        for choice in ["Jurídico", "Comercial de outra org", ""] {
            let resultado = resolver(Some(choice), &candidates, Some(default), None);
            assert_eq!(
                resultado.departamento,
                Some(default),
                "escolha {choice:?} devia cair no padrão"
            );
            assert_eq!(resultado.origin, Origin::Default);
            assert!(
                resultado.motivo.is_some(),
                "escolha recusada precisa de motivo registrado"
            );
        }
    }

    #[test]
    fn without_choice_jev_uses_o_default_with_o_reason_of_abstention() {
        let candidates = vec![candidate("Comercial"), candidate("Suporte")];
        let default = DepartmentId::generate();
        let choice = resolver(
            None,
            &candidates,
            Some(default),
            Some("confiança 0.40 abaixo"),
        );
        assert_eq!(choice.departamento, Some(default));
        assert_eq!(choice.origin, Origin::Default);
        assert_eq!(choice.motivo.as_deref(), Some("confiança 0.40 abaixo"));
    }

    #[test]
    fn without_default_transfere_without_department() {
        let candidates = vec![candidate("Comercial"), candidate("Suporte")];
        for choice in [None, Some("Jurídico")] {
            let resultado = resolver(choice, &candidates, None, None);
            assert_eq!(resultado.departamento, None);
            assert_eq!(resultado.origin, Origin::Default);
            assert!(resultado.motivo.is_some());
        }
    }

    #[test]
    fn name_ambiguo_falls_default() {
        let candidates = vec![candidate("Financeiro"), candidate("Financeiro")];
        let default = DepartmentId::generate();
        let choice = resolver(Some("Financeiro"), &candidates, Some(default), None);
        assert_eq!(choice.departamento, Some(default));
        assert!(
            choice
                .motivo
                .as_deref()
                .unwrap_or_default()
                .contains("ambíguo")
        );
    }

    #[test]
    fn candidate_outside_of_list_of_ids_not_enters() {
        let candidates = vec![candidate("Comercial"), candidate("Suporte")];
        let choice = resolver(Some("Suporte"), &candidates, None, None);
        let valid = ids(&candidates);
        assert!(valid.contains(&choice.departamento.expect("escolheu")));
    }
}

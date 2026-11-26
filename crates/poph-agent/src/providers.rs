use std::collections::BTreeMap;
use std::sync::Arc;

use poph_core::llm::{LlmError, LlmProvider};

#[derive(Clone)]
pub struct ResolvedProvider {
    nome: String,
    client: Arc<dyn LlmProvider>,
    modelo: String,
}

impl ResolvedProvider {
    pub fn new(
        nome: impl Into<String>,
        client: Arc<dyn LlmProvider>,
        modelo: impl Into<String>,
    ) -> Self {
        Self {
            nome: nome.into(),
            client,
            modelo: modelo.into(),
        }
    }

    pub fn nome(&self) -> &str {
        &self.nome
    }

    pub fn client(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.client)
    }

    pub fn modelo(&self) -> &str {
        &self.modelo
    }
}

impl std::fmt::Debug for ResolvedProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvedorResolvido")
            .field("nome", &self.nome)
            .field("modelo", &self.modelo)
            .finish_non_exhaustive()
    }
}

pub struct Providers {
    default: ResolvedProvider,
    named: BTreeMap<String, ResolvedProvider>,
}

impl Providers {
    pub fn new(default: ResolvedProvider, named: BTreeMap<String, ResolvedProvider>) -> Self {
        Self { default, named }
    }

    pub fn default(&self) -> &ResolvedProvider {
        &self.default
    }

    pub fn names(&self) -> Vec<String> {
        self.named.keys().cloned().collect()
    }

    pub fn resolver(&self, choice: &str) -> Result<ResolvedProvider, LlmError> {
        let choice = choice.trim();
        if choice.is_empty() || poph_types::llm_provider_accepted(choice) {
            return Ok(self.default.clone());
        }
        if let Some(provedor) = self.named.get(choice) {
            return Ok(provedor.clone());
        }
        let declared = self.names();
        let list = if declared.is_empty() {
            "nenhum provedor nomeado configurado".to_owned()
        } else {
            declared.join(", ")
        };
        Err(LlmError::Configuracao(format!(
            "provedor `{choice}` não existe nesta instância; use vazio (o padrão do [llm]) ou um de [provedores]: {list}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use poph_core::llm::{FakeProvider, FinishReason, Script};

    use super::*;

    fn fake(nome: &str) -> Arc<dyn LlmProvider> {
        Arc::new(FakeProvider::with_scripts(vec![Script::Events(vec![
            poph_core::llm::LlmEvent::Texto(nome.to_owned()),
            poph_core::llm::LlmEvent::Fim(FinishReason::Completed),
        ])]))
    }

    fn registro() -> Providers {
        let default = ResolvedProvider::new("antigravity2api", fake("padrao"), "modelo-padrao");
        let mut named = BTreeMap::new();
        named.insert(
            "openai".to_owned(),
            ResolvedProvider::new("openai", fake("openai"), "gpt-de-teste"),
        );
        Providers::new(default, named)
    }

    #[test]
    fn empty_name_of_protocolo_caem_default() {
        let provider = registro();
        for choice in ["", "  ", "antigravity2api", "openai_compativel"] {
            let resolvido = provider.resolver(choice).expect("resolve no padrão");
            assert_eq!(resolvido.nome(), "antigravity2api");
            assert_eq!(resolvido.modelo(), "modelo-padrao");
        }
    }

    #[test]
    fn name_declared_returns_o_client_dele() {
        let provider = registro();
        let resolvido = provider.resolver("openai").expect("resolve o nomeado");
        assert_eq!(resolvido.nome(), "openai");
        assert_eq!(resolvido.modelo(), "gpt-de-teste");
    }

    #[test]
    fn name_unknown_error_claro_with_disponiveis() {
        let provider = registro();
        let error = provider.resolver("inexistente").expect_err("recusa");
        let texto = error.to_string();
        assert!(texto.contains("inexistente"), "{texto}");
        assert!(texto.contains("openai"), "{texto}");
    }
}

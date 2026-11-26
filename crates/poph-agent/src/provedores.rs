use std::collections::BTreeMap;
use std::sync::Arc;

use poph_core::llm::{LlmError, LlmProvider};

#[derive(Clone)]
pub struct ProvedorResolvido {
    nome: String,
    cliente: Arc<dyn LlmProvider>,
    modelo: String,
}

impl ProvedorResolvido {
    pub fn novo(
        nome: impl Into<String>,
        cliente: Arc<dyn LlmProvider>,
        modelo: impl Into<String>,
    ) -> Self {
        Self {
            nome: nome.into(),
            cliente,
            modelo: modelo.into(),
        }
    }

    pub fn nome(&self) -> &str {
        &self.nome
    }

    pub fn cliente(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.cliente)
    }

    pub fn modelo(&self) -> &str {
        &self.modelo
    }
}

impl std::fmt::Debug for ProvedorResolvido {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvedorResolvido")
            .field("nome", &self.nome)
            .field("modelo", &self.modelo)
            .finish_non_exhaustive()
    }
}

pub struct Provedores {
    padrao: ProvedorResolvido,
    nomeados: BTreeMap<String, ProvedorResolvido>,
}

impl Provedores {
    pub fn novo(padrao: ProvedorResolvido, nomeados: BTreeMap<String, ProvedorResolvido>) -> Self {
        Self { padrao, nomeados }
    }

    pub fn padrao(&self) -> &ProvedorResolvido {
        &self.padrao
    }

    pub fn nomes(&self) -> Vec<String> {
        self.nomeados.keys().cloned().collect()
    }

    pub fn resolver(&self, escolha: &str) -> Result<ProvedorResolvido, LlmError> {
        let escolha = escolha.trim();
        if escolha.is_empty() || poph_types::llm_provider_accepted(escolha) {
            return Ok(self.padrao.clone());
        }
        if let Some(provedor) = self.nomeados.get(escolha) {
            return Ok(provedor.clone());
        }
        let declarados = self.nomes();
        let lista = if declarados.is_empty() {
            "nenhum provedor nomeado configurado".to_owned()
        } else {
            declarados.join(", ")
        };
        Err(LlmError::Configuracao(format!(
            "provedor `{escolha}` não existe nesta instância; use vazio (o padrão do [llm]) ou um de [provedores]: {lista}"
        )))
    }
}

#[cfg(test)]
mod testes {
    use poph_core::llm::{FakeProvider, FinishReason, Script};

    use super::*;

    fn fake(nome: &str) -> Arc<dyn LlmProvider> {
        Arc::new(FakeProvider::with_scripts(vec![Script::Events(vec![
            poph_core::llm::LlmEvent::Texto(nome.to_owned()),
            poph_core::llm::LlmEvent::Fim(FinishReason::Concluido),
        ])]))
    }

    fn registro() -> Provedores {
        let padrao = ProvedorResolvido::novo("antigravity2api", fake("padrao"), "modelo-padrao");
        let mut nomeados = BTreeMap::new();
        nomeados.insert(
            "openai".to_owned(),
            ProvedorResolvido::novo("openai", fake("openai"), "gpt-de-teste"),
        );
        Provedores::novo(padrao, nomeados)
    }

    #[test]
    fn vazio_e_nome_de_protocolo_caem_no_padrao() {
        let provedores = registro();
        for escolha in ["", "  ", "antigravity2api", "openai_compativel"] {
            let resolvido = provedores.resolver(escolha).expect("resolve no padrão");
            assert_eq!(resolvido.nome(), "antigravity2api");
            assert_eq!(resolvido.modelo(), "modelo-padrao");
        }
    }

    #[test]
    fn nome_declarado_devolve_o_cliente_dele() {
        let provedores = registro();
        let resolvido = provedores.resolver("openai").expect("resolve o nomeado");
        assert_eq!(resolvido.nome(), "openai");
        assert_eq!(resolvido.modelo(), "gpt-de-teste");
    }

    #[test]
    fn nome_desconhecido_e_erro_claro_com_os_disponiveis() {
        let provedores = registro();
        let erro = provedores.resolver("inexistente").expect_err("recusa");
        let texto = erro.to_string();
        assert!(texto.contains("inexistente"), "{texto}");
        assert!(texto.contains("openai"), "{texto}");
    }
}

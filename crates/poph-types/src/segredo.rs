//! Valor sensível (chave de API, chave de cifra, conteúdo de mensagem do cliente).
//!
//! `Debug` e `Display` nunca mostram o valor, e o tipo não implementa `Serialize`: para
//! sair daqui o valor precisa ser pedido de forma explícita com [`Sensivel::expor`].
//! Assim um `{:?}` num log ou num relatório não vaza o conteúdo por engano.

use std::fmt;

/// Texto que não pode aparecer em log, relatório ou resposta da API.
pub type Segredo = Sensivel<String>;

/// Envolve um valor e o oculta em `Debug` e `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct Sensivel<T>(T);

impl<T> Sensivel<T> {
    /// Guarda o valor sensível.
    pub fn new(valor: T) -> Self {
        Self(valor)
    }

    /// Acesso explícito ao valor. Use só onde ele é de fato necessário (ex.: montar um cabeçalho de API).
    pub fn expor(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Sensivel<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[oculto]")
    }
}

impl<T> fmt::Display for Sensivel<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[oculto]")
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn debug_e_display_nao_mostram_o_valor() {
        let s = Segredo::new("valor-muito-secreto-123".to_owned());
        assert_eq!(format!("{s:?}"), "[oculto]");
        assert_eq!(format!("{s}"), "[oculto]");
        assert!(!format!("{s:?} {s} {s:#?}").contains("muito-secreto"));
    }

    #[test]
    fn expor_devolve_o_valor_original() {
        let s = Segredo::new("abc".to_owned());
        assert_eq!(s.expor(), "abc");
    }
}

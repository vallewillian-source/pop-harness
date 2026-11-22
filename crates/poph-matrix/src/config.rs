//! Configuração e segredos do módulo Matrix (MX-07).
//!
//! A seção `[matrix]` é validada por `poph_types::HarnessConfig`; aqui ela é **decidida**: com a
//! seção ausente ou `habilitado = false`, [`ModuloMatrix::preparar`] devolve `None` e nada do módulo
//! existe. Só quando está ligada os dois tokens do Application Service são lidos do ambiente.
//!
//! Regra do segredo: os tokens vivem em [`SegredosMatrix`], no tipo `Sensivel`, e nunca aparecem em
//! `Debug`, `Display`, log, erro ou resposta. Para usá-los (montar o cabeçalho `Authorization` na
//! MX-09) é preciso pedir o valor com `expor()`; não existe outro caminho.

use poph_types::{DomainError, MatrixConfig, Segredo, Sensivel};

/// Token que o Harness usa para falar com o Synapse como Application Service.
pub const VAR_AS_TOKEN: &str = "POPH_MATRIX_AS_TOKEN";
/// Token que o Synapse apresenta ao chamar a rota de transações do Harness.
pub const VAR_HS_TOKEN: &str = "POPH_MATRIX_HS_TOKEN";
/// Token da API administrativa do Synapse, usado só na fase 2 (apagar mídia expirada).
///
/// Documentado aqui de propósito e **sem uso nesta fase**: a MX-46 é que o lê, e só quando a mídia
/// entrar em operação.
pub const VAR_ADMIN_TOKEN: &str = "POPH_MATRIX_ADMIN_TOKEN";

/// Resultado das operações do módulo.
pub type ResultadoMatrix<T> = Result<T, ErroMatrix>;

/// Erro de preparação do módulo. Nenhuma variante carrega o valor do token nem o da configuração.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErroMatrix {
    /// A seção `[matrix]` está inválida (o motivo nomeia a chave, nunca o valor).
    #[error(transparent)]
    Configuracao(#[from] DomainError),
    /// Variável de segredo ausente ou vazia com o módulo ligado.
    #[error(
        "{variavel}: defina a variável de ambiente (o módulo Matrix está ligado em `matrix.habilitado`)"
    )]
    TokenAusente { variavel: &'static str },
}

/// Os dois tokens do Application Service, lidos do ambiente.
///
/// `Debug` mostra `[oculto]` em cada campo (comportamento do `Sensivel`); o tipo não é `Serialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegredosMatrix {
    /// `POPH_MATRIX_AS_TOKEN`: o Harness apresenta ao Synapse.
    pub as_token: Segredo,
    /// `POPH_MATRIX_HS_TOKEN`: o Synapse apresenta ao Harness.
    pub hs_token: Segredo,
}

impl SegredosMatrix {
    /// Lê os dois tokens do ambiente do processo.
    ///
    /// Variável ausente, vazia ou só com espaços conta como ausente.
    pub fn do_ambiente() -> ResultadoMatrix<Self> {
        Self::de(|nome| std::env::var(nome).ok())
    }

    /// Lê os dois tokens de uma fonte qualquer (o ambiente é o caso real; os testes passam um mapa).
    pub fn de(buscar: impl Fn(&str) -> Option<String>) -> ResultadoMatrix<Self> {
        Ok(Self {
            as_token: token(buscar(VAR_AS_TOKEN), VAR_AS_TOKEN)?,
            hs_token: token(buscar(VAR_HS_TOKEN), VAR_HS_TOKEN)?,
        })
    }
}

fn token(valor: Option<String>, variavel: &'static str) -> ResultadoMatrix<Segredo> {
    match valor {
        Some(valor) if !valor.trim().is_empty() => Ok(Sensivel::new(valor)),
        _ => Err(ErroMatrix::TokenAusente { variavel }),
    }
}

/// O módulo pronto para ligar: a seção validada e os segredos.
///
/// `Debug` imprime a configuração (que não tem segredo) e `[oculto]` nos dois tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuloMatrix {
    /// Seção `[matrix]` já validada.
    pub config: MatrixConfig,
    /// Tokens do Application Service.
    pub segredos: SegredosMatrix,
}

impl ModuloMatrix {
    /// Decide se o módulo existe nesta instância e, se existir, carrega os segredos.
    ///
    /// - seção ausente (`None`) → `None`, sem tocar no ambiente;
    /// - `habilitado = false` → `None`, sem tocar no ambiente;
    /// - `habilitado = true` → confere a seção e lê os dois tokens; falta de token é erro que nomeia a
    ///   variável, nunca o valor.
    pub fn preparar(secao: Option<MatrixConfig>) -> ResultadoMatrix<Option<Self>> {
        Self::preparar_com(secao, |nome| std::env::var(nome).ok())
    }

    /// Igual a [`ModuloMatrix::preparar`], com a fonte dos tokens injetada (testes).
    pub fn preparar_com(
        secao: Option<MatrixConfig>,
        buscar: impl Fn(&str) -> Option<String>,
    ) -> ResultadoMatrix<Option<Self>> {
        let Some(config) = secao else {
            return Ok(None);
        };
        if !config.habilitado {
            return Ok(None);
        }
        config.validar()?;
        let segredos = SegredosMatrix::de(buscar)?;
        Ok(Some(Self { config, segredos }))
    }
}

#[cfg(test)]
mod testes {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Valor-canário: se aparecer em qualquer saída, o teste acusa.
    const CANARIO: &str = "CANARIO-MATRIX-9f3a-TAO-SECRETO";

    fn ligada() -> MatrixConfig {
        MatrixConfig {
            habilitado: true,
            homeserver_url: "http://127.0.0.1:18008".to_owned(),
            server_name: "matrix-dev.local".to_owned(),
            ..Default::default()
        }
    }

    fn ambiente() -> BTreeMap<String, String> {
        BTreeMap::from([
            (VAR_AS_TOKEN.to_owned(), CANARIO.to_owned()),
            (VAR_HS_TOKEN.to_owned(), format!("hs-{CANARIO}")),
        ])
    }

    fn fonte(variaveis: BTreeMap<String, String>) -> impl Fn(&str) -> Option<String> {
        move |nome| variaveis.get(nome).cloned()
    }

    #[test]
    fn secao_ausente_deixa_o_modulo_morto_sem_ler_token() {
        let modulo = ModuloMatrix::preparar_com(None, |nome| {
            panic!("não deveria ler {nome} com a seção ausente")
        })
        .expect("seção ausente é válida");
        assert!(modulo.is_none());
    }

    #[test]
    fn desligada_deixa_o_modulo_morto_sem_ler_token() {
        let mut desligada = ligada();
        desligada.habilitado = false;
        let modulo = ModuloMatrix::preparar_com(Some(desligada), |nome| {
            panic!("não deveria ler {nome} com o módulo desligado")
        })
        .expect("módulo desligado é válido");
        assert!(modulo.is_none());
    }

    #[test]
    fn ligada_carrega_configuracao_e_os_dois_tokens() {
        let modulo = ModuloMatrix::preparar_com(Some(ligada()), fonte(ambiente()))
            .expect("configuração válida")
            .expect("módulo ligado");
        assert_eq!(modulo.config.prefixo, "ph");
        assert_eq!(modulo.segredos.as_token.expor(), CANARIO);
        assert_eq!(modulo.segredos.hs_token.expor(), &format!("hs-{CANARIO}"));
    }

    #[test]
    fn token_ausente_ou_vazio_nomeia_a_variavel_sem_o_valor() {
        for (as_token, hs_token, esperado) in [
            (None, Some("hs".to_owned()), VAR_AS_TOKEN),
            (Some("   ".to_owned()), Some("hs".to_owned()), VAR_AS_TOKEN),
            (Some("as".to_owned()), None, VAR_HS_TOKEN),
            (Some("as".to_owned()), Some(String::new()), VAR_HS_TOKEN),
        ] {
            let mut variaveis = BTreeMap::new();
            if let Some(valor) = &as_token {
                variaveis.insert(VAR_AS_TOKEN.to_owned(), valor.clone());
            }
            if let Some(valor) = &hs_token {
                variaveis.insert(VAR_HS_TOKEN.to_owned(), valor.clone());
            }
            let erro = ModuloMatrix::preparar_com(Some(ligada()), fonte(variaveis))
                .expect_err("token faltando deve recusar o módulo");
            let mensagem = erro.to_string();
            assert_eq!(erro, ErroMatrix::TokenAusente { variavel: esperado });
            assert!(
                mensagem.contains(esperado),
                "o erro deve nomear a variável: {mensagem}"
            );
            // O valor que por acaso veio no outro campo não pode aparecer no erro.
            for valor in [as_token, hs_token].into_iter().flatten() {
                if !valor.trim().is_empty() {
                    assert!(
                        !mensagem.contains(&valor),
                        "vazou valor no erro: {mensagem}"
                    );
                }
            }
        }
    }

    #[test]
    fn configuracao_invalida_recusa_o_modulo_citando_a_chave() {
        let mut invalida = ligada();
        invalida.prefixo = "PH!".to_owned();
        let erro = ModuloMatrix::preparar_com(Some(invalida), |_| None)
            .expect_err("prefixo inválido deve recusar o módulo");
        match &erro {
            ErroMatrix::Configuracao(DomainError::Validacao { campo, .. }) => {
                assert_eq!(*campo, "matrix.prefixo")
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
        assert!(!erro.to_string().contains("PH!"));
    }

    #[test]
    fn valor_canario_nunca_aparece_em_debug_display_ou_log() {
        let modulo = ModuloMatrix::preparar_com(Some(ligada()), fonte(ambiente()))
            .expect("configuração válida")
            .expect("módulo ligado");

        let debug = format!("{modulo:?}");
        assert!(debug.contains("[oculto]"), "debug sem máscara: {debug}");
        assert!(!debug.contains(CANARIO), "vazou em Debug: {debug}");
        let display = format!("{}", modulo.segredos.as_token);
        assert_eq!(display, "[oculto]");
        assert!(!format!("{:?}", modulo.segredos).contains(CANARIO));

        // Log de verdade: um assinante que grava tudo o que for emitido.
        let capturado = Arc::new(Mutex::new(Vec::<u8>::new()));
        let assinante = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer({
                let capturado = Arc::clone(&capturado);
                move || Escritor(Arc::clone(&capturado))
            })
            .finish();
        tracing::subscriber::with_default(assinante, || {
            tracing::info!(
                as_token = ?modulo.segredos.as_token,
                hs_token = %modulo.segredos.hs_token,
                "módulo Matrix pronto"
            );
        });
        let log = String::from_utf8(capturado.lock().expect("trava").clone()).expect("utf-8");
        assert!(log.contains("módulo Matrix pronto"), "log não saiu: {log}");
        assert!(!log.contains(CANARIO), "vazou no log: {log}");
    }

    /// Escritor de log em memória para o teste.
    struct Escritor(Arc<Mutex<Vec<u8>>>);

    impl Write for Escritor {
        fn write(&mut self, dados: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trava").extend_from_slice(dados);
            Ok(dados.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

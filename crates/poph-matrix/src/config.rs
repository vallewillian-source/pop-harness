use poph_types::{DomainError, MatrixConfig, Secret, Sensitive};

pub const VAR_AS_TOKEN: &str = "POPH_MATRIX_AS_TOKEN";
pub const VAR_HS_TOKEN: &str = "POPH_MATRIX_HS_TOKEN";
pub const VAR_ADMIN_TOKEN: &str = "POPH_MATRIX_ADMIN_TOKEN";

pub type MatrixResult<T> = Result<T, MatrixError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MatrixError {
    #[error(transparent)]
    Configuration(#[from] DomainError),
    #[error(
        "{variable}: defina a variável de ambiente (o módulo Matrix está ligado em `matrix.habilitado`)"
    )]
    TokenMissing { variable: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixSecrets {
    pub as_token: Secret,
    pub hs_token: Secret,
}

impl MatrixSecrets {
    pub fn from_environment() -> MatrixResult<Self> {
        Self::of(|name| std::env::var(name).ok())
    }

    pub fn of(search: impl Fn(&str) -> Option<String>) -> MatrixResult<Self> {
        Ok(Self {
            as_token: token(search(VAR_AS_TOKEN), VAR_AS_TOKEN)?,
            hs_token: token(search(VAR_HS_TOKEN), VAR_HS_TOKEN)?,
        })
    }
}

pub fn environment_admin_token() -> Option<Secret> {
    admin_token_of(|name| std::env::var(name).ok())
}

pub fn admin_token_of(search: impl Fn(&str) -> Option<String>) -> Option<Secret> {
    search(VAR_ADMIN_TOKEN)
        .filter(|value| !value.trim().is_empty())
        .map(Sensitive::new)
}

fn token(value: Option<String>, variable: &'static str) -> MatrixResult<Secret> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(Sensitive::new(value)),
        _ => Err(MatrixError::TokenMissing { variable }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixModule {
    pub config: MatrixConfig,
    pub secrets: MatrixSecrets,
}

impl MatrixModule {
    pub fn prepare(section: Option<MatrixConfig>) -> MatrixResult<Option<Self>> {
        Self::prepare_with(section, |name| std::env::var(name).ok())
    }

    pub fn prepare_with(
        section: Option<MatrixConfig>,
        search: impl Fn(&str) -> Option<String>,
    ) -> MatrixResult<Option<Self>> {
        let Some(config) = section else {
            return Ok(None);
        };
        if !config.enabled {
            return Ok(None);
        }
        config.validar()?;
        let secrets = MatrixSecrets::of(search)?;
        Ok(Some(Self { config, secrets }))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::*;

    const CANARIO: &str = "CANARIO-MATRIX-9f3a-TAO-SECRETO";

    fn linked() -> MatrixConfig {
        MatrixConfig {
            enabled: true,
            homeserver_url: "http://127.0.0.1:18008".to_owned(),
            server_name: "matrix-dev.local".to_owned(),
            ..Default::default()
        }
    }

    fn environment() -> BTreeMap<String, String> {
        BTreeMap::from([
            (VAR_AS_TOKEN.to_owned(), CANARIO.to_owned()),
            (VAR_HS_TOKEN.to_owned(), format!("hs-{CANARIO}")),
        ])
    }

    fn source(variables: BTreeMap<String, String>) -> impl Fn(&str) -> Option<String> {
        move |name| variables.get(name).cloned()
    }

    #[test]
    fn missing_section_leaves_module_dead_without_reading_token() {
        let module = MatrixModule::prepare_with(None, |name| {
            panic!("não deveria ler {name} com a seção ausente")
        })
        .expect("seção ausente é válida");
        assert!(module.is_none());
    }

    #[test]
    fn disabled_leaves_module_dead_without_reading_token() {
        let mut disabled = linked();
        disabled.enabled = false;
        let module = MatrixModule::prepare_with(Some(disabled), |name| {
            panic!("não deveria ler {name} com o módulo desligado")
        })
        .expect("módulo desligado é válido");
        assert!(module.is_none());
    }

    #[test]
    fn enabled_loads_configuration_and_both_tokens() {
        let module = MatrixModule::prepare_with(Some(linked()), source(environment()))
            .expect("configuração válida")
            .expect("módulo ligado");
        assert_eq!(module.config.prefix, "ph");
        assert_eq!(module.secrets.as_token.expose(), CANARIO);
        assert_eq!(module.secrets.hs_token.expose(), &format!("hs-{CANARIO}"));
    }

    #[test]
    fn absent_or_empty_token_names_variable_without_value() {
        for (as_token, hs_token, expected) in [
            (None, Some("hs".to_owned()), VAR_AS_TOKEN),
            (Some("   ".to_owned()), Some("hs".to_owned()), VAR_AS_TOKEN),
            (Some("as".to_owned()), None, VAR_HS_TOKEN),
            (Some("as".to_owned()), Some(String::new()), VAR_HS_TOKEN),
        ] {
            let mut variables = BTreeMap::new();
            if let Some(value) = &as_token {
                variables.insert(VAR_AS_TOKEN.to_owned(), value.clone());
            }
            if let Some(value) = &hs_token {
                variables.insert(VAR_HS_TOKEN.to_owned(), value.clone());
            }
            let error = MatrixModule::prepare_with(Some(linked()), source(variables))
                .expect_err("token faltando deve recusar o módulo");
            let message = error.to_string();
            assert_eq!(error, MatrixError::TokenMissing { variable: expected });
            assert!(
                message.contains(expected),
                "o erro deve nomear a variável: {message}"
            );
            for value in [as_token, hs_token].into_iter().flatten() {
                if !value.trim().is_empty() {
                    assert!(!message.contains(&value), "vazou valor no erro: {message}");
                }
            }
        }
    }

    #[test]
    fn invalid_configuration_refuses_module_naming_key() {
        let mut invalid = linked();
        invalid.prefix = "PH!".to_owned();
        let error = MatrixModule::prepare_with(Some(invalid), |_| None)
            .expect_err("prefixo inválido deve recusar o módulo");
        match &error {
            MatrixError::Configuration(DomainError::Validation { field, .. }) => {
                assert_eq!(*field, "matrix.prefixo")
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
        assert!(!error.to_string().contains("PH!"));
    }

    #[test]
    fn canary_value_never_appears_in_debug_display_or_log() {
        let module = MatrixModule::prepare_with(Some(linked()), source(environment()))
            .expect("configuração válida")
            .expect("módulo ligado");

        let debug = format!("{module:?}");
        assert!(debug.contains("[oculto]"), "debug sem máscara: {debug}");
        assert!(!debug.contains(CANARIO), "vazou em Debug: {debug}");
        let display = format!("{}", module.secrets.as_token);
        assert_eq!(display, "[oculto]");
        assert!(!format!("{:?}", module.secrets).contains(CANARIO));

        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let assinante = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer({
                let captured = Arc::clone(&captured);
                move || Writer(Arc::clone(&captured))
            })
            .finish();
        tracing::subscriber::with_default(assinante, || {
            tracing::info!(
                as_token = ?module.secrets.as_token,
                hs_token = %module.secrets.hs_token,
                "módulo Matrix pronto"
            );
        });
        let log = String::from_utf8(captured.lock().expect("trava").clone()).expect("utf-8");
        assert!(log.contains("módulo Matrix pronto"), "log não saiu: {log}");
        assert!(!log.contains(CANARIO), "vazou no log: {log}");
    }

    struct Writer(Arc<Mutex<Vec<u8>>>);

    impl Write for Writer {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trava").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

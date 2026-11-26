use serde_json::Value;

use crate::error::RetentionError;

pub const DAY_MS: i64 = 86_400_000;

pub const MAX_DAYS: u32 = 36_500;

pub fn days_of_org(org: &str, config_json: &str, default_days: u32) -> Result<u32, RetentionError> {
    let error = |motivo: String| RetentionError::InvalidConfig {
        org: org.to_owned(),
        motivo,
    };
    let texto = config_json.trim();
    if texto.is_empty() {
        return Ok(default_days);
    }
    let value: Value = serde_json::from_str(texto)
        .map_err(|e| error(format!("a configuração não é um JSON válido ({e})")))?;
    let Some(root) = value.as_object() else {
        return Err(error("a configuração não é um objeto JSON".to_owned()));
    };
    let Some(retencao) = root.get("retencao") else {
        return Ok(default_days);
    };
    let Some(retencao) = retencao.as_object() else {
        return Err(error("`retencao` deve ser um objeto".to_owned()));
    };
    let Some(dias) = retencao.get("dias") else {
        return Ok(default_days);
    };
    if let Some(negative) = dias.as_i64().filter(|n| *n < 0) {
        return Err(error(format!(
            "`retencao.dias` não pode ser negativo ({negative})"
        )));
    }
    let Some(n) = dias.as_u64() else {
        return Err(error(format!(
            "`retencao.dias` deve ser um número inteiro de 0 a {MAX_DAYS} (recebido: {dias})"
        )));
    };
    if n > u64::from(MAX_DAYS) {
        return Err(error(format!(
            "`retencao.dias` passa do máximo de {MAX_DAYS} dias (recebido: {n})"
        )));
    }
    Ok(n as u32)
}

pub fn cutoff_ms(now_ms: i64, dias: u32) -> Option<i64> {
    if dias == 0 {
        return None;
    }
    Some(now_ms.saturating_sub(i64::from(dias).saturating_mul(DAY_MS)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORG: &str = "org-teste";

    #[test]
    fn without_config_uses_default() {
        assert_eq!(days_of_org(ORG, "{}", 30), Ok(30));
        assert_eq!(days_of_org(ORG, "", 30), Ok(30));
        assert_eq!(days_of_org(ORG, r#"{"limites":{}}"#, 30), Ok(30));
        assert_eq!(days_of_org(ORG, r#"{"retencao":{}}"#, 30), Ok(30));
    }

    #[test]
    fn zero_means_never_delete() {
        assert_eq!(days_of_org(ORG, r#"{"retencao":{"dias":0}}"#, 30), Ok(0));
    }

    #[test]
    fn explicit_value_beats_default() {
        assert_eq!(days_of_org(ORG, r#"{"retencao":{"dias":90}}"#, 30), Ok(90));
    }

    #[test]
    fn invalid_value_is_error_never_deletes_all() {
        for bad in [
            r#"{"retencao":{"dias":-1}}"#,
            r#"{"retencao":{"dias":"30"}}"#,
            r#"{"retencao":{"dias":30.5}}"#,
            r#"{"retencao":{"dias":null}}"#,
            r#"{"retencao":{"dias":99999}}"#,
            r#"{"retencao":"30"}"#,
            r#"[1,2]"#,
            r#"{"retencao":"#,
        ] {
            let resultado = days_of_org(ORG, bad, 30);
            assert!(
                matches!(resultado, Err(RetentionError::InvalidConfig { .. })),
                "devia recusar {bad}, veio {resultado:?}"
            );
        }
    }

    #[test]
    fn negative_message_is_readable() {
        let error = days_of_org(ORG, r#"{"retencao":{"dias":-7}}"#, 30).unwrap_err();
        let texto = error.to_string();
        assert!(texto.contains("não pode ser negativo"), "{texto}");
        assert!(texto.contains("Nada foi apagado"), "{texto}");
    }

    #[test]
    fn cutoff_subtracts_days_and_zero_has_no_cutoff() {
        let now = 1_795_131_511_000;
        assert_eq!(cutoff_ms(now, 0), None);
        assert_eq!(cutoff_ms(now, 30), Some(now - 30 * DAY_MS));
        assert_eq!(cutoff_ms(0, 30), Some(-30 * DAY_MS));
    }
}

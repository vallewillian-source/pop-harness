//! Retenção de cada org, lida de `orgs.config_json` (chave `retencao.dias`).
//!
//! Regras:
//! - sem a chave, vale o padrão da configuração do processo (30 dias);
//! - `0` quer dizer "nunca apagar";
//! - qualquer valor que não seja inteiro entre 0 e [`DIAS_MAXIMOS`] é **erro**, nunca "apaga tudo".

use serde_json::Value;

use crate::erro::ErroRetencao;

/// Um dia em milissegundos.
pub const DIA_MS: i64 = 86_400_000;

/// Teto de dias aceito na configuração (100 anos). Acima disso é erro de digitação.
pub const DIAS_MAXIMOS: u32 = 36_500;

/// Lê `retencao.dias` de `config_json` da org. Devolve os dias (0 = não apagar).
pub fn dias_da_org(org: &str, config_json: &str, padrao_dias: u32) -> Result<u32, ErroRetencao> {
    let erro = |motivo: String| ErroRetencao::ConfigInvalida {
        org: org.to_owned(),
        motivo,
    };
    let texto = config_json.trim();
    if texto.is_empty() {
        return Ok(padrao_dias);
    }
    let valor: Value = serde_json::from_str(texto)
        .map_err(|e| erro(format!("a configuração não é um JSON válido ({e})")))?;
    let Some(raiz) = valor.as_object() else {
        return Err(erro("a configuração não é um objeto JSON".to_owned()));
    };
    let Some(retencao) = raiz.get("retencao") else {
        return Ok(padrao_dias);
    };
    let Some(retencao) = retencao.as_object() else {
        return Err(erro("`retencao` deve ser um objeto".to_owned()));
    };
    let Some(dias) = retencao.get("dias") else {
        return Ok(padrao_dias);
    };
    if let Some(negativo) = dias.as_i64().filter(|n| *n < 0) {
        return Err(erro(format!(
            "`retencao.dias` não pode ser negativo ({negativo})"
        )));
    }
    let Some(n) = dias.as_u64() else {
        return Err(erro(format!(
            "`retencao.dias` deve ser um número inteiro de 0 a {DIAS_MAXIMOS} (recebido: {dias})"
        )));
    };
    if n > u64::from(DIAS_MAXIMOS) {
        return Err(erro(format!(
            "`retencao.dias` passa do máximo de {DIAS_MAXIMOS} dias (recebido: {n})"
        )));
    }
    // Cabe em u32: o teste acima limita a DIAS_MAXIMOS.
    Ok(n as u32)
}

/// Instante de corte: o que é anterior a ele venceu. `None` quando a retenção é `0` (nunca apagar).
pub fn corte_ms(agora_ms: i64, dias: u32) -> Option<i64> {
    if dias == 0 {
        return None;
    }
    Some(agora_ms.saturating_sub(i64::from(dias).saturating_mul(DIA_MS)))
}

#[cfg(test)]
mod testes {
    use super::*;

    const ORG: &str = "org-teste";

    #[test]
    fn sem_configuracao_vale_o_padrao() {
        assert_eq!(dias_da_org(ORG, "{}", 30), Ok(30));
        assert_eq!(dias_da_org(ORG, "", 30), Ok(30));
        assert_eq!(dias_da_org(ORG, r#"{"limites":{}}"#, 30), Ok(30));
        assert_eq!(dias_da_org(ORG, r#"{"retencao":{}}"#, 30), Ok(30));
    }

    #[test]
    fn zero_quer_dizer_nunca_apagar() {
        assert_eq!(dias_da_org(ORG, r#"{"retencao":{"dias":0}}"#, 30), Ok(0));
    }

    #[test]
    fn valor_explicito_vence_o_padrao() {
        assert_eq!(dias_da_org(ORG, r#"{"retencao":{"dias":90}}"#, 30), Ok(90));
    }

    #[test]
    fn valor_invalido_e_erro_e_nunca_apaga_tudo() {
        for ruim in [
            r#"{"retencao":{"dias":-1}}"#,
            r#"{"retencao":{"dias":"30"}}"#,
            r#"{"retencao":{"dias":30.5}}"#,
            r#"{"retencao":{"dias":null}}"#,
            r#"{"retencao":{"dias":99999}}"#,
            r#"{"retencao":"30"}"#,
            r#"[1,2]"#,
            r#"{"retencao":"#,
        ] {
            let resultado = dias_da_org(ORG, ruim, 30);
            assert!(
                matches!(resultado, Err(ErroRetencao::ConfigInvalida { .. })),
                "devia recusar {ruim}, veio {resultado:?}"
            );
        }
    }

    #[test]
    fn mensagem_de_negativo_e_legivel() {
        let erro = dias_da_org(ORG, r#"{"retencao":{"dias":-7}}"#, 30).unwrap_err();
        let texto = erro.to_string();
        assert!(texto.contains("não pode ser negativo"), "{texto}");
        assert!(texto.contains("Nada foi apagado"), "{texto}");
    }

    #[test]
    fn corte_recua_os_dias_e_zero_nao_tem_corte() {
        let agora = 1_795_131_511_000;
        assert_eq!(corte_ms(agora, 0), None);
        assert_eq!(corte_ms(agora, 30), Some(agora - 30 * DIA_MS));
        assert_eq!(corte_ms(0, 30), Some(-30 * DIA_MS));
    }
}

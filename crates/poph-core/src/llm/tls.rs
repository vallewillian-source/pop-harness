//! Detecção de falha de TLS nos erros do `reqwest` (HR-10b).
//!
//! O cliente usa `rustls` com o verificador de raízes do sistema (a feature `rustls` do `reqwest`).
//! Quando o certificado do servidor não é confiável, o nome não bate ou o handshake é recusado, o
//! `reqwest` devolve um erro de conexão — o mesmo tipo de erro de "porta fechada". Só o texto da
//! cadeia de causas distingue os dois casos.
//!
//! Regra: falha de TLS é **permanente** (repetir a chamada não conserta certificado), enquanto queda
//! de rede e prazo continuam transitórios. Por isso esta checagem vem antes da classificação de rede
//! em [`super::antigravity`].
//!
//! A detecção é textual de propósito: o `rustls` não é dependência deste crate (só do `reqwest`), e
//! os marcadores usados são os nomes das falhas do `rustls`/`webpki` que aparecem na cadeia. Erro de
//! TLS que não bater em nenhum marcador é tratado como falha de rede transitória — o pior caso é
//! repetir uma chamada que vai falhar de novo, nunca o contrário.

/// Marcadores (em minúsculas) de falha de TLS na cadeia de causas do erro.
const MARCAS: [&str; 10] = [
    "certificate",
    "certificado",
    "handshake",
    "tls",
    "unknownissuer",
    "notvalidfor",
    "invalid peer",
    "peer certificate",
    "invalidcertificate",
    "webpki",
];

/// Teto do texto devolvido, para a mensagem de erro continuar legível.
const TETO: usize = 200;

/// Devolve o detalhe legível quando o erro é de TLS; `None` quando é outra falha.
///
/// O texto escolhido é o da causa mais interna que traz um marcador, limitado a [`TETO`] caracteres
/// e sem URL (o `reqwest` já remove o endereço antes de chegar aqui, mas a limpeza é barata).
pub fn detalhe_de_falha_tls(erro: &dyn std::error::Error) -> Option<String> {
    let mut causas = Vec::new();
    let mut atual = Some(erro);
    while let Some(causa) = atual {
        causas.push(causa.to_string());
        atual = causa.source();
    }
    let marcada = |texto: &String| {
        let baixo = texto.to_ascii_lowercase();
        MARCAS.iter().any(|marca| baixo.contains(marca))
    };
    if !causas.iter().any(marcada) {
        return None;
    }
    let detalhe = causas
        .iter()
        .rev()
        .find(|texto| marcada(texto))
        .map(String::as_str)
        .unwrap_or("handshake TLS recusado");
    Some(limpar(detalhe))
}

/// Corta o texto no teto e tira quebras de linha (mensagem vai para log e `Display`).
fn limpar(texto: &str) -> String {
    let sem_quebra: String = texto
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    sem_quebra.trim().chars().take(TETO).collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    struct Causa(String);

    #[derive(Debug, thiserror::Error)]
    #[error("error sending request")]
    struct Envolucro(#[source] Causa);

    fn detalhe(erro: &dyn std::error::Error) -> Option<String> {
        detalhe_de_falha_tls(erro)
    }

    #[test]
    fn reconhece_certificado_nao_confiavel_na_cadeia() {
        let erro = Envolucro(Causa("invalid peer certificate: UnknownIssuer".to_owned()));
        let texto = detalhe(&erro).expect("é falha de TLS");
        assert!(texto.contains("UnknownIssuer"), "texto: {texto}");
        assert!(!texto.contains("error sending request"));
    }

    #[test]
    fn reconhece_nome_do_servidor_errado() {
        let erro = Envolucro(Causa(
            "received fatal alert: HandshakeFailure (NotValidForName)".to_owned(),
        ));
        assert!(detalhe(&erro).is_some());
    }

    #[test]
    fn queda_de_rede_comum_nao_e_tls() {
        let erro = Envolucro(Causa("Connection refused (os error 111)".to_owned()));
        assert!(detalhe(&erro).is_none());
    }

    #[test]
    fn texto_longo_e_cortado_no_teto() {
        let longo = "certificate ".repeat(100);
        let texto = detalhe(&Envolucro(Causa(longo))).expect("é falha de TLS");
        assert_eq!(texto.chars().count(), TETO);
    }
}

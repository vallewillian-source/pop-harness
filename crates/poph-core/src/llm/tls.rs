const MARKERS: [&str; 10] = [
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

const MAX_DETAIL_LENGTH: usize = 200;

pub fn tls_failure_detail(error: &dyn std::error::Error) -> Option<String> {
    let mut causes = Vec::new();
    let mut current = Some(error);
    while let Some(cause) = current {
        causes.push(cause.to_string());
        current = cause.source();
    }
    let has_marker = |text: &String| {
        let lowercase = text.to_ascii_lowercase();
        MARKERS.iter().any(|marker| lowercase.contains(marker))
    };
    if !causes.iter().any(has_marker) {
        return None;
    }
    let detail = causes
        .iter()
        .rev()
        .find(|text| has_marker(text))
        .map(String::as_str)
        .unwrap_or("handshake TLS recusado");
    Some(clean(detail))
}

fn clean(text: &str) -> String {
    let single_line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    single_line.trim().chars().take(MAX_DETAIL_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    struct Cause(String);

    #[derive(Debug, thiserror::Error)]
    #[error("error sending request")]
    struct Wrapper(#[source] Cause);

    fn detail(error: &dyn std::error::Error) -> Option<String> {
        tls_failure_detail(error)
    }

    #[test]
    fn detects_untrusted_certificate_in_chain() {
        let error = Wrapper(Cause("invalid peer certificate: UnknownIssuer".to_owned()));
        let text = detail(&error).expect("é falha de TLS");
        assert!(text.contains("UnknownIssuer"), "texto: {text}");
        assert!(!text.contains("error sending request"));
    }

    #[test]
    fn detects_wrong_server_name() {
        let error = Wrapper(Cause(
            "received fatal alert: HandshakeFailure (NotValidForName)".to_owned(),
        ));
        assert!(detail(&error).is_some());
    }

    #[test]
    fn ordinary_network_failure_is_not_tls() {
        let error = Wrapper(Cause("Connection refused (os error 111)".to_owned()));
        assert!(detail(&error).is_none());
    }

    #[test]
    fn long_text_is_truncated_at_limit() {
        let long = "certificate ".repeat(100);
        let text = detail(&Wrapper(Cause(long))).expect("é falha de TLS");
        assert_eq!(text.chars().count(), MAX_DETAIL_LENGTH);
    }
}

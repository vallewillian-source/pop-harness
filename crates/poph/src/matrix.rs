use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_matrix::{MatrixSecrets, VAR_AS_TOKEN, VAR_HS_TOKEN};
use poph_types::MatrixConfig;

use crate::config;
use crate::matrix_admin::{self, OutputArgs, SpaceArgs};

pub const DEFAULT_ID: &str = "poph-matrix";

pub const VAR_URL_AS: &str = "POPH_MATRIX_URL_AS";

pub const SHARED_ALIAS: &str = "#dep_.*";

#[derive(Debug, Subcommand)]
pub enum MatrixAction {
    #[command(
        name = "registro",
        about = "Gera (`--saida`) ou confere (`--verificar`) o registro do Application Service"
    )]
    Registration(RegistrationArgs),
    #[command(
        name = "estado",
        about = "Estado do módulo na org: ligado, corte do backfill, atraso, falhas (`--offline --org`)"
    )]
    Status(OutputArgs),
    #[command(
        name = "ligar",
        about = "Liga o módulo na org; grava o corte do backfill na primeira vez (`--offline --org`)"
    )]
    Enable(OutputArgs),
    #[command(
        name = "desligar",
        about = "Desliga o módulo na org; preserva o corte e não apaga nada no Element (`--offline --org`)"
    )]
    Disable(OutputArgs),
    #[command(
        name = "espaco",
        about = "Espaços por departamento: lista, cria ou liga (`--departamento`) e `desligar` (`--offline --org`)"
    )]
    Space(SpaceArgs),
}

#[derive(Debug, Args)]
pub struct RegistrationArgs {
    #[arg(
        long = "saida",
        value_name = "ARQUIVO",
        help = "Arquivo a gerar, com permissão 0600. Recusa sobrescrever sem `--forcar`"
    )]
    pub output: Option<PathBuf>,

    #[arg(
        long = "verificar",
        value_name = "ARQUIVO",
        help = "Arquivo existente a conferir contra a seção `[matrix]` e os tokens do ambiente"
    )]
    pub verify: Option<PathBuf>,

    #[arg(
        long,
        value_name = "BASE",
        help = "Endereço por onde o Synapse alcança o Harness (padrão: variável `POPH_MATRIX_URL_AS`)"
    )]
    pub url: Option<String>,

    #[arg(long, value_name = "ID", default_value = DEFAULT_ID, help = "Id do Application Service no registro (muda por instância na fase 3)")]
    pub id: String,

    #[arg(long = "forcar", help = "Sobrescreve o arquivo existente")]
    pub force: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    action: MatrixAction,
) -> ExitCode {
    match action {
        MatrixAction::Registration(args) => run_registration(flag, environment, args),
        MatrixAction::Status(args) => matrix_admin::state(flag, environment, offline, org, &args),
        MatrixAction::Enable(args) => {
            matrix_admin::set_enabled(flag, environment, offline, org, true, &args)
        }
        MatrixAction::Disable(args) => {
            matrix_admin::set_enabled(flag, environment, offline, org, false, &args)
        }
        MatrixAction::Space(args) => matrix_admin::space(flag, environment, offline, org, &args),
    }
}

fn run_registration(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    args: RegistrationArgs,
) -> ExitCode {
    match (args.output.as_deref(), args.verify.as_deref()) {
        (Some(_), Some(_)) => error("use --saida (gerar) ou --verificar (conferir), não os dois"),
        (None, None) => {
            error("informe --saida <arquivo> para gerar ou --verificar <arquivo> para conferir")
        }
        (Some(target), None) => generate(flag, environment, &args, target),
        (None, Some(file)) => check(flag, environment, &args, file),
    }
}

fn generate(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    args: &RegistrationArgs,
    target: &Path,
) -> ExitCode {
    let (config, secrets) = match read_environment(flag, environment) {
        Ok(pair) => pair,
        Err(message) => return error(&message),
    };
    let url = match registration_url(args, environment, &config) {
        Ok(url) => url,
        Err(message) => return error(&message),
    };

    let text = yaml(&args.id, &url, &config.prefix, &secrets);

    if let Err(message) = write_registration(target, &text, args.force) {
        return error(&message);
    }

    println!(
        "registro do Application Service gravado: {}",
        target.display()
    );
    println!("  id:               {}", args.id);
    println!("  url:              {url}");
    println!("  sender_localpart: {}", sender_localpart(&config.prefix));
    println!(
        "  namespaces:       @{}_.* e #{}_.* (exclusivos), {SHARED_ALIAS} (compartilhado)",
        config.prefix, config.prefix
    );
    println!("  rate_limited:     false");
    println!("  as_token:         [oculto]");
    println!("  hs_token:         [oculto]");
    println!("  permissão:        0600");
    println!();
    println!(
        "O Synapse só lê registros na subida: reinicie o Synapse deste ambiente para carregá-lo."
    );
    ExitCode::SUCCESS
}

fn check(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    args: &RegistrationArgs,
    file: &Path,
) -> ExitCode {
    let (config, secrets) = match read_environment(flag, environment) {
        Ok(pair) => pair,
        Err(message) => return error(&message),
    };
    let url = match registration_url(args, environment, &config) {
        Ok(url) => url,
        Err(message) => return error(&message),
    };
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(reason) => {
            return error(&format!(
                "não consegui ler o registro {}: {reason}",
                file.display()
            ));
        }
    };
    let parsed = match parse_registration(&text) {
        Ok(parsed) => parsed,
        Err(message) => {
            return error(&format!("{}: {message}", file.display()));
        }
    };

    let expected = ExpectedRegistration {
        url: &url,
        prefix: &config.prefix,
        secrets: &secrets,
    };
    let divergences = compare(&parsed, &expected);
    if !divergences.is_empty() {
        eprintln!(
            "{}: registro divergente da configuração atual:",
            file.display()
        );
        for divergence in &divergences {
            eprintln!("  {divergence}");
        }
        eprintln!(
            "\nGere de novo com `poph matrix registro --saida <arquivo> --forcar` ou ajuste a seção [matrix]."
        );
        return ExitCode::from(1);
    }

    println!("registro em dia: {}", file.display());
    println!("  url:              {url}");
    println!("  sender_localpart: {}", sender_localpart(&config.prefix));
    println!("  rate_limited:     false");
    println!("  tokens:           conferem com o ambiente ([oculto])");
    ExitCode::SUCCESS
}

fn read_environment(
    flag: Option<PathBuf>,
    environment: &config::Environment,
) -> Result<(MatrixConfig, MatrixSecrets), String> {
    let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &directory);
    let config = config::load(file.as_deref(), environment).map_err(|error| error.to_string())?;
    let Some(section) = config.matrix.as_ref() else {
        return Err(
            "seção [matrix] ausente na configuração: declare-a no poph.toml (ou por --config) antes de \
             gerar o registro"
                .to_owned(),
        );
    };

    let mut effective = section.clone();
    effective.enabled = true;
    effective.validar().map_err(|error| error.to_string())?;
    let secrets = MatrixSecrets::of(|name| environment.get(name).cloned())
        .map_err(|error| error.to_string())?;
    Ok((effective, secrets))
}

fn registration_url(
    args: &RegistrationArgs,
    environment: &config::Environment,
    _config: &MatrixConfig,
) -> Result<String, String> {
    let raw = args
        .url
        .clone()
        .or_else(|| environment.get(VAR_URL_AS).cloned())
        .unwrap_or_default();
    let url = raw.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err(format!(
            "informe a url do Harness com --url (ou defina {VAR_URL_AS}): é o endereço por onde o \
             Synapse alcança o Harness, ex.: http://192.168.0.123:8080"
        ));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(
            "campo url: use http:// ou https:// no começo (endereço por onde o Synapse alcança o \
             Harness)"
                .to_owned(),
        );
    }
    Ok(url.to_owned())
}

pub fn sender_localpart(prefix: &str) -> String {
    format!("{prefix}_as_bot")
}

pub fn yaml(id: &str, url: &str, prefix: &str, secrets: &MatrixSecrets) -> String {
    let mut text = String::new();
    text.push_str(
        "# ═══════════════════════════════════════════════════════════════════════════\n\
         # Matrix Application Service do POP Harness — ARQUIVO GERADO, NUNCA VERSIONADO.\n\
         #\n\
         # Gerado por `poph matrix registro --saida <arquivo>` (MX-21). Contém os dois tokens do\n\
         # Application Service: guarde com permissão 0600 e não copie para o repositório.\n\
         # O Synapse só lê registros na subida: reinicie o Synapse para carregar alterações.\n\
         # ═══════════════════════════════════════════════════════════════════════════\n",
    );
    text.push_str(&format!("id: {}\n", quote_scalar(id)));
    text.push_str(&format!("url: {}\n", quote_scalar(url)));
    text.push_str(&format!(
        "as_token: {}\n",
        quote_scalar(secrets.as_token.expose())
    ));
    text.push_str(&format!(
        "hs_token: {}\n",
        quote_scalar(secrets.hs_token.expose())
    ));

    text.push_str("rate_limited: false\n");
    text.push_str(&format!(
        "sender_localpart: {}\n",
        quote_scalar(&sender_localpart(prefix))
    ));
    text.push_str("namespaces:\n");
    text.push_str("  users:\n");
    text.push_str("    - exclusive: true\n");
    text.push_str(&format!(
        "      regex: {}\n",
        quote_scalar(&format!("@{prefix}_.*"))
    ));
    text.push_str("  aliases:\n");
    text.push_str("    - exclusive: true\n");
    text.push_str(&format!(
        "      regex: {}\n",
        quote_scalar(&format!("#{prefix}_.*"))
    ));
    text.push_str("    - exclusive: false\n");
    text.push_str(&format!("      regex: {}\n", quote_scalar(SHARED_ALIAS)));
    text.push_str("  rooms: []\n");
    text
}

fn quote_scalar(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other => output.push(other),
        }
    }
    output.push('"');
    output
}

fn write_registration(target: &Path, text: &str, force: bool) -> Result<(), String> {
    if target.exists() && !force {
        return Err(format!(
            "o arquivo {} já existe: use --forcar para sobrescrever",
            target.display()
        ));
    }
    let mut file = open_0600(target, force).map_err(|error| {
        format!(
            "não consegui criar o registro {}: {error}",
            target.display()
        )
    })?;
    file.write_all(text.as_bytes()).map_err(|error| {
        format!(
            "não consegui escrever o registro {}: {error}",
            target.display()
        )
    })?;
    file.flush().map_err(|error| {
        format!(
            "não consegui terminar de escrever o registro {}: {error}",
            target.display()
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn open_0600(target: &Path, force: bool) -> std::io::Result<fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut options = fs::OpenOptions::new();
    options.write(true).mode(0o600);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let file = options.open(target)?;

    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_0600(target: &Path, force: bool) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceItem {
    pub exclusive: bool,

    pub regex: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedRegistration {
    pub scalars: BTreeMap<String, String>,

    pub users: Vec<NamespaceItem>,

    pub aliases: Vec<NamespaceItem>,
}

pub fn parse_registration(text: &str) -> Result<ParsedRegistration, String> {
    #[derive(Default)]
    struct ReaderState {
        in_namespaces: bool,
        list: Option<&'static str>,
        item: Option<NamespaceItem>,
        exclusive_seen: bool,
    }

    fn finish_item(parsed: &mut ParsedRegistration, state: &mut ReaderState) -> Result<(), String> {
        let Some(item) = state.item.take() else {
            return Ok(());
        };
        if !state.exclusive_seen || item.regex.is_empty() {
            return Err(format!(
                "namespaces.{}: item sem `exclusive` e `regex`",
                state.list.unwrap_or("?")
            ));
        }
        match state.list {
            Some("users") => parsed.users.push(item),
            Some("aliases") => parsed.aliases.push(item),
            _ => {}
        }
        state.exclusive_seen = false;
        Ok(())
    }

    let mut parsed = ParsedRegistration::default();
    let mut state = ReaderState::default();

    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        let content = line.trim();
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let location = format!("linha {}", number + 1);

        if indent == 0 {
            finish_item(&mut parsed, &mut state)?;
            let Some((key, value)) = content.split_once(':') else {
                return Err(format!(
                    "{location}: esperava `chave: valor`, achei `{content}`"
                ));
            };
            let key = key.trim();
            let value = value.trim();
            if key == "namespaces" {
                if !value.is_empty() {
                    return Err(format!("{location}: `namespaces` não é escalar"));
                }
                state.in_namespaces = true;
                state.list = None;
                continue;
            }
            if value.is_empty() {
                return Err(format!("{location}: campo `{key}` sem valor"));
            }
            parsed
                .scalars
                .insert(key.to_owned(), unquote(value).to_owned());
            continue;
        }

        if !state.in_namespaces {
            return Err(format!("{location}: linha recuada fora de `namespaces`"));
        }

        if indent == 2 {
            finish_item(&mut parsed, &mut state)?;
            let Some((key, _)) = content.split_once(':') else {
                return Err(format!(
                    "{location}: esperava `users:`, `aliases:` ou `rooms:`"
                ));
            };
            state.list = match key.trim() {
                "users" => Some("users"),
                "aliases" => Some("aliases"),
                "rooms" => None,
                other => {
                    return Err(format!(
                        "{location}: namespaces desconhecido `{other}` (esperava users, aliases ou rooms)"
                    ));
                }
            };
            continue;
        }

        if state.list.is_none() {
            return Err(format!(
                "{location}: item em `namespaces.rooms` não esperado"
            ));
        }

        if let Some(rest) = content.strip_prefix('-') {
            finish_item(&mut parsed, &mut state)?;
            let rest = rest.trim();
            if rest.is_empty() {
                state.item = Some(NamespaceItem {
                    exclusive: false,
                    regex: String::new(),
                });
                continue;
            }
            let Some((key, value)) = rest.split_once(':') else {
                return Err(format!("{location}: esperava `- exclusive: true|false`"));
            };
            let key = key.trim();
            let value = unquote(value.trim());
            let mut new = NamespaceItem {
                exclusive: false,
                regex: String::new(),
            };
            match key {
                "exclusive" => {
                    new.exclusive = parse_exclusivity(key, value, &location)?;
                    state.exclusive_seen = true;
                }
                "regex" => new.regex = value.to_owned(),
                other => {
                    return Err(format!(
                        "{location}: chave desconhecida no item: `{other}` (esperava exclusive ou regex)"
                    ));
                }
            }
            state.item = Some(new);
            continue;
        }

        let Some((key, value)) = content.split_once(':') else {
            return Err(format!("{location}: esperava `regex: ...` dentro do item"));
        };
        match key.trim() {
            "regex" => {
                let Some(item) = state.item.as_mut() else {
                    return Err(format!("{location}: `regex` sem item"));
                };
                item.regex = unquote(value.trim()).to_owned();
            }
            "exclusive" => {
                let exclusive = parse_exclusivity("exclusive", unquote(value.trim()), &location)?;
                let Some(item) = state.item.as_mut() else {
                    return Err(format!("{location}: `exclusive` sem item"));
                };
                item.exclusive = exclusive;
                state.exclusive_seen = true;
            }
            other => return Err(format!("{location}: chave desconhecida no item: `{other}`")),
        }
    }
    finish_item(&mut parsed, &mut state)?;
    if parsed.scalars.is_empty() {
        return Err("arquivo sem nenhum campo do registro".to_owned());
    }
    Ok(parsed)
}

fn parse_exclusivity(key: &str, value: &str, location: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!(
            "{location}: `{key}` esperava true ou false, achei `{other}`"
        )),
    }
}

fn unquote(value: &str) -> &str {
    let Some(without_opening_quote) = value.strip_prefix('"') else {
        return value;
    };
    without_opening_quote.strip_suffix('"').unwrap_or(value)
}

struct ExpectedRegistration<'a> {
    url: &'a str,
    prefix: &'a str,
    secrets: &'a MatrixSecrets,
}

fn compare(parsed: &ParsedRegistration, expected: &ExpectedRegistration<'_>) -> Vec<String> {
    let mut divergences = Vec::new();
    let mut compare_field = |field: &str, found: Option<&str>, expected_value: &str| match found {
        Some(found) if found == expected_value => {}
        Some(found) => divergences.push(format!(
            "campo {field}: esperado `{expected_value}`, encontrado `{found}`"
        )),
        None => divergences.push(format!(
            "campo {field}: ausente (esperado `{expected_value}`)"
        )),
    };

    compare_field(
        "url",
        parsed.scalars.get("url").map(String::as_str),
        expected.url,
    );
    compare_field(
        "sender_localpart",
        parsed.scalars.get("sender_localpart").map(String::as_str),
        &sender_localpart(expected.prefix),
    );
    compare_field(
        "rate_limited",
        parsed.scalars.get("rate_limited").map(String::as_str),
        "false",
    );
    compare_field("as_token", presence(parsed, "as_token"), "presente");
    compare_field("hs_token", presence(parsed, "hs_token"), "presente");

    for (field, in_file, in_environment) in [
        (
            "as_token",
            parsed.scalars.get("as_token"),
            Some(expected.secrets.as_token.expose().as_str()),
        ),
        (
            "hs_token",
            parsed.scalars.get("hs_token"),
            Some(expected.secrets.hs_token.expose().as_str()),
        ),
    ] {
        if let (Some(in_file), Some(in_environment)) = (in_file, in_environment)
            && in_file != in_environment
        {
            let variable = if field == "as_token" {
                VAR_AS_TOKEN
            } else {
                VAR_HS_TOKEN
            };
            divergences.push(format!(
                "campo {field}: o valor do arquivo difere de {variable} no ambiente (gere de novo)"
            ));
        }
    }

    let expected_user = NamespaceItem {
        exclusive: true,
        regex: format!("@{}_.*", expected.prefix),
    };
    if !parsed.users.contains(&expected_user) {
        divergences.push(format!(
            "campo namespaces.users: falta o namespace exclusivo `{}`",
            expected_user.regex
        ));
    }
    let expected_alias = NamespaceItem {
        exclusive: true,
        regex: format!("#{}_.*", expected.prefix),
    };
    if !parsed.aliases.contains(&expected_alias) {
        divergences.push(format!(
            "campo namespaces.aliases: falta o namespace exclusivo `{}`",
            expected_alias.regex
        ));
    }
    let shared_alias = NamespaceItem {
        exclusive: false,
        regex: SHARED_ALIAS.to_owned(),
    };
    if !parsed.aliases.contains(&shared_alias) {
        divergences.push(format!(
            "campo namespaces.aliases: falta o alias compartilhado `{SHARED_ALIAS}` com \
             `exclusive: false`"
        ));
    }
    divergences
}

fn presence<'a>(parsed: &'a ParsedRegistration, field: &str) -> Option<&'a str> {
    parsed
        .scalars
        .get(field)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|_| "presente")
}

fn error(message: &str) -> ExitCode {
    eprintln!("erro: {message}");
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use poph_types::Sensitive;

    const AS_TOKEN_CANARY: &str = "CANARIO-MX21-AS-nao-pode-vazar";
    const HS_TOKEN_CANARY: &str = "CANARIO-MX21-HS-nao-pode-vazar";

    fn secrets() -> MatrixSecrets {
        MatrixSecrets {
            as_token: Sensitive::new(AS_TOKEN_CANARY.to_owned()),
            hs_token: Sensitive::new(HS_TOKEN_CANARY.to_owned()),
        }
    }

    #[test]
    fn generated_yaml_round_trips_and_matches_configuration() {
        let text = yaml("poph-matrix", "http://192.168.0.123:8080", "ph", &secrets());
        let parsed = parse_registration(&text).expect("o próprio yaml gerado tem que ser legível");
        assert_eq!(parsed.scalars["id"], "poph-matrix");
        assert_eq!(parsed.scalars["url"], "http://192.168.0.123:8080");
        assert_eq!(parsed.scalars["as_token"], AS_TOKEN_CANARY);
        assert_eq!(parsed.scalars["sender_localpart"], "ph_as_bot");
        assert_eq!(parsed.scalars["rate_limited"], "false");
        assert_eq!(
            parsed.users,
            vec![NamespaceItem {
                exclusive: true,
                regex: "@ph_.*".to_owned()
            }]
        );
        assert_eq!(
            parsed.aliases,
            vec![
                NamespaceItem {
                    exclusive: true,
                    regex: "#ph_.*".to_owned()
                },
                NamespaceItem {
                    exclusive: false,
                    regex: "#dep_.*".to_owned()
                }
            ]
        );

        let expected = ExpectedRegistration {
            url: "http://192.168.0.123:8080",
            prefix: "ph",
            secrets: &secrets(),
        };
        assert!(compare(&parsed, &expected).is_empty());
    }

    #[test]
    fn prefix_mismatch_reports_namespaces_and_sender() {
        let text = yaml("poph-matrix", "http://h:1", "wa", &secrets());
        let parsed = parse_registration(&text).expect("legível");
        let expected = ExpectedRegistration {
            url: "http://h:1",
            prefix: "ph",
            secrets: &secrets(),
        };
        let divergences = compare(&parsed, &expected);
        let divergence_text = divergences.join("\n");
        assert!(divergence_text.contains("campo sender_localpart"));
        assert!(divergence_text.contains("campo namespaces.users"));
        assert!(divergence_text.contains("campo namespaces.aliases"));
    }

    #[test]
    fn token_values_are_absent_from_divergences() {
        let text = yaml("poph-matrix", "http://outro:2", "ph", &secrets());
        let parsed = parse_registration(&text).expect("legível");
        let other_secrets = MatrixSecrets {
            as_token: Sensitive::new("as-token-diferente".to_owned()),
            hs_token: Sensitive::new("hs-token-diferente".to_owned()),
        };
        let expected = ExpectedRegistration {
            url: "http://esperado:1",
            prefix: "ph",
            secrets: &other_secrets,
        };
        let divergences = compare(&parsed, &expected).join("\n");
        assert!(divergences.contains("campo url"));
        assert!(divergences.contains(VAR_AS_TOKEN));
        assert!(divergences.contains(VAR_HS_TOKEN));
        for canary in [
            AS_TOKEN_CANARY,
            HS_TOKEN_CANARY,
            "as-token-diferente",
            "hs-token-diferente",
        ] {
            assert!(
                !divergences.contains(canary),
                "o valor do token não pode aparecer: {divergences}"
            );
        }
    }

    #[test]
    fn parser_rejects_unsupported_format() {
        for (text, excerpt) in [
            ("id: x\nsem dois pontos\n", "linha 2"),
            (
                "id: x\nnamespaces:\n  users:\n    - regex: \"@ph_.*\"\n",
                "sem `exclusive`",
            ),
            (
                "id: x\nnamespaces:\n  outro: []\n",
                "namespaces desconhecido",
            ),
            (
                "id: x\nnamespaces:\n  users:\n    - exclusive: talvez\n",
                "esperava true ou false",
            ),
        ] {
            let error = parse_registration(text).expect_err("deveria recusar");
            assert!(error.contains(excerpt), "mensagem: {error}");
        }
    }

    #[test]
    fn parser_accepts_mx01_template_with_comments() {
        let text = r##"# comentário
id: poph-matrix-dev
url: "http://192.168.0.123:18010/matrix"
as_token: "abc"
hs_token: "def"
rate_limited: false
sender_localpart: ph_as_bot
namespaces:
  users:
    - exclusive: true
      regex: "@ph_.*"
  aliases:
    - exclusive: true
      regex: "#ph_.*"
    - exclusive: false
      regex: "#dep_.*"
  rooms: []
"##;
        let parsed = parse_registration(text).expect("modelo da MX-01 tem que ser legível");
        assert_eq!(parsed.users.len(), 1);
        assert_eq!(parsed.aliases.len(), 2);
        assert_eq!(parsed.scalars["url"], "http://192.168.0.123:18010/matrix");
    }

    #[test]
    fn quoted_scalars_escape_quotes_and_backslashes() {
        assert_eq!(quote_scalar("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}

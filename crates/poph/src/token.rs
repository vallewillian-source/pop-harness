use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use clap::{Args, Subcommand};
use poph_store::{Page, Store, TokenAdmin};
use poph_types::{AdminTokenId, OrgId};
use rand_core::{OsRng, RngCore};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, open_with_org, parse_id, print_json, validate_limit, validate_text,
};

const PREFIX: &str = "poph_";

pub const GLOBAL_PREFIX: &str = "pophg_";

#[derive(Debug, Subcommand)]
pub enum TokenAction {
    #[command(about = "Cria um token para a org. O valor aparece só nesta saída")]
    Create(CreateToken),
    #[command(about = "Lista os tokens da org (sem valor)")]
    List(ListTokens),
    #[command(about = "Mostra um token da org (sem valor)")]
    Show(TokenRef),
    #[command(about = "Revoga o token: ele deixa de funcionar; o registro fica para auditoria")]
    Delete(TokenRef),
    #[command(
        about = "Gera um token global (para a configuração) e mostra o valor e o bloco TOML com o hash"
    )]
    Global(GenerateGlobal),
}

#[derive(Debug, Args)]
pub struct CreateToken {
    #[arg(
        long,
        help = "Rótulo para reconhecer o token na lista (até 80 caracteres)"
    )]
    pub label: String,

    #[arg(
        long,
        help = "Saída em JSON (formato estável; o valor do token aparece só aqui)"
    )]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListTokens {
    #[arg(long, default_value_t = 50, help = "Quantos tokens trazer (1 a 200)")]
    pub limit: u32,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct TokenRef {
    #[arg(help = "Id (UUID v7) do token")]
    pub token: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct GenerateGlobal {
    #[arg(
        long,
        default_value = "console",
        help = "Rótulo do token (aparece na auditoria)"
    )]
    pub label: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    action: TokenAction,
) -> ExitCode {
    if let TokenAction::Global(args) = action {
        return generate_global(args);
    }
    let (store, org) = match open_with_org(flag, environment, offline, org) {
        Ok(pair) => pair,
        Err(error) => return failure(error),
    };
    match action {
        TokenAction::Create(args) => create(&store, org, args),
        TokenAction::List(args) => list(&store, org, args),
        TokenAction::Show(args) => show(&store, org, args),
        TokenAction::Delete(args) => revoke(&store, org, args),
        TokenAction::Global(_) => unreachable!("tratado acima"),
    }
}

fn create(store: &Store, org: OrgId, args: CreateToken) -> ExitCode {
    let label = match validate_text("rótulo", &args.label, 80) {
        Ok(label) => label,
        Err(error) => return failure(error),
    };
    let secret = generate_secret();
    let hash = match generate_hash(&secret) {
        Ok(hash) => hash,
        Err(error) => return failure(error),
    };
    match store.tokens().create(Some(org), &label, &hash) {
        Ok(record) => {
            let value = format!("{PREFIX}{}_{secret}", record.id);
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "token": token_json(&record),
                    "valor_do_token": value,
                    "aviso": "guarde o valor agora: ele não é mostrado de novo",
                }));
            } else {
                println!("token criado para a org {org}: {label}\n");
                println!("valor do token (aparece só agora):\n  {value}\n");
                println!(
                    "Ele não fica guardado em claro. Se você perder este valor, revogue o token \
                     (`token delete`) e crie outro."
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn generate_global(args: GenerateGlobal) -> ExitCode {
    let label = match validate_text("rótulo", &args.label, 80) {
        Ok(label) => label,
        Err(error) => return failure(error),
    };
    let value = format!("{GLOBAL_PREFIX}{}", generate_secret());
    let hash = match generate_hash(&value) {
        Ok(hash) => hash,
        Err(error) => return failure(error),
    };
    let block = format!(
        "[admin]\ntokens_globais = [\n  {{ label = \"{label}\", hash = \"{hash}\" }},\n]\n"
    );
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "label": label,
            "valor_do_token": value,
            "hash": hash,
            "config_toml": block,
            "aviso": "guarde o valor agora: a configuração guarda só o hash",
        }));
    } else {
        println!("token global gerado: {label}\n");
        println!("valor do token (aparece só agora):\n  {value}\n");
        println!("Guarde a configuração (o valor em claro não fica nela):\n");
        println!("{block}");
        println!(
            "Cole o bloco em poph.toml e reinicie o servidor. Um token global enxerga todas as \
             orgs; para um token de uma org só, use `poph token create --org ...`."
        );
    }
    ExitCode::SUCCESS
}

fn list(store: &Store, org: OrgId, args: ListTokens) -> ExitCode {
    let page: Page = match validate_limit(args.limit) {
        Ok(page) => page,
        Err(error) => return failure(error),
    };
    match store.tokens().list(org, page) {
        Ok(tokens) => {
            if args.json {
                let items: Vec<Value> = tokens.iter().map(token_json).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": items.len(),
                    "tokens": items,
                }));
            } else if tokens.is_empty() {
                println!("Nenhum token na org.");
            } else {
                for token in &tokens {
                    println!("{}", summary_line(token));
                }
                println!("\n{} token(s).", tokens.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn show(store: &Store, org: OrgId, args: TokenRef) -> ExitCode {
    let token = match fetch(store, org, &args.token) {
        Ok(token) => token,
        Err(error) => return failure(error),
    };
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": org.to_string(),
            "token": token_json(&token),
            "valor_disponivel": false,
        }));
    } else {
        print!("{}", token_text(&token));
        println!("  valor: não disponível (o valor só aparece na criação e não fica guardado)");
    }
    ExitCode::SUCCESS
}

fn revoke(store: &Store, org: OrgId, args: TokenRef) -> ExitCode {
    let token = match fetch(store, org, &args.token) {
        Ok(token) => token,
        Err(error) => return failure(error),
    };
    let already_revoked = token.revoked_at.is_some();
    if let Err(error) = store.tokens().revoke(org, token.id) {
        return failure(describe(&error));
    }
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": org.to_string(),
            "token_id": token.id.to_string(),
            "revogado": true,
            "ja_estava_revogado": already_revoked,
        }));
    } else if already_revoked {
        println!("token {} já estava revogado (nada mudou)", token.id);
    } else {
        println!(
            "token {} revogado: ele não funciona mais. O registro fica na lista para auditoria.",
            token.id
        );
    }
    ExitCode::SUCCESS
}

fn fetch(store: &Store, org: OrgId, text: &str) -> Result<TokenAdmin, String> {
    let id: AdminTokenId = parse_id(text)?;
    match store.tokens().get(org, id) {
        Ok(token) => Ok(token),
        Err(error) if error.is_not_found() => Err(format!("não existe token `{text}` nesta org")),
        Err(error) => Err(describe(&error)),
    }
}

fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut secret = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(secret, "{byte:02x}");
    }
    secret
}

fn generate_hash(value: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| format!("não consegui guardar o token com segurança: {error}"))
}

fn status(token: &TokenAdmin) -> &'static str {
    if token.revoked_at.is_some() {
        "revogado"
    } else {
        "ativo"
    }
}

fn token_json(token: &TokenAdmin) -> Value {
    json!({
        "id": token.id.to_string(),
        "org_id": token.org_id.map(|org| org.to_string()),
        "label": token.label,
        "status": if token.revoked_at.is_some() { "revoked" } else { "active" },
        "created_at": token.created_at,
        "revoked_at": token.revoked_at,
    })
}

fn summary_line(token: &TokenAdmin) -> String {
    format!("{}  {:<9} {}", token.id, status(token), token.label)
}

fn token_text(token: &TokenAdmin) -> String {
    format!(
        "token {}\n  rótulo: {}\n  situação: {}\n",
        token.id,
        token.label,
        status(token)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_has_256_bits_in_hex() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 64);
        assert!(
            secret
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(
            generate_secret(),
            secret,
            "dois segredos não podem sair iguais"
        );
    }

    #[test]
    fn org_token_value_carries_uuid_and_secret() {
        let id = AdminTokenId::generate();
        let secret = generate_secret();
        let value = format!("{PREFIX}{id}_{secret}");
        let rest = value.strip_prefix(PREFIX).expect("prefixo");
        let (uuid, secret_part) = rest.split_once('_').expect("dois campos");
        assert_eq!(uuid, id.to_string());
        assert_eq!(secret_part, secret);
        assert_eq!(uuid.parse::<AdminTokenId>().expect("uuid v7 no valor"), id);
    }

    #[test]
    fn global_token_has_own_prefix() {
        let value = format!("{GLOBAL_PREFIX}{}", generate_secret());
        assert!(value.starts_with("pophg_"));
        assert!(!value.starts_with(PREFIX));
    }

    #[test]
    fn hash_is_argon2_with_salt_and_verifies_value() {
        use argon2::password_hash::PasswordVerifier;
        let value = generate_secret();
        let hash = generate_hash(&value).expect("hash");
        assert!(hash.starts_with("$argon2id$"), "hash: {hash}");
        assert!(
            !hash.contains(&value),
            "o hash não pode conter o valor em claro"
        );
        let parsed = argon2::password_hash::PasswordHash::new(&hash).expect("formato PHC");
        assert!(
            Argon2::default()
                .verify_password(value.as_bytes(), &parsed)
                .is_ok()
        );
        assert!(
            Argon2::default()
                .verify_password(b"outro", &parsed)
                .is_err()
        );
        assert_ne!(generate_hash(&value).unwrap(), hash);
    }
}

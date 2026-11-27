//! `poph token`: tokens de administração de uma org. HR-08.
//!
//! - O token **global** vem da configuração; esta CLI não cria token global.
//! - O token **de org** é gerado aqui, guardado só como hash argon2 e mostrado em claro **uma
//!   única vez**, na saída da criação. Depois disso nenhum comando (nem `--json`) mostra o valor,
//!   porque ele não fica guardado em lugar nenhum.
//! - Não há `update`: o segredo não muda. Para trocar de token, cria-se outro e revoga-se o antigo.
//!   `delete` revoga (o token para de funcionar e o registro fica para auditoria).

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
    abrir_com_org, describe, failure, parse_id, print_json, validate_limit, validate_text,
};

/// Prefixo do valor gerado (ajuda a reconhecer o segredo em logs e em varreduras).
///
/// Formato do token de org (HR-20): `poph_<uuid do token>_<segredo>`. O uuid localiza o registro
/// no banco e o segredo é conferido contra o hash argon2 gravado. O token global usa outro
/// prefixo (`pophg_`) e vive só na configuração, como hash.
const PREFIXO: &str = "poph_";

/// Prefixo do token global (vem da configuração, não do banco).
pub const PREFIXO_GLOBAL: &str = "pophg_";

/// Subcomandos de `poph token` (todos exigem `--org`, exceto `global`).
#[derive(Debug, Subcommand)]
pub enum AcaoToken {
    /// Cria um token para a org. O valor aparece só nesta saída.
    Create(CriarToken),
    /// Lista os tokens da org (sem valor).
    List(ListarTokens),
    /// Mostra um token da org (sem valor).
    Show(ReferenciaToken),
    /// Revoga o token: ele deixa de funcionar; o registro fica para auditoria.
    Delete(ReferenciaToken),
    /// Gera um token global (para a configuração) e mostra o valor e o bloco TOML com o hash.
    Global(GerarGlobal),
}

/// `poph token create`.
#[derive(Debug, Args)]
pub struct CriarToken {
    /// Rótulo para reconhecer o token na lista (até 80 caracteres).
    #[arg(long)]
    pub label: String,

    /// Saída em JSON (formato estável; o valor do token aparece só aqui).
    #[arg(long)]
    pub json: bool,
}

/// `poph token list`.
#[derive(Debug, Args)]
pub struct ListarTokens {
    /// Quantos tokens trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Referência a um token pelo id (UUID v7).
#[derive(Debug, Args)]
pub struct ReferenciaToken {
    /// Id (UUID v7) do token.
    pub token: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph token global`: gera um token global e o bloco de configuração com o hash.
#[derive(Debug, Args)]
pub struct GerarGlobal {
    /// Rótulo do token (aparece na auditoria).
    #[arg(long, default_value = "console")]
    pub label: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Executa um subcomando de `poph token`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Environment,
    offline: bool,
    org: Option<&str>,
    acao: AcaoToken,
) -> ExitCode {
    // `global` não toca no banco: o token global vive só na configuração.
    if let AcaoToken::Global(args) = acao {
        return gerar_global(args);
    }
    let (store, org) = match abrir_com_org(flag, ambiente, offline, org) {
        Ok(par) => par,
        Err(erro) => return failure(erro),
    };
    match acao {
        AcaoToken::Create(args) => criar(&store, org, args),
        AcaoToken::List(args) => listar(&store, org, args),
        AcaoToken::Show(args) => mostrar(&store, org, args),
        AcaoToken::Delete(args) => revogar(&store, org, args),
        AcaoToken::Global(_) => unreachable!("tratado acima"),
    }
}

fn criar(store: &Store, org: OrgId, args: CriarToken) -> ExitCode {
    let rotulo = match validate_text("rótulo", &args.label, 80) {
        Ok(rotulo) => rotulo,
        Err(erro) => return failure(erro),
    };
    // O segredo é gerado antes do registro: o valor final é `poph_<uuid>_<segredo>`, e o uuid
    // só existe depois de criar. O hash guardado cobre o segredo (a parte aleatória).
    let segredo = gerar_segredo();
    let hash = match gerar_hash(&segredo) {
        Ok(hash) => hash,
        Err(erro) => return failure(erro),
    };
    match store.tokens().create(Some(org), &rotulo, &hash) {
        Ok(registro) => {
            let valor = format!("{PREFIXO}{}_{segredo}", registro.id);
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "token": json_token(&registro),
                    "valor_do_token": valor,
                    "aviso": "guarde o valor agora: ele não é mostrado de novo",
                }));
            } else {
                println!("token criado para a org {org}: {rotulo}\n");
                println!("valor do token (aparece só agora):\n  {valor}\n");
                println!(
                    "Ele não fica guardado em claro. Se você perder este valor, revogue o token \
                     (`token delete`) e crie outro."
                );
            }
            ExitCode::SUCCESS
        }
        Err(erro) => failure(describe(&erro)),
    }
}

/// `poph token global`: gera o valor e o bloco de configuração, sem gravar em lugar nenhum.
fn gerar_global(args: GerarGlobal) -> ExitCode {
    let rotulo = match validate_text("rótulo", &args.label, 80) {
        Ok(rotulo) => rotulo,
        Err(erro) => return failure(erro),
    };
    let valor = format!("{PREFIXO_GLOBAL}{}", gerar_segredo());
    let hash = match gerar_hash(&valor) {
        Ok(hash) => hash,
        Err(erro) => return failure(erro),
    };
    let bloco = format!(
        "[admin]\ntokens_globais = [\n  {{ label = \"{rotulo}\", hash = \"{hash}\" }},\n]\n"
    );
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "label": rotulo,
            "valor_do_token": valor,
            "hash": hash,
            "config_toml": bloco,
            "aviso": "guarde o valor agora: a configuração guarda só o hash",
        }));
    } else {
        println!("token global gerado: {rotulo}\n");
        println!("valor do token (aparece só agora):\n  {valor}\n");
        println!("Guarde a configuração (o valor em claro não fica nela):\n");
        println!("{bloco}");
        println!(
            "Cole o bloco em poph.toml e reinicie o servidor. Um token global enxerga todas as \
             orgs; para um token de uma org só, use `poph token create --org ...`."
        );
    }
    ExitCode::SUCCESS
}

fn listar(store: &Store, org: OrgId, args: ListarTokens) -> ExitCode {
    let pagina: Page = match validate_limit(args.limit) {
        Ok(pagina) => pagina,
        Err(erro) => return failure(erro),
    };
    match store.tokens().list(org, pagina) {
        Ok(tokens) => {
            if args.json {
                let itens: Vec<Value> = tokens.iter().map(json_token).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": itens.len(),
                    "tokens": itens,
                }));
            } else if tokens.is_empty() {
                println!("Nenhum token na org.");
            } else {
                for token in &tokens {
                    println!("{}", linha_resumo(token));
                }
                println!("\n{} token(s).", tokens.len());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => failure(describe(&erro)),
    }
}

fn mostrar(store: &Store, org: OrgId, args: ReferenciaToken) -> ExitCode {
    let token = match obter(store, org, &args.token) {
        Ok(token) => token,
        Err(erro) => return failure(erro),
    };
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": org.to_string(),
            "token": json_token(&token),
            "valor_disponivel": false,
        }));
    } else {
        print!("{}", texto_token(&token));
        println!("  valor: não disponível (o valor só aparece na criação e não fica guardado)");
    }
    ExitCode::SUCCESS
}

fn revogar(store: &Store, org: OrgId, args: ReferenciaToken) -> ExitCode {
    let token = match obter(store, org, &args.token) {
        Ok(token) => token,
        Err(erro) => return failure(erro),
    };
    let ja_estava_revogado = token.revoked_at.is_some();
    if let Err(erro) = store.tokens().revoke(org, token.id) {
        return failure(describe(&erro));
    }
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": org.to_string(),
            "token_id": token.id.to_string(),
            "revogado": true,
            "ja_estava_revogado": ja_estava_revogado,
        }));
    } else if ja_estava_revogado {
        println!("token {} já estava revogado (nada mudou)", token.id);
    } else {
        println!(
            "token {} revogado: ele não funciona mais. O registro fica na lista para auditoria.",
            token.id
        );
    }
    ExitCode::SUCCESS
}

/// Busca o token pelo id, dentro da org. Id inválido e token de outra org viram erro legível.
fn obter(store: &Store, org: OrgId, texto: &str) -> Result<TokenAdmin, String> {
    let id: AdminTokenId = parse_id(texto)?;
    match store.tokens().get(org, id) {
        Ok(token) => Ok(token),
        Err(erro) if erro.is_not_found() => Err(format!("não existe token `{texto}` nesta org")),
        Err(erro) => Err(describe(&erro)),
    }
}

/// Segredo novo: 32 bytes aleatórios em hexadecimal (256 bits).
fn gerar_segredo() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut segredo = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(segredo, "{byte:02x}");
    }
    segredo
}

/// Hash argon2id (formato PHC, com sal próprio). Este é o único registro do token no banco.
fn gerar_hash(valor: &str) -> Result<String, String> {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .map(|hash| hash.to_string())
        .map_err(|erro| format!("não consegui guardar o token com segurança: {erro}"))
}

fn situacao(token: &TokenAdmin) -> &'static str {
    if token.revoked_at.is_some() {
        "revogado"
    } else {
        "ativo"
    }
}

fn json_token(token: &TokenAdmin) -> Value {
    json!({
        "id": token.id.to_string(),
        "org_id": token.org_id.map(|org| org.to_string()),
        "label": token.label,
        "status": if token.revoked_at.is_some() { "revoked" } else { "active" },
        "created_at": token.created_at,
        "revoked_at": token.revoked_at,
    })
}

fn linha_resumo(token: &TokenAdmin) -> String {
    format!("{}  {:<9} {}", token.id, situacao(token), token.label)
}

fn texto_token(token: &TokenAdmin) -> String {
    format!(
        "token {}\n  rótulo: {}\n  situação: {}\n",
        token.id,
        token.label,
        situacao(token)
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn segredo_tem_256_bits_em_hexadecimal() {
        let segredo = gerar_segredo();
        assert_eq!(segredo.len(), 64);
        assert!(
            segredo
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(
            gerar_segredo(),
            segredo,
            "dois segredos não podem sair iguais"
        );
    }

    #[test]
    fn valor_do_token_de_org_carrega_o_uuid_e_o_segredo() {
        let id = AdminTokenId::generate();
        let segredo = gerar_segredo();
        let valor = format!("{PREFIXO}{id}_{segredo}");
        let resto = valor.strip_prefix(PREFIXO).expect("prefixo");
        let (uuid, parte_secreta) = resto.split_once('_').expect("dois campos");
        assert_eq!(uuid, id.to_string());
        assert_eq!(parte_secreta, segredo);
        assert_eq!(uuid.parse::<AdminTokenId>().expect("uuid v7 no valor"), id);
    }

    #[test]
    fn token_global_tem_prefixo_proprio() {
        let valor = format!("{PREFIXO_GLOBAL}{}", gerar_segredo());
        assert!(valor.starts_with("pophg_"));
        assert!(!valor.starts_with(PREFIXO));
    }

    #[test]
    fn hash_e_argon2_com_sal_e_confere_o_valor() {
        use argon2::password_hash::PasswordVerifier;
        let valor = gerar_segredo();
        let hash = gerar_hash(&valor).expect("hash");
        assert!(hash.starts_with("$argon2id$"), "hash: {hash}");
        assert!(
            !hash.contains(&valor),
            "o hash não pode conter o valor em claro"
        );
        let lido = argon2::password_hash::PasswordHash::new(&hash).expect("formato PHC");
        assert!(
            Argon2::default()
                .verify_password(valor.as_bytes(), &lido)
                .is_ok()
        );
        assert!(Argon2::default().verify_password(b"outro", &lido).is_err());
        // Sal próprio: o mesmo valor gera hashes diferentes.
        assert_ne!(gerar_hash(&valor).unwrap(), hash);
    }
}

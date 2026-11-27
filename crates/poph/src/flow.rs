use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_server::flow_service::{
    Actor, Diagnostic, FlowError, FlowOperations, RevisionCache, audit, comparison_json, flow_json,
    flow_with_draft_json, revision_summary_json, validation_json,
};
use poph_store::{FlowFilter, Store};
use poph_types::{HarnessConfig, OrgId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, open_with_configuration, parse_page, parse_status, print_json,
    resolve_agent, resolve_flow, resolve_org,
};

const REPORT_VERSION: u64 = 1;

#[derive(Debug, Subcommand)]
pub enum FlowAction {
    #[command(
        name = "validar",
        about = "Compila e valida um script, sem gravar nada. Sai com 1 se há erro"
    )]
    Validate(FileArgs),
    #[command(
        name = "publicar",
        about = "Publica o script como revisão nova. Se o fluxo ainda não existe, cria e publica a revisão 1"
    )]
    Publish(PublishArgs),
    #[command(
        name = "listar",
        about = "Lista os fluxos da org (primeira página, em ordem de id)"
    )]
    List(ListArgs),
    #[command(
        name = "mostrar",
        about = "Mostra um fluxo: o rascunho e a revisão que vale hoje. Aceita id ou slug"
    )]
    Show(ReferenceArgs),
    #[command(
        about = "Compara duas revisões: o que mudou e quantos atendimentos estão em cada uma"
    )]
    Diff(DiffArgs),
    #[command(
        name = "formatar",
        about = "Mostra o script na forma canônica. Com `--escrever`, grava no próprio arquivo"
    )]
    Format(FormatArgs),
    #[command(
        name = "liberar",
        about = "Libera o fluxo a um agente (o agente só usa fluxos liberados a ele)"
    )]
    Release(PermissionArgs),
    #[command(name = "retirar", about = "Tira a permissão do fluxo de um agente")]
    Withdraw(PermissionArgs),
}

#[derive(Debug, Args)]
pub struct FileArgs {
    #[arg(
        id = "arquivo",
        value_name = "ARQUIVO",
        help = "Arquivo do script do fluxo"
    )]
    pub file: PathBuf,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PublishArgs {
    #[arg(
        id = "arquivo",
        value_name = "ARQUIVO",
        help = "Arquivo do script a publicar"
    )]
    pub file: PathBuf,

    #[arg(
        long,
        help = "Versão do fluxo lida com `mostrar` (`If-Match`). Obrigatória quando o fluxo já existe"
    )]
    pub if_match: Option<u64>,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long, help = "Situação: `active` ou `inactive`. Sem ele, lista todos")]
    pub status: Option<String>,

    #[arg(long, default_value_t = 50, help = "Quantos fluxos trazer (1 a 200)")]
    pub limit: u32,

    #[arg(long, help = "`next_cursor` da página anterior")]
    pub cursor: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ReferenceArgs {
    #[arg(
        id = "fluxo",
        value_name = "FLUXO",
        help = "Id (UUID v7) ou slug do fluxo"
    )]
    pub flow: String,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    #[arg(
        id = "fluxo",
        value_name = "FLUXO",
        help = "Id (UUID v7) ou slug do fluxo"
    )]
    pub flow: String,

    #[arg(
        long = "de",
        value_name = "DE",
        id = "de",
        help = "Número da revisão antiga. Padrão: a revisão nova menos 1"
    )]
    pub from: Option<u64>,

    #[arg(
        long = "para",
        value_name = "PARA",
        id = "para",
        help = "Número da revisão nova. Padrão: a revisão que vale hoje"
    )]
    pub to: Option<u64>,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct FormatArgs {
    #[arg(
        id = "arquivo",
        value_name = "ARQUIVO",
        help = "Arquivo do script a formatar"
    )]
    pub file: PathBuf,

    #[arg(
        long = "escrever",
        id = "escrever",
        help = "Grava o resultado no próprio arquivo, em vez de mostrar"
    )]
    pub write: bool,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionArgs {
    #[arg(
        id = "agente",
        value_name = "AGENTE",
        help = "Agente: id (UUID v7) ou slug"
    )]
    pub agent: String,

    #[arg(
        long = "fluxo",
        id = "fluxo",
        value_name = "FLUXO",
        help = "Fluxo: id (UUID v7) ou slug"
    )]
    pub flow: String,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

struct Session {
    store: Store,
    cache: RevisionCache,
    config: HarnessConfig,
}

impl Session {
    fn open(flag: Option<PathBuf>, environment: &config::Environment) -> Result<Self, String> {
        let (store, config) = open_with_configuration(flag, environment)?;
        let cache = RevisionCache::new(
            usize::try_from(config.flows.revision_cache_mb).unwrap_or(usize::MAX),
        );
        Ok(Self {
            store,
            cache,
            config,
        })
    }

    fn operations(&self) -> FlowOperations<'_> {
        FlowOperations::new(&self.store, &self.cache, &self.config)
    }

    fn org(&self, text: Option<&str>) -> Result<OrgId, String> {
        let text =
            text.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
        resolve_org(&self.store, text)
    }
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
    action: FlowAction,
) -> ExitCode {
    let result = match action {
        FlowAction::Validate(args) => {
            Session::open(flag, environment).and_then(|session| validate(&session, org, args))
        }
        FlowAction::Format(args) => {
            Session::open(flag, environment).and_then(|session| format(&session, args))
        }
        FlowAction::Publish(args) => {
            Session::open(flag, environment).and_then(|session| publish(&session, org, args))
        }
        FlowAction::List(args) => {
            Session::open(flag, environment).and_then(|session| list(&session, org, args))
        }
        FlowAction::Show(args) => {
            Session::open(flag, environment).and_then(|session| show(&session, org, args))
        }
        FlowAction::Diff(args) => {
            Session::open(flag, environment).and_then(|session| diff(&session, org, args))
        }
        FlowAction::Release(args) => Session::open(flag, environment)
            .and_then(|session| permission(&session, org, args, true)),
        FlowAction::Withdraw(args) => Session::open(flag, environment)
            .and_then(|session| permission(&session, org, args, false)),
    };
    match result {
        Ok(exit_code) => exit_code,
        Err(message) => failure(message),
    }
}

fn validate(session: &Session, org: Option<&str>, args: FileArgs) -> Result<ExitCode, String> {
    let source = read_script(&args.file)?;
    let org = match org {
        Some(text) => Some(resolve_org(&session.store, text)?),
        None => None,
    };
    let validation = session
        .operations()
        .validate(org, &source)
        .map_err(|error| describe(&error))?;
    if args.json {
        print_json(&with_version(validation_json(&validation)));
    } else {
        if validation.is_valid() {
            println!("{}: script válido.", args.file.display());
        } else {
            println!("{}: script inválido.", args.file.display());
        }
        print_diagnostics(&validation.diagnostics);
        for resolution in &validation.resolutions {
            match &resolution.department_id {
                Some(id) => println!("  departamento \"{}\": encontrado ({id})", resolution.nome),
                None => println!(
                    "  departamento \"{}\": não encontrado nesta org",
                    resolution.nome
                ),
            }
        }
    }
    Ok(exit_code(validation.is_valid()))
}

fn format(session: &Session, args: FormatArgs) -> Result<ExitCode, String> {
    let source = read_script(&args.file)?;
    let text = match session.operations().format(&source) {
        Ok(text) => text,
        Err(FlowError::InvalidScript(diagnostics)) => {
            print_diagnostics(&diagnostics);
            let errors = diagnostics.iter().filter(|d| d.is_error()).count();
            return Err(format!(
                "o script não compila: {errors} erro(s) a corrigir antes de formatar"
            ));
        }
        Err(other) => return Err(other.to_string()),
    };
    if args.write {
        fs::write(&args.file, &text)
            .map_err(|error| format!("não consegui gravar `{}`: {error}", args.file.display()))?;
    }
    if args.json {
        print_json(&json!({
            "versao_relatorio": REPORT_VERSION,
            "source": text,
            "written": args.write,
        }));
    } else if args.write {
        println!(
            "{}: formatado e gravado no próprio arquivo.",
            args.file.display()
        );
    } else {
        print!("{text}");
    }
    Ok(ExitCode::SUCCESS)
}

fn publish(session: &Session, org: Option<&str>, args: PublishArgs) -> Result<ExitCode, String> {
    let org = session.org(org)?;
    let source = read_script(&args.file)?;
    let ops = session.operations();
    let compiled = ops.compile(&source);
    if compiled.has_error() {
        print_diagnostics(&compiled.diagnostics);
        let errors = compiled.diagnostics.iter().filter(|d| d.is_error()).count();
        return Err(format!(
            "o script não passa na validação: {errors} erro(s) a corrigir; nada foi gravado"
        ));
    }
    let Some(ir) = compiled.ir.as_ref() else {
        return Err("erro interno: o compilador não devolveu a estrutura do script".to_owned());
    };
    let actor = Actor::cli();
    let existing = session
        .store
        .flows()
        .by_slug(org, &ir.fluxo.id)
        .map_err(|error| describe(&error))?;
    match existing {
        None => {
            let creation = ops
                .create(org, &source, true, &actor)
                .map_err(|error| error.to_string())?;
            let publication = creation
                .publication
                .as_ref()
                .ok_or_else(|| "erro interno: a criação não publicou a revisão 1".to_owned())?;
            if args.json {
                print_json(&json!({
                    "versao_relatorio": REPORT_VERSION,
                    "created": true,
                    "flow": flow_with_draft_json(&creation.flow),
                    "revision": revision_summary_json(&publication.revision),
                    "warnings": publication.warnings,
                }));
            } else {
                println!(
                    "fluxo `{}` criado e publicado como revisão {}.",
                    creation.flow.slug, publication.revision.numero
                );
                print_diagnostics(&publication.warnings);
            }
        }
        Some(flow) => {
            let expected = args.if_match.ok_or_else(|| {
                format!(
                    "o fluxo `{}` já existe: informe --if-match com a versão atual (veja com \
                     `poph --org ... flow mostrar {}`)",
                    flow.slug, flow.slug
                )
            })?;
            let publication = ops
                .publish(org, &flow, expected, &source, &actor)
                .map_err(|error| error.to_string())?;
            if args.json {
                print_json(&json!({
                    "versao_relatorio": REPORT_VERSION,
                    "created": false,
                    "flow_id": flow.id.to_string(),
                    "revision": revision_summary_json(&publication.revision),
                    "version": publication.flow_version,
                    "warnings": publication.warnings,
                }));
            } else {
                println!(
                    "fluxo `{}` publicado como revisão {}.",
                    flow.slug, publication.revision.numero
                );
                print_diagnostics(&publication.warnings);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn list(session: &Session, org: Option<&str>, args: ListArgs) -> Result<ExitCode, String> {
    let org = session.org(org)?;
    let page = parse_page(args.limit, args.cursor.as_deref())?;
    let filter = FlowFilter {
        status: args.status.as_deref().map(parse_status).transpose()?,
    };
    let flows = session
        .store
        .flows()
        .list(org, &filter, page)
        .map_err(|error| describe(&error))?;
    let next_cursor = if flows.len() < page.limit as usize {
        None
    } else {
        flows.last().map(|flow| flow.id.to_string())
    };
    let items: Vec<Value> = flows.iter().map(flow_json).collect();
    if args.json {
        print_json(&json!({
            "versao_relatorio": REPORT_VERSION,
            "org_id": org.to_string(),
            "items": items,
            "next_cursor": next_cursor,
        }));
    } else if items.is_empty() {
        println!("Nenhum fluxo nesta org.");
    } else {
        for item in &items {
            println!(
                "{}  {:<9} {:<28} {}  (revisão {}, versão {})",
                json_value_text(&item["id"]),
                json_value_text(&item["status"]),
                json_value_text(&item["slug"]),
                json_value_text(&item["name"]),
                json_value_text(&item["current_revision"]),
                json_value_text(&item["version"]),
            );
        }
        if let Some(cursor) = &next_cursor {
            println!("\nPróxima página: --cursor {cursor}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn show(session: &Session, org: Option<&str>, args: ReferenceArgs) -> Result<ExitCode, String> {
    let org = session.org(org)?;
    let flow = resolve_flow(&session.store, org, &args.flow)?;
    let revision = session
        .store
        .revisions()
        .current(org, flow.id)
        .map_err(|error| describe(&error))?;
    let mut body = flow_with_draft_json(&flow);
    body["revision"] = revision.as_ref().map_or(Value::Null, revision_summary_json);
    if args.json {
        print_json(&with_version(body));
        return Ok(ExitCode::SUCCESS);
    }
    println!("fluxo `{}` — {}", flow.slug, flow.name);
    println!("  id: {}", flow.id);
    println!("  situação: {}", json_value_text(&body["status"]));
    println!("  versão: {}", flow.version);
    match &revision {
        Some(r) => println!(
            "  revisão que vale hoje: {} (publicada por {})",
            r.numero, r.publicada_por
        ),
        None => println!("  revisão que vale hoje: nenhuma (só rascunho)"),
    }
    println!("\nRascunho:\n{}", flow.rascunho_fonte);
    Ok(ExitCode::SUCCESS)
}

fn diff(session: &Session, org: Option<&str>, args: DiffArgs) -> Result<ExitCode, String> {
    let org = session.org(org)?;
    let flow = resolve_flow(&session.store, org, &args.flow)?;
    let comparison = session
        .operations()
        .compare(org, &flow, args.from, args.to)
        .map_err(|error| error.to_string())?;
    if args.json {
        print_json(&with_version(comparison_json(flow.id, &comparison)));
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "revisão {} → revisão {}: {} mudança(s).",
        comparison.from.numero,
        comparison.to.numero,
        comparison.changes.len()
    );
    println!(
        "  atendimentos ativos: {} na revisão {}, {} na revisão {}.",
        comparison.active_from, comparison.from.numero, comparison.active_to, comparison.to.numero
    );
    for change in &comparison.changes {
        println!("  - {}", change.texto);
    }
    Ok(ExitCode::SUCCESS)
}

fn permission(
    session: &Session,
    org: Option<&str>,
    args: PermissionArgs,
    release: bool,
) -> Result<ExitCode, String> {
    let org = session.org(org)?;
    let flow = resolve_flow(&session.store, org, &args.flow)?;
    let agent = resolve_agent(&session.store, org, &args.agent)?;
    let result = if release {
        session.store.flows().release(org, agent.id, flow.id)
    } else {
        session.store.flows().revoke(org, agent.id, flow.id)
    };
    result.map_err(|error| describe(&error))?;
    audit(
        &session.store,
        org,
        &Actor::cli(),
        "agent_flows_update",
        "agent",
        agent.id.to_string(),
    )
    .map_err(|error| describe(&error))?;
    let action = if release { "liberado" } else { "retirado" };
    if args.json {
        print_json(&json!({
            "versao_relatorio": REPORT_VERSION,
            "org_id": org.to_string(),
            "agent_id": agent.id.to_string(),
            "flow_id": flow.id.to_string(),
            "action": action,
        }));
    } else if release {
        println!("fluxo `{}` liberado ao agente `{}`.", flow.slug, agent.name);
    } else {
        println!("fluxo `{}` retirado do agente `{}`.", flow.slug, agent.name);
    }
    Ok(ExitCode::SUCCESS)
}

fn read_script(file: &Path) -> Result<String, String> {
    fs::read_to_string(file)
        .map_err(|error| format!("não consegui ler `{}`: {error}", file.display()))
}

fn exit_code(without_error: bool) -> ExitCode {
    if without_error {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn with_version(mut value: Value) -> Value {
    value["versao_relatorio"] = json!(REPORT_VERSION);
    value
}

fn json_value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "-".to_owned(),
        other => other.to_string(),
    }
}

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        let location = match (diagnostic.line(), diagnostic.column()) {
            (Some(line), Some(column)) => format!("linha {line}, coluna {column}: "),
            _ => String::new(),
        };
        println!(
            "  {location}{} {}: {}",
            diagnostic.severity.as_text(),
            diagnostic.code.as_text(),
            diagnostic.mensagem
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_reflects_error() {
        assert_eq!(exit_code(true), ExitCode::SUCCESS);
        assert_eq!(exit_code(false), ExitCode::from(1));
    }

    #[test]
    fn json_value_text_unquotes_strings_and_marks_null() {
        assert_eq!(json_value_text(&json!("ativo")), "ativo");
        assert_eq!(json_value_text(&Value::Null), "-");
        assert_eq!(json_value_text(&json!(3)), "3");
    }

    #[test]
    fn with_version_adds_report_version() {
        let value = with_version(json!({ "valid": true }));
        assert_eq!(value["versao_relatorio"], 1);
        assert_eq!(value["valid"], true);
    }
}

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_matrix::spaces::SpaceOrigin as StoredOrigin;
use poph_matrix::{
    Classification, DepartmentSpace, MatrixClient, MatrixModule, SpaceError, Spaces,
};
use poph_server::matrix_beginning::SpaceStorePort;
use poph_store::{NewAuditEntry, StateMatrixOrg, Store};
use poph_types::{DepartmentId, HarnessConfig, OrgId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{describe, parse_id, print_json, require_offline, resolve_org};

pub const CONFLICT_EXIT: u8 = 3;

#[derive(Debug, Args)]
pub struct OutputArgs {
    #[arg(long, help = "Saída em JSON (formato estável, `versao_relatorio` = 1)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SpaceArgs {
    #[arg(
        long = "departamento",
        value_name = "ID",
        help = "Departamento do espaço (id UUID v7 da org). Sem ele, lista os espaços da org"
    )]
    pub department: Option<String>,

    #[arg(
        long = "espaco",
        value_name = "ID_OU_ALIAS",
        help = "Espaço que já existe no Element: id `!…` ou alias `#…`. Sem ele, o módulo cria o espaço do departamento pelo alias próprio. Só vale junto com `--departamento`"
    )]
    pub space: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável, `versao_relatorio` = 1)")]
    pub json: bool,

    #[command(subcommand)]
    pub action: Option<SpaceAction>,
}

#[derive(Debug, Subcommand)]
pub enum SpaceAction {
    #[command(
        name = "desligar",
        about = "Desliga o espaço do departamento: a linha sai do Harness; a sala e o espaço continuam no Element"
    )]
    Disable(DisableSpaceArgs),
}

#[derive(Debug, Args)]
pub struct DisableSpaceArgs {
    #[arg(
        long = "departamento",
        value_name = "ID",
        help = "Departamento cujo espaço sai do módulo (id UUID v7 da org)"
    )]
    pub department: String,

    #[arg(long, help = "Saída em JSON (formato estável, `versao_relatorio` = 1)")]
    pub json: bool,
}

type CommandResult = Result<ExitCode, String>;

struct Context {
    store: Arc<Store>,
    org: OrgId,
    config: HarnessConfig,
}

fn open(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
) -> Result<Context, String> {
    require_offline(offline)?;
    let text = org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let config = config::load(file.as_deref(), environment)
        .map_err(|error| format!("configuração inválida: {error}"))?;
    let store =
        Store::open(&config).map_err(|error| format!("não consegui abrir o banco: {error}"))?;
    let org = resolve_org(&store, text)?;
    Ok(Context {
        store: Arc::new(store),
        org,
        config,
    })
}

pub fn state(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    args: &OutputArgs,
) -> ExitCode {
    let ctx = match open(flag, environment, offline, org) {
        Ok(ctx) => ctx,
        Err(message) => return error(&message),
    };
    respond(show_state(&ctx, args.json))
}

pub fn set_enabled(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    enable: bool,
    args: &OutputArgs,
) -> ExitCode {
    let ctx = match open(flag, environment, offline, org) {
        Ok(ctx) => ctx,
        Err(message) => return error(&message),
    };
    respond(change_link(&ctx, enable, args.json))
}

pub fn space(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    args: &SpaceArgs,
) -> ExitCode {
    let ctx = match open(flag, environment, offline, org) {
        Ok(ctx) => ctx,
        Err(message) => return error(&message),
    };
    let result = match (&args.action, &args.department) {
        (Some(SpaceAction::Disable(disable)), None) if args.space.is_none() => {
            disable_space(&ctx, &disable.department, disable.json)
        }
        (Some(_), _) => Err(
            "use `espaco desligar --departamento <id>` sozinho: não combine com --departamento ou --espaco"
                .to_owned(),
        ),
        (None, Some(department)) => {
            ensure_space(&ctx, department, args.space.as_deref(), args.json)
        }
        (None, None) if args.space.is_some() => {
            Err("--espaco só vale junto com --departamento (o espaço é de um departamento)".to_owned())
        }
        (None, None) => list_spaces(&ctx, args.json),
    };
    respond(result)
}

fn respond(result: CommandResult) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(message) => error(&message),
    }
}

fn error(message: &str) -> ExitCode {
    eprintln!("erro: {message}");
    ExitCode::from(1)
}

fn show_state(ctx: &Context, json: bool) -> CommandResult {
    let row = read(ctx.store.matrix().state(ctx.org))?;
    let cursors = read(ctx.store.matrix().read_cursors(ctx.org))?;
    let delay = read(
        ctx.store
            .matrix()
            .delay_of_messages(ctx.org, cursors.messages),
    )?;
    let age_s = delay
        .oldest_at
        .map(|created_at| ((ctx.store.now() - created_at).max(0) / 1000) as u64)
        .unwrap_or(0);
    let failures = read(ctx.store.matrix().count_failures(ctx.org))?;
    let last_failure = read(ctx.store.matrix().list_failures(ctx.org, 1))?
        .first()
        .map(|failure| failure.motivo.clone());
    let slug = read(ctx.store.orgs().get(ctx.org))?.slug;

    if json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org": slug,
            "ligado": row.linked,
            "corte_do_backfill_ms": row.linked_at,
            "versao": row.updated_at,
            "conectado": Value::Null,
            "atraso": { "mensagens": delay.pending, "idade_mais_antiga_s": age_s },
            "falhas_pendentes": failures,
            "ultima_falha": last_failure,
        }));
    } else {
        println!("módulo Matrix da org `{slug}`");
        println!("  ligado:                 {}", yes_no(row.linked));
        println!(
            "  corte do backfill:      {}",
            row.linked_at
                .map(data_utc)
                .unwrap_or_else(|| "nunca ligado".to_owned())
        );
        println!("  conectado ao Synapse:   desconhecido pela CLI (a sonda roda no servidor)");
        println!(
            "  pendentes de espelhar:  {} mensagem(ns){}",
            delay.pending,
            if delay.pending > 0 {
                format!(", a mais antiga há {age_s} s")
            } else {
                String::new()
            }
        );
        println!("  falhas isoladas:        {failures}");
        println!(
            "  último motivo de falha: {}",
            last_failure.as_deref().unwrap_or("nenhum")
        );
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug)]
enum LinkError {
    VersionChanged,
    Other(String),
}

fn change_link(ctx: &Context, enable: bool, json: bool) -> CommandResult {
    let before = read(ctx.store.matrix().state(ctx.org))?;
    let expected = before.updated_at;
    match apply_link(ctx, enable, expected) {
        Ok(after) => {
            let slug = read(ctx.store.orgs().get(ctx.org))?.slug;
            if json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org": slug,
                    "ligado": after.linked,
                    "corte_do_backfill_ms": after.linked_at,
                    "versao": after.updated_at,
                }));
            } else if enable {
                println!("módulo Matrix ligado na org `{slug}`.");
                println!(
                    "  corte do backfill: {} (só o que chegar daqui em diante é espelhado)",
                    after.linked_at.map(data_utc).unwrap_or_default()
                );
                println!("  o servidor pega a mudança no próximo ciclo, sem reiniciar.");
            } else {
                println!("módulo Matrix desligado na org `{slug}`.");
                println!(
                    "  nada foi apagado: salas e espaços continuam no Element, e o corte do backfill fica registrado."
                );
                println!("  o servidor pega a mudança no próximo ciclo, sem reiniciar.");
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(LinkError::VersionChanged) => {
            eprintln!(
                "erro: o estado do módulo mudou entre a leitura e a gravação (outro processo gravou ao mesmo tempo). Nada foi alterado: rode `poph matrix estado` e tente de novo"
            );
            Ok(ExitCode::from(CONFLICT_EXIT))
        }
        Err(LinkError::Other(message)) => Err(message),
    }
}

fn apply_link(ctx: &Context, enable: bool, expected: i64) -> Result<StateMatrixOrg, LinkError> {
    let current = ctx
        .store
        .matrix()
        .state(ctx.org)
        .map_err(|error| LinkError::Other(describe(&error)))?;
    if current.updated_at != expected {
        return Err(LinkError::VersionChanged);
    }
    let after = if enable {
        ctx.store.matrix().enable(ctx.org)
    } else {
        ctx.store.matrix().disable(ctx.org)
    }
    .map_err(|error| LinkError::Other(describe(&error)))?;
    record_audit(ctx, "update", "matrix", ctx.org.to_string()).map_err(LinkError::Other)?;
    Ok(after)
}

fn list_spaces(ctx: &Context, json: bool) -> CommandResult {
    let slug = read(ctx.store.orgs().get(ctx.org))?.slug;
    let spaces = read(ctx.store.matrix().list_spaces(ctx.org))?;
    let mut items = Vec::new();
    for space in &spaces {
        let name = ctx
            .store
            .departments()
            .get(ctx.org, space.departamento)
            .ok()
            .map(|department| department.name);
        items.push((space, name));
    }
    if json {
        let listed: Vec<Value> = items
            .iter()
            .map(|(space, name)| {
                json!({
                    "departamento": space.departamento.to_string(),
                    "departamento_nome": name,
                    "espaco": space.room_id,
                    "alias": space.alias,
                    "origem": space.origin.as_text(),
                })
            })
            .collect();
        print_json(&json!({ "versao_relatorio": 1, "org": slug, "espacos": listed }));
    } else if items.is_empty() {
        println!("nenhum espaço por departamento na org `{slug}`.");
    } else {
        println!("espaços por departamento da org `{slug}`: {}", items.len());
        for (space, name) in &items {
            println!(
                "  - {} → {} ({})",
                name.as_deref().unwrap_or("departamento sem nome"),
                space.room_id,
                origin_description(space.origin.as_text())
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn ensure_space(
    ctx: &Context,
    department_text: &str,
    existing: Option<&str>,
    json: bool,
) -> CommandResult {
    let department_id: DepartmentId = parse_id(department_text)?;
    let department = read(ctx.store.departments().get(ctx.org, department_id))?;
    let existing = existing.map(str::trim).filter(|text| !text.is_empty());

    let module = match MatrixModule::prepare(ctx.config.matrix.clone()) {
        Ok(Some(module)) => module,
        Ok(None) => {
            return Err(
                "o módulo Matrix não está ligado nesta configuração: ponha `habilitado = true` na seção [matrix]"
                    .to_owned(),
            );
        }
        Err(error) => return Err(error.to_string()),
    };
    let client = MatrixClient::from_parts(&module.config, &module.secrets).map_err(|_| {
        "não consegui montar a conexão com o Synapse: confira `homeserver_url` na seção [matrix]"
            .to_owned()
    })?;
    let spaces = Spaces::of_config(
        client,
        SpaceStorePort(Arc::clone(&ctx.store)),
        &module.config,
    )
    .map_err(|_| "não consegui montar o serviço de espaços: confira a seção [matrix]".to_owned())?;

    let target = DepartmentSpace {
        id: department.id,
        name: department.name.clone(),
    };
    let stored =
        run_async(spaces.ensure_space(ctx.org, &target, existing))?.map_err(space_message)?;
    record_audit(ctx, "update", "matrix_space", department.id.to_string())?;

    let origin = match stored.origin {
        StoredOrigin::Automatic => "auto",
        StoredOrigin::Linked => "linked",
    };
    if json {
        print_json(&json!({
            "versao_relatorio": 1,
            "departamento": department.id.to_string(),
            "departamento_nome": department.name,
            "espaco": stored.room_id,
            "alias": stored.alias,
            "origem": origin,
        }));
    } else {
        println!(
            "espaço do departamento `{}`: {} ({})",
            department.name,
            stored.room_id,
            origin_description(origin)
        );
        println!("  alias: {}", stored.alias);
    }
    Ok(ExitCode::SUCCESS)
}

fn disable_space(ctx: &Context, department_text: &str, json: bool) -> CommandResult {
    let department_id: DepartmentId = parse_id(department_text)?;
    let department = read(ctx.store.departments().get(ctx.org, department_id))?;
    let disabled = read(ctx.store.matrix().disable_space(ctx.org, department.id))?;
    record_audit(ctx, "delete", "matrix_space", department.id.to_string())?;
    if json {
        print_json(&json!({
            "versao_relatorio": 1,
            "departamento": department.id.to_string(),
            "removido": disabled.removed,
            "salas_esquecidas": disabled.rooms_forgotten,
        }));
    } else if disabled.removed {
        println!(
            "espaço do departamento `{}` desligado: {} sala(s) deixaram de estar registradas nele.",
            department.name, disabled.rooms_forgotten
        );
        println!(
            "  nada foi apagado no Element. Na próxima alteração de conversa desse departamento, a sala volta a ser posicionada no mesmo espaço."
        );
    } else {
        println!(
            "o departamento `{}` não tinha espaço ligado: nada a fazer.",
            department.name
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn space_message(error: SpaceError) -> String {
    match error {
        SpaceError::WithoutPermission { what_is_missing } => what_is_missing,
        SpaceError::InvalidSpace { reason } => format!("espaço informado inválido: {reason}"),
        SpaceError::Internal { .. } => {
            "falha interna do módulo Matrix ao garantir o espaço (defeito do módulo, não do ambiente)"
                .to_owned()
        }
        SpaceError::Armazenamento(_) => {
            "o armazenamento do módulo Matrix não respondeu: tente de novo".to_owned()
        }
        SpaceError::Call(call) => match call.classification() {
            Classification::Transient => {
                "o Synapse não respondeu: tente de novo em instantes".to_owned()
            }
            Classification::Definitive => {
                "o Synapse recusou a operação do espaço: confira o espaço informado no Element"
                    .to_owned()
            }
        },
    }
}

fn run_async<F: std::future::Future>(future: F) -> Result<F::Output, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("não consegui iniciar o runtime: {error}"))?;
    Ok(runtime.block_on(future))
}

fn origin_description(origin: &str) -> &'static str {
    match origin {
        "auto" | "automatico" => "criado pelo módulo",
        _ => "ligado a um espaço que já existia no Element",
    }
}

fn read<T>(result: Result<T, poph_store::StoreError>) -> Result<T, String> {
    result.map_err(|error| describe(&error))
}

fn record_audit(ctx: &Context, acao: &str, entity: &str, id: String) -> Result<(), String> {
    read(ctx.store.audit().register(
        Some(ctx.org),
        NewAuditEntry {
            ator_tipo: "cli".into(),
            actor_id: None,
            acao: acao.into(),
            entity: entity.into(),
            entity_id: Some(id),
            detail_json: "{}".into(),
        },
    ))?;
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value { "sim" } else { "não" }
}

pub fn data_utc(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3600,
        (rest % 3600) / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use poph_store::NewOrg;
    use std::fs;

    #[test]
    fn data_utc_converts_zero_and_one_known_instant() {
        assert_eq!(data_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(data_utc(1_795_289_241_000), "2026-11-21 19:27 UTC");
        assert_eq!(data_utc(1_709_164_800_000), "2024-02-29 00:00 UTC");
    }

    #[test]
    fn enable_refuses_when_version_changed_and_writes_nothing() {
        let folder =
            std::env::temp_dir().join(format!("poph-matrix-admin-412-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("pasta de teste");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco de teste");
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja".into(),
                slug: "loja-412".into(),
                config_json: "{}".into(),
            })
            .expect("cria a org");
        let ctx = Context {
            store: Arc::new(store),
            org: org.id,
            config: HarnessConfig::default(),
        };
        let current = ctx.store.matrix().state(org.id).expect("estado").updated_at;

        let refused = apply_link(&ctx, true, current + 1);
        assert!(
            matches!(refused, Err(LinkError::VersionChanged)),
            "com versão velha tem que recusar: {refused:?}"
        );
        assert!(
            !ctx.store.matrix().state(org.id).expect("estado").linked,
            "recusado não pode ter ligado o módulo"
        );

        let stored = apply_link(&ctx, true, current).expect("com a versão certa grava");
        assert!(stored.linked);
        let _ = fs::remove_dir_all(&folder);
    }
}

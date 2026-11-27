mod agent;
mod assembly;
mod backup;
mod case;
mod category;
mod chat;
mod cluster;
mod config;
mod department;
mod doctor;
mod file;
mod flow;
mod jev;
mod kb;
mod logs;
mod matrix;
mod matrix_admin;
mod model;
mod org;
mod product;
mod rag;
mod retention;
mod server;
mod support;
mod token;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, CommandFactory, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "poph",
    version,
    about = "POP Harness: motor de agentes multi-tenant",
    arg_required_else_help = true
)]
struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "ARQUIVO",
        help = "Arquivo de configuração TOML. Sem ele, usa POPH_CONFIG ou ./poph.toml, se existir"
    )]
    config: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        value_name = "ORG",
        help = "Org do contexto (id UUID v7 ou slug). Obrigatória nos comandos que mexem em dados de org"
    )]
    org: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Opera direto no banco, sem servidor. Obrigatório em `org`, `token`, `category`, `department`, `kb` e `file` enquanto o modo com servidor não existe"
    )]
    offline: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
enum Command {
    #[command(
        about = "Confere o ambiente: configuração, pasta de dados, bancos e variáveis obrigatórias"
    )]
    Doctor(DoctorArgs),
    #[command(
        about = "Cria ou atualiza os bancos (`harness.db` e `rag.db`). Rode com o servidor parado"
    )]
    Migrate(MigrateArgs),
    #[command(
        subcommand,
        about = "Gerencia o modelo de embedding (`poph model install` baixa e confere o modelo)"
    )]
    Model(model::ModelAction),
    #[command(
        subcommand,
        about = "Gerencia agentes: identidade, modelo, parâmetros, handoff e categorias"
    )]
    Agent(agent::AgentAction),
    #[command(
        subcommand,
        about = "Gerencia as instruções estruturadas e o texto composto do agente"
    )]
    Instructions(agent::InstructionsAction),
    #[command(
        subcommand,
        about = "Gerencia as orgs (tenants): criar, listar, mostrar, alterar e excluir"
    )]
    Org(org::OrgAction),
    #[command(
        subcommand,
        about = "Gerencia os tokens de administração de uma org (`--org`)"
    )]
    Token(token::TokenAction),
    #[command(
        subcommand,
        about = "Gerencia as categorias de produto de uma org (`--org`)"
    )]
    Category(category::CategoryAction),
    #[command(subcommand, about = "Gerencia os departamentos de uma org (`--org`)")]
    Department(department::DepartmentAction),
    #[command(
        subcommand,
        about = "Base de conhecimento: grupos, itens, vínculo com agentes e índice (HR-17)"
    )]
    Kb(kb::KbAction),
    #[command(
        subcommand,
        about = "Índice da busca semântica: `poph rag reindex` (mesmo comando do `poph kb reindex`)"
    )]
    Rag(rag::RagAction),
    #[command(
        subcommand,
        about = "Produtos do catálogo: criar, listar, alterar, excluir e reindexar (HR-18)"
    )]
    Product(product::ProductAction),
    #[command(
        subcommand,
        about = "Biblioteca de arquivos do agente: enviar, listar, mostrar e excluir (HR-24)"
    )]
    File(file::FileAction),
    #[command(about = "Conversa com um agente pelo terminal (streaming; Ctrl-C cancela o turno)")]
    Chat(chat::ChatArgs),
    #[command(
        subcommand,
        about = "Jev (decisões rápidas): `eval` mede um gate sobre um conjunto rotulado (HR-28)"
    )]
    Jev(jev::JevAction),
    #[command(
        subcommand,
        about = "Fluxos da org: validar, publicar, listar, mostrar, comparar, formatar e liberar a agentes"
    )]
    Flow(flow::FlowAction),
    #[command(
        subcommand,
        about = "Fichas da org (um atendimento por Ficha): listar e mostrar, com as execuções de fluxo"
    )]
    Ficha(case::CaseAction),
    #[command(
        subcommand,
        about = "Retenção do histórico: apaga o que venceu (`run`) e mostra a configuração (`show`)"
    )]
    Retention(retention::RetentionAction),
    #[command(
        subcommand,
        about = "Exporta uma org inteira num arquivo único e reconstrói a partir dele (HR-53)"
    )]
    Backup(backup::BackupAction),
    #[command(
        subcommand,
        about = "Roteamento entre instâncias: mostra e confere o `cluster.toml` (HR-50a)"
    )]
    Cluster(cluster::ClusterAction),
    #[command(
        subcommand,
        about = "Módulo Matrix: registro do Application Service (`registro`) e, por org (`--offline --org`), `estado`, `ligar`, `desligar` e `espaco`"
    )]
    Matrix(matrix::MatrixAction),
    #[command(about = "Sobe o servidor HTTP (API administrativa, `/healthz` e `/metrics`)")]
    Serve(server::ServeArgs),
    #[command(about = "Imprime o documento OpenAPI (base do cliente TypeScript do Console)")]
    Openapi(server::OpenapiArgs),
}

#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long, help = "Saída em JSON (formato estável, sem segredos)")]
    json: bool,
}

#[derive(Debug, Args)]
struct MigrateArgs {
    #[arg(
        long,
        help = "Saída em JSON (versões antes/depois e migrações aplicadas)"
    )]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        let _ = Cli::command().print_help();
        println!();
        return ExitCode::from(2);
    };
    let environment = process_environment();
    match command {
        Command::Doctor(args) => run_doctor(cli.config, &environment, args),
        Command::Migrate(args) => run_migrate(cli.config, &environment, args),
        Command::Model(action) => model::run(cli.config, &environment, action),
        Command::Agent(acao) => {
            agent::run_agent(cli.config, &environment, cli.org.as_deref(), acao)
        }
        Command::Instructions(acao) => {
            agent::run_instructions(cli.config, &environment, cli.org.as_deref(), acao)
        }
        Command::Org(acao) => org::run(cli.config, &environment, cli.offline, acao),
        Command::Token(acao) => token::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::Category(acao) => category::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::Department(acao) => department::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::Jev(action) => jev::run(cli.config, &environment, action),
        Command::Flow(acao) => flow::run(cli.config, &environment, cli.org.as_deref(), acao),
        Command::Ficha(acao) => case::run(cli.config, &environment, cli.org.as_deref(), acao),
        Command::Retention(action) => {
            retention::run(cli.config, &environment, cli.org.as_deref(), action)
        }
        Command::Backup(acao) => backup::run(cli.config, &environment, cli.offline, acao),
        Command::Cluster(acao) => cluster::run(cli.config, &environment, acao),
        Command::Matrix(acao) => matrix::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::Kb(acao) => kb::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::Rag(acao) => rag::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::Product(acao) => product::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::File(acao) => file::run(
            cli.config,
            &environment,
            cli.offline,
            cli.org.as_deref(),
            acao,
        ),
        Command::Chat(args) => chat::run(cli.config, &environment, cli.org.as_deref(), args),
        Command::Serve(args) => server::run(cli.config, &environment, args),
        Command::Openapi(args) => server::run_openapi(args),
    }
}

fn process_environment() -> config::Environment {
    std::env::vars_os()
        .map(|(chave, value)| {
            (
                chave.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect()
}

fn run_migrate(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    args: MigrateArgs,
) -> ExitCode {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let config = match config::load(file.as_deref(), environment) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };

    let reports = match poph_store::migrate_databases(&config.harness_db(), &config.rag_db()) {
        Ok(reports) => reports,
        Err(error) => {
            eprintln!("não consegui migrar os bancos: {error}");
            return ExitCode::from(1);
        }
    };

    if args.json {
        println!("{}", migration_json(&reports));
    } else {
        println!("POP Harness: migração dos bancos\n");
        println!("{}", reports.harness.summary());
        println!("{}", reports.rag.summary());
        if reports.already_up_to_date() {
            println!("\nNada a fazer: os dois bancos já estavam em dia.");
        } else {
            println!("\nBancos prontos para uso.");
        }
    }
    ExitCode::SUCCESS
}

fn migration_json(reports: &poph_store::MigrationReports) -> String {
    fn database_entry(report: &poph_store::MigrationReport) -> serde_json::Value {
        serde_json::json!({
            "banco": report.database,
            "caminho": report.path.display().to_string(),
            "versao_antes": report.version_before,
            "versao_depois": report.version_after,
            "aplicadas": report.applied.iter().map(|m| serde_json::json!({
                "versao": m.version,
                "nome": m.name,
            })).collect::<Vec<_>>(),
        })
    }
    let value = serde_json::json!({
        "versao_relatorio": 1,
        "harness": database_entry(&reports.harness),
        "rag": database_entry(&reports.rag),
        "ja_estava_em_dia": reports.already_up_to_date(),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn run_doctor(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    args: DoctorArgs,
) -> ExitCode {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let loaded = config::load(file.as_deref(), environment);

    let log = loaded
        .as_ref()
        .map(|config| config.log.clone())
        .unwrap_or_default();
    if let Err(error) = logs::start(
        &log,
        environment.get(logs::VAR_RUST_LOG).map(String::as_str),
    ) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }

    let segredos = config::secrets(environment);
    let key_name = loaded
        .as_ref()
        .map(|config| config.llm.api_key_env.clone())
        .unwrap_or_else(|_| config::VAR_ANTIGRAVITY.to_owned());
    let llm_key = config::llm_key(environment, &key_name);
    let agents_by_provider = loaded
        .as_ref()
        .ok()
        .and_then(|config| poph_store::agents_by_provider_at(&config.harness_db()).ok());
    let report = doctor::generate(doctor::Input {
        config: loaded.as_ref(),
        file: file.as_deref(),
        segredos: &segredos,
        llm_key: llm_key.as_ref(),
        environment: Some(environment),
        agents_by_provider: agents_by_provider.as_deref(),
    });

    if args.json {
        println!("{}", report.json());
    } else {
        print!("{}", report.texto());
    }

    if report.has_error() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

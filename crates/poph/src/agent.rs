use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_agent::compose_instructions;
use poph_store::{AgentChange, Agente as Agent, Instructions, NewAgent, Page, Status, Store};
use poph_types::{
    AgentId, DepartmentId, HarnessConfig, OrgId, ProductCategoryId, StructuredInstructions,
};
use serde_json::{Value, json};

use crate::config;

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    #[command(about = "Cria o agente na org")]
    Create(CreateAgent),
    #[command(about = "Lista os agentes da org (paginado por id)")]
    List(ListAgents),
    #[command(about = "Mostra um agente, as categorias e o texto composto das instruções")]
    Show(AgentReference),
    #[command(about = "Altera um agente com `If-Match`; também define as categorias de produto")]
    Update(UpdateAgent),
    #[command(about = "Exclui um agente (instruções, vínculos e conversas saem junto)")]
    Delete(AgentReference),
}

#[derive(Debug, Subcommand)]
pub enum InstructionsAction {
    #[command(about = "Grava as instruções do agente (o agente não pode ter instruções ainda)")]
    Create(WriteInstructions),
    #[command(about = "Altera as instruções existentes com `If-Match`")]
    Update(WriteInstructions),
    #[command(about = "Mostra o JSON estruturado e o texto composto")]
    Show(AgentReference),
    #[command(about = "Apaga as instruções do agente")]
    Delete(AgentReference),
    #[command(about = "Lista as instruções da org (um registro por agente que já tem instruções)")]
    List(ListInstructions),
}

#[derive(Debug, Args)]
pub struct AgentReference {
    #[arg(
        id = "agente",
        value_name = "AGENTE",
        help = "Id (UUID v7) ou slug do agente"
    )]
    pub agent: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CreateAgent {
    #[arg(long, help = "Nome de exibição (3 a 120 caracteres)")]
    pub name: String,

    #[arg(
        long,
        help = "Slug único na org (`^[a-z0-9-]{3,64}$`). Sem ele, é derivado do nome"
    )]
    pub slug: Option<String>,

    #[arg(long, default_value = "", help = "Descrição curta")]
    pub description: String,

    #[arg(
        long = "type",
        default_value = "custom",
        id = "tipo",
        value_name = "TIPO",
        help = "Tipo: `custom` (padrão) ou `standard` (paridade com o POP Atende)"
    )]
    pub kind: String,

    #[arg(
        long,
        default_value = "generic",
        help = "Categoria do POP Atende (metadado; `generic` por padrão)"
    )]
    pub category: String,

    #[arg(
        long,
        help = "Departamento padrão (id): onde as conversas novas entram"
    )]
    pub department: Option<DepartmentId>,

    #[arg(long, help = "Departamento humano do gate G4")]
    pub handoff_department: Option<DepartmentId>,

    #[arg(long, default_value = "antigravity2api", help = "Provedor do LLM")]
    pub provider: String,

    #[arg(
        long,
        default_value = "gemini-3.8-flash-tiered",
        help = "Modelo do LLM (vale por agente)"
    )]
    pub model: String,

    #[arg(
        long,
        help = "Temperatura do LLM (sem valor, usa o padrão do provedor)"
    )]
    pub temperature: Option<f64>,

    #[arg(
        long,
        help = "Teto de tokens de saída (sem valor, usa o padrão da configuração)"
    )]
    pub max_output_tokens: Option<i64>,

    #[arg(
        long,
        default_value_t = 64_000,
        help = "Orçamento de contexto em tokens"
    )]
    pub max_context_tokens: u32,

    #[arg(
        long,
        default_value_t = 2_000,
        help = "Janela de agrupamento em ms (0 a 8000)"
    )]
    pub grouping_window_ms: u32,

    #[arg(
        long,
        default_value_t = 4,
        help = "Máximo de rodadas de ferramenta por turno"
    )]
    pub max_tool_rounds: u32,

    #[arg(
        long,
        default_value_t = 10,
        help = "Máximo de chamadas de ferramenta por mensagem"
    )]
    pub max_tool_calls: u32,

    #[arg(long, default_value_t = 3, help = "Quantos trechos o RAG devolve")]
    pub rag_top_k: u32,

    #[arg(
        long,
        default_value_t = 0.0,
        help = "Corte mínimo de relevância do RAG (0 a 1)"
    )]
    pub rag_min_score: f64,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListAgents {
    #[arg(long, default_value_t = 50, help = "Quantos agentes trazer (1 a 200)")]
    pub limit: u32,

    #[arg(long, help = "Só os agentes que podem abrir turno (`active`)")]
    pub only_active: bool,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UpdateAgent {
    #[arg(
        id = "agente",
        value_name = "AGENTE",
        help = "Id (UUID v7) ou slug do agente"
    )]
    pub agent: String,

    #[arg(long, help = "Versão lida (`If-Match`). Obrigatória")]
    pub if_match: u64,

    #[arg(long, help = "Novo nome")]
    pub name: Option<String>,

    #[arg(long, help = "Nova descrição")]
    pub description: Option<String>,

    #[arg(
        long,
        help = "Nova situação: `active`, `inactive` ou `archived` (D-05: só `active` abre turno)"
    )]
    pub status: Option<String>,

    #[arg(long, help = "Novo departamento padrão (id)")]
    pub department: Option<DepartmentId>,

    #[arg(
        long,
        help = "Liga/desliga o gate G1 de transferência (`--handoff true|false`)"
    )]
    pub handoff: Option<bool>,

    #[arg(long, help = "Novo departamento humano do gate G4 (id)")]
    pub handoff_department: Option<DepartmentId>,

    #[arg(long, help = "Novo provedor do LLM")]
    pub provider: Option<String>,

    #[arg(long, help = "Novo modelo do LLM")]
    pub model: Option<String>,

    #[arg(long, help = "Nova temperatura")]
    pub temperature: Option<f64>,

    #[arg(long, help = "Novo teto de tokens de saída")]
    pub max_output_tokens: Option<i64>,

    #[arg(long, help = "Novo orçamento de contexto em tokens")]
    pub max_context_tokens: Option<u32>,

    #[arg(long, help = "Nova janela de agrupamento em ms")]
    pub grouping_window_ms: Option<u32>,

    #[arg(long, help = "Novo máximo de rodadas de ferramenta")]
    pub max_tool_rounds: Option<u32>,

    #[arg(long, help = "Novo máximo de chamadas de ferramenta por mensagem")]
    pub max_tool_calls: Option<u32>,

    #[arg(long, help = "Novo top-k do RAG")]
    pub rag_top_k: Option<u32>,

    #[arg(long, help = "Novo corte mínimo do RAG")]
    pub rag_min_score: Option<f64>,

    #[arg(long, help = "Flags do Jev do agente (JSON)")]
    pub jev_json: Option<String>,

    #[arg(
        long = "category-id",
        id = "categorias",
        value_name = "CATEGORIAS",
        help = "Substitui as categorias de produto do agente (repita a opção; sem ela, não mexe)"
    )]
    pub categories: Option<Vec<ProductCategoryId>>,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct WriteInstructions {
    #[arg(
        id = "agente",
        value_name = "AGENTE",
        help = "Id (UUID v7) ou slug do agente"
    )]
    pub agent: String,

    #[arg(
        long,
        help = "Versão lida da instrução (`If-Match`). Obrigatória no `update`"
    )]
    pub if_match: Option<u64>,

    #[arg(
        long,
        value_name = "ARQUIVO",
        conflicts_with = "markdown_file",
        help = "Arquivo JSON com o `StructuredInstructions`. O texto é recomposto pelo compositor"
    )]
    pub file: Option<PathBuf>,

    #[arg(
        long,
        value_name = "ARQUIVO",
        help = "Arquivo markdown com o prompt editado à mão. Sem recomposição (`derived_from_json = 0`)"
    )]
    pub markdown_file: Option<PathBuf>,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListInstructions {
    #[arg(
        long,
        default_value_t = 50,
        help = "Quantos registros trazer (1 a 200)"
    )]
    pub limit: u32,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

pub fn run_agent(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
    action: AgentAction,
) -> ExitCode {
    match action {
        AgentAction::Create(args) => {
            let (store, org, configuration) = match open(flag, environment, org) {
                Ok(pair) => pair,
                Err(error) => return failure(&error),
            };
            create(&store, org, &configuration, args)
        }
        AgentAction::List(args) => {
            let (store, org, _) = match open(flag, environment, org) {
                Ok(pair) => pair,
                Err(error) => return failure(&error),
            };
            list(&store, org, args)
        }
        AgentAction::Show(args) => {
            let (store, org, _) = match open(flag, environment, org) {
                Ok(pair) => pair,
                Err(error) => return failure(&error),
            };
            show(&store, org, args)
        }
        AgentAction::Update(args) => {
            let (store, org, configuration) = match open(flag, environment, org) {
                Ok(pair) => pair,
                Err(error) => return failure(&error),
            };
            update(&store, org, &configuration, args)
        }
        AgentAction::Delete(args) => {
            let (store, org, _) = match open(flag, environment, org) {
                Ok(pair) => pair,
                Err(error) => return failure(&error),
            };
            delete(&store, org, args)
        }
    }
}

pub fn run_instructions(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
    action: InstructionsAction,
) -> ExitCode {
    let (store, org, _) = match open(flag, environment, org) {
        Ok(pair) => pair,
        Err(error) => return failure(&error),
    };
    match action {
        InstructionsAction::Create(args) => write_instructions(&store, org, args, 0),
        InstructionsAction::Update(args) => {
            let Some(expected) = args.if_match else {
                return failure("informe --if-match com a versão lida das instruções");
            };
            write_instructions(&store, org, args, expected)
        }
        InstructionsAction::Show(args) => show_instructions(&store, org, args),
        InstructionsAction::Delete(args) => delete_instructions(&store, org, args),
        InstructionsAction::List(args) => list_instructions(&store, org, args),
    }
}

fn create(store: &Store, org: OrgId, configuration: &HarnessConfig, args: CreateAgent) -> ExitCode {
    if args.name.trim().len() < 3 {
        return failure("o nome do agente precisa de ao menos 3 caracteres");
    }
    if args.kind != "custom" && args.kind != "standard" {
        return failure("--type aceita `custom` ou `standard`");
    }
    if let Err(error) = configuration.validate_provider_choice(&args.provider) {
        return failure(format!("--provider: {error}"));
    }
    let slug = match args.slug.clone() {
        Some(slug) => slug,
        None => derive_slug(&args.name),
    };
    if !is_valid_slug(&slug) {
        return failure(format!(
            "slug `{slug}` inválido: use 3 a 64 caracteres de a-z, 0-9 e `-`"
        ));
    }
    let new = NewAgent {
        name: args.name,
        slug,
        description: args.description,
        agent_type: args.kind,
        category: args.category,
        default_department: args.department,
        provedor: args.provider,
        modelo: args.model,
        temperatura: args.temperature,
        max_tokens_saida: args.max_output_tokens,
        max_context_tokens: args.max_context_tokens,
        grouping_window_ms: args.grouping_window_ms,
        max_tool_rounds: args.max_tool_rounds,
        max_tools_per_message: args.max_tool_calls,
        rag_top_k: args.rag_top_k,
        rag_cutoff: args.rag_min_score,
    };
    match store.agents().create(org, new) {
        Ok(agent) => {
            if args.handoff_department.is_some() {
                let change = AgentChange {
                    handoff_active: Some(true),
                    handoff_department: args.handoff_department,
                    ..AgentChange::default()
                };
                match store.agents().update(org, agent.id, agent.version, change) {
                    Ok(agent) => return agent_success(&agent, args.json, "agente criado"),
                    Err(error) => {
                        return failure(format!("agente criado, mas o handoff falhou: {error}"));
                    }
                }
            }
            agent_success(&agent, args.json, "agente criado")
        }
        Err(error) => failure(format!("não consegui criar o agente: {error}")),
    }
}

fn list(store: &Store, org: OrgId, args: ListAgents) -> ExitCode {
    let agents = if args.only_active {
        store.agents().list_active(org)
    } else {
        store.agents().list(org, Page::with_limit(args.limit))
    };
    match agents {
        Ok(agents) => {
            if args.json {
                let items: Vec<Value> = agents.iter().map(agent_json).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": items.len(),
                    "agentes": items,
                }));
            } else if agents.is_empty() {
                println!("Nenhum agente na org.");
            } else {
                for agent in &agents {
                    println!("{}", summary_line(agent));
                }
                println!("\n{} agente(s).", agents.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(format!("não consegui listar os agentes: {error}")),
    }
}

fn show(store: &Store, org: OrgId, args: AgentReference) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    let categories = match store.agents().categories(org, agent.id) {
        Ok(categories) => categories,
        Err(error) => return failure(format!("não consegui ler as categorias: {error}")),
    };
    let instructions = store.instructions().get(org, agent.id).ok();

    if args.json {
        let mut value = agent_json(&agent);
        if let Value::Object(map) = &mut value {
            map.insert(
                "product_category_ids".to_owned(),
                json!(
                    categories
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                ),
            );
            map.insert(
                "instructions".to_owned(),
                instructions
                    .as_ref()
                    .map(instructions_json)
                    .unwrap_or(Value::Null),
            );
        }
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": org.to_string(),
            "agent": value,
        }));
        return ExitCode::SUCCESS;
    }

    print!("{}", agent_text(&agent, &categories));
    match instructions {
        Some(instructions) => {
            println!(
                "\ninstruções (versão {}, derivado do JSON: {})",
                instructions.version, instructions.derived_from_json
            );
            if instructions.markdown.is_empty() {
                println!("(texto composto vazio)");
            } else {
                println!("\n{}", instructions.markdown);
            }
        }
        None => println!("\nsem instruções gravadas (use `poph instructions create`)."),
    }
    ExitCode::SUCCESS
}

fn update(store: &Store, org: OrgId, configuration: &HarnessConfig, args: UpdateAgent) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    let status = match args.status.as_deref() {
        Some(text) => match Status::from_text(text) {
            Ok(status) => Some(status),
            Err(error) => return failure(format!("--status inválido: {error}")),
        },
        None => None,
    };
    if let Some(text) = &args.jev_json
        && serde_json::from_str::<Value>(text).is_err()
    {
        return failure("--jev-json precisa ser um JSON válido");
    }
    if let Some(choice) = args.provider.as_deref()
        && let Err(error) = configuration.validate_provider_choice(choice)
    {
        return failure(format!("--provider: {error}"));
    }
    let has_categories = args.categories.is_some();
    let has_change = args.name.is_some()
        || args.description.is_some()
        || args.status.is_some()
        || args.department.is_some()
        || args.handoff.is_some()
        || args.handoff_department.is_some()
        || args.provider.is_some()
        || args.model.is_some()
        || args.temperature.is_some()
        || args.max_output_tokens.is_some()
        || args.max_context_tokens.is_some()
        || args.grouping_window_ms.is_some()
        || args.max_tool_rounds.is_some()
        || args.max_tool_calls.is_some()
        || args.rag_top_k.is_some()
        || args.rag_min_score.is_some()
        || args.jev_json.is_some();
    let change = AgentChange {
        name: args.name,
        description: args.description,
        status,
        default_department: args.department,
        handoff_active: args.handoff,
        handoff_department: args.handoff_department,
        provedor: args.provider,
        modelo: args.model,
        temperatura: args.temperature,
        max_tokens_saida: args.max_output_tokens,
        max_context_tokens: args.max_context_tokens,
        grouping_window_ms: args.grouping_window_ms,
        max_tool_rounds: args.max_tool_rounds,
        max_tools_per_message: args.max_tool_calls,
        rag_top_k: args.rag_top_k,
        rag_cutoff: args.rag_min_score,
        jev_json: args.jev_json,
    };
    let mut current = agent;
    if has_change {
        current = match store
            .agents()
            .update(org, current.id, args.if_match, change)
        {
            Ok(current) => current,
            Err(error) => return failure(format!("não consegui alterar o agente: {error}")),
        };
    }
    if let Some(categories) = args.categories {
        current =
            match store
                .agents()
                .define_categories(org, current.id, current.version, &categories)
            {
                Ok(current) => current,
                Err(error) => {
                    return failure(format!("não consegui definir as categorias: {error}"));
                }
            };
    }
    if !has_change && !has_categories {
        return failure("nada a alterar: informe ao menos um campo");
    }
    agent_success(&current, args.json, "agente alterado")
}

fn delete(store: &Store, org: OrgId, args: AgentReference) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    match store.agents().delete(org, agent.id) {
        Ok(()) => {
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "removido": true,
                    "agent_id": agent.id.to_string(),
                }));
            } else {
                println!("agente {} removido", agent.id);
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(format!("não consegui excluir o agente: {error}")),
    }
}

fn write_instructions(
    store: &Store,
    org: OrgId,
    args: WriteInstructions,
    expected: u64,
) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    let (structured, markdown, derived) = match (&args.file, &args.markdown_file) {
        (Some(file), None) => {
            let text = match std::fs::read_to_string(file) {
                Ok(text) => text,
                Err(error) => {
                    return failure(format!("não consegui ler {}: {error}", file.display()));
                }
            };
            let data = match StructuredInstructions::from_tolerant_json(&text) {
                Ok(data) => data,
                Err(error) => return failure(format!("JSON de instruções inválido: {error}")),
            };
            if let Err(error) = data.validar() {
                return failure(error.to_string());
            }
            let markdown = compose_instructions(&data);
            (data.to_json(), markdown, true)
        }
        (None, Some(file)) => {
            let markdown = match std::fs::read_to_string(file) {
                Ok(text) => text,
                Err(error) => {
                    return failure(format!("não consegui ler {}: {error}", file.display()));
                }
            };
            let structured = store
                .instructions()
                .get(org, agent.id)
                .map(|instructions| instructions.structured_json)
                .unwrap_or_else(|_| "{}".to_owned());
            (structured, markdown, false)
        }
        (Some(_), Some(_)) => {
            return failure(
                "use --file (JSON estruturado) ou --markdown-file (prompt manual), não os dois",
            );
        }
        (None, None) => {
            return failure(
                "informe --file com o JSON estruturado ou --markdown-file com o prompt manual",
            );
        }
    };

    match store
        .instructions()
        .save(org, agent.id, expected, &structured, &markdown, derived)
    {
        Ok(instructions) => {
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "instructions": instructions_json(&instructions),
                }));
            } else {
                println!(
                    "instruções gravadas: agente {} · versão {} · derivado do JSON: {}",
                    agent.id, instructions.version, instructions.derived_from_json
                );
                println!("\n{}", instructions.markdown);
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(format!("não consegui gravar as instruções: {error}")),
    }
}

fn show_instructions(store: &Store, org: OrgId, args: AgentReference) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    match store.instructions().get(org, agent.id) {
        Ok(instructions) => {
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "instructions": instructions_json(&instructions),
                }));
            } else {
                println!(
                    "agente {} · versão {} · derivado do JSON: {}",
                    agent.id, instructions.version, instructions.derived_from_json
                );
                println!(
                    "\nestruturado:\n{}",
                    format_structured_json(&instructions.structured_json)
                );
                println!("\nmarkdown:\n{}", instructions.markdown);
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(format!("não consegui ler as instruções: {error}")),
    }
}

fn delete_instructions(store: &Store, org: OrgId, args: AgentReference) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    match store.instructions().delete(org, agent.id) {
        Ok(()) => {
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "removido": true,
                    "agent_id": agent.id.to_string(),
                }));
            } else {
                println!("instruções do agente {} removidas", agent.id);
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(format!("não consegui apagar as instruções: {error}")),
    }
}

fn list_instructions(store: &Store, org: OrgId, args: ListInstructions) -> ExitCode {
    match store.instructions().list(org, Page::with_limit(args.limit)) {
        Ok(instructions) => {
            if args.json {
                let items: Vec<Value> = instructions.iter().map(instructions_json).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": items.len(),
                    "instructions": items,
                }));
            } else if instructions.is_empty() {
                println!("Nenhuma instrução gravada na org.");
            } else {
                for item in &instructions {
                    println!(
                        "{} · versão {} · derivado do JSON: {}",
                        item.agent_id, item.version, item.derived_from_json
                    );
                }
                println!("\n{} registro(s).", instructions.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(format!("não consegui listar as instruções: {error}")),
    }
}

fn open(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
) -> Result<(Store, OrgId, HarnessConfig), String> {
    let Some(org_text) = org else {
        return Err("informe --org com o id (UUID v7) ou o slug da org".to_owned());
    };
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let configuration = config::load(file.as_deref(), environment)
        .map_err(|error| format!("configuração inválida: {error}"))?;
    let store = Store::open(&configuration)
        .map_err(|error| format!("não consegui abrir o banco: {error}"))?;
    let org = resolve_org(&store, org_text)?;
    Ok((store, org, configuration))
}

fn resolve_org(store: &Store, text: &str) -> Result<OrgId, String> {
    if let Ok(id) = text.parse::<OrgId>() {
        return store
            .orgs()
            .get(id)
            .map(|org| org.id)
            .map_err(|error| format!("org `{text}`: {error}"));
    }
    store
        .orgs()
        .by_slug(text)
        .map(|org| org.id)
        .map_err(|error| format!("org `{text}`: {error}"))
}

fn resolve_agent(store: &Store, org: OrgId, text: &str) -> Result<Agent, String> {
    if let Ok(id) = text.parse::<AgentId>() {
        return store
            .agents()
            .get(org, id)
            .map_err(|error| format!("agente `{text}`: {error}"));
    }
    store
        .agents()
        .by_slug(org, text)
        .map_err(|error| format!("agente `{text}`: {error}"))
}

pub(crate) fn derive_slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_hyphen = false;
    for character in name.chars().flat_map(char::to_lowercase).map(strip_accent) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_hyphen = false;
        } else if !last_was_hyphen && !slug.is_empty() {
            slug.push('-');
            last_was_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(64);
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

fn strip_accent(character: char) -> char {
    match character {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

pub(crate) fn is_valid_slug(slug: &str) -> bool {
    let size = slug.chars().count();
    (3..=64).contains(&size)
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn summary_line(agent: &Agent) -> String {
    let status = if agent.can_open_turn() {
        "responde"
    } else {
        "não responde"
    };
    format!(
        "{}  {:<24} slug={:<24} status={:<8} modelo={}",
        agent.id,
        truncate(&agent.name, 24),
        agent.slug,
        status,
        agent.modelo
    )
}

fn agent_text(agent: &Agent, categories: &[ProductCategoryId]) -> String {
    let department = agent
        .default_department
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let handoff = agent
        .handoff_department
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_owned());
    format!(
        "agente {}\n\
         \x20 nome: {}\n\
         \x20 slug: {}\n\
         \x20 descrição: {}\n\
         \x20 status: {} ({})\n\
         \x20 tipo: {} · categoria: {}\n\
         \x20 departamento padrão: {}\n\
         \x20 handoff: {} · departamento humano: {}\n\
         \x20 modelo: {} · provedor: {}\n\
         \x20 temperature: {} · max_output_tokens: {}\n\
         \x20 contexto: {} tokens ({} mensagens)\n\
         \x20 agrupamento: {} ms · rodadas de ferramenta: {} · chamadas/mensagem: {}\n\
         \x20 rag: top_k {} · corte {}\n\
         \x20 categorias de produto: {}\n\
         \x20 versão: {}\n",
        agent.id,
        agent.name,
        agent.slug,
        agent.description,
        agent.status.as_text(),
        if agent.can_open_turn() {
            "abre turno"
        } else {
            "não abre turno (D-05)"
        },
        agent.agent_type,
        agent.category,
        department,
        if agent.handoff_active {
            "ligado"
        } else {
            "desligado"
        },
        handoff,
        agent.modelo,
        agent.provedor,
        agent
            .temperatura
            .map(|value| value.to_string())
            .unwrap_or_else(|| "padrão".to_owned()),
        agent
            .max_tokens_saida
            .map(|value| value.to_string())
            .unwrap_or_else(|| "padrão".to_owned()),
        agent.max_context_tokens,
        agent.max_context_messages,
        agent.grouping_window_ms,
        agent.max_tool_rounds,
        agent.max_tools_per_message,
        agent.rag_top_k,
        agent.rag_cutoff,
        if categories.is_empty() {
            "nenhuma".to_owned()
        } else {
            categories
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        },
        agent.version
    )
}

fn agent_json(agent: &Agent) -> Value {
    json!({
        "id": agent.id.to_string(),
        "org_id": agent.org_id.to_string(),
        "name": agent.name,
        "slug": agent.slug,
        "description": agent.description,
        "status": agent.status.as_text(),
        "abre_turno": agent.can_open_turn(),
        "agent_type": agent.agent_type,
        "category": agent.category,
        "default_department_id": agent.default_department.map(|id| id.to_string()),
        "human_handoff_enabled": agent.handoff_active,
        "human_handoff_department_id": agent.handoff_department.map(|id| id.to_string()),
        "provider": agent.provedor,
        "model": agent.modelo,
        "temperature": agent.temperatura,
        "max_output_tokens": agent.max_tokens_saida,
        "max_context_messages": agent.max_context_messages,
        "max_context_tokens": agent.max_context_tokens,
        "grouping_window_ms": agent.grouping_window_ms,
        "max_tool_rounds": agent.max_tool_rounds,
        "max_tool_calls_per_message": agent.max_tools_per_message,
        "rag_top_k": agent.rag_top_k,
        "rag_min_score": agent.rag_cutoff,
        "jev": serde_json::from_str::<Value>(&agent.jev_json).unwrap_or_else(|_| json!({})),
        "version": agent.version,
        "created_at": agent.created_at,
        "updated_at": agent.updated_at,
    })
}

fn instructions_json(instructions: &Instructions) -> Value {
    json!({
        "agent_id": instructions.agent_id.to_string(),
        "org_id": instructions.org_id.to_string(),
        "structured": serde_json::from_str::<Value>(&instructions.structured_json)
            .unwrap_or_else(|_| json!(instructions.structured_json)),
        "markdown": instructions.markdown,
        "derived_from_json": instructions.derived_from_json,
        "version": instructions.version,
        "created_at": instructions.created_at,
        "updated_at": instructions.updated_at,
    })
}

fn format_structured_json(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| text.to_owned())
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut short_text: String = text.chars().take(limit.saturating_sub(1)).collect();
    short_text.push('…');
    short_text
}

fn agent_success(agent: &Agent, json: bool, message: &str) -> ExitCode {
    if json {
        print_json(&json!({
            "versao_relatorio": 1,
            "mensagem": message,
            "agent": agent_json(agent),
        }));
    } else {
        println!("{message}\n");
        print!("{}", agent_text(agent, &[]));
    }
    ExitCode::SUCCESS
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

fn failure(message: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", message.as_ref());
    ExitCode::from(1)
}

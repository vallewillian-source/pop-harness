//! `poph agent` e `poph instructions`: configuração do agente pela CLI.
//!
//! Cobre a entidade agente (campos `V1` da PARIDADE_AGENTE.md), o vínculo agente↔categorias de
//! produto e as instruções estruturadas com o texto composto pelo compositor (`poph-core`).
//!
//! Regras que valem em todos os subcomandos:
//! - `--org` (UUID ou slug) é obrigatório: não existe acesso sem o escopo da org;
//! - escrita de agente e de instruções exige `--if-match` com a versão lida (exceto o `create`);
//! - agente é referenciado por id (UUID v7) ou por slug;
//! - `--json` devolve um formato estável, sem segredo nenhum.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_agent::compose_instructions;
use poph_store::{AgentChange, Agente, Instructions, NewAgent, Page, Status, Store};
use poph_types::{
    AgentId, DepartmentId, HarnessConfig, OrgId, ProductCategoryId, StructuredInstructions,
};
use serde_json::{Value, json};

use crate::config;

/// Subcomandos de `poph agent`.
#[derive(Debug, Subcommand)]
pub enum AcaoAgente {
    /// Cria o agente na org.
    Create(CriarAgente),
    /// Lista os agentes da org (paginado por id).
    List(ListarAgentes),
    /// Mostra um agente, as categorias e o texto composto das instruções.
    Show(ReferenciaAgente),
    /// Altera um agente com `If-Match`; também define as categorias de produto.
    Update(AtualizarAgente),
    /// Exclui um agente (instruções, vínculos e conversas saem junto).
    Delete(ReferenciaAgente),
}

/// Subcomandos de `poph instructions`.
#[derive(Debug, Subcommand)]
pub enum AcaoInstrucoes {
    /// Grava as instruções do agente (o agente não pode ter instruções ainda).
    Create(GravarInstrucoes),
    /// Altera as instruções existentes com `If-Match`.
    Update(GravarInstrucoes),
    /// Mostra o JSON estruturado e o texto composto.
    Show(ReferenciaAgente),
    /// Apaga as instruções do agente.
    Delete(ReferenciaAgente),
    /// Lista as instruções da org (um registro por agente que já tem instruções).
    List(ListarInstrucoes),
}

/// Referência a um agente por id ou slug.
#[derive(Debug, Args)]
pub struct ReferenciaAgente {
    /// Id (UUID v7) ou slug do agente.
    pub agente: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph agent create`.
#[derive(Debug, Args)]
pub struct CriarAgente {
    /// Nome de exibição (3 a 120 caracteres).
    #[arg(long)]
    pub name: String,

    /// Slug único na org (`^[a-z0-9-]{3,64}$`). Sem ele, é derivado do nome.
    #[arg(long)]
    pub slug: Option<String>,

    /// Descrição curta.
    #[arg(long, default_value = "")]
    pub description: String,

    /// Tipo: `custom` (padrão) ou `standard` (paridade com o POP Atende).
    #[arg(long = "type", default_value = "custom")]
    pub tipo: String,

    /// Categoria do POP Atende (metadado; `generic` por padrão).
    #[arg(long, default_value = "generic")]
    pub category: String,

    /// Departamento padrão (id): onde as conversas novas entram.
    #[arg(long)]
    pub department: Option<DepartmentId>,

    /// Departamento humano do gate G4.
    #[arg(long)]
    pub handoff_department: Option<DepartmentId>,

    /// Provedor do LLM.
    #[arg(long, default_value = "antigravity2api")]
    pub provider: String,

    /// Modelo do LLM (vale por agente).
    #[arg(long, default_value = "gemini-3.8-flash-tiered")]
    pub model: String,

    /// Temperatura do LLM (sem valor, usa o padrão do provedor).
    #[arg(long)]
    pub temperature: Option<f64>,

    /// Teto de tokens de saída (sem valor, usa o padrão da configuração).
    #[arg(long)]
    pub max_output_tokens: Option<i64>,

    /// Orçamento de contexto em tokens.
    #[arg(long, default_value_t = 64_000)]
    pub max_context_tokens: u32,

    /// Janela de agrupamento em ms (0 a 8000).
    #[arg(long, default_value_t = 2_000)]
    pub grouping_window_ms: u32,

    /// Máximo de rodadas de ferramenta por turno.
    #[arg(long, default_value_t = 4)]
    pub max_tool_rounds: u32,

    /// Máximo de chamadas de ferramenta por mensagem.
    #[arg(long, default_value_t = 10)]
    pub max_tool_calls: u32,

    /// Quantos trechos o RAG devolve.
    #[arg(long, default_value_t = 3)]
    pub rag_top_k: u32,

    /// Corte mínimo de relevância do RAG (0 a 1).
    #[arg(long, default_value_t = 0.0)]
    pub rag_min_score: f64,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph agent list`.
#[derive(Debug, Args)]
pub struct ListarAgentes {
    /// Quantos agentes trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Só os agentes que podem abrir turno (`active`).
    #[arg(long)]
    pub only_active: bool,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph agent update`.
#[derive(Debug, Args)]
pub struct AtualizarAgente {
    /// Id (UUID v7) ou slug do agente.
    pub agente: String,

    /// Versão lida (`If-Match`). Obrigatória.
    #[arg(long)]
    pub if_match: u64,

    /// Novo nome.
    #[arg(long)]
    pub name: Option<String>,

    /// Nova descrição.
    #[arg(long)]
    pub description: Option<String>,

    /// Nova situação: `active`, `inactive` ou `archived` (D-05: só `active` abre turno).
    #[arg(long)]
    pub status: Option<String>,

    /// Novo departamento padrão (id).
    #[arg(long)]
    pub department: Option<DepartmentId>,

    /// Liga/desliga o gate G1 de transferência (`--handoff true|false`).
    #[arg(long)]
    pub handoff: Option<bool>,

    /// Novo departamento humano do gate G4 (id).
    #[arg(long)]
    pub handoff_department: Option<DepartmentId>,

    /// Novo provedor do LLM.
    #[arg(long)]
    pub provider: Option<String>,

    /// Novo modelo do LLM.
    #[arg(long)]
    pub model: Option<String>,

    /// Nova temperatura.
    #[arg(long)]
    pub temperature: Option<f64>,

    /// Novo teto de tokens de saída.
    #[arg(long)]
    pub max_output_tokens: Option<i64>,

    /// Novo orçamento de contexto em tokens.
    #[arg(long)]
    pub max_context_tokens: Option<u32>,

    /// Nova janela de agrupamento em ms.
    #[arg(long)]
    pub grouping_window_ms: Option<u32>,

    /// Novo máximo de rodadas de ferramenta.
    #[arg(long)]
    pub max_tool_rounds: Option<u32>,

    /// Novo máximo de chamadas de ferramenta por mensagem.
    #[arg(long)]
    pub max_tool_calls: Option<u32>,

    /// Novo top-k do RAG.
    #[arg(long)]
    pub rag_top_k: Option<u32>,

    /// Novo corte mínimo do RAG.
    #[arg(long)]
    pub rag_min_score: Option<f64>,

    /// Flags do Jev do agente (JSON).
    #[arg(long)]
    pub jev_json: Option<String>,

    /// Substitui as categorias de produto do agente (repita a opção; sem ela, não mexe).
    #[arg(long = "category-id")]
    pub categorias: Option<Vec<ProductCategoryId>>,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph instructions create|update`.
#[derive(Debug, Args)]
pub struct GravarInstrucoes {
    /// Id (UUID v7) ou slug do agente.
    pub agente: String,

    /// Versão lida da instrução (`If-Match`). Obrigatória no `update`.
    #[arg(long)]
    pub if_match: Option<u64>,

    /// Arquivo JSON com o `StructuredInstructions`. O texto é recomposto pelo compositor.
    #[arg(long, value_name = "ARQUIVO", conflicts_with = "markdown_file")]
    pub file: Option<PathBuf>,

    /// Arquivo markdown com o prompt editado à mão. Sem recomposição (`derived_from_json = 0`).
    #[arg(long, value_name = "ARQUIVO")]
    pub markdown_file: Option<PathBuf>,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph instructions list`.
#[derive(Debug, Args)]
pub struct ListarInstrucoes {
    /// Quantos registros trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

// ───────────────────────────────── Execução ─────────────────────────────────

/// Executa um subcomando de `poph agent`.
pub fn executar_agente(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
    acao: AcaoAgente,
) -> ExitCode {
    match acao {
        AcaoAgente::Create(args) => {
            let (store, org, configuracao) = match abrir(flag, ambiente, org) {
                Ok(par) => par,
                Err(erro) => return falha(&erro),
            };
            criar(&store, org, &configuracao, args)
        }
        AcaoAgente::List(args) => {
            let (store, org, _) = match abrir(flag, ambiente, org) {
                Ok(par) => par,
                Err(erro) => return falha(&erro),
            };
            listar(&store, org, args)
        }
        AcaoAgente::Show(args) => {
            let (store, org, _) = match abrir(flag, ambiente, org) {
                Ok(par) => par,
                Err(erro) => return falha(&erro),
            };
            mostrar(&store, org, args)
        }
        AcaoAgente::Update(args) => {
            let (store, org, configuracao) = match abrir(flag, ambiente, org) {
                Ok(par) => par,
                Err(erro) => return falha(&erro),
            };
            atualizar(&store, org, &configuracao, args)
        }
        AcaoAgente::Delete(args) => {
            let (store, org, _) = match abrir(flag, ambiente, org) {
                Ok(par) => par,
                Err(erro) => return falha(&erro),
            };
            excluir(&store, org, args)
        }
    }
}

/// Executa um subcomando de `poph instructions`.
pub fn executar_instrucoes(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
    acao: AcaoInstrucoes,
) -> ExitCode {
    let (store, org, _) = match abrir(flag, ambiente, org) {
        Ok(par) => par,
        Err(erro) => return falha(&erro),
    };
    match acao {
        AcaoInstrucoes::Create(args) => gravar_instrucoes(&store, org, args, 0),
        AcaoInstrucoes::Update(args) => {
            let Some(esperada) = args.if_match else {
                return falha("informe --if-match com a versão lida das instruções");
            };
            gravar_instrucoes(&store, org, args, esperada)
        }
        AcaoInstrucoes::Show(args) => mostrar_instrucoes(&store, org, args),
        AcaoInstrucoes::Delete(args) => apagar_instrucoes(&store, org, args),
        AcaoInstrucoes::List(args) => listar_instrucoes(&store, org, args),
    }
}

fn criar(store: &Store, org: OrgId, configuracao: &HarnessConfig, args: CriarAgente) -> ExitCode {
    if args.name.trim().len() < 3 {
        return falha("o nome do agente precisa de ao menos 3 caracteres");
    }
    if args.tipo != "custom" && args.tipo != "standard" {
        return falha("--type aceita `custom` ou `standard`");
    }
    if let Err(erro) = configuracao.validate_provider_choice(&args.provider) {
        return falha(format!("--provider: {erro}"));
    }
    let slug = match args.slug.clone() {
        Some(slug) => slug,
        None => derivar_slug(&args.name),
    };
    if !slug_valido(&slug) {
        return falha(format!(
            "slug `{slug}` inválido: use 3 a 64 caracteres de a-z, 0-9 e `-`"
        ));
    }
    let novo = NewAgent {
        name: args.name,
        slug,
        description: args.description,
        agent_type: args.tipo,
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
    match store.agents().create(org, novo) {
        Ok(agente) => {
            if args.handoff_department.is_some() {
                let mudanca = AgentChange {
                    handoff_active: Some(true),
                    handoff_department: args.handoff_department,
                    ..AgentChange::default()
                };
                match store
                    .agents()
                    .update(org, agente.id, agente.version, mudanca)
                {
                    Ok(agente) => return sucesso_agente(&agente, args.json, "agente criado"),
                    Err(erro) => {
                        return falha(format!("agente criado, mas o handoff falhou: {erro}"));
                    }
                }
            }
            sucesso_agente(&agente, args.json, "agente criado")
        }
        Err(erro) => falha(format!("não consegui criar o agente: {erro}")),
    }
}

fn listar(store: &Store, org: OrgId, args: ListarAgentes) -> ExitCode {
    let agentes = if args.only_active {
        store.agents().list_active(org)
    } else {
        store.agents().list(org, Page::with_limit(args.limit))
    };
    match agentes {
        Ok(agentes) => {
            if args.json {
                let itens: Vec<Value> = agentes.iter().map(json_agente).collect();
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": itens.len(),
                    "agentes": itens,
                }));
            } else if agentes.is_empty() {
                println!("Nenhum agente na org.");
            } else {
                for agente in &agentes {
                    println!("{}", linha_resumo(agente));
                }
                println!("\n{} agente(s).", agentes.len());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(format!("não consegui listar os agentes: {erro}")),
    }
}

fn mostrar(store: &Store, org: OrgId, args: ReferenciaAgente) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agente) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    let categorias = match store.agents().categories(org, agente.id) {
        Ok(categorias) => categorias,
        Err(erro) => return falha(format!("não consegui ler as categorias: {erro}")),
    };
    let instrucoes = store.instructions().get(org, agente.id).ok();

    if args.json {
        let mut valor = json_agente(&agente);
        if let Value::Object(mapa) = &mut valor {
            mapa.insert(
                "product_category_ids".to_owned(),
                json!(
                    categorias
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                ),
            );
            mapa.insert(
                "instructions".to_owned(),
                instrucoes
                    .as_ref()
                    .map(json_instrucoes)
                    .unwrap_or(Value::Null),
            );
        }
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "org_id": org.to_string(),
            "agent": valor,
        }));
        return ExitCode::SUCCESS;
    }

    print!("{}", texto_agente(&agente, &categorias));
    match instrucoes {
        Some(instrucoes) => {
            println!(
                "\ninstruções (versão {}, derivado do JSON: {})",
                instrucoes.version, instrucoes.derived_from_json
            );
            if instrucoes.markdown.is_empty() {
                println!("(texto composto vazio)");
            } else {
                println!("\n{}", instrucoes.markdown);
            }
        }
        None => println!("\nsem instruções gravadas (use `poph instructions create`)."),
    }
    ExitCode::SUCCESS
}

fn atualizar(
    store: &Store,
    org: OrgId,
    configuracao: &HarnessConfig,
    args: AtualizarAgente,
) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agente) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    let status = match args.status.as_deref() {
        Some(texto) => match Status::from_text(texto) {
            Ok(status) => Some(status),
            Err(erro) => return falha(format!("--status inválido: {erro}")),
        },
        None => None,
    };
    if args.jev_json.is_some() {
        // Valida o JSON antes de mandar para o banco.
        if let Some(texto) = &args.jev_json
            && serde_json::from_str::<Value>(texto).is_err()
        {
            return falha("--jev-json precisa ser um JSON válido");
        }
    }
    if let Some(escolha) = args.provider.as_deref()
        && let Err(erro) = configuracao.validate_provider_choice(escolha)
    {
        return falha(format!("--provider: {erro}"));
    }
    let tem_categorias = args.categorias.is_some();
    let tem_mudanca = args.name.is_some()
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
    let mudanca = AgentChange {
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
    let mut atual = agente;
    if tem_mudanca {
        atual = match store.agents().update(org, atual.id, args.if_match, mudanca) {
            Ok(atual) => atual,
            Err(erro) => return falha(format!("não consegui alterar o agente: {erro}")),
        };
    }
    if let Some(categorias) = args.categorias {
        atual = match store
            .agents()
            .define_categories(org, atual.id, atual.version, &categorias)
        {
            Ok(atual) => atual,
            Err(erro) => return falha(format!("não consegui definir as categorias: {erro}")),
        };
    }
    if !tem_mudanca && !tem_categorias {
        return falha("nada a alterar: informe ao menos um campo");
    }
    sucesso_agente(&atual, args.json, "agente alterado")
}

fn excluir(store: &Store, org: OrgId, args: ReferenciaAgente) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agente) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    match store.agents().delete(org, agente.id) {
        Ok(()) => {
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "removido": true,
                    "agent_id": agente.id.to_string(),
                }));
            } else {
                println!("agente {} removido", agente.id);
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(format!("não consegui excluir o agente: {erro}")),
    }
}

fn gravar_instrucoes(store: &Store, org: OrgId, args: GravarInstrucoes, esperada: u64) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agente) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    let (estruturado, markdown, derivado) = match (&args.file, &args.markdown_file) {
        (Some(arquivo), None) => {
            let texto = match std::fs::read_to_string(arquivo) {
                Ok(texto) => texto,
                Err(erro) => {
                    return falha(format!("não consegui ler {}: {erro}", arquivo.display()));
                }
            };
            let dados = match StructuredInstructions::from_tolerant_json(&texto) {
                Ok(dados) => dados,
                Err(erro) => return falha(format!("JSON de instruções inválido: {erro}")),
            };
            if let Err(erro) = dados.validar() {
                return falha(erro.to_string());
            }
            let markdown = compose_instructions(&dados);
            (dados.to_json(), markdown, true)
        }
        (None, Some(arquivo)) => {
            let markdown = match std::fs::read_to_string(arquivo) {
                Ok(texto) => texto,
                Err(erro) => {
                    return falha(format!("não consegui ler {}: {erro}", arquivo.display()));
                }
            };
            let estruturado = store
                .instructions()
                .get(org, agente.id)
                .map(|instrucoes| instrucoes.structured_json)
                .unwrap_or_else(|_| "{}".to_owned());
            (estruturado, markdown, false)
        }
        (Some(_), Some(_)) => {
            return falha(
                "use --file (JSON estruturado) ou --markdown-file (prompt manual), não os dois",
            );
        }
        (None, None) => {
            return falha(
                "informe --file com o JSON estruturado ou --markdown-file com o prompt manual",
            );
        }
    };

    match store
        .instructions()
        .save(org, agente.id, esperada, &estruturado, &markdown, derivado)
    {
        Ok(instrucoes) => {
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "instructions": json_instrucoes(&instrucoes),
                }));
            } else {
                println!(
                    "instruções gravadas: agente {} · versão {} · derivado do JSON: {}",
                    agente.id, instrucoes.version, instrucoes.derived_from_json
                );
                println!("\n{}", instrucoes.markdown);
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(format!("não consegui gravar as instruções: {erro}")),
    }
}

fn mostrar_instrucoes(store: &Store, org: OrgId, args: ReferenciaAgente) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agente) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    match store.instructions().get(org, agente.id) {
        Ok(instrucoes) => {
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "instructions": json_instrucoes(&instrucoes),
                }));
            } else {
                println!(
                    "agente {} · versão {} · derivado do JSON: {}",
                    agente.id, instrucoes.version, instrucoes.derived_from_json
                );
                println!(
                    "\nestruturado:\n{}",
                    estruturar_para_exibir(&instrucoes.structured_json)
                );
                println!("\nmarkdown:\n{}", instrucoes.markdown);
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(format!("não consegui ler as instruções: {erro}")),
    }
}

fn apagar_instrucoes(store: &Store, org: OrgId, args: ReferenciaAgente) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agente) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    match store.instructions().delete(org, agente.id) {
        Ok(()) => {
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "removido": true,
                    "agent_id": agente.id.to_string(),
                }));
            } else {
                println!("instruções do agente {} removidas", agente.id);
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(format!("não consegui apagar as instruções: {erro}")),
    }
}

fn listar_instrucoes(store: &Store, org: OrgId, args: ListarInstrucoes) -> ExitCode {
    match store.instructions().list(org, Page::with_limit(args.limit)) {
        Ok(instrucoes) => {
            if args.json {
                let itens: Vec<Value> = instrucoes.iter().map(json_instrucoes).collect();
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": itens.len(),
                    "instructions": itens,
                }));
            } else if instrucoes.is_empty() {
                println!("Nenhuma instrução gravada na org.");
            } else {
                for item in &instrucoes {
                    println!(
                        "{} · versão {} · derivado do JSON: {}",
                        item.agent_id, item.version, item.derived_from_json
                    );
                }
                println!("\n{} registro(s).", instrucoes.len());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(format!("não consegui listar as instruções: {erro}")),
    }
}

// ───────────────────────────────── Apoio ─────────────────────────────────

/// Abre a configuração e o banco, e resolve a org do contexto.
fn abrir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
) -> Result<(Store, OrgId, HarnessConfig), String> {
    let Some(texto_org) = org else {
        return Err("informe --org com o id (UUID v7) ou o slug da org".to_owned());
    };
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao = config::carregar(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    let store =
        Store::open(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))?;
    let org = resolver_org(&store, texto_org)?;
    Ok((store, org, configuracao))
}

fn resolver_org(store: &Store, texto: &str) -> Result<OrgId, String> {
    if let Ok(id) = texto.parse::<OrgId>() {
        return store
            .orgs()
            .get(id)
            .map(|org| org.id)
            .map_err(|erro| format!("org `{texto}`: {erro}"));
    }
    store
        .orgs()
        .by_slug(texto)
        .map(|org| org.id)
        .map_err(|erro| format!("org `{texto}`: {erro}"))
}

fn resolver_agente(store: &Store, org: OrgId, texto: &str) -> Result<Agente, String> {
    if let Ok(id) = texto.parse::<AgentId>() {
        return store
            .agents()
            .get(org, id)
            .map_err(|erro| format!("agente `{texto}`: {erro}"));
    }
    store
        .agents()
        .by_slug(org, texto)
        .map_err(|erro| format!("agente `{texto}`: {erro}"))
}

pub(crate) fn derivar_slug(nome: &str) -> String {
    let mut slug = String::with_capacity(nome.len());
    let mut ultimo_traco = false;
    for caractere in nome.chars().flat_map(char::to_lowercase).map(sem_acento) {
        if caractere.is_ascii_alphanumeric() {
            slug.push(caractere);
            ultimo_traco = false;
        } else if !ultimo_traco && !slug.is_empty() {
            slug.push('-');
            ultimo_traco = true;
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

/// Reduz as vogais acentuadas do português ao ASCII, para o slug não perder a letra.
fn sem_acento(caractere: char) -> char {
    match caractere {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        outro => outro,
    }
}

pub(crate) fn slug_valido(slug: &str) -> bool {
    let tamanho = slug.chars().count();
    (3..=64).contains(&tamanho)
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn linha_resumo(agente: &Agente) -> String {
    let situacao = if agente.can_open_turn() {
        "responde"
    } else {
        "não responde"
    };
    format!(
        "{}  {:<24} slug={:<24} status={:<8} modelo={}",
        agente.id,
        truncar(&agente.name, 24),
        agente.slug,
        situacao,
        agente.modelo
    )
}

fn texto_agente(agente: &Agente, categorias: &[ProductCategoryId]) -> String {
    let departamento = agente
        .default_department
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let handoff = agente
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
        agente.id,
        agente.name,
        agente.slug,
        agente.description,
        agente.status.as_text(),
        if agente.can_open_turn() {
            "abre turno"
        } else {
            "não abre turno (D-05)"
        },
        agente.agent_type,
        agente.category,
        departamento,
        if agente.handoff_active {
            "ligado"
        } else {
            "desligado"
        },
        handoff,
        agente.modelo,
        agente.provedor,
        agente
            .temperatura
            .map(|valor| valor.to_string())
            .unwrap_or_else(|| "padrão".to_owned()),
        agente
            .max_tokens_saida
            .map(|valor| valor.to_string())
            .unwrap_or_else(|| "padrão".to_owned()),
        agente.max_context_tokens,
        agente.max_context_messages,
        agente.grouping_window_ms,
        agente.max_tool_rounds,
        agente.max_tools_per_message,
        agente.rag_top_k,
        agente.rag_cutoff,
        if categorias.is_empty() {
            "nenhuma".to_owned()
        } else {
            categorias
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        },
        agente.version
    )
}

fn json_agente(agente: &Agente) -> Value {
    json!({
        "id": agente.id.to_string(),
        "org_id": agente.org_id.to_string(),
        "name": agente.name,
        "slug": agente.slug,
        "description": agente.description,
        "status": agente.status.as_text(),
        "abre_turno": agente.can_open_turn(),
        "agent_type": agente.agent_type,
        "category": agente.category,
        "default_department_id": agente.default_department.map(|id| id.to_string()),
        "human_handoff_enabled": agente.handoff_active,
        "human_handoff_department_id": agente.handoff_department.map(|id| id.to_string()),
        "provider": agente.provedor,
        "model": agente.modelo,
        "temperature": agente.temperatura,
        "max_output_tokens": agente.max_tokens_saida,
        "max_context_messages": agente.max_context_messages,
        "max_context_tokens": agente.max_context_tokens,
        "grouping_window_ms": agente.grouping_window_ms,
        "max_tool_rounds": agente.max_tool_rounds,
        "max_tool_calls_per_message": agente.max_tools_per_message,
        "rag_top_k": agente.rag_top_k,
        "rag_min_score": agente.rag_cutoff,
        "jev": serde_json::from_str::<Value>(&agente.jev_json).unwrap_or_else(|_| json!({})),
        "version": agente.version,
        "created_at": agente.created_at,
        "updated_at": agente.updated_at,
    })
}

fn json_instrucoes(instrucoes: &Instructions) -> Value {
    json!({
        "agent_id": instrucoes.agent_id.to_string(),
        "org_id": instrucoes.org_id.to_string(),
        "structured": serde_json::from_str::<Value>(&instrucoes.structured_json)
            .unwrap_or_else(|_| json!(instrucoes.structured_json)),
        "markdown": instrucoes.markdown,
        "derived_from_json": instrucoes.derived_from_json,
        "version": instrucoes.version,
        "created_at": instrucoes.created_at,
        "updated_at": instrucoes.updated_at,
    })
}

fn estruturar_para_exibir(texto: &str) -> String {
    serde_json::from_str::<Value>(texto)
        .ok()
        .and_then(|valor| serde_json::to_string_pretty(&valor).ok())
        .unwrap_or_else(|| texto.to_owned())
}

fn truncar(texto: &str, limite: usize) -> String {
    if texto.chars().count() <= limite {
        return texto.to_owned();
    }
    let mut curto: String = texto.chars().take(limite.saturating_sub(1)).collect();
    curto.push('…');
    curto
}

fn sucesso_agente(agente: &Agente, json: bool, mensagem: &str) -> ExitCode {
    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "mensagem": mensagem,
            "agent": json_agente(agente),
        }));
    } else {
        println!("{mensagem}\n");
        print!("{}", texto_agente(agente, &[]));
    }
    ExitCode::SUCCESS
}

fn imprimir_json(valor: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(valor).unwrap_or_else(|_| valor.to_string())
    );
}

fn falha(mensagem: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", mensagem.as_ref());
    ExitCode::from(1)
}

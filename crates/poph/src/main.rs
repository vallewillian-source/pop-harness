//! Binário `poph`: CLI e servidor num só executável.
//!
//! Hoje: `--version`, `--help`, `doctor`, `migrate`, `model install`, `agent`, `instructions`,
//! `org`, `token`, `category`, `department`, `kb`, `rag`, `product`, `file`, `chat`, `flow`, `ficha`,
//! `retention`, `backup`, `jev`, `serve` e `openapi`. Saída: 0 quando está tudo certo; 1 quando há erro a
//! corrigir; 2 sem comando.

mod agente;
mod apoio;
mod arquivo;
mod backup;
mod categoria;
mod chat;
mod cluster;
mod config;
mod departamento;
mod doctor;
mod ficha;
mod flow;
mod jev;
mod kb;
mod logs;
mod matrix;
mod matrix_admin;
mod modelo;
mod montagem;
mod org;
mod produto;
mod rag;
mod retencao;
mod servidor;
mod token;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, CommandFactory, Parser, Subcommand};

/// Interface de linha de comando do POP Harness.
#[derive(Debug, Parser)]
#[command(
    name = "poph",
    version,
    about = "POP Harness: motor de agentes multi-tenant",
    arg_required_else_help = true
)]
struct Cli {
    /// Arquivo de configuração TOML. Sem ele, usa POPH_CONFIG ou ./poph.toml, se existir.
    #[arg(long, global = true, value_name = "ARQUIVO")]
    config: Option<PathBuf>,

    /// Org do contexto (id UUID v7 ou slug). Obrigatória nos comandos que mexem em dados de org.
    #[arg(long, global = true, value_name = "ORG")]
    org: Option<String>,

    /// Opera direto no banco, sem servidor. Obrigatório em `org`, `token`, `category`,
    /// `department`, `kb` e `file` enquanto o modo com servidor não existe.
    #[arg(long, global = true)]
    offline: bool,

    #[command(subcommand)]
    comando: Option<Comando>,
}

/// Subcomandos disponíveis.
// A variante da CLI carrega os argumentos de todos os subcomandos, e eles crescem a cada tarefa;
// o valor vive só no `main` (nunca em vetor), então a diferença de tamanho não custa nada.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
enum Comando {
    /// Confere o ambiente: configuração, pasta de dados, bancos e variáveis obrigatórias.
    Doctor(DoctorArgs),
    /// Cria ou atualiza os bancos (`harness.db` e `rag.db`). Rode com o servidor parado.
    Migrate(MigrateArgs),
    /// Gerencia o modelo de embedding (`poph model install` baixa e confere o modelo).
    #[command(subcommand)]
    Model(modelo::AcaoModelo),
    /// Gerencia agentes: identidade, modelo, parâmetros, handoff e categorias.
    #[command(subcommand)]
    Agent(agente::AcaoAgente),
    /// Gerencia as instruções estruturadas e o texto composto do agente.
    #[command(subcommand)]
    Instructions(agente::AcaoInstrucoes),
    /// Gerencia as orgs (tenants): criar, listar, mostrar, alterar e excluir.
    #[command(subcommand)]
    Org(org::AcaoOrg),
    /// Gerencia os tokens de administração de uma org (`--org`).
    #[command(subcommand)]
    Token(token::AcaoToken),
    /// Gerencia as categorias de produto de uma org (`--org`).
    #[command(subcommand)]
    Category(categoria::AcaoCategoria),
    /// Gerencia os departamentos de uma org (`--org`).
    #[command(subcommand)]
    Department(departamento::AcaoDepartamento),
    /// Base de conhecimento: grupos, itens, vínculo com agentes e índice (HR-17).
    #[command(subcommand)]
    Kb(kb::AcaoKb),
    /// Índice da busca semântica: `poph rag reindex` (mesmo comando do `poph kb reindex`).
    #[command(subcommand)]
    Rag(rag::AcaoRag),
    /// Produtos do catálogo: criar, listar, alterar, excluir e reindexar (HR-18).
    #[command(subcommand)]
    Product(produto::AcaoProduto),
    /// Biblioteca de arquivos do agente: enviar, listar, mostrar e excluir (HR-24).
    #[command(subcommand)]
    File(arquivo::AcaoArquivo),
    /// Conversa com um agente pelo terminal (streaming; Ctrl-C cancela o turno).
    Chat(chat::ChatArgs),
    /// Jev (decisões rápidas): `eval` mede um gate sobre um conjunto rotulado (HR-28).
    #[command(subcommand)]
    Jev(jev::AcaoJev),
    /// Fluxos da org: validar, publicar, listar, mostrar, comparar, formatar e liberar a agentes.
    #[command(subcommand)]
    Flow(flow::AcaoFluxo),
    /// Fichas da org (um atendimento por Ficha): listar e mostrar, com as execuções de fluxo.
    #[command(subcommand)]
    Ficha(ficha::AcaoFicha),
    /// Retenção do histórico: apaga o que venceu (`run`) e mostra a configuração (`show`).
    #[command(subcommand)]
    Retention(retencao::AcaoRetencao),
    /// Exporta uma org inteira num arquivo único e reconstrói a partir dele (HR-53).
    #[command(subcommand)]
    Backup(backup::AcaoBackup),
    /// Roteamento entre instâncias: mostra e confere o `cluster.toml` (HR-50a).
    #[command(subcommand)]
    Cluster(cluster::AcaoCluster),
    /// Módulo Matrix: registro do Application Service (`registro`) e, por org (`--offline --org`),
    /// `estado`, `ligar`, `desligar` e `espaco`.
    #[command(subcommand)]
    Matrix(matrix::AcaoMatrix),
    /// Sobe o servidor HTTP (API administrativa, `/healthz` e `/metrics`).
    Serve(servidor::ServeArgs),
    /// Imprime o documento OpenAPI (base do cliente TypeScript do Console).
    Openapi(servidor::OpenapiArgs),
}

/// Opções do `doctor`.
#[derive(Debug, Args)]
struct DoctorArgs {
    /// Saída em JSON (formato estável, sem segredos).
    #[arg(long)]
    json: bool,
}

/// Opções do `migrate`.
#[derive(Debug, Args)]
struct MigrateArgs {
    /// Saída em JSON (versões antes/depois e migrações aplicadas).
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(comando) = cli.comando else {
        let _ = Cli::command().print_help();
        println!();
        return ExitCode::from(2);
    };
    let ambiente = ambiente_do_processo();
    match comando {
        Comando::Doctor(args) => executar_doctor(cli.config, &ambiente, args),
        Comando::Migrate(args) => executar_migrate(cli.config, &ambiente, args),
        Comando::Model(acao) => modelo::executar(cli.config, &ambiente, acao),
        Comando::Agent(acao) => {
            agente::executar_agente(cli.config, &ambiente, cli.org.as_deref(), acao)
        }
        Comando::Instructions(acao) => {
            agente::executar_instrucoes(cli.config, &ambiente, cli.org.as_deref(), acao)
        }
        Comando::Org(acao) => org::executar(cli.config, &ambiente, cli.offline, acao),
        Comando::Token(acao) => {
            token::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::Category(acao) => {
            categoria::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::Department(acao) => {
            departamento::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::Jev(acao) => jev::executar(cli.config, &ambiente, acao),
        Comando::Flow(acao) => flow::executar(cli.config, &ambiente, cli.org.as_deref(), acao),
        Comando::Ficha(acao) => ficha::executar(cli.config, &ambiente, cli.org.as_deref(), acao),
        Comando::Retention(acao) => {
            retencao::executar(cli.config, &ambiente, cli.org.as_deref(), acao)
        }
        Comando::Backup(acao) => backup::executar(cli.config, &ambiente, cli.offline, acao),
        Comando::Cluster(acao) => cluster::executar(cli.config, &ambiente, acao),
        Comando::Matrix(acao) => {
            matrix::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::Kb(acao) => {
            kb::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::Rag(acao) => {
            rag::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::Product(acao) => {
            produto::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::File(acao) => {
            arquivo::executar(cli.config, &ambiente, cli.offline, cli.org.as_deref(), acao)
        }
        Comando::Chat(args) => chat::executar(cli.config, &ambiente, cli.org.as_deref(), args),
        Comando::Serve(args) => servidor::executar(cli.config, &ambiente, args),
        Comando::Openapi(args) => servidor::executar_openapi(args),
    }
}

/// Variáveis do processo como mapa de texto (valores que não são UTF-8 viram texto aproximado).
fn ambiente_do_processo() -> config::Ambiente {
    std::env::vars_os()
        .map(|(chave, valor)| {
            (
                chave.to_string_lossy().into_owned(),
                valor.to_string_lossy().into_owned(),
            )
        })
        .collect()
}

/// `poph migrate`: cria/atualiza os dois bancos e mostra o que foi aplicado.
fn executar_migrate(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    args: MigrateArgs,
) -> ExitCode {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let config = match config::carregar(arquivo.as_deref(), ambiente) {
        Ok(config) => config,
        Err(erro) => {
            eprintln!("{erro}");
            return ExitCode::from(1);
        }
    };

    let relatorios = match poph_store::migrar_bancos(&config.harness_db(), &config.rag_db()) {
        Ok(relatorios) => relatorios,
        Err(erro) => {
            eprintln!("não consegui migrar os bancos: {erro}");
            return ExitCode::from(1);
        }
    };

    if args.json {
        println!("{}", json_migracao(&relatorios));
    } else {
        println!("POP Harness: migração dos bancos\n");
        println!("{}", relatorios.harness.resumo());
        println!("{}", relatorios.rag.resumo());
        if relatorios.ja_estava_em_dia() {
            println!("\nNada a fazer: os dois bancos já estavam em dia.");
        } else {
            println!("\nBancos prontos para uso.");
        }
    }
    ExitCode::SUCCESS
}

/// JSON estável da migração (sem segredo nenhum: só versões, nomes e caminhos).
fn json_migracao(relatorios: &poph_store::RelatoriosMigracao) -> String {
    fn banco(relatorio: &poph_store::RelatorioMigracao) -> serde_json::Value {
        serde_json::json!({
            "banco": relatorio.banco,
            "caminho": relatorio.caminho.display().to_string(),
            "versao_antes": relatorio.versao_antes,
            "versao_depois": relatorio.versao_depois,
            "aplicadas": relatorio.aplicadas.iter().map(|m| serde_json::json!({
                "versao": m.versao,
                "nome": m.nome,
            })).collect::<Vec<_>>(),
        })
    }
    let valor = serde_json::json!({
        "versao_relatorio": 1,
        "harness": banco(&relatorios.harness),
        "rag": banco(&relatorios.rag),
        "ja_estava_em_dia": relatorios.ja_estava_em_dia(),
    });
    serde_json::to_string_pretty(&valor).unwrap_or_else(|_| valor.to_string())
}

fn executar_doctor(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    args: DoctorArgs,
) -> ExitCode {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let carregada = config::carregar(arquivo.as_deref(), ambiente);

    // Se a configuração não carregou, os logs usam o padrão; o erro aparece no relatório.
    let log = carregada
        .as_ref()
        .map(|config| config.log.clone())
        .unwrap_or_default();
    if let Err(erro) = logs::iniciar(&log, ambiente.get(logs::VAR_RUST_LOG).map(String::as_str)) {
        eprintln!("{erro}");
        return ExitCode::from(1);
    }

    let segredos = config::segredos(ambiente);
    let nome_da_chave = carregada
        .as_ref()
        .map(|config| config.llm.chave_env.clone())
        .unwrap_or_else(|_| config::VAR_ANTIGRAVITY.to_owned());
    let chave_llm = config::chave_llm(ambiente, &nome_da_chave);
    let agentes_por_provedor = carregada
        .as_ref()
        .ok()
        .and_then(|config| poph_store::agentes_por_provedor_em(&config.harness_db()).ok());
    let relatorio = doctor::gerar(doctor::Entrada {
        config: carregada.as_ref(),
        arquivo: arquivo.as_deref(),
        segredos: &segredos,
        chave_llm: chave_llm.as_ref(),
        ambiente: Some(ambiente),
        agentes_por_provedor: agentes_por_provedor.as_deref(),
    });

    if args.json {
        println!("{}", relatorio.json());
    } else {
        print!("{}", relatorio.texto());
    }

    if relatorio.tem_erro() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

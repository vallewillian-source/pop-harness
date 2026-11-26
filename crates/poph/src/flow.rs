//! `poph flow`: fluxos da org pela linha de comando (FX-31, PRD §7.3).
//!
//! Subcomandos: `validar`, `publicar`, `listar`, `mostrar`, `diff`, `formatar`, `liberar` e `retirar`.
//! Operam direto no banco, como os outros comandos de org. Compilar, validar, publicar, comparar e
//! formatar são as funções que a API chama (`poph_server::fluxos_servico`): a CLI não tem regra
//! própria.
//!
//! `--org` é obrigatório nos comandos que leem ou gravam dados da org. `validar` só precisa dele para
//! resolver departamentos, e `formatar` nunca precisa.
//!
//! Saída: 0 quando deu certo; 1 quando há erro de negócio (script inválido, versão velha, fluxo ou
//! agente inexistente...); 2 uso incorreto (o clap decide). Com `--json`, a saída tem
//! `versao_relatorio` = 1 e as mesmas chaves que a API devolve.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_server::fluxos_servico::{
    Ator, CacheRevisoes, Diagnostico, FalhaFluxo, Operacoes, auditar, json_comparacao, json_fluxo,
    json_fluxo_com_rascunho, json_revisao_resumo, json_validacao,
};
use poph_store::{FiltroFluxos, Store};
use poph_types::{HarnessConfig, OrgId};
use serde_json::{Value, json};

use crate::apoio::{
    abrir_com_configuracao, descrever, falha, imprimir_json, pagina_de, resolver_agente,
    resolver_fluxo, resolver_org, status_de_texto,
};
use crate::config;

/// Versão do formato JSON desta saída (muda só se a forma mudar).
const VERSAO_RELATORIO: u64 = 1;

/// Subcomandos de `poph flow`.
#[derive(Debug, Subcommand)]
pub enum AcaoFluxo {
    /// Compila e valida um script, sem gravar nada. Sai com 1 se há erro.
    Validar(ArquivoArgs),
    /// Publica o script como revisão nova. Se o fluxo ainda não existe, cria e publica a revisão 1.
    Publicar(PublicarArgs),
    /// Lista os fluxos da org (primeira página, em ordem de id).
    Listar(ListarArgs),
    /// Mostra um fluxo: o rascunho e a revisão que vale hoje. Aceita id ou slug.
    Mostrar(ReferenciaArgs),
    /// Compara duas revisões: o que mudou e quantos atendimentos estão em cada uma.
    Diff(DiffArgs),
    /// Mostra o script na forma canônica. Com `--escrever`, grava no próprio arquivo.
    Formatar(FormatarArgs),
    /// Libera o fluxo a um agente (o agente só usa fluxos liberados a ele).
    Liberar(PermissaoArgs),
    /// Tira a permissão do fluxo de um agente.
    Retirar(PermissaoArgs),
}

/// Opções que só leem um arquivo de script.
#[derive(Debug, Args)]
pub struct ArquivoArgs {
    /// Arquivo do script do fluxo.
    pub arquivo: PathBuf,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Opções de `publicar`.
#[derive(Debug, Args)]
pub struct PublicarArgs {
    /// Arquivo do script a publicar.
    pub arquivo: PathBuf,

    /// Versão do fluxo lida com `mostrar` (`If-Match`). Obrigatória quando o fluxo já existe.
    #[arg(long)]
    pub if_match: Option<u64>,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Opções de `listar`.
#[derive(Debug, Args)]
pub struct ListarArgs {
    /// Situação: `active` ou `inactive`. Sem ele, lista todos.
    #[arg(long)]
    pub status: Option<String>,

    /// Quantos fluxos trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// `next_cursor` da página anterior.
    #[arg(long)]
    pub cursor: Option<String>,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Referência a um fluxo, pelo id ou pelo slug.
#[derive(Debug, Args)]
pub struct ReferenciaArgs {
    /// Id (UUID v7) ou slug do fluxo.
    pub fluxo: String,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Opções de `diff`.
#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Id (UUID v7) ou slug do fluxo.
    pub fluxo: String,

    /// Número da revisão antiga. Padrão: a revisão nova menos 1.
    #[arg(long)]
    pub de: Option<u64>,

    /// Número da revisão nova. Padrão: a revisão que vale hoje.
    #[arg(long)]
    pub para: Option<u64>,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Opções de `formatar`.
#[derive(Debug, Args)]
pub struct FormatarArgs {
    /// Arquivo do script a formatar.
    pub arquivo: PathBuf,

    /// Grava o resultado no próprio arquivo, em vez de mostrar.
    #[arg(long)]
    pub escrever: bool,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Opções de `liberar` e `retirar`.
#[derive(Debug, Args)]
pub struct PermissaoArgs {
    /// Agente: id (UUID v7) ou slug.
    pub agente: String,

    /// Fluxo: id (UUID v7) ou slug.
    #[arg(long)]
    pub fluxo: String,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Banco, cache de revisões e configuração de uma execução do comando.
struct Sessao {
    store: Store,
    cache: CacheRevisoes,
    config: HarnessConfig,
}

impl Sessao {
    fn abrir(flag: Option<PathBuf>, ambiente: &config::Ambiente) -> Result<Self, String> {
        let (store, config) = abrir_com_configuracao(flag, ambiente)?;
        let cache = CacheRevisoes::novo(
            usize::try_from(config.flows.revision_cache_mb).unwrap_or(usize::MAX),
        );
        Ok(Self {
            store,
            cache,
            config,
        })
    }

    fn operacoes(&self) -> Operacoes<'_> {
        Operacoes::novo(&self.store, &self.cache, &self.config)
    }

    fn org(&self, texto: Option<&str>) -> Result<OrgId, String> {
        let texto =
            texto.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
        resolver_org(&self.store, texto)
    }
}

/// Executa um subcomando de `poph flow`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
    acao: AcaoFluxo,
) -> ExitCode {
    let resultado = match acao {
        AcaoFluxo::Validar(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| validar(&sessao, org, args))
        }
        AcaoFluxo::Formatar(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| formatar(&sessao, args))
        }
        AcaoFluxo::Publicar(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| publicar(&sessao, org, args))
        }
        AcaoFluxo::Listar(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| listar(&sessao, org, args))
        }
        AcaoFluxo::Mostrar(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| mostrar(&sessao, org, args))
        }
        AcaoFluxo::Diff(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| diff(&sessao, org, args))
        }
        AcaoFluxo::Liberar(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| permissao(&sessao, org, args, true))
        }
        AcaoFluxo::Retirar(args) => {
            Sessao::abrir(flag, ambiente).and_then(|sessao| permissao(&sessao, org, args, false))
        }
    };
    match resultado {
        Ok(codigo) => codigo,
        Err(mensagem) => falha(mensagem),
    }
}

fn validar(sessao: &Sessao, org: Option<&str>, args: ArquivoArgs) -> Result<ExitCode, String> {
    let fonte = ler_script(&args.arquivo)?;
    let org = match org {
        Some(texto) => Some(resolver_org(&sessao.store, texto)?),
        None => None,
    };
    let validacao = sessao
        .operacoes()
        .validar(org, &fonte)
        .map_err(|erro| descrever(&erro))?;
    if args.json {
        imprimir_json(&com_versao(json_validacao(&validacao)));
    } else {
        if validacao.valido() {
            println!("{}: script válido.", args.arquivo.display());
        } else {
            println!("{}: script inválido.", args.arquivo.display());
        }
        imprimir_diagnosticos(&validacao.diagnosticos);
        for resolucao in &validacao.resolucoes {
            match &resolucao.departamento_id {
                Some(id) => println!("  departamento \"{}\": encontrado ({id})", resolucao.nome),
                None => println!(
                    "  departamento \"{}\": não encontrado nesta org",
                    resolucao.nome
                ),
            }
        }
    }
    Ok(codigo(validacao.valido()))
}

fn formatar(sessao: &Sessao, args: FormatarArgs) -> Result<ExitCode, String> {
    let fonte = ler_script(&args.arquivo)?;
    let texto = match sessao.operacoes().formatar(&fonte) {
        Ok(texto) => texto,
        Err(FalhaFluxo::ScriptInvalido(diagnosticos)) => {
            imprimir_diagnosticos(&diagnosticos);
            let erros = diagnosticos.iter().filter(|d| d.e_erro()).count();
            return Err(format!(
                "o script não compila: {erros} erro(s) a corrigir antes de formatar"
            ));
        }
        Err(outro) => return Err(outro.to_string()),
    };
    if args.escrever {
        fs::write(&args.arquivo, &texto)
            .map_err(|erro| format!("não consegui gravar `{}`: {erro}", args.arquivo.display()))?;
    }
    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": VERSAO_RELATORIO,
            "source": texto,
            "written": args.escrever,
        }));
    } else if args.escrever {
        println!(
            "{}: formatado e gravado no próprio arquivo.",
            args.arquivo.display()
        );
    } else {
        print!("{texto}");
    }
    Ok(ExitCode::SUCCESS)
}

fn publicar(sessao: &Sessao, org: Option<&str>, args: PublicarArgs) -> Result<ExitCode, String> {
    let org = sessao.org(org)?;
    let fonte = ler_script(&args.arquivo)?;
    let ops = sessao.operacoes();
    // Compila antes para saber o slug: é o `id` do script. Um script com erro não grava nada.
    let compilado = ops.compilar(&fonte);
    if compilado.tem_erro() {
        imprimir_diagnosticos(&compilado.diagnosticos);
        let erros = compilado.diagnosticos.iter().filter(|d| d.e_erro()).count();
        return Err(format!(
            "o script não passa na validação: {erros} erro(s) a corrigir; nada foi gravado"
        ));
    }
    let Some(ir) = compilado.ir.as_ref() else {
        return Err("erro interno: o compilador não devolveu a estrutura do script".to_owned());
    };
    let ator = Ator::cli();
    let existente = sessao
        .store
        .fluxos()
        .por_slug(org, &ir.fluxo.id)
        .map_err(|erro| descrever(&erro))?;
    match existente {
        None => {
            let criacao = ops
                .criar(org, &fonte, true, &ator)
                .map_err(|erro| erro.to_string())?;
            let feita = criacao
                .publicacao
                .as_ref()
                .ok_or_else(|| "erro interno: a criação não publicou a revisão 1".to_owned())?;
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": VERSAO_RELATORIO,
                    "created": true,
                    "flow": json_fluxo_com_rascunho(&criacao.fluxo),
                    "revision": json_revisao_resumo(&feita.revisao),
                    "warnings": feita.avisos,
                }));
            } else {
                println!(
                    "fluxo `{}` criado e publicado como revisão {}.",
                    criacao.fluxo.slug, feita.revisao.numero
                );
                imprimir_diagnosticos(&feita.avisos);
            }
        }
        Some(fluxo) => {
            let esperada = args.if_match.ok_or_else(|| {
                format!(
                    "o fluxo `{}` já existe: informe --if-match com a versão atual (veja com \
                     `poph --org ... flow mostrar {}`)",
                    fluxo.slug, fluxo.slug
                )
            })?;
            let feita = ops
                .publicar(org, &fluxo, esperada, &fonte, &ator)
                .map_err(|erro| erro.to_string())?;
            // O rascunho e a auditoria da publicação já são gravados por `publicar_revisao`, na mesma
            // escrita que cria a revisão: aqui não há segunda gravação nem auditoria duplicada.
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": VERSAO_RELATORIO,
                    "created": false,
                    "flow_id": fluxo.id.to_string(),
                    "revision": json_revisao_resumo(&feita.revisao),
                    "version": feita.versao_do_fluxo,
                    "warnings": feita.avisos,
                }));
            } else {
                println!(
                    "fluxo `{}` publicado como revisão {}.",
                    fluxo.slug, feita.revisao.numero
                );
                imprimir_diagnosticos(&feita.avisos);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn listar(sessao: &Sessao, org: Option<&str>, args: ListarArgs) -> Result<ExitCode, String> {
    let org = sessao.org(org)?;
    let pagina = pagina_de(args.limit, args.cursor.as_deref())?;
    let filtro = FiltroFluxos {
        status: args.status.as_deref().map(status_de_texto).transpose()?,
    };
    let fluxos = sessao
        .store
        .fluxos()
        .listar(org, &filtro, pagina)
        .map_err(|erro| descrever(&erro))?;
    let next_cursor = if fluxos.len() < pagina.limite as usize {
        None
    } else {
        fluxos.last().map(|fluxo| fluxo.id.to_string())
    };
    let itens: Vec<Value> = fluxos.iter().map(json_fluxo).collect();
    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": VERSAO_RELATORIO,
            "org_id": org.to_string(),
            "items": itens,
            "next_cursor": next_cursor,
        }));
    } else if itens.is_empty() {
        println!("Nenhum fluxo nesta org.");
    } else {
        for item in &itens {
            println!(
                "{}  {:<9} {:<28} {}  (revisão {}, versão {})",
                texto_de(&item["id"]),
                texto_de(&item["status"]),
                texto_de(&item["slug"]),
                texto_de(&item["name"]),
                texto_de(&item["current_revision"]),
                texto_de(&item["version"]),
            );
        }
        if let Some(cursor) = &next_cursor {
            println!("\nPróxima página: --cursor {cursor}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn mostrar(sessao: &Sessao, org: Option<&str>, args: ReferenciaArgs) -> Result<ExitCode, String> {
    let org = sessao.org(org)?;
    let fluxo = resolver_fluxo(&sessao.store, org, &args.fluxo)?;
    let revisao = sessao
        .store
        .revisoes()
        .atual(org, fluxo.id)
        .map_err(|erro| descrever(&erro))?;
    let mut corpo = json_fluxo_com_rascunho(&fluxo);
    corpo["revision"] = revisao.as_ref().map_or(Value::Null, json_revisao_resumo);
    if args.json {
        imprimir_json(&com_versao(corpo));
        return Ok(ExitCode::SUCCESS);
    }
    println!("fluxo `{}` — {}", fluxo.slug, fluxo.nome);
    println!("  id: {}", fluxo.id);
    println!("  situação: {}", texto_de(&corpo["status"]));
    println!("  versão: {}", fluxo.versao);
    match &revisao {
        Some(r) => println!(
            "  revisão que vale hoje: {} (publicada por {})",
            r.numero, r.publicada_por
        ),
        None => println!("  revisão que vale hoje: nenhuma (só rascunho)"),
    }
    println!("\nRascunho:\n{}", fluxo.rascunho_fonte);
    Ok(ExitCode::SUCCESS)
}

fn diff(sessao: &Sessao, org: Option<&str>, args: DiffArgs) -> Result<ExitCode, String> {
    let org = sessao.org(org)?;
    let fluxo = resolver_fluxo(&sessao.store, org, &args.fluxo)?;
    let comparacao = sessao
        .operacoes()
        .comparar(org, &fluxo, args.de, args.para)
        .map_err(|erro| erro.to_string())?;
    if args.json {
        imprimir_json(&com_versao(json_comparacao(fluxo.id, &comparacao)));
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "revisão {} → revisão {}: {} mudança(s).",
        comparacao.de.numero,
        comparacao.para.numero,
        comparacao.mudancas.len()
    );
    println!(
        "  atendimentos ativos: {} na revisão {}, {} na revisão {}.",
        comparacao.ativas_de, comparacao.de.numero, comparacao.ativas_para, comparacao.para.numero
    );
    for mudanca in &comparacao.mudancas {
        println!("  - {}", mudanca.texto);
    }
    Ok(ExitCode::SUCCESS)
}

fn permissao(
    sessao: &Sessao,
    org: Option<&str>,
    args: PermissaoArgs,
    liberar: bool,
) -> Result<ExitCode, String> {
    let org = sessao.org(org)?;
    let fluxo = resolver_fluxo(&sessao.store, org, &args.fluxo)?;
    let agente = resolver_agente(&sessao.store, org, &args.agente)?;
    let resultado = if liberar {
        sessao.store.fluxos().liberar(org, agente.id, fluxo.id)
    } else {
        sessao.store.fluxos().revogar(org, agente.id, fluxo.id)
    };
    resultado.map_err(|erro| descrever(&erro))?;
    auditar(
        &sessao.store,
        org,
        &Ator::cli(),
        "agent_flows_update",
        "agent",
        agente.id.to_string(),
    )
    .map_err(|erro| descrever(&erro))?;
    let acao = if liberar { "liberado" } else { "retirado" };
    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": VERSAO_RELATORIO,
            "org_id": org.to_string(),
            "agent_id": agente.id.to_string(),
            "flow_id": fluxo.id.to_string(),
            "action": acao,
        }));
    } else if liberar {
        println!(
            "fluxo `{}` liberado ao agente `{}`.",
            fluxo.slug, agente.nome
        );
    } else {
        println!(
            "fluxo `{}` retirado do agente `{}`.",
            fluxo.slug, agente.nome
        );
    }
    Ok(ExitCode::SUCCESS)
}

// ───────────────────────────── apoio local ─────────────────────────────

fn ler_script(arquivo: &Path) -> Result<String, String> {
    fs::read_to_string(arquivo)
        .map_err(|erro| format!("não consegui ler `{}`: {erro}", arquivo.display()))
}

/// Código de saída: 0 quando não há erro, 1 quando há.
fn codigo(sem_erro: bool) -> ExitCode {
    if sem_erro {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Acrescenta `versao_relatorio` ao objeto de saída JSON.
fn com_versao(mut valor: Value) -> Value {
    valor["versao_relatorio"] = json!(VERSAO_RELATORIO);
    valor
}

/// Texto de um campo JSON, sem as aspas (para a saída de texto).
fn texto_de(valor: &Value) -> String {
    match valor {
        Value::String(texto) => texto.clone(),
        Value::Null => "-".to_owned(),
        outro => outro.to_string(),
    }
}

/// Uma linha por diagnóstico, com linha e coluna quando o compilador as conhece.
fn imprimir_diagnosticos(diagnosticos: &[Diagnostico]) {
    for diagnostico in diagnosticos {
        let lugar = match (diagnostico.linha(), diagnostico.coluna()) {
            (Some(linha), Some(coluna)) => format!("linha {linha}, coluna {coluna}: "),
            _ => String::new(),
        };
        println!(
            "  {lugar}{} {}: {}",
            diagnostico.severidade.como_texto(),
            diagnostico.codigo.como_texto(),
            diagnostico.mensagem
        );
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn codigo_de_saida_segue_o_erro() {
        assert_eq!(codigo(true), ExitCode::SUCCESS);
        assert_eq!(codigo(false), ExitCode::from(1));
    }

    #[test]
    fn texto_de_tira_as_aspas_e_marca_o_vazio() {
        assert_eq!(texto_de(&json!("ativo")), "ativo");
        assert_eq!(texto_de(&Value::Null), "-");
        assert_eq!(texto_de(&json!(3)), "3");
    }

    #[test]
    fn com_versao_acrescenta_o_formato() {
        let valor = com_versao(json!({ "valid": true }));
        assert_eq!(valor["versao_relatorio"], 1);
        assert_eq!(valor["valid"], true);
    }
}

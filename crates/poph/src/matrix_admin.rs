//! `poph matrix estado|ligar|desligar|espaco` — operação do módulo Matrix pela linha de comando (MX-23).
//!
//! Segue o padrão dos outros comandos administrativos da CLI (`org`, `department`, `token`): exige
//! `--offline` e `--org` e opera direto no banco. A CLI **não** fala com o servidor e não usa token:
//! a paridade com a API administrativa (MX-22) está nas regras, não no transporte:
//!
//! - o corte do backfill é gravado pelo mesmo `ligar` do repositório (desligar preserva o corte);
//! - a versão do estado é o `updated_at` de `matrix_orgs`, o mesmo valor do `ETag` da API. Ligar e
//!   desligar leem a versão antes e conferem antes de gravar; se ela mudou entre as duas leituras,
//!   a saída é `3` (o equivalente ao `412` da API) e nada é gravado;
//! - criar e ligar espaço usa o mesmo serviço de espaços (MX-12) que a API usa, com a mesma porta
//!   sobre o repositório. Isso fala com o Synapse configurado em `[matrix]`, então exige os dois
//!   tokens no ambiente (`POPH_MATRIX_AS_TOKEN` e `POPH_MATRIX_HS_TOKEN`);
//! - desligar espaço só apaga o registro no Harness: nada sai do Element.
//!
//! O que a CLI **não** sabe: se o Synapse está respondendo agora. Essa sonda roda no servidor, então
//! `conectado` aparece como `null` no `estado`. Nenhum token ou valor sensível aparece na saída.
//!
//! Saídas: `0` ok; `1` erro (org, departamento, configuração, Synapse, banco); `3` a versão mudou
//! entre a leitura e a gravação. O `--json` tem formato estável, com `versao_relatorio` = 1.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_matrix::spaces::SpaceOrigin as OrigemGravada;
use poph_matrix::{
    Classification, DepartmentSpace, MatrixClient, MatrixModule, SpaceError, Spaces,
};
use poph_server::matrix_inicio::PortaEspacosStore;
use poph_store::{NewAuditEntry, StateMatrixOrg, Store};
use poph_types::{DepartmentId, HarnessConfig, OrgId};
use serde_json::{Value, json};

use crate::apoio::{descrever, exigir_offline, imprimir_json, parse_id, resolver_org};
use crate::config;

/// Código de saída quando a versão do estado mudou entre a leitura e a gravação (o `412` da API).
pub const SAIDA_CONFLITO: u8 = 3;

/// Opções comuns: só a saída em JSON.
#[derive(Debug, Args)]
pub struct SaidaArgs {
    /// Saída em JSON (formato estável, `versao_relatorio` = 1).
    #[arg(long)]
    pub json: bool,
}

/// Opções de `poph matrix espaco`: sem `--departamento` lista; com ele cria ou liga.
#[derive(Debug, Args)]
pub struct EspacoArgs {
    /// Departamento do espaço (id UUID v7 da org). Sem ele, lista os espaços da org.
    #[arg(long = "departamento", value_name = "ID")]
    pub department: Option<String>,

    /// Espaço que já existe no Element: id `!…` ou alias `#…`. Sem ele, o módulo cria o espaço do
    /// departamento pelo alias próprio. Só vale junto com `--departamento`.
    #[arg(long = "espaco", value_name = "ID_OU_ALIAS")]
    pub space: Option<String>,

    /// Saída em JSON (formato estável, `versao_relatorio` = 1).
    #[arg(long)]
    pub json: bool,

    /// `desligar`: tira o espaço do departamento sem apagar nada no Element.
    #[command(subcommand)]
    pub acao: Option<AcaoEspaco>,
}

/// Subcomandos de `poph matrix espaco`.
#[derive(Debug, Subcommand)]
pub enum AcaoEspaco {
    /// Desliga o espaço do departamento: a linha sai do Harness; a sala e o espaço continuam no Element.
    Desligar(DesligarEspacoArgs),
}

/// Opções de `poph matrix espaco desligar`.
#[derive(Debug, Args)]
pub struct DesligarEspacoArgs {
    /// Departamento cujo espaço sai do módulo (id UUID v7 da org).
    #[arg(long = "departamento", value_name = "ID")]
    pub department: String,

    /// Saída em JSON (formato estável, `versao_relatorio` = 1).
    #[arg(long)]
    pub json: bool,
}

/// Resultado de uma operação: texto para o operador ou erro com a mensagem pronta.
type Resultado = Result<ExitCode, String>;

/// O que todo subcomando precisa: banco aberto, org resolvida e a configuração.
struct Contexto {
    store: Arc<Store>,
    org: OrgId,
    config: HarnessConfig,
}

/// Abre a configuração e o banco e resolve a org. Exige `--offline` e `--org`.
fn abrir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
) -> Result<Contexto, String> {
    exigir_offline(offline)?;
    let text = org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let config = config::carregar(arquivo.as_deref(), ambiente)
        .map_err(|error| format!("configuração inválida: {error}"))?;
    let store =
        Store::open(&config).map_err(|error| format!("não consegui abrir o banco: {error}"))?;
    let org = resolver_org(&store, text)?;
    Ok(Contexto {
        store: Arc::new(store),
        org,
        config,
    })
}

/// Executa `poph matrix estado|ligar|desligar|espaco`.
pub fn state(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    args: &SaidaArgs,
) -> ExitCode {
    let ctx = match abrir(flag, ambiente, offline, org) {
        Ok(ctx) => ctx,
        Err(message) => return error(&message),
    };
    responder(mostrar_estado(&ctx, args.json))
}

/// `poph matrix ligar` ou `desligar`.
pub fn ligar_ou_desligar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    ligar: bool,
    args: &SaidaArgs,
) -> ExitCode {
    let ctx = match abrir(flag, ambiente, offline, org) {
        Ok(ctx) => ctx,
        Err(message) => return error(&message),
    };
    responder(alterar_ligacao(&ctx, ligar, args.json))
}

/// `poph matrix espaco …`.
pub fn space(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    args: &EspacoArgs,
) -> ExitCode {
    let ctx = match abrir(flag, ambiente, offline, org) {
        Ok(ctx) => ctx,
        Err(message) => return error(&message),
    };
    let resultado = match (&args.acao, &args.department) {
        (Some(AcaoEspaco::Desligar(desligar)), None) if args.space.is_none() => {
            desligar_espaco(&ctx, &desligar.department, desligar.json)
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
        (None, None) => listar_espacos(&ctx, args.json),
    };
    responder(resultado)
}

fn responder(resultado: Resultado) -> ExitCode {
    match resultado {
        Ok(code) => code,
        Err(message) => error(&message),
    }
}

fn error(message: &str) -> ExitCode {
    eprintln!("erro: {message}");
    ExitCode::from(1)
}

// ───────────────────────────── estado ─────────────────────────────

/// `poph matrix estado`: ligado, corte do backfill, atraso, falhas e a versão do estado.
fn mostrar_estado(ctx: &Contexto, json: bool) -> Resultado {
    let linha = ler(ctx.store.matrix().state(ctx.org))?;
    let cursores = ler(ctx.store.matrix().read_cursors(ctx.org))?;
    let atraso = ler(ctx
        .store
        .matrix()
        .delay_of_messages(ctx.org, cursores.messages))?;
    let idade_s = atraso
        .oldest_at
        .map(|criada_em| ((ctx.store.now() - criada_em).max(0) / 1000) as u64)
        .unwrap_or(0);
    let falhas = ler(ctx.store.matrix().count_failures(ctx.org))?;
    let ultima_falha = ler(ctx.store.matrix().list_failures(ctx.org, 1))?
        .first()
        .map(|falha| falha.motivo.clone());
    let slug = ler(ctx.store.orgs().get(ctx.org))?.slug;

    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "org": slug,
            "ligado": linha.linked,
            "corte_do_backfill_ms": linha.linked_at,
            "versao": linha.updated_at,
            "conectado": Value::Null,
            "atraso": { "mensagens": atraso.pending, "idade_mais_antiga_s": idade_s },
            "falhas_pendentes": falhas,
            "ultima_falha": ultima_falha,
        }));
    } else {
        println!("módulo Matrix da org `{slug}`");
        println!("  ligado:                 {}", sim_nao(linha.linked));
        println!(
            "  corte do backfill:      {}",
            linha
                .linked_at
                .map(data_utc)
                .unwrap_or_else(|| "nunca ligado".to_owned())
        );
        println!("  conectado ao Synapse:   desconhecido pela CLI (a sonda roda no servidor)");
        println!(
            "  pendentes de espelhar:  {} mensagem(ns){}",
            atraso.pending,
            if atraso.pending > 0 {
                format!(", a mais antiga há {idade_s} s")
            } else {
                String::new()
            }
        );
        println!("  falhas isoladas:        {falhas}");
        println!(
            "  último motivo de falha: {}",
            ultima_falha.as_deref().unwrap_or("nenhum")
        );
    }
    Ok(ExitCode::SUCCESS)
}

// ───────────────────────────── ligar e desligar ─────────────────────────────

/// Por que uma alteração de ligação não foi gravada.
#[derive(Debug)]
enum ErroLigacao {
    /// A versão lida antes mudou até a gravação: quem gravou foi outro processo.
    VersaoMudou,
    /// Falha do banco ou da configuração, com a mensagem pronta.
    Other(String),
}

/// `poph matrix ligar|desligar`: lê a versão, confere e grava.
fn alterar_ligacao(ctx: &Contexto, ligar: bool, json: bool) -> Resultado {
    let antes = ler(ctx.store.matrix().state(ctx.org))?;
    let esperada = antes.updated_at;
    match aplicar_ligacao(ctx, ligar, esperada) {
        Ok(depois) => {
            let slug = ler(ctx.store.orgs().get(ctx.org))?.slug;
            if json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org": slug,
                    "ligado": depois.linked,
                    "corte_do_backfill_ms": depois.linked_at,
                    "versao": depois.updated_at,
                }));
            } else if ligar {
                println!("módulo Matrix ligado na org `{slug}`.");
                println!(
                    "  corte do backfill: {} (só o que chegar daqui em diante é espelhado)",
                    depois.linked_at.map(data_utc).unwrap_or_default()
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
        Err(ErroLigacao::VersaoMudou) => {
            eprintln!(
                "erro: o estado do módulo mudou entre a leitura e a gravação (outro processo gravou ao mesmo tempo). Nada foi alterado: rode `poph matrix estado` e tente de novo"
            );
            Ok(ExitCode::from(SAIDA_CONFLITO))
        }
        Err(ErroLigacao::Other(message)) => Err(message),
    }
}

/// Grava ligar/desligar só se a versão ainda é a `esperada`. Registra a auditoria depois da gravação.
fn aplicar_ligacao(
    ctx: &Contexto,
    ligar: bool,
    esperada: i64,
) -> Result<StateMatrixOrg, ErroLigacao> {
    let atual = ctx
        .store
        .matrix()
        .state(ctx.org)
        .map_err(|error| ErroLigacao::Other(descrever(&error)))?;
    if atual.updated_at != esperada {
        return Err(ErroLigacao::VersaoMudou);
    }
    let depois = if ligar {
        ctx.store.matrix().enable(ctx.org)
    } else {
        ctx.store.matrix().disable(ctx.org)
    }
    .map_err(|error| ErroLigacao::Other(descrever(&error)))?;
    auditar(ctx, "update", "matrix", ctx.org.to_string()).map_err(ErroLigacao::Other)?;
    Ok(depois)
}

// ───────────────────────────── espaços ─────────────────────────────

/// `poph matrix espaco` (sem `--departamento`): lista os espaços da org.
fn listar_espacos(ctx: &Contexto, json: bool) -> Resultado {
    let slug = ler(ctx.store.orgs().get(ctx.org))?.slug;
    let spaces = ler(ctx.store.matrix().list_spaces(ctx.org))?;
    let mut itens = Vec::new();
    for space in &spaces {
        let name = ctx
            .store
            .departments()
            .get(ctx.org, space.departamento)
            .ok()
            .map(|department| department.name);
        itens.push((space, name));
    }
    if json {
        let lista: Vec<Value> = itens
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
        imprimir_json(&json!({ "versao_relatorio": 1, "org": slug, "espacos": lista }));
    } else if itens.is_empty() {
        println!("nenhum espaço por departamento na org `{slug}`.");
    } else {
        println!("espaços por departamento da org `{slug}`: {}", itens.len());
        for (space, name) in &itens {
            println!(
                "  - {} → {} ({})",
                name.as_deref().unwrap_or("departamento sem nome"),
                space.room_id,
                descricao_origem(space.origin.as_text())
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `poph matrix espaco --departamento <id> [--espaco …]`: cria o espaço ou liga um que já existe.
fn ensure_space(
    ctx: &Contexto,
    departamento_texto: &str,
    existing: Option<&str>,
    json: bool,
) -> Resultado {
    let departamento_id: DepartmentId = parse_id(departamento_texto)?;
    let department = ler(ctx.store.departments().get(ctx.org, departamento_id))?;
    let existing = existing.map(str::trim).filter(|text| !text.is_empty());

    let modulo = match MatrixModule::prepare(ctx.config.matrix.clone()) {
        Ok(Some(modulo)) => modulo,
        Ok(None) => {
            return Err(
                "o módulo Matrix não está ligado nesta configuração: ponha `habilitado = true` na seção [matrix]"
                    .to_owned(),
            );
        }
        // A mensagem do erro nomeia a variável ou o campo; nunca traz o valor.
        Err(error) => return Err(error.to_string()),
    };
    let client = MatrixClient::from_parts(&modulo.config, &modulo.secrets).map_err(|_| {
        "não consegui montar a conexão com o Synapse: confira `homeserver_url` na seção [matrix]"
            .to_owned()
    })?;
    let spaces = Spaces::of_config(
        client,
        PortaEspacosStore(Arc::clone(&ctx.store)),
        &modulo.config,
    )
    .map_err(|_| "não consegui montar o serviço de espaços: confira a seção [matrix]".to_owned())?;

    let alvo = DepartmentSpace {
        id: department.id,
        name: department.name.clone(),
    };
    let gravado = executar_assincrono(spaces.ensure_space(ctx.org, &alvo, existing))?
        .map_err(mensagem_do_espaco)?;
    auditar(ctx, "update", "matrix_space", department.id.to_string())?;

    let origin = match gravado.origin {
        OrigemGravada::Automatic => "auto",
        OrigemGravada::Linked => "linked",
    };
    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "departamento": department.id.to_string(),
            "departamento_nome": department.name,
            "espaco": gravado.room_id,
            "alias": gravado.alias,
            "origem": origin,
        }));
    } else {
        println!(
            "espaço do departamento `{}`: {} ({})",
            department.name,
            gravado.room_id,
            descricao_origem(origin)
        );
        println!("  alias: {}", gravado.alias);
    }
    Ok(ExitCode::SUCCESS)
}

/// `poph matrix espaco desligar --departamento <id>`.
fn desligar_espaco(ctx: &Contexto, departamento_texto: &str, json: bool) -> Resultado {
    let departamento_id: DepartmentId = parse_id(departamento_texto)?;
    // Departamento de outra org (ou inexistente) não passa daqui: nada muda.
    let department = ler(ctx.store.departments().get(ctx.org, departamento_id))?;
    let desligado = ler(ctx.store.matrix().disable_space(ctx.org, department.id))?;
    auditar(ctx, "delete", "matrix_space", department.id.to_string())?;
    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "departamento": department.id.to_string(),
            "removido": desligado.removed,
            "salas_esquecidas": desligado.rooms_forgotten,
        }));
    } else if desligado.removed {
        println!(
            "espaço do departamento `{}` desligado: {} sala(s) deixaram de estar registradas nele.",
            department.name, desligado.rooms_forgotten
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

/// Tradução do erro do serviço de espaços para o operador. Mesmo critério da API (MX-22): falta de
/// permissão vira instrução; falha de rede vira "tente de novo"; nada do corpo do Synapse sai.
fn mensagem_do_espaco(error: SpaceError) -> String {
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
        SpaceError::Call(chamada) => match chamada.classification() {
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

/// Roda uma future do serviço de espaços até o fim, num runtime de um thread só (a CLI é síncrona).
fn executar_assincrono<F: std::future::Future>(futuro: F) -> Result<F::Output, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("não consegui iniciar o runtime: {error}"))?;
    Ok(runtime.block_on(futuro))
}

fn descricao_origem(origin: &str) -> &'static str {
    match origin {
        "auto" | "automatico" => "criado pelo módulo",
        _ => "ligado a um espaço que já existia no Element",
    }
}

// ───────────────────────────── apoio ─────────────────────────────

/// Converte o erro do banco na frase do operador.
fn ler<T>(resultado: Result<T, poph_store::StoreError>) -> Result<T, String> {
    resultado.map_err(|error| descrever(&error))
}

/// Auditoria da alteração, com ator `cli` (a API grava `admin_token`).
fn auditar(ctx: &Contexto, acao: &str, entidade: &str, id: String) -> Result<(), String> {
    ler(ctx.store.audit().register(
        Some(ctx.org),
        NewAuditEntry {
            ator_tipo: "cli".into(),
            actor_id: None,
            acao: acao.into(),
            entity: entidade.into(),
            entity_id: Some(id),
            detail_json: "{}".into(),
        },
    ))?;
    Ok(())
}

fn sim_nao(valor: bool) -> &'static str {
    if valor { "sim" } else { "não" }
}

/// Milissegundos desde 1970 (UTC) em `AAAA-MM-DD HH:MM UTC`, sem depender de crate de data.
pub fn data_utc(ms: i64) -> String {
    let segundos = ms.div_euclid(1000);
    let dias = segundos.div_euclid(86_400);
    let resto = segundos.rem_euclid(86_400);
    // Conversão de dias desde 1970-01-01 para ano, mês e dia (algoritmo civil, de Howard Hinnant).
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let ano = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dia = doy - (153 * mp + 2) / 5 + 1;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 };
    let ano = if mes <= 2 { ano + 1 } else { ano };
    format!(
        "{ano:04}-{mes:02}-{dia:02} {:02}:{:02} UTC",
        resto / 3600,
        (resto % 3600) / 60
    )
}

#[cfg(test)]
mod testes {
    use super::*;
    use poph_store::NewOrg;
    use std::fs;

    #[test]
    fn data_utc_converte_zero_e_um_instante_conhecido() {
        assert_eq!(data_utc(0), "1970-01-01 00:00 UTC");
        // 1795289241 é sáb 21/11/2026 16:27 em UTC-03:00, ou 19:27 UTC.
        assert_eq!(data_utc(1_795_289_241_000), "2026-11-21 19:27 UTC");
        // 29/02/2024 existe (ano bissexto).
        assert_eq!(data_utc(1_709_164_800_000), "2024-02-29 00:00 UTC");
    }

    /// Prova a checagem de versão (`412`) sem HTTP: uma versão velha não grava nada.
    /// Quebra se `aplicar_ligacao` deixar de conferir `atualizado_em` antes de gravar.
    #[test]
    fn ligar_recusa_quando_a_versao_mudou_e_nao_grava() {
        let pasta =
            std::env::temp_dir().join(format!("poph-matrix-admin-412-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta de teste");
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco de teste");
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja".into(),
                slug: "loja-412".into(),
                config_json: "{}".into(),
            })
            .expect("cria a org");
        let ctx = Contexto {
            store: Arc::new(store),
            org: org.id,
            config: HarnessConfig::default(),
        };
        let atual = ctx.store.matrix().state(org.id).expect("estado").updated_at;

        let recusado = aplicar_ligacao(&ctx, true, atual + 1);
        assert!(
            matches!(recusado, Err(ErroLigacao::VersaoMudou)),
            "com versão velha tem que recusar: {recusado:?}"
        );
        assert!(
            !ctx.store.matrix().state(org.id).expect("estado").linked,
            "recusado não pode ter ligado o módulo"
        );

        let gravado = aplicar_ligacao(&ctx, true, atual).expect("com a versão certa grava");
        assert!(gravado.linked);
        let _ = fs::remove_dir_all(&pasta);
    }
}

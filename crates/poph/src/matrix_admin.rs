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
use poph_matrix::espacos::OrigemEspaco as OrigemGravada;
use poph_matrix::{
    Classificacao, ClienteMatrix, DepartamentoEspaco, ErroEspaco, Espacos, ModuloMatrix,
};
use poph_server::matrix_inicio::PortaEspacosStore;
use poph_store::{EstadoMatrixOrg, NovoRegistroAuditoria, Store};
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
    #[arg(long, value_name = "ID")]
    pub departamento: Option<String>,

    /// Espaço que já existe no Element: id `!…` ou alias `#…`. Sem ele, o módulo cria o espaço do
    /// departamento pelo alias próprio. Só vale junto com `--departamento`.
    #[arg(long, value_name = "ID_OU_ALIAS")]
    pub espaco: Option<String>,

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
    #[arg(long, value_name = "ID")]
    pub departamento: String,

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
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let config = config::carregar(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    let store =
        Store::abrir(&config).map_err(|erro| format!("não consegui abrir o banco: {erro}"))?;
    let org = resolver_org(&store, texto)?;
    Ok(Contexto {
        store: Arc::new(store),
        org,
        config,
    })
}

/// Executa `poph matrix estado|ligar|desligar|espaco`.
pub fn estado(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    args: &SaidaArgs,
) -> ExitCode {
    let ctx = match abrir(flag, ambiente, offline, org) {
        Ok(ctx) => ctx,
        Err(mensagem) => return erro(&mensagem),
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
        Err(mensagem) => return erro(&mensagem),
    };
    responder(alterar_ligacao(&ctx, ligar, args.json))
}

/// `poph matrix espaco …`.
pub fn espaco(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    args: &EspacoArgs,
) -> ExitCode {
    let ctx = match abrir(flag, ambiente, offline, org) {
        Ok(ctx) => ctx,
        Err(mensagem) => return erro(&mensagem),
    };
    let resultado = match (&args.acao, &args.departamento) {
        (Some(AcaoEspaco::Desligar(desligar)), None) if args.espaco.is_none() => {
            desligar_espaco(&ctx, &desligar.departamento, desligar.json)
        }
        (Some(_), _) => Err(
            "use `espaco desligar --departamento <id>` sozinho: não combine com --departamento ou --espaco"
                .to_owned(),
        ),
        (None, Some(departamento)) => {
            garantir_espaco(&ctx, departamento, args.espaco.as_deref(), args.json)
        }
        (None, None) if args.espaco.is_some() => {
            Err("--espaco só vale junto com --departamento (o espaço é de um departamento)".to_owned())
        }
        (None, None) => listar_espacos(&ctx, args.json),
    };
    responder(resultado)
}

fn responder(resultado: Resultado) -> ExitCode {
    match resultado {
        Ok(codigo) => codigo,
        Err(mensagem) => erro(&mensagem),
    }
}

fn erro(mensagem: &str) -> ExitCode {
    eprintln!("erro: {mensagem}");
    ExitCode::from(1)
}

// ───────────────────────────── estado ─────────────────────────────

/// `poph matrix estado`: ligado, corte do backfill, atraso, falhas e a versão do estado.
fn mostrar_estado(ctx: &Contexto, json: bool) -> Resultado {
    let linha = ler(ctx.store.matrix().estado(ctx.org))?;
    let cursores = ler(ctx.store.matrix().ler_cursores(ctx.org))?;
    let atraso = ler(ctx
        .store
        .matrix()
        .atraso_das_mensagens(ctx.org, cursores.mensagens))?;
    let idade_s = atraso
        .mais_antiga_em
        .map(|criada_em| ((ctx.store.agora() - criada_em).max(0) / 1000) as u64)
        .unwrap_or(0);
    let falhas = ler(ctx.store.matrix().contar_falhas(ctx.org))?;
    let ultima_falha = ler(ctx.store.matrix().listar_falhas(ctx.org, 1))?
        .first()
        .map(|falha| falha.motivo.clone());
    let slug = ler(ctx.store.orgaos().obter(ctx.org))?.slug;

    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "org": slug,
            "ligado": linha.ligado,
            "corte_do_backfill_ms": linha.ligado_em,
            "versao": linha.atualizado_em,
            "conectado": Value::Null,
            "atraso": { "mensagens": atraso.pendentes, "idade_mais_antiga_s": idade_s },
            "falhas_pendentes": falhas,
            "ultima_falha": ultima_falha,
        }));
    } else {
        println!("módulo Matrix da org `{slug}`");
        println!("  ligado:                 {}", sim_nao(linha.ligado));
        println!(
            "  corte do backfill:      {}",
            linha
                .ligado_em
                .map(data_utc)
                .unwrap_or_else(|| "nunca ligado".to_owned())
        );
        println!("  conectado ao Synapse:   desconhecido pela CLI (a sonda roda no servidor)");
        println!(
            "  pendentes de espelhar:  {} mensagem(ns){}",
            atraso.pendentes,
            if atraso.pendentes > 0 {
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
    Outro(String),
}

/// `poph matrix ligar|desligar`: lê a versão, confere e grava.
fn alterar_ligacao(ctx: &Contexto, ligar: bool, json: bool) -> Resultado {
    let antes = ler(ctx.store.matrix().estado(ctx.org))?;
    let esperada = antes.atualizado_em;
    match aplicar_ligacao(ctx, ligar, esperada) {
        Ok(depois) => {
            let slug = ler(ctx.store.orgaos().obter(ctx.org))?.slug;
            if json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org": slug,
                    "ligado": depois.ligado,
                    "corte_do_backfill_ms": depois.ligado_em,
                    "versao": depois.atualizado_em,
                }));
            } else if ligar {
                println!("módulo Matrix ligado na org `{slug}`.");
                println!(
                    "  corte do backfill: {} (só o que chegar daqui em diante é espelhado)",
                    depois.ligado_em.map(data_utc).unwrap_or_default()
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
        Err(ErroLigacao::Outro(mensagem)) => Err(mensagem),
    }
}

/// Grava ligar/desligar só se a versão ainda é a `esperada`. Registra a auditoria depois da gravação.
fn aplicar_ligacao(
    ctx: &Contexto,
    ligar: bool,
    esperada: i64,
) -> Result<EstadoMatrixOrg, ErroLigacao> {
    let atual = ctx
        .store
        .matrix()
        .estado(ctx.org)
        .map_err(|erro| ErroLigacao::Outro(descrever(&erro)))?;
    if atual.atualizado_em != esperada {
        return Err(ErroLigacao::VersaoMudou);
    }
    let depois = if ligar {
        ctx.store.matrix().ligar(ctx.org)
    } else {
        ctx.store.matrix().desligar(ctx.org)
    }
    .map_err(|erro| ErroLigacao::Outro(descrever(&erro)))?;
    auditar(ctx, "update", "matrix", ctx.org.to_string()).map_err(ErroLigacao::Outro)?;
    Ok(depois)
}

// ───────────────────────────── espaços ─────────────────────────────

/// `poph matrix espaco` (sem `--departamento`): lista os espaços da org.
fn listar_espacos(ctx: &Contexto, json: bool) -> Resultado {
    let slug = ler(ctx.store.orgaos().obter(ctx.org))?.slug;
    let espacos = ler(ctx.store.matrix().listar_espacos(ctx.org))?;
    let mut itens = Vec::new();
    for espaco in &espacos {
        let nome = ctx
            .store
            .departamentos()
            .obter(ctx.org, espaco.departamento)
            .ok()
            .map(|departamento| departamento.nome);
        itens.push((espaco, nome));
    }
    if json {
        let lista: Vec<Value> = itens
            .iter()
            .map(|(espaco, nome)| {
                json!({
                    "departamento": espaco.departamento.to_string(),
                    "departamento_nome": nome,
                    "espaco": espaco.room_id,
                    "alias": espaco.alias,
                    "origem": espaco.origem.como_texto(),
                })
            })
            .collect();
        imprimir_json(&json!({ "versao_relatorio": 1, "org": slug, "espacos": lista }));
    } else if itens.is_empty() {
        println!("nenhum espaço por departamento na org `{slug}`.");
    } else {
        println!("espaços por departamento da org `{slug}`: {}", itens.len());
        for (espaco, nome) in &itens {
            println!(
                "  - {} → {} ({})",
                nome.as_deref().unwrap_or("departamento sem nome"),
                espaco.room_id,
                descricao_origem(espaco.origem.como_texto())
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `poph matrix espaco --departamento <id> [--espaco …]`: cria o espaço ou liga um que já existe.
fn garantir_espaco(
    ctx: &Contexto,
    departamento_texto: &str,
    existente: Option<&str>,
    json: bool,
) -> Resultado {
    let departamento_id: DepartmentId = parse_id(departamento_texto)?;
    let departamento = ler(ctx.store.departamentos().obter(ctx.org, departamento_id))?;
    let existente = existente.map(str::trim).filter(|texto| !texto.is_empty());

    let modulo = match ModuloMatrix::preparar(ctx.config.matrix.clone()) {
        Ok(Some(modulo)) => modulo,
        Ok(None) => {
            return Err(
                "o módulo Matrix não está ligado nesta configuração: ponha `habilitado = true` na seção [matrix]"
                    .to_owned(),
            );
        }
        // A mensagem do erro nomeia a variável ou o campo; nunca traz o valor.
        Err(erro) => return Err(erro.to_string()),
    };
    let cliente = ClienteMatrix::das_partes(&modulo.config, &modulo.segredos).map_err(|_| {
        "não consegui montar a conexão com o Synapse: confira `homeserver_url` na seção [matrix]"
            .to_owned()
    })?;
    let espacos = Espacos::da_config(
        cliente,
        PortaEspacosStore(Arc::clone(&ctx.store)),
        &modulo.config,
    )
    .map_err(|_| "não consegui montar o serviço de espaços: confira a seção [matrix]".to_owned())?;

    let alvo = DepartamentoEspaco {
        id: departamento.id,
        nome: departamento.nome.clone(),
    };
    let gravado = executar_assincrono(espacos.garantir_espaco(ctx.org, &alvo, existente))?
        .map_err(mensagem_do_espaco)?;
    auditar(ctx, "update", "matrix_space", departamento.id.to_string())?;

    let origem = match gravado.origem {
        OrigemGravada::Automatico => "auto",
        OrigemGravada::Ligado => "linked",
    };
    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "departamento": departamento.id.to_string(),
            "departamento_nome": departamento.nome,
            "espaco": gravado.room_id,
            "alias": gravado.alias,
            "origem": origem,
        }));
    } else {
        println!(
            "espaço do departamento `{}`: {} ({})",
            departamento.nome,
            gravado.room_id,
            descricao_origem(origem)
        );
        println!("  alias: {}", gravado.alias);
    }
    Ok(ExitCode::SUCCESS)
}

/// `poph matrix espaco desligar --departamento <id>`.
fn desligar_espaco(ctx: &Contexto, departamento_texto: &str, json: bool) -> Resultado {
    let departamento_id: DepartmentId = parse_id(departamento_texto)?;
    // Departamento de outra org (ou inexistente) não passa daqui: nada muda.
    let departamento = ler(ctx.store.departamentos().obter(ctx.org, departamento_id))?;
    let desligado = ler(ctx.store.matrix().desligar_espaco(ctx.org, departamento.id))?;
    auditar(ctx, "delete", "matrix_space", departamento.id.to_string())?;
    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "departamento": departamento.id.to_string(),
            "removido": desligado.removido,
            "salas_esquecidas": desligado.salas_esquecidas,
        }));
    } else if desligado.removido {
        println!(
            "espaço do departamento `{}` desligado: {} sala(s) deixaram de estar registradas nele.",
            departamento.nome, desligado.salas_esquecidas
        );
        println!(
            "  nada foi apagado no Element. Na próxima alteração de conversa desse departamento, a sala volta a ser posicionada no mesmo espaço."
        );
    } else {
        println!(
            "o departamento `{}` não tinha espaço ligado: nada a fazer.",
            departamento.nome
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Tradução do erro do serviço de espaços para o operador. Mesmo critério da API (MX-22): falta de
/// permissão vira instrução; falha de rede vira "tente de novo"; nada do corpo do Synapse sai.
fn mensagem_do_espaco(erro: ErroEspaco) -> String {
    match erro {
        ErroEspaco::SemPermissao { o_que_falta } => o_que_falta,
        ErroEspaco::EspacoInvalido { motivo } => format!("espaço informado inválido: {motivo}"),
        ErroEspaco::Interno { .. } => {
            "falha interna do módulo Matrix ao garantir o espaço (defeito do módulo, não do ambiente)"
                .to_owned()
        }
        ErroEspaco::Armazenamento(_) => {
            "o armazenamento do módulo Matrix não respondeu: tente de novo".to_owned()
        }
        ErroEspaco::Chamada(chamada) => match chamada.classificacao() {
            Classificacao::Transitoria => {
                "o Synapse não respondeu: tente de novo em instantes".to_owned()
            }
            Classificacao::Definitiva => {
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
        .map_err(|erro| format!("não consegui iniciar o runtime: {erro}"))?;
    Ok(runtime.block_on(futuro))
}

fn descricao_origem(origem: &str) -> &'static str {
    match origem {
        "auto" | "automatico" => "criado pelo módulo",
        _ => "ligado a um espaço que já existia no Element",
    }
}

// ───────────────────────────── apoio ─────────────────────────────

/// Converte o erro do banco na frase do operador.
fn ler<T>(resultado: Result<T, poph_store::ErroStore>) -> Result<T, String> {
    resultado.map_err(|erro| descrever(&erro))
}

/// Auditoria da alteração, com ator `cli` (a API grava `admin_token`).
fn auditar(ctx: &Contexto, acao: &str, entidade: &str, id: String) -> Result<(), String> {
    ler(ctx.store.auditoria().registrar(
        Some(ctx.org),
        NovoRegistroAuditoria {
            ator_tipo: "cli".into(),
            ator_id: None,
            acao: acao.into(),
            entidade: entidade.into(),
            entidade_id: Some(id),
            detalhe_json: "{}".into(),
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
    use poph_store::NovaOrg;
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
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco de teste");
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: "Loja".into(),
                slug: "loja-412".into(),
                config_json: "{}".into(),
            })
            .expect("cria a org");
        let ctx = Contexto {
            store: Arc::new(store),
            org: org.id,
            config: HarnessConfig::default(),
        };
        let atual = ctx
            .store
            .matrix()
            .estado(org.id)
            .expect("estado")
            .atualizado_em;

        let recusado = aplicar_ligacao(&ctx, true, atual + 1);
        assert!(
            matches!(recusado, Err(ErroLigacao::VersaoMudou)),
            "com versão velha tem que recusar: {recusado:?}"
        );
        assert!(
            !ctx.store.matrix().estado(org.id).expect("estado").ligado,
            "recusado não pode ter ligado o módulo"
        );

        let gravado = aplicar_ligacao(&ctx, true, atual).expect("com a versão certa grava");
        assert!(gravado.ligado);
        let _ = fs::remove_dir_all(&pasta);
    }
}

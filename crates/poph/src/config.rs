//! Carregamento da configuração: padrão < arquivo TOML < variáveis de ambiente.
//!
//! - Arquivo: escolhido por `--config`, depois pela variável `POPH_CONFIG`, depois `poph.toml`
//!   na pasta atual (se existir). Sem arquivo, valem só os padrões.
//! - Ambiente: variáveis `POPH_...` para os campos não secretos (tabela [`VARIAVEIS_DE_CONFIG`]).
//! - Segredos: só do ambiente (ver [`segredos`]); nunca entram no arquivo nem na saída.
//!
//! Erros dizem a origem (arquivo ou variável) e a chave. Para isso a configuração é montada
//! como tabela TOML, e a chave culpada é achada testando cada valor isolado.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use poph_types::{DomainError, HarnessConfig, Segredo};
use serde::Deserialize;
use thiserror::Error;
use toml::{Table, Value};

/// Variáveis de ambiente, como mapa (permite testar sem mexer no ambiente do processo).
pub type Ambiente = BTreeMap<String, String>;

/// Variável que aponta o arquivo de configuração.
pub const VAR_CONFIG: &str = "POPH_CONFIG";
/// Nome do arquivo procurado na pasta atual quando nada foi indicado.
pub const ARQUIVO_PADRAO: &str = "poph.toml";

/// Variável que aponta o `cluster.toml` (HR-50a).
pub const VAR_CLUSTER: &str = "POPH_CLUSTER";
/// Nome do arquivo de cluster procurado ao lado do `poph.toml`.
pub const ARQUIVO_CLUSTER: &str = "cluster.toml";
/// Segredo compartilhado entre as instâncias (HMAC do encaminhamento, HR-50b).
pub const VAR_CLUSTER_SECRET: &str = "POPH_CLUSTER_SECRET";

/// Chave da cifra dos segredos guardados (AES-256-GCM, a partir da HR-07/HR-26).
pub const VAR_CIFRA: &str = "POPH_CHAVE_CIFRA";
/// Chave do provedor de LLM (`llm.chave_env` padrão). Mesmo nome usado pelo POP Atende.
pub const VAR_ANTIGRAVITY: &str = "ANTIGRAVITY_API_KEY";
/// Chave do Jev (TypeSafe). Mesmo nome usado pelo POP Atende.
pub const VAR_TYPESAFE: &str = "TYPESAFE_API_KEY";
/// Chave da Evolution API (WhatsApp). Mesmo nome usado pelo POP Atende.
pub const VAR_EVOLUTION: &str = "EVOLUTION_API_KEY";

/// Erro de configuração. Cada variante diz de onde veio o problema.
#[derive(Debug, Error)]
pub enum ErroConfig {
    /// Arquivo pedido explicitamente que não existe.
    #[error("arquivo de configuração não encontrado: {}", .0.display())]
    ArquivoAusente(PathBuf),
    /// Arquivo que existe mas não pode ser lido.
    #[error("não consegui ler o arquivo de configuração {}: {motivo}", .arquivo.display())]
    Leitura { arquivo: PathBuf, motivo: String },
    /// Arquivo com erro de sintaxe TOML (a mensagem traz linha e coluna).
    #[error("{}: erro de sintaxe no TOML: {motivo}", .arquivo.display())]
    Sintaxe { arquivo: PathBuf, motivo: String },
    /// Chave com valor ou nome errado, vinda do arquivo ou de uma variável.
    #[error("{origem}: chave `{chave}` inválida: {motivo}")]
    Chave {
        origem: String,
        chave: String,
        motivo: String,
    },
    /// Variável de ambiente com valor que não converte para o tipo esperado.
    #[error("{variavel}: valor inválido ({motivo})")]
    Ambiente {
        variavel: &'static str,
        motivo: String,
    },
    /// Valor convertido, mas fora da regra do domínio (ex.: porta 80).
    #[error("configuração inválida: {0}")]
    Validacao(#[from] DomainError),
    /// Falha interna de montagem (não deveria acontecer).
    #[error("erro interno ao montar a configuração: {0}")]
    Interno(String),
}

/// Segredos lidos do ambiente. Sem `Serialize`, e `Debug` oculta os valores.
#[derive(Debug, Clone, Default)]
pub struct Segredos {
    /// Chave de cifra (`POPH_CHAVE_CIFRA`).
    pub cifra: Option<Segredo>,
    /// Chave do Jev (`TYPESAFE_API_KEY`).
    pub typesafe: Option<Segredo>,
    /// Chave da Evolution (`EVOLUTION_API_KEY`).
    pub evolution: Option<Segredo>,
}

/// Tipo de valor aceito numa variável de configuração.
#[derive(Debug, Clone, Copy)]
enum Tipo {
    Texto,
    Inteiro,
    Booleano,
}

/// Variáveis de ambiente que sobrepõem o arquivo: nome, caminho na configuração e tipo.
const VARIAVEIS_DE_CONFIG: &[(&str, &[&str], Tipo)] = &[
    (
        "POPH_SERVIDOR_ENDERECO",
        &["servidor", "endereco"],
        Tipo::Texto,
    ),
    ("POPH_SERVIDOR_PORTA", &["servidor", "porta"], Tipo::Inteiro),
    ("POPH_DADOS_DIRETORIO", &["dados", "diretorio"], Tipo::Texto),
    ("POPH_LOG_NIVEL", &["log", "nivel"], Tipo::Texto),
    ("POPH_LOG_FORMATO", &["log", "formato"], Tipo::Texto),
    ("POPH_RETENCAO_DIAS", &["retencao", "dias"], Tipo::Inteiro),
    ("POPH_LLM_URL_BASE", &["llm", "url_base"], Tipo::Texto),
    ("POPH_LLM_MODELO", &["llm", "modelo"], Tipo::Texto),
    ("POPH_JEV_ATIVO", &["jev", "ativo"], Tipo::Booleano),
    ("POPH_JEV_URL_BASE", &["jev", "url_base"], Tipo::Texto),
    ("POPH_FLUXOS_ATIVO", &["fluxos", "ativo"], Tipo::Booleano),
    (
        "POPH_FLUXOS_MAX_FLUXOS_POR_ORG",
        &["fluxos", "max_fluxos_por_org"],
        Tipo::Inteiro,
    ),
    (
        "POPH_FLUXOS_MAX_FONTE_BYTES",
        &["fluxos", "max_fonte_bytes"],
        Tipo::Inteiro,
    ),
    ("POPH_FLUXOS_MAX_NOS", &["fluxos", "max_nos"], Tipo::Inteiro),
    (
        "POPH_FLUXOS_MAX_DADOS_BYTES",
        &["fluxos", "max_dados_bytes"],
        Tipo::Inteiro,
    ),
    (
        "POPH_FLUXOS_MAX_TICKS_REGRA",
        &["fluxos", "max_ticks_regra"],
        Tipo::Inteiro,
    ),
    (
        "POPH_FLUXOS_MAX_HEAP_REGRA",
        &["fluxos", "max_heap_regra"],
        Tipo::Inteiro,
    ),
    (
        "POPH_FLUXOS_MAX_SALTOS_DECISAO",
        &["fluxos", "max_saltos_decisao"],
        Tipo::Inteiro,
    ),
    (
        "POPH_FLUXOS_CACHE_REVISOES_MB",
        &["fluxos", "cache_revisoes_mb"],
        Tipo::Inteiro,
    ),
];

/// Uma origem de valor: arquivo ou variável, com o caminho da chave e o valor.
#[derive(Debug, Clone)]
struct Fonte {
    origem: String,
    caminho: Vec<String>,
    valor: Value,
}

/// Escolhe o arquivo de configuração: `--config`, depois `POPH_CONFIG`, depois `poph.toml` na pasta atual.
pub fn resolver_arquivo(
    flag: Option<&Path>,
    ambiente: &Ambiente,
    pasta_atual: &Path,
) -> Option<PathBuf> {
    if let Some(caminho) = flag {
        return Some(caminho.to_path_buf());
    }
    if let Some(caminho) = ambiente
        .get(VAR_CONFIG)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        return Some(PathBuf::from(caminho));
    }
    let padrao = pasta_atual.join(ARQUIVO_PADRAO);
    padrao.is_file().then_some(padrao)
}

/// Monta a configuração final: padrões, depois o arquivo (se houver), depois o ambiente.
pub fn carregar(arquivo: Option<&Path>, ambiente: &Ambiente) -> Result<HarnessConfig, ErroConfig> {
    let base = padrao_como_tabela()?;
    let mut fontes = Vec::new();
    if let Some(caminho) = arquivo {
        fontes.extend(fontes_do_arquivo(caminho)?);
    }
    fontes.extend(fontes_do_ambiente(ambiente)?);

    let mut mesclada = base.clone();
    for fonte in &fontes {
        definir(&mut mesclada, &fonte.caminho, fonte.valor.clone());
    }
    let config = match HarnessConfig::deserialize(Value::Table(mesclada)) {
        Ok(config) => config,
        Err(_) => return Err(achar_culpado(&base, &fontes)),
    };
    config.validar()?;
    Ok(config)
}

/// Caminho do `cluster.toml`: `POPH_CLUSTER` quando definido; senão `cluster.toml` no mesmo
/// diretório do `poph.toml` resolvido (ou na pasta atual, quando não há arquivo de configuração).
///
/// O caminho é devolvido mesmo quando o arquivo não existe: é o candidato que a recarga procura.
pub fn caminho_cluster(
    ambiente: &Ambiente,
    arquivo_de_configuracao: Option<&Path>,
    pasta_atual: &Path,
) -> PathBuf {
    if let Some(caminho) = ambiente
        .get(VAR_CLUSTER)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        return PathBuf::from(caminho);
    }
    let pasta = arquivo_de_configuracao
        .and_then(Path::parent)
        .filter(|pasta| !pasta.as_os_str().is_empty())
        .unwrap_or(pasta_atual);
    pasta.join(ARQUIVO_CLUSTER)
}

/// Segredo compartilhado do cluster (`POPH_CLUSTER_SECRET`). Vazio conta como ausente.
pub fn segredo_cluster(ambiente: &Ambiente) -> Option<Segredo> {
    lido(ambiente, VAR_CLUSTER_SECRET)
}

/// Lê os segredos do ambiente. Valor vazio ou só com espaços conta como ausente.
pub fn segredos(ambiente: &Ambiente) -> Segredos {
    Segredos {
        cifra: lido(ambiente, VAR_CIFRA),
        typesafe: lido(ambiente, VAR_TYPESAFE),
        evolution: lido(ambiente, VAR_EVOLUTION),
    }
}

pub fn chave_llm(ambiente: &Ambiente, chave_env: &str) -> Option<Segredo> {
    lido(ambiente, chave_env)
}

fn lido(ambiente: &Ambiente, variavel: &str) -> Option<Segredo> {
    ambiente
        .get(variavel)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| Segredo::new(v.to_owned()))
}

fn padrao_como_tabela() -> Result<Table, ErroConfig> {
    match Value::try_from(HarnessConfig::default()) {
        Ok(Value::Table(tabela)) => Ok(tabela),
        Ok(outro) => Err(ErroConfig::Interno(format!(
            "padrões viraram {} em vez de tabela",
            outro.type_str()
        ))),
        Err(erro) => Err(ErroConfig::Interno(erro.to_string())),
    }
}

fn fontes_do_arquivo(caminho: &Path) -> Result<Vec<Fonte>, ErroConfig> {
    if !caminho.exists() {
        return Err(ErroConfig::ArquivoAusente(caminho.to_path_buf()));
    }
    let texto = fs::read_to_string(caminho).map_err(|erro| ErroConfig::Leitura {
        arquivo: caminho.to_path_buf(),
        motivo: erro.to_string(),
    })?;
    let tabela: Table = toml::from_str(&texto).map_err(|erro| ErroConfig::Sintaxe {
        arquivo: caminho.to_path_buf(),
        motivo: erro.to_string(),
    })?;
    let origem = caminho.display().to_string();
    let mut folhas = Vec::new();
    achatar(&tabela, &mut Vec::new(), &mut folhas);
    Ok(folhas
        .into_iter()
        .map(|(caminho, valor)| Fonte {
            origem: origem.clone(),
            caminho,
            valor,
        })
        .collect())
}

/// Vira a árvore do arquivo numa lista de (caminho, valor). Tabela vazia não gera nada.
fn achatar(tabela: &Table, prefixo: &mut Vec<String>, saida: &mut Vec<(Vec<String>, Value)>) {
    for (chave, valor) in tabela {
        prefixo.push(chave.clone());
        match valor {
            Value::Table(sub) => achatar(sub, prefixo, saida),
            _ => saida.push((prefixo.clone(), valor.clone())),
        }
        prefixo.pop();
    }
}

fn fontes_do_ambiente(ambiente: &Ambiente) -> Result<Vec<Fonte>, ErroConfig> {
    let mut fontes = Vec::new();
    for (variavel, caminho, tipo) in VARIAVEIS_DE_CONFIG {
        let Some(texto) = ambiente
            .get(*variavel)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let valor = match tipo {
            Tipo::Texto => Value::String(texto.to_owned()),
            Tipo::Inteiro => {
                texto
                    .parse::<i64>()
                    .map(Value::Integer)
                    .map_err(|_| ErroConfig::Ambiente {
                        variavel,
                        motivo: "esperado um número inteiro".to_owned(),
                    })?
            }
            Tipo::Booleano => match texto {
                "true" => Value::Boolean(true),
                "false" => Value::Boolean(false),
                _ => {
                    return Err(ErroConfig::Ambiente {
                        variavel,
                        motivo: "use true ou false".to_owned(),
                    });
                }
            },
        };
        fontes.push(Fonte {
            origem: (*variavel).to_owned(),
            caminho: caminho.iter().map(|p| (*p).to_owned()).collect(),
            valor,
        });
    }
    Ok(fontes)
}

/// Põe `valor` no caminho dado. Se o caminho passa por algo que não é tabela, não faz nada:
/// o erro aparece depois, na desserialização, com a chave certa.
fn definir(tabela: &mut Table, caminho: &[String], valor: Value) {
    match caminho {
        [] => {}
        [ultimo] => {
            tabela.insert(ultimo.clone(), valor);
        }
        [primeiro, resto @ ..] => {
            let entrada = tabela
                .entry(primeiro.clone())
                .or_insert_with(|| Value::Table(Table::new()));
            if let Value::Table(sub) = entrada {
                definir(sub, resto, valor);
            }
        }
    }
}

/// Acha a primeira fonte que, sozinha sobre os padrões, já quebra a leitura.
fn achar_culpado(base: &Table, fontes: &[Fonte]) -> ErroConfig {
    for fonte in fontes {
        let mut teste = base.clone();
        definir(&mut teste, &fonte.caminho, fonte.valor.clone());
        if let Err(erro) = HarnessConfig::deserialize(Value::Table(teste)) {
            return ErroConfig::Chave {
                origem: fonte.origem.clone(),
                chave: fonte.caminho.join("."),
                motivo: erro.to_string(),
            };
        }
    }
    ErroConfig::Interno(
        "a configuração não leu, mas nenhuma chave isolada explica o erro".to_owned(),
    )
}

#[cfg(test)]
mod testes {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use poph_types::GatesJev;

    use super::*;

    /// Pasta temporária única para os arquivos de teste (sem dependência extra).
    fn pasta_temporaria() -> PathBuf {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!("poph-config-{}-{n}", std::process::id()));
        fs::create_dir_all(&pasta).expect("cria pasta de teste");
        pasta
    }

    fn arquivo(conteudo: &str) -> PathBuf {
        let caminho = pasta_temporaria().join("poph.toml");
        fs::write(&caminho, conteudo).expect("escreve arquivo de teste");
        caminho
    }

    fn amb(pares: &[(&str, &str)]) -> Ambiente {
        pares
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn sem_arquivo_e_sem_ambiente_vale_o_padrao() {
        let cfg = carregar(None, &Ambiente::new()).expect("carrega");
        assert_eq!(cfg, HarnessConfig::default());
    }

    #[test]
    fn precedencia_padrao_menos_arquivo_menos_ambiente() {
        let caminho = arquivo(
            "[servidor]\nporta = 2000\n\n[log]\nnivel = \"debug\"\n\n[llm]\nmodelo = \"modelo-do-arquivo\"\n",
        );
        let ambiente = amb(&[("POPH_SERVIDOR_PORTA", "3000")]);
        let cfg = carregar(Some(&caminho), &ambiente).expect("carrega");
        // Ambiente vence o arquivo.
        assert_eq!(cfg.servidor.porta, 3000);
        // Arquivo vence o padrão.
        assert_eq!(cfg.log.nivel, "debug");
        assert_eq!(cfg.llm.modelo, "modelo-do-arquivo");
        // Quem não foi definido em lugar nenhum fica com o padrão.
        assert_eq!(cfg.retencao.dias, 30);
        assert_eq!(cfg.limites.turnos_simultaneos, 200);
    }

    #[test]
    fn variavel_vazia_nao_sobrepoe_nada() {
        let caminho = arquivo("[servidor]\nporta = 2001\n");
        let cfg =
            carregar(Some(&caminho), &amb(&[("POPH_SERVIDOR_PORTA", "  ")])).expect("carrega");
        assert_eq!(cfg.servidor.porta, 2001);
    }

    #[test]
    fn sintaxe_toml_quebrada_diz_o_arquivo() {
        let caminho = arquivo("[servidor\nporta = 2000\n");
        match carregar(Some(&caminho), &Ambiente::new()) {
            Err(ErroConfig::Sintaxe { arquivo, motivo }) => {
                assert_eq!(arquivo, caminho);
                assert!(!motivo.is_empty());
            }
            outro => panic!("esperado erro de sintaxe, veio {outro:?}"),
        }
    }

    #[test]
    fn chave_desconhecida_no_arquivo_diz_arquivo_e_chave() {
        let caminho = arquivo("[servidor]\nportaa = 2000\n");
        match carregar(Some(&caminho), &Ambiente::new()) {
            Err(ErroConfig::Chave { origem, chave, .. }) => {
                assert_eq!(origem, caminho.display().to_string());
                assert_eq!(chave, "servidor.portaa");
            }
            outro => panic!("esperado erro de chave, veio {outro:?}"),
        }
    }

    #[test]
    fn tipo_errado_no_arquivo_aponta_a_chave() {
        let caminho = arquivo("[servidor]\nporta = \"abc\"\n");
        match carregar(Some(&caminho), &Ambiente::new()) {
            Err(ErroConfig::Chave { chave, .. }) => assert_eq!(chave, "servidor.porta"),
            outro => panic!("esperado erro de chave, veio {outro:?}"),
        }
    }

    #[test]
    fn variavel_com_numero_invalido_aponta_a_variavel() {
        match carregar(None, &amb(&[("POPH_SERVIDOR_PORTA", "abc")])) {
            Err(ErroConfig::Ambiente { variavel, .. }) => {
                assert_eq!(variavel, "POPH_SERVIDOR_PORTA")
            }
            outro => panic!("esperado erro de variável, veio {outro:?}"),
        }
    }

    #[test]
    fn variavel_com_porta_fora_de_faixa_cai_na_validacao() {
        match carregar(None, &amb(&[("POPH_SERVIDOR_PORTA", "80")])) {
            Err(ErroConfig::Validacao(DomainError::Validacao { campo, .. })) => {
                assert_eq!(campo, "servidor.porta")
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn arquivo_pedido_e_inexistente_e_erro_claro() {
        let caminho = pasta_temporaria().join("nao-existe.toml");
        assert!(matches!(
            carregar(Some(&caminho), &Ambiente::new()),
            Err(ErroConfig::ArquivoAusente(_))
        ));
    }

    #[test]
    fn evolution_so_existe_quando_a_secao_existe() {
        let sem = carregar(None, &Ambiente::new()).expect("carrega");
        assert!(sem.evolution.is_none());
        let caminho = arquivo("[evolution]\nurl_base = \"https://evo.exemplo.test\"\n");
        let com = carregar(Some(&caminho), &Ambiente::new()).expect("carrega");
        assert_eq!(
            com.evolution.map(|e| e.url_base),
            Some("https://evo.exemplo.test".to_owned())
        );
    }

    #[test]
    fn llm_sem_os_campos_novos_usa_os_padroes() {
        let caminho = arquivo(
            "[llm]\nprovedor = \"openai_compativel\"\nurl_base = \"https://api.exemplo.test/v1\"\n",
        );
        let cfg = carregar(Some(&caminho), &Ambiente::new()).expect("carrega");
        assert_eq!(cfg.llm.provedor, "openai_compativel");
        assert_eq!(cfg.llm.chave_env, VAR_ANTIGRAVITY);
        assert!(cfg.llm.recusa_turno_modelo);
        let chave = chave_llm(
            &amb(&[("OUTRA_CHAVE", "valor-de-teste")]),
            &cfg.llm.chave_env,
        );
        assert!(chave.is_none(), "a chave vem do nome configurado");
    }

    #[test]
    fn jev_vem_desligado_e_pode_ligar_por_ambiente() {
        let cfg = carregar(None, &Ambiente::new()).expect("carrega");
        assert!(!cfg.jev.ativo);
        let ligado = carregar(None, &amb(&[("POPH_JEV_ATIVO", "true")])).expect("carrega");
        assert!(ligado.jev.ativo);
    }

    /// Padrão de instalação: G3, G5, G8 e G9 ligados; G1, G2, G4, G6 e G7 desligados. Se alguém
    /// voltar G3, G5, G8 ou G9 para `false` no padrão, ou ligar um dos outros cinco, este teste reprova.
    #[test]
    fn gates_padrao_de_instalacao_ligam_g3_g5_g8_e_g9() {
        let cfg = carregar(None, &Ambiente::new()).expect("carrega");
        assert_eq!(cfg.jev.gates, GatesJev::padrao());
        assert!(cfg.jev.gates.g3, "G3 deve vir ligado por padrão");
        assert!(cfg.jev.gates.g5, "G5 deve vir ligado por padrão");
        assert!(cfg.jev.gates.g8, "G8 deve vir ligado por padrão");
        assert!(cfg.jev.gates.g9, "G9 deve vir ligado por padrão");
        for (nome, ligado) in [
            ("g1", cfg.jev.gates.g1),
            ("g2", cfg.jev.gates.g2),
            ("g4", cfg.jev.gates.g4),
            ("g6", cfg.jev.gates.g6),
            ("g7", cfg.jev.gates.g7),
        ] {
            assert!(!ligado, "{nome} deve continuar desligado por padrão");
        }
    }

    /// Arquivo que liga o Jev mas não fala de gates: G3, G5 e G8 continuam ligados.
    #[test]
    fn arquivo_sem_a_secao_de_gates_mantem_g3_g5_e_g8_ligados() {
        let caminho = arquivo("[jev]\nativo = true\n");
        let cfg = carregar(Some(&caminho), &Ambiente::new()).expect("carrega");
        assert!(cfg.jev.ativo);
        assert!(cfg.jev.gates.g3 && cfg.jev.gates.g5 && cfg.jev.gates.g8);
        assert!(!cfg.jev.gates.g1 && !cfg.jev.gates.g2);
    }

    /// Arquivo com `[jev.gates]` parcial: o campo que não aparece cai no padrão, campo a campo.
    #[test]
    fn arquivo_com_gates_parcial_so_muda_o_que_escreveu() {
        let caminho = arquivo("[jev]\nativo = true\n\n[jev.gates]\ng1 = true\n");
        let cfg = carregar(Some(&caminho), &Ambiente::new()).expect("carrega");
        assert!(cfg.jev.gates.g1, "o que o arquivo liga, fica ligado");
        assert!(
            cfg.jev.gates.g3 && cfg.jev.gates.g5 && cfg.jev.gates.g8,
            "G3, G5 e G8 ausentes ficam no padrão"
        );
        assert!(!cfg.jev.gates.g2);
    }

    /// Desligar G3, G5 e G8 no arquivo vale de verdade: é o que o operador usa para voltar atrás.
    #[test]
    fn arquivo_pode_desligar_g3_g5_e_g8() {
        let caminho =
            arquivo("[jev]\nativo = true\n\n[jev.gates]\ng3 = false\ng5 = false\ng8 = false\n");
        let cfg = carregar(Some(&caminho), &Ambiente::new()).expect("carrega");
        assert!(!cfg.jev.gates.g3, "g3 = false no arquivo desliga o G3");
        assert!(!cfg.jev.gates.g5, "g5 = false no arquivo desliga o G5");
        assert!(!cfg.jev.gates.g8, "g8 = false no arquivo desliga o G8");
        // O G9 não foi mencionado no arquivo: continua no padrão de instalação.
        assert_eq!(
            cfg.jev.gates,
            GatesJev {
                g9: true,
                ..GatesJev::default()
            }
        );
    }

    #[test]
    fn resolver_arquivo_respeita_flag_depois_variavel_depois_pasta() {
        let pasta = pasta_temporaria();
        let vazio = Ambiente::new();
        assert_eq!(resolver_arquivo(None, &vazio, &pasta), None);
        fs::write(pasta.join(ARQUIVO_PADRAO), "").expect("escreve");
        assert_eq!(
            resolver_arquivo(None, &vazio, &pasta),
            Some(pasta.join(ARQUIVO_PADRAO))
        );
        let por_variavel = amb(&[(VAR_CONFIG, "/tmp/outro.toml")]);
        assert_eq!(
            resolver_arquivo(None, &por_variavel, &pasta),
            Some(PathBuf::from("/tmp/outro.toml"))
        );
        assert_eq!(
            resolver_arquivo(Some(Path::new("/tmp/flag.toml")), &por_variavel, &pasta),
            Some(PathBuf::from("/tmp/flag.toml"))
        );
    }

    #[test]
    fn segredos_vem_do_ambiente_e_vazio_conta_como_ausente() {
        let ambiente = amb(&[(VAR_CIFRA, "segredo-de-teste-cifra")]);
        let s = segredos(&ambiente);
        assert!(s.cifra.is_some());
        assert!(s.typesafe.is_none());
        assert!(!format!("{s:?}").contains("segredo-de-teste-cifra"));
    }

    /// A chave do provedor é lida pelo nome configurado, e não pelo nome padrão.
    #[test]
    fn chave_do_llm_vem_do_nome_configurado() {
        let ambiente = amb(&[
            (VAR_ANTIGRAVITY, "chave-padrao"),
            ("MINHA_CHAVE_LLM", "chave-propria"),
        ]);
        let padrao = chave_llm(&ambiente, VAR_ANTIGRAVITY).expect("chave padrão");
        assert_eq!(padrao.expor(), "chave-padrao");
        let propria = chave_llm(&ambiente, "MINHA_CHAVE_LLM").expect("chave própria");
        assert_eq!(propria.expor(), "chave-propria");
        assert!(chave_llm(&ambiente, "OUTRA_CHAVE").is_none());
        assert!(chave_llm(&ambiente, VAR_ANTIGRAVITY).is_some_and(|c| !c.expor().is_empty()));
    }

    /// Sem arquivo e sem ambiente, a seção `[fluxos]` vale o padrão do PRD §9.
    #[test]
    fn fluxos_sem_arquivo_e_sem_ambiente_vale_o_padrao() {
        let cfg = carregar(None, &Ambiente::new()).expect("carrega");
        assert_eq!(cfg.fluxos, poph_types::FluxosConfig::default());
    }

    /// Valor lido de cada chave de `[fluxos]` que tem variável de ambiente, pelo nome da chave.
    fn valor_de(fluxos: &poph_types::FluxosConfig, chave: &str) -> u64 {
        match chave {
            "max_fluxos_por_org" => u64::from(fluxos.max_fluxos_por_org),
            "max_fonte_bytes" => fluxos.max_fonte_bytes,
            "max_nos" => u64::from(fluxos.max_nos),
            "max_dados_bytes" => fluxos.max_dados_bytes,
            "max_ticks_regra" => fluxos.max_ticks_regra,
            "max_heap_regra" => fluxos.max_heap_regra,
            "max_saltos_decisao" => u64::from(fluxos.max_saltos_decisao),
            "cache_revisoes_mb" => u64::from(fluxos.cache_revisoes_mb),
            outro => panic!("chave sem leitura no teste: {outro}"),
        }
    }

    /// Cada chave numérica de `[fluxos]`: o arquivo define um valor, a variável define outro, e a
    /// variável vence. Com a variável ausente, o valor do arquivo aparece.
    #[test]
    fn variavel_de_cada_chave_de_fluxos_vence_o_arquivo() {
        let casos = [
            (
                "POPH_FLUXOS_MAX_FLUXOS_POR_ORG",
                "max_fluxos_por_org",
                "50",
                "200",
            ),
            (
                "POPH_FLUXOS_MAX_FONTE_BYTES",
                "max_fonte_bytes",
                "2048",
                "4096",
            ),
            ("POPH_FLUXOS_MAX_NOS", "max_nos", "20", "30"),
            (
                "POPH_FLUXOS_MAX_DADOS_BYTES",
                "max_dados_bytes",
                "2048",
                "4096",
            ),
            (
                "POPH_FLUXOS_MAX_TICKS_REGRA",
                "max_ticks_regra",
                "500",
                "600",
            ),
            (
                "POPH_FLUXOS_MAX_HEAP_REGRA",
                "max_heap_regra",
                "131072",
                "262144",
            ),
            (
                "POPH_FLUXOS_MAX_SALTOS_DECISAO",
                "max_saltos_decisao",
                "4",
                "5",
            ),
            (
                "POPH_FLUXOS_CACHE_REVISOES_MB",
                "cache_revisoes_mb",
                "8",
                "32",
            ),
        ];
        for (variavel, chave, do_arquivo, do_ambiente) in casos {
            let caminho = arquivo(&format!("[fluxos]\n{chave} = {do_arquivo}\n"));
            let so_arquivo = carregar(Some(&caminho), &Ambiente::new()).expect("carrega");
            assert_eq!(
                valor_de(&so_arquivo.fluxos, chave),
                do_arquivo.parse::<u64>().unwrap(),
                "{chave} vem do arquivo"
            );
            let vence =
                carregar(Some(&caminho), &amb(&[(variavel, do_ambiente)])).expect("carrega");
            assert_eq!(
                valor_de(&vence.fluxos, chave),
                do_ambiente.parse::<u64>().unwrap(),
                "{variavel} vence o arquivo"
            );
        }
    }

    #[test]
    fn variavel_de_ativo_vence_o_arquivo_e_aceita_so_true_ou_false() {
        let caminho = arquivo("[fluxos]\nativo = true\n");
        let desligado =
            carregar(Some(&caminho), &amb(&[("POPH_FLUXOS_ATIVO", "false")])).expect("carrega");
        assert!(!desligado.fluxos.ativo);
        match carregar(None, &amb(&[("POPH_FLUXOS_ATIVO", "sim")])) {
            Err(ErroConfig::Ambiente { variavel, .. }) => assert_eq!(variavel, "POPH_FLUXOS_ATIVO"),
            outro => panic!("esperado erro de variável, veio {outro:?}"),
        }
    }

    #[test]
    fn variavel_de_fluxos_com_texto_aponta_a_variavel() {
        match carregar(None, &amb(&[("POPH_FLUXOS_MAX_NOS", "abc")])) {
            Err(ErroConfig::Ambiente { variavel, .. }) => {
                assert_eq!(variavel, "POPH_FLUXOS_MAX_NOS")
            }
            outro => panic!("esperado erro de variável, veio {outro:?}"),
        }
    }

    /// Número negativo não cabe no tipo: a chave culpada é `fluxos.max_nos`, e a origem é a variável.
    #[test]
    fn variavel_de_fluxos_negativa_aponta_a_chave_e_a_variavel() {
        match carregar(None, &amb(&[("POPH_FLUXOS_MAX_NOS", "-1")])) {
            Err(ErroConfig::Chave { origem, chave, .. }) => {
                assert_eq!(origem, "POPH_FLUXOS_MAX_NOS");
                assert_eq!(chave, "fluxos.max_nos");
            }
            outro => panic!("esperado erro de chave, veio {outro:?}"),
        }
    }

    #[test]
    fn variavel_de_fluxos_fora_da_faixa_cai_na_validacao_com_a_chave() {
        match carregar(None, &amb(&[("POPH_FLUXOS_MAX_NOS", "61")])) {
            Err(ErroConfig::Validacao(DomainError::Validacao { campo, .. })) => {
                assert_eq!(campo, "fluxos.max_nos")
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn arquivo_com_fluxos_fora_da_faixa_cai_na_validacao_com_a_chave() {
        let caminho = arquivo("[fluxos]\nmax_saltos_decisao = 0\n");
        match carregar(Some(&caminho), &Ambiente::new()) {
            Err(ErroConfig::Validacao(DomainError::Validacao { campo, .. })) => {
                assert_eq!(campo, "fluxos.max_saltos_decisao")
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn arquivo_com_tipo_errado_em_fluxos_aponta_a_chave() {
        let caminho = arquivo("[fluxos]\ncache_revisoes_mb = \"muito\"\n");
        match carregar(Some(&caminho), &Ambiente::new()) {
            Err(ErroConfig::Chave { chave, .. }) => assert_eq!(chave, "fluxos.cache_revisoes_mb"),
            outro => panic!("esperado erro de chave, veio {outro:?}"),
        }
    }

    /// Desligar o motor pelo arquivo vale, e o G9 segue a mesma regra dos outros gates.
    #[test]
    fn arquivo_pode_desligar_o_motor_e_o_g9() {
        let caminho = arquivo("[fluxos]\nativo = false\n\n[jev.gates]\ng9 = false\n");
        let cfg = carregar(Some(&caminho), &Ambiente::new()).expect("carrega");
        assert!(!cfg.fluxos.ativo);
        assert!(!cfg.jev.gates.g9);
    }
}

//! `poph doctor`: confere o ambiente antes de subir o servidor.
//!
//! O que confere: binário e compilador usados, arquivo de configuração, pasta de dados (existe
//! ou pode ser criada, e aceita escrita), os bancos `harness.db` e `rag.db` (formato e versão de
//! esquema), e as variáveis de segredo obrigatórias.
//!
//! Regras:
//! - Segredo aparece só como "definida" ou "ausente". O valor nunca entra no relatório.
//! - Ausência de banco é aviso (o banco nasce na HR-07). Erro real faz a saída ser diferente de 0.
//! - Escreve só um arquivo de teste na pasta de dados, e apaga logo em seguida.

use std::fs::{self, File};
use std::path::Path;

use poph_types::{HarnessConfig, Secret};
use serde::Serialize;
use tracing::debug;

use crate::config::{
    self, ErroConfig, Segredos, VAR_ANTIGRAVITY, VAR_CIFRA, VAR_EVOLUTION, VAR_TYPESAFE,
};

/// Versão do formato do `--json`. Muda só se a forma do JSON mudar.
pub const VERSAO_RELATORIO: u32 = 1;

/// Resultado de uma verificação.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Nivel {
    /// Está certo.
    Ok,
    /// Não impede o uso, mas merece atenção (ex.: banco ainda não criado).
    Aviso,
    /// Impede o uso; a saída do comando vira diferente de zero.
    Erro,
}

/// Uma linha do relatório.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verificacao {
    /// O que foi conferido.
    pub nome: String,
    /// Resultado.
    pub nivel: Nivel,
    /// Explicação em uma frase. Nunca contém valor de segredo.
    pub detalhe: String,
}

/// Dados do binário que está rodando.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Binario {
    /// Nome do binário.
    pub nome: &'static str,
    /// Versão do pacote.
    pub versao: &'static str,
    /// Compilador usado para gerar este binário.
    pub rustc: &'static str,
}

/// Relatório completo do `poph doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Relatorio {
    /// Versão do formato deste JSON (`VERSAO_RELATORIO`).
    pub versao_relatorio: u32,
    /// `ok` quando nenhuma verificação é erro; senão `erro`.
    pub status: &'static str,
    /// Binário que gerou o relatório.
    pub binario: Binario,
    /// Verificações, na ordem em que foram feitas.
    pub verificacoes: Vec<Verificacao>,
}

/// O que o doctor precisa saber para conferir.
#[derive(Debug, Clone, Copy)]
pub struct Entrada<'a> {
    /// Configuração carregada, ou o erro que impediu carregar.
    pub config: Result<&'a HarnessConfig, &'a ErroConfig>,
    /// Arquivo de configuração usado, se houver.
    pub arquivo: Option<&'a Path>,
    /// Segredos lidos do ambiente (só a presença é conferida).
    pub segredos: &'a Segredos,
    pub chave_llm: Option<&'a Secret>,
    pub ambiente: Option<&'a crate::config::Ambiente>,
    pub agentes_por_provedor: Option<&'a [(String, u64)]>,
}

/// Faz todas as verificações e devolve o relatório.
pub fn gerar(entrada: Entrada<'_>) -> Relatorio {
    let mut verificacoes = vec![binario_verificacao(), arquivo_verificacao(entrada.arquivo)];

    match entrada.config {
        Err(erro) => verificacoes.push(Verificacao {
            nome: "configuração".to_owned(),
            nivel: Nivel::Erro,
            detalhe: erro.to_string(),
        }),
        Ok(config) => {
            verificacoes.push(Verificacao {
                nome: "configuração".to_owned(),
                nivel: Nivel::Ok,
                detalhe: "valores válidos".to_owned(),
            });
            verificacoes.push(pasta_de_dados(&config.data.directory));
            verificacoes.push(banco("harness.db", &config.harness_db()));
            verificacoes.push(banco("rag.db", &config.rag_db()));
            verificacoes.extend(fluxos_verificacoes(&config.flows));
            verificacoes.extend(provedores_verificacoes(config, entrada));
        }
    }

    let jev_ativo = matches!(entrada.config, Ok(config) if config.jev.enabled);
    let evolution_configurada = matches!(entrada.config, Ok(config) if config.evolution.is_some());

    verificacoes.push(variavel_obrigatoria(
        VAR_CIFRA,
        entrada.segredos.cifra.is_some(),
    ));
    let nome_da_chave_llm = match entrada.config {
        Ok(config) => config.llm.api_key_env.as_str(),
        Err(_) => VAR_ANTIGRAVITY,
    };
    verificacoes.push(variavel_obrigatoria(
        nome_da_chave_llm,
        entrada.chave_llm.is_some(),
    ));
    if jev_ativo {
        verificacoes.push(variavel_obrigatoria(
            VAR_TYPESAFE,
            entrada.segredos.typesafe.is_some(),
        ));
    }
    if evolution_configurada {
        verificacoes.push(variavel_obrigatoria(
            VAR_EVOLUTION,
            entrada.segredos.evolution.is_some(),
        ));
    }

    for verificacao in &verificacoes {
        debug!(verificacao = %verificacao.nome, nivel = ?verificacao.nivel, "verificação do doctor");
    }

    let tem_erro = verificacoes.iter().any(|v| v.nivel == Nivel::Erro);
    Relatorio {
        versao_relatorio: VERSAO_RELATORIO,
        status: if tem_erro { "erro" } else { "ok" },
        binario: Binario {
            nome: env!("CARGO_PKG_NAME"),
            versao: env!("CARGO_PKG_VERSION"),
            rustc: env!("POPH_RUSTC_VERSAO"),
        },
        verificacoes,
    }
}

impl Relatorio {
    /// `true` se alguma verificação é erro.
    pub fn tem_erro(&self) -> bool {
        self.status == "erro"
    }

    /// Texto para ler no terminal.
    pub fn texto(&self) -> String {
        let mut saida = String::from("POP Harness: diagnóstico do ambiente\n\n");
        for verificacao in &self.verificacoes {
            let marca = match verificacao.nivel {
                Nivel::Ok => "[ok]   ",
                Nivel::Aviso => "[aviso]",
                Nivel::Erro => "[erro] ",
            };
            saida.push_str(&format!(
                "{marca} {}: {}\n",
                verificacao.nome, verificacao.detalhe
            ));
        }
        let erros = self
            .verificacoes
            .iter()
            .filter(|v| v.nivel == Nivel::Erro)
            .count();
        let avisos = self
            .verificacoes
            .iter()
            .filter(|v| v.nivel == Nivel::Aviso)
            .count();
        saida.push('\n');
        if erros == 0 {
            saida.push_str(&format!("Ambiente pronto ({avisos} aviso(s)).\n"));
        } else {
            saida.push_str(&format!(
                "{erros} erro(s) a corrigir ({avisos} aviso(s)).\n"
            ));
        }
        saida
    }

    /// JSON estável (versão do formato em `versao_relatorio`), sem segredo.
    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|erro| {
            format!("{{\"versao_relatorio\":{VERSAO_RELATORIO},\"status\":\"erro\",\"detalhe\":\"{erro}\"}}")
        })
    }
}

fn binario_verificacao() -> Verificacao {
    Verificacao {
        nome: "binário e toolchain".to_owned(),
        nivel: Nivel::Ok,
        detalhe: format!(
            "poph {}, compilado com {}",
            env!("CARGO_PKG_VERSION"),
            env!("POPH_RUSTC_VERSAO")
        ),
    }
}

fn arquivo_verificacao(arquivo: Option<&Path>) -> Verificacao {
    let (nivel, detalhe) = match arquivo {
        None => (
            Nivel::Ok,
            "nenhum arquivo indicado; usando padrões e ambiente".to_owned(),
        ),
        Some(caminho) if caminho.is_file() => (Nivel::Ok, format!("lido de {}", caminho.display())),
        Some(caminho) => (Nivel::Erro, format!("{} não encontrado", caminho.display())),
    };
    Verificacao {
        nome: "arquivo de configuração".to_owned(),
        nivel,
        detalhe,
    }
}

fn provedores_verificacoes(config: &HarnessConfig, entrada: Entrada<'_>) -> Vec<Verificacao> {
    let mut verificacoes = Vec::new();
    for (nome, sobreposicao) in &config.providers {
        let efetivo = sobreposicao.effective(&config.llm);
        let chave = match entrada.ambiente {
            Some(ambiente) => config::chave_llm(ambiente, &efetivo.api_key_env),
            None => None,
        };
        let (nivel, chave_texto) = match (entrada.ambiente, chave.is_some()) {
            (None, _) => (Nivel::Ok, String::new()),
            (Some(_), true) => (Nivel::Ok, format!("chave {} definida", efetivo.api_key_env)),
            (Some(_), false) => (
                Nivel::Aviso,
                format!("chave {} ausente no ambiente", efetivo.api_key_env),
            ),
        };
        let sufixo = if chave_texto.is_empty() {
            String::new()
        } else {
            format!(" · {chave_texto}")
        };
        verificacoes.push(Verificacao {
            nome: format!("provedor {nome}"),
            nivel,
            detalhe: format!(
                "{} · {} chamadas simultâneas{sufixo}",
                efetivo.url_base, efetivo.concurrent_calls
            ),
        });
    }
    let Some(contagem) = entrada.agentes_por_provedor else {
        return verificacoes;
    };
    let mut orfaos = Vec::new();
    for (escolha, total) in contagem {
        if *total == 0
            || HarnessConfig::provider_is_default(escolha)
            || config.providers.contains_key(escolha.trim())
        {
            continue;
        }
        orfaos.push(format!("{escolha} ({total})"));
    }
    if !orfaos.is_empty() {
        verificacoes.push(Verificacao {
            nome: "provedor dos agentes".to_owned(),
            nivel: Nivel::Aviso,
            detalhe: format!(
                "{} agente(s) apontam para provedor inexistente: {}",
                orfaos.len(),
                orfaos.join(", ")
            ),
        });
    }
    verificacoes
}

fn pasta_de_dados(diretorio: &Path) -> Verificacao {
    let caminho = std::path::absolute(diretorio).unwrap_or_else(|_| diretorio.to_path_buf());
    let (nivel, detalhe) = if caminho.is_dir() {
        if gravavel(&caminho) {
            (
                Nivel::Ok,
                format!("{} existe e aceita escrita", caminho.display()),
            )
        } else {
            (
                Nivel::Erro,
                format!("{} existe, mas não aceita escrita", caminho.display()),
            )
        }
    } else if caminho.exists() {
        (
            Nivel::Erro,
            format!("{} existe e não é uma pasta", caminho.display()),
        )
    } else {
        match caminho.ancestors().skip(1).find(|pai| pai.is_dir()) {
            Some(pai) if gravavel(pai) => (
                Nivel::Ok,
                format!(
                    "{} ainda não existe; será criada (a pasta acima aceita escrita)",
                    caminho.display()
                ),
            ),
            Some(pai) => (
                Nivel::Erro,
                format!(
                    "{} não existe e {} não aceita escrita",
                    caminho.display(),
                    pai.display()
                ),
            ),
            None => (
                Nivel::Erro,
                format!("{} não tem pasta-pai existente", caminho.display()),
            ),
        }
    };
    Verificacao {
        nome: "pasta de dados".to_owned(),
        nivel,
        detalhe,
    }
}

/// Confere escrita criando e apagando um arquivo de teste na pasta.
fn gravavel(pasta: &Path) -> bool {
    let teste = pasta.join(format!(".poph-doctor-{}.tmp", std::process::id()));
    let ok = File::create(&teste).is_ok();
    let _ = fs::remove_file(&teste);
    ok
}

fn banco(nome: &str, caminho: &Path) -> Verificacao {
    let nome = format!("banco {nome}");
    let (nivel, detalhe) = if !caminho.exists() {
        (
            Nivel::Aviso,
            format!(
                "{} ainda não existe; crie com `poph migrate`",
                caminho.display()
            ),
        )
    } else if caminho.is_dir() {
        (
            Nivel::Erro,
            format!("{} é uma pasta, não um banco", caminho.display()),
        )
    } else {
        // A versão vem da tabela `schema_migrations` (a mesma que o `poph migrate` mantém).
        match poph_store::file_version(caminho) {
            Ok(Some(versao)) => (
                Nivel::Ok,
                format!("formato SQLite, esquema versão {versao}"),
            ),
            Ok(None) => (
                Nivel::Aviso,
                format!(
                    "{} existe, mas ainda não foi migrado; rode `poph migrate`",
                    caminho.display()
                ),
            ),
            Err(motivo) => (
                Nivel::Erro,
                format!(
                    "{} não é um banco SQLite válido: {motivo}",
                    caminho.display()
                ),
            ),
        }
    };
    Verificacao {
        nome,
        nivel,
        detalhe,
    }
}

/// Estado do módulo de fluxos (FX-26): se o motor está ligado e os tetos em uso.
///
/// Desligado é aviso, não erro: a Ficha continua funcionando, só o motor fica parado. Os tetos
/// aparecem sempre, porque valem mesmo com o motor desligado (a validação roda antes).
fn fluxos_verificacoes(fluxos: &poph_types::FlowsConfig) -> Vec<Verificacao> {
    let (nivel, detalhe) = if fluxos.enabled {
        (Nivel::Ok, "sim: o motor de fluxos roda".to_owned())
    } else {
        (
            Nivel::Aviso,
            "não: o motor de fluxos está desligado; as Fichas continuam sendo criadas".to_owned(),
        )
    };
    vec![
        Verificacao {
            nome: "fluxos.ativo".to_owned(),
            nivel,
            detalhe,
        },
        Verificacao {
            nome: "fluxos.tetos".to_owned(),
            nivel: Nivel::Ok,
            detalhe: format!(
                "fluxos por org {}; script {} bytes; nós {}; dados {} bytes; passos de regra {}; \
                 memória de regra {} bytes; saltos de decisão {}; cache de revisões {} MB",
                fluxos.max_flows_per_org,
                fluxos.max_source_bytes,
                fluxos.max_nodes,
                fluxos.max_data_bytes,
                fluxos.max_rule_ticks,
                fluxos.max_rule_heap,
                fluxos.max_decision_hops,
                fluxos.revision_cache_mb,
            ),
        },
    ]
}

fn variavel_obrigatoria(nome: &str, presente: bool) -> Verificacao {
    let (nivel, detalhe) = if presente {
        (Nivel::Ok, "definida (valor não exibido)".to_owned())
    } else {
        (Nivel::Erro, format!("ausente; defina {nome} no ambiente"))
    };
    Verificacao {
        nome: format!("variável {nome}"),
        nivel,
        detalhe,
    }
}

#[cfg(test)]
mod testes {
    use std::path::PathBuf;

    use crate::config::Ambiente;

    use super::*;

    fn pasta_temporaria(rotulo: &str) -> PathBuf {
        let pasta = std::env::temp_dir().join(format!(
            "poph-doctor-{rotulo}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&pasta).expect("cria pasta de teste");
        pasta
    }

    /// Configuração de teste apontando para uma pasta própria.
    fn config_em(pasta: &Path) -> HarnessConfig {
        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config
    }

    fn segredos_completos() -> Segredos {
        crate::config::segredos(&Ambiente::from([
            (VAR_CIFRA.to_owned(), "SEGREDO-CIFRA-XYZ-1".to_owned()),
            (VAR_ANTIGRAVITY.to_owned(), "SEGREDO-LLM-XYZ-2".to_owned()),
        ]))
    }

    fn chave_llm_de_teste() -> Secret {
        Secret::new("SEGREDO-LLM-XYZ-2".to_owned())
    }

    fn achar<'a>(relatorio: &'a Relatorio, nome: &str) -> &'a Verificacao {
        relatorio
            .verificacoes
            .iter()
            .find(|v| v.nome == nome)
            .unwrap_or_else(|| panic!("verificação {nome} não encontrada"))
    }

    #[test]
    fn ambiente_correto_sai_sem_erro_e_cria_nada_fora_da_pasta_de_dados() {
        let pasta = pasta_temporaria("ok");
        let config = config_em(&pasta);
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos_completos(),
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        assert!(!relatorio.tem_erro(), "{}", relatorio.texto());
        assert_eq!(achar(&relatorio, "banco harness.db").nivel, Nivel::Aviso);
        assert_eq!(achar(&relatorio, "banco rag.db").nivel, Nivel::Aviso);
    }

    #[test]
    fn doctor_lista_provedores_e_avisa_agente_com_provedor_inexistente() {
        let pasta = pasta_temporaria("provedores");
        let mut config = config_em(&pasta);
        config.providers.insert(
            "openai-forte".to_owned(),
            poph_types::ProviderConfig {
                url_base: Some("https://api.exemplo.com/v1".to_owned()),
                concurrent_calls: Some(4),
                ..poph_types::ProviderConfig::default()
            },
        );
        let ambiente = Ambiente::from([
            (VAR_CIFRA.to_owned(), "SEGREDO-CIFRA-XYZ-1".to_owned()),
            (VAR_ANTIGRAVITY.to_owned(), "SEGREDO-LLM-XYZ-2".to_owned()),
        ]);
        let agentes = vec![
            ("antigravity2api".to_owned(), 1_u64),
            ("openai-forte".to_owned(), 2_u64),
            ("sumiu-do-toml".to_owned(), 3_u64),
        ];
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos_completos(),
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: Some(&ambiente),
            agentes_por_provedor: Some(&agentes),
        });

        let linha = achar(&relatorio, "provedor openai-forte");
        assert_eq!(linha.nivel, Nivel::Ok, "{}", linha.detalhe);
        assert!(
            linha.detalhe.contains("https://api.exemplo.com/v1"),
            "{}",
            linha.detalhe
        );
        assert!(
            linha.detalhe.contains("4 chamadas simultâneas"),
            "{}",
            linha.detalhe
        );
        assert!(linha.detalhe.contains(VAR_ANTIGRAVITY), "{}", linha.detalhe);
        assert!(linha.detalhe.contains("definida"), "{}", linha.detalhe);

        let orfaos = achar(&relatorio, "provedor dos agentes");
        assert_eq!(orfaos.nivel, Nivel::Aviso);
        assert!(
            orfaos.detalhe.contains("sumiu-do-toml"),
            "{}",
            orfaos.detalhe
        );
        assert!(
            !orfaos.detalhe.contains("openai-forte"),
            "{}",
            orfaos.detalhe
        );
    }

    /// O doctor mostra se o motor de fluxos roda e os tetos em uso (FX-26).
    #[test]
    fn doctor_lista_fluxos_ativo_e_os_tetos() {
        let pasta = pasta_temporaria("fluxos");
        let config = config_em(&pasta);
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos_completos(),
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        let ativo = achar(&relatorio, "fluxos.ativo");
        assert_eq!(ativo.nivel, Nivel::Ok);
        assert!(ativo.detalhe.starts_with("sim"), "{}", ativo.detalhe);
        let tetos = achar(&relatorio, "fluxos.tetos");
        assert!(
            tetos.detalhe.contains("cache de revisões 16 MB"),
            "{}",
            tetos.detalhe
        );
        assert!(tetos.detalhe.contains("nós 60"), "{}", tetos.detalhe);
        assert!(
            tetos.detalhe.contains("saltos de decisão 10"),
            "{}",
            tetos.detalhe
        );
        assert!(relatorio.texto().contains("fluxos.tetos"));
    }

    /// Motor desligado é aviso, e não erro: as Fichas continuam.
    #[test]
    fn doctor_avisa_motor_de_fluxos_desligado_sem_erro() {
        let pasta = pasta_temporaria("fluxos-off");
        let mut config = config_em(&pasta);
        config.flows.enabled = false;
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos_completos(),
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        let ativo = achar(&relatorio, "fluxos.ativo");
        assert_eq!(ativo.nivel, Nivel::Aviso);
        assert!(ativo.detalhe.starts_with("não"), "{}", ativo.detalhe);
        assert!(!relatorio.tem_erro(), "{}", relatorio.texto());
    }

    #[test]
    fn variavel_ausente_vira_erro_que_diz_o_nome_da_variavel() {
        let pasta = pasta_temporaria("sem-cifra");
        let config = config_em(&pasta);
        let segredos = crate::config::segredos(&Ambiente::from([(
            VAR_ANTIGRAVITY.to_owned(),
            "SEGREDO-LLM-XYZ-2".to_owned(),
        )]));
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos,
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        assert!(relatorio.tem_erro());
        let v = achar(&relatorio, "variável POPH_CHAVE_CIFRA");
        assert_eq!(v.nivel, Nivel::Erro);
        assert!(v.detalhe.contains("POPH_CHAVE_CIFRA"));
    }

    #[test]
    fn nome_da_chave_do_llm_vem_da_configuracao() {
        let pasta = pasta_temporaria("chave-llm");
        let mut config = config_em(&pasta);
        config.llm.api_key_env = "MINHA_CHAVE_LLM".to_owned();
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos_completos(),
            chave_llm: None,
            ambiente: None,
            agentes_por_provedor: None,
        });
        let v = achar(&relatorio, "variável MINHA_CHAVE_LLM");
        assert_eq!(v.nivel, Nivel::Erro);
        assert!(v.detalhe.contains("MINHA_CHAVE_LLM"), "{}", v.detalhe);
        assert!(!relatorio.texto().contains("variável ANTIGRAVITY_API_KEY"));
    }

    #[test]
    fn chave_do_jev_so_e_exigida_com_jev_ligado() {
        let pasta = pasta_temporaria("jev");
        let mut config = config_em(&pasta);
        let segredos = segredos_completos();
        let desligado = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos,
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        assert!(
            desligado
                .verificacoes
                .iter()
                .all(|v| v.nome != "variável TYPESAFE_API_KEY")
        );
        config.jev.enabled = true;
        let ligado = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos,
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        assert_eq!(
            achar(&ligado, "variável TYPESAFE_API_KEY").nivel,
            Nivel::Erro
        );
    }

    #[test]
    fn segredo_nunca_aparece_em_texto_nem_em_json() {
        let pasta = pasta_temporaria("segredo");
        let config = config_em(&pasta);
        let segredos = segredos_completos();
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos,
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        let texto = relatorio.texto();
        let json = relatorio.json();
        for valor in ["SEGREDO-CIFRA-XYZ-1", "SEGREDO-LLM-XYZ-2"] {
            assert!(!texto.contains(valor), "vazou no texto");
            assert!(!json.contains(valor), "vazou no JSON");
        }
    }

    #[test]
    fn json_tem_formato_estavel() {
        let pasta = pasta_temporaria("json");
        let config = config_em(&pasta);
        let relatorio = gerar(Entrada {
            config: Ok(&config),
            arquivo: None,
            segredos: &segredos_completos(),
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        let valor: serde_json::Value =
            serde_json::from_str(&relatorio.json()).expect("JSON válido");
        assert_eq!(valor["versao_relatorio"], 1);
        assert_eq!(valor["status"], "ok");
        assert_eq!(valor["binario"]["nome"], "poph");
        assert!(valor["verificacoes"].is_array());
        let primeira = &valor["verificacoes"][0];
        assert!(primeira["nome"].is_string());
        assert!(["ok", "aviso", "erro"].contains(&primeira["nivel"].as_str().unwrap_or("")));
    }

    #[test]
    fn banco_migrado_mostra_a_versao_do_esquema() {
        let pasta = pasta_temporaria("banco");
        let harness = pasta.join("harness.db");
        let rag = pasta.join("rag.db");
        let store = poph_store::Store::open_at(&harness, &rag).expect("cria os bancos");
        drop(store);

        let v = banco("harness.db", &harness);
        assert_eq!(v.nivel, Nivel::Ok, "{}", v.detalhe);
        let esperado = format!("versão {}", poph_store::harness_target_version());
        assert!(v.detalhe.contains(&esperado), "{}", v.detalhe);

        let r = banco("rag.db", &rag);
        assert_eq!(r.nivel, Nivel::Ok, "{}", r.detalhe);
        assert!(r.detalhe.contains("versão 0"), "{}", r.detalhe);
    }

    #[test]
    fn banco_existente_sem_migracao_vira_aviso() {
        let pasta = pasta_temporaria("sem-migracao");
        let caminho = pasta.join("rag.db");
        fs::write(&caminho, b"").expect("escreve arquivo vazio");
        let v = banco("rag.db", &caminho);
        assert_eq!(v.nivel, Nivel::Aviso, "{}", v.detalhe);
        assert!(v.detalhe.contains("poph migrate"), "{}", v.detalhe);
    }

    #[test]
    fn arquivo_que_nao_e_sqlite_vira_erro() {
        let pasta = pasta_temporaria("lixo");
        let caminho = pasta.join("rag.db");
        fs::write(&caminho, b"isto nao e um banco").expect("escreve");
        assert_eq!(banco("rag.db", &caminho).nivel, Nivel::Erro);
    }

    #[test]
    fn pasta_de_dados_como_arquivo_vira_erro() {
        let pasta = pasta_temporaria("arquivo-no-lugar");
        let caminho = pasta.join("dados");
        fs::write(&caminho, b"x").expect("escreve");
        assert_eq!(pasta_de_dados(&caminho).nivel, Nivel::Erro);
    }

    #[test]
    fn erro_de_configuracao_aparece_no_relatorio() {
        let erro = ErroConfig::Chave {
            origem: "poph.toml".to_owned(),
            chave: "servidor.portaa".to_owned(),
            motivo: "unknown field".to_owned(),
        };
        let relatorio = gerar(Entrada {
            config: Err(&erro),
            arquivo: None,
            segredos: &segredos_completos(),
            chave_llm: Some(&chave_llm_de_teste()),
            ambiente: None,
            agentes_por_provedor: None,
        });
        assert!(relatorio.tem_erro());
        assert!(relatorio.texto().contains("servidor.portaa"));
    }
}

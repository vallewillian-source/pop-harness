//! Roteamento estático entre instâncias (HR-50a): leitura, validação e recarga do `cluster.toml`.
//!
//! O mapa é **configuração de arquivo**, separado do `poph.toml` e igual em todas as instâncias
//! (PRD §12). O caminho vem da raiz de composição (`poph`): `POPH_CLUSTER` quando definido, senão
//! `cluster.toml` no mesmo diretório do `poph.toml`. O formato e as regras vivem em
//! [`poph_types::config::ClusterConfig`]; aqui ficam a leitura do arquivo, os erros com origem e
//! chave, o estado vivo do processo e a troca atômica na recarga.
//!
//! **Segredo:** o `shared_secret` só existe em `POPH_CLUSTER_SECRET` (ambiente). Nesta rodada ele
//! é lido na subida (ou injetado pelo teste) e guardado fora do tipo do mapa; nunca é lido do
//! arquivo, impresso, publicado em resposta ou gravado em log.
//!
//! **Recarga:** relê o arquivo e troca o mapa **só se a leitura e a validação passarem**. Arquivo
//! inválido mantém o mapa antigo em uso — o operador não perde o roteamento por um erro de
//! digitação. Arquivo ausente volta para o modo instância única (é o que o operador pediu ao
//! recarregar).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use poph_types::Secret;
use poph_types::config::{ClusterConfig, ClusterInstance};
use serde::Serialize;
use thiserror::Error;

use crate::estado::Estado;

/// Variável de ambiente com o segredo compartilhado entre as instâncias.
pub const VAR_SEGREDO: &str = "POPH_CLUSTER_SECRET";

/// Erro de leitura/validação do `cluster.toml`. Diz o arquivo e, quando possível, a chave.
#[derive(Debug, Error)]
pub enum ErroCluster {
    /// Arquivo existe mas não pode ser lido.
    #[error("não consegui ler o arquivo do cluster {}: {motivo}", .arquivo.display())]
    Leitura { arquivo: PathBuf, motivo: String },
    /// Arquivo com erro de sintaxe TOML (linha e coluna na mensagem).
    #[error("{}: erro de sintaxe no TOML: {motivo}", .arquivo.display())]
    Sintaxe { arquivo: PathBuf, motivo: String },
    /// Campo desconhecido ou com valor que não desserializa.
    #[error("{}: chave `{chave}` inválida: {motivo}", .arquivo.display())]
    Chave {
        arquivo: PathBuf,
        chave: String,
        motivo: String,
    },
    /// Mapa que não passa nas regras (uma linha por problema).
    #[error("{}:\n{}", .arquivo.display(), .problemas.join("\n"))]
    Validacao {
        arquivo: PathBuf,
        problemas: Vec<String>,
    },
    /// Cluster declarado sem o segredo compartilhado no ambiente.
    #[error(
        "{}: o cluster declara instâncias, mas falta o segredo compartilhado; defina {VAR_SEGREDO} \
         no ambiente (o segredo nunca vai no arquivo)",
        .arquivo.display()
    )]
    SegredoAusente { arquivo: PathBuf },
}

impl ErroCluster {
    /// Lista de problemas pronta para o `details` de uma resposta de erro.
    pub fn problemas(&self) -> Vec<String> {
        match self {
            Self::Leitura { motivo, .. } | Self::Sintaxe { motivo, .. } => vec![motivo.clone()],
            Self::Chave { chave, motivo, .. } => vec![format!("{chave}: {motivo}")],
            Self::Validacao { problemas, .. } => problemas.clone(),
            Self::SegredoAusente { .. } => vec![format!(
                "defina {VAR_SEGREDO} no ambiente: o cluster declara instâncias e o segredo não \
                 vai no arquivo"
            )],
        }
    }
}

/// Estado do cluster no processo: o caminho candidato, o mapa em uso e o segredo da subida.
///
/// O caminho é guardado mesmo quando o arquivo não existe, para a recarga poder encontrá-lo depois.
#[derive(Debug, Clone)]
pub struct ClusterVivo {
    /// Caminho candidato do `cluster.toml` (`POPH_CLUSTER` ou o padrão ao lado do `poph.toml`).
    pub caminho: PathBuf,
    /// Mapa em uso (vazio = modo instância única).
    pub config: ClusterConfig,
    /// Segredo compartilhado lido de `POPH_CLUSTER_SECRET` na subida (nunca serializado).
    pub segredo: Option<Secret>,
}

impl ClusterVivo {
    /// Modo instância única, sem arquivo lido: é o padrão de uma instalação sem cluster.
    pub fn instancia_unica(caminho: PathBuf) -> Self {
        Self {
            caminho,
            config: ClusterConfig::default(),
            segredo: None,
        }
    }

    /// `true` quando o mapa declara um cluster.
    pub fn declara_cluster(&self) -> bool {
        self.config.declares_cluster()
    }

    /// `"cluster"` ou `"instancia_unica"`.
    pub fn modo(&self) -> &'static str {
        if self.declara_cluster() {
            "cluster"
        } else {
            "instancia_unica"
        }
    }

    /// Impressão digital do mapa em uso (inclui a instância local).
    pub fn fingerprint(&self) -> String {
        self.config.fingerprint()
    }

    /// Impressão digital do conteúdo compartilhado do mapa (sem a instância local): é a que vai no
    /// cabeçalho `x-poph-mapa` do encaminhamento e a que duas instâncias comparam entre si.
    pub fn fingerprint_do_mapa(&self) -> String {
        self.config.map_fingerprint()
    }

    /// Resumo sem segredo, para `poph cluster show` e para a resposta da recarga.
    pub fn resumo(&self) -> ResumoCluster {
        ResumoCluster::de(self)
    }
}

/// Leitura do `cluster.toml`. Arquivo ausente **não é erro**: é o modo instância única.
pub fn carregar(caminho: &Path, segredo: Option<&Secret>) -> Result<ClusterConfig, ErroCluster> {
    if !caminho.exists() {
        return Ok(ClusterConfig::default());
    }
    let texto = fs::read_to_string(caminho).map_err(|erro| ErroCluster::Leitura {
        arquivo: caminho.to_path_buf(),
        motivo: erro.to_string(),
    })?;
    let tabela: toml::Table = toml::from_str(&texto).map_err(|erro| ErroCluster::Sintaxe {
        arquivo: caminho.to_path_buf(),
        motivo: erro.to_string(),
    })?;
    // Arquivo vazio (só comentários, por exemplo) é o modo instância única. Arquivo com qualquer
    // chave precisa se identificar: um `cluster.toml` sem `instance_id` é configuração quebrada,
    // não um convite para a instância achar que é a dona de todas as orgs.
    let arquivo_vazio = tabela.is_empty();
    let config: ClusterConfig =
        toml::Value::Table(tabela)
            .try_into()
            .map_err(|erro: toml::de::Error| {
                let mensagem = erro.to_string();
                ErroCluster::Chave {
                    arquivo: caminho.to_path_buf(),
                    chave: chave_do_erro_do_serde(&mensagem),
                    motivo: mensagem,
                }
            })?;
    let problemas = if arquivo_vazio {
        Vec::new()
    } else if config.empty() {
        vec![
            "instance_id: obrigatório quando existe cluster.toml (é o nome desta instância no              mapa)"
                .to_owned(),
        ]
    } else {
        config.problems()
    };
    if !problemas.is_empty() {
        return Err(ErroCluster::Validacao {
            arquivo: caminho.to_path_buf(),
            problemas,
        });
    }
    if config.declares_cluster() && segredo.is_none() {
        return Err(ErroCluster::SegredoAusente {
            arquivo: caminho.to_path_buf(),
        });
    }
    Ok(config)
}

/// Extrai o nome do campo do erro do serde/toml (`unknown field \`x\``).
fn chave_do_erro_do_serde(mensagem: &str) -> String {
    if let Some(resto) = mensagem.split("unknown field `").nth(1)
        && let Some((chave, _)) = resto.split_once('`')
    {
        return chave.to_owned();
    }
    if let Some(resto) = mensagem.split("missing field `").nth(1)
        && let Some((chave, _)) = resto.split_once('`')
    {
        return chave.to_owned();
    }
    "(arquivo)".to_owned()
}

/// Resumo do mapa sem segredo: é o que sai na CLI e na resposta da recarga.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResumoCluster {
    /// `cluster` ou `instancia_unica`.
    pub modo: &'static str,
    /// Nome desta instância no mapa (vazio no modo instância única sem `instance_id`).
    pub instance_id: String,
    /// Instância padrão declarada (a implícita de um cluster de uma instância só não aparece).
    pub default_instance: Option<String>,
    /// `true` quando o segredo compartilhado está configurado (o valor nunca sai).
    pub segredo_configurado: bool,
    /// Instâncias declaradas, em ordem de id.
    pub instancias: Vec<ResumoInstancia>,
    /// Mapa org → instância, em ordem de chave.
    pub orgs: BTreeMap<String, String>,
}

/// Uma instância no resumo: id, endereço e se é a local.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResumoInstancia {
    /// Identificador da instância.
    pub id: String,
    /// Endereço base.
    pub url: String,
    /// `true` na instância em que este processo roda.
    pub local: bool,
}

impl ResumoCluster {
    fn de(vivo: &ClusterVivo) -> Self {
        let local = vivo.config.instance_id.trim();
        let mut instancias: Vec<ResumoInstancia> = vivo
            .config
            .instances
            .iter()
            .map(|ClusterInstance { id, url }| ResumoInstancia {
                id: id.trim().to_owned(),
                url: url.trim().to_owned(),
                local: !local.is_empty() && id.trim() == local,
            })
            .collect();
        instancias.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            modo: vivo.modo(),
            instance_id: local.to_owned(),
            default_instance: vivo
                .config
                .default_instance
                .as_deref()
                .map(str::trim)
                .filter(|valor| !valor.is_empty())
                .map(str::to_owned),
            segredo_configurado: vivo.segredo.is_some(),
            instancias,
            orgs: vivo
                .config
                .orgs
                .iter()
                .map(|(org, destino)| (org.trim().to_owned(), destino.trim().to_owned()))
                .collect(),
        }
    }
}

/// Uma org que mudou de instância na recarga.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrocaDeOrg {
    /// Org que trocou de dona.
    pub org: String,
    /// Instância anterior (`null` quando não tinha destino antes).
    pub de: Option<String>,
    /// Instância nova (`null` quando deixou de ter destino nomeado).
    pub para: Option<String>,
}

/// Resultado da recarga: o antes, o depois e o que mudou.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatoRecarga {
    /// `true` quando a impressão digital mudou.
    pub mudou: bool,
    /// Impressão digital do mapa que estava em uso.
    pub fingerprint_antes: String,
    /// Impressão digital do mapa que passou a valer.
    pub fingerprint_depois: String,
    /// Resumo do mapa anterior.
    pub antes: ResumoCluster,
    /// Resumo do mapa novo.
    pub depois: ResumoCluster,
    /// Orgs cujo destino efetivo mudou (inclui as que saíram ou entraram no mapa).
    pub orgs_que_trocaram: Vec<TrocaDeOrg>,
    /// Instâncias que passaram a existir.
    pub instancias_adicionadas: Vec<String>,
    /// Instâncias que deixaram de existir.
    pub instancias_removidas: Vec<String>,
}

impl RelatoRecarga {
    fn entre(antes: &ClusterVivo, depois: &ClusterVivo) -> Self {
        let fingerprint_antes = antes.fingerprint();
        let fingerprint_depois = depois.fingerprint();
        let ids_antes: Vec<&str> = antes
            .config
            .instances
            .iter()
            .map(|instancia| instancia.id.trim())
            .collect();
        let ids_depois: Vec<&str> = depois
            .config
            .instances
            .iter()
            .map(|instancia| instancia.id.trim())
            .collect();
        let instancias_adicionadas = ids_depois
            .iter()
            .filter(|id| !ids_antes.contains(id))
            .map(|id| (*id).to_owned())
            .collect();
        let instancias_removidas = ids_antes
            .iter()
            .filter(|id| !ids_depois.contains(id))
            .map(|id| (*id).to_owned())
            .collect();
        // O destino efetivo de uma org considera o mapa, o padrão e a instância única: uma org
        // que não estava em nenhum dos dois mapas pode passar a ser atendida pela padrão.
        let mut orgs: Vec<String> = antes
            .config
            .orgs
            .keys()
            .chain(depois.config.orgs.keys())
            .cloned()
            .collect();
        orgs.sort();
        orgs.dedup();
        let orgs_que_trocaram = orgs
            .into_iter()
            .filter_map(|org| {
                let de = antes.config.org_instance(&org).map(str::to_owned);
                let para = depois.config.org_instance(&org).map(str::to_owned);
                (de != para).then_some(TrocaDeOrg { org, de, para })
            })
            .collect();
        Self {
            mudou: fingerprint_antes != fingerprint_depois,
            fingerprint_antes,
            fingerprint_depois,
            antes: antes.resumo(),
            depois: depois.resumo(),
            orgs_que_trocaram,
            instancias_adicionadas,
            instancias_removidas,
        }
    }
}

/// Relê o arquivo do cluster e troca o mapa em uso. Arquivo inválido mantém o mapa antigo.
pub fn recarregar(estado: &Estado) -> Result<RelatoRecarga, ErroCluster> {
    let anterior = estado.cluster();
    let config = carregar(&anterior.caminho, anterior.segredo.as_ref())?;
    let novo = ClusterVivo {
        caminho: anterior.caminho.clone(),
        config,
        segredo: anterior.segredo.clone(),
    };
    let relato = RelatoRecarga::entre(&anterior, &novo);
    estado.trocar_cluster(novo);
    Ok(relato)
}

// ───────────────────── conferência remota do mapa (`poph cluster check --remoto`, HR-50b-2) ──────

/// Prazo de cada consulta ao `/healthz` de uma instância no `poph cluster check --remoto`.
///
/// Curto de propósito: o comando é diagnóstico e não pode ficar pendurado numa instância fora do
/// ar. As consultas saem em paralelo, então o pior caso é este prazo, não a soma.
pub const PRAZO_DA_CONSULTA_REMOTA: Duration = Duration::from_secs(3);

/// Resultado da conferência remota de uma instância.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticoRemoto {
    /// Id da instância como está no mapa.
    pub instancia: String,
    /// Endereço base declarado no mapa.
    pub url: String,
    /// `ok`, `inacessivel`, `mapa_divergente` ou `identidade_divergente`.
    pub situacao: &'static str,
    /// Frase de operação quando há problema (`null` quando está tudo certo).
    pub mensagem: Option<String>,
    /// Impressão digital do mapa que a instância declarou (quando ela respondeu).
    pub fingerprint_mapa: Option<String>,
    /// Nome que a instância declarou para si mesma (quando respondeu).
    pub instancia_declarada: Option<String>,
}

impl DiagnosticoRemoto {
    /// `true` quando a instância respondeu com o mesmo mapa e o mesmo nome.
    pub fn ok(&self) -> bool {
        self.situacao == "ok"
    }

    /// Uma linha para o relatório em texto.
    pub fn linha(&self) -> String {
        let detalhe = match self.situacao {
            "ok" => format!(
                "ok (mapa {})",
                self.fingerprint_mapa.as_deref().unwrap_or("não informado")
            ),
            "inacessivel" => format!(
                "inacessível — {}",
                self.mensagem.as_deref().unwrap_or("sem resposta")
            ),
            "mapa_divergente" => format!(
                "mapa divergente — ela usa {}",
                self.fingerprint_mapa
                    .as_deref()
                    .unwrap_or("(não informado)")
            ),
            _ => format!(
                "identidade divergente — declarou `{}`, o mapa diz `{}`",
                self.instancia_declarada.as_deref().unwrap_or("(nada)"),
                self.instancia
            ),
        };
        format!("  {} ({}): {detalhe}", self.instancia, self.url)
    }
}

/// Consulta o `/healthz` de cada instância declarada no mapa (menos a local) e compara o mapa.
///
/// Não muda nada no processo: é leitura. A instância local fica de fora porque o mapa em uso é
/// ela mesma — quem quiser conferir o arquivo usa o `poph cluster check` sem `--remoto`. O segredo
/// compartilhado não é usado nem enviado: `/healthz` é público na rede interna.
pub async fn conferir_remoto(config: &ClusterConfig) -> Vec<DiagnosticoRemoto> {
    let local = config.instance_id.trim().to_owned();
    let local_fingerprint = config.map_fingerprint();
    let cliente = match reqwest::Client::builder()
        .connect_timeout(PRAZO_DA_CONSULTA_REMOTA)
        .timeout(PRAZO_DA_CONSULTA_REMOTA)
        .build()
    {
        Ok(cliente) => cliente,
        Err(erro) => {
            return config
                .instances
                .iter()
                .filter(|instancia| instancia.id.trim() != local)
                .map(|instancia| DiagnosticoRemoto {
                    instancia: instancia.id.trim().to_owned(),
                    url: instancia.url.trim().to_owned(),
                    situacao: "inacessivel",
                    mensagem: Some(format!("não consegui montar o cliente HTTP: {erro}")),
                    fingerprint_mapa: None,
                    instancia_declarada: None,
                })
                .collect();
        }
    };
    let consultas = config
        .instances
        .iter()
        .filter(|instancia| instancia.id.trim() != local)
        .map(|instancia| {
            let cliente = cliente.clone();
            let id = instancia.id.trim().to_owned();
            let url = instancia.url.trim().to_owned();
            let local_fingerprint = local_fingerprint.clone();
            async move {
                let endereco = format!("{}/healthz", url.trim_end_matches('/'));
                let resposta = cliente.get(&endereco).send().await;
                let corpo = match resposta {
                    Ok(resposta) if resposta.status().is_success() => {
                        // Texto + `serde_json`: o cliente desta crate não liga a feature `json`.
                        resposta
                            .text()
                            .await
                            .ok()
                            .and_then(|texto| serde_json::from_str::<serde_json::Value>(&texto).ok())
                    }
                    Ok(resposta) => {
                        return DiagnosticoRemoto {
                            instancia: id,
                            url,
                            situacao: "inacessivel",
                            mensagem: Some(format!(
                                "o /healthz respondeu {}",
                                resposta.status().as_u16()
                            )),
                            fingerprint_mapa: None,
                            instancia_declarada: None,
                        };
                    }
                    Err(erro) => {
                        return DiagnosticoRemoto {
                            instancia: id,
                            url,
                            situacao: "inacessivel",
                            mensagem: Some(erro.to_string()),
                            fingerprint_mapa: None,
                            instancia_declarada: None,
                        };
                    }
                };
                let remoto = corpo
                    .as_ref()
                    .and_then(|corpo| corpo.get("cluster_map_fingerprint"))
                    .and_then(|valor| valor.as_str())
                    .map(str::to_owned);
                let declarada = corpo
                    .as_ref()
                    .and_then(|corpo| corpo.get("instance_id"))
                    .and_then(|valor| valor.as_str())
                    .map(str::to_owned);
                let Some(remoto) = remoto else {
                    return DiagnosticoRemoto {
                        instancia: id,
                        url,
                        situacao: "inacessivel",
                        mensagem: Some(
                            "a instância respondeu sem dizer o mapa (versão anterior a HR-50b-2?)"
                                .to_owned(),
                        ),
                        fingerprint_mapa: None,
                        instancia_declarada: declarada,
                    };
                };
                let situacao = if remoto != local_fingerprint {
                    "mapa_divergente"
                } else if declarada.as_deref() != Some(id.as_str()) {
                    "identidade_divergente"
                } else {
                    "ok"
                };
                let mensagem = match situacao {
                    "mapa_divergente" => Some(format!(
                        "o mapa desta instância é {local_fingerprint} e o dela é {remoto}; atualize \
                         o cluster.toml dos dois lados e recarregue"
                    )),
                    "identidade_divergente" => Some(format!(
                        "a instância declarou o nome `{}`, mas o mapa desta instância a chama de \
                         `{id}`; corrija o `instance_id` dela",
                        declarada.as_deref().unwrap_or("(nada)")
                    )),
                    _ => None,
                };
                DiagnosticoRemoto {
                    instancia: id,
                    url,
                    situacao,
                    mensagem,
                    fingerprint_mapa: Some(remoto),
                    instancia_declarada: declarada,
                }
            }
        })
        .collect::<Vec<_>>();
    futures::future::join_all(consultas).await
}

#[cfg(test)]
mod testes {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn pasta() -> PathBuf {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-cluster-teste-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta de teste");
        pasta
    }

    fn arquivo(texto: &str) -> PathBuf {
        let caminho = pasta().join("cluster.toml");
        fs::write(&caminho, texto).expect("escreve cluster.toml");
        caminho
    }

    fn segredo() -> Secret {
        Secret::new("segredo-compartilhado-de-teste".to_owned())
    }

    const BOM: &str = r#"
instance_id = "a"
default_instance = "b"

[[instances]]
id = "a"
url = "http://127.0.0.1:7480"

[[instances]]
id = "b"
url = "https://harness-b.exemplo"

[orgs]
org_01 = "a"
org_02 = "b"
"#;

    #[test]
    fn arquivo_ausente_e_modo_instancia_unica() {
        let caminho = pasta().join("nao-existe.toml");
        let config = carregar(&caminho, None).expect("sem arquivo não é erro");
        assert!(config.empty());
        assert!(!config.declares_cluster());
    }

    #[test]
    fn arquivo_bom_carrega_com_o_mapa() {
        let caminho = arquivo(BOM);
        let config = carregar(&caminho, Some(&segredo())).expect("carrega");
        assert_eq!(config.instance_id, "a");
        assert_eq!(config.default_instance.as_deref(), Some("b"));
        assert_eq!(config.instances.len(), 2);
        assert_eq!(config.org_instance("org_01"), Some("a"));
        assert_eq!(config.org_instance("org_02"), Some("b"));
        assert_eq!(config.org_instance("org_nova"), Some("b"));
    }

    #[test]
    fn toml_invalido_diz_o_arquivo() {
        let caminho = arquivo("[orgs\norg_01 = \"a\"\n");
        match carregar(&caminho, Some(&segredo())) {
            Err(ErroCluster::Sintaxe { arquivo, motivo }) => {
                assert_eq!(arquivo, caminho);
                assert!(!motivo.is_empty());
            }
            outro => panic!("esperado erro de sintaxe, veio {outro:?}"),
        }
    }

    #[test]
    fn campo_desconhecido_diz_a_chave() {
        let caminho = arquivo("instance_id = \"a\"\ninstancias = []\n");
        match carregar(&caminho, Some(&segredo())) {
            Err(ErroCluster::Chave { chave, .. }) => assert_eq!(chave, "instancias"),
            outro => panic!("esperado erro de chave, veio {outro:?}"),
        }
    }

    #[test]
    fn cada_regra_violada_vira_um_problema_com_a_chave() {
        let mut casos: Vec<(String, &str)> = vec![
            ("instance_id = \"\"\n".to_owned(), "instance_id"),
            (
                "instance_id = \"a\"\n[[instances]]\nid = \"a\"\nurl = \"ftp://x\"\n".to_owned(),
                "instances[1].url",
            ),
            (
                "instance_id = \"a\"\ndefault_instance = \"c\"\n[[instances]]\nid = \"a\"\nurl = \"http://x\"\n"
                    .to_owned(),
                "default_instance",
            ),
            (
                "instance_id = \"a\"\n[[instances]]\nid = \"a\"\nurl = \"http://x\"\n[orgs]\norg_01 = \"c\"\n"
                    .to_owned(),
                "orgs.org_01",
            ),
            (
                "instance_id = \"a\"\n[[instances]]\nid = \"b\"\nurl = \"http://y\"\n[[instances]]\nid = \"b\"\nurl = \"http://z\"\n"
                    .to_owned(),
                "aparece duas vezes",
            ),
        ];
        for (texto, esperado) in casos.drain(..) {
            let caminho = arquivo(&texto);
            match carregar(&caminho, Some(&segredo())) {
                Err(ErroCluster::Validacao { problemas, .. }) => {
                    let juntos = problemas.join("\n");
                    assert!(
                        juntos.contains(esperado),
                        "esperado `{esperado}` nos problemas: {juntos}"
                    );
                }
                outro => panic!("esperado erro de validação para {esperado}, veio {outro:?}"),
            }
        }
    }

    #[test]
    fn cluster_sem_o_segredo_no_ambiente_e_recusado() {
        let caminho = arquivo(BOM);
        match carregar(&caminho, None) {
            Err(ErroCluster::SegredoAusente { .. }) => {}
            outro => panic!("esperado erro de segredo ausente, veio {outro:?}"),
        }
        // Com segredo, carrega.
        assert!(carregar(&caminho, Some(&segredo())).is_ok());
    }

    #[test]
    fn resumo_nao_traz_o_valor_do_segredo() {
        let caminho = arquivo(BOM);
        let config = carregar(&caminho, Some(&segredo())).expect("carrega");
        let vivo = ClusterVivo {
            caminho: caminho.clone(),
            config,
            segredo: Some(segredo()),
        };
        let texto = serde_json::to_string(&vivo.resumo()).expect("json do resumo");
        assert!(texto.contains("\"segredo_configurado\":true"));
        assert!(!texto.contains("segredo-compartilhado-de-teste"));
        assert!(!format!("{vivo:?}").contains("segredo-compartilhado-de-teste"));
    }
}

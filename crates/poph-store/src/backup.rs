//! Exportação e importação de uma org em arquivo único (HR-53).
//!
//! O retrato (`RetratoOrg`) é a org inteira como linhas de banco: a configuração da org, o
//! catálogo, os departamentos, os agentes com instruções e vínculos, a base de conhecimento, a
//! biblioteca de arquivos e as integrações — e, com `com_conversas`, o histórico (conversas,
//! mensagens, anexos, turnos e as duas filas) e os carrinhos.
//!
//! Regras que valem aqui:
//! - **ids preservados**: o retrato leva as linhas como estão, então o material importado é o
//!   mesmo (mesmos ids, mesmas versões, mesmos carimbos). É o que mantém a conversa ligada ao
//!   agente e a mensagem ligada ao turno;
//! - **escopo por org**: toda leitura e toda escrita passa por `org_id`; nada de outra org entra
//!   ou sai;
//! - **segredo nunca sai**: `integrations.secrets_enc` é lido só para o chamador decifrar os
//!   *nomes* dos campos; a marca `tem_segredo` diz que havia segredo, e o blob vai em
//!   `RetratoOrg::segredos_cifrados`, que nunca é serializado;
//! - **sem caminho de blob**: `blob_path` sai do retrato e é recalculado na importação
//!   (`<org de destino>/<sha256>`), porque o caminho depende da org dona;
//! - **importação idempotente e atômica**: cada linha é localizada pela chave da tabela
//!   (`id`, ou as colunas da chave primária composta) e atualizada; o que não está no arquivo
//!   **não é apagado**. Tudo roda num savepoint do escritor: ou o retrato inteiro entra, ou nada;
//! - **atualização de linha existente é decisão de quem chama**: [`Store::importar_retrato`] grava
//!   o retrato como ele é; quem precisa proteger o que já existe no destino confere antes com
//!   [`Store::conferir_atualizacoes_importacao`] (a CLI recusa sem `--sobrescrever`, a API sem
//!   `?sobrescrever=true`);
//! - **conflito de chave natural é recusado**: se o destino já tem um agente/produto/categoria/
//!   departamento/grupo com o mesmo slug ou nome e id diferente, a importação para antes de
//!   gravar, com a lista do que conflitou. Nada de "merge" silencioso.
//!
//! O que **não** entra: tokens de administração (credencial), `audit_log` (histórico de
//! infraestrutura), `secrets_enc` (segredo) e o blob em si (o chamador lê e grava pelo
//! `poph-files`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use poph_types::{DomainError, OrgId};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde_json::{Map, Value};

use crate::Store;
use crate::erro::{ErroStore, Resultado};

/// Nome do formato gravado no arquivo (confere na importação).
pub const FORMATO_BACKUP: &str = "poph-backup-org";
/// Versão do formato que esta versão do POP Harness escreve e entende.
pub const VERSAO_FORMATO: u32 = 2;

/// Tabela do retrato: nome e colunas que identificam a linha dentro da org.
struct Tabela {
    nome: &'static str,
    chave: &'static [&'static str],
}

/// Tabelas que vão em todo backup, na ordem em que precisam ser gravadas (pai antes do filho).
const TABELAS_SEMPRE: &[Tabela] = &[
    Tabela {
        nome: "product_categories",
        chave: &["id"],
    },
    Tabela {
        nome: "departments",
        chave: &["id"],
    },
    Tabela {
        nome: "products",
        chave: &["id"],
    },
    Tabela {
        nome: "agents",
        chave: &["id"],
    },
    Tabela {
        nome: "agent_product_categories",
        chave: &["agent_id", "category_id"],
    },
    Tabela {
        nome: "agent_instructions",
        chave: &["agent_id"],
    },
    Tabela {
        nome: "knowledge_groups",
        chave: &["id"],
    },
    Tabela {
        nome: "knowledge_items",
        chave: &["id"],
    },
    Tabela {
        nome: "agent_knowledge_groups",
        chave: &["agent_id", "group_id"],
    },
    Tabela {
        nome: "file_library",
        chave: &["id"],
    },
    Tabela {
        nome: "integrations",
        chave: &["id"],
    },
    Tabela {
        nome: "fluxos",
        chave: &["id"],
    },
    Tabela {
        nome: "fluxo_revisoes",
        chave: &["id"],
    },
    Tabela {
        nome: "agente_fluxos",
        chave: &["agent_id", "fluxo_id"],
    },
];

/// Tabelas do histórico, só com `--com-conversas`.
const TABELAS_CONVERSAS: &[Tabela] = &[
    Tabela {
        nome: "conversations",
        chave: &["id"],
    },
    Tabela {
        nome: "messages",
        chave: &["id"],
    },
    Tabela {
        nome: "attachments",
        chave: &["id"],
    },
    Tabela {
        nome: "turns",
        chave: &["id"],
    },
    Tabela {
        nome: "outbox",
        chave: &["id"],
    },
    Tabela {
        nome: "inbox",
        chave: &["id"],
    },
    Tabela {
        nome: "carts",
        chave: &["id"],
    },
    Tabela {
        nome: "cart_items",
        chave: &["id"],
    },
    Tabela {
        nome: "fichas",
        chave: &["id"],
    },
    Tabela {
        nome: "ficha_eventos",
        chave: &["id"],
    },
    Tabela {
        nome: "fluxo_execucoes",
        chave: &["id"],
    },
    Tabela {
        nome: "fluxo_execucao_etapas",
        chave: &["id"],
    },
    Tabela {
        nome: "fluxo_efeitos",
        chave: &["id"],
    },
];

/// Chaves naturais (única por org) que a importação confere **antes** de gravar.
///
/// Sem esta conferência, o `INSERT` falharia com "já existe um registro com esse valor nesta org"
/// sem dizer qual recurso era, e só depois de tentar. Aqui o operador recebe o nome do recurso e
/// do valor, e nada é gravado.
struct ChaveNatural {
    tabela: &'static str,
    /// Coluna no banco, comparada sem diferenciar maiúsculas.
    coluna: &'static str,
    /// Nome do recurso para a mensagem.
    recurso: &'static str,
}

const CHAVES_NATURAIS: &[ChaveNatural] = &[
    ChaveNatural {
        tabela: "fluxos",
        coluna: "slug",
        recurso: "fluxo",
    },
    ChaveNatural {
        tabela: "agents",
        coluna: "slug",
        recurso: "agente",
    },
    ChaveNatural {
        tabela: "products",
        coluna: "slug",
        recurso: "produto",
    },
    ChaveNatural {
        tabela: "product_categories",
        coluna: "name",
        recurso: "categoria",
    },
    ChaveNatural {
        tabela: "departments",
        coluna: "name",
        recurso: "departamento",
    },
    ChaveNatural {
        tabela: "knowledge_groups",
        coluna: "name",
        recurso: "grupo de conhecimento",
    },
];

/// Uma org inteira, como linhas de banco.
#[derive(Clone)]
pub struct RetratoOrg {
    /// A linha da tabela `orgs`.
    pub org: Value,
    /// Tabela → linhas, na ordem de gravação.
    pub tabelas: BTreeMap<String, Vec<Value>>,
    /// Blobs cifrados dos segredos de integração (id da integração → `secrets_enc`).
    ///
    /// Serve só para o chamador decifrar os **nomes** dos campos preenchidos. Nunca é
    /// serializado nem gravado em arquivo.
    pub segredos_cifrados: BTreeMap<String, Vec<u8>>,
}

impl fmt::Debug for RetratoOrg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Nunca imprime o conteúdo cifrado dos segredos.
        f.debug_struct("RetratoOrg")
            .field("org", &self.org)
            .field("tabelas", &self.tabelas)
            .field(
                "segredos_cifrados",
                &format_args!(
                    "{} integração(ões) com segredo",
                    self.segredos_cifrados.len()
                ),
            )
            .finish()
    }
}

/// Para onde a importação escreve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinoImportacao {
    /// A org do arquivo: usa o id de origem e cria a org se ela não existir.
    DaOrigem,
    /// Uma org já existente (a linha da org não é tocada).
    Existente(OrgId),
}

/// Quantas linhas de uma tabela entraram (novas) e quantas foram atualizadas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TabelaGravada {
    /// Linhas criadas agora.
    pub inseridas: u64,
    /// Linhas que já existiam e foram atualizadas com o valor do arquivo.
    pub atualizadas: u64,
}

/// O que a importação fez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatorioImportacao {
    /// Org que recebeu o material.
    pub org: OrgId,
    /// `true` quando a org nasceu nesta importação.
    pub org_criada: bool,
    /// Tabela → linhas inseridas/atualizadas.
    pub tabelas: BTreeMap<String, TabelaGravada>,
    /// Integrações que vieram com segredo no arquivo e precisam ser digitadas de novo.
    pub segredos_a_redigitar: u64,
}

impl RelatorioImportacao {
    /// Total de linhas gravadas (novas + atualizadas), sem contar a org.
    pub fn total(&self) -> u64 {
        self.tabelas
            .values()
            .map(|t| t.inseridas + t.atualizadas)
            .sum()
    }
}

/// Até quantos exemplos de linha a conferência de atualização devolve.
pub const MAX_EXEMPLOS_ATUALIZACAO: usize = 5;

/// O que a importação **atualizaria** no destino: linhas do arquivo que já existem lá, localizadas
/// pela chave da tabela.
///
/// É só leitura — a conferência não grava nada. Serve a quem precisa recusar a importação antes de
/// sobrescrever o estado que já está no destino (CLI sem `--sobrescrever`, API sem
/// `?sobrescrever=true`). Importar num destino limpo (ou numa org nova) devolve
/// `AtualizacoesImportacao::default()`, ou seja, nada a sobrescrever.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtualizacoesImportacao {
    /// Tabela → quantas linhas seriam atualizadas (só as tabelas com pelo menos uma).
    pub tabelas: BTreeMap<String, u64>,
    /// Total de linhas que seriam atualizadas (soma de `tabelas`).
    pub total: u64,
    /// Até [`MAX_EXEMPLOS_ATUALIZACAO`] exemplos legíveis: `tabela: slug`, `tabela: nome` ou
    /// `tabela: id`.
    pub exemplos: Vec<String>,
}

impl AtualizacoesImportacao {
    /// `true` quando nada seria atualizado (a importação só insere).
    pub fn vazia(&self) -> bool {
        self.total == 0
    }

    /// Resumo curto por tabela, para a mensagem de recusa: `agents 1, products 2`.
    pub fn resumo(&self) -> String {
        self.tabelas
            .iter()
            .map(|(tabela, linhas)| format!("{tabela} {linhas}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn registrar(&mut self, tabela: &str, linha: &Value) {
        *self.tabelas.entry(tabela.to_owned()).or_insert(0) += 1;
        self.total += 1;
        if self.exemplos.len() < MAX_EXEMPLOS_ATUALIZACAO {
            self.exemplos
                .push(format!("{tabela}: {}", identificador_da_linha(linha)));
        }
    }
}

impl Store {
    /// Lê a org inteira. `com_conversas` inclui o histórico e os carrinhos.
    ///
    /// A leitura inteira acontece numa só conexão do pool: todas as tabelas saem do mesmo
    /// instante (o servidor deve estar parado, mas o retrato não fica rasgado se não estiver).
    pub fn retrato_da_org(&self, org: OrgId, com_conversas: bool) -> Resultado<RetratoOrg> {
        let tabelas: Vec<&'static Tabela> = TABELAS_SEMPRE
            .iter()
            .chain(TABELAS_CONVERSAS.iter().filter(|_| com_conversas))
            .collect();
        self.leitura(move |conn| {
            let linha_org = ler_org(conn, org)?;
            let mut retrato = BTreeMap::new();
            for tabela in tabelas {
                retrato.insert(tabela.nome.to_owned(), ler_tabela(conn, tabela.nome, org)?);
            }
            let segredos_cifrados = ler_segredos_cifrados(conn, org)?;
            Ok(RetratoOrg {
                org: linha_org,
                tabelas: retrato,
                segredos_cifrados,
            })
        })
    }

    /// Grava o retrato inteiro, numa só transação (savepoint do escritor).
    ///
    /// Idempotente: reimportar o mesmo retrato atualiza as mesmas linhas, sem duplicar. Não
    /// apaga nada que não esteja no retrato. Uma linha por chave de tabela; conflito de chave
    /// natural com id diferente é recusado antes de qualquer gravação.
    ///
    /// Atualizar linha que já existe é **permitido** aqui: a política de recusar sobrescrita por
    /// padrão é do chamador, que confere antes com
    /// [`Store::conferir_atualizacoes_importacao`].
    pub fn importar_retrato(
        &self,
        retrato: RetratoOrg,
        destino: DestinoImportacao,
    ) -> Resultado<RelatorioImportacao> {
        self.escrita(move |conn| importar(conn, &retrato, destino))
    }

    /// Verifica sobrescritas e importa na mesma transação do escritor.
    pub fn importar_retrato_protegido(
        &self,
        retrato: RetratoOrg,
        destino: DestinoImportacao,
        sobrescrever: bool,
    ) -> Resultado<std::result::Result<RelatorioImportacao, AtualizacoesImportacao>> {
        self.escrita(move |conn| {
            let (org, criada) = destino_da_importacao(conn, &retrato, destino)?;
            conferir_chaves_naturais(conn, &retrato, org, criada)?;
            if !sobrescrever {
                let atualizacoes = atualizacoes_da_importacao(conn, &retrato, destino)?;
                if !atualizacoes.vazia() {
                    return Ok(Err(atualizacoes));
                }
            }
            importar(conn, &retrato, destino).map(Ok)
        })
    }

    /// Lista, **sem gravar nada**, as chaves naturais do retrato que já existem no destino com
    /// outro id (a importação recusaria por causa delas).
    ///
    /// É a mesma conferência que [`Store::importar_retrato`] faz antes de escrever; serve a quem
    /// precisa mostrar os conflitos antes de decidir (a API responde `409` com a lista). Lista
    /// vazia = nada conflita.
    pub fn conferir_conflitos_importacao(
        &self,
        retrato: &RetratoOrg,
        destino: DestinoImportacao,
    ) -> Resultado<Vec<String>> {
        self.leitura(|conn| {
            let (org_id, org_criada) = destino_da_importacao(conn, retrato, destino)?;
            conflitos_das_chaves_naturais(conn, retrato, org_id, org_criada)
        })
    }

    /// Lista, **sem gravar nada**, as linhas do retrato que já existem no destino e que a
    /// importação atualizaria com o valor do arquivo.
    ///
    /// É a conferência que sustenta "recusar sobrescrita por padrão": a CLI recusa sem
    /// `--sobrescrever` e a API sem `?sobrescrever=true`. Lista vazia (`vazia()` verdadeiro) =
    /// destino limpo, a importação só insere. Não confunde com
    /// [`Store::conferir_conflitos_importacao`]: conflito é chave natural repetida **com outro
    /// id** (recusa sempre); aqui é a **mesma linha**, que seria regravada.
    pub fn conferir_atualizacoes_importacao(
        &self,
        retrato: &RetratoOrg,
        destino: DestinoImportacao,
    ) -> Resultado<AtualizacoesImportacao> {
        self.leitura(|conn| atualizacoes_da_importacao(conn, retrato, destino))
    }
}

// ─────────────────────────────── Leitura do retrato ───────────────────────────────

fn ler_org(conn: &Connection, org: OrgId) -> Resultado<Value> {
    let mut consulta = conn.prepare("SELECT * FROM orgs WHERE id = ?1")?;
    let colunas = nomes_das_colunas(&consulta);
    let mut linhas = consulta.query([org.to_string()])?;
    match linhas.next()? {
        Some(linha) => linha_json("orgs", &colunas, linha),
        None => Err(DomainError::NaoEncontrado {
            entidade: "org",
            id: org.to_string(),
        }
        .into()),
    }
}

fn ler_tabela(conn: &Connection, tabela: &str, org: OrgId) -> Resultado<Vec<Value>> {
    // `integrations` ganha a marca de que existe segredo; o blob cifrado fica de fora da linha.
    let sql = if tabela == "integrations" {
        "SELECT *, secrets_enc IS NOT NULL AS tem_segredo FROM integrations \
         WHERE org_id = ?1 ORDER BY rowid"
            .to_owned()
    } else {
        format!("SELECT * FROM {tabela} WHERE org_id = ?1 ORDER BY rowid")
    };
    let mut consulta = conn.prepare(&sql)?;
    let colunas = nomes_das_colunas(&consulta);
    let mut linhas = consulta.query([org.to_string()])?;
    let mut valores = Vec::new();
    while let Some(linha) = linhas.next()? {
        valores.push(linha_json(tabela, &colunas, linha)?);
    }
    Ok(valores)
}

fn ler_segredos_cifrados(conn: &Connection, org: OrgId) -> Resultado<BTreeMap<String, Vec<u8>>> {
    let mut consulta = conn.prepare(
        "SELECT id, secrets_enc FROM integrations \
         WHERE org_id = ?1 AND secrets_enc IS NOT NULL",
    )?;
    let mut linhas = consulta.query([org.to_string()])?;
    let mut mapa = BTreeMap::new();
    while let Some(linha) = linhas.next()? {
        let id: String = linha.get(0)?;
        let blob: Vec<u8> = linha.get(1)?;
        mapa.insert(id, blob);
    }
    Ok(mapa)
}

fn nomes_das_colunas(consulta: &rusqlite::Statement<'_>) -> Vec<String> {
    consulta.column_names().map(str::to_owned).collect()
}

fn linha_json(tabela: &str, colunas: &[String], linha: &rusqlite::Row<'_>) -> Resultado<Value> {
    let mut objeto = Map::new();
    for (indice, coluna) in colunas.iter().enumerate() {
        // O segredo cifrado não entra no arquivo (nem para ser lido): só a marca `tem_segredo`.
        if tabela == "integrations" && coluna == "secrets_enc" {
            continue;
        }
        objeto.insert(
            coluna.clone(),
            valor_json(tabela, coluna, linha.get_ref(indice)?)?,
        );
    }
    // O caminho do blob depende da org de destino: é recalculado na importação.
    if tabela == "file_library" || tabela == "attachments" {
        objeto.remove("blob_path");
    }
    // `tem_segredo` vem do SQLite como 0/1; no arquivo é booleano.
    if tabela == "integrations"
        && let Some(valor) = objeto.get_mut("tem_segredo")
    {
        *valor = Value::Bool(valor.as_i64().is_some_and(|numero| numero != 0));
    }
    Ok(Value::Object(objeto))
}

fn valor_json(tabela: &str, coluna: &str, valor: ValueRef<'_>) -> Resultado<Value> {
    Ok(match valor {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(numero) => Value::from(numero),
        ValueRef::Real(numero) => serde_json::Number::from_f64(numero)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(texto) => {
            let texto = std::str::from_utf8(texto).map_err(|_| {
                ErroStore::Interno(format!(
                    "a coluna `{coluna}` de `{tabela}` não é texto UTF-8"
                ))
            })?;
            Value::String(texto.to_owned())
        }
        ValueRef::Blob(_) => {
            return Err(ErroStore::Interno(format!(
                "a coluna `{coluna}` de `{tabela}` é binária e não deveria ir para o backup"
            )));
        }
    })
}

// ─────────────────────────────── Importação ───────────────────────────────

fn importar(
    conn: &Connection,
    retrato: &RetratoOrg,
    destino: DestinoImportacao,
) -> Resultado<RelatorioImportacao> {
    let (org_id, org_criada) = destino_da_importacao(conn, retrato, destino)?;

    // Antes de gravar qualquer linha: chave natural repetida com id diferente é recusa.
    conferir_chaves_naturais(conn, retrato, org_id, org_criada)?;

    if matches!(destino, DestinoImportacao::DaOrigem) {
        gravar_org(conn, &retrato.org, org_id)?;
    }

    let mut relatorio = RelatorioImportacao {
        org: org_id,
        org_criada,
        tabelas: BTreeMap::new(),
        segredos_a_redigitar: 0,
    };
    // A ordem de gravação é a das listas (pai antes do filho), nunca a ordem alfabética do mapa.
    let ordem: Vec<&'static Tabela> = TABELAS_SEMPRE
        .iter()
        .chain(TABELAS_CONVERSAS.iter())
        .collect();
    for tabela in &ordem {
        let Some(linhas) = retrato.tabelas.get(tabela.nome) else {
            continue;
        };
        let colunas = colunas_da_tabela(conn, tabela.nome)?;
        let mut gravado = TabelaGravada::default();
        for linha in linhas {
            let atualizada = gravar_linha(conn, tabela, org_id, linha, &colunas)?;
            if atualizada {
                gravado.atualizadas += 1;
            } else {
                gravado.inseridas += 1;
            }
        }
        if tabela.nome == "integrations" {
            let com_segredo = linhas
                .iter()
                .filter(|linha| linha.get("tem_segredo") == Some(&Value::Bool(true)))
                .count() as u64;
            relatorio.segredos_a_redigitar += com_segredo;
        }
        relatorio.tabelas.insert(tabela.nome.to_owned(), gravado);
    }
    if !retrato.tabelas.contains_key("fichas") && retrato.tabelas.contains_key("conversations") {
        conn.execute_batch(&format!(r#"INSERT INTO fichas
            (id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em, fechada_em, fechada_por, fechada_motivo, version)
            SELECT lower(substr(printf('%012x', max(c.created_at, 0)), 1, 8) || '-' ||
                substr(printf('%012x', max(c.created_at, 0)), 9, 4) || '-7' ||
                substr(hex(randomblob(2)), 1, 3) || '-' ||
                substr('89ab', 1 + (random() & 3), 1) || substr(hex(randomblob(2)), 1, 3) || '-' || hex(randomblob(6))),
                c.org_id, c.id, c.agent_id, c.channel, c.external_peer,
                CASE WHEN c.status = 'closed' THEN 'closed' ELSE 'open' END, c.created_at,
                CASE WHEN c.status = 'closed' THEN c.updated_at END,
                CASE WHEN c.status = 'closed' THEN 'migracao' END,
                CASE WHEN c.status = 'closed' THEN 'conversa importada de backup anterior' END, 1
            FROM conversations c WHERE c.org_id = '{}' AND NOT EXISTS
                (SELECT 1 FROM fichas f WHERE f.conversation_id = c.id);"#, org_id))?;
    }
    if let Some(conversas) = retrato.tabelas.get("conversations") {
        for conversa in conversas {
            let id = conversa.get("id").and_then(Value::as_str).ok_or_else(|| {
                ErroStore::Dominio(DomainError::Validacao {
                    campo: "backup",
                    motivo: "conversa sem id".to_owned(),
                })
            })?;
            let existe: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM fichas WHERE org_id = ?1 AND conversation_id = ?2)",
                rusqlite::params![org_id.to_string(), id],
                |row| row.get(0),
            )?;
            if !existe {
                return Err(ErroStore::Dominio(DomainError::Validacao {
                    campo: "backup",
                    motivo: "conversa importada sem Ficha correspondente".to_owned(),
                }));
            }
        }
    }
    // Tabela que não faz parte do formato é recusada (arquivo de outra versão ou alterado).
    for nome in retrato.tabelas.keys() {
        if tabela_conhecida(nome).is_none() {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "backup",
                motivo: format!(
                    "o arquivo traz a tabela `{nome}`, que não faz parte do formato desta versão"
                ),
            }));
        }
    }
    Ok(relatorio)
}

fn destino_da_importacao(
    conn: &Connection,
    retrato: &RetratoOrg,
    destino: DestinoImportacao,
) -> Resultado<(OrgId, bool)> {
    let da_origem = org_da_linha(&retrato.org)?;
    match destino {
        DestinoImportacao::Existente(org) => {
            if !org_existe(conn, org)? {
                return Err(DomainError::NaoEncontrado {
                    entidade: "org",
                    id: org.to_string(),
                }
                .into());
            }
            Ok((org, false))
        }
        DestinoImportacao::DaOrigem => {
            let existe = org_existe(conn, da_origem)?;
            Ok((da_origem, !existe))
        }
    }
}

fn org_da_linha(linha: &Value) -> Resultado<OrgId> {
    let id = linha.get("id").and_then(Value::as_str).ok_or_else(|| {
        ErroStore::Dominio(DomainError::Validacao {
            campo: "org",
            motivo: "o arquivo não traz o id da org de origem".to_owned(),
        })
    })?;
    id.parse::<OrgId>().map_err(|_| {
        ErroStore::Dominio(DomainError::Validacao {
            campo: "org",
            motivo: "o id da org no arquivo não é um UUID v7".to_owned(),
        })
    })
}

fn org_existe(conn: &Connection, org: OrgId) -> Resultado<bool> {
    let existe: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM orgs WHERE id = ?1",
            [org.to_string()],
            |linha| linha.get(0),
        )
        .optional()?;
    Ok(existe.is_some())
}

/// Linhas do arquivo que já existem no destino, pela chave da tabela (só leitura).
///
/// A linha da org entra na conta apenas quando o destino é a própria org de origem: o caminho
/// `Existente` não escreve em `orgs` (a org de destino mantém nome e configuração), então não há
/// o que sobrescrever ali.
fn atualizacoes_da_importacao(
    conn: &Connection,
    retrato: &RetratoOrg,
    destino: DestinoImportacao,
) -> Resultado<AtualizacoesImportacao> {
    let (org_id, org_criada) = destino_da_importacao(conn, retrato, destino)?;
    let mut atualizacoes = AtualizacoesImportacao::default();
    if matches!(destino, DestinoImportacao::DaOrigem) && !org_criada {
        atualizacoes.registrar("orgs", &retrato.org);
    }
    for tabela in TABELAS_SEMPRE.iter().chain(TABELAS_CONVERSAS.iter()) {
        let Some(linhas) = retrato.tabelas.get(tabela.nome) else {
            continue;
        };
        for linha in linhas {
            if linha_do_arquivo_ja_existe(conn, tabela, org_id, linha)? {
                atualizacoes.registrar(tabela.nome, linha);
            }
        }
    }
    Ok(atualizacoes)
}

/// Confere se a linha do arquivo já existe no destino, pela chave da tabela.
///
/// Mesma localização que a gravação usa (`filtro_da_chave` + escopo de org), sem escrever. Linha
/// sem alguma coluna da chave não conta: a gravação recusaria com mensagem própria.
fn linha_do_arquivo_ja_existe(
    conn: &Connection,
    tabela: &Tabela,
    org_id: OrgId,
    linha: &Value,
) -> Resultado<bool> {
    let objeto = objeto_da_linha(tabela.nome, linha)?;
    let mut valores = BTreeMap::new();
    for chave in tabela.chave {
        let Some(valor) = objeto.get(*chave) else {
            return Ok(false);
        };
        valores.insert((*chave).to_owned(), valor_sql(tabela.nome, chave, valor)?);
    }
    linha_existe(conn, tabela, &valores, Some(org_id))
}

/// Identificador legível de uma linha para os exemplos: slug, nome (quando houver) ou id.
fn identificador_da_linha(linha: &Value) -> String {
    for campo in ["slug", "name", "id"] {
        if let Some(valor) = linha.get(campo).and_then(Value::as_str) {
            return valor.to_owned();
        }
    }
    "(sem identificador)".to_owned()
}

fn tabela_conhecida(nome: &str) -> Option<&'static Tabela> {
    TABELAS_SEMPRE
        .iter()
        .chain(TABELAS_CONVERSAS.iter())
        .find(|tabela| tabela.nome == nome)
}

/// Conflitos de chave natural que a importação encontraria no destino (sem gravar nada).
///
/// Cada item é legível para o operador: ex. `agente bia (já existe nesta org com outro id)`.
/// Lista vazia = o retrato pode entrar.
fn conflitos_das_chaves_naturais(
    conn: &Connection,
    retrato: &RetratoOrg,
    org_id: OrgId,
    org_criada: bool,
) -> Resultado<Vec<String>> {
    let mut conflitos: Vec<String> = Vec::new();
    // Ids são preservados na importação (é o que mantém conversa↔agente e mensagem↔turno). Se o
    // mesmo id já existe em OUTRA org desta instância, a gravação bateria na chave primária: diz
    // isso uma vez, em vez de deixar o SQLite reclamar do nome de uma tabela qualquer.
    //
    // Também é o caso de "importar o mesmo backup em duas orgs da mesma instância" — que não é
    // suportado de propósito (o formato preserva os ids); importar na MESMA org é idempotente.
    'fora: for tabela in TABELAS_SEMPRE.iter().chain(TABELAS_CONVERSAS.iter()) {
        if tabela.chave != ["id"] {
            continue;
        }
        for linha in retrato.tabelas.get(tabela.nome).into_iter().flatten() {
            let Some(id) = linha.get("id").and_then(Value::as_str) else {
                continue;
            };
            let existe_em_outra: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT org_id FROM {} WHERE id = ?1 AND org_id <> ?2",
                        tabela.nome
                    ),
                    rusqlite::params![id, org_id.to_string()],
                    |linha| linha.get(0),
                )
                .optional()?;
            if existe_em_outra.is_some() {
                conflitos.push(format!(
                    "`{}` {id} (o id do arquivo já existe nesta instância em outra org)",
                    tabela.nome
                ));
                break 'fora;
            }
        }
    }
    for chave in CHAVES_NATURAIS {
        let Some(linhas) = retrato.tabelas.get(chave.tabela) else {
            continue;
        };
        for linha in linhas {
            let (Some(valor), Some(id)) = (
                linha.get(chave.coluna).and_then(Value::as_str),
                linha.get("id").and_then(Value::as_str),
            ) else {
                continue;
            };
            let existente: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT id FROM {} WHERE org_id = ?1 AND lower({}) = lower(?2)",
                        chave.tabela, chave.coluna
                    ),
                    rusqlite::params![org_id.to_string(), valor],
                    |linha| linha.get(0),
                )
                .optional()?;
            if let Some(existente) = existente
                && existente != id
            {
                conflitos.push(format!(
                    "{} `{valor}` (já existe nesta org com outro id)",
                    chave.recurso
                ));
            }
        }
    }
    if org_criada && let Some(slug) = retrato.org.get("slug").and_then(Value::as_str) {
        let existente: Option<String> = conn
            .query_row("SELECT id FROM orgs WHERE slug = ?1", [slug], |linha| {
                linha.get(0)
            })
            .optional()?;
        if existente.is_some() {
            conflitos.push(format!("org `{slug}` (o slug já existe nesta instância)"));
        }
    }
    Ok(conflitos)
}

fn conferir_chaves_naturais(
    conn: &Connection,
    retrato: &RetratoOrg,
    org_id: OrgId,
    org_criada: bool,
) -> Resultado<()> {
    let conflitos = conflitos_das_chaves_naturais(conn, retrato, org_id, org_criada)?;
    if conflitos.is_empty() {
        return Ok(());
    }
    let mostrados = conflitos.iter().take(5).cloned().collect::<Vec<_>>();
    let resto = conflitos.len().saturating_sub(mostrados.len());
    let mut motivo = format!(
        "a importação foi recusada antes de gravar: {}",
        mostrados.join("; ")
    );
    if resto > 0 {
        motivo.push_str(&format!("; e mais {resto}"));
    }
    motivo.push_str(
        ". Nada foi gravado: renomeie um dos dois lados ou importe numa instância sem esses recursos",
    );
    Err(ErroStore::Dominio(DomainError::Validacao {
        campo: "backup",
        motivo,
    }))
}

fn gravar_org(conn: &Connection, linha: &Value, org_id: OrgId) -> Resultado<()> {
    let tabela = Tabela {
        nome: "orgs",
        chave: &["id"],
    };
    let objeto = objeto_da_linha(tabela.nome, linha)?;
    let colunas = colunas_da_tabela(conn, tabela.nome)?;
    gravar_objeto(conn, &tabela, &objeto, Some(org_id), &colunas)?;
    Ok(())
}

fn gravar_linha(
    conn: &Connection,
    tabela: &Tabela,
    org_id: OrgId,
    linha: &Value,
    colunas: &BTreeSet<String>,
) -> Resultado<bool> {
    let mut objeto = objeto_da_linha(tabela.nome, linha)?;
    // Campos informativos do arquivo (não são colunas): a marca de segredo e os nomes dos campos.
    if tabela.nome == "integrations" {
        objeto.remove("tem_segredo");
        objeto.remove("segredos_campos");
    }
    objeto.insert("org_id".to_owned(), Value::String(org_id.to_string()));
    if tabela.nome == "file_library" || tabela.nome == "attachments" {
        let sha = objeto
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ErroStore::Dominio(DomainError::Validacao {
                    campo: "backup",
                    motivo: format!("linha de `{}` sem sha256", tabela.nome),
                })
            })?;
        objeto.insert(
            "blob_path".to_owned(),
            Value::String(format!("{org_id}/{sha}")),
        );
    }
    if tabela.nome == "fichas"
        && let Some(conversa) = objeto.get("conversation_id").and_then(Value::as_str)
    {
        let existe: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE org_id = ?1 AND id = ?2)",
            rusqlite::params![org_id.to_string(), conversa],
            |row| row.get(0),
        )?;
        if !existe {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "backup",
                motivo: "a conversa da Ficha não pertence à organização de destino".to_owned(),
            }));
        }
    }
    if tabela.nome == "fluxo_revisoes" {
        let id = objeto.get("id").and_then(Value::as_str).unwrap_or("");
        let mut consulta =
            conn.prepare("SELECT * FROM fluxo_revisoes WHERE org_id = ?1 AND id = ?2")?;
        let nomes = nomes_das_colunas(&consulta);
        let mut linhas = consulta.query(rusqlite::params![org_id.to_string(), id])?;
        if let Some(linha) = linhas.next()? {
            let existente = linha_json(tabela.nome, &nomes, linha)?;
            if existente == Value::Object(objeto.clone()) {
                return Ok(true);
            }
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "backup",
                motivo: "uma revisão publicada existente é imutável e difere do arquivo".to_owned(),
            }));
        }
    }
    gravar_objeto(conn, tabela, &objeto, Some(org_id), colunas)
}

fn objeto_da_linha(tabela: &str, linha: &Value) -> Resultado<Map<String, Value>> {
    linha.as_object().cloned().ok_or_else(|| {
        ErroStore::Dominio(DomainError::Validacao {
            campo: "backup",
            motivo: format!("uma linha de `{tabela}` no arquivo não é um objeto JSON"),
        })
    })
}

fn gravar_objeto(
    conn: &Connection,
    tabela: &Tabela,
    objeto: &Map<String, Value>,
    escopo: Option<OrgId>,
    colunas: &BTreeSet<String>,
) -> Resultado<bool> {
    // A org entra no filtro de todas as tabelas de negócio (a própria `orgs` usa o id como chave).
    let escopo = match tabela.nome {
        "orgs" => None,
        _ => Some(escopo.ok_or_else(|| {
            ErroStore::Interno(format!("tabela `{}` sem escopo de org", tabela.nome))
        })?),
    };
    let mut valores: BTreeMap<String, SqlValue> = BTreeMap::new();
    for (nome, valor) in objeto {
        if !colunas.contains(nome.as_str()) {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "backup",
                motivo: format!(
                    "o arquivo traz a coluna `{nome}` em `{}`, que não existe nesta versão do \
                     esquema (arquivo de versão mais nova?)",
                    tabela.nome
                ),
            }));
        }
        valores.insert(nome.clone(), valor_sql(tabela.nome, nome, valor)?);
    }
    for chave in tabela.chave {
        if !valores.contains_key(*chave) {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "backup",
                motivo: format!("linha de `{}` sem a chave `{chave}`", tabela.nome),
            }));
        }
    }

    let chaves: BTreeSet<&str> = tabela.chave.iter().copied().collect();
    if linha_existe(conn, tabela, &valores, escopo)? {
        atualizar_linha(conn, tabela, &valores, &chaves, escopo)?;
        Ok(true)
    } else {
        inserir_linha(conn, tabela, &valores)?;
        Ok(false)
    }
}

fn valor_sql(tabela: &str, coluna: &str, valor: &Value) -> Resultado<SqlValue> {
    Ok(match valor {
        Value::Null => SqlValue::Null,
        Value::Bool(booleano) => SqlValue::Integer(i64::from(*booleano)),
        Value::Number(numero) => {
            if let Some(inteiro) = numero.as_i64() {
                SqlValue::Integer(inteiro)
            } else if let Some(real) = numero.as_f64() {
                SqlValue::Real(real)
            } else {
                return Err(ErroStore::Dominio(DomainError::Validacao {
                    campo: "backup",
                    motivo: format!("o valor de `{tabela}.{coluna}` não é um número válido"),
                }));
            }
        }
        Value::String(texto) => SqlValue::Text(texto.clone()),
        Value::Array(_) | Value::Object(_) => {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "backup",
                motivo: format!(
                    "o valor de `{tabela}.{coluna}` é um objeto ou lista; o banco espera um valor \
                     simples"
                ),
            }));
        }
    })
}

fn colunas_da_tabela(conn: &Connection, tabela: &str) -> Resultado<BTreeSet<String>> {
    let mut consulta = conn.prepare(&format!("PRAGMA table_info({tabela})"))?;
    let mut linhas = consulta.query([])?;
    let mut nomes = BTreeSet::new();
    while let Some(linha) = linhas.next()? {
        let nome: String = linha.get(1)?;
        nomes.insert(nome);
    }
    Ok(nomes)
}

fn linha_existe(
    conn: &Connection,
    tabela: &Tabela,
    valores: &BTreeMap<String, SqlValue>,
    escopo: Option<OrgId>,
) -> Resultado<bool> {
    let (filtro, parametros) = filtro_da_chave(tabela, valores, escopo, 0);
    let existe: Option<i64> = conn
        .query_row(
            &format!("SELECT 1 FROM {} WHERE {filtro}", tabela.nome),
            params_from_iter(parametros),
            |linha| linha.get(0),
        )
        .optional()?;
    Ok(existe.is_some())
}

/// Monta `WHERE ...` com os parâmetros posicionais a partir de `inicio` (0 = `?1`).
fn filtro_da_chave(
    tabela: &Tabela,
    valores: &BTreeMap<String, SqlValue>,
    escopo: Option<OrgId>,
    inicio: usize,
) -> (String, Vec<SqlValue>) {
    let mut filtro = Vec::new();
    let mut parametros = Vec::new();
    let mut posicao = inicio;
    if let Some(org) = escopo {
        posicao += 1;
        filtro.push(format!("org_id = ?{posicao}"));
        parametros.push(SqlValue::Text(org.to_string()));
    }
    for chave in tabela.chave {
        posicao += 1;
        filtro.push(format!("{chave} = ?{posicao}"));
        parametros.push(valores.get(*chave).cloned().unwrap_or(SqlValue::Null));
    }
    (filtro.join(" AND "), parametros)
}

fn inserir_linha(
    conn: &Connection,
    tabela: &Tabela,
    valores: &BTreeMap<String, SqlValue>,
) -> Resultado<()> {
    let colunas: Vec<&str> = valores.keys().map(String::as_str).collect();
    let marcadores: Vec<String> = (1..=colunas.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        tabela.nome,
        colunas.join(", "),
        marcadores.join(", ")
    );
    let parametros: Vec<SqlValue> = valores.values().cloned().collect();
    conn.execute(&sql, params_from_iter(parametros))
        .map_err(|erro| ErroStore::da_escrita(tabela.nome, erro))?;
    Ok(())
}

fn atualizar_linha(
    conn: &Connection,
    tabela: &Tabela,
    valores: &BTreeMap<String, SqlValue>,
    chaves: &BTreeSet<&str>,
    escopo: Option<OrgId>,
) -> Resultado<()> {
    let mut definicoes = Vec::new();
    let mut parametros: Vec<SqlValue> = Vec::new();
    let mut posicao = 0;
    for (nome, valor) in valores {
        if chaves.contains(nome.as_str()) || nome == "org_id" {
            continue;
        }
        posicao += 1;
        definicoes.push(format!("{nome} = ?{posicao}"));
        parametros.push(valor.clone());
    }
    if definicoes.is_empty() {
        // Linha que só tem as colunas da chave: já está no estado do arquivo.
        return Ok(());
    }
    let (filtro, parametros_filtro) = filtro_da_chave(tabela, valores, escopo, posicao);
    parametros.extend(parametros_filtro);
    let sql = format!(
        "UPDATE {} SET {} WHERE {filtro}",
        tabela.nome,
        definicoes.join(", ")
    );
    conn.execute(&sql, params_from_iter(parametros))
        .map_err(|erro| ErroStore::da_escrita(tabela.nome, erro))?;
    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::{NovaCategoria, NovaOrg, NovoAgente, Pagina};

    fn banco() -> Store {
        let pasta = std::env::temp_dir().join(format!(
            "poph-backup-store-{}",
            uuid::Uuid::now_v7().simple()
        ));
        Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco")
    }

    fn org_com_dados(store: &Store, slug: &str) -> OrgId {
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: "{\"retencao\":{\"dias\":7}}".to_owned(),
            })
            .expect("cria a org")
            .id;
        store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente");
        store
            .categorias()
            .criar(
                org,
                NovaCategoria {
                    nome: "Pragas".to_owned(),
                    descricao: "urbanas".to_owned(),
                },
            )
            .expect("cria a categoria");
        org
    }

    #[test]
    fn retrato_traz_a_org_e_nao_traz_outra_org() {
        let store = banco();
        let org_a = org_com_dados(&store, "org-a");
        let org_b = org_com_dados(&store, "org-b");
        let retrato = store.retrato_da_org(org_a, true).expect("retrato");
        assert_eq!(retrato.org["id"], Value::String(org_a.to_string()));
        assert_eq!(retrato.tabelas["agents"].len(), 1);
        assert_eq!(retrato.tabelas["product_categories"].len(), 1);
        // A org B tem os mesmos recursos, mas nada dela pode aparecer no retrato de A.
        for linhas in retrato.tabelas.values() {
            for linha in linhas {
                assert_eq!(linha["org_id"], Value::String(org_a.to_string()));
            }
        }
        // Sem conversas, as tabelas do histórico não entram.
        let sem_conversas = store.retrato_da_org(org_a, false).expect("retrato");
        assert!(!sem_conversas.tabelas.contains_key("conversations"));
        assert!(retrato.tabelas.contains_key("conversations"));
        let _ = org_b;
    }

    #[test]
    fn importar_duas_vezes_nao_duplica() {
        let store = banco();
        let origem = org_com_dados(&store, "origem");
        let retrato = store.retrato_da_org(origem, true).expect("retrato");
        let primeiro = store
            .importar_retrato(retrato.clone(), DestinoImportacao::DaOrigem)
            .expect("primeira importação");
        assert_eq!(primeiro.segredos_a_redigitar, 0);
        assert!(!primeiro.org_criada);
        let segundo = store
            .importar_retrato(retrato, DestinoImportacao::DaOrigem)
            .expect("segunda importação");
        assert!(segundo.tabelas.values().all(|t| t.inseridas == 0));
        assert_eq!(segundo.total(), primeiro.total());
        let agentes = store.agentes().listar(origem, Pagina::default()).unwrap();
        assert_eq!(agentes.len(), 1);
    }

    #[test]
    fn importar_numa_instancia_limpa_cria_a_org_e_preserva_os_ids() {
        let origem = banco();
        let org = org_com_dados(&origem, "origem");
        let agente = origem.agentes().listar(org, Pagina::default()).unwrap()[0].id;
        let retrato = origem.retrato_da_org(org, true).expect("retrato");

        let destino = banco();
        let relatorio = destino
            .importar_retrato(retrato, DestinoImportacao::DaOrigem)
            .expect("importa");
        assert!(relatorio.org_criada);
        assert_eq!(relatorio.org, org);
        let agente_importado = destino.agentes().obter(org, agente).expect("agente");
        assert_eq!(agente_importado.slug, "bia");
        let org_importada = destino.orgaos().obter(org).expect("org");
        assert_eq!(org_importada.config_json, "{\"retencao\":{\"dias\":7}}");
    }

    #[test]
    fn importar_numa_org_existente_nao_toca_a_linha_da_org() {
        let origem = banco();
        let org_origem = org_com_dados(&origem, "origem");
        let retrato = origem.retrato_da_org(org_origem, true).expect("retrato");

        let destino = banco();
        let alvo = destino
            .orgaos()
            .criar(NovaOrg {
                nome: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        let relatorio = destino
            .importar_retrato(retrato, DestinoImportacao::Existente(alvo))
            .expect("importa");
        assert!(!relatorio.org_criada);
        assert_eq!(relatorio.org, alvo);
        let org_alvo = destino.orgaos().obter(alvo).expect("org alvo");
        assert_eq!(org_alvo.nome, "Alvo");
        assert_eq!(org_alvo.slug, "alvo");
    }

    /// Cria uma conversa com o nome do cliente já gravado (MX-32).
    fn conversa_com_nome(store: &Store, org: OrgId, nome: &str) -> crate::Conversa {
        let agente = store.agentes().listar(org, Pagina::default()).unwrap()[0].id;
        let conversa = store
            .conversas()
            .criar(
                org,
                crate::NovaConversa {
                    agent_id: agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999990000".to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("cria a conversa");
        store
            .conversas()
            .definir_nome_cliente(org, conversa.id, nome)
            .expect("grava o nome do cliente");
        conversa
    }

    #[test]
    fn backup_fluxos_fichas_e_revisoes_preserva_estado() {
        let origem = banco();
        let org = org_com_dados(&origem, "origem");
        let conversa = conversa_com_nome(&origem, org, "Cliente fictício");
        let agente = origem.agentes().listar(org, Pagina::default()).unwrap()[0].id;
        let ficha = origem.retrato_da_org(org, true).unwrap().tabelas["fichas"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        origem.escrita(move |conn| {
            let org = org.to_string();
            let agente = agente.to_string();
            for (id, slug, atual) in [("flow-a", "first", 2), ("flow-b", "second", 1)] {
                conn.execute("INSERT INTO fluxos (id, org_id, slug, nome, revisao_atual) VALUES (?1, ?2, ?3, ?3, ?4)", rusqlite::params![id, org, slug, atual])?;
                conn.execute("INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em) VALUES (?1, ?2, ?3, 1)", rusqlite::params![org, agente, id])?;
            }
            for (id, fluxo, numero) in [("rev-a1", "flow-a", 1), ("rev-a2", "flow-a", 2), ("rev-b1", "flow-b", 1)] {
                conn.execute("INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json, publicada_em, publicada_por) VALUES (?1, ?2, ?3, ?4, 'fonte', 'hash', '{}', 1, 'cli')", rusqlite::params![id, org, fluxo, numero])?;
            }
            conn.execute("INSERT INTO fluxo_execucoes (id, org_id, ficha_id, fluxo_id, revisao_id, no_atual, iniciada_em) VALUES ('exec', ?1, ?2, 'flow-a', 'rev-a1', 'etapa', 1)", rusqlite::params![org, ficha])?;
            conn.execute("INSERT INTO fluxo_execucao_etapas (id, org_id, execucao_id, no_id, ordem, entrou_em) VALUES ('etapa', ?1, 'exec', 'etapa', 1, 1)", [&org])?;
            conn.execute("INSERT INTO fluxo_efeitos (id, org_id, ficha_id, execucao_id, chave, tipo, created_at) VALUES ('efeito', ?1, ?2, 'exec', 'um', 'mensagem_ao_cliente', 1)", rusqlite::params![org, ficha])?;
            Ok(())
        }).unwrap();
        let retrato = origem.retrato_da_org(org, true).unwrap();
        assert_eq!(retrato.tabelas["fluxos"].len(), 2);
        assert_eq!(retrato.tabelas["fluxo_revisoes"].len(), 3);
        assert_eq!(retrato.tabelas["agente_fluxos"].len(), 2);
        for nome in [
            "fichas",
            "ficha_eventos",
            "fluxo_execucoes",
            "fluxo_execucao_etapas",
            "fluxo_efeitos",
        ] {
            assert!(!retrato.tabelas[nome].is_empty(), "{nome}");
        }
        assert_eq!(
            retrato.tabelas["fichas"][0]["conversation_id"],
            conversa.id.to_string()
        );
        let destino = banco();
        destino
            .importar_retrato_protegido(retrato.clone(), DestinoImportacao::DaOrigem, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            destino.retrato_da_org(org, true).unwrap().tabelas,
            retrato.tabelas
        );
        assert!(
            destino
                .importar_retrato_protegido(retrato.clone(), DestinoImportacao::DaOrigem, false)
                .unwrap()
                .is_err()
        );
        destino
            .importar_retrato_protegido(retrato.clone(), DestinoImportacao::DaOrigem, true)
            .unwrap()
            .unwrap();
        let mut adulterado = retrato.clone();
        adulterado.tabelas.get_mut("fluxo_revisoes").unwrap()[0]["fonte"] = Value::from("alterada");
        assert!(
            destino
                .importar_retrato_protegido(adulterado, DestinoImportacao::DaOrigem, true)
                .is_err()
        );
        assert_eq!(
            destino.retrato_da_org(org, true).unwrap().tabelas,
            retrato.tabelas
        );
        let mut incompleto = retrato.clone();
        incompleto.tabelas.get_mut("fichas").unwrap().clear();
        let banco_incompleto = banco();
        assert!(
            banco_incompleto
                .importar_retrato(incompleto, DestinoImportacao::DaOrigem)
                .is_err()
        );
        assert!(banco_incompleto.orgaos().obter(org).is_err());
        let sem = origem.retrato_da_org(org, false).unwrap();
        assert!(!sem.tabelas.contains_key("fichas"));
        let vazio = banco();
        vazio
            .importar_retrato(sem, DestinoImportacao::DaOrigem)
            .unwrap();
        assert!(vazio.retrato_da_org(org, true).unwrap().tabelas["fichas"].is_empty());
        let mut antigo = retrato.clone();
        for nome in [
            "fluxos",
            "fluxo_revisoes",
            "agente_fluxos",
            "fichas",
            "ficha_eventos",
            "fluxo_execucoes",
            "fluxo_execucao_etapas",
            "fluxo_efeitos",
        ] {
            antigo.tabelas.remove(nome);
        }
        let documento =
            crate::backup_documento::documento(&antigo, true, 0, &BTreeMap::new(), &|_| None);
        let mut documento = documento;
        documento["versao_formato"] = Value::from(1);
        let antigo = crate::backup_documento::retrato_do_documento(&documento).unwrap();
        let destino_antigo = banco();
        destino_antigo
            .importar_retrato(antigo, DestinoImportacao::DaOrigem)
            .unwrap();
        let fichas = destino_antigo
            .retrato_da_org(org, true)
            .unwrap()
            .tabelas
            .remove("fichas")
            .unwrap();
        assert_eq!(fichas.len(), 1);
        assert_eq!(fichas[0]["conversation_id"], conversa.id.to_string());
    }

    #[test]
    fn backup_com_conversas_leva_o_nome_do_cliente() {
        let origem = banco();
        let org = org_com_dados(&origem, "origem");
        let conversa = conversa_com_nome(&origem, org, "Ana Cliente");
        let retrato = origem.retrato_da_org(org, true).expect("retrato");
        assert_eq!(
            retrato.tabelas["conversations"][0]["client_name"],
            Value::String("Ana Cliente".to_owned())
        );

        let destino = banco();
        destino
            .importar_retrato(retrato, DestinoImportacao::DaOrigem)
            .expect("importa");
        let importada = destino
            .conversas()
            .obter(org, conversa.id)
            .expect("conversa importada");
        assert_eq!(importada.nome_cliente.as_deref(), Some("Ana Cliente"));
    }

    #[test]
    fn backup_antigo_sem_a_coluna_importa_e_deixa_o_nome_nulo() {
        let origem = banco();
        let org = org_com_dados(&origem, "origem");
        let conversa = conversa_com_nome(&origem, org, "Ana Cliente");
        let mut retrato = origem.retrato_da_org(org, true).expect("retrato");
        // Arquivo gerado antes de a coluna existir: a linha não traz `client_name`.
        for linha in retrato.tabelas.get_mut("conversations").expect("conversas") {
            linha.as_object_mut().expect("objeto").remove("client_name");
        }

        let destino = banco();
        destino
            .importar_retrato(retrato, DestinoImportacao::DaOrigem)
            .expect("importa o backup antigo");
        let importada = destino
            .conversas()
            .obter(org, conversa.id)
            .expect("conversa importada");
        assert_eq!(importada.nome_cliente, None);
    }

    #[test]
    fn backup_antigo_nao_apaga_o_nome_ja_gravado_no_destino() {
        let origem = banco();
        let org = org_com_dados(&origem, "origem");
        let conversa = conversa_com_nome(&origem, org, "Ana Cliente");
        let retrato_novo = origem.retrato_da_org(org, true).expect("retrato");
        let mut retrato_antigo = retrato_novo.clone();
        for linha in retrato_antigo
            .tabelas
            .get_mut("conversations")
            .expect("conversas")
        {
            linha.as_object_mut().expect("objeto").remove("client_name");
        }

        let destino = banco();
        destino
            .importar_retrato(retrato_novo, DestinoImportacao::DaOrigem)
            .expect("primeira importação, com o nome");
        destino
            .importar_retrato(retrato_antigo, DestinoImportacao::DaOrigem)
            .expect("reimporta o backup antigo");
        let depois = destino
            .conversas()
            .obter(org, conversa.id)
            .expect("conversa");
        assert_eq!(
            depois.nome_cliente.as_deref(),
            Some("Ana Cliente"),
            "coluna que o arquivo não traz fica como estava"
        );
    }

    #[test]
    fn conflito_de_slug_com_id_diferente_e_recusado_sem_gravar() {
        let origem = banco();
        let org_origem = org_com_dados(&origem, "origem");
        let retrato = origem.retrato_da_org(org_origem, true).expect("retrato");

        let destino = banco();
        let alvo = destino
            .orgaos()
            .criar(NovaOrg {
                nome: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        destino
            .agentes()
            .criar(
                alvo,
                NovoAgente {
                    nome: "Outra Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente que conflita");
        let erro = destino
            .importar_retrato(retrato, DestinoImportacao::Existente(alvo))
            .expect_err("devia recusar");
        assert!(erro.to_string().contains("agente `bia`"), "{erro}");
        // Nada foi gravado: a org alvo continua sem categoria.
        assert_eq!(
            destino
                .categorias()
                .listar(alvo, Pagina::default())
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn conflitos_da_importacao_saem_sem_gravar() {
        let origem = banco();
        let org_origem = org_com_dados(&origem, "origem");
        let retrato = origem.retrato_da_org(org_origem, false).expect("retrato");

        // Instância limpa: nada conflita.
        let limpa = banco();
        assert!(
            limpa
                .conferir_conflitos_importacao(&retrato, DestinoImportacao::DaOrigem)
                .expect("confere")
                .is_empty()
        );

        // Destino com o mesmo slug de agente e outro id: um conflito, e nada gravado.
        let destino = banco();
        let alvo = destino
            .orgaos()
            .criar(NovaOrg {
                nome: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        destino
            .agentes()
            .criar(
                alvo,
                NovoAgente {
                    nome: "Outra Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente que conflita");
        let conflitos = destino
            .conferir_conflitos_importacao(&retrato, DestinoImportacao::Existente(alvo))
            .expect("confere");
        assert_eq!(conflitos.len(), 1, "{conflitos:?}");
        assert!(conflitos[0].contains("agente `bia`"), "{conflitos:?}");
        // A conferência é leitura: a org alvo continua sem categoria.
        assert_eq!(
            destino
                .categorias()
                .listar(alvo, Pagina::default())
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn conferencia_de_atualizacao_aponta_o_que_seria_sobrescrito_sem_gravar() {
        let origem = banco();
        let org_origem = org_com_dados(&origem, "origem");
        let retrato = origem.retrato_da_org(org_origem, true).expect("retrato");
        let linhas_do_arquivo: u64 = retrato.tabelas.values().map(|l| l.len() as u64).sum();

        // Instância limpa: nada seria atualizado, e a conferência não cria a org.
        let limpa = banco();
        let vazio = limpa
            .conferir_atualizacoes_importacao(&retrato, DestinoImportacao::DaOrigem)
            .expect("confere");
        assert!(vazio.vazia(), "{vazio:?}");
        assert_eq!(vazio.total, 0);
        assert!(vazio.exemplos.is_empty());
        assert!(
            limpa.orgaos().obter(org_origem).is_err(),
            "a conferência é leitura: não pode criar a org"
        );

        // Na própria org de origem, tudo o que o arquivo traria conta como atualização — inclusive
        // a linha da org, que a importação regravaria.
        let mesmo = banco();
        mesmo
            .importar_retrato(retrato.clone(), DestinoImportacao::DaOrigem)
            .expect("primeira importação");
        let atualizacoes = mesmo
            .conferir_atualizacoes_importacao(&retrato, DestinoImportacao::DaOrigem)
            .expect("confere");
        assert!(!atualizacoes.vazia());
        assert_eq!(
            atualizacoes.total,
            linhas_do_arquivo + 1,
            "{atualizacoes:?}"
        );
        assert_eq!(atualizacoes.tabelas.get("agents"), Some(&1));
        assert_eq!(atualizacoes.tabelas.get("orgs"), Some(&1));
        assert!(
            atualizacoes
                .exemplos
                .iter()
                .any(|exemplo| exemplo == "agents: bia"),
            "exemplo legível com slug: {:?}",
            atualizacoes.exemplos
        );
        assert!(atualizacoes.exemplos.len() <= MAX_EXEMPLOS_ATUALIZACAO);
        assert!(
            atualizacoes.resumo().contains("agents 1"),
            "{}",
            atualizacoes.resumo()
        );
        // E a conferência não gravou nada: o retrato continua igual ao que entrou.
        let depois = mesmo.retrato_da_org(org_origem, true).expect("retrato");
        assert_eq!(depois.tabelas["agents"], retrato.tabelas["agents"]);

        // Numa org de destino, a linha da org não é tocada; só as do arquivo contam.
        let destino = banco();
        let alvo = destino
            .orgaos()
            .criar(NovaOrg {
                nome: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        assert!(
            destino
                .conferir_atualizacoes_importacao(&retrato, DestinoImportacao::Existente(alvo))
                .expect("confere")
                .vazia(),
            "org alvo vazia: a importação só insere"
        );
        destino
            .importar_retrato(retrato.clone(), DestinoImportacao::Existente(alvo))
            .expect("importa");
        let na_org_alvo = destino
            .conferir_atualizacoes_importacao(&retrato, DestinoImportacao::Existente(alvo))
            .expect("confere");
        assert_eq!(na_org_alvo.total, linhas_do_arquivo);
        assert!(
            !na_org_alvo.tabelas.contains_key("orgs"),
            "o caminho Existente não escreve em orgs"
        );
    }

    #[test]
    fn coluna_desconhecida_e_recusada() {
        let store = banco();
        let org = org_com_dados(&store, "origem");
        let mut retrato = store.retrato_da_org(org, false).expect("retrato");
        retrato.tabelas.get_mut("agents").unwrap()[0]
            .as_object_mut()
            .unwrap()
            .insert("coluna_do_futuro".to_owned(), Value::from(1));
        let erro = store
            .importar_retrato(retrato, DestinoImportacao::DaOrigem)
            .expect_err("devia recusar");
        assert!(erro.to_string().contains("coluna_do_futuro"), "{erro}");
    }
}

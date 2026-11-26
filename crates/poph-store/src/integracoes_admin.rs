//! Operações administrativas de integrações sem acesso aos segredos.
use crate::{
    Store,
    erro::{ErroStore, Resultado},
    modelo::Pagina,
};
use poph_types::{AgentId, DomainError, IntegrationId, OrgId};
use rusqlite::{OptionalExtension, Row, params};
use serde_json::Value;

/// Campos de uma integração para a API administrativa (nunca o segredo em claro).
///
/// `configuracao` é o JSON do operador (público, devolvido na resposta) e `segredos_cifrados` é o
/// blob de `secrets_enc` — **nunca** serializado: só o servidor decifra, e apenas para saber quais
/// campos estão preenchidos. O `Debug` redige o blob para não vazar em log de erro.
/// O que uma atualização de integração pode mudar (cada `None` = manter).
#[derive(Debug, Default)]
pub struct AlteracaoIntegracao<'a> {
    /// Versão lida pelo operador (`If-Match`).
    pub versao: u64,
    /// Situação (`active`, `inactive`, `archived`).
    pub status: Option<&'a str>,
    /// Estado de conexão (estado de execução).
    pub estado: Option<&'a str>,
    /// Configuração do operador, substituindo a anterior por inteiro.
    pub configuracao: Option<&'a Value>,
    /// Segredos: `None` mantém, `Some(None)` limpa, `Some(Some(blob))` troca o blob cifrado.
    pub segredos: Option<Option<&'a [u8]>>,
}

pub struct IntegracaoAdmin {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub connection_state: String,
    pub version: u64,
    pub tem_segredo: bool,
    /// Configuração do operador, já decodificada de `config_json`.
    pub configuracao: Value,
    /// Segredos cifrados (blob do AES-256-GCM). `None` = nenhum segredo gravado.
    pub segredos_cifrados: Option<Vec<u8>>,
}

impl std::fmt::Debug for IntegracaoAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntegracaoAdmin")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("connection_state", &self.connection_state)
            .field("version", &self.version)
            .field("tem_segredo", &self.tem_segredo)
            .field("configuracao", &self.configuracao)
            .field(
                "segredos_cifrados",
                &self.segredos_cifrados.as_ref().map(|_| "…"),
            )
            .finish()
    }
}

fn linha(r: &Row<'_>) -> rusqlite::Result<IntegracaoAdmin> {
    let configuracao: String = r.get(6)?;
    Ok(IntegracaoAdmin {
        id: r.get(0)?,
        kind: r.get(1)?,
        status: r.get(2)?,
        connection_state: r.get(3)?,
        version: r.get::<_, i64>(4)? as u64,
        tem_segredo: r.get(5)?,
        configuracao: serde_json::from_str(&configuracao)
            .unwrap_or_else(|_| Value::Object(Default::default())),
        segredos_cifrados: r.get(7)?,
    })
}
const COLUNAS: &str = "id, kind, status, connection_state, version, secrets_enc IS NOT NULL, config_json, secrets_enc";
impl Store {
    /// Lista integrações pertencentes ao agente e à organização.
    pub fn integracoes_listar(
        &self,
        org: OrgId,
        agente: AgentId,
        pagina: Pagina,
    ) -> Resultado<Vec<IntegracaoAdmin>> {
        self.agentes().obter(org, agente)?;
        self.leitura(|conn| {
            let mut q = conn.prepare(&format!("SELECT {COLUNAS} FROM integrations WHERE org_id=?1 AND agent_id=?2 AND (?3 IS NULL OR id>?3) ORDER BY id LIMIT ?4"))?;
            let rows = q.query_map(params![org.to_string(),agente.to_string(),pagina.apos.map(|x|x.to_string()),pagina.limite],linha)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
    /// Obtém apenas os campos públicos.
    pub fn integracao_obter(
        &self,
        org: OrgId,
        agente: AgentId,
        id: IntegrationId,
    ) -> Resultado<IntegracaoAdmin> {
        let item = self.leitura(|conn| Ok(conn.query_row(&format!("SELECT {COLUNAS} FROM integrations WHERE org_id=?1 AND agent_id=?2 AND id=?3"),params![org.to_string(),agente.to_string(),id.to_string()],linha).optional()?))?;
        crate::leitores::exigir(item, "integração", &id.to_string())
    }
    /// Cria uma integração sem configuração e sem segredo.
    pub fn integracao_criar(
        &self,
        org: OrgId,
        agente: AgentId,
        kind: String,
        status: String,
    ) -> Resultado<IntegracaoAdmin> {
        self.integracao_criar_com_configuracao(
            org,
            agente,
            kind,
            status,
            Value::Object(Default::default()),
            None,
        )
    }

    /// Cria uma integração com a configuração pública e o blob de segredos já cifrado.
    ///
    /// Nasce na versão 1: configuração e segredo de criação não são uma edição separada, então não
    /// gastam uma versão do `If-Match`.
    pub fn integracao_criar_com_configuracao(
        &self,
        org: OrgId,
        agente: AgentId,
        kind: String,
        status: String,
        configuracao: Value,
        segredos: Option<Vec<u8>>,
    ) -> Resultado<IntegracaoAdmin> {
        self.agentes().obter(org, agente)?;
        let id = IntegrationId::generate();
        let now = self.agora();
        let configuracao = configuracao.to_string();
        self.escrita(move |conn| { conn.execute("INSERT INTO integrations (id,org_id,agent_id,kind,status,config_json,secrets_enc,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![id.to_string(),org.to_string(),agente.to_string(),kind,status,configuracao,segredos,now])?; Ok(()) })?;
        self.integracao_obter(org, agente, id)
    }
    /// Atualiza situação, estado, configuração e segredos, com verificação atômica de versão.
    pub fn integracao_atualizar(
        &self,
        org: OrgId,
        agente: AgentId,
        id: IntegrationId,
        alteracao: AlteracaoIntegracao<'_>,
    ) -> Resultado<IntegracaoAdmin> {
        let now = self.agora();
        let AlteracaoIntegracao {
            versao,
            status,
            estado,
            configuracao,
            segredos,
        } = alteracao;
        let status = status.map(str::to_owned);
        let estado = estado.map(str::to_owned);
        let configuracao = configuracao.map(|valor| valor.to_string());
        let (limpar_segredos, segredos) = match segredos {
            None => (false, None),
            Some(None) => (true, None),
            Some(Some(blob)) => (false, Some(blob.to_vec())),
        };
        self.escrita(move |conn| {
            let n=conn.execute(
                "UPDATE integrations SET status=COALESCE(?4,status), connection_state=COALESCE(?5,connection_state), config_json=COALESCE(?6,config_json), secrets_enc=CASE WHEN ?7 THEN NULL ELSE COALESCE(?8,secrets_enc) END, version=version+1, updated_at=?9 WHERE org_id=?1 AND agent_id=?2 AND id=?3 AND version=?10",
                params![org.to_string(),agente.to_string(),id.to_string(),status,estado,configuracao,limpar_segredos,segredos,now,versao as i64],
            )?;
            if n==0 {
                let atual: Option<i64>=conn.query_row("SELECT version FROM integrations WHERE org_id=?1 AND agent_id=?2 AND id=?3",params![org.to_string(),agente.to_string(),id.to_string()],|r|r.get(0)).optional()?;
                return Err(ErroStore::Dominio(match atual {Some(atual)=>DomainError::VersionConflict{entity:"integração",id:id.to_string(),expected:versao,current:atual as u64},None=>DomainError::NotFound{entity:"integração",id:id.to_string()}}));
            }
            Ok(())
        })?;
        self.integracao_obter(org, agente, id)
    }
    /// Exclui uma integração pertencente ao agente.
    pub fn integracao_excluir(
        &self,
        org: OrgId,
        agente: AgentId,
        id: IntegrationId,
    ) -> Resultado<()> {
        self.escrita(move |conn| {
            let n = conn.execute(
                "DELETE FROM integrations WHERE org_id=?1 AND agent_id=?2 AND id=?3",
                params![org.to_string(), agente.to_string(), id.to_string()],
            )?;
            if n == 0 {
                return Err(ErroStore::Dominio(DomainError::NotFound {
                    entity: "integração",
                    id: id.to_string(),
                }));
            }
            Ok(())
        })
    }
}

impl Store {
    /// Integração do canal dentro da org (agente dono, tipo e situação). `None` quando não existe.
    ///
    /// Não devolve configuração nem segredo: o webhook só precisa saber de quem é a integração.
    pub fn integracao_do_canal(
        &self,
        org: OrgId,
        integracao: IntegrationId,
    ) -> Resultado<Option<crate::modelo::IntegracaoDoCanal>> {
        let texto_org = org.to_string();
        let texto_integracao = integracao.to_string();
        let achada = self.leitura(move |conn| {
            conn.query_row(
                "SELECT agent_id, kind, status FROM integrations WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_integracao],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        match achada {
            None => Ok(None),
            Some((agente, kind, status)) => Ok(Some(crate::modelo::IntegracaoDoCanal {
                id: integracao,
                agent_id: agente.parse::<AgentId>()?,
                kind,
                status,
            })),
        }
    }

    /// Grava o estado de conexão que o canal informou (ex.: `connected`, `disconnected`).
    ///
    /// Não mexe na `version`: é estado de execução, não edição de configuração, então não pode
    /// dar 412 numa alteração administrativa que estiver em curso. Devolve `false` quando a
    /// integração não existe (nada é criado).
    pub fn integracao_definir_estado_conexao(
        &self,
        org: OrgId,
        integracao: IntegrationId,
        estado: &str,
    ) -> Resultado<bool> {
        let agora = self.agora();
        let texto_org = org.to_string();
        let texto_integracao = integracao.to_string();
        let estado = estado.to_owned();
        self.escrita(move |conn| {
            let alteradas = conn
                .execute(
                    "UPDATE integrations SET connection_state = ?3, updated_at = ?4
                      WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_integracao, estado, agora],
                )
                .map_err(ErroStore::from)?;
            Ok(alteradas > 0)
        })
    }
}

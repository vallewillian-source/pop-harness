//! Tabelas do módulo Matrix (MX-08): org ligada, salas, espaços, cursores e falhas.
//!
//! Decisão de fronteira: o repositório mora em `poph-store`, e não em `poph-matrix`, porque o
//! banco e os tipos de linha são responsabilidade deste crate. `poph-matrix` conhece o Synapse (é
//! um cliente HTTP); quem lê e grava as tabelas do módulo é o crate que já fala com o banco, e o
//! `poph-server` (que importa os dois) liga uma coisa na outra. Assim a tabela de dependências do
//! `AGENTS.md` não ganha aresta nova: `poph-matrix` continua dependendo só de `poph-types`.
//!
//! Regras que este repositório garante:
//! - **Toda operação recebe a org.** A única leitura sem org é [`Matrix::orgs_ligadas`], que é o
//!   passo de descoberta do laço de espelhamento (o laço ainda não sabe qual org olhar), no mesmo
//!   espírito de `Tokens::por_hash`.
//! - **Idempotência**: garantir a mesma sala duas vezes devolve a que existe; marcar `ready` de
//!   novo não muda nada; os cursores só andam para a frente; a falha da mesma mensagem atualiza a
//!   linha em vez de duplicar.
//! - **Nada de dado de cliente**: as colunas guardam ids, estados e o código curto do erro.

use std::collections::HashMap;

use poph_types::{ConversationId, DepartmentId, DomainError, MessageId, OrgId, OutboxId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::ConversationStatus;
use crate::repos::{LIMITE_NOME_CLIENTE, id_da_coluna, u32_da_coluna, u64_da_coluna};

/// Situação da criação da sala de uma conversa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstadoSala {
    /// Pedido de criação feito e linha gravada; a sala pode ainda não existir no Synapse.
    Criando,
    /// Sala existe no Synapse e o estado inicial foi aplicado.
    Pronta,
}

impl EstadoSala {
    /// Texto gravado na coluna `state`.
    pub fn como_texto(self) -> &'static str {
        match self {
            EstadoSala::Criando => "creating",
            EstadoSala::Pronta => "ready",
        }
    }

    fn de_texto(valor: &str) -> rusqlite::Result<Self> {
        match valor {
            "creating" => Ok(EstadoSala::Criando),
            "ready" => Ok(EstadoSala::Pronta),
            outro => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!(
                    "estado de sala do Matrix desconhecido no banco: `{outro}`"
                ))),
            )),
        }
    }
}

/// Quem criou o espaço do departamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigemEspaco {
    /// O módulo criou o espaço no Synapse.
    Automatico,
    /// O espaço já existia no Synapse e foi ligado ao departamento.
    Ligado,
}

impl OrigemEspaco {
    /// Texto gravado na coluna `origin`.
    pub fn como_texto(self) -> &'static str {
        match self {
            OrigemEspaco::Automatico => "auto",
            OrigemEspaco::Ligado => "linked",
        }
    }

    fn de_texto(valor: &str) -> rusqlite::Result<Self> {
        match valor {
            "auto" => Ok(OrigemEspaco::Automatico),
            "linked" => Ok(OrigemEspaco::Ligado),
            outro => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!(
                    "origem de espaço do Matrix desconhecida no banco: `{outro}`"
                ))),
            )),
        }
    }
}

/// Estado do módulo numa org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstadoMatrixOrg {
    /// Org dona.
    pub org_id: OrgId,
    /// Módulo ligado.
    pub ligado: bool,
    /// Corte do backfill (ms): o instante em que a org ligou o módulo pela última vez.
    /// `None` quando nunca ligou.
    pub ligado_em: Option<i64>,
    /// Última alteração (ms); `0` quando a org nunca ligou nem desligou.
    pub atualizado_em: i64,
}

impl EstadoMatrixOrg {
    /// Linha de uma org que nunca ligou o módulo.
    fn desligado(org: OrgId) -> Self {
        Self {
            org_id: org,
            ligado: false,
            ligado_em: None,
            atualizado_em: 0,
        }
    }
}

/// Sala do Matrix de uma conversa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalaMatrix {
    /// Conversa dona da sala.
    pub conversa: ConversationId,
    /// Org dona.
    pub org_id: OrgId,
    /// Id da sala no Synapse (`!…:servidor`).
    pub room_id: String,
    /// Espaço do departamento em que a sala foi posta, quando houver.
    pub espaco: Option<String>,
    /// Última situação da conversa espelhada.
    pub ultimo_status: Option<ConversationStatus>,
    /// Último nome do cliente já enviado ao Matrix (nome da sala, `client_name` do metadata e
    /// nome de exibição do fantasma). `None` = a sala ainda não recebeu nome (MX-33).
    pub nome_cliente_espelhado: Option<String>,
    /// Situação da criação.
    pub estado: EstadoSala,
    /// Criação da linha (ms).
    pub criada_em: i64,
    /// Última alteração (ms).
    pub atualizada_em: i64,
}

/// Espaço do Matrix ligado a um departamento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EspacoMatrix {
    /// Id da linha.
    pub id: uuid::Uuid,
    /// Org dona.
    pub org_id: OrgId,
    /// Departamento.
    pub departamento: DepartmentId,
    /// Id do espaço no Synapse.
    pub room_id: String,
    /// Alias do espaço no Synapse (`#…:servidor`).
    pub alias: String,
    /// Quem criou o espaço.
    pub origem: OrigemEspaco,
    /// Criação da linha (ms).
    pub criada_em: i64,
    /// Última alteração (ms).
    pub atualizada_em: i64,
}

/// O que o desligamento do espaço de um departamento fez (MX-22).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EspacoDesligado {
    /// O departamento tinha espaço gravado e a linha saiu.
    pub removido: bool,
    /// Salas que deixaram de estar registradas naquele espaço.
    pub salas_esquecidas: u64,
}

/// Atraso do cursor: o que ainda falta espelhar numa org.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtrasoMatrix {
    /// Mensagens gravadas depois do cursor (só as que o espelho espelha: `user` e `assistant`).
    pub pendentes: u64,
    /// Criação (ms) da mais antiga delas; `None` quando não há nenhuma.
    pub mais_antiga_em: Option<i64>,
}

/// Cursor do espelhamento no que se refere a conversas.
///
/// É o par (última alteração, id), e não só o instante, para não perder conversa quando duas
/// mudam no mesmo milissegundo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorConversas {
    /// Última `updated_at` já conferida (ms).
    pub atualizado_em: i64,
    /// Id da última conversa conferida nesse instante.
    pub id: String,
}

/// Cursores do espelhamento de uma org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursoresMatrix {
    /// Org dona.
    pub org_id: OrgId,
    /// `rowid` da última mensagem espelhada (o mesmo cursor de `Mensagens::desde`).
    pub mensagens: u64,
    /// Id da mensagem que está em `mensagens` (vazio quando não há). Deixa o laço reconhecer uma
    /// mensagem já espelhada quando o `rowid` foi reaproveitado (MX-14).
    pub mensagens_id: String,
    /// Última conversa conferida.
    pub conversas: CursorConversas,
    /// Versão da linha (`0` quando a org ainda não tem linha de cursor).
    pub versao: u64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

/// Mensagem isolada depois de falha definitiva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalhaMatrix {
    /// Id da linha.
    pub id: uuid::Uuid,
    /// Org dona.
    pub org_id: OrgId,
    /// Mensagem que não foi espelhada.
    pub message_id: MessageId,
    /// Código curto do erro (nunca o texto do cliente).
    pub motivo: String,
    /// Quantas tentativas já falharam.
    pub tentativas: u32,
    /// Criação da linha (ms).
    pub criada_em: i64,
    /// Última alteração (ms).
    pub atualizada_em: i64,
}

/// Limite de caracteres do motivo curto da falha (o mesmo `CHECK` da tabela).
pub const LIMITE_MOTIVO_FALHA: usize = 64;

/// Limite de linhas de falha por listagem.
pub const LIMITE_FALHAS: u32 = 200;

/// Acesso às tabelas do módulo Matrix.
pub struct Matrix<'a>(pub(crate) &'a Store);

impl Matrix<'_> {
    // ─────────────────────────── Org ligada ───────────────────────────

    /// Liga o módulo na org. Repetir não falha nem muda o corte do backfill.
    pub fn ligar(&self, org: OrgId) -> Resultado<EstadoMatrixOrg> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_orgs (org_id, enabled, enabled_at, updated_at)
                 VALUES (?1, 1, ?2, ?2)
                 ON CONFLICT (org_id) DO UPDATE SET
                     enabled = 1,
                     enabled_at = excluded.enabled_at,
                     updated_at = excluded.updated_at
                 WHERE matrix_orgs.enabled = 0",
                params![texto_org, agora],
            )
            .map(|_| ())
            .map_err(|erro| StoreError::from_write("org", erro))
        })?;
        self.estado(org)
    }

    /// Desliga o módulo na org. `enabled_at` (o corte do backfill) fica como estava.
    pub fn desligar(&self, org: OrgId) -> Resultado<EstadoMatrixOrg> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_orgs (org_id, enabled, enabled_at, updated_at)
                 VALUES (?1, 0, NULL, ?2)
                 ON CONFLICT (org_id) DO UPDATE SET
                     enabled = 0,
                     updated_at = excluded.updated_at
                 WHERE matrix_orgs.enabled = 1",
                params![texto_org, agora],
            )
            .map(|_| ())
            .map_err(|erro| StoreError::from_write("org", erro))
        })?;
        self.estado(org)
    }

    /// Estado do módulo na org (desligado quando nunca foi ligado).
    pub fn estado(&self, org: OrgId) -> Resultado<EstadoMatrixOrg> {
        let texto_org = org.to_string();
        let encontrado = self.0.read(move |conn| {
            conn.query_row(
                "SELECT org_id, enabled, enabled_at, updated_at FROM matrix_orgs WHERE org_id = ?1",
                params![texto_org],
                linha_estado_org,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        Ok(encontrado.unwrap_or_else(|| EstadoMatrixOrg::desligado(org)))
    }

    /// Orgs com o módulo ligado, na ordem do id.
    ///
    /// **Única leitura sem org deste repositório, de propósito:** é o passo de descoberta do laço
    /// de espelhamento, que ainda não sabe qual org olhar. Ela devolve só o que o laço precisa
    /// (org, corte do backfill), nada de conteúdo de conversa.
    pub fn orgs_ligadas(&self) -> Resultado<Vec<EstadoMatrixOrg>> {
        self.0.read(|conn| {
            let mut consulta = conn.prepare(
                "SELECT org_id, enabled, enabled_at, updated_at FROM matrix_orgs
                  WHERE enabled = 1 ORDER BY org_id",
            )?;
            let linhas = consulta.query_map([], linha_estado_org)?;
            let mut orgs = Vec::new();
            for linha in linhas {
                orgs.push(linha?);
            }
            Ok(orgs)
        })
    }

    // ─────────────────────────── Salas ───────────────────────────

    /// Garante a linha da sala da conversa e devolve a que está gravada.
    ///
    /// Idempotente por conversa: chamar de novo com o mesmo `room_id` devolve a mesma linha. Com um
    /// `room_id` **diferente** devolve a linha que já existe (a primeira ganha) — quem chama compara
    /// e decide recriar. Sala já ligada a outra conversa é recusada com conflito.
    pub fn garantir_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        room_id: &str,
        espaco: Option<&str>,
    ) -> Resultado<SalaMatrix> {
        exigir_texto("sala", "room_id", room_id, 255)?;
        if let Some(espaco) = espaco {
            exigir_texto("espaço", "space_room_id", espaco, 255)?;
        }
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let sala = room_id.to_owned();
        let espaco = espaco.map(str::to_owned);
        let gravada = self.0.write(move |conn| {
            // A sala é de uma conversa só: recusa antes de gravar quando já está em outra.
            let dona: Option<String> = conn
                .query_row(
                    "SELECT conversation_id FROM matrix_rooms WHERE room_id = ?1",
                    params![sala],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if let Some(dona) = dona.filter(|dona| *dona != texto_conversa) {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "sala do Matrix",
                    id: sala.clone(),
                    reason: format!("esta sala já está ligada à conversa {dona}"),
                }));
            }
            conn.execute(
                "INSERT INTO matrix_rooms
                    (conversation_id, org_id, room_id, space_room_id, state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'creating', ?5, ?5)
                 ON CONFLICT (conversation_id) DO NOTHING",
                params![texto_conversa, texto_org, sala, espaco, agora],
            )
            .map_err(|erro| StoreError::from_write("sala", erro))?;
            sala_em(conn, &texto_org, &texto_conversa)
        })?;
        gravada.ok_or_else(|| sala_ausente(&conversa.to_string()))
    }

    /// Passa a sala de `creating` para `ready` (idempotente: já pronta continua pronta).
    pub fn marcar_sala_pronta(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Resultado<SalaMatrix> {
        self.marcar_sala_pronta_com_nome(org, conversa, None)
    }

    /// Passa a sala de `creating` para `ready` gravando o nome do cliente que foi usado na criação
    /// (`None` quando a sala nasceu com o identificador), numa escrita só.
    ///
    /// O nome vai para `client_name_espelhado` (MX-33): é o que o laço de estado compara para saber
    /// se precisa renomear a sala. A guarda de estado continua a mesma de
    /// [`Matrix::marcar_sala_pronta`]: já pronta não é reescrita.
    pub fn marcar_sala_pronta_com_nome(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome_espelhado: Option<&str>,
    ) -> Resultado<SalaMatrix> {
        if let Some(nome) = nome_espelhado {
            exigir_texto(
                "nome do cliente",
                "client_name_espelhado",
                nome,
                LIMITE_NOME_CLIENTE,
            )?;
        }
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let nome = nome_espelhado.map(str::to_owned);
        let gravada = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET state = 'ready', client_name_espelhado = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND state = 'creating'",
                params![texto_org, texto_conversa, nome, agora],
            )
            .map_err(|erro| StoreError::from_write("sala", erro))?;
            sala_em(conn, &texto_org, &texto_conversa)
        })?;
        gravada.ok_or_else(|| sala_ausente(&conversa.to_string()))
    }

    /// Grava o último nome do cliente já espelhado na sala (MX-33) e devolve a linha.
    ///
    /// Idempotente: repetir o mesmo nome não reescreve (nem sobe `updated_at`). Nome vazio é recusado
    /// — a regra é que o nome nunca volta a vazio: conversa sem nome mantém o que a sala já tem.
    /// Sala de outra org ou inexistente é `NaoEncontrado`.
    pub fn definir_nome_cliente_espelhado(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome: &str,
    ) -> Resultado<SalaMatrix> {
        exigir_texto(
            "nome do cliente",
            "client_name_espelhado",
            nome,
            LIMITE_NOME_CLIENTE,
        )?;
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let nome = nome.to_owned();
        let gravada = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET client_name_espelhado = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND client_name_espelhado IS NOT ?3",
                params![texto_org, texto_conversa, nome, agora],
            )
            .map_err(|erro| StoreError::from_write("nome da sala", erro))?;
            sala_em(conn, &texto_org, &texto_conversa)
        })?;
        gravada.ok_or_else(|| sala_ausente(&conversa.to_string()))
    }

    /// Sala da conversa (ou `None` quando a conversa ainda não tem sala nesta org).
    pub fn sala(&self, org: OrgId, conversa: ConversationId) -> Resultado<Option<SalaMatrix>> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        self.0
            .read(move |conn| sala_em(conn, &texto_org, &texto_conversa))
    }

    /// Sala `ready` de cada conversa pedida, numa consulta só (RP-01): a lista de conversas traz o
    /// `matrix_room_id` da página inteira. Conversa sem sala pronta (sem linha ou ainda em
    /// `creating`) fica de fora do resultado. Sempre filtrado por org.
    pub fn salas_prontas(
        &self,
        org: OrgId,
        conversas: &[ConversationId],
    ) -> Resultado<HashMap<ConversationId, String>> {
        if conversas.is_empty() {
            return Ok(HashMap::new());
        }
        let texto_org = org.to_string();
        let lista = serde_json::to_string(
            &conversas
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_owned());
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT conversation_id, room_id FROM matrix_rooms
                  WHERE org_id = ?1 AND state = 'ready'
                    AND conversation_id IN (SELECT value FROM json_each(?2))",
            )?;
            let linhas = consulta.query_map(params![texto_org, lista], |linha| {
                Ok((
                    id_da_coluna::<ConversationId>(linha, 0)?,
                    linha.get::<_, String>(1)?,
                ))
            })?;
            let mut salas = HashMap::new();
            for linha in linhas {
                let (conversa, sala) = linha?;
                salas.insert(conversa, sala);
            }
            Ok(salas)
        })
    }

    /// Sala pelo id do Synapse (usado quando chega evento de uma sala).
    pub fn sala_por_room_id(&self, org: OrgId, room_id: &str) -> Resultado<Option<SalaMatrix>> {
        let texto_org = org.to_string();
        let sala = room_id.to_owned();
        self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS_SALA} FROM matrix_rooms WHERE org_id = ?1 AND room_id = ?2"
                ),
                params![texto_org, sala],
                linha_sala,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    /// Grava a última situação da conversa espelhada (idempotente: repetir o mesmo valor não muda
    /// nada além do carimbo de tempo).
    pub fn definir_ultimo_status(
        &self,
        org: OrgId,
        conversa: ConversationId,
        status: ConversationStatus,
    ) -> Resultado<SalaMatrix> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let gravada = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET last_status = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2",
                params![texto_org, texto_conversa, status.as_text(), agora],
            )
            .map_err(|erro| StoreError::from_write("sala", erro))?;
            sala_em(conn, &texto_org, &texto_conversa)
        })?;
        gravada.ok_or_else(|| sala_ausente(&conversa.to_string()))
    }

    /// Grava o espaço em que a sala está (MX-12) e devolve a linha.
    ///
    /// Idempotente: repetir o mesmo espaço não muda nada além do carimbo de tempo; `None` tira a
    /// sala do espaço (conversa sem departamento). Sala de outra org ou inexistente é
    /// `NaoEncontrado`.
    pub fn definir_espaco_da_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        espaco: Option<&str>,
    ) -> Resultado<SalaMatrix> {
        if let Some(espaco) = espaco {
            exigir_texto("espaço", "space_room_id", espaco, 255)?;
        }
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let espaco = espaco.map(str::to_owned);
        let gravada = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET space_room_id = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND space_room_id IS NOT ?3",
                params![texto_org, texto_conversa, espaco, agora],
            )
            .map_err(|erro| StoreError::from_write("espaço da sala", erro))?;
            sala_em(conn, &texto_org, &texto_conversa)
        })?;
        gravada.ok_or_else(|| sala_ausente(&conversa.to_string()))
    }

    // ─────────────────────────── Espaços ───────────────────────────

    /// Garante o espaço do departamento e devolve o que está gravado.
    ///
    /// Idempotente por departamento: chamar de novo devolve o mesmo espaço (o alias gravado não é
    /// trocado). Espaço ou alias já ligado a **outro** departamento da mesma org é recusado.
    pub fn garantir_espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
        room_id: &str,
        alias: &str,
        origem: OrigemEspaco,
    ) -> Resultado<EspacoMatrix> {
        exigir_texto("espaço", "space_room_id", room_id, 255)?;
        exigir_texto("espaço", "alias", alias, 255)?;
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_departamento = departamento.to_string();
        let sala = room_id.to_owned();
        let alias = alias.to_owned();
        let gravado = self.0.write(move |conn| {
            // Espaço e alias são de um departamento só dentro da org.
            let dono: Option<String> = conn
                .query_row(
                    "SELECT department_id FROM matrix_spaces
                      WHERE org_id = ?1 AND (space_room_id = ?2 OR alias = ?3)",
                    params![texto_org, sala, alias],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if let Some(dono) = dono.filter(|dono| *dono != texto_departamento) {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "espaço do Matrix",
                    id: sala.clone(),
                    reason: format!("este espaço já está ligado ao departamento {dono}"),
                }));
            }
            conn.execute(
                "INSERT INTO matrix_spaces
                    (id, org_id, department_id, space_room_id, alias, origin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT (org_id, department_id) DO NOTHING",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    texto_org,
                    texto_departamento,
                    sala,
                    alias,
                    origem.como_texto(),
                    agora
                ],
            )
            .map_err(|erro| StoreError::from_write("espaço", erro))?;
            espaco_em(conn, &texto_org, &texto_departamento)
        })?;
        gravado.ok_or_else(|| {
            StoreError::Domain(DomainError::NotFound {
                entity: "espaço do Matrix",
                id: departamento.to_string(),
            })
        })
    }

    /// Espaço do departamento (ou `None`).
    pub fn espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
    ) -> Resultado<Option<EspacoMatrix>> {
        let texto_org = org.to_string();
        let texto_departamento = departamento.to_string();
        self.0
            .read(move |conn| espaco_em(conn, &texto_org, &texto_departamento))
    }

    /// Espaços da org, na ordem de criação.
    pub fn listar_espacos(&self, org: OrgId) -> Resultado<Vec<EspacoMatrix>> {
        let texto_org = org.to_string();
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_ESPACO} FROM matrix_spaces WHERE org_id = ?1 ORDER BY id"
            ))?;
            let linhas = consulta.query_map(params![texto_org], linha_espaco)?;
            let mut espacos = Vec::new();
            for linha in linhas {
                espacos.push(linha?);
            }
            Ok(espacos)
        })
    }

    /// Desliga o espaço do departamento (MX-22): apaga a linha de `matrix_spaces` e esquece o
    /// vínculo das salas que estavam nele.
    ///
    /// **Não fala com o Synapse e não apaga nada dele**: o espaço e as salas continuam existindo no
    /// Element; o módulo é que deixa de saber que elas estão lá. Repetir não falha (devolve
    /// `removido = false` e nenhuma sala esquecida). O efeito é reversível: a próxima alteração de
    /// uma conversa daquele departamento volta a pôr a sala no espaço (MX-15), reaproveitando o
    /// mesmo espaço do Element pelo alias.
    pub fn desligar_espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
    ) -> Resultado<EspacoDesligado> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_departamento = departamento.to_string();
        self.0.write(move |conn| {
            let gravado: Option<String> = conn
                .query_row(
                    "SELECT space_room_id FROM matrix_spaces
                      WHERE org_id = ?1 AND department_id = ?2",
                    params![texto_org, texto_departamento],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some(espaco) = gravado else {
                return Ok(EspacoDesligado::default());
            };
            conn.execute(
                "DELETE FROM matrix_spaces WHERE org_id = ?1 AND department_id = ?2",
                params![texto_org, texto_departamento],
            )
            .map_err(|erro| StoreError::from_write("espaço", erro))?;
            let esquecidas = conn
                .execute(
                    "UPDATE matrix_rooms SET space_room_id = NULL, updated_at = ?3
                      WHERE org_id = ?1 AND space_room_id = ?2",
                    params![texto_org, espaco, agora],
                )
                .map_err(|erro| StoreError::from_write("espaço da sala", erro))?;
            Ok(EspacoDesligado {
                removido: true,
                salas_esquecidas: u64::try_from(esquecidas).unwrap_or(u64::MAX),
            })
        })
    }

    // ─────────────────────────── Cursores ───────────────────────────

    /// Cursores da org (zerados quando ela nunca espelhou nada).
    pub fn ler_cursores(&self, org: OrgId) -> Resultado<CursoresMatrix> {
        let texto_org = org.to_string();
        let encontrado = self.0.read(move |conn| cursores_em(conn, &texto_org))?;
        Ok(encontrado.unwrap_or_else(|| CursoresMatrix {
            org_id: org,
            mensagens: 0,
            mensagens_id: String::new(),
            conversas: CursorConversas::default(),
            versao: 0,
            atualizado_em: 0,
        }))
    }

    /// Cursor de mensagens por org e o que falta espelhar (MX-22: o "atraso" da API).
    ///
    /// O filtro é o mesmo do laço de espelhamento (`role IN ('user','assistant')`), então o número é
    /// exatamente o que falta entregar ao Synapse — não conta mensagem de ferramenta nem de sistema,
    /// que o espelho nunca envia.
    pub fn atraso_das_mensagens(&self, org: OrgId, cursor: u64) -> Resultado<AtrasoMatrix> {
        let texto_org = org.to_string();
        let cursor = i64::try_from(cursor).unwrap_or(i64::MAX);
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT count(*), min(created_at) FROM messages
                  WHERE org_id = ?1 AND rowid > ?2 AND role IN ('user', 'assistant')",
                params![texto_org, cursor],
                |linha| {
                    Ok(AtrasoMatrix {
                        pendentes: u64_da_coluna(linha, 0)?,
                        mais_antiga_em: linha.get::<_, Option<i64>>(1)?,
                    })
                },
            )
            .map_err(StoreError::from)
        })
    }

    /// Avança o cursor de mensagens **só para a frente** e devolve os cursores resultantes.
    ///
    /// Passar o mesmo `rowid` (ou um menor) não muda nada: é o que torna o laço idempotente quando
    /// ele repete um lote.
    pub fn avancar_mensagens(&self, org: OrgId, rowid: u64) -> Resultado<CursoresMatrix> {
        if rowid > i64::MAX as u64 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "messages_cursor",
                reason: "cursor de mensagens fora da faixa".to_owned(),
            }));
        }
        let agora = self.0.now();
        let texto_org = org.to_string();
        let cursor = rowid as i64;
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado (org_id, messages_cursor, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (org_id) DO UPDATE SET
                     messages_cursor = excluded.messages_cursor,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at
                 WHERE matrix_estado.messages_cursor < excluded.messages_cursor",
                params![texto_org, cursor, agora],
            )
            .map(|_| ())
            .map_err(|erro| StoreError::from_write("messages_cursor", erro))
        })?;
        self.ler_cursores(org)
    }

    /// Avança o cursor de mensagens **só para a frente**, guardando também o id da mensagem do
    /// cursor (MX-14). Quem espelha usa este; o [`Matrix::avancar_mensagens`] fica para quem não
    /// precisa do id.
    pub fn avancar_mensagens_com_id(
        &self,
        org: OrgId,
        rowid: u64,
        mensagem: &str,
    ) -> Resultado<CursoresMatrix> {
        if rowid > i64::MAX as u64 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "messages_cursor",
                reason: "cursor de mensagens fora da faixa".to_owned(),
            }));
        }
        let agora = self.0.now();
        let texto_org = org.to_string();
        let cursor = rowid as i64;
        let id = mensagem.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado (org_id, messages_cursor, messages_cursor_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (org_id) DO UPDATE SET
                     messages_cursor = excluded.messages_cursor,
                     messages_cursor_id = excluded.messages_cursor_id,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at
                 WHERE matrix_estado.messages_cursor < excluded.messages_cursor",
                params![texto_org, cursor, id, agora],
            )
            .map(|_| ())
            .map_err(|erro| StoreError::from_write("messages_cursor", erro))
        })?;
        self.ler_cursores(org)
    }

    /// Põe o cursor de mensagens num ponto **anterior ou igual** ao atual (o ponto seguro quando o
    /// cursor ficou à frente do maior `rowid` que existe — R5 do PRD). Só para esse caso: o avanço
    /// normal é [`Matrix::avancar_mensagens_com_id`].
    pub fn recuar_cursor_mensagens(
        &self,
        org: OrgId,
        rowid: u64,
        mensagem: &str,
    ) -> Resultado<CursoresMatrix> {
        if rowid > i64::MAX as u64 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "messages_cursor",
                reason: "cursor de mensagens fora da faixa".to_owned(),
            }));
        }
        let agora = self.0.now();
        let texto_org = org.to_string();
        let cursor = rowid as i64;
        let id = mensagem.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado (org_id, messages_cursor, messages_cursor_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (org_id) DO UPDATE SET
                     messages_cursor = excluded.messages_cursor,
                     messages_cursor_id = excluded.messages_cursor_id,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at",
                params![texto_org, cursor, id, agora],
            )
            .map(|_| ())
            .map_err(|erro| StoreError::from_write("messages_cursor", erro))
        })?;
        self.ler_cursores(org)
    }

    /// Avança o cursor de conversas **só para a frente** (compara o par instante + id).
    pub fn avancar_conversas(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> Resultado<CursoresMatrix> {
        if atualizado_em < 0 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "conversations_cursor_at",
                reason: "instante do cursor de conversas não pode ser negativo".to_owned(),
            }));
        }
        let agora = self.0.now();
        let texto_org = org.to_string();
        let conversa = id.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado
                    (org_id, conversations_cursor_at, conversations_cursor_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (org_id) DO UPDATE SET
                     conversations_cursor_at = excluded.conversations_cursor_at,
                     conversations_cursor_id = excluded.conversations_cursor_id,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at
                 WHERE matrix_estado.conversations_cursor_at < excluded.conversations_cursor_at
                    OR (matrix_estado.conversations_cursor_at = excluded.conversations_cursor_at
                        AND matrix_estado.conversations_cursor_id < excluded.conversations_cursor_id)",
                params![texto_org, atualizado_em, conversa, agora],
            )
            .map(|_| ())
            .map_err(|erro| StoreError::from_write("conversations_cursor_at", erro))
        })?;
        self.ler_cursores(org)
    }

    // ─────────────────────────── Falhas ───────────────────────────

    /// Registra a falha definitiva de uma mensagem.
    ///
    /// Idempotente por mensagem: repetir atualiza `motivo`, `tentativas` e o carimbo de tempo em vez
    /// de criar outra linha. `motivo` é o código curto do erro (`M_UNKNOWN`,
    /// `envio_rejeitado`…), nunca o texto do cliente.
    pub fn registrar_falha(
        &self,
        org: OrgId,
        mensagem: MessageId,
        motivo: &str,
        tentativas: u32,
    ) -> Resultado<FalhaMatrix> {
        exigir_texto("motivo", "motivo", motivo, LIMITE_MOTIVO_FALHA)?;
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_mensagem = mensagem.to_string();
        let codigo = motivo.to_owned();
        let tentativas = i64::from(tentativas.max(1));
        let gravada = self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_falhas
                    (id, org_id, message_id, motivo, tentativas, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT (org_id, message_id) DO UPDATE SET
                     motivo = excluded.motivo,
                     tentativas = excluded.tentativas,
                     updated_at = excluded.updated_at",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    texto_org,
                    texto_mensagem,
                    codigo,
                    tentativas,
                    agora
                ],
            )
            .map_err(|erro| StoreError::from_write("motivo", erro))?;
            falha_em(conn, &texto_org, &texto_mensagem)
        })?;
        gravada.ok_or_else(|| {
            StoreError::Domain(DomainError::NotFound {
                entity: "falha do Matrix",
                id: mensagem.to_string(),
            })
        })
    }

    /// Quantas mensagens estão isoladas por falha definitiva na org.
    pub fn contar_falhas(&self, org: OrgId) -> Resultado<u64> {
        let texto_org = org.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT count(*) FROM matrix_falhas WHERE org_id = ?1",
                params![texto_org],
                |linha| u64_da_coluna(linha, 0),
            )
            .map_err(StoreError::from)
        })
    }

    /// Falhas da org, das mais recentes para as mais antigas.
    pub fn listar_falhas(&self, org: OrgId, limite: u32) -> Resultado<Vec<FalhaMatrix>> {
        let texto_org = org.to_string();
        let limite = i64::from(limite.clamp(1, LIMITE_FALHAS));
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_FALHA} FROM matrix_falhas WHERE org_id = ?1
                  ORDER BY updated_at DESC, message_id LIMIT ?2"
            ))?;
            let linhas = consulta.query_map(params![texto_org, limite], linha_falha)?;
            let mut falhas = Vec::new();
            for linha in linhas {
                falhas.push(linha?);
            }
            Ok(falhas)
        })
    }

    /// Tira a mensagem da lista de falhas (quando ela é reprocessada com sucesso).
    ///
    /// Devolve `true` quando havia linha para apagar.
    pub fn remover_falha(&self, org: OrgId, mensagem: MessageId) -> Resultado<bool> {
        let texto_org = org.to_string();
        let texto_mensagem = mensagem.to_string();
        let apagou = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM matrix_falhas WHERE org_id = ?1 AND message_id = ?2",
                params![texto_org, texto_mensagem],
            )
            .map_err(StoreError::from)
        })?;
        Ok(apagou > 0)
    }
}

const COLUNAS_SALA: &str = "conversation_id, org_id, room_id, space_room_id, last_status, \
     client_name_espelhado, state, created_at, updated_at";
const COLUNAS_ESPACO: &str =
    "id, org_id, department_id, space_room_id, alias, origin, created_at, updated_at";
const COLUNAS_FALHA: &str = "id, org_id, message_id, motivo, tentativas, created_at, updated_at";

fn uuid_da_coluna(linha: &rusqlite::Row<'_>, indice: usize) -> rusqlite::Result<uuid::Uuid> {
    let texto: String = linha.get(indice)?;
    texto.parse().map_err(|erro: uuid::Error| {
        rusqlite::Error::FromSqlConversionFailure(
            indice,
            rusqlite::types::Type::Text,
            Box::new(erro),
        )
    })
}

fn linha_estado_org(linha: &rusqlite::Row<'_>) -> rusqlite::Result<EstadoMatrixOrg> {
    Ok(EstadoMatrixOrg {
        org_id: id_da_coluna(linha, 0)?,
        ligado: linha.get::<_, i64>(1)? != 0,
        ligado_em: linha.get(2)?,
        atualizado_em: linha.get(3)?,
    })
}

fn linha_sala(linha: &rusqlite::Row<'_>) -> rusqlite::Result<SalaMatrix> {
    let ultimo_status: Option<String> = linha.get(4)?;
    let ultimo_status = match ultimo_status {
        Some(texto) => Some(ConversationStatus::from_text(&texto).map_err(|erro| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(erro),
            )
        })?),
        None => None,
    };
    Ok(SalaMatrix {
        conversa: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        room_id: linha.get(2)?,
        espaco: linha.get(3)?,
        ultimo_status,
        nome_cliente_espelhado: linha.get(5)?,
        estado: EstadoSala::de_texto(&linha.get::<_, String>(6)?)?,
        criada_em: linha.get(7)?,
        atualizada_em: linha.get(8)?,
    })
}

fn linha_espaco(linha: &rusqlite::Row<'_>) -> rusqlite::Result<EspacoMatrix> {
    Ok(EspacoMatrix {
        id: uuid_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        departamento: id_da_coluna(linha, 2)?,
        room_id: linha.get(3)?,
        alias: linha.get(4)?,
        origem: OrigemEspaco::de_texto(&linha.get::<_, String>(5)?)?,
        criada_em: linha.get(6)?,
        atualizada_em: linha.get(7)?,
    })
}

fn linha_falha(linha: &rusqlite::Row<'_>) -> rusqlite::Result<FalhaMatrix> {
    Ok(FalhaMatrix {
        id: uuid_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        message_id: id_da_coluna(linha, 2)?,
        motivo: linha.get(3)?,
        tentativas: u32_da_coluna(linha, 4)?,
        criada_em: linha.get(5)?,
        atualizada_em: linha.get(6)?,
    })
}

fn sala_em(
    conn: &Connection,
    texto_org: &str,
    texto_conversa: &str,
) -> Resultado<Option<SalaMatrix>> {
    conn.query_row(
        &format!(
            "SELECT {COLUNAS_SALA} FROM matrix_rooms
              WHERE org_id = ?1 AND conversation_id = ?2"
        ),
        params![texto_org, texto_conversa],
        linha_sala,
    )
    .optional()
    .map_err(StoreError::from)
}

fn espaco_em(
    conn: &Connection,
    texto_org: &str,
    texto_departamento: &str,
) -> Resultado<Option<EspacoMatrix>> {
    conn.query_row(
        &format!(
            "SELECT {COLUNAS_ESPACO} FROM matrix_spaces
              WHERE org_id = ?1 AND department_id = ?2"
        ),
        params![texto_org, texto_departamento],
        linha_espaco,
    )
    .optional()
    .map_err(StoreError::from)
}

fn falha_em(
    conn: &Connection,
    texto_org: &str,
    texto_mensagem: &str,
) -> Resultado<Option<FalhaMatrix>> {
    conn.query_row(
        &format!(
            "SELECT {COLUNAS_FALHA} FROM matrix_falhas
              WHERE org_id = ?1 AND message_id = ?2"
        ),
        params![texto_org, texto_mensagem],
        linha_falha,
    )
    .optional()
    .map_err(StoreError::from)
}

fn cursores_em(conn: &Connection, texto_org: &str) -> Resultado<Option<CursoresMatrix>> {
    conn.query_row(
        "SELECT org_id, messages_cursor, conversations_cursor_at, conversations_cursor_id,
                version, updated_at, messages_cursor_id
           FROM matrix_estado WHERE org_id = ?1",
        params![texto_org],
        |linha| {
            Ok(CursoresMatrix {
                org_id: id_da_coluna(linha, 0)?,
                mensagens: u64_da_coluna(linha, 1)?,
                mensagens_id: linha.get(6)?,
                conversas: CursorConversas {
                    atualizado_em: linha.get(2)?,
                    id: linha.get(3)?,
                },
                versao: u64_da_coluna(linha, 4)?,
                atualizado_em: linha.get(5)?,
            })
        },
    )
    .optional()
    .map_err(StoreError::from)
}

fn sala_ausente(texto_conversa: &str) -> StoreError {
    StoreError::Domain(DomainError::NotFound {
        entity: "sala do Matrix",
        id: texto_conversa.to_owned(),
    })
}

/// Recusa texto vazio (ou acima do teto da coluna) antes de chegar ao SQLite, com o campo certo.
fn exigir_texto(
    entidade: &'static str,
    campo: &'static str,
    valor: &str,
    teto: usize,
) -> Resultado<()> {
    let tamanho = valor.chars().count();
    if valor.trim().is_empty() {
        return Err(StoreError::Domain(DomainError::Validation {
            field: campo,
            reason: format!("{entidade} não pode ficar vazia"),
        }));
    }
    if tamanho > teto {
        return Err(StoreError::Domain(DomainError::Validation {
            field: campo,
            reason: format!("{entidade} passa de {teto} caracteres ({tamanho})"),
        }));
    }
    Ok(())
}

// ─────────────── Aviso de falha de entrega da resposta humana (MX-19) ───────────────

/// Desfecho do aviso de uma falha de resposta humana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesfechoAviso {
    /// O aviso entrou na sala.
    Avisado,
    /// A conversa não tem sala: não há onde avisar.
    SemSala,
    /// O Synapse recusou o aviso de vez: não será repetido.
    Recusado,
}

impl DesfechoAviso {
    /// Texto gravado na coluna `desfecho`.
    pub fn como_texto(self) -> &'static str {
        match self {
            DesfechoAviso::Avisado => "avisado",
            DesfechoAviso::SemSala => "sem_sala",
            DesfechoAviso::Recusado => "recusado",
        }
    }
}

/// Falha definitiva de uma resposta humana que ainda não teve desfecho de aviso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalhaDeRespostaHumana {
    /// Org dona da falha.
    pub org_id: OrgId,
    /// Item de saída que falhou.
    pub outbox_id: OutboxId,
    /// Conversa da resposta.
    pub conversa: ConversationId,
    /// Último erro gravado pela fila de saída. Só serve para classificar o motivo: não vai à sala
    /// nem ao log, porque pode carregar dado do canal.
    pub ultimo_erro: Option<String>,
}

impl Matrix<'_> {
    /// Falhas definitivas de resposta humana, sem desfecho de aviso, das orgs com o módulo ligado.
    ///
    /// Uma resposta humana é um item de saída cuja mensagem (`origin = 'human'`) tem o mesmo
    /// `turn_id`. Quem está em `failed` e ainda não tem linha em `matrix_avisos_falha` aparece
    /// aqui. Itens em `sending` (falha transitória) não aparecem: não são definitivos.
    pub fn falhas_de_resposta_humana(
        &self,
        org: OrgId,
        limite: u32,
    ) -> Resultado<Vec<FalhaDeRespostaHumana>> {
        let texto_org = org.to_string();
        let limite = i64::from(limite.clamp(1, 500));
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT o.id, o.conversation_id, o.last_error FROM outbox o
                  WHERE o.org_id = ?1 AND o.state = 'failed'
                    AND EXISTS (SELECT 1 FROM messages m
                                 WHERE m.org_id = o.org_id AND m.turn_id = o.turn_id
                                   AND m.origin = 'human')
                    AND NOT EXISTS (SELECT 1 FROM matrix_avisos_falha a
                                     WHERE a.org_id = o.org_id AND a.outbox_id = o.id)
                  ORDER BY o.created_at, o.id LIMIT ?2",
            )?;
            let linhas = consulta.query_map(params![texto_org, limite], |linha| {
                Ok(FalhaDeRespostaHumana {
                    org_id: org,
                    outbox_id: id_da_coluna(linha, 0)?,
                    conversa: id_da_coluna(linha, 1)?,
                    ultimo_erro: linha.get(2)?,
                })
            })?;
            let mut falhas = Vec::new();
            for linha in linhas {
                falhas.push(linha?);
            }
            Ok(falhas)
        })
    }

    /// Grava o desfecho do aviso de uma falha. Idempotente: a segunda chamada não muda nada e
    /// devolve `false`. Devolve `true` quando a linha é nova. Item de outra org (ou inexistente)
    /// também devolve `false` e não grava nada.
    pub fn registrar_desfecho_de_aviso(
        &self,
        org: OrgId,
        falha: OutboxId,
        desfecho: DesfechoAviso,
        motivo: &str,
    ) -> Resultado<bool> {
        exigir_texto("motivo do aviso", "motivo", motivo, 64)?;
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_falha = falha.to_string();
        let texto_desfecho = desfecho.como_texto();
        let motivo = motivo.to_owned();
        self.0.write(move |conn| {
            // Só grava quando o item de saída é mesmo da org: o id é global, então a org vem
            // conferida aqui, e não só pela chamada.
            conn.execute(
                "INSERT INTO matrix_avisos_falha
                    (org_id, outbox_id, desfecho, motivo, registrado_em)
                 SELECT ?1, ?2, ?3, ?4, ?5 FROM outbox WHERE id = ?2 AND org_id = ?1
                 ON CONFLICT (org_id, outbox_id) DO NOTHING",
                params![texto_org, texto_falha, texto_desfecho, motivo, agora],
            )
            .map(|linhas| linhas > 0)
            .map_err(StoreError::from)
        })
    }
}

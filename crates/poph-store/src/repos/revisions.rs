use poph_types::{AuditId, DomainError, FlowId, OrgId, RevisionId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{belongs, column_id, column_u32, column_u64, conflict_or_missing, require};
use crate::Store;
use crate::backup_document::sha256_hex;
use crate::error::{Resultado, StoreError};
use crate::model::{FlowRevision, RevisionPublication};

const COLUMNS: &str = "id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
    schema_versao, publicada_em, publicada_por, resolucoes_json";

pub const ACTION_PUBLISH: &str = "flow_publish";

pub const ACTORS_OF_AUDIT: [&str; 5] = ["admin_token", "system", "plugin", "cli", "matrix_user"];

pub const LIMIT_REVISIONS: u32 = 50;

pub struct Revisions<'a>(pub(crate) &'a Store);

impl Revisions<'_> {
    pub fn publish_revision(
        &self,
        org: OrgId,
        flow: FlowId,
        publication: RevisionPublication,
    ) -> Resultado<FlowRevision> {
        if publication.fonte.trim().is_empty() {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "fonte",
                reason: "o texto publicado não pode ficar vazio: é a origem que a revisão guarda"
                    .to_owned(),
            }));
        }
        let Some(schema_versao) = publication.schema_versao else {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "schema_versao",
                reason:
                    "informe a versão do esquema da IR (`docs/FLUXOS_LINGUAGEM.md` §8): revisão \
                         sem ela não entra"
                        .to_owned(),
            }));
        };
        if schema_versao == 0 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "schema_versao",
                reason: "a versão do esquema começa em 1".to_owned(),
            }));
        }
        let definicao_json = check_ir(&publication.definicao_json, schema_versao)?;
        let resolucoes_json = check_resolutions(&publication.resolucoes_json)?;
        let ator_tipo = check_actor(&publication.ator_tipo)?;
        let actor_id = publication
            .actor_id
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty());

        let fonte = publication.fonte;
        let fonte_sha256 = sha256_hex(fonte.as_bytes());
        let expected = publication.expected;
        let expected_i64 = i64::try_from(expected).unwrap_or(i64::MAX);
        let now = self.0.now();
        let text_org = org.to_string();
        let text_flow = flow.to_string();

        self.0.write(move |conn| {
            let advanced = conn
                .execute(
                    "UPDATE fluxos
                        SET revisao_atual = COALESCE(revisao_atual, 0) + 1,
                            rascunho_fonte = ?4,
                            version = version + 1
                      WHERE org_id = ?1 AND id = ?2 AND version = ?3",
                    params![text_org, text_flow, expected_i64, fonte],
                )
                .map_err(|error| StoreError::from_write("rascunho_fonte", error))?;
            if advanced == 0 {
                return Err(conflict_or_missing(
                    conn, "fluxos", "fluxo", &text_flow, &text_org, expected,
                ));
            }
            let numero: i64 = conn
                .query_row(
                    "SELECT revisao_atual FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_flow],
                    |row| row.get(0),
                )
                .map_err(StoreError::from)?;

            let revisao_id = RevisionId::generate();
            let detail = serde_json::json!({
                "revisao": numero,
                "revisao_id": revisao_id.to_string(),
                "schema_versao": schema_versao,
                "fonte_sha256": fonte_sha256,
            });
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'flow', ?6, ?7, ?8)",
                params![
                    AuditId::generate().to_string(),
                    text_org,
                    ator_tipo,
                    actor_id,
                    ACTION_PUBLISH,
                    text_flow,
                    detail.to_string(),
                    now
                ],
            )
            .map_err(|error| {
                StoreError::from_write_with("ator_tipo", &[("actor_kind", "ator_tipo")], error)
            })?;

            conn.execute(
                "INSERT INTO fluxo_revisoes
                    (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                     schema_versao, publicada_em, publicada_por, resolucoes_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    revisao_id.to_string(),
                    text_org,
                    text_flow,
                    numero,
                    fonte,
                    fonte_sha256,
                    definicao_json,
                    i64::from(schema_versao),
                    now,
                    ator_tipo,
                    resolucoes_json
                ],
            )
            .map_err(|error| StoreError::from_write("numero", error))?;

            read_by_number(conn, &text_org, &text_flow, numero)?.ok_or_else(|| {
                StoreError::Internal("a revisão foi gravada e não voltou na leitura".to_owned())
            })
        })
    }

    pub fn get(&self, org: OrgId, flow: FlowId, numero: u64) -> Resultado<FlowRevision> {
        let text_org = org.to_string();
        let text_flow = flow.to_string();
        let number_i64 = i64::try_from(numero).unwrap_or(i64::MAX);
        let found = self.0.read(move |conn| {
            belongs(conn, "fluxos", "fluxo", &text_org, &text_flow)?;
            read_by_number(conn, &text_org, &text_flow, number_i64)
        })?;
        require(found, "revisão de fluxo", &numero.to_string())
    }

    pub fn get_by_id(&self, org: OrgId, revision: RevisionId) -> Resultado<FlowRevision> {
        let text_org = org.to_string();
        let text_revision = revision.to_string();
        let found = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM fluxo_revisoes WHERE org_id = ?1 AND id = ?2"),
                params![text_org, text_revision],
                revision_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        require(found, "revisão de fluxo", &revision.to_string())
    }

    pub fn current(&self, org: OrgId, flow: FlowId) -> Resultado<Option<FlowRevision>> {
        let text_org = org.to_string();
        let text_flow = flow.to_string();
        self.0.read(move |conn| {
            let numero: Option<Option<i64>> = conn
                .query_row(
                    "SELECT revisao_atual FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_flow],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            match numero {
                None => Err(StoreError::Domain(DomainError::NotFound {
                    entity: "fluxo",
                    id: text_flow.clone(),
                })),
                Some(None) => Ok(None),
                Some(Some(numero)) => read_by_number(conn, &text_org, &text_flow, numero)?
                    .map_or_else(
                        || {
                            Err(StoreError::Internal(format!(
                                "o fluxo {text_flow} aponta para a revisão {numero}, que não \
                                 existe"
                            )))
                        },
                        |revision| Ok(Some(revision)),
                    ),
            }
        })
    }

    pub fn list(
        &self,
        org: OrgId,
        flow: FlowId,
        after: Option<u64>,
        limit: u32,
    ) -> Resultado<Vec<FlowRevision>> {
        let text_org = org.to_string();
        let text_flow = flow.to_string();
        let after = after.map(|numero| i64::try_from(numero).unwrap_or(i64::MAX));
        let limit = i64::from(limit.clamp(1, 500));
        self.0.read(move |conn| {
            belongs(conn, "fluxos", "fluxo", &text_org, &text_flow)?;
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM fluxo_revisoes
                      WHERE org_id = ?1 AND fluxo_id = ?2 AND (?3 IS NULL OR numero > ?3)
                      ORDER BY numero LIMIT ?4"
                ))
                .map_err(StoreError::from)?;
            let rows = statement
                .query_map(params![text_org, text_flow, after, limit], revision_row)
                .map_err(StoreError::from)?;
            let mut revisions = Vec::new();
            for row in rows {
                revisions.push(row.map_err(StoreError::from)?);
            }
            Ok(revisions)
        })
    }
}

fn read_by_number(
    conn: &Connection,
    org: &str,
    flow: &str,
    numero: i64,
) -> Resultado<Option<FlowRevision>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM fluxo_revisoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND numero = ?3"
        ),
        params![org, flow, numero],
        revision_row,
    )
    .optional()
    .map_err(StoreError::from)
}

fn check_ir(definicao_json: &str, schema_versao: u32) -> Resultado<String> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(definicao_json) else {
        return Err(StoreError::Domain(DomainError::Validation {
            field: "definicao_json",
            reason: "a IR entra como objeto JSON já validado por quem chama".to_owned(),
        }));
    };
    if let Some(declared) = map.get("schema_versao") {
        let declared = declared.as_u64();
        if declared != Some(u64::from(schema_versao)) {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "schema_versao",
                reason: format!(
                    "a IR declara {declared:?} e a publicação informa {schema_versao}: os dois têm \
                     de ser a mesma versão"
                ),
            }));
        }
    }
    Ok(definicao_json.to_owned())
}

fn check_resolutions(resolucoes_json: &str) -> Resultado<String> {
    let texto = resolucoes_json.trim();
    if texto.is_empty() {
        return Ok("{}".to_owned());
    }
    if !matches!(
        serde_json::from_str::<serde_json::Value>(texto),
        Ok(serde_json::Value::Object(_))
    ) {
        return Err(StoreError::Domain(DomainError::Validation {
            field: "resolucoes_json",
            reason: "as resoluções entram como objeto JSON (ex.: nome de departamento → id)"
                .to_owned(),
        }));
    }
    Ok(texto.to_owned())
}

fn check_actor(ator_tipo: &str) -> Resultado<String> {
    let actor = ator_tipo.trim();
    if ACTORS_OF_AUDIT.contains(&actor) {
        return Ok(actor.to_owned());
    }
    Err(StoreError::Domain(DomainError::Validation {
        field: "ator_tipo",
        reason: format!(
            "use um destes tipos de ator: {}",
            ACTORS_OF_AUDIT.join(", ")
        ),
    }))
}

fn revision_row(row: &Row<'_>) -> rusqlite::Result<FlowRevision> {
    Ok(FlowRevision {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        fluxo_id: column_id(row, 2)?,
        numero: column_u64(row, 3)?,
        fonte: row.get(4)?,
        fonte_sha256: row.get(5)?,
        definicao_json: row.get(6)?,
        schema_versao: column_u32(row, 7)?,
        publicada_em: row.get(8)?,
        publicada_por: row.get(9)?,
        resolucoes_json: row.get(10)?,
    })
}

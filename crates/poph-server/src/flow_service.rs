use std::fmt;
use std::sync::Arc;

use poph_flows::{
    Compiled, Departamento, Ir, Limites, RevisionKey, compile, diff, format_with,
    resolve_departments, stage_block, validar,
};

pub use poph_flows::{Change, Diagnostic, Resolution, RevisionCache};
use poph_store::{
    FlowRevision, Fluxo, NewAuditEntry, NewFlow, Page, RevisionPublication, Store, StoreError,
};
use poph_types::{DomainError, FlowId, HarnessConfig, OrgId};
use serde_json::{Value, json};

const DEPARTMENT_LIMIT: u32 = 200;

#[derive(Debug, Clone)]
pub struct Actor {
    pub kind: String,
    pub id: Option<String>,
}

impl Actor {
    pub fn token(id: String) -> Self {
        Self {
            kind: "admin_token".to_owned(),
            id: Some(id),
        }
    }

    pub fn cli() -> Self {
        Self {
            kind: "cli".to_owned(),
            id: None,
        }
    }
}

#[derive(Debug)]
pub enum FlowError {
    StaleVersion {
        id: String,
        expected: u64,
        current: u64,
    },
    EmptyScript,
    InvalidScript(Vec<Diagnostic>),
    MissingRevision,
    DivergentIr(&'static str),
    Internal,
    FlowLimit {
        current: u64,
        cap: u64,
    },
    Database(StoreError),
}

impl From<StoreError> for FlowError {
    fn from(error: StoreError) -> Self {
        Self::Database(error)
    }
}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleVersion {
                expected, current, ..
            } => write!(
                f,
                "a versão não confere: você informou a versão {expected}, mas o fluxo está na \
                 versão {current}. Leia de novo e repita com a versão atual"
            ),
            Self::EmptyScript => write!(f, "o script está vazio: não há o que publicar"),
            Self::InvalidScript(diagnostics) => {
                let errors = diagnostics.iter().filter(|d| d.is_error()).count();
                write!(
                    f,
                    "o script não passa na validação: {errors} erro(s) a corrigir; nada foi gravado"
                )
            }
            Self::MissingRevision => write!(
                f,
                "o fluxo ainda não tem revisão publicada: publique o rascunho antes"
            ),
            Self::DivergentIr(_) => write!(
                f,
                "a revisão publicada não confere com o texto guardado: a leitura falha fechada"
            ),
            Self::Internal => write!(
                f,
                "erro interno ao ler ou gravar o fluxo (veja o log do servidor)"
            ),
            Self::FlowLimit { current, cap } => {
                write!(f, "a organização já tem {current} fluxos e o teto é {cap}")
            }
            Self::Database(error) => write!(f, "{error}"),
        }
    }
}

#[derive(Debug)]
pub struct Validation {
    pub ir: Option<Ir>,
    pub diagnostics: Vec<Diagnostic>,
    pub resolutions: Vec<Resolution>,
}

impl Validation {
    pub fn is_valid(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

#[derive(Debug)]
pub struct Publication {
    pub revision: FlowRevision,
    pub warnings: Vec<Diagnostic>,
    pub flow_version: u64,
}

#[derive(Debug)]
pub struct Creation {
    pub flow: Fluxo,
    pub publication: Option<Publication>,
}

#[derive(Debug)]
pub struct Comparison {
    pub from: FlowRevision,
    pub to: FlowRevision,
    pub changes: Vec<Change>,
    pub active_from: u64,
    pub active_to: u64,
}

#[derive(Debug)]
pub struct Preview {
    pub stage: String,
    pub revision: u64,
    pub block: String,
}

pub struct FlowOperations<'a> {
    store: &'a Store,
    cache: &'a RevisionCache,
    limits: Limites,
    max_flows_per_org: u64,
}

impl<'a> FlowOperations<'a> {
    pub fn new(store: &'a Store, cache: &'a RevisionCache, config: &HarnessConfig) -> Self {
        Self {
            store,
            cache,
            limits: Limites::from_config(&config.flows),
            max_flows_per_org: u64::from(config.flows.max_flows_per_org),
        }
    }

    pub fn compile(&self, source: &str) -> Compiled {
        compile(source, &self.limits)
    }

    pub fn departments(&self, org: OrgId) -> Result<Vec<Departamento>, StoreError> {
        let mut all: Vec<Departamento> = Vec::new();
        let mut after: Option<uuid::Uuid> = None;
        loop {
            let batch_size = self.store.departments().list(
                org,
                Page {
                    after,
                    limit: DEPARTMENT_LIMIT,
                },
            )?;
            if batch_size.is_empty() {
                return Ok(all);
            }
            let full_page = batch_size.len() >= DEPARTMENT_LIMIT as usize;
            after = batch_size.last().map(|department| department.id.as_uuid());
            all.extend(batch_size.into_iter().map(|department| Departamento {
                id: department.id.to_string(),
                nome: department.name,
            }));
            if !full_page {
                return Ok(all);
            }
        }
    }

    pub fn validate(&self, org: Option<OrgId>, source: &str) -> Result<Validation, StoreError> {
        let compiled = self.compile(source);
        let mut diagnostics = compiled.diagnostics;
        let mut resolutions = Vec::new();
        if let (Some(ir), Some(org)) = (&compiled.ir, org) {
            let (found, warnings) = resolve_departments(ir, &self.departments(org)?);
            diagnostics.extend(warnings);
            resolutions = found;
        }
        Ok(Validation {
            ir: compiled.ir,
            diagnostics,
            resolutions,
        })
    }

    pub fn format(&self, source: &str) -> Result<String, FlowError> {
        format_with(source, &self.limits)
            .map_err(|failure| FlowError::InvalidScript(failure.diagnostics))
    }

    pub fn create(
        &self,
        org: OrgId,
        source: &str,
        publish: bool,
        actor: &Actor,
    ) -> Result<Creation, FlowError> {
        let compiled = self.compile(source);
        if compiled.has_error() {
            return Err(FlowError::InvalidScript(compiled.diagnostics));
        }
        let Some(ir) = compiled.ir else {
            tracing::error!("o compilador devolveu script sem erro e sem IR");
            return Err(FlowError::Internal);
        };
        let today = self.store.flows().count_rows(org)?;
        if today >= self.max_flows_per_org {
            return Err(FlowError::FlowLimit {
                current: today,
                cap: self.max_flows_per_org,
            });
        }
        let flow = self.store.flows().create(
            org,
            NewFlow {
                slug: ir.fluxo.id.clone(),
                name: ir.fluxo.nome.clone(),
                description: ir.fluxo.descricao.clone().unwrap_or_default(),
                rascunho_fonte: source.to_owned(),
            },
        )?;
        audit(
            self.store,
            org,
            actor,
            "flow_create",
            "flow",
            flow.id.to_string(),
        )?;
        let publication = if publish {
            Some(self.publish(org, &flow, flow.version, source, actor)?)
        } else {
            None
        };
        Ok(Creation { flow, publication })
    }

    pub fn publish(
        &self,
        org: OrgId,
        flow: &Fluxo,
        expected: u64,
        source: &str,
        actor: &Actor,
    ) -> Result<Publication, FlowError> {
        if expected != flow.version {
            return Err(FlowError::StaleVersion {
                id: flow.id.to_string(),
                expected,
                current: flow.version,
            });
        }
        if source.trim().is_empty() {
            return Err(FlowError::EmptyScript);
        }
        let compiled = self.compile(source);
        if compiled.has_error() {
            return Err(FlowError::InvalidScript(compiled.diagnostics));
        }
        let Some(ir) = compiled.ir else {
            tracing::error!("o compilador devolveu script sem erro e sem IR");
            return Err(FlowError::Internal);
        };
        let ir_errors: Vec<Diagnostic> =
            validar(&ir).into_iter().filter(|d| d.is_error()).collect();
        if !ir_errors.is_empty() {
            return Err(FlowError::InvalidScript(ir_errors));
        }
        let (resolutions, department_warnings) = resolve_departments(&ir, &self.departments(org)?);
        let definition_json = serde_json::to_string(&ir).map_err(|error| {
            tracing::error!(error = %error, "a IR compilada não serializa");
            FlowError::Internal
        })?;
        let resolutions_json = serde_json::to_string(&json!({ "departamentos": resolutions }))
            .map_err(|_| FlowError::Internal)?;
        let revision = self.store.revisions().publish_revision(
            org,
            flow.id,
            RevisionPublication {
                expected,
                fonte: source.to_owned(),
                definicao_json: definition_json,
                schema_versao: Some(ir.schema_versao),
                resolucoes_json: resolutions_json,
                ator_tipo: actor.kind.clone(),
                actor_id: actor.id.clone(),
            },
        )?;
        let flow_version = self.store.flows().get(org, flow.id)?.version;
        let mut warnings: Vec<Diagnostic> = compiled
            .diagnostics
            .into_iter()
            .filter(|d| !d.is_error())
            .collect();
        warnings.extend(department_warnings);
        Ok(Publication {
            revision,
            warnings,
            flow_version,
        })
    }

    pub fn revision_ir(&self, flow: &Fluxo, revision: &FlowRevision) -> Result<Arc<Ir>, FlowError> {
        let stored: Ir = serde_json::from_str(&revision.definicao_json).map_err(|error| {
            tracing::error!(
                error = %error,
                revision_id = %revision.id,
                "a IR gravada da revisão não é JSON legível"
            );
            FlowError::Internal
        })?;
        let key = RevisionKey::with_hash(
            flow.slug.clone(),
            u32::try_from(revision.numero).unwrap_or(u32::MAX),
            revision.fonte_sha256.clone(),
        );
        self.cache
            .load(&key, &stored, &revision.fonte, &self.limits)
            .map_err(|failure| {
                tracing::error!(
                    failure = %failure.code(),
                    revision_id = %revision.id,
                    "a revisão publicada não confere com o texto guardado"
                );
                FlowError::DivergentIr(failure.code())
            })
    }

    pub fn compare(
        &self,
        org: OrgId,
        flow: &Fluxo,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Result<Comparison, FlowError> {
        let to = match to {
            Some(number) => number,
            None => flow.revisao_atual.ok_or(FlowError::MissingRevision)?,
        };
        let from = from.unwrap_or_else(|| to.saturating_sub(1).max(1));
        let from_revision = self.store.revisions().get(org, flow.id, from)?;
        let to_revision = self.store.revisions().get(org, flow.id, to)?;
        let from_ir = self.revision_ir(flow, &from_revision)?;
        let to_ir = self.revision_ir(flow, &to_revision)?;
        let changes = diff(&from_ir, &to_ir);
        let counts = self.store.executions().active_by_revision(org, flow.id)?;
        let active_count = |revision: &FlowRevision| -> u64 {
            counts
                .iter()
                .find(|(count_id, _)| *count_id == revision.id)
                .map_or(0, |(_, count)| *count)
        };
        Ok(Comparison {
            active_from: active_count(&from_revision),
            active_to: active_count(&to_revision),
            from: from_revision,
            to: to_revision,
            changes,
        })
    }

    pub fn preview(
        &self,
        org: OrgId,
        flow: &Fluxo,
        stage: Option<&str>,
    ) -> Result<Preview, FlowError> {
        let revision = self
            .store
            .revisions()
            .current(org, flow.id)?
            .ok_or(FlowError::MissingRevision)?;
        let ir = self.revision_ir(flow, &revision)?;
        let stage = stage
            .filter(|text| !text.is_empty())
            .map_or_else(|| ir.start.clone(), str::to_owned);
        let block = stage_block(&ir, &stage)
            .map_err(|error: DomainError| FlowError::Database(StoreError::Domain(error)))?;
        Ok(Preview {
            stage,
            revision: revision.numero,
            block,
        })
    }
}

pub fn audit(
    store: &Store,
    org: OrgId,
    actor: &Actor,
    acao: &str,
    entity: &str,
    id: String,
) -> Result<(), StoreError> {
    store.audit().register(
        Some(org),
        NewAuditEntry {
            ator_tipo: actor.kind.clone(),
            actor_id: actor.id.clone(),
            acao: acao.into(),
            entity: entity.into(),
            entity_id: Some(id),
            detail_json: "{}".into(),
        },
    )?;
    Ok(())
}

pub fn flow_json(f: &Fluxo) -> Value {
    json!({
        "id": f.id.to_string(),
        "slug": f.slug,
        "name": f.name,
        "description": f.description,
        "status": f.status.as_text(),
        "current_revision": f.revisao_atual,
        "version": f.version,
    })
}

pub fn flow_with_draft_json(f: &Fluxo) -> Value {
    let mut body = flow_json(f);
    body["draft_source"] = json!(f.rascunho_fonte);
    body
}

pub fn revision_summary_json(r: &FlowRevision) -> Value {
    json!({
        "id": r.id.to_string(),
        "flow_id": r.fluxo_id.to_string(),
        "number": r.numero,
        "schema_version": r.schema_versao,
        "source_sha256": r.fonte_sha256,
        "published_at": r.publicada_em,
        "published_by": r.publicada_por,
        "resolutions": serde_json::from_str::<Value>(&r.resolucoes_json).unwrap_or(Value::Null),
    })
}

pub fn full_revision_json(r: &FlowRevision) -> Value {
    let mut body = revision_summary_json(r);
    body["source"] = json!(r.fonte);
    body["definition"] = serde_json::from_str::<Value>(&r.definicao_json).unwrap_or(Value::Null);
    body
}

pub fn validation_json(v: &Validation) -> Value {
    json!({
        "valid": v.is_valid(),
        "ir": v.ir,
        "diagnostics": v.diagnostics,
        "resolutions": v.resolutions,
    })
}

pub fn comparison_json(flow: FlowId, c: &Comparison) -> Value {
    json!({
        "flow_id": flow.to_string(),
        "from": { "revision": c.from.numero, "active_runs": c.active_from },
        "to": { "revision": c.to.numero, "active_runs": c.active_to },
        "changes": c.changes,
    })
}

pub fn preview_json(p: &Preview) -> Value {
    json!({
        "stage": p.stage,
        "revision": p.revision,
        "prompt": p.block,
    })
}

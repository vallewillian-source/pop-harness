use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use poph_types::{DomainError, OrgId};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde_json::{Map, Value};

use crate::Store;
use crate::error::{Resultado, StoreError};

pub const BACKUP_FORMAT: &str = "poph-backup-org";
pub const VERSAO_FORMATO: u32 = 2;

struct Table {
    name: &'static str,
    key: &'static [&'static str],
}

const ALWAYS_TABLES: &[Table] = &[
    Table {
        name: "product_categories",
        key: &["id"],
    },
    Table {
        name: "departments",
        key: &["id"],
    },
    Table {
        name: "products",
        key: &["id"],
    },
    Table {
        name: "agents",
        key: &["id"],
    },
    Table {
        name: "agent_product_categories",
        key: &["agent_id", "category_id"],
    },
    Table {
        name: "agent_instructions",
        key: &["agent_id"],
    },
    Table {
        name: "knowledge_groups",
        key: &["id"],
    },
    Table {
        name: "knowledge_items",
        key: &["id"],
    },
    Table {
        name: "agent_knowledge_groups",
        key: &["agent_id", "group_id"],
    },
    Table {
        name: "file_library",
        key: &["id"],
    },
    Table {
        name: "integrations",
        key: &["id"],
    },
    Table {
        name: "fluxos",
        key: &["id"],
    },
    Table {
        name: "fluxo_revisoes",
        key: &["id"],
    },
    Table {
        name: "agente_fluxos",
        key: &["agent_id", "fluxo_id"],
    },
];

const CONVERSATION_TABLES: &[Table] = &[
    Table {
        name: "conversations",
        key: &["id"],
    },
    Table {
        name: "messages",
        key: &["id"],
    },
    Table {
        name: "attachments",
        key: &["id"],
    },
    Table {
        name: "turns",
        key: &["id"],
    },
    Table {
        name: "outbox",
        key: &["id"],
    },
    Table {
        name: "inbox",
        key: &["id"],
    },
    Table {
        name: "carts",
        key: &["id"],
    },
    Table {
        name: "cart_items",
        key: &["id"],
    },
    Table {
        name: "fichas",
        key: &["id"],
    },
    Table {
        name: "ficha_eventos",
        key: &["id"],
    },
    Table {
        name: "fluxo_execucoes",
        key: &["id"],
    },
    Table {
        name: "fluxo_execucao_etapas",
        key: &["id"],
    },
    Table {
        name: "fluxo_efeitos",
        key: &["id"],
    },
    Table {
        name: "site_conversation_cursors",
        key: &["conversation_id"],
    },
    Table {
        name: "site_turns",
        key: &["conversation_id", "id"],
    },
    Table {
        name: "site_events",
        key: &["conversation_id", "seq"],
    },
    Table {
        name: "adapter_conversations",
        key: &["id"],
    },
    Table {
        name: "adapter_message_keys",
        key: &["conversation_id", "idempotency_key"],
    },
];

struct NaturalKey {
    table: &'static str,
    column: &'static str,
    resource: &'static str,
}

const NATURAL_KEYS: &[NaturalKey] = &[
    NaturalKey {
        table: "fluxos",
        column: "slug",
        resource: "fluxo",
    },
    NaturalKey {
        table: "agents",
        column: "slug",
        resource: "agente",
    },
    NaturalKey {
        table: "products",
        column: "slug",
        resource: "produto",
    },
    NaturalKey {
        table: "product_categories",
        column: "name",
        resource: "categoria",
    },
    NaturalKey {
        table: "departments",
        column: "name",
        resource: "departamento",
    },
    NaturalKey {
        table: "knowledge_groups",
        column: "name",
        resource: "grupo de conhecimento",
    },
];

#[derive(Clone)]
pub struct OrgSnapshot {
    pub org: Value,
    pub tables: BTreeMap<String, Vec<Value>>,
    pub encrypted_secrets: BTreeMap<String, Vec<u8>>,
}

impl fmt::Debug for OrgSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrgSnapshot")
            .field("org", &self.org)
            .field("tabelas", &self.tables)
            .field(
                "segredos_cifrados",
                &format_args!(
                    "{} integração(ões) com segredo",
                    self.encrypted_secrets.len()
                ),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportTarget {
    Origin,
    Existing(OrgId),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoredTable {
    pub inserted: u64,
    pub updated: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub org: OrgId,
    pub created_org: bool,
    pub tables: BTreeMap<String, StoredTable>,
    pub secrets_to_reenter: u64,
}

impl ImportReport {
    pub fn total(&self) -> u64 {
        self.tables.values().map(|t| t.inserted + t.updated).sum()
    }
}

pub const MAX_UPDATE_EXAMPLES: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportUpdates {
    pub tables: BTreeMap<String, u64>,
    pub total: u64,
    pub examples: Vec<String>,
}

impl ImportUpdates {
    pub fn empty(&self) -> bool {
        self.total == 0
    }

    pub fn summary(&self) -> String {
        self.tables
            .iter()
            .map(|(table, rows)| format!("{table} {rows}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn register(&mut self, table: &str, row: &Value) {
        *self.tables.entry(table.to_owned()).or_insert(0) += 1;
        self.total += 1;
        if self.examples.len() < MAX_UPDATE_EXAMPLES {
            self.examples
                .push(format!("{table}: {}", row_identifier(row)));
        }
    }
}

impl Store {
    pub fn org_snapshot(&self, org: OrgId, com_conversas: bool) -> Resultado<OrgSnapshot> {
        let tables: Vec<&'static Table> = ALWAYS_TABLES
            .iter()
            .chain(CONVERSATION_TABLES.iter().filter(|_| com_conversas))
            .collect();
        self.read(move |conn| {
            let org_row = read_org(conn, org)?;
            let mut snapshot = BTreeMap::new();
            for table in tables {
                let rows = read_table(conn, table.name, org)?;
                if !rows.is_empty() || !table.name.starts_with("site_") {
                    snapshot.insert(table.name.to_owned(), rows);
                }
            }
            let encrypted_secrets = read_encrypted_secrets(conn, org)?;
            Ok(OrgSnapshot {
                org: org_row,
                tables: snapshot,
                encrypted_secrets,
            })
        })
    }

    pub fn import_snapshot(
        &self,
        snapshot: OrgSnapshot,
        target: ImportTarget,
    ) -> Resultado<ImportReport> {
        self.write(move |conn| import(conn, &snapshot, target))
    }

    pub fn import_protected_snapshot(
        &self,
        snapshot: OrgSnapshot,
        target: ImportTarget,
        sobrescrever: bool,
    ) -> Resultado<std::result::Result<ImportReport, ImportUpdates>> {
        self.write(move |conn| {
            let (org, created) = import_destination(conn, &snapshot, target)?;
            check_natural_keys(conn, &snapshot, org, created)?;
            if !sobrescrever {
                let updates = import_updates(conn, &snapshot, target)?;
                if !updates.empty() {
                    return Ok(Err(updates));
                }
            }
            import(conn, &snapshot, target).map(Ok)
        })
    }

    pub fn check_import_conflicts(
        &self,
        snapshot: &OrgSnapshot,
        target: ImportTarget,
    ) -> Resultado<Vec<String>> {
        self.read(|conn| {
            let (org_id, created_org) = import_destination(conn, snapshot, target)?;
            natural_key_conflicts(conn, snapshot, org_id, created_org)
        })
    }

    pub fn check_import_updates(
        &self,
        snapshot: &OrgSnapshot,
        target: ImportTarget,
    ) -> Resultado<ImportUpdates> {
        self.read(|conn| import_updates(conn, snapshot, target))
    }
}

fn read_org(conn: &Connection, org: OrgId) -> Resultado<Value> {
    let mut query = conn.prepare("SELECT * FROM orgs WHERE id = ?1")?;
    let columns = column_names(&query);
    let mut rows = query.query([org.to_string()])?;
    match rows.next()? {
        Some(row) => row_json("orgs", &columns, row),
        None => Err(DomainError::NotFound {
            entity: "org",
            id: org.to_string(),
        }
        .into()),
    }
}

fn read_table(conn: &Connection, table: &str, org: OrgId) -> Resultado<Vec<Value>> {
    let sql = if table == "integrations" {
        "SELECT *, secrets_enc IS NOT NULL AS tem_segredo FROM integrations \
         WHERE org_id = ?1 ORDER BY rowid"
            .to_owned()
    } else {
        format!("SELECT * FROM {table} WHERE org_id = ?1 ORDER BY rowid")
    };
    let mut query = conn.prepare(&sql)?;
    let columns = column_names(&query);
    let mut rows = query.query([org.to_string()])?;
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        values.push(row_json(table, &columns, row)?);
    }
    Ok(values)
}

fn read_encrypted_secrets(conn: &Connection, org: OrgId) -> Resultado<BTreeMap<String, Vec<u8>>> {
    let mut query = conn.prepare(
        "SELECT id, secrets_enc FROM integrations \
         WHERE org_id = ?1 AND secrets_enc IS NOT NULL",
    )?;
    let mut rows = query.query([org.to_string()])?;
    let mut map = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let blob: Vec<u8> = row.get(1)?;
        map.insert(id, blob);
    }
    Ok(map)
}

fn column_names(query: &rusqlite::Statement<'_>) -> Vec<String> {
    query.column_names().map(str::to_owned).collect()
}

fn row_json(table: &str, columns: &[String], row: &rusqlite::Row<'_>) -> Resultado<Value> {
    let mut object = Map::new();
    for (index, column) in columns.iter().enumerate() {
        if table == "integrations" && column == "secrets_enc" {
            continue;
        }
        object.insert(
            column.clone(),
            value_json(table, column, row.get_ref(index)?)?,
        );
    }
    if table == "file_library" || table == "attachments" {
        object.remove("blob_path");
    }
    if table == "integrations"
        && let Some(value) = object.get_mut("tem_segredo")
    {
        *value = Value::Bool(value.as_i64().is_some_and(|numero| numero != 0));
    }
    Ok(Value::Object(object))
}

fn value_json(table: &str, column: &str, value: ValueRef<'_>) -> Resultado<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(numero) => Value::from(numero),
        ValueRef::Real(numero) => serde_json::Number::from_f64(numero)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(texto) => {
            let texto = std::str::from_utf8(texto).map_err(|_| {
                StoreError::Internal(format!(
                    "a coluna `{column}` de `{table}` não é texto UTF-8"
                ))
            })?;
            Value::String(texto.to_owned())
        }
        ValueRef::Blob(_) => {
            return Err(StoreError::Internal(format!(
                "a coluna `{column}` de `{table}` é binária e não deveria ir para o backup"
            )));
        }
    })
}

fn import(
    conn: &Connection,
    snapshot: &OrgSnapshot,
    target: ImportTarget,
) -> Resultado<ImportReport> {
    let (org_id, created_org) = import_destination(conn, snapshot, target)?;

    check_natural_keys(conn, snapshot, org_id, created_org)?;

    if matches!(target, ImportTarget::Origin) {
        write_org(conn, &snapshot.org, org_id)?;
    }

    let mut report = ImportReport {
        org: org_id,
        created_org,
        tables: BTreeMap::new(),
        secrets_to_reenter: 0,
    };
    let ordem: Vec<&'static Table> = ALWAYS_TABLES
        .iter()
        .chain(CONVERSATION_TABLES.iter())
        .collect();
    for table in &ordem {
        let Some(rows) = snapshot.tables.get(table.name) else {
            continue;
        };
        let columns = table_columns(conn, table.name)?;
        let mut stored = StoredTable::default();
        for row in rows {
            let updated = write_row(conn, table, org_id, row, &columns)?;
            if updated {
                stored.updated += 1;
            } else {
                stored.inserted += 1;
            }
        }
        if table.name == "integrations" {
            let with_secret = rows
                .iter()
                .filter(|row| row.get("tem_segredo") == Some(&Value::Bool(true)))
                .count() as u64;
            report.secrets_to_reenter += with_secret;
        }
        report.tables.insert(table.name.to_owned(), stored);
    }
    if !snapshot.tables.contains_key("fichas") && snapshot.tables.contains_key("conversations") {
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
            FROM conversations c WHERE c.org_id = '{}' AND c.channel <> 'site' AND NOT EXISTS
                (SELECT 1 FROM fichas f WHERE f.conversation_id = c.id);"#, org_id))?;
    }
    if let Some(conversations) = snapshot.tables.get("conversations") {
        for conversation in conversations {
            if conversation.get("channel").and_then(Value::as_str) == Some("site") {
                continue;
            }
            let id = conversation
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    StoreError::Domain(DomainError::Validation {
                        field: "backup",
                        reason: "conversa sem id".to_owned(),
                    })
                })?;
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM fichas WHERE org_id = ?1 AND conversation_id = ?2)",
                rusqlite::params![org_id.to_string(), id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(StoreError::Domain(DomainError::Validation {
                    field: "backup",
                    reason: "conversa importada sem Ficha correspondente".to_owned(),
                }));
            }
        }
    }
    for name in snapshot.tables.keys() {
        if known_table(name).is_none() {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "backup",
                reason: format!(
                    "o arquivo traz a tabela `{name}`, que não faz parte do formato desta versão"
                ),
            }));
        }
    }
    Ok(report)
}

fn import_destination(
    conn: &Connection,
    snapshot: &OrgSnapshot,
    target: ImportTarget,
) -> Resultado<(OrgId, bool)> {
    let from_origin = row_org(&snapshot.org)?;
    match target {
        ImportTarget::Existing(org) => {
            if !org_exists(conn, org)? {
                return Err(DomainError::NotFound {
                    entity: "org",
                    id: org.to_string(),
                }
                .into());
            }
            Ok((org, false))
        }
        ImportTarget::Origin => {
            let exists = org_exists(conn, from_origin)?;
            Ok((from_origin, !exists))
        }
    }
}

fn row_org(row: &Value) -> Resultado<OrgId> {
    let id = row.get("id").and_then(Value::as_str).ok_or_else(|| {
        StoreError::Domain(DomainError::Validation {
            field: "org",
            reason: "o arquivo não traz o id da org de origem".to_owned(),
        })
    })?;
    id.parse::<OrgId>().map_err(|_| {
        StoreError::Domain(DomainError::Validation {
            field: "org",
            reason: "o id da org no arquivo não é um UUID v7".to_owned(),
        })
    })
}

fn org_exists(conn: &Connection, org: OrgId) -> Resultado<bool> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM orgs WHERE id = ?1",
            [org.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn import_updates(
    conn: &Connection,
    snapshot: &OrgSnapshot,
    target: ImportTarget,
) -> Resultado<ImportUpdates> {
    let (org_id, created_org) = import_destination(conn, snapshot, target)?;
    let mut updates = ImportUpdates::default();
    if matches!(target, ImportTarget::Origin) && !created_org {
        updates.register("orgs", &snapshot.org);
    }
    for table in ALWAYS_TABLES.iter().chain(CONVERSATION_TABLES.iter()) {
        let Some(rows) = snapshot.tables.get(table.name) else {
            continue;
        };
        for row in rows {
            if row_exists_in_file(conn, table, org_id, row)? {
                updates.register(table.name, row);
            }
        }
    }
    Ok(updates)
}

fn row_exists_in_file(
    conn: &Connection,
    table: &Table,
    org_id: OrgId,
    row: &Value,
) -> Resultado<bool> {
    let object = row_object(table.name, row)?;
    let mut values = BTreeMap::new();
    for key in table.key {
        let Some(value) = object.get(*key) else {
            return Ok(false);
        };
        values.insert((*key).to_owned(), sql_value(table.name, key, value)?);
    }
    row_exists(conn, table, &values, Some(org_id))
}

fn row_identifier(row: &Value) -> String {
    for field in ["slug", "name", "id"] {
        if let Some(value) = row.get(field).and_then(Value::as_str) {
            return value.to_owned();
        }
    }
    "(sem identificador)".to_owned()
}

fn known_table(name: &str) -> Option<&'static Table> {
    ALWAYS_TABLES
        .iter()
        .chain(CONVERSATION_TABLES.iter())
        .find(|table| table.name == name)
}

fn natural_key_conflicts(
    conn: &Connection,
    snapshot: &OrgSnapshot,
    org_id: OrgId,
    created_org: bool,
) -> Resultado<Vec<String>> {
    let mut conflicts: Vec<String> = Vec::new();
    'fora: for table in ALWAYS_TABLES.iter().chain(CONVERSATION_TABLES.iter()) {
        if table.key != ["id"] {
            continue;
        }
        for row in snapshot.tables.get(table.name).into_iter().flatten() {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };
            let exists_in_other: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT org_id FROM {} WHERE id = ?1 AND org_id <> ?2",
                        table.name
                    ),
                    rusqlite::params![id, org_id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            if exists_in_other.is_some() {
                conflicts.push(format!(
                    "`{}` {id} (o id do arquivo já existe nesta instância em outra org)",
                    table.name
                ));
                break 'fora;
            }
        }
    }
    for key in NATURAL_KEYS {
        let Some(rows) = snapshot.tables.get(key.table) else {
            continue;
        };
        for row in rows {
            let (Some(value), Some(id)) = (
                row.get(key.column).and_then(Value::as_str),
                row.get("id").and_then(Value::as_str),
            ) else {
                continue;
            };
            let existing: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT id FROM {} WHERE org_id = ?1 AND lower({}) = lower(?2)",
                        key.table, key.column
                    ),
                    rusqlite::params![org_id.to_string(), value],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(existing) = existing
                && existing != id
            {
                conflicts.push(format!(
                    "{} `{value}` (já existe nesta org com outro id)",
                    key.resource
                ));
            }
        }
    }
    if created_org && let Some(slug) = snapshot.org.get("slug").and_then(Value::as_str) {
        let existing: Option<String> = conn
            .query_row("SELECT id FROM orgs WHERE slug = ?1", [slug], |row| {
                row.get(0)
            })
            .optional()?;
        if existing.is_some() {
            conflicts.push(format!("org `{slug}` (o slug já existe nesta instância)"));
        }
    }
    Ok(conflicts)
}

fn check_natural_keys(
    conn: &Connection,
    snapshot: &OrgSnapshot,
    org_id: OrgId,
    created_org: bool,
) -> Resultado<()> {
    let conflicts = natural_key_conflicts(conn, snapshot, org_id, created_org)?;
    if conflicts.is_empty() {
        return Ok(());
    }
    let shown = conflicts.iter().take(5).cloned().collect::<Vec<_>>();
    let rest = conflicts.len().saturating_sub(shown.len());
    let mut reason = format!(
        "a importação foi recusada antes de gravar: {}",
        shown.join("; ")
    );
    if rest > 0 {
        reason.push_str(&format!("; e mais {rest}"));
    }
    reason.push_str(
        ". Nada foi gravado: renomeie um dos dois lados ou importe numa instância sem esses recursos",
    );
    Err(StoreError::Domain(DomainError::Validation {
        field: "backup",
        reason,
    }))
}

fn write_org(conn: &Connection, row: &Value, org_id: OrgId) -> Resultado<()> {
    let table = Table {
        name: "orgs",
        key: &["id"],
    };
    let object = row_object(table.name, row)?;
    let columns = table_columns(conn, table.name)?;
    write_object(conn, &table, &object, Some(org_id), &columns)?;
    Ok(())
}

fn write_row(
    conn: &Connection,
    table: &Table,
    org_id: OrgId,
    row: &Value,
    columns: &BTreeSet<String>,
) -> Resultado<bool> {
    let mut object = row_object(table.name, row)?;
    if table.name == "integrations" {
        object.remove("tem_segredo");
        object.remove("segredos_campos");
    }
    object.insert("org_id".to_owned(), Value::String(org_id.to_string()));
    if table.name == "file_library" || table.name == "attachments" {
        let sha = object
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StoreError::Domain(DomainError::Validation {
                    field: "backup",
                    reason: format!("linha de `{}` sem sha256", table.name),
                })
            })?;
        object.insert(
            "blob_path".to_owned(),
            Value::String(format!("{org_id}/{sha}")),
        );
    }
    if table.name == "fichas"
        && let Some(conversation) = object.get("conversation_id").and_then(Value::as_str)
    {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE org_id = ?1 AND id = ?2)",
            rusqlite::params![org_id.to_string(), conversation],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "backup",
                reason: "a conversa da Ficha não pertence à organização de destino".to_owned(),
            }));
        }
    }
    if table.name == "fluxo_revisoes" {
        let id = object.get("id").and_then(Value::as_str).unwrap_or("");
        let mut query =
            conn.prepare("SELECT * FROM fluxo_revisoes WHERE org_id = ?1 AND id = ?2")?;
        let names = column_names(&query);
        let mut rows = query.query(rusqlite::params![org_id.to_string(), id])?;
        if let Some(row) = rows.next()? {
            let existing = row_json(table.name, &names, row)?;
            if existing == Value::Object(object.clone()) {
                return Ok(true);
            }
            return Err(StoreError::Domain(DomainError::Validation {
                field: "backup",
                reason: "uma revisão publicada existente é imutável e difere do arquivo".to_owned(),
            }));
        }
    }
    write_object(conn, table, &object, Some(org_id), columns)
}

fn row_object(table: &str, row: &Value) -> Resultado<Map<String, Value>> {
    row.as_object().cloned().ok_or_else(|| {
        StoreError::Domain(DomainError::Validation {
            field: "backup",
            reason: format!("uma linha de `{table}` no arquivo não é um objeto JSON"),
        })
    })
}

fn write_object(
    conn: &Connection,
    table: &Table,
    object: &Map<String, Value>,
    scope: Option<OrgId>,
    columns: &BTreeSet<String>,
) -> Resultado<bool> {
    let scope = match table.name {
        "orgs" => None,
        _ => Some(scope.ok_or_else(|| {
            StoreError::Internal(format!("tabela `{}` sem escopo de org", table.name))
        })?),
    };
    let mut values: BTreeMap<String, SqlValue> = BTreeMap::new();
    for (name, value) in object {
        if !columns.contains(name.as_str()) {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "backup",
                reason: format!(
                    "o arquivo traz a coluna `{name}` em `{}`, que não existe nesta versão do \
                     esquema (arquivo de versão mais nova?)",
                    table.name
                ),
            }));
        }
        values.insert(name.clone(), sql_value(table.name, name, value)?);
    }
    for key in table.key {
        if !values.contains_key(*key) {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "backup",
                reason: format!("linha de `{}` sem a chave `{key}`", table.name),
            }));
        }
    }

    let keys: BTreeSet<&str> = table.key.iter().copied().collect();
    if row_exists(conn, table, &values, scope)? {
        update_row(conn, table, &values, &keys, scope)?;
        Ok(true)
    } else {
        insert_row(conn, table, &values)?;
        Ok(false)
    }
}

fn sql_value(table: &str, column: &str, value: &Value) -> Resultado<SqlValue> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(boolean) => SqlValue::Integer(i64::from(*boolean)),
        Value::Number(numero) => {
            if let Some(integer) = numero.as_i64() {
                SqlValue::Integer(integer)
            } else if let Some(real) = numero.as_f64() {
                SqlValue::Real(real)
            } else {
                return Err(StoreError::Domain(DomainError::Validation {
                    field: "backup",
                    reason: format!("o valor de `{table}.{column}` não é um número válido"),
                }));
            }
        }
        Value::String(texto) => SqlValue::Text(texto.clone()),
        Value::Array(_) | Value::Object(_) => {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "backup",
                reason: format!(
                    "o valor de `{table}.{column}` é um objeto ou lista; o banco espera um valor \
                     simples"
                ),
            }));
        }
    })
}

fn table_columns(conn: &Connection, table: &str) -> Resultado<BTreeSet<String>> {
    let mut query = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = query.query([])?;
    let mut names = BTreeSet::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        names.insert(name);
    }
    Ok(names)
}

fn row_exists(
    conn: &Connection,
    table: &Table,
    values: &BTreeMap<String, SqlValue>,
    scope: Option<OrgId>,
) -> Resultado<bool> {
    let (filter, parameters) = key_filter(table, values, scope, 0);
    let exists: Option<i64> = conn
        .query_row(
            &format!("SELECT 1 FROM {} WHERE {filter}", table.name),
            params_from_iter(parameters),
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn key_filter(
    table: &Table,
    values: &BTreeMap<String, SqlValue>,
    scope: Option<OrgId>,
    start_of: usize,
) -> (String, Vec<SqlValue>) {
    let mut filter = Vec::new();
    let mut parameters = Vec::new();
    let mut position = start_of;
    if let Some(org) = scope {
        position += 1;
        filter.push(format!("org_id = ?{position}"));
        parameters.push(SqlValue::Text(org.to_string()));
    }
    for key in table.key {
        position += 1;
        filter.push(format!("{key} = ?{position}"));
        parameters.push(values.get(*key).cloned().unwrap_or(SqlValue::Null));
    }
    (filter.join(" AND "), parameters)
}

fn insert_row(
    conn: &Connection,
    table: &Table,
    values: &BTreeMap<String, SqlValue>,
) -> Resultado<()> {
    let columns: Vec<&str> = values.keys().map(String::as_str).collect();
    let markers: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table.name,
        columns.join(", "),
        markers.join(", ")
    );
    let parameters: Vec<SqlValue> = values.values().cloned().collect();
    conn.execute(&sql, params_from_iter(parameters))
        .map_err(|error| StoreError::from_write(table.name, error))?;
    Ok(())
}

fn update_row(
    conn: &Connection,
    table: &Table,
    values: &BTreeMap<String, SqlValue>,
    keys: &BTreeSet<&str>,
    scope: Option<OrgId>,
) -> Resultado<()> {
    let mut definitions = Vec::new();
    let mut parameters: Vec<SqlValue> = Vec::new();
    let mut position = 0;
    for (name, value) in values {
        if keys.contains(name.as_str()) || name == "org_id" {
            continue;
        }
        position += 1;
        definitions.push(format!("{name} = ?{position}"));
        parameters.push(value.clone());
    }
    if definitions.is_empty() {
        return Ok(());
    }
    let (filter, filter_parameters) = key_filter(table, values, scope, position);
    parameters.extend(filter_parameters);
    let sql = format!(
        "UPDATE {} SET {} WHERE {filter}",
        table.name,
        definitions.join(", ")
    );
    conn.execute(&sql, params_from_iter(parameters))
        .map_err(|error| StoreError::from_write(table.name, error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewAgent, NewCategory, NewOrg, Page};

    fn database() -> Store {
        let folder = std::env::temp_dir().join(format!(
            "poph-backup-store-{}",
            uuid::Uuid::now_v7().simple()
        ));
        Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre o banco")
    }

    fn org_with_data(store: &Store, slug: &str) -> OrgId {
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: "{\"retencao\":{\"dias\":7}}".to_owned(),
            })
            .expect("cria a org")
            .id;
        store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        store
            .categories()
            .create(
                org,
                NewCategory {
                    name: "Pragas".to_owned(),
                    description: "urbanas".to_owned(),
                },
            )
            .expect("cria a categoria");
        org
    }

    #[test]
    fn snapshot_carries_org_and_no_other_org() {
        let store = database();
        let org_a = org_with_data(&store, "org-a");
        let org_b = org_with_data(&store, "org-b");
        let snapshot = store.org_snapshot(org_a, true).expect("retrato");
        assert_eq!(snapshot.org["id"], Value::String(org_a.to_string()));
        assert_eq!(snapshot.tables["agents"].len(), 1);
        assert_eq!(snapshot.tables["product_categories"].len(), 1);
        for rows in snapshot.tables.values() {
            for row in rows {
                assert_eq!(row["org_id"], Value::String(org_a.to_string()));
            }
        }
        let without_conversations = store.org_snapshot(org_a, false).expect("retrato");
        assert!(!without_conversations.tables.contains_key("conversations"));
        assert!(snapshot.tables.contains_key("conversations"));
        let _ = org_b;
    }

    #[test]
    fn importing_twice_does_not_duplicate() {
        let store = database();
        let origin = org_with_data(&store, "origem");
        let snapshot = store.org_snapshot(origin, true).expect("retrato");
        let first = store
            .import_snapshot(snapshot.clone(), ImportTarget::Origin)
            .expect("primeira importação");
        assert_eq!(first.secrets_to_reenter, 0);
        assert!(!first.created_org);
        let second = store
            .import_snapshot(snapshot, ImportTarget::Origin)
            .expect("segunda importação");
        assert!(second.tables.values().all(|t| t.inserted == 0));
        assert_eq!(second.total(), first.total());
        let agents = store.agents().list(origin, Page::default()).unwrap();
        assert_eq!(agents.len(), 1);
    }

    #[test]
    fn import_into_clean_instance_creates_org_and_preserves_ids() {
        let origin = database();
        let org = org_with_data(&origin, "origem");
        let agent = origin.agents().list(org, Page::default()).unwrap()[0].id;
        let snapshot = origin.org_snapshot(org, true).expect("retrato");

        let target = database();
        let report = target
            .import_snapshot(snapshot, ImportTarget::Origin)
            .expect("importa");
        assert!(report.created_org);
        assert_eq!(report.org, org);
        let imported_agent = target.agents().get(org, agent).expect("agente");
        assert_eq!(imported_agent.slug, "bia");
        let imported_org = target.orgs().get(org).expect("org");
        assert_eq!(imported_org.config_json, "{\"retencao\":{\"dias\":7}}");
    }

    #[test]
    fn import_into_existing_org_does_not_touch_the_org_row() {
        let origin = database();
        let org_origem = org_with_data(&origin, "origem");
        let snapshot = origin.org_snapshot(org_origem, true).expect("retrato");

        let destination = database();
        let target = destination
            .orgs()
            .create(NewOrg {
                name: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        let report = destination
            .import_snapshot(snapshot, ImportTarget::Existing(target))
            .expect("importa");
        assert!(!report.created_org);
        assert_eq!(report.org, target);
        let target_org = destination.orgs().get(target).expect("org alvo");
        assert_eq!(target_org.name, "Alvo");
        assert_eq!(target_org.slug, "alvo");
    }

    fn conversation_with_name(store: &Store, org: OrgId, name: &str) -> crate::Conversa {
        let agent = store.agents().list(org, Page::default()).unwrap()[0].id;
        let conversation = store
            .conversations()
            .create(
                org,
                crate::NewConversation {
                    agent_id: agent,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999990000".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa");
        store
            .conversations()
            .set_customer_name(org, conversation.id, name)
            .expect("grava o nome do cliente");
        conversation
    }

    #[test]
    fn backup_of_flows_records_and_revisions_preserves_state() {
        let origin = database();
        let org = org_with_data(&origin, "origem");
        let conversation = conversation_with_name(&origin, org, "Cliente fictício");
        let agent = origin.agents().list(org, Page::default()).unwrap()[0].id;
        let case = origin.org_snapshot(org, true).unwrap().tables["fichas"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        origin.write(move |conn| {
            let org = org.to_string();
            let agent = agent.to_string();
            for (id, slug, current) in [("flow-a", "first", 2), ("flow-b", "second", 1)] {
                conn.execute("INSERT INTO fluxos (id, org_id, slug, nome, revisao_atual) VALUES (?1, ?2, ?3, ?3, ?4)", rusqlite::params![id, org, slug, current])?;
                conn.execute("INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em) VALUES (?1, ?2, ?3, 1)", rusqlite::params![org, agent, id])?;
            }
            for (id, flow, numero) in [("rev-a1", "flow-a", 1), ("rev-a2", "flow-a", 2), ("rev-b1", "flow-b", 1)] {
                conn.execute("INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json, publicada_em, publicada_por) VALUES (?1, ?2, ?3, ?4, 'fonte', 'hash', '{}', 1, 'cli')", rusqlite::params![id, org, flow, numero])?;
            }
            conn.execute("INSERT INTO fluxo_execucoes (id, org_id, ficha_id, fluxo_id, revisao_id, no_atual, iniciada_em) VALUES ('exec', ?1, ?2, 'flow-a', 'rev-a1', 'etapa', 1)", rusqlite::params![org, case])?;
            conn.execute("INSERT INTO fluxo_execucao_etapas (id, org_id, execucao_id, no_id, ordem, entrou_em) VALUES ('etapa', ?1, 'exec', 'etapa', 1, 1)", [&org])?;
            conn.execute("INSERT INTO fluxo_efeitos (id, org_id, ficha_id, execucao_id, chave, tipo, created_at) VALUES ('efeito', ?1, ?2, 'exec', 'um', 'mensagem_ao_cliente', 1)", rusqlite::params![org, case])?;
            Ok(())
        }).unwrap();
        let snapshot = origin.org_snapshot(org, true).unwrap();
        assert_eq!(snapshot.tables["fluxos"].len(), 2);
        assert_eq!(snapshot.tables["fluxo_revisoes"].len(), 3);
        assert_eq!(snapshot.tables["agente_fluxos"].len(), 2);
        for name in [
            "fichas",
            "ficha_eventos",
            "fluxo_execucoes",
            "fluxo_execucao_etapas",
            "fluxo_efeitos",
        ] {
            assert!(!snapshot.tables[name].is_empty(), "{name}");
        }
        assert_eq!(
            snapshot.tables["fichas"][0]["conversation_id"],
            conversation.id.to_string()
        );
        let target = database();
        target
            .import_protected_snapshot(snapshot.clone(), ImportTarget::Origin, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            target.org_snapshot(org, true).unwrap().tables,
            snapshot.tables
        );
        assert!(
            target
                .import_protected_snapshot(snapshot.clone(), ImportTarget::Origin, false)
                .unwrap()
                .is_err()
        );
        target
            .import_protected_snapshot(snapshot.clone(), ImportTarget::Origin, true)
            .unwrap()
            .unwrap();
        let mut adulterado = snapshot.clone();
        adulterado.tables.get_mut("fluxo_revisoes").unwrap()[0]["fonte"] = Value::from("alterada");
        assert!(
            target
                .import_protected_snapshot(adulterado, ImportTarget::Origin, true)
                .is_err()
        );
        assert_eq!(
            target.org_snapshot(org, true).unwrap().tables,
            snapshot.tables
        );
        let mut incompleto = snapshot.clone();
        incompleto.tables.get_mut("fichas").unwrap().clear();
        let incomplete_database = database();
        assert!(
            incomplete_database
                .import_snapshot(incompleto, ImportTarget::Origin)
                .is_err()
        );
        assert!(incomplete_database.orgs().get(org).is_err());
        let without = origin.org_snapshot(org, false).unwrap();
        assert!(!without.tables.contains_key("fichas"));
        let empty = database();
        empty
            .import_snapshot(without, ImportTarget::Origin)
            .unwrap();
        assert!(empty.org_snapshot(org, true).unwrap().tables["fichas"].is_empty());
        let mut old = snapshot.clone();
        for name in [
            "fluxos",
            "fluxo_revisoes",
            "agente_fluxos",
            "fichas",
            "ficha_eventos",
            "fluxo_execucoes",
            "fluxo_execucao_etapas",
            "fluxo_efeitos",
        ] {
            old.tables.remove(name);
        }
        let document = crate::backup_document::document(&old, true, 0, &BTreeMap::new(), &|_| None);
        let mut document = document;
        document["versao_formato"] = Value::from(1);
        let old = crate::backup_document::document_snapshot(&document).unwrap();
        let old_target = database();
        old_target
            .import_snapshot(old, ImportTarget::Origin)
            .unwrap();
        let cases = old_target
            .org_snapshot(org, true)
            .unwrap()
            .tables
            .remove("fichas")
            .unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0]["conversation_id"], conversation.id.to_string());
    }

    #[test]
    fn backup_with_conversations_carries_the_customer_name() {
        let origin = database();
        let org = org_with_data(&origin, "origem");
        let conversation = conversation_with_name(&origin, org, "Ana Cliente");
        let snapshot = origin.org_snapshot(org, true).expect("retrato");
        assert_eq!(
            snapshot.tables["conversations"][0]["client_name"],
            Value::String("Ana Cliente".to_owned())
        );

        let target = database();
        target
            .import_snapshot(snapshot, ImportTarget::Origin)
            .expect("importa");
        let imported = target
            .conversations()
            .get(org, conversation.id)
            .expect("conversa importada");
        assert_eq!(imported.customer_name.as_deref(), Some("Ana Cliente"));
    }

    #[test]
    fn old_backup_without_the_column_imports_and_leaves_the_name_null() {
        let origin = database();
        let org = org_with_data(&origin, "origem");
        let conversation = conversation_with_name(&origin, org, "Ana Cliente");
        let mut snapshot = origin.org_snapshot(org, true).expect("retrato");
        for row in snapshot.tables.get_mut("conversations").expect("conversas") {
            row.as_object_mut().expect("objeto").remove("client_name");
        }

        let target = database();
        target
            .import_snapshot(snapshot, ImportTarget::Origin)
            .expect("importa o backup antigo");
        let imported = target
            .conversations()
            .get(org, conversation.id)
            .expect("conversa importada");
        assert_eq!(imported.customer_name, None);
    }

    #[test]
    fn old_backup_does_not_erase_the_name_already_stored_at_the_target() {
        let origin = database();
        let org = org_with_data(&origin, "origem");
        let conversation = conversation_with_name(&origin, org, "Ana Cliente");
        let new_snapshot = origin.org_snapshot(org, true).expect("retrato");
        let mut old_snapshot = new_snapshot.clone();
        for row in old_snapshot
            .tables
            .get_mut("conversations")
            .expect("conversas")
        {
            row.as_object_mut().expect("objeto").remove("client_name");
        }

        let target = database();
        target
            .import_snapshot(new_snapshot, ImportTarget::Origin)
            .expect("primeira importação, com o nome");
        target
            .import_snapshot(old_snapshot, ImportTarget::Origin)
            .expect("reimporta o backup antigo");
        let after = target
            .conversations()
            .get(org, conversation.id)
            .expect("conversa");
        assert_eq!(
            after.customer_name.as_deref(),
            Some("Ana Cliente"),
            "coluna que o arquivo não traz fica como estava"
        );
    }

    #[test]
    fn slug_conflict_with_different_id_is_rejected_without_writing() {
        let origin = database();
        let org_origem = org_with_data(&origin, "origem");
        let snapshot = origin.org_snapshot(org_origem, true).expect("retrato");

        let destination = database();
        let target = destination
            .orgs()
            .create(NewOrg {
                name: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        destination
            .agents()
            .create(
                target,
                NewAgent {
                    name: "Outra Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente que conflita");
        let error = destination
            .import_snapshot(snapshot, ImportTarget::Existing(target))
            .expect_err("devia recusar");
        assert!(error.to_string().contains("agente `bia`"), "{error}");
        assert_eq!(
            destination
                .categories()
                .list(target, Page::default())
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn import_conflicts_are_reported_without_writing() {
        let origin = database();
        let org_origem = org_with_data(&origin, "origem");
        let snapshot = origin.org_snapshot(org_origem, false).expect("retrato");

        let clean = database();
        assert!(
            clean
                .check_import_conflicts(&snapshot, ImportTarget::Origin)
                .expect("confere")
                .is_empty()
        );

        let destination = database();
        let target = destination
            .orgs()
            .create(NewOrg {
                name: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        destination
            .agents()
            .create(
                target,
                NewAgent {
                    name: "Outra Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente que conflita");
        let conflicts = destination
            .check_import_conflicts(&snapshot, ImportTarget::Existing(target))
            .expect("confere");
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        assert!(conflicts[0].contains("agente `bia`"), "{conflicts:?}");
        assert_eq!(
            destination
                .categories()
                .list(target, Page::default())
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn update_check_reports_what_would_be_overwritten_without_writing() {
        let origin = database();
        let org_origem = org_with_data(&origin, "origem");
        let snapshot = origin.org_snapshot(org_origem, true).expect("retrato");
        let file_rows: u64 = snapshot.tables.values().map(|l| l.len() as u64).sum();

        let clean = database();
        let empty = clean
            .check_import_updates(&snapshot, ImportTarget::Origin)
            .expect("confere");
        assert!(empty.empty(), "{empty:?}");
        assert_eq!(empty.total, 0);
        assert!(empty.examples.is_empty());
        assert!(
            clean.orgs().get(org_origem).is_err(),
            "a conferência é leitura: não pode criar a org"
        );

        let same = database();
        same.import_snapshot(snapshot.clone(), ImportTarget::Origin)
            .expect("primeira importação");
        let updates = same
            .check_import_updates(&snapshot, ImportTarget::Origin)
            .expect("confere");
        assert!(!updates.empty());
        assert_eq!(updates.total, file_rows + 1, "{updates:?}");
        assert_eq!(updates.tables.get("agents"), Some(&1));
        assert_eq!(updates.tables.get("orgs"), Some(&1));
        assert!(
            updates
                .examples
                .iter()
                .any(|example| example == "agents: bia"),
            "exemplo legível com slug: {:?}",
            updates.examples
        );
        assert!(updates.examples.len() <= MAX_UPDATE_EXAMPLES);
        assert!(
            updates.summary().contains("agents 1"),
            "{}",
            updates.summary()
        );
        let after = same.org_snapshot(org_origem, true).expect("retrato");
        assert_eq!(after.tables["agents"], snapshot.tables["agents"]);

        let destination = database();
        let target = destination
            .orgs()
            .create(NewOrg {
                name: "Alvo".to_owned(),
                slug: "alvo".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org alvo")
            .id;
        assert!(
            destination
                .check_import_updates(&snapshot, ImportTarget::Existing(target))
                .expect("confere")
                .empty(),
            "org alvo vazia: a importação só insere"
        );
        destination
            .import_snapshot(snapshot.clone(), ImportTarget::Existing(target))
            .expect("importa");
        let in_target_org = destination
            .check_import_updates(&snapshot, ImportTarget::Existing(target))
            .expect("confere");
        assert_eq!(in_target_org.total, file_rows);
        assert!(
            !in_target_org.tables.contains_key("orgs"),
            "o caminho Existente não escreve em orgs"
        );
    }

    #[test]
    fn unknown_column_is_rejected() {
        let store = database();
        let org = org_with_data(&store, "origem");
        let mut snapshot = store.org_snapshot(org, false).expect("retrato");
        snapshot.tables.get_mut("agents").unwrap()[0]
            .as_object_mut()
            .unwrap()
            .insert("coluna_do_futuro".to_owned(), Value::from(1));
        let error = store
            .import_snapshot(snapshot, ImportTarget::Origin)
            .expect_err("devia recusar");
        assert!(error.to_string().contains("coluna_do_futuro"), "{error}");
    }
}

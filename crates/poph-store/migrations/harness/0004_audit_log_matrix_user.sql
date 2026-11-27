
CREATE TABLE audit_log_new (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    REFERENCES orgs (id) ON DELETE SET NULL,
    actor_kind  TEXT    NOT NULL
                CHECK (actor_kind IN ('admin_token', 'system', 'plugin', 'cli', 'matrix_user')),
    actor_id    TEXT,
    action      TEXT    NOT NULL,
    entity      TEXT    NOT NULL,
    entity_id   TEXT,
    detail_json TEXT    NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL
) STRICT;

INSERT INTO audit_log_new
    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at)
SELECT id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at
  FROM audit_log;

DROP TABLE audit_log;
ALTER TABLE audit_log_new RENAME TO audit_log;

CREATE INDEX ix_audit_log_time ON audit_log (created_at);
CREATE INDEX ix_audit_log_org ON audit_log (org_id, created_at);

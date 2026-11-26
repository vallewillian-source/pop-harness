
CREATE TABLE matrix_midia (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id TEXT    NOT NULL,
    agent_id        TEXT    NOT NULL,
    room_id         TEXT    NOT NULL CHECK (length(room_id) BETWEEN 1 AND 255),
    event_id        TEXT    NOT NULL CHECK (length(event_id) BETWEEN 1 AND 255),
    mxc             TEXT    NOT NULL CHECK (length(mxc) BETWEEN 1 AND 512),
    name            TEXT    NOT NULL DEFAULT '',
    mime            TEXT    NOT NULL DEFAULT '',
    kind            TEXT    NOT NULL CHECK (kind IN ('image', 'document')),
    size_bytes      INTEGER NOT NULL DEFAULT 0 CHECK (size_bytes >= 0),
    sent_at         INTEGER NOT NULL,
    redacted_at     INTEGER,
    deleted_at      INTEGER,
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT    CHECK (last_error IS NULL OR length(last_error) <= 64),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_matrix_midia_evento ON matrix_midia (org_id, event_id);

CREATE INDEX ix_matrix_midia_pendentes ON matrix_midia (org_id, sent_at)
    WHERE redacted_at IS NULL OR deleted_at IS NULL;

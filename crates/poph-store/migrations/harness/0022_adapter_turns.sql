CREATE TABLE adapter_turns (
    org_id           TEXT    NOT NULL,
    conversation_id  TEXT    NOT NULL,
    id               TEXT    NOT NULL,
    message_id       TEXT    NOT NULL,
    idempotency_key  TEXT    NOT NULL,
    fingerprint      TEXT    NOT NULL,
    status           TEXT    NOT NULL
                     CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    engine_turn_id   TEXT,
    reply_message_id TEXT,
    error_code       TEXT,
    input_tokens     INTEGER NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens    INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    created_at       INTEGER NOT NULL,
    started_at       INTEGER,
    finished_at      INTEGER,
    PRIMARY KEY (org_id, conversation_id, id),
    UNIQUE (org_id, conversation_id, idempotency_key),
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (message_id) REFERENCES messages (id) ON DELETE CASCADE,
    FOREIGN KEY (reply_message_id) REFERENCES messages (id) ON DELETE SET NULL
) STRICT;

CREATE UNIQUE INDEX ix_adapter_turns_active
    ON adapter_turns (org_id, conversation_id)
    WHERE status IN ('queued', 'running');

CREATE INDEX ix_adapter_turns_conversation
    ON adapter_turns (org_id, conversation_id, created_at);

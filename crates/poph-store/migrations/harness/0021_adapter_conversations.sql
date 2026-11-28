CREATE TABLE adapter_conversations (
    org_id       TEXT    NOT NULL,
    id           TEXT    NOT NULL,
    channel      TEXT    NOT NULL CHECK (length(channel) BETWEEN 1 AND 32),
    external_ref TEXT    NOT NULL,
    create_case  INTEGER NOT NULL CHECK (create_case IN (0, 1)),
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_adapter_conversations_ref
    ON adapter_conversations (org_id, channel, external_ref);

CREATE TABLE adapter_message_keys (
    org_id              TEXT    NOT NULL,
    conversation_id     TEXT    NOT NULL,
    idempotency_key     TEXT    NOT NULL,
    message_id          TEXT    NOT NULL,
    fingerprint         TEXT    NOT NULL,
    external_message_id TEXT,
    created_at          INTEGER NOT NULL,
    PRIMARY KEY (org_id, conversation_id, idempotency_key),
    UNIQUE (org_id, message_id),
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (message_id) REFERENCES messages (id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_adapter_message_keys_message
    ON adapter_message_keys (org_id, conversation_id, message_id);

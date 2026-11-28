ALTER TABLE adapter_conversations
    ADD COLUMN last_event_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_event_seq >= 0);

CREATE TABLE adapter_events (
    org_id          TEXT    NOT NULL,
    conversation_id TEXT    NOT NULL,
    seq             INTEGER NOT NULL CHECK (seq > 0),
    turn_id         TEXT    NOT NULL,
    kind            TEXT    NOT NULL CHECK (kind IN (
                        'turn_start', 'text_delta', 'tool_call', 'tool_result',
                        'turn_end', 'turn_failed', 'turn_cancelled'
                    )),
    text            TEXT,
    tool_name       TEXT,
    result          TEXT CHECK (result IN ('executed', 'refused')),
    error_code      TEXT,
    PRIMARY KEY (org_id, conversation_id, seq),
    FOREIGN KEY (org_id, conversation_id, turn_id)
        REFERENCES adapter_turns (org_id, conversation_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_adapter_events_turn ON adapter_events (org_id, conversation_id, turn_id);

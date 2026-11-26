CREATE UNIQUE INDEX ux_site_conversation_reference
    ON conversations (org_id, agent_id, external_peer) WHERE channel = 'site';

CREATE TABLE site_conversation_cursors (
    org_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    last_seq INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (org_id, conversation_id),
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE TABLE site_turns (
    org_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    error_code TEXT,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    PRIMARY KEY (org_id, conversation_id, id),
    UNIQUE (org_id, conversation_id, idempotency_key),
    FOREIGN KEY (org_id, conversation_id) REFERENCES site_conversation_cursors (org_id, conversation_id) ON DELETE CASCADE,
    FOREIGN KEY (message_id) REFERENCES messages (id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_site_active_turn ON site_turns (org_id, conversation_id)
    WHERE status IN ('queued', 'running');

CREATE TABLE site_events (
    org_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    turn_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('message', 'turn_start', 'text_delta', 'turn_end', 'turn_failed', 'turn_cancelled')),
    message_id TEXT,
    role TEXT CHECK (role IN ('user', 'assistant')),
    text TEXT,
    error_code TEXT,
    PRIMARY KEY (org_id, conversation_id, seq),
    FOREIGN KEY (org_id, conversation_id, turn_id) REFERENCES site_turns (org_id, conversation_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_site_events_turn ON site_events (org_id, conversation_id, turn_id);

CREATE TRIGGER site_no_transfer BEFORE UPDATE OF status ON conversations
    WHEN NEW.channel = 'site' AND NEW.status = 'transferred'
    BEGIN
        SELECT RAISE(ABORT, 'conversa do site não permite transferência');
    END;

CREATE TRIGGER site_no_case BEFORE INSERT ON fichas
    WHEN NEW.canal = 'site' OR EXISTS (
        SELECT 1 FROM conversations WHERE id = NEW.conversation_id AND channel = 'site'
    )
    BEGIN
        SELECT RAISE(ABORT, 'conversa do site não permite Ficha');
    END;

CREATE TRIGGER site_no_outbox BEFORE INSERT ON outbox
    WHEN EXISTS (
        SELECT 1 FROM conversations
        WHERE org_id = NEW.org_id AND id = NEW.conversation_id AND channel = 'site'
    )
    BEGIN
        SELECT RAISE(ABORT, 'conversa do site não permite envio pela fila');
    END;

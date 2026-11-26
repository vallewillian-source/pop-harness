
CREATE TABLE matrix_orgs (
    org_id     TEXT    PRIMARY KEY NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    enabled    INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    enabled_at INTEGER,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE matrix_rooms (
    conversation_id TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    room_id         TEXT    NOT NULL CHECK (length(room_id) BETWEEN 1 AND 255),
    space_room_id   TEXT,
    last_status     TEXT    CHECK (last_status IN ('open', 'transferred', 'closed')),
    state           TEXT    NOT NULL DEFAULT 'creating'
                            CHECK (state IN ('creating', 'ready')),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_matrix_rooms_room_id ON matrix_rooms (room_id);
CREATE INDEX ix_matrix_rooms_org_state ON matrix_rooms (org_id, state);

CREATE TABLE matrix_spaces (
    id            TEXT    PRIMARY KEY NOT NULL,
    org_id        TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    department_id TEXT    NOT NULL,
    space_room_id TEXT    NOT NULL CHECK (length(space_room_id) BETWEEN 1 AND 255),
    alias         TEXT    NOT NULL CHECK (length(alias) BETWEEN 1 AND 255),
    origin        TEXT    NOT NULL CHECK (origin IN ('auto', 'linked')),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    FOREIGN KEY (org_id, department_id) REFERENCES departments (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_matrix_spaces_org_department ON matrix_spaces (org_id, department_id);
CREATE UNIQUE INDEX ux_matrix_spaces_org_room ON matrix_spaces (org_id, space_room_id);
CREATE UNIQUE INDEX ux_matrix_spaces_org_alias ON matrix_spaces (org_id, alias);

CREATE TABLE matrix_estado (
    org_id                  TEXT    PRIMARY KEY NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    messages_cursor         INTEGER NOT NULL DEFAULT 0 CHECK (messages_cursor >= 0),
    conversations_cursor_at INTEGER NOT NULL DEFAULT 0 CHECK (conversations_cursor_at >= 0),
    conversations_cursor_id TEXT    NOT NULL DEFAULT '',
    version                 INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    updated_at              INTEGER NOT NULL
) STRICT;

CREATE TABLE matrix_falhas (
    id         TEXT    PRIMARY KEY NOT NULL,
    org_id     TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    message_id TEXT    NOT NULL,
    motivo     TEXT    NOT NULL CHECK (length(motivo) BETWEEN 1 AND 64),
    tentativas INTEGER NOT NULL DEFAULT 1 CHECK (tentativas >= 1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_matrix_falhas_org_message ON matrix_falhas (org_id, message_id);
CREATE INDEX ix_matrix_falhas_org_time ON matrix_falhas (org_id, updated_at);

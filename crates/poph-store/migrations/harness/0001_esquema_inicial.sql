

CREATE TABLE orgs (
    id          TEXT    PRIMARY KEY NOT NULL,
    name        TEXT    NOT NULL,
    slug        TEXT    NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'inactive', 'archived')),
    config_json TEXT    NOT NULL DEFAULT '{}',
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_orgs_slug ON orgs (slug);
CREATE INDEX ix_orgs_status ON orgs (status);

CREATE TABLE admin_tokens (
    id         TEXT    PRIMARY KEY NOT NULL,
    org_id     TEXT    REFERENCES orgs (id) ON DELETE CASCADE,
    label      TEXT    NOT NULL,
    token_hash TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
) STRICT;

CREATE UNIQUE INDEX ux_admin_tokens_hash ON admin_tokens (token_hash);
CREATE INDEX ix_admin_tokens_org ON admin_tokens (org_id);


CREATE TABLE product_categories (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    status      TEXT    NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'inactive', 'archived')),
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_product_categories_name ON product_categories (org_id, lower(name));
CREATE UNIQUE INDEX ux_product_categories_org_id ON product_categories (org_id, id);

CREATE TABLE products (
    id           TEXT    PRIMARY KEY NOT NULL,
    org_id       TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    category_id  TEXT    NOT NULL,
    name         TEXT    NOT NULL,
    slug         TEXT    NOT NULL,
    aliases_json TEXT    NOT NULL DEFAULT '[]',
    description  TEXT,
    price_text   TEXT    NOT NULL,
    price_cents  INTEGER NOT NULL DEFAULT 0 CHECK (price_cents >= 0),
    unit         TEXT,
    includes     TEXT,
    excludes     TEXT,
    conditions   TEXT,
    tags_json    TEXT    NOT NULL DEFAULT '[]',
    status       TEXT    NOT NULL DEFAULT 'active'
                         CHECK (status IN ('active', 'inactive', 'archived')),
    version      INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    FOREIGN KEY (org_id, category_id) REFERENCES product_categories (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_products_slug ON products (org_id, slug);
CREATE UNIQUE INDEX ux_products_org_id ON products (org_id, id);
CREATE INDEX ix_products_category ON products (org_id, category_id, status);


CREATE TABLE departments (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    status      TEXT    NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'inactive', 'archived')),
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_departments_name ON departments (org_id, lower(name));
CREATE UNIQUE INDEX ux_departments_org_id ON departments (org_id, id);

CREATE TABLE agents (
    id                          TEXT    PRIMARY KEY NOT NULL,
    org_id                      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    name                        TEXT    NOT NULL,
    slug                        TEXT    NOT NULL,
    description                 TEXT    NOT NULL DEFAULT '',
    status                      TEXT    NOT NULL DEFAULT 'active'
                                CHECK (status IN ('active', 'inactive', 'archived')),
    agent_type                  TEXT    NOT NULL DEFAULT 'custom'
                                CHECK (agent_type IN ('custom', 'standard')),
    category                    TEXT    NOT NULL DEFAULT 'generic',
    default_department_id       TEXT    REFERENCES departments (id) ON DELETE SET NULL,
    human_handoff_enabled       INTEGER NOT NULL DEFAULT 0
                                CHECK (human_handoff_enabled IN (0, 1)),
    human_handoff_department_id TEXT    REFERENCES departments (id) ON DELETE SET NULL,
    provider                    TEXT    NOT NULL,
    model                       TEXT    NOT NULL,
    temperature                 REAL,
    max_output_tokens           INTEGER,
    max_context_messages        INTEGER NOT NULL DEFAULT 30 CHECK (max_context_messages >= 1),
    max_context_tokens          INTEGER NOT NULL DEFAULT 64000 CHECK (max_context_tokens >= 1000),
    grouping_window_ms          INTEGER NOT NULL DEFAULT 2000
                                CHECK (grouping_window_ms BETWEEN 0 AND 8000),
    max_tool_rounds             INTEGER NOT NULL DEFAULT 4
                                CHECK (max_tool_rounds BETWEEN 0 AND 20),
    max_tool_calls_per_message  INTEGER NOT NULL DEFAULT 10
                                CHECK (max_tool_calls_per_message BETWEEN 0 AND 100),
    rag_top_k                   INTEGER NOT NULL DEFAULT 3 CHECK (rag_top_k BETWEEN 1 AND 50),
    rag_min_score               REAL    NOT NULL DEFAULT 0.0
                                CHECK (rag_min_score >= 0.0 AND rag_min_score <= 1.0),
    jev_json                    TEXT    NOT NULL DEFAULT '{}',
    version                     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_agents_slug ON agents (org_id, slug);
CREATE UNIQUE INDEX ux_agents_org_id ON agents (org_id, id);
CREATE INDEX ix_agents_status ON agents (org_id, status);

CREATE TABLE agent_product_categories (
    org_id      TEXT NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id    TEXT NOT NULL,
    category_id TEXT NOT NULL,
    PRIMARY KEY (agent_id, category_id),
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, category_id) REFERENCES product_categories (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_agent_product_categories_org ON agent_product_categories (org_id, agent_id);

CREATE TABLE agent_instructions (
    agent_id          TEXT    PRIMARY KEY NOT NULL,
    org_id            TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    structured_json   TEXT    NOT NULL DEFAULT '{}',
    markdown          TEXT    NOT NULL DEFAULT '',
    derived_from_json INTEGER NOT NULL DEFAULT 1 CHECK (derived_from_json IN (0, 1)),
    version           INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_agent_instructions_org ON agent_instructions (org_id);


CREATE TABLE knowledge_groups (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    status      TEXT    NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'inactive', 'archived')),
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_knowledge_groups_name ON knowledge_groups (org_id, lower(name));
CREATE UNIQUE INDEX ux_knowledge_groups_org_id ON knowledge_groups (org_id, id);

CREATE TABLE knowledge_items (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    group_id    TEXT    NOT NULL,
    title       TEXT    NOT NULL,
    content     TEXT    NOT NULL CHECK (length(content) BETWEEN 1 AND 1000),
    tags_json   TEXT    NOT NULL DEFAULT '[]',
    source_type TEXT    NOT NULL DEFAULT 'manual' CHECK (source_type IN ('manual')),
    position    INTEGER NOT NULL DEFAULT 0,
    status      TEXT    NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'inactive', 'archived')),
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY (org_id, group_id) REFERENCES knowledge_groups (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_knowledge_items_group ON knowledge_items (org_id, group_id, position);

CREATE TABLE agent_knowledge_groups (
    org_id     TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id   TEXT    NOT NULL,
    group_id   TEXT    NOT NULL,
    version    INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, group_id),
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, group_id) REFERENCES knowledge_groups (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_agent_knowledge_groups_org ON agent_knowledge_groups (org_id, agent_id);


CREATE TABLE file_library (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id    TEXT    NOT NULL,
    name        TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    mime        TEXT    NOT NULL,
    size_bytes  INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256      TEXT    NOT NULL,
    blob_path   TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_file_library_agent ON file_library (org_id, agent_id);

CREATE TABLE integrations (
    id               TEXT    PRIMARY KEY NOT NULL,
    org_id           TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id         TEXT    NOT NULL,
    kind             TEXT    NOT NULL,
    config_json      TEXT    NOT NULL DEFAULT '{}',
    secrets_enc      BLOB,
    status           TEXT    NOT NULL DEFAULT 'active'
                     CHECK (status IN ('active', 'inactive', 'archived')),
    connection_state TEXT    NOT NULL DEFAULT 'disconnected',
    version          INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_integrations_agent ON integrations (org_id, agent_id);


CREATE TABLE conversations (
    id                   TEXT    PRIMARY KEY NOT NULL,
    org_id               TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id             TEXT    NOT NULL,
    integration_id       TEXT    REFERENCES integrations (id) ON DELETE SET NULL,
    channel              TEXT    NOT NULL,
    external_peer        TEXT,
    status               TEXT    NOT NULL DEFAULT 'open'
                         CHECK (status IN ('open', 'transferred', 'closed')),
    target_department_id TEXT    REFERENCES departments (id) ON DELETE SET NULL,
    summary              TEXT,
    version              INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    last_message_at      INTEGER,
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_conversations_status ON conversations (org_id, status, updated_at);
CREATE INDEX ix_conversations_agent ON conversations (org_id, agent_id, updated_at);
CREATE INDEX ix_conversations_peer ON conversations (org_id, channel, external_peer);
CREATE UNIQUE INDEX ux_conversations_org_id ON conversations (org_id, id);

CREATE TABLE messages (
    id               TEXT    PRIMARY KEY NOT NULL,
    org_id           TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id  TEXT    NOT NULL,
    direction        TEXT    NOT NULL CHECK (direction IN ('in', 'out')),
    role             TEXT    NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'system')),
    text             TEXT    NOT NULL DEFAULT '',
    attachments_json TEXT    NOT NULL DEFAULT '[]',
    external_id      TEXT,
    turn_id          TEXT,
    created_at       INTEGER NOT NULL,
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_messages_conversation ON messages (org_id, conversation_id, created_at);
CREATE UNIQUE INDEX ux_messages_external ON messages (conversation_id, external_id)
    WHERE external_id IS NOT NULL;

CREATE TABLE attachments (
    id                TEXT    PRIMARY KEY NOT NULL,
    org_id            TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    message_id        TEXT    NOT NULL,
    name              TEXT    NOT NULL,
    mime              TEXT    NOT NULL,
    size_bytes        INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256            TEXT    NOT NULL,
    blob_path         TEXT    NOT NULL,
    extracted_text    TEXT,
    extraction_status TEXT    NOT NULL DEFAULT 'pending'
                      CHECK (extraction_status IN ('pending', 'ok', 'failed', 'skipped')),
    created_at        INTEGER NOT NULL,
    FOREIGN KEY (message_id) REFERENCES messages (id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_attachments_message ON attachments (org_id, message_id);


CREATE TABLE carts (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id TEXT    NOT NULL,
    status          TEXT    NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    version         INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_carts_org_id ON carts (org_id, id);
CREATE UNIQUE INDEX ux_carts_open ON carts (org_id, conversation_id) WHERE status = 'open';

CREATE TABLE cart_items (
    id               TEXT    PRIMARY KEY NOT NULL,
    org_id           TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    cart_id          TEXT    NOT NULL,
    product_id       TEXT    REFERENCES products (id) ON DELETE SET NULL,
    quantity         INTEGER NOT NULL DEFAULT 1 CHECK (quantity > 0),
    unit_price_cents INTEGER NOT NULL DEFAULT 0 CHECK (unit_price_cents >= 0),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    FOREIGN KEY (org_id, cart_id) REFERENCES carts (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_cart_items_cart ON cart_items (org_id, cart_id);


CREATE TABLE inbox (
    id                  TEXT    PRIMARY KEY NOT NULL,
    org_id              TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    integration_id      TEXT    NOT NULL,
    external_message_id TEXT    NOT NULL,
    payload_json        TEXT    NOT NULL,
    received_at         INTEGER NOT NULL,
    processed_at        INTEGER,
    attempts            INTEGER NOT NULL DEFAULT 0,
    last_error          TEXT
) STRICT;

CREATE UNIQUE INDEX ux_inbox_external ON inbox (integration_id, external_message_id);
CREATE INDEX ix_inbox_pending ON inbox (org_id, processed_at, received_at);

CREATE TABLE outbox (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id TEXT    NOT NULL,
    turn_id         TEXT    NOT NULL,
    seq             INTEGER NOT NULL CHECK (seq >= 0),
    kind            TEXT    NOT NULL CHECK (kind IN ('text', 'file', 'presence')),
    payload_json    TEXT    NOT NULL DEFAULT '{}',
    state           TEXT    NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'sending', 'sent', 'failed')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    created_at      INTEGER NOT NULL,
    sent_at         INTEGER,
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_outbox_turn_seq ON outbox (turn_id, seq);
CREATE INDEX ix_outbox_state ON outbox (org_id, state, created_at);

CREATE TABLE turns (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id TEXT    NOT NULL,
    agent_id        TEXT    REFERENCES agents (id) ON DELETE SET NULL,
    state           TEXT    NOT NULL DEFAULT 'running'
                    CHECK (state IN ('running', 'done', 'failed', 'skipped')),
    jev_json        TEXT    NOT NULL DEFAULT '{}',
    retrieved_json  TEXT    NOT NULL DEFAULT '[]',
    tools_json      TEXT    NOT NULL DEFAULT '[]',
    timings_json    TEXT    NOT NULL DEFAULT '{}',
    input_tokens    INTEGER NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens   INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    cost_micros     INTEGER NOT NULL DEFAULT 0 CHECK (cost_micros >= 0),
    error           TEXT,
    started_at      INTEGER NOT NULL,
    finished_at     INTEGER,
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_turns_conversation ON turns (org_id, conversation_id, started_at);

CREATE TABLE audit_log (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    REFERENCES orgs (id) ON DELETE SET NULL,
    actor_kind  TEXT    NOT NULL CHECK (actor_kind IN ('admin_token', 'system', 'plugin', 'cli')),
    actor_id    TEXT,
    action      TEXT    NOT NULL,
    entity      TEXT    NOT NULL,
    entity_id   TEXT,
    detail_json TEXT    NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL
) STRICT;

CREATE INDEX ix_audit_log_time ON audit_log (created_at);
CREATE INDEX ix_audit_log_org ON audit_log (org_id, created_at);

-- Esquema inicial de `harness.db` (POP_HARNESS_RS_PRD.md §7.2).
--
-- Regras que valem para todas as tabelas:
--   * toda tabela de negócio tem `org_id` (a tabela `orgs` é a própria org; `schema_migrations`
--     é infraestrutura) e todo acesso filtra por ele;
--   * entidades editáveis têm `version`, exigida no `If-Match` da escrita;
--   * dinheiro é inteiro em centavos (`*_cents`) para não arredondar;
--   * tempo é `INTEGER` em milissegundos desde a época Unix (ordenável e sem dependência de data);
--   * `STRICT` em todas as tabelas: tipo errado falha na escrita, não vira lixo;
--   * as referências entre tabelas de negócio são **compostas com `org_id`** sempre que a coluna
--     é obrigatória (`FOREIGN KEY (org_id, x_id) REFERENCES x (org_id, id)`): assim o banco recusa
--     vínculo entre orgs diferentes, não só a aplicação. Para isso cada tabela-pai tem índice
--     único em `(org_id, id)`.
--   * as referências **opcionais** a departamento/produto/agente usam chave simples com
--     `ON DELETE SET NULL`: a compostas obrigaria a anular o `org_id` na exclusão. Nessas, a org
--     é conferida pelo repositório antes de gravar.
--
-- A ordem de criação respeita as chaves estrangeiras (pai antes do filho).

-- ─────────────────────────── Organizações e acesso ───────────────────────────

CREATE TABLE orgs (
    id          TEXT    PRIMARY KEY NOT NULL,
    name        TEXT    NOT NULL,
    slug        TEXT    NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'inactive', 'archived')),
    -- Retenção, limites e padrões de modelo da org (PRD §7.2). O que não estiver aqui herda a configuração.
    config_json TEXT    NOT NULL DEFAULT '{}',
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_orgs_slug ON orgs (slug);
CREATE INDEX ix_orgs_status ON orgs (status);

-- Tokens de org (os globais vêm da configuração, só o hash). `org_id` nulo = token global.
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

-- ────────────────────────────── Produtos ──────────────────────────────

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

-- ─────────────────────── Departamentos, agentes e instruções ───────────────────────

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
    -- `custom` | `standard`: paridade com o POP Atende (§1 da PARIDADE_AGENTE.md).
    agent_type                  TEXT    NOT NULL DEFAULT 'custom'
                                CHECK (agent_type IN ('custom', 'standard')),
    category                    TEXT    NOT NULL DEFAULT 'generic',
    -- Referência opcional: chave simples + conferência de org no repositório.
    default_department_id       TEXT    REFERENCES departments (id) ON DELETE SET NULL,
    human_handoff_enabled       INTEGER NOT NULL DEFAULT 0
                                CHECK (human_handoff_enabled IN (0, 1)),
    human_handoff_department_id TEXT    REFERENCES departments (id) ON DELETE SET NULL,
    -- Provedor e parâmetros do LLM: no Harness valem por agente (PRD D6).
    provider                    TEXT    NOT NULL,
    model                       TEXT    NOT NULL,
    temperature                 REAL,
    max_output_tokens           INTEGER,
    -- Contexto: `max_context_messages` é paridade/importador; o motor orça por tokens.
    max_context_messages        INTEGER NOT NULL DEFAULT 30 CHECK (max_context_messages >= 1),
    max_context_tokens          INTEGER NOT NULL DEFAULT 64000 CHECK (max_context_tokens >= 1000),
    -- Janela de agrupamento (PRD §6 passo 3; teto de 8 s) e laço de ferramentas (PRD §6 passo 6).
    grouping_window_ms          INTEGER NOT NULL DEFAULT 2000
                                CHECK (grouping_window_ms BETWEEN 0 AND 8000),
    max_tool_rounds             INTEGER NOT NULL DEFAULT 4
                                CHECK (max_tool_rounds BETWEEN 0 AND 20),
    max_tool_calls_per_message  INTEGER NOT NULL DEFAULT 10
                                CHECK (max_tool_calls_per_message BETWEEN 0 AND 100),
    -- RAG por agente (novo no Harness, PRD §9; hoje é global no POP Atende).
    rag_top_k                   INTEGER NOT NULL DEFAULT 3 CHECK (rag_top_k BETWEEN 1 AND 50),
    rag_min_score               REAL    NOT NULL DEFAULT 0.0
                                CHECK (rag_min_score >= 0.0 AND rag_min_score <= 1.0),
    -- Flags e limiares do Jev deste agente (PRD §10). Vazio = herda a configuração.
    jev_json                    TEXT    NOT NULL DEFAULT '{}',
    version                     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX ux_agents_slug ON agents (org_id, slug);
CREATE UNIQUE INDEX ux_agents_org_id ON agents (org_id, id);
CREATE INDEX ix_agents_status ON agents (org_id, status);

-- Categorias de produto vinculadas ao agente (PRD §7.2 fala em `product_category_ids`; normalizado aqui).
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
    -- `StructuredInstructions` (identidade, princípios, escopo, passos, tom, ferramentas, extras).
    structured_json   TEXT    NOT NULL DEFAULT '{}',
    -- Texto composto pelo compositor (vira o prefixo estável do prompt, PRD §6.1).
    markdown          TEXT    NOT NULL DEFAULT '',
    -- 1 = markdown ainda é derivado do JSON; 0 = editado à mão (o POP Atende preserva o editado).
    derived_from_json INTEGER NOT NULL DEFAULT 1 CHECK (derived_from_json IN (0, 1)),
    version           INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_agent_instructions_org ON agent_instructions (org_id);

-- ─────────────────────────── Base de conhecimento ───────────────────────────

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
    -- Limite de 1000 caracteres mantido do POP Atende (PARIDADE_AGENTE.md §3).
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

-- ─────────────────────────── Arquivos e integrações ───────────────────────────

CREATE TABLE file_library (
    id          TEXT    PRIMARY KEY NOT NULL,
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id    TEXT    NOT NULL,
    name        TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    mime        TEXT    NOT NULL,
    size_bytes  INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256      TEXT    NOT NULL,
    -- Caminho relativo dentro de `data/blobs` (PRD §7.1); o conteúdo não fica no SQLite.
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
    -- Segredos do plugin, cifrados (AES-256-GCM na HR-26); nunca em texto puro.
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

-- ─────────────────────── Conversas, mensagens e anexos ───────────────────────

CREATE TABLE conversations (
    id                   TEXT    PRIMARY KEY NOT NULL,
    org_id               TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id             TEXT    NOT NULL,
    integration_id       TEXT    REFERENCES integrations (id) ON DELETE SET NULL,
    channel              TEXT    NOT NULL,
    external_peer        TEXT,
    -- `open` | `transferred` (D12: não responde até reabrir) | `closed`.
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
-- Idempotência da mensagem recebida por conversa (`external_id` nulo não colide).
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
    -- Chave simples: a org do anexo é conferida no repositório (evita índice único extra
    -- em `messages`, que custa o dobro de vazão na escrita — medido na HR-07).
    FOREIGN KEY (message_id) REFERENCES messages (id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_attachments_message ON attachments (org_id, message_id);

-- ────────────────────────────── Carrinho ──────────────────────────────

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
-- Um carrinho aberto por conversa.
CREATE UNIQUE INDEX ux_carts_open ON carts (org_id, conversation_id) WHERE status = 'open';

CREATE TABLE cart_items (
    id               TEXT    PRIMARY KEY NOT NULL,
    org_id           TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    cart_id          TEXT    NOT NULL,
    -- Referência opcional: chave simples + conferência de org no repositório.
    product_id       TEXT    REFERENCES products (id) ON DELETE SET NULL,
    quantity         INTEGER NOT NULL DEFAULT 1 CHECK (quantity > 0),
    unit_price_cents INTEGER NOT NULL DEFAULT 0 CHECK (unit_price_cents >= 0),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    FOREIGN KEY (org_id, cart_id) REFERENCES carts (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_cart_items_cart ON cart_items (org_id, cart_id);

-- ───────────────────── Entrada, saída, turnos e auditoria ─────────────────────

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

-- Idempotência de entrada (PRD §6 passo 1): a mesma mensagem externa entra uma vez só.
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

-- Idempotência de saída (PRD §6 passo 8): `turn_id + seq`.
CREATE UNIQUE INDEX ux_outbox_turn_seq ON outbox (turn_id, seq);
CREATE INDEX ix_outbox_state ON outbox (org_id, state, created_at);

CREATE TABLE turns (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id TEXT    NOT NULL,
    -- Referência opcional: chave simples + conferência de org no repositório.
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
    -- Nulo para ação de token global.
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

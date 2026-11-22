-- Tabelas do módulo Matrix (MX-08; PRD §8, seção "Dados novos").
--
-- Regras que valem aqui:
--   * toda linha tem `org_id` e apagar a org apaga as linhas do módulo (`ON DELETE CASCADE`);
--   * **nenhuma coluna guarda texto de cliente, telefone ou token**: só ids, estados e o código
--     curto do erro (`matrix_falhas.motivo`, no máximo 64 caracteres);
--   * as referências obrigatórias são compostas com `org_id`, como no resto do esquema, para o
--     banco recusar vínculo entre orgs diferentes;
--   * `matrix_falhas.message_id` fica **sem** chave estrangeira de propósito: o índice único extra
--     em `messages` derrubava a vazão de escrita (ver `DADOS.md` §2). A org da mensagem é conferida
--     pelo repositório.
--
-- Estas tabelas **não** viajam no backup de org: a sala pertence ao Synapse do ambiente (ver
-- `BACKUP.md`).

-- Estado do módulo por org. `enabled_at` é o corte do backfill (P8): só se espelha o que foi
-- gravado depois dele. Desligar não apaga a data; religar grava uma data nova.
CREATE TABLE matrix_orgs (
    org_id     TEXT    PRIMARY KEY NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    enabled    INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    enabled_at INTEGER,
    updated_at INTEGER NOT NULL
) STRICT;

-- Sala do Matrix de uma conversa. Nasce em `creating` (pedido de criação feito, linha gravada) e
-- vira `ready` quando a sala existe e o estado inicial foi aplicado.
CREATE TABLE matrix_rooms (
    conversation_id TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    room_id         TEXT    NOT NULL CHECK (length(room_id) BETWEEN 1 AND 255),
    space_room_id   TEXT,
    -- Última situação da conversa que foi espelhada (mesma lista de `conversations.status`).
    last_status     TEXT    CHECK (last_status IN ('open', 'transferred', 'closed')),
    state           TEXT    NOT NULL DEFAULT 'creating'
                            CHECK (state IN ('creating', 'ready')),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

-- Uma sala por conversa (a chave primária) e uma conversa por sala.
CREATE UNIQUE INDEX ux_matrix_rooms_room_id ON matrix_rooms (room_id);
CREATE INDEX ix_matrix_rooms_org_state ON matrix_rooms (org_id, state);

-- Espaço do Matrix por departamento da org. `auto` foi criado pelo módulo; `linked` é um espaço
-- que já existia no Synapse e foi ligado ao departamento.
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

-- Um espaço por departamento; e um espaço (ou alias) do Synapse não serve a dois departamentos
-- da mesma org. A unicidade é por org porque duas orgs do mesmo Synapse podem ligar o mesmo
-- espaço existente (fase 3), e isso não é erro.
CREATE UNIQUE INDEX ux_matrix_spaces_org_department ON matrix_spaces (org_id, department_id);
CREATE UNIQUE INDEX ux_matrix_spaces_org_room ON matrix_spaces (org_id, space_room_id);
CREATE UNIQUE INDEX ux_matrix_spaces_org_alias ON matrix_spaces (org_id, alias);

-- Cursores do espelhamento, um por org.
--
-- `messages_cursor` é o `rowid` da última mensagem já espelhada (o mesmo cursor de
-- `Mensagens::desde`). `conversations_cursor_at` + `conversations_cursor_id` são a chave da última
-- conversa já conferida (o par de `Conversas::alteradas_desde`, para não perder conversa com o
-- mesmo milissegundo).
CREATE TABLE matrix_estado (
    org_id                  TEXT    PRIMARY KEY NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    messages_cursor         INTEGER NOT NULL DEFAULT 0 CHECK (messages_cursor >= 0),
    conversations_cursor_at INTEGER NOT NULL DEFAULT 0 CHECK (conversations_cursor_at >= 0),
    conversations_cursor_id TEXT    NOT NULL DEFAULT '',
    version                 INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    updated_at              INTEGER NOT NULL
) STRICT;

-- Mensagem isolada depois de falha definitiva. `motivo` é o código curto do erro (ex.:
-- `M_UNKNOWN`), nunca o texto do cliente. Repetir a falha da mesma mensagem atualiza a linha.
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

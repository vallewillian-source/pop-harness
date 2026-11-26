

CREATE TABLE fluxos (
    id             TEXT    PRIMARY KEY NOT NULL,
    org_id         TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    slug           TEXT    NOT NULL,
    nome           TEXT    NOT NULL,
    descricao      TEXT    NOT NULL DEFAULT '',
    status         TEXT    NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'inactive')),
    rascunho_fonte TEXT    NOT NULL DEFAULT '',
    revisao_atual  INTEGER CHECK (revisao_atual IS NULL OR revisao_atual >= 1),
    version        INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1)
) STRICT;

CREATE UNIQUE INDEX ux_fluxos_slug ON fluxos (org_id, slug);
CREATE UNIQUE INDEX ux_fluxos_org_id ON fluxos (org_id, id);
CREATE INDEX ix_fluxos_org_status ON fluxos (org_id, status);


CREATE TABLE fluxo_revisoes (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    fluxo_id        TEXT    NOT NULL,
    numero          INTEGER NOT NULL CHECK (numero >= 1),
    fonte           TEXT    NOT NULL,
    fonte_sha256    TEXT    NOT NULL,
    definicao_json  TEXT    NOT NULL,
    schema_versao   INTEGER NOT NULL DEFAULT 1 CHECK (schema_versao >= 1),
    publicada_em    INTEGER NOT NULL,
    publicada_por   TEXT    NOT NULL,
    resolucoes_json TEXT    NOT NULL DEFAULT '{}',
    FOREIGN KEY (org_id, fluxo_id) REFERENCES fluxos (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_fluxo_revisoes_numero ON fluxo_revisoes (org_id, fluxo_id, numero);

CREATE TRIGGER tr_fluxo_revisoes_sem_update
BEFORE UPDATE ON fluxo_revisoes
BEGIN
    SELECT RAISE(ABORT,
        'revisão publicada é imutável: publique uma revisão nova em vez de alterar esta');
END;

CREATE TRIGGER tr_fluxo_revisoes_sem_delete
BEFORE DELETE ON fluxo_revisoes
WHEN EXISTS (SELECT 1 FROM fluxos WHERE id = OLD.fluxo_id)
 AND EXISTS (SELECT 1 FROM orgs WHERE id = OLD.org_id)
BEGIN
    SELECT RAISE(ABORT,
        'revisão publicada é imutável: apague o fluxo (com force) em vez da revisão');
END;


CREATE TABLE agente_fluxos (
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id    TEXT    NOT NULL,
    fluxo_id    TEXT    NOT NULL,
    liberado_em INTEGER NOT NULL,
    PRIMARY KEY (org_id, agent_id, fluxo_id),
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, fluxo_id) REFERENCES fluxos (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_agente_fluxos_fluxo ON agente_fluxos (org_id, fluxo_id);


CREATE TABLE fluxo_execucoes (
    id            TEXT    PRIMARY KEY NOT NULL,
    org_id        TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    ficha_id      TEXT    NOT NULL,
    fluxo_id      TEXT    NOT NULL,
    revisao_id    TEXT    NOT NULL,
    status        TEXT    NOT NULL DEFAULT 'ativa'
                  CHECK (status IN ('ativa', 'concluida', 'abandonada', 'falha')),
    no_atual      TEXT    NOT NULL,
    dados_json    TEXT    NOT NULL DEFAULT '{}',
    resultado     TEXT,
    motivo_fim    TEXT,
    iniciada_em   INTEGER NOT NULL,
    finalizada_em INTEGER,
    version       INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    CHECK ((status = 'ativa') = (finalizada_em IS NULL)),
    FOREIGN KEY (org_id, ficha_id) REFERENCES fichas (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_fluxo_execucoes_ativa ON fluxo_execucoes (ficha_id)
    WHERE status = 'ativa';
CREATE UNIQUE INDEX ux_fluxo_execucoes_org_id ON fluxo_execucoes (org_id, id);
CREATE INDEX ix_fluxo_execucoes_ficha ON fluxo_execucoes (org_id, ficha_id, iniciada_em);
CREATE INDEX ix_fluxo_execucoes_revisao ON fluxo_execucoes (org_id, revisao_id, status);


CREATE TABLE fluxo_execucao_etapas (
    id                     TEXT    PRIMARY KEY NOT NULL,
    org_id                 TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    execucao_id            TEXT    NOT NULL,
    no_id                  TEXT    NOT NULL,
    ordem                  INTEGER NOT NULL CHECK (ordem >= 1),
    status                 TEXT    NOT NULL DEFAULT 'pendente'
                           CHECK (status IN ('pendente', 'parcial', 'concluida')),
    dados_json             TEXT    NOT NULL DEFAULT '{}',
    atores_que_agiram_json TEXT    NOT NULL DEFAULT '[]',
    entrou_em              INTEGER NOT NULL,
    concluiu_em            INTEGER,
    CHECK ((status = 'concluida') = (concluiu_em IS NOT NULL)),
    FOREIGN KEY (org_id, execucao_id) REFERENCES fluxo_execucoes (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_fluxo_execucao_etapas_ordem
    ON fluxo_execucao_etapas (org_id, execucao_id, ordem);


CREATE TABLE fluxo_efeitos (
    id                TEXT    PRIMARY KEY NOT NULL,
    org_id            TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    ficha_id          TEXT    NOT NULL,
    execucao_id       TEXT    NOT NULL,
    chave             TEXT    NOT NULL,
    tipo              TEXT    NOT NULL
                      CHECK (tipo IN ('mensagem_ao_cliente', 'notificar_whatsapp',
                                      'encaminhar')),
    payload_json      TEXT    NOT NULL DEFAULT '{}',
    estado            TEXT    NOT NULL DEFAULT 'pendente'
                      CHECK (estado IN ('pendente', 'enviando', 'enviado', 'falhou')),
    tentativas        INTEGER NOT NULL DEFAULT 0 CHECK (tentativas >= 0),
    ultimo_erro       TEXT,
    proxima_tentativa INTEGER,
    enviado_em        INTEGER,
    created_at        INTEGER NOT NULL,
    CHECK ((estado = 'enviado') = (enviado_em IS NOT NULL)),
    FOREIGN KEY (org_id, ficha_id) REFERENCES fichas (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, execucao_id) REFERENCES fluxo_execucoes (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_fluxo_efeitos_chave ON fluxo_efeitos (org_id, execucao_id, chave);
CREATE INDEX ix_fluxo_efeitos_fila ON fluxo_efeitos (estado, proxima_tentativa, created_at);
CREATE INDEX ix_fluxo_efeitos_org ON fluxo_efeitos (org_id, estado, enviado_em);


CREATE INDEX ix_fichas_org_agente ON fichas (org_id, agent_id, aberta_em);

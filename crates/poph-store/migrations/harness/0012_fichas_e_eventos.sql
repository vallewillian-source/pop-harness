

CREATE TABLE fichas (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id TEXT    REFERENCES conversations (id) ON DELETE SET NULL,
    agent_id        TEXT    NOT NULL,
    canal           TEXT    NOT NULL,
    peer_externo    TEXT,
    status          TEXT    NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'closed')),
    aberta_em       INTEGER NOT NULL,
    fechada_em      INTEGER,
    fechada_por     TEXT
                    CHECK (fechada_por IN ('cliente', 'agente', 'atendente', 'admin', 'fluxo',
                                           'sistema', 'migracao')),
    fechada_motivo  TEXT,
    version         INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    CHECK ((status = 'closed') = (fechada_em IS NOT NULL)),
    CHECK ((status = 'closed') = (fechada_por IS NOT NULL))
) STRICT;

CREATE UNIQUE INDEX ux_fichas_conversa ON fichas (conversation_id);
CREATE INDEX ix_fichas_org_status ON fichas (org_id, status, aberta_em);
CREATE UNIQUE INDEX ux_fichas_org_id ON fichas (org_id, id);


CREATE TABLE ficha_eventos (
    id                 TEXT    PRIMARY KEY NOT NULL,
    org_id             TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    ficha_id           TEXT    NOT NULL,
    execucao_id        TEXT,
    seq                INTEGER NOT NULL CHECK (seq >= 1),
    tipo               TEXT    NOT NULL
                       CHECK (tipo IN ('ficha_aberta', 'fluxo_iniciado', 'etapa_entrou',
                                       'campos_preenchidos', 'acao_executada',
                                       'decisao_resolvida', 'etapa_concluida', 'efeito_criado',
                                       'efeito_enviado', 'efeito_falhou', 'execucao_concluida',
                                       'execucao_abandonada', 'execucao_falhou',
                                       'atendimento_transferido', 'ficha_fechada')),
    ator_tipo          TEXT    NOT NULL
                       CHECK (ator_tipo IN ('agente', 'atendente', 'sistema', 'jev', 'admin')),
    ator_ref           TEXT,
    payload_json       TEXT    NOT NULL DEFAULT '{}',
    chave_idempotencia TEXT,
    created_at         INTEGER NOT NULL,
    FOREIGN KEY (org_id, ficha_id) REFERENCES fichas (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_ficha_eventos_seq ON ficha_eventos (ficha_id, seq);
CREATE UNIQUE INDEX ux_ficha_eventos_idem ON ficha_eventos (ficha_id, chave_idempotencia)
    WHERE chave_idempotencia IS NOT NULL;


INSERT INTO fichas
    (id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em, fechada_em,
     fechada_por, fechada_motivo, version)
SELECT
    lower(
        substr(printf('%012x', max(c.created_at, 0)), 1, 8) || '-' ||
        substr(printf('%012x', max(c.created_at, 0)), 9, 4) || '-7' ||
        substr(hex(randomblob(2)), 1, 3) || '-' ||
        substr('89ab', 1 + (random() & 3), 1) || substr(hex(randomblob(2)), 1, 3) || '-' ||
        hex(randomblob(6))
    ),
    c.org_id,
    c.id,
    c.agent_id,
    c.channel,
    c.external_peer,
    CASE WHEN c.status = 'closed' THEN 'closed' ELSE 'open' END,
    c.created_at,
    CASE WHEN c.status = 'closed' THEN c.updated_at END,
    CASE WHEN c.status = 'closed' THEN 'migracao' END,
    CASE WHEN c.status = 'closed'
         THEN 'conversa já estava fechada quando a Ficha foi criada' END,
    1
  FROM conversations c
 WHERE NOT EXISTS (SELECT 1 FROM fichas f WHERE f.conversation_id = c.id);

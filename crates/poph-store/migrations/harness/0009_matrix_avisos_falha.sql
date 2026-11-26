CREATE INDEX ix_messages_turno_humano ON messages (org_id, turn_id) WHERE origin = 'human';

CREATE TABLE matrix_avisos_falha (
    org_id        TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    outbox_id     TEXT    NOT NULL REFERENCES outbox (id) ON DELETE CASCADE,
    desfecho      TEXT    NOT NULL CHECK (desfecho IN ('avisado', 'sem_sala', 'recusado')),
    motivo        TEXT    NOT NULL CHECK (length(motivo) BETWEEN 1 AND 64),
    registrado_em INTEGER NOT NULL,
    PRIMARY KEY (org_id, outbox_id)
) STRICT;

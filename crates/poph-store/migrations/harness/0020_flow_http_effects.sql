
CREATE TABLE flow_effects_new (
    id                TEXT    PRIMARY KEY NOT NULL,
    org_id            TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    ficha_id          TEXT    NOT NULL,
    execucao_id       TEXT    NOT NULL,
    chave             TEXT    NOT NULL,
    tipo              TEXT    NOT NULL
                      CHECK (tipo IN ('mensagem_ao_cliente', 'notificar_whatsapp',
                                      'encaminhar', 'sincronizar_agenda',
                                      'agendar_transicao')),
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

INSERT INTO flow_effects_new
    (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, estado, tentativas,
     ultimo_erro, proxima_tentativa, enviado_em, created_at)
SELECT id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, estado, tentativas,
       ultimo_erro, proxima_tentativa, enviado_em, created_at
  FROM fluxo_efeitos;

DROP TABLE fluxo_efeitos;
ALTER TABLE flow_effects_new RENAME TO fluxo_efeitos;

CREATE UNIQUE INDEX ux_fluxo_efeitos_chave ON fluxo_efeitos (org_id, execucao_id, chave);
CREATE INDEX ix_fluxo_efeitos_fila ON fluxo_efeitos (estado, proxima_tentativa, created_at);
CREATE INDEX ix_fluxo_efeitos_org ON fluxo_efeitos (org_id, estado, enviado_em);

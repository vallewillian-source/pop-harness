-- MX-19 (aviso de falha de entrega da resposta humana; ver `docs/ENTRADA_E_SAIDA.md` §5 e
-- `docs/DADOS.md` §11.6).
--
-- 1. Índice parcial para achar a mensagem de uma resposta humana pelo `turn_id`. Só as mensagens
--    `origin = 'human'` entram no índice: as demais não pagam custo de escrita a mais (o índice
--    único extra em `messages` derrubou a vazão, ver `DADOS.md` §2).
CREATE INDEX ix_messages_turno_humano ON messages (org_id, turn_id) WHERE origin = 'human';

-- 2. Marca de aviso: uma linha por item de saída cuja falha já teve desfecho na sala. É ela que
--    faz o aviso não se repetir depois de reinício do processo. Não guarda texto de cliente nem
--    o erro bruto: só o desfecho e o código curto do motivo.
CREATE TABLE matrix_avisos_falha (
    org_id        TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    outbox_id     TEXT    NOT NULL REFERENCES outbox (id) ON DELETE CASCADE,
    desfecho      TEXT    NOT NULL CHECK (desfecho IN ('avisado', 'sem_sala', 'recusado')),
    motivo        TEXT    NOT NULL CHECK (length(motivo) BETWEEN 1 AND 64),
    registrado_em INTEGER NOT NULL,
    PRIMARY KEY (org_id, outbox_id)
) STRICT;

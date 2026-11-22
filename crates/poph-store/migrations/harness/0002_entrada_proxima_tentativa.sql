-- HR-26i: instante da próxima tentativa de um item da inbox adiado por falha do modelo.
-- Aditiva: nenhuma linha existente muda. NULL = pode ser atendido já (o caso de sempre).
ALTER TABLE inbox ADD COLUMN next_attempt_at INTEGER;

-- Só os itens ainda pendentes e adiados entram no índice: a fila normal não paga por ele.
CREATE INDEX ix_inbox_proxima_tentativa ON inbox (org_id, next_attempt_at)
    WHERE processed_at IS NULL AND next_attempt_at IS NOT NULL;

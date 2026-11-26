ALTER TABLE inbox ADD COLUMN next_attempt_at INTEGER;

CREATE INDEX ix_inbox_proxima_tentativa ON inbox (org_id, next_attempt_at)
    WHERE processed_at IS NULL AND next_attempt_at IS NOT NULL;

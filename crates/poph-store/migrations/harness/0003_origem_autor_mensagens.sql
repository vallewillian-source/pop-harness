ALTER TABLE messages ADD COLUMN origin TEXT NOT NULL DEFAULT 'agent'
    CHECK (origin IN ('agent', 'human', 'device'));
ALTER TABLE messages ADD COLUMN author_name TEXT;

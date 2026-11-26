CREATE INDEX ix_conversations_org_peer_id
    ON conversations (org_id, external_peer, id);

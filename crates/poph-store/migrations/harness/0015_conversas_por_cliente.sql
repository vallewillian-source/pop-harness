-- RP-02 (H3): índice para a listagem de conversas filtrada pelo cliente (`external_peer`).
--
-- A listagem administrativa aceita `external_peer` (e `status`, `sort=recent`). O índice antigo
-- `ix_conversations_peer` começa por `channel`, que o filtro não traz, então o SQLite não o usa.
-- Este índice começa por `(org_id, external_peer)` e termina em `id`: a busca pelo cliente cai
-- direto nas conversas dele, e a ordem por id (a listagem paginada) sai do próprio índice, sem
-- ordenar de novo. Migração aditiva: só cria o índice, não mexe em dado nem em tabela.
CREATE INDEX ix_conversations_org_peer_id
    ON conversations (org_id, external_peer, id);

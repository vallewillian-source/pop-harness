-- MX-05 (PRD §7 N3): leitura incremental de conversas por `updated_at`.
--
-- `Conversas::alteradas_desde` pagina por `(updated_at, id)` numa consulta global (sem filtro de
-- org). Sem este índice, o plano varre `conversations` inteira e ordena num temporário a cada
-- página (medido em `tests/leitura_incremental_medicao.rs`). Migração aditiva: só cria o índice,
-- não mexe em linha nem em tabela.
CREATE INDEX ix_conversations_atualizada ON conversations (updated_at, id);

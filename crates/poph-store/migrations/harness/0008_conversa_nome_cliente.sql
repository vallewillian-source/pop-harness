-- MX-32 (HARNESS_MATRIX_PRD N6, P10 a P13): nome do cliente do WhatsApp (`pushName`) guardado na
-- conversa, para a sala e o fantasma do cliente no Matrix saírem com o nome, não com o telefone.
--
-- Nulável e sem padrão: conversa antiga (ou conversa cujo webhook nunca trouxe `pushName`) fica
-- `NULL`, e o módulo usa o telefone nesse caso. Migração aditiva: só acrescenta a coluna.
ALTER TABLE conversations ADD COLUMN client_name TEXT;

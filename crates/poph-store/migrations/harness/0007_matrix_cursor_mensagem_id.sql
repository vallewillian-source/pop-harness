-- MX-14 (PRD §8, `matrix_estado`): id da mensagem que está no cursor de mensagens.
--
-- O `rowid` pode ser reaproveitado quando a última linha é apagada (retenção, apagar conversa).
-- Guardar o id junto deixa o laço distinguir "a mesma mensagem" de "uma mensagem nova no mesmo
-- rowid": a primeira é pulada, a segunda é espelhada. Migração aditiva: só acrescenta a coluna,
-- com padrão vazio para as linhas que já existem (org sem id guardado: o laço trata a linha do
-- cursor como não espelhada e a deduplicação por `txnId` do Synapse absorve a repetição).
ALTER TABLE matrix_estado ADD COLUMN messages_cursor_id TEXT NOT NULL DEFAULT '';

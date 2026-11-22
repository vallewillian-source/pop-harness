-- MX-40 (HARNESS_MATRIX_PRD N5): id que o canal devolveu para a mensagem enviada.
--
-- Nulável e sem padrão, aditiva: itens já enviados ficam com `NULL` (o id deles nunca foi guardado).
-- Quem grava é a saída do WhatsApp (`saida_whatsapp.rs`) ao marcar o item como `sent`, com o id que
-- a Evolution devolve na resposta do envio. É o que a fase 2 usa para distinguir o eco do próprio
-- envio (`fromMe` com este id = eco, descartar) da mensagem digitada no aparelho.
--
-- Canal de plugin pode não devolver id: aí a coluna fica `NULL`, e isso não é falha de envio.
ALTER TABLE outbox ADD COLUMN external_message_id TEXT;

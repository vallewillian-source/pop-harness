-- MX-03 (PRD do módulo Matrix, §7 N1 e §3.2 P9): quem produziu a mensagem e o nome do autor.
-- Aditiva: nenhuma linha existente muda. Mensagens que já existem e as que o turno grava
-- continuam `agent`, sem autor. `human` é a resposta do atendente; `device` é a digitada no
-- celular do cliente (fase 2). Entrada (`direction = 'in'`) também fica com o padrão `agent`.
ALTER TABLE messages ADD COLUMN origin TEXT NOT NULL DEFAULT 'agent'
    CHECK (origin IN ('agent', 'human', 'device'));
ALTER TABLE messages ADD COLUMN author_name TEXT;

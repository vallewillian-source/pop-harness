-- Expiração da mídia no Synapse (MX-46; PRD §9, critério A14, fase 2).
--
-- Cada linha é uma mídia que o espelho **publicou** numa sala (`m.image`/`m.file`, MX-43) e que a
-- rotina de expiração precisa apagar no repositório do Synapse e redigir na sala depois de
-- `matrix.midia_retencao_dias`. A tabela existe porque o evento, sozinho, não diz quando o arquivo
-- chegou nem o que já foi feito: sem ela, a rotina teria de varrer a história das salas a cada
-- passagem.
--
-- Regras que valem aqui:
--   * `id` é o id do anexo (`attachments.id`): a mesma mídia publicada duas vezes não vira duas
--     linhas (o `txnId` do envio é derivado dele, então o Synapse devolve o mesmo evento);
--   * **nenhuma coluna guarda token**: só ids, o `mxc://` do arquivo, o nome que já está no evento,
--     tipo, tamanho, carimbos e o código curto do último erro;
--   * as duas etapas têm carimbo próprio (`redacted_at`, `deleted_at`): repetir a passagem não
--     repete a chamada já concluída, e uma falha no meio retoma só o passo que faltou;
--   * `attempts` e `next_attempt_at` são a espera crescente do item que falhou (nada de laço
--     apertado: o item sai da frente e volta depois);
--   * `conversation_id` e `agent_id` ficam **sem** chave estrangeira (como `matrix_falhas.msg_id`):
--     a retenção do histórico pode apagar a conversa antes de a mídia vencer, e a rotina ainda tem
--     de apagar o arquivo do Synapse — o `agent_id` é o que escolhe o fantasma que redige. A org,
--     sim, tem chave estrangeira: apagar a org apaga as linhas do módulo.

CREATE TABLE matrix_midia (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    conversation_id TEXT    NOT NULL,
    -- Agente da conversa: o fantasma dele (moderador da sala) é quem redige o evento.
    agent_id        TEXT    NOT NULL,
    room_id         TEXT    NOT NULL CHECK (length(room_id) BETWEEN 1 AND 255),
    event_id        TEXT    NOT NULL CHECK (length(event_id) BETWEEN 1 AND 255),
    -- `mxc://servidor/id` do arquivo que subiu na sala.
    mxc             TEXT    NOT NULL CHECK (length(mxc) BETWEEN 1 AND 512),
    -- Nome que já está no evento (é o que o Element mostra no aviso de expiração).
    name            TEXT    NOT NULL DEFAULT '',
    mime            TEXT    NOT NULL DEFAULT '',
    -- Categoria do anexo, a mesma da projeção do Python: `image` ou `document`.
    kind            TEXT    NOT NULL CHECK (kind IN ('image', 'document')),
    size_bytes      INTEGER NOT NULL DEFAULT 0 CHECK (size_bytes >= 0),
    -- Quando o arquivo entrou no Synapse (ms). É daqui que o prazo conta.
    sent_at         INTEGER NOT NULL,
    -- Carimbos das duas etapas. NULL = etapa pendente.
    redacted_at     INTEGER,
    deleted_at      INTEGER,
    -- Espera crescente da última falha.
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at INTEGER NOT NULL DEFAULT 0,
    -- Código curto do último erro (ex.: `M_FORBIDDEN`), nunca texto de cliente.
    last_error      TEXT    CHECK (last_error IS NULL OR length(last_error) <= 64),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
) STRICT;

-- Uma linha por evento da sala: reprocessar a mesma publicação não duplica a mídia.
CREATE UNIQUE INDEX ux_matrix_midia_evento ON matrix_midia (org_id, event_id);

-- Fila da expiração: só o que ainda tem etapa pendente, da mais antiga para a mais nova.
CREATE INDEX ix_matrix_midia_pendentes ON matrix_midia (org_id, sent_at)
    WHERE redacted_at IS NULL OR deleted_at IS NULL;

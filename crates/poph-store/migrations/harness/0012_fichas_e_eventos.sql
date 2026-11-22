-- FX-05 (HARNESS_FLUXOS_PRD §6.1-5 e §7.1): Fichas e eventos da Ficha.
--
-- A Ficha é o registro do atendimento: nasce junto com a conversa (FX-07) e fecha quando a conversa
-- fecha. Cada execução de fluxo é sub-item dela (FX-06). Aqui entram só as duas tabelas e o
-- backfill do que já existia no banco.
--
-- Convenções do projeto (docs/DADOS.md): `org_id` em toda tabela de negócio, `STRICT`, ids UUID v7
-- em `TEXT`, tempo em `INTEGER` de milissegundos, JSON em `*_json`, `version` na entidade editável e
-- referência composta `(org_id, id)`. Os nomes de tabela e coluna são exatamente os do PRD §7.1
-- (decisão de contrato do mantenedor na FX-05).
--
-- Migração aditiva: não altera `conversations`, `messages` nem `audit_log`. O backfill não grava
-- evento da Ficha nem linha em `audit_log` (decisão do mantenedor): o próprio `fechada_por =
-- 'migracao'` registra o que aconteceu.

-- ───────────────────────────────── Fichas ─────────────────────────────────

CREATE TABLE fichas (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    -- Uma Ficha por conversa. Chave simples com `ON DELETE SET NULL`, como o PRD manda: a Ficha é
    -- o registro do atendimento e sobrevive à conversa apagada (retenção).
    conversation_id TEXT    REFERENCES conversations (id) ON DELETE SET NULL,
    -- Retrato do atendimento no instante em que a Ficha nasceu, copiado da conversa. Sem chave
    -- estrangeira de propósito: apagar o agente apaga as conversas dele (cascata), esta Ficha
    -- sobrevive ao histórico, e uma chave aqui ou barraria a exclusão ou anularia esta coluna.
    agent_id        TEXT    NOT NULL,
    canal           TEXT    NOT NULL,
    peer_externo    TEXT,
    -- `open` | `closed`. Enquanto a conversa estiver `open` ou `transferred`, a Ficha está `open`.
    status          TEXT    NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'closed')),
    aberta_em       INTEGER NOT NULL,
    fechada_em      INTEGER,
    fechada_por     TEXT
                    CHECK (fechada_por IN ('cliente', 'agente', 'atendente', 'admin', 'fluxo',
                                           'sistema', 'migracao')),
    fechada_motivo  TEXT,
    version         INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    -- Ficha fechada é imutável (PRD §6.1-4): o instante e o autor do fechamento andam com o
    -- status, num sentido e no outro. Ficha aberta não carrega nenhum dos dois.
    CHECK ((status = 'closed') = (fechada_em IS NOT NULL)),
    CHECK ((status = 'closed') = (fechada_por IS NOT NULL))
) STRICT;

-- Uma Ficha por conversa. No SQLite vários `NULL` são distintos num índice único, então as Fichas
-- que sobreviveram à conversa apagada não colidem entre si.
CREATE UNIQUE INDEX ux_fichas_conversa ON fichas (conversation_id);
CREATE INDEX ix_fichas_org_status ON fichas (org_id, status, aberta_em);
-- Alvo das referências compostas das tabelas filhas (`ficha_eventos` aqui, `fluxo_execucoes` e
-- `fluxo_efeitos` na FX-06).
CREATE UNIQUE INDEX ux_fichas_org_id ON fichas (org_id, id);

-- ──────────────────────────── Eventos da Ficha ────────────────────────────

-- A linha do tempo do atendimento (o que o atendente lê no Console). `seq` cresce por Ficha e é o
-- cursor da paginação; `chave_idempotencia` evita evento repetido quando a mesma operação é
-- refeita (reenvio do Synapse, retomada depois de queda).
CREATE TABLE ficha_eventos (
    id                 TEXT    PRIMARY KEY NOT NULL,
    org_id             TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    ficha_id           TEXT    NOT NULL,
    -- Nulo em evento que não é de fluxo. Sem chave estrangeira: `fluxo_execucoes` nasce na FX-06, e
    -- o SQLite não acrescenta FK a tabela existente sem reconstruí-la. O repositório confere org e
    -- execução antes de gravar (`poph-store`).
    execucao_id        TEXT,
    seq                INTEGER NOT NULL CHECK (seq >= 1),
    tipo               TEXT    NOT NULL
                       CHECK (tipo IN ('ficha_aberta', 'fluxo_iniciado', 'etapa_entrou',
                                       'campos_preenchidos', 'acao_executada',
                                       'decisao_resolvida', 'etapa_concluida', 'efeito_criado',
                                       'efeito_enviado', 'efeito_falhou', 'execucao_concluida',
                                       'execucao_abandonada', 'execucao_falhou',
                                       'atendimento_transferido', 'ficha_fechada')),
    ator_tipo          TEXT    NOT NULL
                       CHECK (ator_tipo IN ('agente', 'atendente', 'sistema', 'jev', 'admin')),
    ator_ref           TEXT,
    payload_json       TEXT    NOT NULL DEFAULT '{}',
    chave_idempotencia TEXT,
    created_at         INTEGER NOT NULL,
    FOREIGN KEY (org_id, ficha_id) REFERENCES fichas (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_ficha_eventos_seq ON ficha_eventos (ficha_id, seq);
-- Idempotência por Ficha: `chave_idempotencia` nula não colide (evento sem chave é sempre novo).
CREATE UNIQUE INDEX ux_ficha_eventos_idem ON ficha_eventos (ficha_id, chave_idempotencia)
    WHERE chave_idempotencia IS NOT NULL;

-- ──────────────────────── Backfill das conversas existentes ────────────────────────

-- Uma Ficha para cada conversa que já existe (PRD §6.1-5): `open`/`transferred` → Ficha `open`;
-- `closed` → Ficha `closed` com `fechada_por = 'migracao'`. `aberta_em` é a criação da conversa;
-- `fechada_em` é o último instante conhecido da conversa (`updated_at`, que numa conversa fechada é
-- o próprio fechamento).
--
-- O id é um UUID v7 montado no próprio SQL: 48 bits de tempo vindos da criação da conversa e o
-- resto de `randomblob`, para ficar ordenável por tempo como os ids gerados pelo código. O
-- `NOT EXISTS` deixa o comando idempotente: rodá-lo de novo não cria uma segunda Ficha.
INSERT INTO fichas
    (id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em, fechada_em,
     fechada_por, fechada_motivo, version)
SELECT
    lower(
        substr(printf('%012x', max(c.created_at, 0)), 1, 8) || '-' ||
        substr(printf('%012x', max(c.created_at, 0)), 9, 4) || '-7' ||
        substr(hex(randomblob(2)), 1, 3) || '-' ||
        substr('89ab', 1 + (random() & 3), 1) || substr(hex(randomblob(2)), 1, 3) || '-' ||
        hex(randomblob(6))
    ),
    c.org_id,
    c.id,
    c.agent_id,
    c.channel,
    c.external_peer,
    CASE WHEN c.status = 'closed' THEN 'closed' ELSE 'open' END,
    c.created_at,
    CASE WHEN c.status = 'closed' THEN c.updated_at END,
    CASE WHEN c.status = 'closed' THEN 'migracao' END,
    CASE WHEN c.status = 'closed'
         THEN 'conversa já estava fechada quando a Ficha foi criada' END,
    1
  FROM conversations c
 WHERE NOT EXISTS (SELECT 1 FROM fichas f WHERE f.conversation_id = c.id);

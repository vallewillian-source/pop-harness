-- FX-06 (especificação original do módulo, removida em 23/11/2026; ver docs/FLUXOS.md): fluxos, revisões, permissão por agente, execuções,
-- etapas da execução e efeitos.
--
-- O fluxo pertence à ORGANIZAÇÃO (não ao agente): o agente só recebe permissão de usar o que estiver
-- em `agente_fluxos`. O texto editável é o rascunho (`fluxos.rascunho_fonte`); o que o motor executa
-- é uma revisão publicada e imutável (`fluxo_revisoes`), com a IR em JSON e o texto de origem.
--
-- Convenções do projeto (docs/DADOS.md): `org_id` em toda tabela de negócio, `STRICT`, ids UUID v7
-- em `TEXT`, tempo em `INTEGER` de milissegundos, JSON em `*_json`, `version` na entidade editável e
-- referência composta `(org_id, id)`. Os nomes de tabela e coluna são exatamente os do PRD §7.1
-- (decisão de contrato do mantenedor na FX-06).
--
-- Migração aditiva: não altera nenhuma tabela existente. O único acréscimo fora das seis tabelas é
-- o índice `ix_fichas_org_agente` em `fichas` (filtro `?agente=` da lista de Fichas, que faltou na
-- 0012 — decisão 7 do mantenedor).

-- ───────────────────────────────── Fluxos ─────────────────────────────────

-- O fluxo da organização. `slug` vem do `id` do script (`fluxo(id = "agendar_visita", …)`) e é o
-- nome estável que o Jev devolve na escolha do gate G9.
CREATE TABLE fluxos (
    id             TEXT    PRIMARY KEY NOT NULL,
    org_id         TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    slug           TEXT    NOT NULL,
    nome           TEXT    NOT NULL,
    descricao      TEXT    NOT NULL DEFAULT '',
    -- `inactive` não inicia execução nova; as em andamento terminam na revisão em que estão.
    status         TEXT    NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'inactive')),
    -- Rascunho: texto livre, pode estar inválido (é o que a tela salva enquanto se edita).
    rascunho_fonte TEXT    NOT NULL DEFAULT '',
    -- Número da revisão publicada que vale hoje. Nulo enquanto o fluxo não tem nenhuma revisão.
    -- Sem chave estrangeira para `fluxo_revisoes` de propósito: as duas tabelas se referenciariam
    -- (revisão aponta o fluxo, fluxo aponta a revisão atual) e o SQLite não tem FK adiável ligada
    -- neste projeto. Quem garante a coerência é o repositório, na transação de publicação (FX-09).
    revisao_atual  INTEGER CHECK (revisao_atual IS NULL OR revisao_atual >= 1),
    version        INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1)
) STRICT;

-- `slug` único por organização: é o identificador que o Jev escolhe e o script declara.
CREATE UNIQUE INDEX ux_fluxos_slug ON fluxos (org_id, slug);
-- Alvo das referências compostas das tabelas filhas (revisões e permissão).
CREATE UNIQUE INDEX ux_fluxos_org_id ON fluxos (org_id, id);
CREATE INDEX ix_fluxos_org_status ON fluxos (org_id, status);

-- ──────────────────────── Revisões publicadas (imutáveis) ────────────────────────

-- Cada publicação gera uma revisão nova; a anterior nunca muda. Quem já está num fluxo continua na
-- revisão em que começou (`fluxo_execucoes.revisao_id`), e só os atendimentos novos pegam a nova.
CREATE TABLE fluxo_revisoes (
    id              TEXT    PRIMARY KEY NOT NULL,
    org_id          TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    fluxo_id        TEXT    NOT NULL,
    numero          INTEGER NOT NULL CHECK (numero >= 1),
    -- Texto do script exatamente como foi publicado, e o hash dele: é o que prova que a IR
    -- corresponde ao texto (o motor recompila e compara).
    fonte           TEXT    NOT NULL,
    fonte_sha256    TEXT    NOT NULL,
    -- A IR (`schema_versao` de `docs/FLUXOS_LINGUAGEM.md` §8), já validada sem erro.
    definicao_json  TEXT    NOT NULL,
    schema_versao   INTEGER NOT NULL DEFAULT 1 CHECK (schema_versao >= 1),
    publicada_em    INTEGER NOT NULL,
    -- Tipo do ator que publicou, no vocabulário de `audit_log.actor_kind` (`admin_token`, `cli`,
    -- `system`, `plugin`, `matrix_user`); o id dele e o detalhe ficam em `audit_log`
    -- (`flow_publish`), para não duplicar auditoria dentro da revisão imutável.
    publicada_por   TEXT    NOT NULL,
    -- Resoluções feitas no instante da publicação (ex.: nome de departamento → id), para que o
    -- comportamento da revisão não mude quando o cadastro mudar.
    resolucoes_json TEXT    NOT NULL DEFAULT '{}',
    FOREIGN KEY (org_id, fluxo_id) REFERENCES fluxos (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_fluxo_revisoes_numero ON fluxo_revisoes (org_id, fluxo_id, numero);

-- Imutabilidade no banco (não só na aplicação): revisão publicada não muda nem é apagada solta.
--
-- O `WHEN` do gatilho de exclusão existe para as duas cascatas legítimas continuarem funcionando:
-- apagar o **fluxo** (`DELETE /flows/{id}?force=true`, PRD §6.2) e apagar a **organização**. Nas
-- duas, o gatilho roda como `AFTER DELETE` do pai, então o pai já saiu da tabela quando o SQLite
-- apaga a revisão: com o fluxo fora de `fluxos` **ou** a org fora de `orgs`, a exclusão passa.
-- Apagar a revisão por conta própria, com o fluxo e a org de pé, é recusado.
CREATE TRIGGER tr_fluxo_revisoes_sem_update
BEFORE UPDATE ON fluxo_revisoes
BEGIN
    SELECT RAISE(ABORT,
        'revisão publicada é imutável: publique uma revisão nova em vez de alterar esta');
END;

CREATE TRIGGER tr_fluxo_revisoes_sem_delete
BEFORE DELETE ON fluxo_revisoes
WHEN EXISTS (SELECT 1 FROM fluxos WHERE id = OLD.fluxo_id)
 AND EXISTS (SELECT 1 FROM orgs WHERE id = OLD.org_id)
BEGIN
    SELECT RAISE(ABORT,
        'revisão publicada é imutável: apague o fluxo (com force) em vez da revisão');
END;

-- ────────────────────────── Permissão do agente ──────────────────────────

-- Lista explícita: sem linha aqui, o fluxo não é oferecido ao Jev, não entra no prompt e as
-- ferramentas dele são recusadas (padrão negado). Teto de 20 por agente é regra do repositório.
CREATE TABLE agente_fluxos (
    org_id      TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    agent_id    TEXT    NOT NULL,
    fluxo_id    TEXT    NOT NULL,
    liberado_em INTEGER NOT NULL,
    PRIMARY KEY (org_id, agent_id, fluxo_id),
    -- Apagar o agente apaga **só** a permissão: o fluxo e as revisões são da organização.
    FOREIGN KEY (org_id, agent_id) REFERENCES agents (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, fluxo_id) REFERENCES fluxos (org_id, id) ON DELETE CASCADE
) STRICT;

-- A lista por agente usa o prefixo da chave primária; esta serve ao sentido inverso
-- (`listar_agentes_do_fluxo`).
CREATE INDEX ix_agente_fluxos_fluxo ON agente_fluxos (org_id, fluxo_id);

-- ──────────────────────────── Execuções do fluxo ────────────────────────────

-- Uma execução é o fluxo correndo numa Ficha. É sub-item da Ficha (W6): apagar a Ficha leva as
-- execuções junto; apagar o **fluxo** não apaga execução nenhuma (o histórico do atendimento
-- permanece, §6.2), por isso `fluxo_id` e `revisao_id` ficam sem chave estrangeira — quem confere
-- org e existência é o repositório, na mesma transação (mesmo precedente de
-- `ficha_eventos.execucao_id`).
CREATE TABLE fluxo_execucoes (
    id            TEXT    PRIMARY KEY NOT NULL,
    org_id        TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    ficha_id      TEXT    NOT NULL,
    fluxo_id      TEXT    NOT NULL,
    revisao_id    TEXT    NOT NULL,
    status        TEXT    NOT NULL DEFAULT 'ativa'
                  CHECK (status IN ('ativa', 'concluida', 'abandonada', 'falha')),
    -- Nó em que a execução está (etapa, decisão ou o `fim` em que parou).
    no_atual      TEXT    NOT NULL,
    -- `dados[etapa][campo]` de todo o fluxo; o recorte por etapa fica em `fluxo_execucao_etapas`.
    dados_json    TEXT    NOT NULL DEFAULT '{}',
    -- Resultado do `fim(...)` e motivo do abandono/falha.
    resultado     TEXT,
    motivo_fim    TEXT,
    iniciada_em   INTEGER NOT NULL,
    finalizada_em INTEGER,
    version       INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    -- Execução encerrada tem instante de fim; ativa não tem. Um sentido anda com o outro.
    CHECK ((status = 'ativa') = (finalizada_em IS NULL)),
    FOREIGN KEY (org_id, ficha_id) REFERENCES fichas (org_id, id) ON DELETE CASCADE
) STRICT;

-- Uma execução ativa por Ficha (PRD §7.1): a Ficha é o atendimento, e dois fluxos correndo ao mesmo
-- tempo na mesma conversa deixariam o prompt ambíguo. Execuções encerradas se acumulam como
-- histórico. Índice único **parcial**: só as ativas entram na regra.
CREATE UNIQUE INDEX ux_fluxo_execucoes_ativa ON fluxo_execucoes (ficha_id)
    WHERE status = 'ativa';
-- Alvo das referências compostas das etapas e dos efeitos.
CREATE UNIQUE INDEX ux_fluxo_execucoes_org_id ON fluxo_execucoes (org_id, id);
-- Detalhe da Ficha: execuções dela em ordem de início.
CREATE INDEX ix_fluxo_execucoes_ficha ON fluxo_execucoes (org_id, ficha_id, iniciada_em);
-- Quantos atendimentos estão em cada revisão (é o número que o diff de revisões mostra).
CREATE INDEX ix_fluxo_execucoes_revisao ON fluxo_execucoes (org_id, revisao_id, status);

-- ─────────────────────── Etapas percorridas pela execução ───────────────────────

-- O histórico por etapa (W6): cada entrada numa etapa é uma linha, com os campos colhidos nela e
-- quem já agiu. `ordem` conta as entradas da execução (1, 2, 3…) e é o cursor de leitura.
CREATE TABLE fluxo_execucao_etapas (
    id                     TEXT    PRIMARY KEY NOT NULL,
    org_id                 TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    execucao_id            TEXT    NOT NULL,
    no_id                  TEXT    NOT NULL,
    ordem                  INTEGER NOT NULL CHECK (ordem >= 1),
    status                 TEXT    NOT NULL DEFAULT 'pendente'
                           CHECK (status IN ('pendente', 'parcial', 'concluida')),
    -- Campos desta etapa (o todo fica em `fluxo_execucoes.dados_json`).
    dados_json             TEXT    NOT NULL DEFAULT '{}',
    -- Atores que já executaram ação na etapa: é o que faz o critério `auto` decidir, em qualquer
    -- ordem de chegada.
    atores_que_agiram_json TEXT    NOT NULL DEFAULT '[]',
    entrou_em              INTEGER NOT NULL,
    concluiu_em            INTEGER,
    -- Etapa concluída tem instante de conclusão; pendente e parcial não têm.
    CHECK ((status = 'concluida') = (concluiu_em IS NOT NULL)),
    FOREIGN KEY (org_id, execucao_id) REFERENCES fluxo_execucoes (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_fluxo_execucao_etapas_ordem
    ON fluxo_execucao_etapas (org_id, execucao_id, ordem);

-- ────────────────────────────── Efeitos ──────────────────────────────

-- O que o fluxo manda fazer fora dele (PRD §6.8). Fila própria porque `outbox` é por conversa e a
-- notificação para a equipe vai para **outro número**.
--
-- `chave` é a idempotência do efeito (execução + nó + índice na IR): a mesma transição refeita não
-- gera segunda mensagem. Falha de efeito **não** reverte a transição do fluxo (L11): ela fica
-- `falhou`, com tentativa e espera crescente, e vira evento na Ficha.
CREATE TABLE fluxo_efeitos (
    id                TEXT    PRIMARY KEY NOT NULL,
    org_id            TEXT    NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    ficha_id          TEXT    NOT NULL,
    execucao_id       TEXT    NOT NULL,
    chave             TEXT    NOT NULL,
    tipo              TEXT    NOT NULL
                      CHECK (tipo IN ('mensagem_ao_cliente', 'notificar_whatsapp',
                                      'encaminhar')),
    payload_json      TEXT    NOT NULL DEFAULT '{}',
    estado            TEXT    NOT NULL DEFAULT 'pendente'
                      CHECK (estado IN ('pendente', 'enviando', 'enviado', 'falhou')),
    tentativas        INTEGER NOT NULL DEFAULT 0 CHECK (tentativas >= 0),
    ultimo_erro       TEXT,
    proxima_tentativa INTEGER,
    enviado_em        INTEGER,
    created_at        INTEGER NOT NULL,
    -- Efeito entregue tem instante de entrega; os outros estados não têm.
    CHECK ((estado = 'enviado') = (enviado_em IS NOT NULL)),
    FOREIGN KEY (org_id, ficha_id) REFERENCES fichas (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, execucao_id) REFERENCES fluxo_execucoes (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_fluxo_efeitos_chave ON fluxo_efeitos (org_id, execucao_id, chave);
-- Fila: o que está por entregar, na ordem de tentativa.
CREATE INDEX ix_fluxo_efeitos_fila ON fluxo_efeitos (estado, proxima_tentativa, created_at);
-- Leitura por organização (Console e diagnóstico do `poph doctor`).
CREATE INDEX ix_fluxo_efeitos_org ON fluxo_efeitos (org_id, estado, enviado_em);

-- ───────────── Índice que faltou na 0012: lista de Fichas por agente ─────────────

-- `GET …/fichas?agente=` (PRD §7.3). Aditivo: só índice, tabela `fichas` intocada.
CREATE INDEX ix_fichas_org_agente ON fichas (org_id, agent_id, aberta_em);

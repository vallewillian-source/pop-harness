# Hot memory and concurrency (HR-11)

> Design of the `poph-agent` crate (D-08): what stays in memory in the process, how the turn enters the
> queue, how much memory is allowed, and how the other tasks use this. Numbers measured on this machine
> (x86_64), not estimated.

## 1. Where each thing lives

| Part | Module | What it holds |
|---|---|---|
| Hot configuration | `config_quente` | agent + instructions + enabled tools + RAG top-k/cut, per `(org, agent)` |
| Actors per conversation | `atores` | one tokio task and one queue per `(org, conversation)` |
| Context window | `contexto` | latest messages that fit in the agent's token budget |
| Limits | `limites` | concurrency per org, per agent and per provider; per-minute quota of the org |
| Metrics | `metricas` | live actors, evictions, cache hits, cache memory, wait for a slot |
| Compositor | `compositor` | the text of the prompt's stable prefix (moved out of `poph-core` in this task) |

Everything here is disposable: restarting the process only costs re-reading from SQLite. The real state stays in
`harness.db` (`docs/DADOS.md`).

## 2. Hot configuration cache

- **Key**: `(org_id, agent_id)`. There is no agent read without an org.
- **Content**: `Agente` (model, tool limits, RAG top-k/cut, Jev), saved instructions, the structured JSON already
  parsed, the markdown of the stable prefix and the list of enabled tools.
- **D-04 applied**: `AgenteQuente::ferramentas` is the closed list; `ferramenta_habilitada(nome)` is the filter that
  HR-13 uses in the tool loop. A disabled tool does not get in.
- **D-05 applied**: only `active` enters the cache — `Agentes::obter_para_turno` refuses an inactive or archived agent
  with `DomainError::AgenteInativo`, so there is never an agent in the cache that cannot answer.
- **Invalidation**: `carregar_verificando` reads **two** cheap versions (agent and instructions) and reloads when either
  one changed. Editing only the prompt text counts: the agent version does not change, but the instructions version
  does. There is also `invalidar(org, agente)` and `esquecer_org(org)` for whoever edits in bulk, and `carregar`
  (without verification) for when the caller already knows nothing changed.
- **Validity**: 60 s by default (`OpcoesConfigQuente::validade`). After that, the next `carregar` reads again.
- **Cap**: 64 MB of estimated memory by default, with a weight per entry (`bytes`). The cache does not grow past the
  cap; with 10 thousand agents it evicts the least used ones (group 5 test).
- **Markdown**: uses the **saved** text — including text edited by hand. The cache does not rebuild the prompt: whoever
  writes it (CLI/API) rebuilds it, not whoever reads it.

## 3. Actors per conversation

- One tokio task and one queue (`mpsc`) per `(org, conversation)`. The consumer is single, so **two turns of the same
  conversation never run in parallel** and the arrival order is kept.
- **Eviction for idleness**: after `ociosidade` (90 s by default) without an event, the actor leaves memory. The exit
  only happens with the queue empty and under the same lock the send uses — there is no window in which an event is
  left orphaned or in which two actors serve the same conversation.
- **Reload**: when the conversation comes back, the processor receives `ao_acordar(org, conversa)` before the first
  event and reloads whatever it needs from the database. Nothing from durable state lives in the actor.
- **Full queue**: `enviar` returns `ErroAgente::FilaCheia` (default: 64 events per conversation) instead of growing
  without limit. Holding and resending is the job of the `inbox` (HR-12).
- **Actor cap**: 4096 by default. When the cap is passed, only an **idle** actor is evicted; if all are busy, the cap
  is exceeded (and counted in `poph_atores_acima_do_teto_total`), because evicting a busy actor would break "one turn
  per conversation".
- **No tokio runtime**: `enviar` returns `ErroAgente::Interno` instead of panicking.

## 4. Context window

- The most recent messages enter, in order, up to the budget of `agents.max_context_tokens` and the cap of
  `agents.max_context_messages`.
- Estimated count: ~4 characters per token, plus 4 fixed tokens per message. The real tokenizer belongs to the
  provider; HR-13 subtracts the reported usage and HR-14 spends part of the budget on the summary.
- A message that alone does not fit is **cut** (with `…`) when it is the most recent one: a turn without the customer's
  words does not exist. The others that do not fit are left out (`descartadas`), to become a summary in HR-14.
- `Janela::folga()` says how much room is left — it is the number HR-14 uses to decide the size of the summary.

## 5. Limits

| Level | Config key | Default |
|---|---|---|
| Instance | `limites.turnos_simultaneos` | 200 (D-07) |
| Org | `limites.turnos_simultaneos_por_org` | 50 |
| Agent | `OpcoesLimites::com_agente` (0 = no own cap) | no cap |
| Provider | `OpcoesLimites::com_provedor` (`llm.simultaneos`) | 32 |
| Org quota | `limites.taxa_por_org_por_minuto` | 600 |

- Separate semaphores per org: **a saturated org does not delay another org** (there is no queue or global lock shared
  between orgs beyond the instance cap, which is the design).
- `adquirir_turno` returns a `Vaga` (slot); releasing it is leaving scope. A wait with a time cap →
  `ErroAgente::SemVaga`, which is transient (`eh_transitorio()`), so the caller can requeue.
- `consumir_cota(org)` is a per-minute token bucket; when it runs out, it returns `CotaEsgotada` with the suggested wait
  time.

## 6. Metrics (`/metrics` from HR-20)

`poph_atores_vivos`, `poph_atores_despejados_total`, `poph_atores_acima_do_teto_total`,
`poph_eventos_entregues_total`, `poph_eventos_recusados_total`, `poph_config_quente_acertos_total`,
`poph_config_quente_erros_total`, `poph_config_quente_invalidadas_total`, `poph_config_quente_bytes`,
`poph_config_agentes_total`, `poph_espera_vaga_segundos`. They do not repeat the name of the base catalog
(`poph-types::MetricasBase`), so both can be registered in the same registry.

## 7. How HR-12 and HR-13 use it

HR-12 (inbox/outbox):
1. saves the input and calls `atores.enviar(Tarefa { org, conversa, mensagem })`;
2. if `FilaCheia` comes back, the message stays `pending` in the `inbox` and is resent later (the webhook ACK was
   already given; the guarantee is the unique key of the `inbox`);
3. on startup, whatever is `pending` goes back to the actor's queue.

HR-13 (turn):
1. `config.carregar_verificando(org, agente)` → `AgenteQuente`;
2. `limitador.adquirir_turno(org, agente, espera)` and `limitar_ao_provedor` before calling the LLM;
3. `contexto::do_banco(store, org, conversa, agente)` for the window;
4. `agente_quente.markdown` is the stable prefix; `agente_quente.ferramentas` filters the toolset;
5. the processor runs inside the actor, so "one turn at a time" is already guaranteed.

## 8. Measured numbers (x86_64, 12 cores, debug build)

Command: `cargo test -p poph-agent --test memoria_e_concorrencia -- --ignored --nocapture custo`

| Measure | Value |
|---|---|
| Enqueue one event in the actor (lock + channel) | ~0.97 µs/event |
| Full actor loop (receive + no-op processing + release) | ~1.73 µs/event |
| Hot configuration read from cache (hit) | ~1.18 µs/read |
| Cap test load: 10 thousand agents in cache with a 1 MB cap | cache stays ≤ 1 MB and < 10 thousand entries |

The actor's cost is three orders of magnitude below the internal latency target (M6: ≤ 2 ms p50) and is negligible next
to the LLM (6.5 s to the first text, HR-01). Reading the cache avoids two queries per turn; the difference shows up as
~1 µs, but the real gain is not touching the disk on the critical path.

## 9. Known limits (what is not covered)

- **Invalidation is checked by reading versions**, not by a signal from the writer: if the CLI/API writes instructions,
  the next turn already sees them; if it writes in bulk, each key pays one query per primary key. There is no
  invalidation bus (it was not needed in V1).
- **The actor cap can be exceeded** if all are busy (by design: prefer exceeding it over breaking "one turn per
  conversation").
- **An event refused because the queue is full is not retried here**: HR-12 keeps it in the `inbox`.
- **There is no cancellation of a turn in progress**: the turn runs until it ends. Cancelling the LLM call belongs to
  HR-13.
- **The window is a token estimate**; it does not use the Gemini tokenizer.
- **Not measured**: long runs (hours, leaks/fragmentation), ARM64 (HR-56), behavior with 500 simultaneously active
  conversations (HR-55).
- **`poph` does not use this crate yet**: the link to the turn and the CLI belongs to HR-13.

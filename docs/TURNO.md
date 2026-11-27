# Turn: minimal engine and `poph chat` (HR-13)

> Phase 1 milestone: the conversation works end to end through the CLI. This document explains the design,
> the decisions, what the next task consumes, and what is **not** covered.
> Code: `crates/poph-agent/src/turno/` (engine) and `crates/poph/src/chat.rs` (CLI).

## 1. The path of a turn

```
customer message
   │
   ├─ chat:  messages.gravar(entrada) ─────────────────────────────┐
   └─ integration: Fluxo::receber_mensagem (message + inbox, atomic)
                            │
                    Fluxo::despachar → conversation actor (HR-11)
                            │
                    ProcessadorTurno → Motor::executar
                            │
   1. conversation + agent (D-05: an inactive agent does not open a turn; D12: a transferred conversation does not reply)
   2. grouping window (Agente.grouping_window_ms, ceiling of 8 s)
   3. group = tail of customer messages that still have no reply
   4. turns.iniciar (state=running)
   5. turn slot (Limites: org, agent, instance) + provider slot
   6. prompt = stable prefix + volatile block + context window (HR-11)
   7. tool loop (limit of rounds and of calls per message, D-04)
   8. reply = messages.gravar(saida) + outbox.enfileirar(turno, seq 0)
   9. turns.finalizar (times, tokens, model, reason, error)

   (HR-29) if the Jev's G1 decided to transfer, the turn ends at step 7½:
     7½. risk question → guidance text + automatic message → messages.gravar +
         outbox.enfileirar → conversas.transferir(status=transferred, audit) → turns.finalizar
         The text model is NOT called; the following turns fall into step 1 (D12).
                            │
        Fluxo::enviar_pendentes(transport) → customer
```

`Motor::executar` returns `ResumoTurno` (turn recorded, state, text, messages handled, times,
tokens). It returns `Err` only when storage fails before the turn is opened. An inactive agent, a
transferred conversation, an already-answered message, and a provider error all end as a `ResumoTurno`
with `estado` and `erro` set. The processor has to close the entry in both cases.

## 2. Grouping window

- The wait is measured **from the arrival of the message** that triggered the turn: `window − elapsed`,
  never more than `grouping_window_ms` (0 uses 2 s) nor the 8 s ceiling from PRD §6.3.
- After the wait, the engine reads the conversation and groups **all** consecutive customer messages that
  still have no reply. That is why a "hi" followed by "I'd like a quote" within the window becomes a single
  turn, while a message that arrives after the reply opens another turn.
- The pause is injectable (`Pausa`), so the test proves the window with a controlled clock, without sleeping.
- The **Jev's G2** (wait for more / process now) governs the window since HR-30. With G2 on, the Jev's batch
  goes out **before** the wait (its time fits in the window), and the decision adjusts the wait:
  `esperar_mais` (wait more) doubles the window up to the 8 s ceiling, `processar_agora` (process now)
  reduces it to the short 250 ms confirmation, and the fallback keeps the fixed window. If the customer sends
  another message during the wait, the batch is redone. Details and measurements in `docs/JEV.md` §5-B;
  test in `tests/g2_window.rs`.

## 3. Stable prompt (PRD §6.1)

Message order:

1. `system`: the markdown from the composer (HR-09), **byte-for-byte identical** between turns of the same agent.
   It comes from `agent_instructions.markdown` and is never recomposed during a turn (it may have been edited by hand).
2. `system`: volatile block delimited by `<contexto_do_turno>`: time (UTC), channel, conversation id,
   service state, and summary (when HR-14 writes one).
3. history: the last messages, within the agent's token budget (HR-11).

When RAG (HR-17), files (HR-23), and the Jev (HR-28) come in, they go **after** the prefix, in the volatile
block (or as a following message), never inside it.

The **Jev** came in with HR-28 and **does not** touch the prompt: it runs in parallel with the search, decides the
gates, and writes the result to `turns.jev_json` ([`JEV.md`](JEV.md)). G1 has had effect since HR-29
(it transfers the conversation and ends the turn before the model), G2 since HR-30 (it governs the grouping
window), and G4 since HR-32 (it chooses the transfer department, validated by the code). The effect of G3
(skip the search, HR-31) is still missing. All of them start disabled and only turn on after being measured.

## 4. Tool loop (D-04)

- Only the tools **enabled on the agent** *and* implemented in the process are offered to the model.
  In this task the production registry (`SemFerramentas`) is empty: `consultar_base` is HR-17,
  `buscar_produto`/cart are HR-18/19, `enviar_arquivo` is HR-24, and `transferir` depends on the Jev.
  Each one registers in `RegistroFerramentas` without touching the loop.
- The tool registry **does not depend on the knowledge base** (HR-46f): `enviar_arquivo` and the four cart
  tools use only the database and are always registered; `consultar_base` and `buscar_produto` only appear
  when the embedding index has opened. Without `poph model install`, the request to the model still includes
  `enviar_arquivo`, so the agent can still send files.
- A disabled (or invented) tool call becomes an **error result** for the model, not an execution.
- `max_tool_rounds` limits the rounds: on the last pass the request goes **without tools**, which is how the
  turn ends in text. `max_tool_calls_per_message` limits the calls in one reply; the calls past the limit
  receive a limit error.
- Arguments arrive whole before the tool runs (HR-10 guarantee).

## 5. Streaming, cancellation, and errors

- Each `EventoLlm::Texto` goes to the `Observador` right away: `chat` prints the piece (text) or a JSON line
  (`--json`).
- Ctrl-C in `chat` requests `Cancelamento`; the provider closes the stream with `ErroLlm::Cancelada` and the
  turn is recorded as **failed** with the reason. The partial reply is **not** sent or recorded as an agent
  message: it never reached the customer, and saving it would make the next turn "think" the agent had already
  answered that.
- Provider error: turn `failed` with the reason and nothing in the `outbox`. In `poph serve`, the entry is
  postponed and retried, with a limit (see section 11, "Model failure"). In `poph chat`, the turn ends there, as before.
- **No slot available** (instance, org, or agent slot, or provider call slot): the turn is recorded as failed and
  the entry goes into the same ladder as a model failure (section 11, HR-26k). There is no longer any unlimited
  retry on this path. The wait for a slot inside the turn is `espera_vaga` (5 s).
- `tempo_maximo_turno` (default 180 s) closes the turn with an error instead of leaving the actor stuck.

## 6. Record in `turns`

`timings_json` stores the stages and, alongside them, `modelo` and `motivo_fim` — `turns` has no column of its
own for these two, and HR-04b closed the schema (a new migration requires the maintainer's authorization).

```json
{"fila_ms":2002,"montagem_ms":0,"primeiro_texto_ms":3719,"total_ms":4181,
 "rodadas_ferramenta":0,"modelo":"gemini-3.8-flash-tiered","motivo_fim":"concluido"}
```

- `fila_ms`: from the arrival of the message that triggered the turn until the turn starts (includes the window).
- `montagem_ms`: assembling the prompt and the tools.
- `primeiro_texto_ms`: from the start of the turn to the model's first piece of text.
- `total_ms`: the whole turn (includes the tool rounds).
- `tokens_entrada`/`tokens_saida`: summed across the turn's calls.
- `tools_json`: record of the turn's tools. It stores `{"nomes": [...], "chamadas": [...]}`:
  `nomes` is the list published as `tools` (every call that reached the loop, including the refused one, as
  before), and `chamadas` holds the per-call detail (HR-47b, see below). `custo_micros` stays at 0: the config
  still has no price table per model (known limit from HR-10).

### Record of tool calls (HR-47b)

Each call the model makes goes into `chamadas`, in the order it happened:

```json
{"nome":"consultar_base","rodada":0,"ordem":1,"estado":"executada","motivo":null,
 "duracao_ms":12,"argumentos":"{\"consulta\":\"prazo\"}","resultado":"Prazo de 5 dias úteis."}
```

- `estado` (state): `executada` (the tool ran, even when it returned a content error to the model) or
  `recusada` (refused, did not run).
- `motivo` (reason) for a refusal, only when `recusada`: `nao_habilitada` (not on the agent's list, D-04),
  `desconhecida` (a name the process does not register), `argumento_invalido` (invalid argument),
  `erro_de_execucao` (execution error), `limite_de_chamadas` (per-message ceiling), and `fora_do_turno`
  (G3 at `nada`, or a catalog already consulted in the turn).
- `duracao_ms`: execution time; zero for refusals before running.
- `argumentos` and `resultado`: **text summaries**, cut at 1024 characters with the mark `…[cortado]`
  (cut). In the arguments, a field whose name indicates a secret (`token`, `senha`, `secret`, `chave`,
  `api_key`, `password`, `authorization`, `credencial`) has its value replaced with `[redigido]` (redacted).
  The result is the text the model received, without redaction (it is output from our tools).
- A turn that failed at the provider after already calling a tool records the calls made; a turn without
  calls records `"chamadas": []`.

## 7. Temperature and token limit per agent

HR-03 found the two fields recorded and ignored in POP Atende. Here they take effect: the turn puts
`temperatura` and `max_tokens` in the `Pedido`, and the provider client prefers what came in the request over
the default in the `[llm]` block. This is the only addition made to `poph-core/src/llm/` (two optional fields
in `Pedido` and one line in `montar_corpo`), and nothing changes for anyone who does not set them.

## 8. What the next task consumes

| Task | What to use |
|---|---|
| HR-14 (rolling summary) | `Prompt`/`bloco_volatil` already receives `resumo`; `contexto::Janela::folga()` says how much room is left; generate outside the critical path and save with `Conversas::definir_resumo` |
| HR-17 (RAG) | register `consultar_base` in `RegistroFerramentas`; the search runs in parallel before the 1st call, and the result goes into the volatile block (marked as untrusted content) |
| HR-18/19 (products, cart) | same, through `RegistroFerramentas`; the `ContextoFerramenta` already carries org, conversation, agent, and turn |
| HR-20 (API/SSE) | `Motor::executar` + an `Observador` that publishes the events; `turns` already stores the "why"; `Motor::com_cancelamento` serves the orderly shutdown |
| HR-28 (Jev) | done: `Motor::com_jev`, decisions in `turns.jev_json` and in `ResumoTurno`, in parallel with the search (see `JEV.md`) |
| HR-29 (G1) | done: `Motor::com_handoff`, `AvaliadorGates::orientar`, `Conversas::transferir` (status + audit in one transaction), and the `EventoTurno::Transferencia` event (see `JEV.md` §5-A) |
| HR-30 (gate G2) | replace the fixed pause with "wait more / process now" |
| HR-55 (load) | 500 active conversations: `Atores` + `Motor` are already cheap `Clone` and partitioned by org |

## 9. Known limits

- **No RAG, no Jev, no products, no files**: in this task the model answers only with the instructions and the
  history. The production toolset is empty on purpose.
- **Empty reply**: if the model returns no text (for example, only tool calls up to the limit), the turn closes
  `done` with empty text and **nothing** goes to the `outbox`. The Console will show the turn without a reply;
  there is no retry.
- **Time in UTC**, without local time zone: there is no date crate in the PRD §5.3 list. The Jev (HR-30) is the
  one that needs the time window in local time.
- **`custo_micros` is 0** (no price table in the configuration — limit inherited from HR-10).
- **Grouped message**: the `inbox` items of the messages that entered the same turn are completed together.
  The lookup is `listar_pendentes(org, 500)` per turn; with a queue longer than that, the item left over is
  dispatched again and falls into "message already answered" (turn `skipped`).
- **`--conversa` from another agent** is refused; to switch agents, open a new conversation.
- **Memory starts zeroed.** Each `poph chat` run without `--conversa` opens a new conversation; to continue an
  earlier one, pass its id.
- **Real transport** (Evolution/plugin) is phase 4. `TransporteConsole` confirms text items; `file`/`presence`
  are confirmed without printing, so they are not sent forever.
- **`chat` does not go through the `inbox`**: it records the message and triggers the turn directly
  (`Tarefa::nova`), because a human is waiting for the reply in the same session. The `inbox` idempotency belongs
  to the external channel.
- **Text sanitization**: what the customer writes enters as user content; there is no "untrusted content" marking
  in this phase (it comes with RAG/files, HR-17/23).
- **aarch64**: only `cargo check` on the crates without C code (D-02/D-10); the ARM binary is HR-56.

## 10. Demonstration (19/11/2026, local gateway `127.0.0.1:8045`, `gemini-3.8-flash-tiered`)

Agent created by the CLI (`poph agent create`, `poph instructions create --file`), a 5-message conversation through
`poph chat`. Full transcript in `/tmp/hr13-demo-final/transcricao.txt` (outside the repository); the times come
from `turns`:

| # | queue | assembly | 1st text | total | tokens (in/out) |
|---|---|---|---|---|---|
| 1 | 2002 ms | 0 ms | 3719 ms | 4181 ms | 414 / 43 |
| 2 | 2003 ms | 0 ms | 4492 ms | 4854 ms | 471 / 48 |
| 3 | 2001 ms | 0 ms | 3744 ms | 4189 ms | 529 / 53 |
| 4 | 2003 ms | 0 ms | 5081 ms | 5712 ms | 592 / 78 |
| 5 | 2001 ms | 0 ms | 3704 ms | 4237 ms | 677 / 41 |

10 messages recorded, 5 outgoing items `sent`, no pending entries. The `fila_ms` of ~2000 ms is the grouping
window (the agent's default of 2 s).

## 11. Turn in `poph serve` (HR-26h)

The server answers WhatsApp messages. The path is the same as `poph chat`, with the input coming from the queue
instead of the terminal:

1. The Evolution webhook records the customer's message in the incoming queue (`inbox`) and in the conversation.
2. The dispatch loop (`crates/poph-server/src/turn_input.rs`) reads the pending queue every
   `[turno] intervalo_despacho_ms` (default 1000 ms; accepts 100 to 60000). It dispatches **only the input**: it
   sends nothing. The output is handled by the WhatsApp loop (`whatsapp_output.rs`).
3. The conversation actor runs the turn. One turn at a time per conversation is guaranteed by the actor itself.
4. The reply goes to the conversation as `out` and to the outgoing queue; the WhatsApp loop delivers it.

Turn assembly (engine, model, base, products, Jev, transfer) is the same in both commands:
`crates/poph/src/assembly.rs`. `chat` keeps its own metrics, outside `/metrics`; `serve` publishes its metrics
on `/metrics`.

Without the provider key (`llm.chave_env`, `ANTIGRAVITY_API_KEY` by default) the server still starts and keeps
receiving: messages wait in the incoming queue and are answered once the server is restarted with the key. With
the key, the base and the agent catalog of the configured instances are checked before attending.

### When it does not reply

The message stays recorded and the queue item is completed **without a reply**, with the reason on the turn
(state `ignorado`, which means skipped). It is not marked as a failure. Cases:

- conversation transferred to a human, or closed (until someone reopens it);
- agent inactive or archived;
- WhatsApp integration inactive or archived, or no longer exists.

### Without the model key

Without `ANTIGRAVITY_API_KEY`, `poph serve` starts normally and keeps receiving messages, but does not run turns.
The warning is printed at startup. Messages stay in the queue (they are not lost) and are answered when the server
starts with the key. Changing the key or the interval requires restarting the server.

### Resuming at startup

- A pending message in the queue (it arrived while the server was stopped, or with turns turned off) is dispatched
  on the first pass of the loop.
- A reply that was being sent when the process crashed (`sending`) is resent by the WhatsApp loop. The risk of a
  duplicate when the crash happens between sending and confirmation follows what has already been accepted (D11).
- A message that was in a turn when the process crashed stays in the queue. When it is retried, the turn checks
  the history: if there is already a reply after it, it does not reply again.

### Model failure: retry later, with a limit (HR-26i)

When the model fails in the middle of a turn (provider down or erroring after the client's own retries, connection
dropped, or the turn's maximum time exceeded), the message is **not** left without a reply silently. The ladder is:

1. The attempt fails. The turn stays `falhou` (failed) with the reason, as before.
2. The queue items that this turn would have answered (the message and those in the same group) are **postponed**:
   they stay in the queue, do not go to the outgoing queue, and count one more attempt. The time of the next attempt
   is stored in the database (column `next_attempt_at` of `inbox`), so the postponement survives a restart.
3. Once that time has passed, the dispatch loop picks up the item again and runs a new turn.
4. If the attempt succeeds, the reply follows the normal path (conversation, `outbox`, WhatsApp).
5. If the limit runs out, the entry is **completed without a reply**, with the reason stored, and the turn stays as a
   final failure. It does not return to the queue.

Defaults, all under `[turno]` in `poph.toml`:

| Key | Default | Range | What it does |
|---|---|---|---|
| `tentativas_falha_modelo` | 3 | 1 to 10 | Total attempts, counting the first. |
| `atraso_falha_modelo_ms` | 30000 | 1000 to 600000 | Wait before the second attempt. Each following wait is four times the previous one, up to 10 min. |

With the defaults, the waits are 30 s (before the 2nd attempt) and 120 s (before the 3rd). With
`tentativas_falha_modelo = 1`, the first failure is final. Changing it requires restarting the server.

Rules that apply to every retry:

- **Does not duplicate a reply.** Before calling the model again, the turn redoes the analysis of the group: if the
  agent has already replied after the message (for example, the previous turn saved the reply and crashed before
  closing the entry), it completes without calling the model.
- **Does not retry if something has already gone out.** If the failed turn had already queued output for the customer
  (today, a library file sent before the failure), the failure is final and the entry is completed with the reason.
  Retrying would send the file again. The customer receives the file and does not receive the text.
- **Postponed items do not go to the output.** Nothing goes to the `outbox` because of the postponement.

Classification (HR-26i and HR-26k):

- **Enters the ladder:** model failure (provider down after the retries, dropped connection, turn's maximum time
  exceeded) and no slot available (turn or provider). No call reaches the model when there is no slot, so the retry
  does not duplicate anything.
- **Does not retry, warns right away (HR-26k):** provider refusal (`Recusado`: key refused (401), request refused (400),
  and other codes that are neither 429 nor 5xx). The entry is completed without a reply on the first attempt, the turn
  stays `falhou`, and a `WARN` is logged with the sanitized reason.
- **Does not retry:** local configuration error, invalid request, and cancellation.

The no-slot case uses the same `tentativas_falha_modelo` and `atraso_falha_modelo_ms`. There is no separate knob: it is
the same capacity ladder, and a separate knob would only add another setting for the same effect.

Metrics on `/metrics`: `poph_turnos_falha_modelo_adiadas_total` (scheduled retries, includes no-slot),
`poph_turnos_falha_modelo_definitivas_total` (messages completed without a reply: after the limit or because of a provider
refusal), and `poph_turnos_falha_modelo_recusadas_total` (refusals only; part of the final ones).
Logs: one `WARN` line per postponement (with the attempt, out of how many, and the wait), one per give-up
(`tentativas esgotadas; mensagem concluída sem resposta`, "attempts exhausted; message completed without a reply"), and one per
provider refusal (`provedor recusou o pedido: ... confira a chave e o modelo do agente`, "provider refused the request: ...
check the agent's key and model"). All with the sanitized reason.

### Record and metrics

- One `INFO` line per turn that ends, with org, conversation, turn, state, duration, and tokens. A turn without a reply
  goes out as `INFO` with the reason. A failed turn goes out as `WARN` with the reason.
- No log line carries the customer's or the reply's text, phone number, JID, key, or secret. The reason is cut at
  160 characters, and long numbers and identifiers with `@` are hidden.
- On `/metrics`: `poph_turnos_iniciados_total`, `poph_turnos_concluidos_total`, `poph_turnos_falhos_total`,
  `poph_turnos_ignorados_total`, the two model-failure counters (section above), and the duration measures.

### What it does not do (and limits)

- **Model failure with no reply at the end.** After the attempts run out, the message is completed **without a reply**
  (there is no instability message to the customer). The decision was to retry with a limit, not to warn the customer.
- **Per-minute quota of the org does not enter the turn.** The limiter has the accounting (`consumir_cota`), but the turn
  path does not call it today, so it does not limit or postpone anything. If it is turned on, it enters the same ladder.
- **No-slot is proven in the engine, not in the full server.** The server test with a fake Evolution does not hold an
  occupied slot; the no-slot ladder is proven in `poph-agent`.
- **Provider refusal that is transient in practice.** A 400 caused by a momentary provider failure counts as a refusal and
  is not retried. Today the classification does not distinguish the two.
- **TLS failure is not retried, but enters the ladder.** The provider marks TLS as non-retryable; the turn still treats it
  as a model failure (HR-26i decision, unchanged in this round).
- **Turn interrupted by the process** stays with state `rodando` (running) in the history. The message is retried at
  startup, but the old record is not closed.
- **Interval and keys** are read at startup: changing them requires restarting the server.
- **Proven with a fake LLM** (`ProvedorFake` in the test, a fake SSE server in the smoke test) and a fake Evolution.
  No test talks to the real model or to a WhatsApp number.

### How to prove it

```
cd apps/pop-harness
cargo test -p poph-server --test turn_whatsapp --locked
```

Seven cases: a message becomes a reply once (with a count of calls, and the key only in the header); a redelivery does not
open a second turn; a transferred conversation and an inactive integration do not reply; without the key the message stays
in the queue and is answered at startup; a reply that was being sent when the crash happened goes out at startup; metrics on
`/metrics`; logs without sensitive data (with a capture that proves the line exists).

Model failure (HR-26i), with a fake provider and a fake Evolution:

```
cargo test -p poph-agent --test turn --locked failure_model
cargo test -p poph-agent --test file_library --locked failure_model
cargo test -p poph-server --test turn_whatsapp --locked model_failure
```

No slot and provider refusal (HR-26k), with a fake provider and a fake Evolution:

```
cargo test -p poph-agent --test turn --locked missing_slot
cargo test -p poph-agent --test turn --locked provider_refusal
cargo test -p poph-server --test turn_whatsapp --locked provider_refusal
```

Proofs: no turn slot in every attempt (three failed turns, no call to the model, nothing in the output, no retry after the
limit); no slot on the first try and a reply on the second (one call, one reply, one output); provider slot missing with the
same ladder (no call goes out while the slot is occupied); provider refusal (401) with a single call, no postponement, with
a metric and a warning in the log without sensitive data.

Proofs: two failures and one reply (three calls to the model, one output, increasing wait before each attempt, nothing
dispatched before the time); three failures (three calls, entry completed, no output, no new attempt afterwards); a one-attempt
limit; maximum time enters the same ladder; a reply already saved does not call the model; a postponed item leaves the queue
together with the customer's new message; a file already sent is not repeated; through the server, metrics and logs without
sensitive data.

## 12. Console test chat: SSE, attachments, and cancellation (HR-22)

The Console's test chat tab (HR-46) talks to the same engine as production: the route
`POST /admin/v1/orgs/{org}/agents/{agente}/test-chat/{conversa}/messages` records the customer's message and runs a turn
directly on the `Motor`, with the same provider, Jev, base, products, and tools as `poph serve`. It is not a separate test engine.

- **Test conversation:** a normal conversation with `channel = "test"` and no integration, created by `POST …/test-chat`.
  Zeroed memory is a consequence of being new (`summary` is null); the full history (messages, attachments, turns) comes back
  through `GET …/test-chat/{conversa}`.
- **SSE:** the turn runs in a task, and each `Observador` event becomes a `text/event-stream` event with a sequential `id`:
  `mensagem` (what was recorded, before the turn opens), `inicio` (start), `texto` (deltas in order), `base`, `ferramenta`
  (tool), `transferencia` (transfer), and `fim` (end) or `erro` (error). The `Fim` of the engine is not passed through: the SSE
  `fim` is built from the `ResumoTurno`, with turn, tokens, errors, and times. Contract in `docs/API.md`.
- **One turn per conversation:** the reservation is in memory, per instance (`Estado::reservar_turno_de_teste`). With a turn in
  flight, the other connection receives `409 turno_em_andamento` (turn in progress) before any recording — the second tab's
  message does not enter the turn that is already running.
- **Disconnection:** the SSE flow carries a guard with the turn's cancellation and the reservation. When the client closes the
  connection, cancellation is triggered, the turn ends `failed` with the reason (`Cancelada`, cancelled), and **nothing** goes to
  the `outbox`: a partial reply never reached the client. The reservation is released at that same instant, so the conversation
  accepts the next message.
- **Output:** the text has already gone out through SSE; when it finishes, the server confirms the console's `outbox` with
  `TransporteConsole::silencioso` (silent console transport), so the item does not stay pending in the queue.
- **Attachments:** the body is `multipart/form-data` (`text` + `file`, repeatable), and each file goes through
  `poph_files::receber` from HR-23 (16 MB, type detected by content, extraction isolated in `poph-extrator`). The extracted text
  enters the prompt through the HR-35 path, between the untrusted-content marks, and the image goes as a multimodal part (§13).
  Body limit for the route: 24 MB.
- **Without the model key** there is no engine in the state: the route responds `503 turno_indisponivel` (turn unavailable) before
  recording the message.

## 13. Customer image in the prompt (HR-23b)

The turn reads the attachments of each customer message in the history. Until HR-22, only the extracted text
(`attachments.extracted_text`) went into the prompt: an image was recorded, appeared in the history, and **did not** reach the
model — a message with only an image produced a request with no content. Now:

- **The image goes as a multimodal part.** An attachment whose real MIME type is `image/jpeg`, `image/png`, `image/gif`, or
  `image/webp` becomes `Parte::Imagem { mime, bytes }` in the same position where the file's text used to go: in a user message
  **after** the customer message that brought the file. The bytes come from the blob (`data/blobs/<org>/<sha256>`) by the hash stored
  in `attachments`; nothing is re-extracted.
- **A document stays data, never instructions.** The extracted text of the PDF/Office file keeps the `AVISO_NAO_CONFIAVEL` notice
  (untrusted-content warning) and the marks of the untrusted block (HR-35). A message with an image and a document carries both:
  the marked text and the image.
- **File-only message.** If the message text is empty and there is an attachment, the blank text message does not go into the
  request: the content is the file's part. Before, the request went with an empty text.
- **Turn ceilings** (constants in `crates/poph-agent/src/turno/mod.rs`), counted over the whole context window, not per message:
  - `MAX_IMAGENS_POR_TURNO` = **4** images;
  - `MAX_BYTES_POR_IMAGEM` = **5 MB** per image (intake accepts 16 MB; in the request the image becomes base64, ~4/3 of its size);
  - `MAX_BYTES_DE_IMAGENS_POR_TURNO` = **8 MB** in total.
  What goes over the ceiling is **omitted** with `tracing::warn!` (attachment id and size, never bytes or base64) and the turn continues.
- **Degradation.** Without the file configuration in the engine (`Motor::com_arquivos`), without a blob on disk, invalid hash, or a blob
  above the read limit: the attachment is left out, with a log entry, and the turn **never** fails because of the image. The document
  text, if any, still goes.
- **Who turns the configuration on:** the composition root (`crates/poph/src/assembly.rs`) passes `FileConfig::default(configuracao.blobs())`
  to the engine. It is the same folder that intake and the test chat use to write the blob.
- **Provider that does not accept images:** there is no per-provider capability declaration today — `validar_pedido` in `poph-core`
  refuses MIME types outside the list and empty images before sending, and the configured providers (antigravity) accept images. If a
  provider starts refusing them, the current path is a turn failure (the HR-26i retry), not a send without the part: degradation by
  capability waits until that configuration exists.
- **How to prove it:** `cargo test -p poph-agent --test image_attachment` (5 tests: image only, image + document, missing blob, ceiling,
  and turn without file configuration).

The turn tests capture logs to check that raw image data does not leak. The G7 file-injection tests seed
customer attachments directly in the `attachments` table so they can exercise the turn independently
of channel ingestion.

## 14. Crash and recovery (HR-57)

What happens when the process dies (`kill -9`, power loss, OOM) in the middle of a service. The rule is: **the customer's message is
not lost, the turn does not get stuck as "in progress", the conversation goes back to service, and the reply does not go out twice.**

- **Guarantees that already came from the queue.** The `inbox` item is only completed when the turn ends (`Fluxo::concluir_entrada`); a
  crash in the middle leaves the item pending and startup dispatches it again (at least once). If the reply was already saved after the
  customer's message, the turn redoes the analysis (`analisar`) and completes as `skipped` without calling the model — this is what
  prevents a duplicate reply (HR-12).
- **An interrupted turn does not stay `running`.** At startup, `turn_input::recover_interrupted_turns` closes as `failed` every turn
  that is still `running` and started **up to** the startup instant, with the error `interrompido: o processo caiu no meio do atendimento; a
  mensagem foi mantida` (interrupted: the process crashed in the middle of the service; the message was kept)
  (`Turnos::falhar_orfaos`, additive read/write, no migration). A turn opened in this instance after startup is never touched. The sweep runs
  in `servir_em`, before any service.
- **Reply and outgoing queue in the same transaction.** The agent's reply and the `outbox` item are saved by `Mensagens::gravar_com_saida` in a
  single write (the same goes for the transfer message, the G7 message, and the G8 message). Before, they were two writes: a death between them
  left the reply in the history and never sent to the customer.
- **Output stuck in `sending`.** A WhatsApp item in `sending` is resent by the outgoing loop (D11 — it may duplicate on the channel, which is the
  price of at-least-once delivery). A console conversation item (no integration) is confirmed at startup by `enviar_pendentes_do_console` with the
  silent transport: the text was already shown to the operator through the stream, and the item must not get stuck.
- **"One turn per conversation" lock (test chat).** The reservation is in memory, per instance, with a 30-minute validity
  (`Estado::VALIDADE_DA_RESERVA`). A crash takes the memory with it, so the conversation accepts a message on the first request after startup;
  the validity exists for a reservation that escapes inside a live process (bug, cancellation that did not go through `Drop`).
- **Provider failure is a controlled state, not a hung turn.** Provider down, 5xx, timeout, or a cut stream: the turn ends `failed` with the
  reason recorded, with no invented reply and no partial text becoming a reply. On WhatsApp, the model failure enters the HR-26i retry ladder
  (with a limit) and, if output was already queued, does not retry, to avoid duplication.
- **Jev failure does not bring down the turn.** The batch that fails becomes a fallback recorded in `turns.jev_json.chamada.falha` and the turn
  continues (`cargo test -p poph-agent --test jev_turn`).
- **Known limit:** a message from the Console's test chat (`channel = "test"`) is **not** in the `inbox`. If it dies before the turn ends, it stays
  recorded and without an automatic reply after startup (the operator resends it). The channel path (WhatsApp) resumes and replies once.
- **How to prove it:**
  - `cargo test -p poph-server --test crash_resilience` — orphan turn closed at startup, conversation goes back to service, provider unavailable,
    cut stream, expired reservation, and console output stuck in `sending`;
  - `cargo test -p poph-server --test turn_whatsapp` — `interrupted_turn_after_crash_resumes_once_on_start` (one reply, one send) and
    `reply_stuck_in_sending_after_crash_goes_out_on_start`;
  - `cargo test -p poph-store --test storage` — `orphan_turns_closes_only_those_started_before_boot` and
    `response_and_output_item_enter_same_write`;
  - run with the real binary, `kill -9` at six points, in `/tmp/hr57-run/queda.py`.

The shared server state keeps test-chat reservations by conversation. The stream releases its reservation on
completion, error, or disconnect. The 30-minute reservation TTL recovers a slot if that release is missed
inside a live process. Reservations are in memory and disappear when the process ends.

## 15. Flows in the turn (FX-23, FX-24, FX-25)

When the engine is on (`[fluxos] ativo = true`), a turn runs the flow step before the text model is called:

1. **Active execution on the Case.** The block of the current step (`poph_flows::render_block`) goes into the
   volatile part of the prompt. The Jev is not asked: a Case has at most one active execution.
2. **No active execution and flows granted to the agent.** The state sent to the Jev carries the list (name and
   trigger) and gate G9 chooses one. A confident choice is validated by the code (flow of the org, active, granted to
   the agent of the Case, published revision) and opens the execution in one write: the first step, the
   `fluxo_iniciado` event and the entry effects. The prompt starts with the block of the entry step. See `JEV.md` §5-I.
3. **Reserve.** If the Jev is down, the gate is off, the answer is unusable or the choice fails validation, nothing
   starts and the model receives the catalogue of flows with the `fluxo_iniciar` tool.
4. **`nenhum` or low confidence.** Nothing starts, and the catalogue does not go into the prompt.

Tools offered to the model (filtered by the agent's enabled list, D-04):

| Tool | When it is offered | What it does |
|---|---|---|
| `fluxo_executar_acao(acao, campos?, chave_idempotencia?)` | with an active execution | validates and records the action of the current step, actor `agente` |
| `fluxo_abandonar(motivo)` | with an active execution | abandons the execution, actor `agente` |
| `fluxo_iniciar(fluxo_id)` | only in the reserve (step 3) | validates like G9 and opens the execution |
| `atendimento_encerrar` | always registered, whatever the engine setting | closes the conversation and its Case (FX-25) |

Rules that the code keeps: the actor is set by the code, never taken from the model's arguments; there is no
auto-transition (an action that is not valid on the current step is refused with the list of valid ones, and nothing
is recorded); the transition is written atomically by `Store::execucoes()` (`aplicar_transicao`); effects are queued
and delivered after the text of the turn is final; the model's answer has no internal ids.

A database or read error inside this step never breaks the turn: the turn continues without a flow, and the reason
goes to the log.

Tests: `poph-agent/tests/flow_turn.rs`, `poph-agent/tests/flow_tools.rs`, `poph-jev/tests/g9_fluxo.rs`; end to
end in `poph-server/tests/fluxos_e2e.rs`. Details in [`FLUXOS.md`](FLUXOS.md) §3.

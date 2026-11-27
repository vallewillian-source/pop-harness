# Input and output — idempotency, durable queue and resumption (HR-12)

> Depends on HR-07 (storage) and HR-11 (actors).
> Code: `crates/poph-agent/src/entry_output.rs`; persistence in
> `crates/poph-store/src/repos/caixas.rs` (`inbox`/`outbox`).
> Decisions that govern this: PRD §6 steps 1 and 8, D11 (delivery at least once), D-05
> (inactive agent does not open a turn), D-07 (2,000 msg/s per instance).

## 1. The design at a glance

```
webhook ──► Fluxo::receber_mensagem ──► [single transaction] messages + inbox ──► ACK to the channel
                                              │
                                              ▼
                       Fluxo::despachar (or cycle) ──► conversation actor (HR-11)
                                              │             │
                                              │             ▼
                                              │      turn (HR-13): LLM, tools...
                                              │             │
                                              │             ├─► outbox.enfileirar (turn_id, seq)
                                              │             └─► Fluxo::concluir_entrada
                                              ▼
                        Fluxo::enviar_pendentes ──► TransporteSaida (Evolution/plugin/console)
                                     pending ──► sending ──► sent | failed
```

Two durable queues, no critical state held only in memory:

| Where | What it guarantees |
|---|---|
| `inbox` | the customer's message enters once (unique key `(integration_id, external_message_id)`) and only leaves the queue when the turn completes |
| `outbox` | the reply leaves once per `(turn_id, seq)`; `sending` survives a crash and is resent on startup (D11) |
| memory (`Fluxo`) | who is already in flight, who is being sent right now, and when each item can be retried (backoff). Lost on restart on purpose: the database is the source of truth |

Two **in-memory reservations** avoid duplicate work when two cycles run at the same time in the same process: one
for the inbound item (it stays reserved from dispatch until the turn completes) and one for the outbound item (it
stays reserved only while being sent). They do not replace the database — what guarantees "once" after a crash is
`inbox`/`outbox`.

## 2. States

### Inbound (`inbox`)

| Column | Meaning |
|---|---|
| `processed_at` null | still in the queue; it will be dispatched |
| `processed_at` filled in | the turn completed — successfully or with a turn error |
| `attempts` / `last_error` | how many times it was completed and why, when there was an error |

The item is **not** marked as processed at dispatch. That is what makes the `SIGKILL` test pass: if the process dies
with the event already handed to the actor (but not yet processed), the `inbox` stays pending and resumption
dispatches it again. The cost is "at least once" on input as well.

### Outbound (`outbox`)

| State | Meaning | Who writes it |
|---|---|---|
| `pending` | created by the turn, not yet attempted | `enfileirar_saida` |
| `sending` | **already recorded before calling the channel**; if the process dies here, resumption resends | `iniciar_envio` |
| `sent` | the channel confirmed | `confirmar_envio` |
| `failed` | definitive error or attempts exhausted | `falhar_envio` |

`attempts` counts **real send attempts** (not state transitions). A transient failure keeps the item in `sending`
with `last_error`, and the in-memory backoff decides when to try again; if the process restarts, the item is retried
right away (the in-memory backoff is lost — this is the desired behavior on startup).

## 3. What HR-13 consumes

```rust
// 1. Write the customer's message and the inbox item in one transaction, and only then ACK.
let gravado = fluxo.receber_mensagem(org, MensagemRecebida {
    integration_id, id_externo, conversa, texto, anexos_json,
})?;                            // RecebidoComMensagem { situacao, item, mensagem }
// gravado.situacao == Recebido::Duplicado → answer OK to the channel and do not generate a turn

// 2. Dispatch (the server worker calls ciclo at each interval, or this):
fluxo.despachar(org)?;          // RelatoDespacho { despachados, adiados, invalidos, desistentes }

// 3. In the actor's processor (implements poph_agent::atores::Processador):
fn processar(&self, tarefa: Tarefa) -> Futuro {
    // tarefa.entrada: Some(inbox_id) when it came from an integration, None in the CLI chat
    // ... turn: context window, Jev, RAG, LLM, tools ...
    // reply:
    fluxo.enfileirar_saida(org, NovaSaida { conversation_id, turn_id, seq, tipo, conteudo_json })?;
    // at the end, always (with or without error):
    if let Some(entrada) = tarefa.entrada {
        fluxo.concluir_entrada(org, entrada, erro_do_turno)?;
    }
    Ok(())
}

// 4. Sending (the real transport comes in HR-25/HR-26; in CI it is the TransporteDeTeste):
fluxo.enviar_pendentes(org, transporte.as_ref()).await?;

// 5. On process startup, before accepting new work:
fluxo.retomar(transporte.as_ref()).await?;   // resends what was left in sending and drains the inbox
```

Other useful signatures: `fluxo.estatisticas(org)` (queue sizes), `fluxo.orgs()`,
`fluxo.atualizar_medidores()`, `fluxo.iniciar_laco(transporte)` (background loop every
`intervalo_ciclo`).

`Tarefa` (from `atores`) gained the field `entrada: Option<InboxId>` and the constructors `Tarefa::nova`
(no inbox) and `Tarefa::da_entrada`. It is the only hook the dispatch needed in HR-11; nothing else in the actor
changed.

## 4. Resumption on startup

`retomar` does, in this order:

1. counts the items in `sending` (these are the ones that may have left without confirmation) and adds them to
   `poph_fluxo_reenvios_retomada_total`;
2. dispatches the pending `inbox` of all orgs;
3. sends `pending` + `sending` of all orgs.

Resending `sending` may duplicate the message on the channel. This is decision D11 (WhatsApp does not recognize
resends; the alternative would be losing the reply).

## 5. Backoff

`Backoff { base, fator, teto }`, default 500 ms × 2 up to 60 s. It applies to:

- dispatch refused by the actor (queue full, actor stopped);
- sending with a transient failure.

`max_tentativas` (default 8) decides when to give up:

- **inbound**: the item stays recorded with `last_error` and `processed_at` filled in (the customer's message is in
  `messages`, nothing is lost, but it is not answered) and counts in `poph_fluxo_entradas_desistentes_total`;
- **outbound**: it goes to `failed` with a readable reason and counts in `poph_fluxo_saidas_esgotadas_total`.

An invalid payload in the `inbox` does not enter a retry loop: it is recorded with the parser's error and counts in
`poph_fluxo_entradas_invalidas_total`.

## 6. Metrics (no HTTP; HR-20 publishes them at `/metrics`)

| Name | Type | What it measures |
|---|---|---|
| `poph_fluxo_inbox_pendentes` | gauge | items not processed, summed across orgs |
| `poph_fluxo_outbox_enviando` | gauge | items in `sending`, summed across orgs |
| `poph_fluxo_entradas_novas_total` | counter | new deliveries |
| `poph_fluxo_entradas_duplicadas_total` | counter | redeliveries discarded |
| `poph_fluxo_entradas_invalidas_total` | counter | unreadable payload |
| `poph_fluxo_entradas_adiadas_total` | counter | dispatches postponed by backoff |
| `poph_fluxo_entradas_desistentes_total` | counter | dispatch gave up after the attempts |
| `poph_fluxo_entradas_concluidas_total` | counter | items completed by the turn |
| `poph_fluxo_saidas_enviadas_total` | counter | replies confirmed |
| `poph_fluxo_saidas_transitorias_total` | counter | failures that will be retried |
| `poph_fluxo_saidas_definitivas_total` | counter | failures that will not be retried |
| `poph_fluxo_saidas_esgotadas_total` | counter | items that became `failed` after attempts |
| `poph_fluxo_reenvios_retomada_total` | counter | items in `sending` on startup |

The gauges are per instance (summed across orgs). Counting per org is HR-21/HR-20, which has labels.

## 7. How the tests prove it

`crates/poph-agent/tests/input_output.rs` — 14 tests, all in `scripts/ci.sh`:

| Required group | Test |
|---|---|
| same message 2× = 1 processing | `mesma_mensagem_entregue_duas_vezes_e_processada_uma_vez` (the 2nd delivery answers `Duplicado`, a single turn) |
| `SIGKILL` between recording and processing | `sigkill_entre_gravar_e_processar_nao_perde_a_mensagem` (real child process killed with `SIGKILL`; resumption processes it and the customer's message is still there) |
| `SIGKILL` between `sending` and `sent` | `sigkill_entre_sending_e_sent_reenvia_uma_vez` (resend 1 on resumption; a second resumption does not resend) |
| repeated failure → `failed` | `falha_repetida_vai_para_failed_com_motivo_e_contador` (3 real attempts, readable reason) |
| backoff grows and stops at the cap | `backoff_cresce_e_para_no_teto` + `item_em_backoff_nao_e_reenviado_antes_da_hora` (injected clock, no sleeping) |
| no read crosses orgs | `caixas_nao_cruzam_org` |

Extras: `falha_definitiva_nao_retenta`, `saida_idempotente_por_turno_e_sequencia`,
`fila_cheia_adia_e_nao_perde_o_evento` (queue of size 1: the postponed items are processed later),
`dois_despachos_concorrentes_nao_duplicam_o_evento` and `dois_envios_concorrentes_nao_duplicam_a_saida`
(the in-memory reservations).

There is also `medir_ingestao`, marked `#[ignore]`: it measures the throughput and latency of the atomic ingestion
ACK. It does not run in CI because it is a measurement, not a check:

```
cargo test --release -p poph-agent --test input_output -- --ignored --nocapture measure_ingestion
```

The two `SIGKILL` tests run the same test binary again with `--exact <child name>`: the parent waits for the child to write a marker (the controlled point) and sends `SIGKILL`.
There is no simulation.

## 8. What is not covered

- **Field encryption and integration secrets** (HR-26): the `inbox` payload is text.
- **Forwarding between instances** (PRD §12): here it is always the local instance.
- **Real transport**: only the trait. Evolution and plugins come in phase 4.
- **Cancelling a turn in progress** (HR-13).
- **Retention/cleanup of the `inbox`** (HR-51): nothing deletes old processed items.
- **Concurrency between processes**: the design is one writer per process and one process per database. Sending for
  an org within a cycle is sequential (a slow channel delays the following items of that pass); orgs are also walked
  in order. Parallelizing sending per org is HR-20.
- **Orphan reservation**: if the turn accepts the event and never calls `concluir_entrada` (a defect in HR-13), the
  item stays reserved until the process restarts. On startup, resumption picks it up again.
- **Long load** (HR-55): memory leak of the in-memory schedules (`agenda_entrada`/`agenda_saida` only lose an entry
  when the item is completed/sent; an item that is never retried because it is completed leaves). No test over hours.
- **ARM64**: `poph-agent` depends on `poph-store` (SQLite in C), so it stays out of the workstation's aarch64
  `cargo check` (D-02/D-10); verification and measurement are HR-56, on the ARM64 test machine.

## 4. Closed conversation and the next message (MX-06)

Closing a conversation (`POST …/conversations/{id}/close`) changes its status to `closed`. It stops receiving messages
and stops counting as an active service. When the customer writes again, the intake only looks for `open` or
`transferred` conversations of the same channel and contact. If it finds none, it opens a new conversation, with the
integration's agent and an empty history. The closed conversation stays in the database, with its messages, and is
not reopened.

Proof: `mensagem_depois_do_fechamento_abre_conversa_nova` in `crates/poph-server/tests/api.rs`. The behavior already
existed (the search in `caixas.rs` filters `status IN ('open', 'transferred')`); MX-06 only documents it and proves it.

## 5. The human attendant's reply (MX-18)

When an attendant replies through Element, the event arrives through the Matrix Application Service routes
(`api/matrix_as.rs`) and is processed in `matrix_response.rs`. The reply enters the `outbox` through the same atomic
operation as MX-04 (`Mensagens::registrar_resposta_humana`): a message (`direction out`, `role assistant`,
`origin human`, with the author's name and the Matrix event id in `external_id`), a queue item with `text` and `seq 0`,
a conversation moved from `open` → `transferred`, and an `human_takeover` audit record, all in a single write. From
there on, the path is the same as any reply: the outbound consumer picks up the item and delivers it through Evolution.

Three details matter for anyone changing this:

- **Idempotency is the `event_id`.** Synapse resends the transaction when it does not receive a 2xx; the second write
  returns `Duplicada` and nothing is queued again. A resend never becomes a second send.
- **The queue text is in Markdown**, `**[Nome]**:` followed by the body as the attendant wrote it. The conversion to
  the WhatsApp format (`to_whatsapp`, which also cuts at 4,000 with `…`) happens **once**, at sending — converting
  here would make sending convert again and `**bold**` would arrive as italics.
- **Our database failure becomes 5xx**, never 4xx: Synapse treats any non-2xx as "Application Service down" and holds
  the queue for minutes. Unknown room and closed conversation are `200` (for the closed one, with a notice in the room
  for the attendant).

Proof: `crates/poph-server/tests/matrix_resposta.rs` — 9 end-to-end tests with a fake Synapse and a fake Evolution
(prefix and single send, `transferred`, resend without a second send, isolation between orgs, closed conversation,
unknown room, database failure with resend, media, name cache).

## 6. Customer name on intake (MX-32)

`receber_do_canal` records the customer's name (`pushName`, in `metadados["push_name"]` of the event) together with the
message, in the same transaction. The name is written in its own savepoint: if it fails, the message still gets in,
and the failure shows only as a log line with a short code. A repeated message (redelivery) does not rewrite the name.
Rules and columns: `DADOS.md`, migration `0008`.

Proofs: `crates/poph-store/tests/customer_name.rs` (delivery records the name, a name failure does not drop the message,
redelivery does not rewrite it) and `crates/poph-server/tests/webhook_evolution.rs` (create with the name, last name
wins, cleanup, group and echo do not record, isolated failure, log canary).

## 7. The conversation state in the room (MX-15)

The message loop carries the text to the room; the state loop (`matrix_state.rs`) carries what happens to the
conversation. On each pass it reads the changed conversations (cursor on `updated_at` + id, `DADOS.md` §11.5) and
compares the situation with what the room has already mirrored:

- **`open → transferred`**: notice "atendimento humano assumiu" ("human service took over", with the department, when
  there is one) and metadata rewritten. This is what the attendant sees when they take over the conversation in Element.
- **`transferred → open`**: notice "o agente automático retomou o atendimento" ("the automatic agent resumed the service").
- **`* → closed`**: notice "conversa encerrada" ("conversation closed") and metadata with `status = closed` — the only
  situation value that Element distinguishes.
- **Same situation**: no notice and no write to Synapse. The conversation is re-evaluated on each change (a new
  message raises `updated_at`), but it only writes when the situation changes.
- **New department** (transfer to another department): the room changes space (`m.space.child` in the new one, leaves
  the old one) without a repeated notice.
- **Customer name that arrives later** (MX-33): the room is renamed (`m.room.name`), the metadata is rewritten with the
  conversation's current state, and the customer's ghost receives the new name; **without** `m.notice` — renaming is
  not a service event. The name is only marked as mirrored (`matrix_rooms.client_name_espelhado`, `DADOS.md` §11.7)
  after the three calls, so repeating the pass writes nothing.

The notice goes out through the Application Service bot when it is already in the room; since MX-11 does not invite the
bot to the service room, the normal case today is sending through the agent's ghost — the room's owner. The notice's
`txnId` is (conversation, instant of the change, situation): repeating the pass after a failure does not duplicate the
notice, and a second transfer of the same conversation has its own notice.

Proofs: `crates/poph-server/tests/matrix_estado.rs` — 23 tests with a real database and a fake Synapse (transitions,
repetition without writing, situation race, space change, transient failure without advancing the cursor, two
conversations in the same millisecond, room being created, organization turned off, and the customer's name arriving
after the room).

Wired into `poph serve` by MX-20 (`matrix_beginning.rs`).

## 8. Wiring on the server (MX-20)

See [Matrix runtime rules](MATRIX.md) for the worker behavior and operational limits.

The Matrix module enters `poph serve` through `matrix_beginning::start`, called in `servir_em` before the server accepts
connections. It assembles the ready parts — client, ghost ids, spaces and the three real ports over the database —,
injects the process numbers (`Estado::matrix_metrics`) and starts the message loop (MX-14), the state loop (MX-15)
and the Application Service receiver with delivery of the attendant's reply (MX-16 + MX-18).

- **Off** (`[matrix]` absent or `habilitado = false`): no task enters the runtime, no `poph_matrix_*` is declared, and
  the Synapse routes answer `404` (A8). If a token is missing with the module on, the same happens — with a short error
  code in the log, without bringing the server down.
- **Turning an org on and off** does not require a restart: both loops reread `matrix_orgs` on each cycle/pass.
  Turning an org off pauses its work until the next cycle; what was written while it was off stays out on purpose
  (turning it back on moves the backfill cutoff, P8).
- **Synapse down** does not stop the server from serving: a light probe (`GET /_matrix/client/versions`) runs every
  30 s and marks the module as connected or degraded, with the wait doubling up to 60 s while it keeps failing. The loops
  retry at their own pace; the customer's turn and output do not depend on the module. The state is readable at
  `Estado::matrix_conectado()` (`None` = module off).
- **Orderly shutdown** (`Ctrl-C`): the loops receive the stop request and finish what they are doing before leaving,
  with a 10 s wait per task (`SHUTDOWN_TIMEOUT`). If a task exceeds that wait, the server logs a warning and moves
  on. The cursor advances after success, so the next startup continues from the same point. Repeating the send uses
  the same `txnId`.
- **Customer name in the room**: the provisioning port passes `conversations.client_name` (MX-32) to
  `ConversationToRoom.client_name`, so the room and the ghost are created with the name WhatsApp sent; without a name,
  they fall back to the identifier.

Proofs: `crates/poph-server/tests/matrix_inicio.rs` — 8 tests with a real database and a fake Synapse (off, with no
task and `404`; Synapse down at startup without stopping `serve`; probe marking connected and going back to degraded;
room created with the customer's name and a mirrored message; turning an org off and on without restarting; orderly
shutdown without losing a pending message; and the attendant's message, delivered through the Application Service
route, entering the outbound queue with the conversation transferred).

## 8. Notice when the human reply fails for good (MX-19)

When the attendant's reply enters the outbound queue and WhatsApp does not accept it for good (the item becomes
`failed`), the attendant needs to know, in the room, that the customer did not receive it. The service
`crates/poph-server/src/matrix_failure_notice.rs` does this on each pass (once every 5 s in the background loop, enabled
by `matrix_inicio`):

1. finds the `failed` items whose message is a human reply (`origin = 'human'`, by `turn_id`) of the orgs with the module
   on, which do not yet have an outcome in `matrix_avisos_falha`;
2. writes in the room `Não consegui entregar sua mensagem ao cliente: <motivo curto>` ("I could not deliver your message
   to the customer: <short reason>"), as `m.notice`, on behalf of the conversation's agent ghost. The reason is one of
   three fixed texts: "o WhatsApp não respondeu depois de várias tentativas" ("WhatsApp did not respond after several
   attempts"), "o WhatsApp recusou o envio" ("WhatsApp refused the send") or "motivo não informado" ("no reason given");
3. records the outcome (`avisado`, `sem_sala` or `recusado`) with the reason code.

What changes in practice:

- **One notice per failure**, even after restarting Harness: the mark is in the database.
- **A transient failure does not notify.** While the queue keeps retrying (item in `sending`), nothing appears in the
  room. If the next attempt succeeds, the attendant gets no notice at all.
- **Synapse down does not lose the notice**: the failure stays for the next pass. A definitive error from Synapse marks
  `recusado` and is not repeated (it appears in the log and in the error metrics by class).
- **Conversation without a room** is marked `sem_sala`, with no attempt on Synapse.
- **Human replies only.** A failure of the agent's output or from another channel does not generate a notice.
- **No customer data.** The notice and the log do not carry customer text, phone number or the raw queue error.

Limit: the notice goes out on behalf of the agent's ghost. If that ghost is no longer in the room, Synapse refuses the
notice for good, the failure stays as `recusado`, and there is no new attempt.

Proofs: `crates/poph-server/tests/matrix_falhas_aviso.rs` (11 tests, including the background loop).

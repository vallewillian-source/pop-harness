# Matrix module: runtime rules

What the Matrix side of `poph-server` does at runtime: mirroring messages into rooms, mirroring the
conversation state, delivering the attendant reply to WhatsApp, warning the room when that reply
fails, expiring published media, wiring the module into `poph serve` and the `poph_matrix_*`
metrics. The protocol details (rooms, ghosts, Application Service namespace, media blocks) live in
`poph-matrix` and in `CONTRATO_CANAL.md`; this page keeps the rules that are not visible from the
code alone.

## 1. Message mirroring (MX-14, `matrix_mirror`)

The loop is **not** in the customer path: it reads `messages` later, in pages, and delivers to the
Synapse through the target port. `poph serve` does not start it by itself; whoever mounts the module
calls `run`.

Each cycle:

1. reads the orgs with the module enabled and each org's message cursor (`matrix_state`);
2. if a cursor is ahead of the highest `rowid` that exists, it steps back to that point;
3. reads `messages` from the lowest cursor, in pages of `PAGE` rows, so memory stays bounded;
4. decides per message: already mirrored (by position or by the cursor's id), before the backfill
   cutoff, from a disabled org, or to mirror. A message to mirror goes through the format decision:
   `Error` goes to `matrix_falhas` without calling the Synapse. A customer message with an
   attachment also becomes a media event, ahead of the text;
5. delivers in parallel **across rooms** (up to the configured concurrency) and in order **inside a
   room**. A transient failure stops that room at the failing point;
6. advances the cursor only up to the first unresolved row (contiguous watermark): nothing is
   skipped and the pending row comes back next cycle.

Failure rules:

- **Transient** (network, 5xx, deadline, 429 without wait): stays pending; the cycle stops at the
  point and the next one waits a delay that doubles per failed cycle, capped at `MAX_DELAY`.
- **Definitive** (Synapse 4xx): counts the attempt per message in memory. Up to
  `DEFINITIVE_ATTEMPTS` it stays pending; at that count it is written to `matrix_falhas` with the
  short error code and the queue moves on. A repeated `txnId` does not duplicate in the Synapse, so
  repeating is safe.

The read is global and the cursor is per org: `messages` is a single table for all orgs, so the loop
reads once from the lowest cursor among enabled orgs and filters by org afterwards. Each org only
processes its own rows after its own cursor.

The cursor stores the message id as well as the `rowid`: a `rowid` can be reused when the last row
is deleted, so a row with the same `rowid` and another id is a new message and is mirrored; with the
same id it was already mirrored and is skipped.

Customer media (MX-43): the attachment is already in the database when the loop reaches the message.
For an inbound customer message with an attachment the loop builds the media steps **before** the
text: reads the blob (bounded by `poph_matrix::asset::ASSET_BYTES_LIMIT`), uploads it as the
customer ghost and publishes `m.image`/`m.file` with the `com.<mark>.attachment` block. The customer
text, when present, is the caption and goes out after the media.

What never enters the room, and why:

- attachment above the limit, empty or unreadable on disk: an `m.notice` in the room says why and
  the message is still resolved, so the cursor does not get stuck on it;
- definitive Synapse failure on the media: the same notice before isolating it in `matrix_falhas`,
  after `DEFINITIVE_ATTEMPTS` attempts;
- transient failure: stays pending, like any other send.

The media `txnId` derives from the attachment id, so reprocessing the same message does not
duplicate the event. The upload itself has no idempotency key in the Synapse: a crash between the
upload and the event can leave a file without an event (known limit).

Every delivered media row is recorded with room, event and `mxc://`; that row is what the expiry
routine reads later to redact the event and delete the file. Without the row the media would stay in
the Synapse forever.

No log line carries customer text: only ids, counters and the short error code.

## 2. Conversation state mirroring (MX-15, `matrix_state`)

When a conversation is transferred to a person, reopened or closed, the room gets a short
`m.notice` and the metadata event the Element reads is rewritten, **in that order**, and only then
is the mirrored state stored. A department-only change moves the room to another space, with no
notice.

Each pass reads the enabled orgs and each org's conversation cursor: the pair (`updated_at`, id),
not only the instant, so two conversations changed in the same millisecond are not lost. It reads
`conversations` from the lowest cursor, in pages, and compares each conversation's status with what
the room already mirrored.

Customer name that arrived (or changed) after the room was created: the room is renamed, the
metadata is rewritten with the conversation's **current** state, the customer ghost gets the new
name, and only then the name counts as mirrored.

Rules worth knowing:

- **The notice does not repeat.** The notice `txnId` derives from (conversation, change instant,
  status): repeating the pass after a failure does not duplicate the notice, and a second transfer
  of the same conversation has another instant, hence another notice.
- **A transient failure does not consume the conversation.** If the notice or the metadata does not
  go out, the org cursor does not advance past that conversation; the next pass retries with a
  doubling delay.
- **A definitive failure does not block the queue.** A notice refused with 4xx (429 aside) is
  recorded with the error code and counted in the metrics; the conversation is skipped and comes
  back on its next change. There is no state-failure table.
- **`closed` does not go back to `active` by delay.** Before writing the metadata the conversation
  is read again: if the status changed during the pass, nothing is written and the conversation
  waits for the next pass with the new status.
- **No room, no notice.** A conversation without a room yet is skipped.
- **A space failure does not block the state.** Moving the room depends on permission in the space;
  when it fails, the notice and the metadata that already went out stand, and the move is tried
  again on the conversation's next change.
- **Renaming is not a notice.** Changing the room name when the customer name shows up produces no
  `m.notice`. The name never goes back to empty (a conversation without a name keeps what the room
  has), there is no backfill sweep (the old room corrects itself on the next change), and a name an
  attendant changed by hand is overwritten by the customer name.
- **The customer name never reaches the log**: the lines carry org, conversation and the error code.

Boundary: `StatePort` is the persistence (conversations, room and cursor) and `StateStorePort` is
the real implementation over the `Store`; `StateTarget` is the Synapse side and `SynapseTarget` is
the real implementation. `poph-matrix` stays unaware of the database and `poph-agent` stays unaware
of this module. Nothing here stores or logs customer text, phone or token.

## 3. Attendant reply to the customer (MX-18, `matrix_response`)

Implements the MX-16 event delivery. For each event the Synapse delivers in a transaction it calls
the MX-17 classifier and then:

- human reply: finds the conversation by the room, resolves the attendant name, converts the body to
  what WhatsApp renders, prefixes `*[Name]*:` and calls the MX-04 atomic operation (message +
  outbox item + `transferred` conversation + audit in a single write), with the event id as the
  idempotency key;
- attendant media: the file goes to the customer (MX-44): downloads the content from the Synapse
  with a byte cap and without following redirects, checks the kind **by content** against the closed
  outbound list (image, PDF, Office, audio, video), stores it as an org blob and calls the MX-44
  atomic operation (message `human` with attachment + `file` item in the queue + `transferred`
  conversation + audit, in a single write). What cannot be sent (large file, refused kind, media the
  Synapse no longer serves) becomes a text notice in the room and the conversation does not change;
- ignore: nothing, only the reason code in the log.

What never happens here:

- **4xx/5xx for content.** Unknown room, closed conversation, refused file and media that no longer
  exists in the room are expected situations: they answer `200` without sending anything to the
  customer. The only `Err` is a transient failure of ours (database unavailable, download that comes
  back on the next delivery), which MX-16 turns into `5xx` so the Synapse resends.
- **Second send on redelivery.** The Synapse may repeat the transaction; idempotency is by
  `event_id` (MX-04 answers `Duplicated` and nothing is queued again).
- **Customer data in the log.** It logs room, event id, reason and result, never the text.

Who signs the download and the file notices (MX-44): always a user of the Application Service
namespace, in practice the ghost of the agent that owns the conversation. The Synapse only lets the
`as_token` impersonate who matches the namespace of the AS registration; the human attendant who
sent the file in the Element is a regular user and gets `403 M_FORBIDDEN`. That applies to the media
**download** and to the **notice** in the room. A conversation without an agent has no ghost to
sign: it is an `error` in the log with the reason `fantasma_do_agente_indisponivel` and the message
is not sent back to the Synapse queue, because there is no third authentication path.

Duplication on file redelivery: the message and the queue item are idempotent by event id. The
**send** to the customer is not: Evolution has no send key, and the decision is to resend accepting
the risk of the customer receiving the file twice when the result of the first send does not come
back, because a file that never arrives with nobody knowing is worse for the service. The outbox
decides to repeat, not this module.

Finding the conversation: the event carries only the `room_id` and the MX-08 repository reads the
room by (`org`, `room_id`), so the port walks the orgs with the module enabled and stops at the
first one that knows the room. The result is cached in memory with a cap (`DEFAULT_ROOM_CAP`), so
the cost is once per room, not per message. The sender name cache has its own caps and TTL; a name
that is empty or only whitespace is ignored, so the `*[Name]*` prefix never becomes empty or breaks
the bold markup. A body that already carries Markdown is not converted twice. Text above the channel
limit is cut by the channel into a single message.

## 4. Failure notice in the room (MX-19, `matrix_failure_notice`)

When the attendant writes in the Element and the reply cannot go out through WhatsApp for good (the
outbox item turns `failed`), the attendant must know, in the room, that the customer did not receive
it. This service does that once per failure:

1. finds the definitive human-reply failures (a `failed` item whose message has `origin = human`,
   joined by `turn_id`) in the orgs with the module enabled that have no outcome yet;
2. writes in the room `Não consegui entregar sua mensagem ao cliente: <short reason>` as `m.notice`
   (the MX-17 classifier ignores notices, so the notice never becomes a reply back);
3. stores the outcome. That row is what prevents a second notice after a process restart: the next
   pass does not even reach the Synapse for that failure.

Rules:

- **A transient failure does not notify.** An item in `sending` is not definitive and does not
  appear; if the next attempt succeeds, nothing is written in the room.
- **The notice is idempotent both ways.** The notice `txnId` derives from the outbox item and the
  outcome row is written **after** the Synapse accepts the notice. If the process dies between the
  two steps, the next pass repeats with the same `txnId` and the Synapse does not duplicate.
- **A transient Synapse error marks nothing.** The next pass tries again. A definitive error marks
  `recusado`: repeating does not help and it stays in the log and the metrics.
- **A conversation without a room** marks `sem_sala`: there is nowhere to notify and it is not
  looked up again on every pass.

Reason codes are short and distinct; the last outbox error is what classifies them. A pass looks at
`DEFAULT_BATCH` failures at most, added across all orgs, every `DEFAULT_PERIOD`.

## 5. Media expiry (MX-46, `matrix_expiry`)

After `matrix.midia_retencao_dias` (default 4, from 1 to 365), media the mirror published in the
room is deleted from the Synapse repository and the event is redacted with the block the Element
reads (`com.<mark>.attachment_expired`). The attendant sees the "no longer available" note in place
of the event, drawn by the frontend itself: no extra `m.notice` and no message per expired file.

One pass:

1. reads the orgs with the module enabled;
2. in each one, reads from the media table what is already due (`sent_at <= now - days`) and still
   has a pending step, oldest first, up to the batch cap;
3. per media item: **redacts** the event in the room and **deletes** the file through the admin API,
   each step with its own stamp in the database;
4. what failed gets a growing wait (`attempts`, `next_attempt_at`) and comes back later; the next
   pass serves the others instead of repeating the problematic one.

Rules:

- **Without an admin token the routine stays off**, with a clear log warning on startup. It is not
  an error: the module starts, mirrors and replies, and only the expiry does not run. The token
  lives in the sensitive type and never appears in log, error, metric or test.
- **The clock is injectable**: tests simulate the future without sleeping, and before the deadline
  nothing happens.
- **Retention wins over tidiness**: if the redaction fails, the file is deleted anyway and the next
  pass retries only the redaction.
- **No customer data in the log**: only org, conversation, technical ids and the short error code.
- The wait starts at `WAIT_BASE` and doubles up to `MAX_WAIT`; the exponent is capped at
  `MAX_EXPONENT` so an item that has been failing for a long time does not overflow. One pass runs
  every `DEFAULT_PERIOD` (one hour) and the due date is computed in milliseconds per day from the
  retention day.

## 6. Wiring into `poph serve` (MX-20, `matrix_beginning`)

The composition root of the module inside the server. It decides nothing the pieces already decide:
it assembles client, ids, spaces and the three real ports over the database, injects the process
counters and starts the loops — the message mirror (MX-14), the conversation state mirror (MX-15),
the Application Service receiver (MX-16) with the attendant reply delivery (MX-18) injected, and the
media expiry (MX-46), which only starts when the Synapse admin token is in the environment.

Rules:

- **Off is off.** `[matrix]` absent or `habilitado = false` means `start` returns `None`, no task
  enters the runtime, no `poph_matrix_*` metric is declared and the Synapse routes answer `404`,
  because the receiver is not injected.
- **Server startup.** Invalid Matrix configuration leaves the module off. If the module starts
  and Synapse is unreachable, the probe marks the connection as degraded and each worker retries
  at its own pace. The turn and the customer outbox keep running.
- **Enabling or disabling an org needs no restart.** Both loops re-read the org table each
  cycle/pass, so disabling an org stops its work by the next cycle.
- **Orderly shutdown.** On process shutdown the loops get the stop request and finish what they are
  doing before leaving; the cursor only advances after success, so nothing pending is lost and the
  next start continues from where it stopped. The server waits up to `SHUTDOWN_TIMEOUT` per
  task. If a task exceeds that wait, the server logs a warning and stops waiting for it.
  A send interrupted before the cursor advances can repeat with the same `txnId`.
- **No customer data in the log.** Only counters, org, conversation and short code.

A connection probe talks to the Synapse from time to time and records what it finds; the absence of
a connection never disables anything, it only shows in the state and the log. The probe period is
`PROBE_PERIOD` while the Synapse answers and the wait between probes grows up to `PROBE_DELAY_CAP` while
it does not.

## 7. Metrics (MX-26, `matrix_metrics`)

The `poph_matrix_*` catalog is declared **only when the module is prepared**: the first registration
puts it in the process registry and later calls return the same handles, so registering twice does
not duplicate anything in the output. With the module disabled nobody calls it and no
`poph_matrix_*` appears in `/metrics`.

The registry still has no labels, so there are no per-org series: the lag is the worst among the
enabled orgs and each error class has its own counter. The handles that do not appear in any
`/metrics` (the throwaway set) exist for the loop and the delivery when nobody injected the process
counters (tests and mounts without the module on).

## 8. Contracts the Matrix integration tests hold (`crates/poph-server/tests/matrix_*.rs`)

These rules were written as comments in the test files and are kept here. Each one is checked by a test
in the file named in brackets.

- **Module off (MX-20, `matrix_beginning`):** no module task runs, no `poph_matrix_*` metric is published, and the
  Application Service routes answer `404`.
- **Synapse down at start:** the server starts and serves; the module is degraded instead of stopping the process. The
  connection probe reports "connected" as soon as Synapse answers.
- **Ordered shutdown:** a message still pending at shutdown is not lost and goes out once on the next start.
- **Room name (MX-32, `matrix_beginning`):** the room created by the module uses the customer's name
  (`conversations.client_name`).
- **Room placed in the department space (MX-51, `matrix_beginning`):** when the state loop sees a conversation before its
  room exists, the room still joins the space. The state read that finds the space the room belongs to is used when the
  conversation record is empty.
- **Backfill cut (P8, `matrix_resilience`, `matrix_beginning`):** turning the module on moves the backfill cut to now.
  Messages written while the org was off are not mirrored; only later ones are. Turning an org off stops its mirroring
  without a restart, because the loop rereads the org list on every cycle.
- **Customer identifier (RP-02, `matrix_beginning`):** the room metadata and the listing by customer use the same rule. A
  conversation with the same phone in another org is not listed.
- **Application Service masquerade (`matrix_resposta`, `matrix_midia_atendente`):** the Application Service can act only as
  users in its namespace. A request for a user outside it gets `403 M_FORBIDDEN`, as the real Synapse does. So the
  attendant's file download and failure notice are signed by the agent's ghost, never by the human attendant.
- **Idempotency by transaction (`matrix_resposta`, `matrix_midia_atendente`):** the same `txnId` twice is one send and the
  module does not call delivery again. A new transaction with the same event (Synapse can rebuild the batch) is also one
  send, because the idempotency key is the `event_id`, in the atomic operation (MX-04, MX-44).
- **Attendant reply (MX-18, `matrix_resposta`):** a reply becomes one send with the attendant's prefix, the conversation
  goes to `transferred`, and the agent stops answering. A message from the customer after that does not reopen it.
- **Conversation closed (`matrix_resposta`):** nothing goes to the customer and a notice is written in the room.
- **Unknown room (`matrix_resposta`):** a room no org knows is answered with `200` and nothing is stored.
- **Database failure on delivery (`matrix_resposta`):** answered with `503`, never `4xx`, so Synapse retries and the retry
  is processed.
- **Conversation without an agent (`matrix_resposta`):** there is no ghost to download the file or sign the notice. The case
  is recorded as an error and a counter, and the message is not put back in the Synapse queue, because a retry would not
  bring the agent back.
- **Media from the attendant (MX-44, `matrix_midia_atendente`):** the file is downloaded from the room, under the agent's
  ghost, and sent to the customer as `image` or `document`. The MIME type is read from the content, not from the file
  name: a ZIP named `.pdf` is refused. A file above 16 MB or a type the Harness does not send becomes a notice in the room
  and is not sent. A download that fails transiently is a `5xx` for Synapse to retry, with no notice and no send.
- **Audio and video from the attendant (MX-48, `matrix_midia_atendente`):** they go to the customer as documents with their
  real MIME type, with the attendant's caption. The native voice route was not proven in a real environment, so it is not
  used.
- **Customer media (MX-43, `matrix_midia`):** an image or PDF in the room is sent as `m.image` or `m.file`, published by the
  customer's ghost, with the blob size and the `mxc://` returned by the upload. The caption goes after the media, in
  message order. Reprocessing the same media does not duplicate the event (the same `txnId` is reused).
- **Media above the limit (`matrix_midia`):** a file above 16 MB becomes a notice in the room. The cursor moves past it and
  the next message goes out in the same cycle. A file whose blob is missing on disk also becomes a notice and does not hold
  the cursor. A media refused for good by Synapse leaves the notice before it is isolated.
- **Media expiry (MX-46, `matrix_midia`):** each delivered media is recorded to expire, with the event and the `mxc` the
  room received. The expiry routine reads that record after the deadline.
- **Client first (A1/A4, `matrix_resilience`):** the customer gets the reply even when Synapse is down or slow (5 s per
  call). The check is by order, not by time: when the second reply reaches the Evolution fake, no Synapse call of the
  module has been answered yet. The numbers are in `MATRIZ_RESILIENCIA.md`.
- **Synapse outage (A4, `matrix_resilience`):** messages pile up in the queue. When Synapse returns, the room receives all
  of them in database order, once. Retried attempts are fine; each `txnId` is accepted once.
- **Poisoned message (`matrix_resilience`, `matrix_falhas_aviso`):** a message that Synapse refuses for good is isolated
  after the definitive-failure limit (`TENTATIVAS_DEFINITIVAS`). The messages behind it go out in the same cycle or the next
  one.
- **Crashes (A5, `matrix_resilience`):** a crash during room creation does not create a second room: the next start finds
  the room by its alias and completes the row. A crash during a mirror send is redone with the same `txnId`, so the room
  does not receive the message twice. A crash before the attendant's response is recorded is written once on the next start.
- **Log values (`matrix_metricas`):** the customer's phone, the message text and the tokens never appear in the logs. The
  Synapse error text can carry a token, and that text is not logged.
- **Log line of an isolated message (`matrix_metricas`):** the line that announces the isolation (not the line of a new
  attempt) carries the org and the conversation.
- **Counters of the attendant reply (`matrix_metricas`):** a new reply is queued once; the same event again (a Synapse
  resend) counts as duplicate and queues nothing; a room no one knows counts as unknown; a media from the attendant writes
  only a notice in the room; a reply in a closed conversation sends nothing and writes a notice.
- **Metric lag (`matrix_metricas`):** the cursor lag rises while Synapse is down and returns to zero once the backlog is
  delivered.

## 9. Organization isolation and secret handling (MX-28)

The organization isolation tests in `crates/poph-server/tests/matrix_isolation.rs` use a running
HTTP server, temporary storage and a local fake Synapse. The Application Service resolves a
conversation from its room and scopes delivery to that organization. An unknown room is
acknowledged without a write, and the unknown-room counter records it.

Matrix administration follows the organization token scope. Department space changes require a
department from the same organization. A customer name received through the WhatsApp webhook
applies to the conversation and mirrored room of the integration's organization.

The tests verify that the Matrix Application Service tokens stay out of HTTP responses, logs,
errors, files and the delivery queue. Requests to Synapse carry authorization in a header.

## 10. CLI operation (`poph matrix`) (MX-22, MX-23, MX-12)

`poph matrix estado`, `ligar`, `desligar` and `espaco` follow the other administrative CLI groups:
they require `--offline` and `--org` and write straight to the database. The CLI never talks to
the server and never uses a token. The parity with the administrative API is in the rules, not in
the transport:

- The backfill cut is written by the same `ligar` repository call; `desligar` keeps the cut.
- The state version is `updated_at` of `matrix_orgs`, the same value the API exposes as `ETag`.
  `ligar` and `desligar` read the version, check it before writing and, when another process wrote
  in between, exit with code `3` (the API's `412`) without writing anything.
- Creating or linking a department space uses the same space service the API uses (MX-12) and
  therefore talks to the configured Synapse: it needs `POPH_MATRIX_AS_TOKEN` and
  `POPH_MATRIX_HS_TOKEN` in the environment.
- Unlinking a space deletes only the Harness row; nothing is removed from Element.
- The CLI cannot tell whether Synapse is answering right now — that probe runs in the server, so
  `estado` reports `conectado` as `null`. No token or sensitive value appears in the output.
- Exit codes: `0` ok; `1` error (org, department, configuration, Synapse, database); `3` the version
  changed between read and write. `--json` has a stable format with `versao_relatorio` = 1.

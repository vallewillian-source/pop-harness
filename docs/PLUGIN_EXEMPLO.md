# Example plugin (echo) and the link to the queues (HR-27)

> Contract: `docs/PLUGIN_PROTOCOLO.md` (HR-25). Code: `crates/poph-plugin-eco/`
> (the plugin) and `crates/poph-agent/src/fila_integracao.rs` (the link with `inbox`/`outbox`).

This document answers three questions:

1. **how a real plugin talks to Harness** — with an example that runs;
2. **how the output queue and the input queue connect to the channel** — what was missing after HR-25;
3. **what does not exist yet** — so the example is not confused with the WhatsApp work of HR-26.

## 1. The example plugin

`crates/poph-plugin-eco/` builds the binary **`plugin-eco`**, a small program that implements the JSON-lines protocol
over stdio:

| Receives | Does | Replies |
|---|---|---|
| `handshake` | checks the protocol version | `handshake_ok` with the manifest (`kind: "eco"`, `text: true`, `file_out: true`, `typing: true`), with the configuration schema (`modo`, `remetente` required) and a secret (`token`, required) only so the example shows the generated form (HR-44c) |
| `comando` / `enviar_texto` | reverses the received text | `resultado` ok **and** an inbound `evento` with the reversed text |
| `comando` / `enviar_arquivo` | nothing (a real plugin would upload the file) | `resultado` ok |
| `comando` / `definir_presenca` | notes it on stderr | `resultado` ok |
| `saude` | counts the messages handled | `saude` ok with the detail |
| `encerrar` | exits with code 0 | — |

Two decisions that apply to any new plugin:

- **Stable `id_externo`.** The event the plugin returns uses `eco-<turno:seq>` — the command's idempotency key. If
  Harness resends the same message after a crash, the event arrives with the same id and the `inbox` discards the
  repetition (delivery is "at least once").
- **A line outside the protocol is noise.** Invalid JSON or an unknown message goes to stderr and the loop continues;
  the plugin never dies because of a strange line.

## 2. How to run it

```bash
apps/pop-harness/scripts/plugin-echo.sh
```

The script builds the plugin and the demonstration, runs everything in a temporary database and deletes the folder at
the end. Run by hand, the demonstration takes an optional folder (`plugin-eco-demo [folder]`, default: a temporary
folder, deleted at the end) and processes **no turn**: only the round trip through the queues. It looks for the
`plugin-eco` binary in the same build tree, up to three parent folders of the example, so build the binaries first with
`cargo build -p poph-plugin-eco --bins`.

Expected output (summarized; the lines are printed by the code and stay in Portuguese):

```text
2. supervisor pronto: Eco (exemplo) v0.1.0 (kind=eco, protocolo=1)
3. item na fila de saída: `olá, mundo` (estado Pendente)
4. envio pelo plugin: enviados=1 falharam=0 → estado Enviado
5. inbox → mensagem (in/user): "odnum ,álo"
6. reentrega do mesmo comando: gravados=1 duplicados=1 (nada duplicado na conversa)
9. fila de saída no fim: pendentes=0 enviados=1
==> processos do plugin ainda vivos: 0
```

What the demonstration does, step by step:

1. opens a database (`harness.db`) in a temporary folder, with an org, an agent and the contact's conversation;
2. starts `plugin-eco` **as a child process** through the `Supervisor` and performs the handshake;
3. enqueues a `text` item in the `outbox` (`{"texto": "olá, mundo"}`), as the turn would;
4. delivers the item through `EnvioPeloCanal` — the same transport WhatsApp will use;
5. waits for the event to come back and shows the message recorded in the conversation (`"odnum ,álo"`);
6. resends the same command and shows that the `inbox` counts it as `duplicado` instead of recording it again.

## 3. The link with the queues (`poph-agent/src/fila_integracao.rs`)

It lives in `poph-agent`, not in `poph-plugins`, for two reasons: whoever knows the `outbox`/`inbox` and the
`TransporteSaida` is the engine (`poph-agent`), and `poph-plugins` remains pure code (only the protocol and the
supervisor), which keeps it in the workstation's ARM check without pulling in SQLite.

### Output: `outbox` → channel

```rust
let transporte = EnvioPeloCanal::novo(store, canal);   // canal: Arc<dyn Integration>
fluxo.enviar_pendentes(org, &transporte).await?;        // pending/sending → sent | failed
```

`EnvioPeloCanal` implements `TransporteSaida` (HR-12) and translates the `outbox` item into the protocol command:

| `outbox` item | Becomes | Payload fields used |
|---|---|---|
| `text` | `enviar_texto` | `texto` |
| `file` | `enviar_arquivo` | `blob_ref`, `nome`, `mime`, `tamanho_bytes`, `sha256`, `legenda` |
| `presence` | `definir_presenca` | `estado` (`digitando`/`pausado`) |

The destination (`peer`) comes from the conversation's `external_peer` of the item — the channel does not appear in the
payload. A **transient** channel error (`indisponível`, `tempo esgotado`, a refusal marked as transient) puts the item
back in the queue as `sending` to be retried; a definitive error (`conversa sem peer`, invalid payload) marks it
`failed` with a readable reason. The item only becomes `sent` after the channel confirms — this is what prevents a
message from being lost on a crash in the middle of sending.

### Input: channel → `inbox`

```rust
let resolvedor = Arc::new(ConversaDoCanal::novo(store, agente, "whatsapp", Some(integracao)));
let entrada = InboxDoCanal::iniciar(fluxo, org, resolvedor);  // implements Receptor
```

`InboxDoCanal` implements `Receptor` (HR-25): `Receptor::receber` only **enqueues** (the channel's read loop cannot
wait for the database), and a separate task records it through the HR-12 path — `Fluxo::receber_mensagem`, which records
the message and the `inbox` item in the **same transaction**, with idempotency by `(integração, id externo)`. A
redelivery becomes `duplicado`, not another message.

`ConversaDoCanal` decides the conversation: it looks for an org conversation with the same channel and the same
`external_peer` (preferring the open one); if there is none, it opens a new one with the configured agent. Without this,
an event from a new contact would have nowhere to go.

Counters (`InboxDoCanal::contadores`) — per integration, never summed across orgs:

| Counter | Meaning |
|---|---|
| `gravados` | new event became a message |
| `duplicados` | the `inbox` refused it for a repeated external id |
| `recusados` | the internal queue (1024) was full — backpressure |
| `falhas` | could not resolve the conversation or record |
| `estados` | channel state notices (connected, QR, error) |
| `com_anexo` | event carried an attachment (see limit 3 below) |

## 4. How to write a new plugin based on this one

1. Copy the structure of `crates/poph-plugin-eco/src/main.rs`: line loop, `HostMessage` on input, `PluginMessage` on output.
2. Answer the `handshake` with a truthful manifest (capabilities and configuration schema) and refuse a different protocol version.
3. Confirm **every** command with `resultado` and the same `id` — this is what turns the `outbox` item into `sent`.
4. Deliver what arrives from the customer as `evento`, with a **stable** `id_externo` (the channel's id). Redelivery is
   normal: the `inbox` is what deduplicates.
5. Never write a secret to `stderr`, to a file or into an event: the secret arrives only in the handshake.
6. If the plugin crashes, it will be restarted with backoff; do not keep state that does not survive that.
7. Register it in Harness with `OpcoesPlugin::novo(caminho, integracao_id, org)` — program, arguments and environment
   declared; secret only through `com_segredo`.

A plugin in another language follows the same JSON as `docs/PLUGIN_PROTOCOLO.md`; the Rust example uses the types of
`poph-types` on purpose, so that a contract change breaks the build instead of breaking in production.

## 5. What the test suite proves

`crates/poph-plugin-eco/tests/conversation.rs` (5 tests) and `tests/crash.rs` (1 test), with `plugin-eco` as a real child
process. The tests swap the supervisor's timeouts for shorter ones (same behaviour as the default, shorter waiting), and
the crash test copies the binary under a unique name so that `kill -9` hits exactly that test's process and no other
plugin running on the machine:

- **full conversation:** `outbox` item → supervisor → plugin → event → `inbox` → message in the conversation, with the
  reversed text checked;
- **idempotency:** resending the same command does not create a second message (`duplicados = 1`);
- **file:** the `file` item from HR-24 becomes `enviar_arquivo` and the item becomes `sent`;
- **conversation without a contact:** definitive failure with a readable reason; the channel is not even called;
- **two contacts:** each `peer` opens its own conversation, without mixing;
- **invalid payload:** definitive failure, without calling the plugin;
- **`kill -9` on the plugin:** the supervisor does not fall, restarts the plugin (`reinicios ≥ 1`), the item is sent
  again, no message is lost and none is duplicated.

## 6. What does not exist yet (known limits)

1. **It is not WhatsApp.** HR-26 brings the native module and the webhook reply. Here the path is the queues, proven
   with the plugin — the same `EnvioPeloCanal` and the same `InboxDoCanal` serve.
2. **The server does not start integrations yet.** `poph serve` does not load plugins; today whoever turns it on is the
   process that builds the `Fluxo` (the demonstration and the tests). Turning it on in the server is the integration of
   HR-26/HR-38.
3. **Received attachment does not become a row in `attachments`.** Until HR-26 records the blob and the extraction, the
   attachment stays **described** in the message's attachment field, as the channel delivered it — nothing is lost, but
   the turn still does not read the customer's file through this path.
4. **Searching a conversation by contact is not indexed.** The resolver remembers each contact's conversation in cache
   (checked in the database on each use, only an open conversation counts), so the normal case costs one read by id. When
   the contact is not in the cache — first contact, transferred conversation or a full cache (cap of 4,096 contacts) —
   the search scans up to 50 pages (10 thousand conversations), because the conversations repository does not yet have a
   search by `peer`. It shows up in an org with a very large history; the right fix is for HR-26 to expose this search in
   the repository.
5. **Input queue limited to 1024 events**: with the database locked and the channel delivering non-stop, the excess is
   refused and counted (`recusados`) — the event is not recorded.
6. **The example plugin is not in the distribution package**: `poph` does not depend on `poph-plugin-eco`, and
   `scripts/build-dist.sh` only packages `poph` and `poph-extrator`.

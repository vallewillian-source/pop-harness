# EvolutionAPI client (`poph-evolution`)

First piece of native WhatsApp (HR-26a). It talks to EvolutionAPI through the same routes and bodies
that POP Atende uses (`core/integrations/evolution_api.py`, `core/integrations/evolution.py`).

## Operations covered

| Operation | Route | Retries? |
|---|---|---|
| Create instance | `POST /instance/create` | no |
| Pair (QR and code) | `GET /instance/connect/{instância}` (with `?number=` for a code) | yes, once |
| Connection state | `GET /instance/connectionState/{instância}` | yes, once |
| List instances | `GET /instance/fetchInstances` | yes, once |
| Disconnect | `DELETE /instance/logout/{instância}` | no |
| Delete | `DELETE /instance/delete/{instância}` | no |
| Set webhook | `POST /webhook/set/{instância}` | yes, once (idempotent configuration) |
| Send text | `POST /message/sendText/{instância}` | **no** |
| Send file | `POST /message/sendMedia/{instância}` | **no** |
| "Typing" | `POST /chat/sendPresence/{instância}` | no |
| Download received media | `POST /chat/getBase64FromMediaMessage/{instância}` | **no** |

Authentication: the key goes only in the `apikey` header. It never appears in an error, `Debug` output or log.

Implemented in the crate: `EvolutionClient` (the operations above) and `WhatsAppEvolution` (the integration in the
`Integration` contract, with sending text, files, presence and health).

**Presence.** "Typing" and "paused" are the channel's own words: `true` goes out as `composing`, `false` as `paused`
(`POST /chat/sendPresence/{instância}`). Evolution returns no ID for presence, so the call returns `None`. A presence
that Evolution refuses **does not fail the turn**: the error is logged at `debug` and the send continues, because
"typing" is cosmetic and the customer must still get the reply.

## ID of the sent message (MX-40)

The response to a send (`sendText` and `sendMedia`) carries the ID that WhatsApp gave the message. The client reads
the formats Evolution has already used, in this order: `key.id`, `message.key.id`, `id` and `data.key.id` — the first
one that is a non-empty string. There is no synthetic ID: if none comes back, the result is "sem id" (no id).

- **`EvolutionClient::send_text` and `send_attachment`** return `Option<String>` with that ID.
- **`WhatsAppEvolution::send_with_receipt(OutboundCommand)`** is the receipt path: it delivers text or a file and returns
  the ID; presence returns `None` (Evolution does not return an ID for "typing").
- **The generic channel contract does not change:** `Integration::send_text`/`send_file` still return
  `Result<(), ErroIntegracao>` and discard the ID, because the same contract serves a plugin in an external process,
  which may not return any.
- **What Harness stores:** the WhatsApp output (`crates/poph-server/src/saida_whatsapp.rs`) records the ID in
  `outbox.external_message_id` when it confirms the send (migration `0011`). A send that fails, or a channel that
  returns no ID, leaves the column null — and that is not a send error.

Format observed in the test fake (`{"key": {"id": "…"}}`) and in POP Atende's records; the real format of a
production instance is only confirmed with a real number.

## Configuration

- Evolution address (`url_base`, `http://` on the local network or `https://`), instance name and the time limit per
  attempt (default 20 s).
- Key: environment variable `EVOLUTION_API_KEY`. It does not go in the configuration file.
- Webhook secret: required. It goes in the `X-Evolution-Webhook-Secret` header, which Evolution repeats on every
  delivery.
- Optional counters: `ConfigEvolution::with_metrics` turns on `poph_chamadas_evolution_total` (one per HTTP request)
  and `poph_erros_evolution_total` (one per request that failed). Refusals made before anything leaves (file above
  the cap, configuration) do not count as calls.

## Limits

- **File** (send and download): 16 MB (`ATTACHMENT_CAP`), checked before uploading or decoding. Media refused on send does
  not go out; media above the cap on download is refused without decoding.
- **Evolution response**: 1 MB for common responses; for media download, what is needed for 16 MB in base64. Beyond
  that, reading stops and the error is `ResponseTooLarge`.
- **Expired media**: Evolution keeps received media for a few days (`ATTACHMENT_DAYS = 4`), then answers the download
  with an error. A download is only treated as expired with a 400/404 response that contains "message not found",
  "media not found", "media not available" or "expired/expirad" (these are the provider's own texts, matched as they
  are). Any other 400 is a common error.
- **Provider error text**: cleaned before it becomes a message. Numbers, JIDs (anything with `@`), any run of 7 or more
  digits and the key itself are removed; the text is cut at 120 characters.
  Limitation: free text without a number (for example, a contact name that the provider echoes) is not detected.
- **Retries**: only reads and configuration are retried, and only once, on network drop, timeout, 429 or 5xx. Sending
  and downloading are never retried inside the client. Whoever decides to try again later is the output queue, which
  uses the error's transient marking.

## Incoming messages (webhook, HR-26b)

Public server route, outside `/admin/v1`:

```
POST /webhook/evolution/{org_id}/{instancia}
Header: X-Evolution-Webhook-Secret: <instance secret>
```

**Configuration.** Each instance links an org to a WhatsApp integration in Harness, in the configuration file (with no
secret inside):

```toml
[evolution]
url_base = "http://127.0.0.1:18081"

[[evolution.instancias]]
org_id = "<org id>"
instancia = "loja"                       # instance name in Evolution
integracao_id = "<WhatsApp integration id>"
segredo_env = "POPH_EVO_SEGREDO_LOJA"    # name of the variable that holds the secret
```

The secret comes only from the environment (`POPH_EVO_SEGREDO_LOJA=...`), read at startup. The Evolution key stays in
`EVOLUTION_API_KEY`, used to download media. Without the secret variable, the instance answers 401 until it exists
(the server warns in the log, with the variable's name and not its value).

**Responses.** Designed for Evolution, which resends 5xx and timeouts (up to 10 times in ~21 min) and does not resend
4xx:

| Situation | Response | Effect |
|---|---|---|
| New message recorded | 200, `situacao: nova` | inbox, conversation, message and attachments recorded in one transaction |
| Same message again (redelivery) | 200, `situacao: duplicada` | returns the same IDs; does not record or download media |
| Connection state (`open`, `close`, QR) | 200, `efeito: estado` | updates the integration's `connection_state` |
| `connecting` state or transient error | 200, `efeito: nenhum` | changes nothing |
| Event with no effect (e.g. `send.message`) | 200, `efeito: nenhum` | does not record |
| State for an integration that does not exist | 200, `efeito: integracao_inexistente` | does not create a row |
| Missing or wrong secret | 401, `webhook_nao_autorizado` | does not read the body nor record |
| Instance or org not configured | 404 | — |
| Message for an integration that does not exist or is archived | 404 | does not record (without an owner agent there is no conversation) |
| Body that is not a JSON object | 400 | — |
| Our failure to record (database or blob) | 503, `indisponivel` | nothing is left half-done; Evolution resends |

**Messages.** Text and captions become the message text. Images are downloaded by Evolution, stored in the blob store
(by hash, in the org's folder) and become attachments. PDF and Office go through the same intake of library files
(real type from the content, text reading with the extractor). Media that fails (expired, over 16 MB, unsupported
type, unreadable file) **does not bring down the message**: it comes in with the text it had and a warning in the log.
Audio, video, stickers, contacts and location do not come in (decision of the maintainer: file = image + PDF/Office).

**`fromMe`.** A message with `fromMe = true` is **discarded by default**, because the webhook returns as `fromMe` both
our own send and what the customer typed on the phone; without the filter the agent would talk to itself. Accepting it
is `Policy::accept_from_me` (`ChannelConfig::accept_from_me`), off by default. Whoever turns it on decides what to do
with the event: today the server turns it on only for the org with the Matrix module active (MX-41), where the echo of
our own send is discarded and the message typed on the device enters as `direction out` and `origin device`, with no
turn.

**Unsupported content (MX-45).** Audio, video, sticker, contact, location and poll enter as **history text**, with the
fixed notice from `poph_types::UnsupportedContent` (for audio, `[O cliente enviou um áudio, que o sistema não consegue
abrir]`), so the agent knows what happened and can answer; the file itself is not downloaded. The Matrix room shows a
notice in place of that text (`poph_types::MidiaForaDoEscopo`). The `fromMe` echo of those types stays discarded: it was
not the customer who sent it.

**Group versus person.** In a one-to-one conversation the `peer` is the number, digits only (what Evolution accepts on
send). In a group the `peer` is the full JID (`…@g.us`) and whoever wrote goes in `remetente`, so the reply reaches the
group and the turn knows the person.

**Conversation.** A message goes to the open or transferred conversation of the same contact, in that integration. If
there is none, a new one opens with the integration's agent and its default department. A transferred conversation
keeps recording, with no reply from the agent (D12).

**Idempotency.** The key is the integration plus the WhatsApp message id. A redelivery returns the same item and
repeats nothing.

**Security.** The event body carries the instance's `apikey`. It is not stored raw, not logged and not returned in the
response. Errors and logs contain only internal ids and categories: no phone number, JID, key or secret. The secret
comparison is in constant time.

**Limits of this stage.**

- The conversation is **not answered** yet: the turn (model call and sending the reply) is HR-26c.
- An archived integration's state is not checked in the state webhook; the rule "inactive agent stops answering" is
  left for the turn.
- A message for an **inactive** integration is recorded normally (nothing the customer sent is lost); the turn decides
  not to answer.
- The recorded time is the time of receipt, not WhatsApp's `messageTimestamp` (which stays in `evento_json` of the inbox).
- Text extracted from PDF/Office is stored already wrapped as untrusted content. The turn must use it as it is, without
  wrapping it again.
- Blob written before the transaction: if the database fails afterwards, a file is left without an attachment row (same
  content, same hash; it is reused on a later delivery).
- PDF and Office through the webhook **were not tested end to end**: text reading uses the `poph-extrator` program,
  which must sit next to the executable. The image was tested.
- Not tested with a real number: the real-conversation scenario depends on the WhatsApp test account. Covered by tests
  with a fake Evolution and with the body formats captured from Evolution 2.3.7 in the isolated environment.

## Output: from the output queue to WhatsApp (HR-26c)

The reply is written by the turn (which does not yet run in `poph serve`). It records the agent's message as `out` in
the conversation and enqueues a `text` item (or `file`, for a library file) in the output queue. The consumer lives in
`crates/poph-server/src/saida_whatsapp.rs`, runs as a 1 s loop inside `poph serve` (only when an Evolution instance is
configured) and follows these rules:

- **Only picks up what belongs to the configured WhatsApp.** An item whose conversation is from the console, another
  channel, or an integration that is not in `[[evolution.instancias]]` stays as it is. It is not marked as a failure.
- **No Evolution key** (`EVOLUTION_API_KEY` missing from the environment), the item stays `pending` and does not use up
  an attempt. The key is read at server startup: after setting the key, `poph serve` must be restarted for the items to
  go out.
- **Text over 4000 characters** goes out cut, ending in `…`, in a single message. It is not split (POP Atende rule,
  `WHATSAPP_TEXT_LIMIT = 4000`).
- **File** goes out as media, with the content read from the library's blob. A file over 16 MB is refused **before** it
  is read and becomes `failed` with the reason; the text of the same turn continues.
- **No retry within an attempt.** Sending text and media makes one request per attempt.
- **Transient failure** (network, 429, 5xx) goes back to the queue with growing waits (500 ms, doubling, cap of 60 s).
  After 8 attempts it becomes `failed` with "tentativas de envio esgotadas" ("send attempts exhausted").
- **Definitive failure** (4xx, expired media, file over the cap) goes to `failed` immediately and is not retried. The
  recorded reason already comes without phone number, JID or key.
- **Repetition between attempts is accepted.** If Evolution receives the message and the response is lost, the next
  attempt may duplicate the message on the customer's WhatsApp. This is the risk accepted in the queue (D11); Evolution
  has no idempotency key to prevent it.
- **Server restart**: items in `sending` are tried again on startup (they are pending).

Limits of this delivery:

- The ID of the message that Evolution returns when sending is not recorded. The channel trait returns only success or
  error. So the conversation's `out` line does not get the external ID; it is recorded at the time of the turn, without
  an external ID.
- The consumer does not check whether the integration is archived in the database; only the server configuration.
- `poph chat` confirms only the items of console conversations (fixed in HR-26f). WhatsApp ones are left for the output
  loop.
- The order between messages of the same conversation can flip when one of them is waiting for a new attempt.

## Turn on the server (HR-26h)

The message that comes in through the webhook becomes a turn, and the reply goes out through the output queue above.
The loop that reads the input queue, and what happens when the turn does not reply (transferred, integration inactive,
no model key), are in [`TURNO.md`](TURNO.md), section 11.

A defect in the input (HR-26b) was fixed in this delivery: the input queue stored the channel event, without the
conversation and the message the turn reads. Now the item stores both ids, and the channel event is kept alongside, in
`canal`.

Interval setting: `[turno] intervalo_despacho_ms` (default 1000).

When the model fails in the middle of a turn, the customer's message is tried again up to 3 times
(`[turno] tentativas_falha_modelo`), with a wait of 30 s and then 120 s (`atraso_falha_modelo_ms`). If all of them fail,
the message is completed with no reply and nothing goes out on WhatsApp. Details in [`TURNO.md`](TURNO.md), section 11.

## What it does not do yet

- **Test with a real number**: not done. The WhatsApp test account is still pending; acceptance uses a local fake server
  and an isolated Evolution.
- **Real number and real model**: both ends (input and output) were proven with a fake Evolution and a fake model. The
  proof with a real WhatsApp number is still pending.
- **Create instance and pair through the Harness API**: the operations exist in the client, but there is no
  administrative route that calls them.

## How to run the tests

```
cd apps/pop-harness
cargo test -p poph-evolution --locked
cargo test -p poph-server --test saida_whatsapp --locked
cargo test -p poph-server --test webhook_evolution --locked
cargo test -p poph-server --test turno_whatsapp --locked
```

The tests use a fake server on `127.0.0.1` with a script of responses. None of them talks to the real Evolution or to a
WhatsApp number.

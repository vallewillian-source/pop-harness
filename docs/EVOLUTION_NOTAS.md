# WhatsApp through EvolutionAPI in POP Harness (HR-26)

This document answers the three checks HR-26 asks for — **(a)** webhook redelivery on 5xx,
**(b)** webhook authentication and **(c)** the long-message rule — and describes what the native module
delivers, with the evidence in the POP Atende code.

- **Code:** `crates/poph-evolution` (`client.rs`, `webhook.rs`, `text.rs`, `integration.rs`).
- **Contract:** the same as HR-25 (`poph_types::Integration`), equal to the external-process plugin.
- **Tests:** 50 (9 unit, 17 for input, 23 for the client/channel against a fake Evolution, and 1 doc test).

---

## 1. The three checks

### (a) Does Evolution redeliver the webhook when we reply 5xx?

**Not measured with a real instance** — it depends on the test account, which belongs to the maintainer. What the
repository proves:

1. **POP Atende assumes yes.** It keeps an event-deduplication table with a 24 h validity
   (`core/server/routes/evolution_bridge.py:184`, `_WEBHOOK_DEDUPE_TTL_HOURS = 24`), key
   `wev_<instância>_<id da mensagem>` (`wev_<instance>_<message id>`, a key format emitted by the code) and
   `INSERT … ON CONFLICT DO NOTHING`. Deduplicating by id only makes sense if the same delivery can arrive twice.
2. **It replies 5xx on several paths** (`evolution_bridge.py:999`, `1002`, `1015`, `1021` — failure/timeout when
   calling the agent).
3. **And yet the redelivery would be discarded**: the event claim happens **before** any processing
   (`evolution_bridge.py:4701`, the comment "claim the webhook event BEFORE any processing" is in the code in
   English) and there is no release of the claim when processing fails. In other words: if Evolution redelivers
   after a 5xx, POP Atende answers `duplicate_message_claimed` (a code emitted by the code) and the customer's message
   **is lost**.

**What the Harness does (a decision, not inheritance).** The HR-12 `inbox` saves the message and only then the webhook
receives `200`; the item only gets `processed_at` when the turn completes
(`crates/poph-agent/src/entrada_saida.rs`, module documentation). The unique key
`(integration_id, external_message_id)` makes a second delivery harmless **without** preventing a delivery that
failed from being processed later. Therefore: Evolution redelivery is **welcome** here, and the effect is exactly one
turn per message.

**How to close this measurement when a test account exists:** start the instance, point the webhook to an endpoint
that answers `500` on purpose and count the deliveries of the same `key.id` in the Evolution log (or in a file of the
fake endpoint). If it does not redeliver, nothing changes in the Harness — the message is still processed, because
resuming the `inbox` dispatches a pending item at startup.

### (b) How to authenticate the webhook

- **Header:** `X-Evolution-Webhook-Secret` (header name kept as the code emits it), checked against the secret set in
  the integration.
- **Refusal:** wrong or missing secret → the delivery is refused; with no secret configured, **no** delivery is
  accepted (`WebhookError::NoSecretConfigured`, the error variant name).
- **Constant-time comparison** (`webhook::check_secret`): the content does not leak through response time.
- **On the Evolution side**, the secret travels in the `webhook/set` call itself (`headers`), as POP Atende already
  does — this is what `EvolutionClient::set_webhook` builds when the channel starts.
- Evidence in POP Atende: `core/server/routes/evolution_bridge.py:4577-4579` (`X-Evolution-Webhook-Secret`,
  `403 Invalid Evolution webhook secret`, a message emitted by the code).

### (c) How POP Atende handles a long message — and the rule reproduced

**It does not split: it cuts.** `apps/backend/core/utils/markdown_to_whatsapp.py`:

- `WHATSAPP_TEXT_LIMIT = 4000`;
- converts Markdown into what WhatsApp renders (bold `*x*`, italic `_x_`, strikethrough `~x~`, title in bold, link
  `texto (url)`, list `• item`, table flattened into `coluna: valor`);
- and, at the end, `text[: LIMITE - 1].rstrip() + "…"`.

The task asked "how it splits long messages and sends them in sequence". **That does not exist in POP Atende today**:
a text above 4000 characters reaches the customer truncated, not in two messages. The Harness reproduces the behavior
(`text::to_whatsapp` + `text::truncate`), because channel parity is worth more than the assumption in the task — and
because splitting into N messages changes what the customer sees.

**Consequence recorded:** a turn that produces a reply above 4000 characters loses its end. Splitting into a sequence
(with the `(turn_id, seq)` of the existing `outbox`) is a change in behavior visible to the customer: it is for the
maintainer to decide, not for this delivery.

---

## 2. What the module delivers

| Part | What it does | Where |
|---|---|---|
| `EvolutionClient` | create instance, pair (QR), state, disconnect, delete, `webhook/set`, `sendText`, `sendMedia`, `getBase64FromMediaMessage`, `sendPresence` | `client.rs` |
| `webhook::interpret` | `messages.upsert` → `InboundEvent` (text, image, PDF/Office) and `connection.update`/`qrcode.updated` → `EventoEstado` | `webhook.rs` |
| `text::to_whatsapp` | Markdown → WhatsApp format + cut at 4000 | `text.rs` |
| `WhatsAppEvolution` | the channel as `Integration`: send text, send a library file, presence, health, state | `integration.rs` |

Rules that came from POP Atende, and why:

- **`fromMe = true` is discarded** — it is the echo of our own message (`evolution.py:16`).
- **Idempotency by the WhatsApp `key.id`** — a redelivery does not generate a second turn.
- **Only images and documents (PDF/Office) come in as attachments.** Audio, video, stickers, contacts and location are
  ignored **with a reason**; and a document whose MIME type is not on the list is refused. This is the maintainer's
  decision (file = image + PDF/Office as text, no audio).
- **Group vs. person:** an individual conversation uses the number (digits only, which is what `sendText` accepts);
  a group uses the full jid (`…@g.us`) and whoever wrote the message goes in `remetente`.
- **Media:** a **16 MB** limit, checked **before** decoding the base64 (a huge file is not loaded into memory just to be
  refused later); Evolution keeps media for **4 days**, and past that it answers "Message not found" →
  `EvolutionError::AttachmentExpired`, which is a **definitive** error (it is not retried).
- **Retry:** one, only on network drop/timeout/429/5xx; 400/401/404 are not repeated.
- **The key (`apikey`) only in the header** and never in an error, `Debug` or a log — there is a test that fails if it
  appears in the error message.
- **Failed media does not bring down the text:** if the download times out or the media has expired, the event reaches
  the turn **without the attachment**, with the customer's text preserved.

## 3. How to run the tests

```bash
cd apps/pop-harness
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p poph-evolution            # 50 tests, none uses an external network
```

The tests' fake server is a `TcpListener` on an ephemeral port with a combined script
(`crates/poph-evolution/tests/evolution.rs`): it records what it received (method, path, headers, body) and answers
what the test sends. There is no real key and no real Evolution.

## 4. What is missing for production

1. **Real test account** (the maintainer's item): create an instance, read the QR, send and receive text, image and PDF
   on a real instance. It is the only step of the HR-26 acceptance that does not close here.
2. **Manifest registration on the server:** today the API refuses to configure an integration (`422
   manifest_indisponivel`, an error code emitted by the code) until there is a trusted source for the manifest. The
   WhatsApp manifest is already written in code (`integration.rs`), with `apikey` and `webhook_secret` marked as
   `format: password` + `x-secret: true`; whoever hosts this registration turns on the integration config.
3. **Webhook route on the server** (`poph serve`): the channel already knows how to authenticate and read the body, but
   no HTTP route calls it yet — it needs the integration registration and the turn engine on the server, which does not
   run turns today.
4. **Media persistence:** the channel delivers the content and the flow decides; saving the blob is `poph-files`'
   job (HR-23), which already exists — the connection point is the webhook route, together with item 3.

## 5. Field notes

- **`sendMedia` accepts base64** (this is what POP Atende uses when there is no temporary public URL). For a large file
  this is expensive: 16 MB becomes ~21 MB of JSON. The path with a public URL (temporary link) was not implemented,
  because the Harness does not yet serve files over HTTP.
- **`getBase64FromMediaMessage` requires the `data` object from the webhook**, not just the id: with the id alone,
  Evolution looks in its own database and answers "Message not found" for media it did not keep
  (`evolution.py:272`).
- **`webhook/set` is per instance, with `byEvents: false`** — the same as POP Atende (`evolution_api.py:214`), and with
  the list `MESSAGES_UPSERT`, `CONNECTION_UPDATE`, `QRCODE_UPDATED` (event names, kept as they are).
- **`WEBHOOK_GLOBAL_ENABLED` stays `false`** in both POP Atende compose files
  (`infra/docker-compose.dev.yml:107`, `infra/docker-compose.oci.yml:197`): the configuration is always per instance,
  never global.

## 6. Dependencies and why

- **`reqwest` with the `rustls` feature** (HR-10b): the same TLS stack as the LLM client and the Jev client. Evolution
  speaks `http` on the local network and `https` when exposed; `http://` keeps working. Because this feature compiles C
  (`aws-lc-sys`), this crate is **out of the station's `cargo check` for aarch64** — the ARM verification is the build on
  the ARM64 test machine (HR-56), not a host-side check.
- **`base64` 0.22**: already in the tree (rustls depends on it), so depending on it adds nothing new to the lock. It
  encodes the media sent to `sendMedia` and decodes what comes back from `getBase64FromMediaMessage`.
- **`regex` 1.x**: already in the tree as a transitive dependency. The channel's Markdown conversion uses the same rules
  as POP Atende, and writing those rules by hand would be more fragile than porting the regular expressions.


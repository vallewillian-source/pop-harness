# Channel contract (integration) and turn events

> Describes what exists in the code today. The source of the types is
> `crates/poph-types/src/integracao.rs`; the external-process plugin protocol is in
> [`PLUGIN_PROTOCOLO.md`](PLUGIN_PROTOCOLO.md); the full example is in [`PLUGIN_EXEMPLO.md`](PLUGIN_EXEMPLO.md).
> If the code changes, this document changes in the same commit. Section 7 lists what **does not exist** and
> would be needed for a channel that observes the conversation (the Matrix case); it is a survey, not a decision.

## 1. Roles

A **channel** is the customer's end: messages arrive from there and the reply goes back there. There are two
kinds today, with the same contract:

- **Native:** WhatsApp through EvolutionAPI (`crates/poph-evolution`), a Rust module that implements the
  `Integration` trait.
- **External process:** any program that speaks the JSON-lines protocol over stdio, supervised by
  `poph-plugins`. The `poph-plugin-eco` plugin is the model.

Each integration belongs to **one agent** (`integrations.agent_id`) and to an org. The protocol version is
`PROTOCOLO_VERSAO = 1`; a difference in the handshake is refused.

## 2. Manifest

The channel describes itself in the handshake (`Manifesto`):

| Field | What it is for |
|---|---|
| `nome`, `kind`, `versao`, `protocolo` | identification; `kind` is the same as in the `integrations` table |
| `capacidades` | `text`, `image_in`, `file_in`, `file_out`, `typing`, `read_receipt` (absent = false) |
| `limites` | `max_anexo_bytes` (default 16 MB), `max_texto_bytes` (0 = no declared limit), `tipos_aceitos` |
| `configuracao` | JSON Schema of the configuration form (the Console draws it from this) |
| `segredos` | JSON Schema of the secrets the operator fills in |

Secrets travel only in the handshake and never in an event, command, error or log. The server does not keep the
manifest: it asks the plugin declared in `[plugins].integracoes` and keeps the result in memory
(`PLUGIN_PROTOCOLO.md` §8.1).

## 3. Input: `InboundEvent`

What the channel delivers to the Harness:

| Field | Meaning |
|---|---|
| `integracao` | integration that received it |
| `id_externo` | id of the message in the channel; **idempotency key** (required) |
| `peer` | the other party: phone, jid, room (required) |
| `remetente` | who wrote it, when different from `peer` (group) |
| `texto` | text received; empty when it is only an attachment |
| `anexos` | list of `AnexoEntrada`: `blob` (already stored: name, mime, sha256, size) or `bytes` (base64 in the event) |
| `ocorrido_em` | ms since the epoch (required, positive) |
| `metadados` | channel data without a field of its own (reply id, mentions) |

`InboundEvent::validar` refuses an event without `id_externo`, without `peer`, without a time, or with an invalid
attachment (sha256 that is not 64 hex characters, zero size, no name or mime).

**Customer name (`push_name`, MX-32).** The WhatsApp plugin puts the contact's name in
`metadados["push_name"]` when the message is from an individual contact received: it is not sent for a message
we sent (`fromMe`, which carries the instance owner's name) nor for a group (which carries the name of whoever
wrote it). Without the field, the conversation keeps the name it already had. The field is not part of
`InboundEvent` as a field of its own, so the channel contract does not change. It is written by
`receber_do_canal`, inside the message transaction (`DADOS.md`, migration `0008`), and not by
`ConversaDoCanal`: the WhatsApp webhook does not go through it.

**What the Harness does with it** (`ENTRADA_E_SAIDA.md`): it writes the message and the `inbox` item in one
transaction, with the unique key `(integration_id, external_message_id)`; a duplicate returns success without a
second turn; the OK to the channel only goes out after the commit. The `inbox` item stores only
`{conversa, mensagem}` (`EventoEntrada`); the content is in `messages` and `attachments`. The item is only
completed when the turn ends, so a failure in the middle makes the dispatch repeat (at least once).

## 4. Output: `ComandoSaida`

What the Harness tells the channel to deliver. Every command carries an idempotency key `turn_id:seq` (except
presence).

| Command | Fields | Note |
|---|---|---|
| `EnviarTexto` (`SendText`) | `idempotencia`, `peer`, `texto` | on WhatsApp, text longer than 4000 characters is cut with `…`, in a single message |
| `EnviarArquivo` (`SendFile`) | `idempotencia`, `peer`, `nome`, `mime`, `tamanho_bytes`, `sha256`, `blob_ref`, `legenda?` | only files from the agent's library (D13); `blob_ref` is `<org>/<sha256>` |
| `DefinirPresenca` (`SetPresence`) | `peer`, `estado` (`digitando` = typing, or `pausado` = paused) | |

States of an `outbox` row: `pending`, `sending` (saved **before** calling the channel), `sent`, `failed`. On
resume, what was left in `sending` is sent again: this is the only situation in which the customer can receive a
duplicate (D11). Default backoff 500 ms × 2 up to 60 s.

## 5. State, health and the trait

The channel reports changes with `EventoEstado`: `conectado` (connected), `desconectado` (disconnected), `qr`
(code and expiry) and `erro` (error: code, message, whether to retry). The health question returns
`Saude { ok, detalhe }`.

The trait that both kinds of channel implement:

```rust
pub trait Integration: Send + Sync + 'static {
    fn manifesto(&self) -> Option<Manifesto>;        // empty before the handshake
    fn estado(&self) -> EstadoIntegracao;            // Iniciando, Pronto, Reiniciando, Caida, Encerrada
    fn enviar_texto(&self, comando: SendText) -> FuturoComando;
    fn enviar_arquivo(&self, comando: SendFile) -> FuturoComando;
    fn definir_presenca(&self, comando: SetPresence) -> FuturoComando;
    fn saude(&self) -> FuturoSaude;
}
```

Whoever receives what the channel produces implements `Receptor::receber(EventoIntegracao)`, with
`EventoIntegracao::Entrada(InboundEvent)` or `EventoIntegracao::Estado(EventoEstado)`. The call is synchronous on
purpose: the receiver enqueues and returns. The ready connection between the contract and the queues is in
`poph-agent/src/fila_integracao.rs` (`InboxDoCanal`, `EnvioPeloCanal`).

Supervision of an external plugin: a size limit per line, a timeout per command, restart with backoff and a
failure limit; a plugin that hangs, crashes or emits garbage does not bring down the server or the other
integrations (`PLUGIN_PROTOCOLO.md` §5).

## 6. Turn events

`EventoTurno` (in `poph-agent/src/turno/mod.rs`) is what the turn reports to whoever follows it: `Inicio`
(start), `Texto` (pieces of the reply), `Base` (retrieved sources), `Ferramenta` (tool name and round),
`Transferencia` (department, risk type, whether guidance was sent) and `Fim` (end: state and reason).

- They are delivered to an `Observador` **inside the process**. On the integration side (WhatsApp, plugin) the
  observer is null: the reply goes out through the `outbox`. In the Console test chat, the events become SSE
  (`inicio`, `texto`, `base`, `ferramenta`, `transferencia`, `fim`; contract in [`API.md`](API.md)).
- What is **recorded** and can be queried later: `messages`, `turns` (Jev decisions, timings, tokens, tools),
  `conversations.status` and `target_department_id`, `audit_log`. The API reads it through
  `GET /admin/v1/orgs/{org}/conversations`, `…/{id}`, `…/{id}/messages`, `…/{id}/turns`, and reopens through
  `POST …/{id}/reopen`.

## 7. What an observing channel (Matrix) would require and does not exist today

> Note: the bridge that lets the Element read and close POP Harness conversations through the Python
> backend (repasse) is not an observing channel and is not described here. See
> [`REPASSE_PYTHON.md`](REPASSE_PYTHON.md).

Survey done by reading the schema and the code on 21/11/2026. The direction approved by the maintainer: a
removable module in a separate crate, with few changes to the core, and a second integration role, that of
observer of the conversation. **None of this is specified or implemented.**

1. **There is no way to send events out of the process.** `EventoTurno` only reaches observers in memory. For a
   module in a separate crate to receive a copy of each message and of the transfer, a publishing point is
   missing (or incremental reading of what is already recorded).
2. **The contract is for a customer channel.** `Integration` serves one peer at a time and belongs to an agent.
   An observer follows all conversations of an org (or of a department) and does not reply to a `peer`. The
   type for the role and the rule for which conversations it receives are missing.
3. **A human agent's message has nowhere to go.** `messages.direction` accepts `in`/`out` and
   `messages.role` accepts `user`, `assistant`, `tool`, `system`: there is no role for a human agent.
4. **A send started by a human has no turn.** `outbox.turn_id` is required and the idempotency is
   `(turn_id, seq)`. A reply typed in Element would arrive without a turn and would need another key.
5. **Pausing the agent when the human replies.** Today only the transfer (G1/G4) pauses, through
   `conversations.status = transferred` (D12). A human who takes over without a transfer has no state.
6. **Several instances.** Routing between instances today covers `POST /webhook/evolution/{org_id}/{instancia}`
   and `/admin/v1/orgs/{org_id}/…`, because the org is in the path. A Synapse webhook would only serve the same
   mechanism if it also carried the org in the path. **Assumption**: not tested.
7. **Synapse being down must not delay the customer.** The copy to Matrix would have to go out through its own
   queue, after the reply to the customer. Today the only durable queue mechanism is the `outbox` of the customer
   channel.
8. **Connection to Cases** and to the per-department rooms does not exist in the Harness (Cases are out of scope).

Before writing the Matrix specification, points 1, 3, 4 and 5 are the ones that touch the core; the others stay
inside the new module.

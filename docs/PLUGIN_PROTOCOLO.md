# Integration plugin protocol (HR-25)

> Contract: PRD §8.3. Code: `crates/poph-types/src/integration.rs` (types) and
> `crates/poph-plugins/` (supervisor). Fixture plugin: `crates/poph-plugins/tests/fixtures/plugin_fixtura.rs`.
> Tests: `crates/poph-plugins/tests/protocolo.rs`.

This is the contract that a POP Harness channel implements. It applies to the two formats in V1:

- **native** — a Rust module that implements the `Integration` trait directly in the process (the WhatsApp
  channel from HR-26 is the case);
- **external process** — a separate program that speaks the protocol in this document with the Harness.

The Console (HR-38) draws the configuration form for any integration from the **JSON Schema of the manifest**,
with no code per channel.

## 1. Why JSON-lines over stdio (and not a Unix socket)

The plugin is a child of the Harness: the supervisor opens it and receives its `stdin`/`stdout`.

- **Lifecycle tied to the process**: the end of the plugin becomes `EOF` immediately, with no connection timeout and
  no orphan socket to clean up after a crash;
- **Real isolation per process**: a plugin that hangs, crashes or prints garbage shares no memory, descriptor or event
  loop with the server or the other integrations;
- **One line = one message**: you can read the traffic with `cat` and test the plugin with `echo`;
- **No port or system permission**: nothing to open in a firewall, nothing to authenticate on a socket entrance (the
  owner of the process is the Harness, which already handed over the secrets in the handshake).

The Unix socket is left for the day two Harness instances need to share the same plugin — which V1 does not do
(routing is static per org, D4). The contract does not change: the messages are the same.

## 2. Protocol version

`PROTOCOL_VERSION` (today **1**) goes in the handshake on both sides. The supervisor refuses a different version with
`protocolo incompatível: o host fala 1 e o plugin falou N` ("incompatible protocol: the host speaks 1 and the plugin
spoke N"). There is no negotiation and no "try best effort".

A **new optional field** does not change the version: the reader ignores what it does not know (`serde` without
`deny_unknown_fields`). Did a field change from optional to required, change type, or a message get removed? Bump the
version.

## 3. Handshake

First line the host writes to the plugin's input:

```json
{"tipo":"handshake","protocolo":1,
 "integracao":"01a1f6a5-cd9b-7000-8000-000000000000",
 "org":"01a1f6a5-cd9b-7000-8000-000000000001",
 "kind":"whatsapp",
 "config":{"instancia":"pop-atende"},
 "segredos":{"apikey":"…"}}
```

- `integracao` and `org` are UUID v7 (the same ids as in the database).
- `config` is what the operator filled in the form (validated against the manifest schema by the API/Console, not by
  the supervisor).
- `segredos` (secrets) is the **only** place through which a secret reaches the plugin. It never goes in the
  environment or on the command line: `argv` shows up in `ps`/`/proc`, and third-party logs copy the environment.
- The plugin **does not inherit** the Harness environment. The process receives only `PATH`, `HOME`, `LANG`, `LC_ALL`,
  `TZ`, `TMPDIR` and whatever is declared in `PluginOptions::with_env` — the LLM, Jev and encryption keys stay out
  of reach of a third-party program.

The first line the plugin sends back is the acceptance, with the manifest:

```json
{"tipo":"handshake_ok","protocolo":1,
 "manifesto":{
   "nome":"WhatsApp","kind":"whatsapp","versao":"1.0.0","protocolo":1,
   "capacidades":{"text":true,"image_in":true,"file_in":true,"file_out":true,
                  "typing":true,"read_receipt":false},
   "limites":{"max_anexo_bytes":16777216,"max_texto_bytes":65536,
              "tipos_aceitos":["image/png","application/pdf"]},
   "configuracao":{"type":"object","properties":{"instancia":{"type":"string"}}},
   "segredos":{"type":"object","properties":{"apikey":{"type":"string"}}}
 }}
```

The manifest needs `nome`, `kind`, `versao`, `protocolo` and `configuracao`; `capacidades` and `limites` have defaults.
`configuracao` and `segredos` are **JSON Schema of an object** (`type: "object"`) — this is the minimum the supervisor
checks; the rest of the schema belongs to the Console.

Nothing else is accepted before `handshake_ok`: a message outside this is counted as garbage.

## 4. Messages

One JSON message per line, in both directions. `\n` ends a line; `\r\n` is also accepted; an empty line is ignored.

### Host → plugin

| `tipo`      | Fields                          | Plugin reply                             |
|-------------|---------------------------------|------------------------------------------|
| `handshake` | `protocolo`, `integracao`, `org`, `kind`, `config`, `segredos` | `handshake_ok` |
| `comando`   | `id`, `comando`                 | `resultado` with the same `id`           |
| `saude`     | `id`                            | `saude` with the same `id`               |
| `encerrar`  | —                               | none: the process must exit              |

`comando` is one of three (`tipo` inside `comando`):

```json
{"tipo":"comando","id":7,"comando":{"tipo":"enviar_texto","idempotencia":"turno:0",
   "peer":"5511999999999","texto":"olá"}}

{"tipo":"comando","id":8,"comando":{"tipo":"enviar_arquivo","idempotencia":"turno:1",
   "peer":"5511999999999","nome":"manual.pdf","mime":"application/pdf",
   "tamanho_bytes":1234,"sha256":"…","blob_ref":"<org>/<sha256>","legenda":"segue o manual"}}

{"tipo":"comando","id":9,"comando":{"tipo":"definir_presenca","peer":"5511999999999",
   "estado":"digitando"}}      // digitando | pausado  (typing | paused)
```

`idempotencia` comes from the `(turn_id, seq)` of the `outbox` (D11): if the plugin can deduplicate on the channel
side, use this key. The same file may be resent after a crash — delivery is "at least once".

### Plugin → host

| `tipo`         | Fields                                  | When |
|----------------|-----------------------------------------|------|
| `handshake_ok` | `protocolo`, `manifesto`                | reply to the handshake |
| `evento`       | `evento` (`InboundEvent`)               | a customer message arrived |
| `estado`       | `estado` (`conectado`/`desconectado`/`qr`/`erro`) (connected/disconnected/qr/error) | the channel state changed |
| `resultado`    | `id`, `ok`, `erro` (when `ok=false`)    | reply to a `comando` |
| `saude`        | `id`, `saude` (`ok`, `detalhe`)         | reply to `saude` |
| `erro`         | `codigo`, `mensagem`, `transitorio`     | unsolicited problem (becomes an `erro` state event) |

Normalized incoming event:

```json
{"tipo":"evento","evento":{
  "integracao":"01a1f6a5-…","id_externo":"wamid.ABC","peer":"5511999999999",
  "remetente":null,"texto":"quero um orçamento","anexos":[],"ocorrido_em":1795140000000,
  "metadados":{"reply_to":"wamid.XYZ"}}}
```

`id_externo` is the idempotency key of the delivery (the `inbox` uses `(integration_id, external_message_id)`); `peer`
is the contact/room; `ocorrido_em` is in milliseconds.

Attachment, in the two forms from the PRD:

```json
{"fonte":"blob","nome":"manual.pdf","mime":"application/pdf",
 "sha256":"<64 hex>","tamanho_bytes":1234}
{"fonte":"bytes","nome":"foto.jpg","mime":"image/jpeg","dados_base64":"…"}
```

(`fonte` means source: `blob` or `bytes`.)

The supervisor refuses an event without `id_externo`/`peer`/`ocorrido_em`, with a sha256 that does not have 64 hex
characters, or with an empty attachment — and counts it in `invalid_events` without delivering it to the engine.

## 5. Limits, deadlines and restart policy

| Item | Default | What happens when it is exceeded |
|---|---|---|
| Message size (both directions) | 1 MiB | the plugin's line is discarded — and everything up to the next `\n` with it — and counted as junk; a host message is refused with `mensagem de N bytes passa do limite de M` ("message of N bytes exceeds the limit of M") (the plugin is **not** brought down) |
| Handshake | 5 s | process killed, failure counted, restart scheduled |
| Command | 10 s | process killed, failure counted, restart scheduled; the caller gets "tempo esgotado" (timed out) |
| Health | 3 s | same |
| Consecutive failures | 5 | the integration becomes `Caida` (down): it no longer restarts on its own |
| Wait between restarts | 500 ms × 2, up to 60 s | — |
| Shutdown | 500 ms | after that the process is killed (`SIGKILL`) |

A command that exceeds its deadline brings the process down: the other commands in flight on the same integration fail
as "indisponível" (unavailable, transient) and are retried by the `outbox`. This is the price of not letting a stuck
process hold the queue.

The failure count **does not** reset when the plugin starts: starting does not prove it is healthy (a plugin that
crashes on every command starts every time). It resets when the plugin **answers** a command or a health check. So a
plugin that hangs on every delivery stops being restarted after the cap, instead of dropping messages forever.

The read queue is limited (256 lines): a plugin that prints in a loop fills the operating system's pipe and gets
blocked on writing, instead of filling the Harness's memory.

`Caida` is a final state for the supervisor: anyone who wants to try again creates a new `Supervisor` (the
Console/CLI can expose a "reconnect"), or restarts the Harness process.

## 6. Counters (for `/metrics` and for the "why" in the Console)

`CounterReport` (from `Supervisor::counters`): `lines`, `junk` (a line outside the protocol or above the cap),
`events`, `invalid_events`, `states`, `commands`, `health_checks`, `handshakes`, `failures` (consecutive),
`restarts`, `timed_out`, `aborts`. They are per integration — never summed across orgs.

## 7. Minimal plugin example

A program that only does the handshake and returns the required replies (Rust, with no dependency beyond `serde_json`):

```rust
use std::io::{BufRead, Write};
use serde_json::json;

fn main() {
    let stdin = std::io::stdin();
    let mut linhas = stdin.lock().lines();
    // 1. handshake
    let _handshake = linhas.next().expect("handshake").expect("linha");
    println!("{}", json!({
        "tipo":"handshake_ok","protocolo":1,
        "manifesto":{"nome":"Eco","kind":"eco","versao":"1.0.0","protocolo":1,
                     "capacidades":{"text":true},
                     "configuracao":{"type":"object","properties":{}}}
    }));
    std::io::stdout().flush().ok();
    // 2. request loop
    for linha in linhas {
        let Ok(pedido) = serde_json::from_str::<serde_json::Value>(&linha.unwrap()) else { continue };
        match pedido["tipo"].as_str() {
            Some("comando") => println!("{}", json!({"tipo":"resultado","id":pedido["id"],"ok":true})),
            Some("saude") => println!("{}", json!({"tipo":"saude","id":pedido["id"],"saude":{"ok":true}})),
            Some("encerrar") => break,
            _ => {}
        }
        std::io::stdout().flush().ok();
    }
}
```

The real plugin used to test the contract is `tests/fixtures/plugin_fixtura.rs` (modes `normal`,
`trava_no_handshake` (hangs at handshake), `trava_no_comando` (hangs on command), `trava_no_primeiro_comando` (hangs on
first command), `cai_depois_do_handshake` (crashes after handshake), `cai_no_primeiro_comando` (crashes on first
command), `cai_em_todo_comando` (crashes on every command), `lixo` (garbage), `protocolo_errado` (wrong protocol),
`manifesto_invalido` (invalid manifest)). The mode names are identifiers used by the tests, so they stay as they are.
It exists only for the tests: the distribution package (`scripts/build-dist.sh`) does not include it.

The fixture also honours `config.dump_dir`: it writes there `args.txt`, `env.txt`, `segredos.txt` (secret
names and lengths only — the value is never written to disk) and `comandos.jsonl`, plus the marker files
`travado-uma-vez` and `caido-uma-vez` for the "first command only" modes, and `saiu.txt` on `encerrar`.
That dump is how the tests prove what actually **arrived** in the plugin: that the secret travels only in
the handshake and never through argv or the inherited environment. The `lixo` mode prints one non-JSON
line and one 2 MiB line (above the 1 MiB cap); both must be discarded and counted as junk. With the cap of
three consecutive failures, `cai_em_todo_comando` makes the supervisor mark the integration down and stop
restarting it.

## 8. How the Harness uses the contract

```rust
use poph_plugins::{PluginOptions, Supervisor};
use poph_types::Integration;              // the trait, the same for native and plugin

let opcoes = PluginOptions::new("/opt/poph/plugins/whatsapp", integracao_id, org_id)
    .with_kind("whatsapp")
    .with_config(config_do_operador)
    .with_secret("apikey", segredo);      // only goes in the handshake

let canal = Supervisor::start(opcoes, receptor).await?;     // handshake already done here
canal.send_text(comando).await?;                             // through `Integration`
let saude = canal.health().await?;
canal.shutdown().await;
```

For tests and for a consumer that has no destination of its own, `poph_plugins::EventCollector` implements the same
trait keeping every event in memory (`all`, `inbound`, `states`, `count`, `wait_for`).

The `receptor` (receiver) implements `poph_types::Receptor`: it receives `EventoIntegracao::Entrada(InboundEvent)` and
`EventoIntegracao::Estado(EventoEstado)`. The ready-made link between the contract and the queues is in
`poph-agent/src/fila_integracao.rs` (HR-27): `EnvioPeloCanal` (outbox item → `OutboundCommand`) and `InboxDoCanal`
(`InboundEvent` → `inbox`). The full example is `docs/PLUGIN_EXEMPLO.md`.

## 8.1 The manifest on the server (HR-44c)

The server does not store the manifest: it asks the plugin. `[plugins].integracoes` declares `kind` + `programa`
(+ `argumentos`); on the first read of the manifest (route `GET /admin/v1/orgs/{org_id}/manifests/{kind}` or the first
configuration of an integration of that type), the server starts the process through the supervisor, does the
handshake, reads the `manifesto`, ends the process and keeps the result in memory. A failure is cached for 30 s so it
does not turn into a loop of processes.

What validates the operator's configuration against the schemas is the API (`docs/API.md`), with the subset of JSON
Schema described there: `type`, `required`, `enum`, `minLength`/`maxLength`, `minimum`/`maximum` and nested
`properties`.

The catalogue serializes manifest resolution under a mutex so concurrent requests cannot start the same plugin
twice. A successful handshake is cached for the lifetime of the server. The list uses the same cache as an
individual lookup and returns items in kind order. One failed plugin appears as unavailable without failing
the list. Responses contain the declared kind, availability, and public manifest metadata or an operator-facing
message. The server does not send integration configuration or secrets to this handshake. Program paths and
system errors stay in the server log.

## 9. What does not exist yet

- **External plugin as a channel in `poph serve`**: the server starts the plugin only to read the manifest
  (`[plugins].integracoes`, `poph-server/src/manifests.rs`). Receiving and sending customer messages through an
  external plugin inside the server is not connected; the link between the contract and the queues exists
  (`poph-agent/src/fila_integracao.rs`, `docs/PLUGIN_EXEMPLO.md`) and is exercised by a test.
- **Re-encryption of secrets** (`integrations.secrets_enc`) when `POPH_CHAVE_CIFRA` changes.
- **Observer channel for the conversation** (Matrix): see `docs/CONTRATO_CANAL.md` §7.

The native WhatsApp channel (HR-26) and the Console form generated from the manifest (HR-44) exist.

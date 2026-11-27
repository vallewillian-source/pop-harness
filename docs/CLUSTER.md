# POP Harness cluster — static routing between instances (HR-50a)

Each organization lives in **one** instance, with its data in that instance's SQLite database (PRD §12). The map
of which org lives in which instance is a configuration file, `cluster.toml`, separate from `poph.toml`, and it
**must be identical on every instance**. It is static on purpose: when the map changes, you edit the file and
request a reload. There is no dynamic hash that "moves" orgs on its own and leaves orphaned data behind.

This document covers the **map** (format, validation, fingerprint, the `poph cluster show|check` commands, and
reloading the running process with `POST /admin/v1/cluster/reload`) and the **forwarding** of webhooks, admin
calls, live chat (SSE) and `?cluster=true` between instances (section "Forwarding between instances"; HR-50a,
HR-50b-1 and HR-50b-2, all done).

## Where the file lives

| Source | Path |
| --- | --- |
| `POPH_CLUSTER` (environment) | the given path, when not empty |
| default | `cluster.toml` **in the same directory as the resolved `poph.toml`** (`--config`, `POPH_CONFIG` or `./poph.toml`) |
| no `poph.toml` | `cluster.toml` in the folder where the process runs |

**Without `cluster.toml`, the installation runs in single-instance mode**: all orgs live in it, and nothing changes
from today. An empty file (only comments) is also single-instance mode; a file with any key **must** identify
itself with `instance_id`.

The **shared secret** between instances (the forwarding HMAC, HR-50b) comes **only** from `POPH_CLUSTER_SECRET`,
in the environment. It is never written to the file, never printed, and never appears in a response, log or
error. `poph serve` reads the secret at startup; changing it requires restarting the process.

## Format

```toml
# This instance's name in the map. Required when cluster.toml exists.
instance_id = "a"

# Where orgs that are not in the map go. Required when the cluster has more than one instance; with only one,
# it is the implicit default.
default_instance = "b"

# Cluster instances: short id and base address (http:// or https://, with a destination).
[[instances]]
id = "a"
url = "http://10.0.0.11:7480"

[[instances]]
id = "b"
url = "https://harness-b.example"

# Explicit org → instance map, in TOML's [orgs] format. The key is the org id (UUIDv7, without prefix).
[orgs]
01a1ff21-8f3c-7a2b-9c4d-5e6f70819203 = "a"
01a1ff21-8f3c-7d10-8e2f-a1b2c3d4e5f6 = "b"
```

Validated rules (each problem names the key):

- `instance_id` is required, not empty, up to 64 characters (`letters`, digits, `-`, `_`, `.`);
- each instance: `id` in the same format, not repeated, and `url` starting with `http://` or `https://` with a
  destination (no spaces);
- the local instance (`instance_id`) must be declared in `instances`;
- `default_instance`, when declared, must exist in `instances`; it is **required** when the cluster has more than
  one instance (otherwise the orgs outside the map would have no owner);
- each org in `[orgs]` must point to a declared instance; a repeated key in `[orgs]` is refused by the TOML parser
  itself (the file is not even read as configuration);
- unknown field, invalid TOML and unreadable file give an error with the **file** and the **key** (when there is one).

## Fingerprint

`poph cluster show` and the reload return a stable fingerprint of the map: local instance, resolved default
instance, instances sorted by id and orgs sorted by key. Same map, same fingerprint (the order in the file does not
matter); an org moved to another instance changes the fingerprint. It is not cryptographic and **does not include
the secret** — it is used to detect changes, not to authenticate.

Because the local instance is part of this fingerprint, two instances of the same cluster have **different**
fingerprints. To compare the map between them there is a second fingerprint, the **map fingerprint** (without the
local instance): it is what forwarding sends in `x-poph-mapa`, and what `show` prints in the line "map fingerprint
(without the local instance)" (`fingerprint_mapa` in `--json`). The same `cluster.toml` except for `instance_id`
= the same map fingerprint.

## Commands

```bash
poph cluster show            # effective map: mode, local instance, orgs per instance, fingerprint
poph cluster show --json     # stable format: versao_relatorio = 1, no secret
poph cluster check           # validates and lists ALL problems; exits with 1 when there is any
poph cluster check --json
```

- Without `cluster.toml`: `show` says "modo: instância única" (single-instance mode); `check` says there is nothing to fix.
- Cluster declared without `POPH_CLUSTER_SECRET`: `check` fails and says which variable to set.
- `show`/`check` do **not** start a server or touch the database; they only read the file and the environment.
- No output includes the secret value (only "segredo compartilhado: definido" / "não definido", as printed by the code).

## Reloading without restarting

`POST /admin/v1/cluster/reload` (**global** token; org token → `403`; no token → `401`) rereads the file in the
running process. The full contract is in `docs/API.md`, section "Roteamento entre instâncias: mapa e recarga"
(the section title is still in Portuguese, as it is in `docs/API.md`).

- Valid new file → the map takes effect immediately; the response includes `fingerprint_antes` (before),
  `fingerprint_depois` (after), a summary of both maps, the orgs that changed instance, and the instances added/removed.
- Invalid file → `400 cluster_invalido` with `details.problemas`; **the previous map stays in use** — a typo does
  not break routing.
- File removed → back to single-instance mode (this is what the reload reflects).

## Forwarding between instances (HR-50b-1, HR-50b-2)

The map is now applied for real: a request for an org that lives in another instance is forwarded to its owner,
signed. **Admin calls** (`/admin/v1/orgs/{org_id}/…`) and **Evolution webhooks** are forwarded; health, metrics,
OpenAPI, reload, `/admin/v1/orgs` and the Console always stay on the instance that received the request — none of
them has an org in the path. Without `cluster.toml` (single-instance mode), nothing is forwarded.

**Peer headers**

| Header | Content |
| --- | --- |
| `x-poph-instance` | `instance_id` of the forwarding instance |
| `x-poph-timestamp` | time of sending, in Unix seconds |
| `x-poph-signature` | HMAC-SHA256 in hexadecimal |
| `x-poph-hop` | `1` on the first (and only) hop |
| `x-poph-mapa` | **map** fingerprint of the forwarding instance |
| `x-poph-nonce` | 16 random bytes in hexadecimal (32 characters), one per request |

**Signed text** (key: `POPH_CLUSTER_SECRET`; lines separated by `\n`, no line break at the end):

```text
method
path+query
hex(sha256(body))
timestamp
instance
hop
nonce
```

`x-poph-mapa` is `ClusterConfig::fingerprint_do_mapa` (the map fingerprint): the same `cluster.toml` on two
instances produces the same fingerprint even with a different `instance_id` — this is what makes it possible to
detect a divergent map. The fingerprint shown by `poph cluster show` still includes the local instance (it
identifies that instance's file); these are different values with different purposes.

**Rules**

- Timestamp window: 60 s back and forth.
- The **nonce** is what makes each request unique: two identical requests in the same second, with different
  nonces, both pass; the same nonce within 120 s is `401 assinatura_invalida` (replay protection, kept in process
  memory; cap of 10,000 nonces). A missing nonce, or one outside the format (32 hexadecimal characters), is
  `401 assinatura_invalida`.
- `x-poph-hop > 1`, or an org that is not this instance's in a request that was already forwarded: `421 rota_em_laco`.
- `x-poph-mapa` different from the local one: `421 mapa_divergente` with both fingerprints in `details`.
- Owner down: `503 instancia_fora` with `details.instancia` and `details.url`. Timeouts: 5 s to connect, 60 s for
  the owner to start responding; after that the body is streamed.
- A request **without** peer headers follows the usual token path, with no change.
- The shared secret never appears in a response, error or log.

**Test chat SSE (HR-50b-2).** `GET …/test-chat/{conversa}/messages` (and the `POST` that opens the stream) goes
through forwarding without buffering: the first event arrives in milliseconds, events flow one by one, and the 60 s
limit applies only until the owner starts responding — after that the stream has no time limit. A client that
disconnects drops the forwarded request, and the turn is cancelled on the owner. Measured in the two-instance test:
first event in ~0.36 s with the turn hanging for 30 s.

**Org listing with `?cluster=true` (HR-50b-2).** `GET /admin/v1/orgs` stays local (it is not forwarded). With the
parameter and a **global token**, it queries the neighbor instances in the map in parallel (`/admin/v1/orgs` of
each one, with the caller's `Authorization`, 2 s limit per instance) and returns all orgs in `items`, each with
`instancia`, plus `instancias` with `id`, `url`, `situacao` (`local`, `ok`, `inacessivel`) and the org count. An
instance that is down is marked and does not break the list. Without the parameter the response is the usual one;
with it there is no pagination. An org token ignores the parameter and sees only its own org.

**`agents_active_count` (CF-03a).** Every item of the org listing carries this integer in both forms (with and
without `cluster=true`) and in `GET /admin/v1/orgs/{org_id}`: how many agents of the org have `status = active`
(`inactive`/`archived` do not count; an org with no active agent answers `0`). A page is counted with one
aggregated query (`org_id IN (…) GROUP BY org_id`), never one query per row. With `cluster=true` the number comes
in the payload of the instance that owns the org; a neighbour on an older version that does not send the field yet
shows `0`.

**`/healthz` in cluster mode** returns `instance_id` and `cluster_map_fingerprint` (additive, absent in
single-instance mode).

**`poph cluster check --remoto`.** Queries the `/healthz` of each declared instance (except the local one, which is
the file itself), in parallel, with a 3 s limit each, and reports:

- **inacessível** (unreachable) — it did not respond, responded with an error, or responded without stating the map
  (older version);
- **mapa divergente** (divergent map) — its map fingerprint is different (update `cluster.toml` on both sides and reload both);
- **identidade divergente** (divergent identity) — it declares itself with a different `instance_id` (fix its `instance_id`).

It exits with code 1 when there is any problem. Without `--remoto`, the command keeps validating only the file, as
before. In JSON, the report includes `remoto` (one item per instance) and `problemas_remotos`.

### Request body and response streaming

The receiving owner applies the original authorization with its usual scope checks. The forwarding middleware reads
the full request body before sending it because the signature covers its SHA-256 digest. It uses the same body cap
as the receiving route: 1 MiB for the Evolution webhook, 24 MiB for test chat, 64 MiB for backup import, and 2 MiB
for the other org routes. An oversized request receives `413 payload_too_large`. The owner's response body is
streamed back to the caller.

The forwarding integration tests check that a backup export retains its attachment headers and
complete body through the forwarding instance. Flow and Case operations follow the same owner
routing, with writes confined to the owner. In single-instance mode, a request with peer headers
and no configured shared secret is refused with `401`.

## What does NOT exist yet

- **Automatically moving an org to another instance:** changing the map and reloading only changes routing; the
  data stays in the old instance until a manual export/import.
- **Retry/failover:** if the owner fails in the middle of a request, the error goes back to the client; there is no
  retry on another instance.
- **Forwarding with a global token:** the original `Authorization` stays attached, and the owner is the one who validates it.
- **Map in `/healthz` of older versions:** an instance older than HR-50b-2 shows up as unreachable in
  `cluster check --remoto` (it does not state the map).

## Known limits

- **Medium, body of a forwarded request:** the body is read in full before it is forwarded (the signature uses the
  sha256); the owner's response is streamed. A large forwarded body occupies memory on the receiving instance and on
  the owner at the same time, up to the route's limit.
- **Medium, `?cluster=true` listing without pagination:** returns the whole cluster snapshot (up to 20 pages of 200
  orgs per neighbor instance); on an installation with many orgs the response is large and the Console screen has
  to paginate on the client side.
- **Medium, remote listing with the caller's token:** if the token is valid on this instance and not on the neighbor
  (different global tokens), the neighbor shows as `inacessivel` with the HTTP code.
- **Low, `cluster check --remoto` does not check the local instance:** it is the file in use; the command looks at
  the neighbors. The local URL in the map is not queried.
- **Low, `cluster check --remoto` uses `/healthz`:** a public route on the internal network, without the cluster
  secret; anyone who can reach the port can read the map fingerprint (it is not a secret).
- **Medium, `cluster.toml` reloaded only on the instance that received the reload:** the others also need
  `POST /admin/v1/cluster/reload`; a different map between them gives `421 mapa_divergente` (the request is refused,
  never processed with the wrong map).
- **Medium, reload reflects the file:** if the file disappears (or `POPH_CLUSTER` points to a wrong path) and the
  operator requests a reload, the instance goes back to single-instance mode. The reload is explicit; there is no
  protection against this.
- **Low, secret at startup:** changing `POPH_CLUSTER_SECRET` while the process is running does not change the secret
  in use; restart it.
- **Low, map cache:** the map is read from the file at startup and on reload; editing the file without requesting a
  reload changes nothing (that is the design: static).
- **Low, `[orgs]` by id:** the key must be the org id (UUIDv7, like `01a1ff21-8f3c-7…`, without the `org_` prefix),
  which is what appears in the route path and in the webhook. A slug is not resolved by the map.
- **Low, no audit trail:** the reload does not write to `audit_log` (it is instance configuration, not org data);
  the log and the response with before/after are kept.

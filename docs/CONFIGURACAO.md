# POP Harness: configuration, logs and `poph doctor`

Delivered in HR-06. Defaults marked as **proposal** were chosen at this stage and can be calibrated; the others come
from the PRD (§7.4, §8.4, §10, §15) or from POP Atende.

## Precedence

From weakest to strongest: **default < TOML file < environment variable**.

File, in search order:
1. `--config <file>` (applies to any command);
2. the `POPH_CONFIG` variable;
3. `poph.toml` in the current folder, if it exists.

Without a file, the defaults and the environment apply. A file that was requested and does not exist is an error.
The cluster resolver returns its candidate path even before the file exists, so reload knows where to look.

The loader flattens the file into individual key paths and overlays them on the defaults before applying the
environment. An empty table supplies no values. If the merged table fails to deserialize, the loader tries each
source value in isolation against the defaults and reports the first failure with its origin and key. Domain
validation runs after deserialization.

## Configuration errors

Each error says where it came from (the file or the variable) and which key is involved:

- `arquivo.toml: chave \`servidor.portaa\` inválida: unknown field ...` (unknown key)
- `arquivo.toml: erro de sintaxe no TOML: ...` (the message includes line and column)
- `POPH_SERVIDOR_PORTA: valor inválido (esperado um número inteiro)`
- `configuração inválida: campo servidor.porta inválido: ...` (range or format rule)

> Note: the messages above are emitted by the code in Portuguese, so they are kept as they are.

## TOML file fields

| Key | Default | Rule | Origin |
|---|---|---|---|
| `servidor.endereco` | `127.0.0.1` | IP address | proposal |
| `servidor.porta` | `7480` | 1024 to 65535 | proposal |
| `dados.diretorio` | `data` | non-empty path | HR-04 (`.gitignore`) |
| `dados.harness_db` / `rag_db` / `blobs` | inside `diretorio` | optional | PRD §7.1 |
| `dados.modelos` | `modelos` inside `diretorio` | folder of the embedding model (`poph model install`) | HR-15 (D-09) |
| `log.nivel` | `info` | trace, debug, info, warn or error | PRD §15 |
| `log.formato` | `legivel` | `legivel` or `json` | PRD §15 |
| `retencao.dias` | `30` | `0` = do not delete | PRD §7.4 |
| `limites.turnos_simultaneos` | `200` | 1 to 10000 | D-07 |
| `limites.turnos_simultaneos_por_org` | `50` | 1 up to the total | **proposal** |
| `limites.taxa_por_org_por_minuto` | `600` | 1 to 100000 | **proposal** |
| `llm.provedor` | `antigravity2api` | `openai_compativel` or `antigravity2api` (same client) | PRD §8.4 |
| `llm.url_base` | `http://127.0.0.1:8045/v1` | any endpoint that speaks the OpenAI protocol, http or https | POP Atende (`ANTIGRAVITY_BASE_URL`) |
| `llm.chave_env` | `ANTIGRAVITY_API_KEY` | name of the environment variable holding the key | PV-01 |
| `llm.recusa_turno_modelo` | `true` | refuse a request ending on a model turn (the gateway answers HTTP 400) | PV-01 |
| `llm.modelo` | `gemini-3.8-flash-tiered` | non-empty | PRD D6 |
| `llm.timeout_segundos` | `150` | 1 to 600 | POP Atende (`ANTIGRAVITY_TIMEOUT_SECONDS`) |
| `llm.retentativas` | `2` | 0 to 5, only on transient errors | POP Atende (`LLM_RATE_LIMIT_RETRIES`) |
| `llm.simultaneos` | `32` | 1 to 1000 | **proposal** |
| `llm.temperatura` | `0.0` | 0 to 2 | POP Atende (`LLMService` default) |
| `llm.max_tokens_saida` | `4096` | 1 to 131072 | POP Atende (`AGENT_MAX_OUTPUT_TOKENS`) |
| `[provedores.<nome>]` | empty | named provider an agent's `provedor` field can choose; the name is `[a-z0-9_-]` and cannot be `openai_compativel` or `antigravity2api` | PV-02 |
| `provedores.<nome>.url_base` | from `[llm]` | any endpoint that speaks the OpenAI protocol, http or https | PV-02 |
| `provedores.<nome>.chave_env` | from `[llm]` | name of the environment variable holding the key; without it the provider is not registered and the log warns at startup | PV-02 |
| `provedores.<nome>.provedor` / `recusa_turno_modelo` / `modelo` / `timeout_segundos` / `retentativas` / `simultaneos` / `temperatura` / `max_tokens_saida` | from `[llm]` | same rules as the `[llm]` key of the same name; `simultaneos` is that provider's own concurrency cap | PV-02 |
| `jev.ativo` | `false` | turns Jev on; only after measurement (PRD §10) | PRD §10 |
| `jev.url_base` | `https://api.typesafe.ai` | http or https | POP Atende (`TYPESAFE_BASE_URL`) |
| `jev.modelo` | `jev-latest` | non-empty | POP Atende (`JEV_MODEL`) |
| `jev.timeout_segundos` | `8` | 1 to 60 | PRD §8.5 |
| `jev.retentativas` | `1` | 0 to 3, only on 429 or 5xx | PRD §8.5 |
| `jev.gates.g1` … `g9` | g3, g5, g8 and g9 `true`; the others `false` | turns each gate on; installation default: g3, g5 and g8 since 20/11/2026, g9 per the flows PRD §9 | PRD §10, `docs/JEV.md`, flows PRD §9 |
| `[evolution] url_base` | absent | section exists = WhatsApp integration configured | POP Atende |
| `[[plugins.integracoes]] kind` / `programa` | none | `kind` unique and non-empty; `programa` is the plugin's local path | HR-44c |
| `[[plugins.integracoes]] argumentos` | `[]` | process arguments; **never** a secret (it shows up in `ps`) | HR-44c |
| `[matrix] habilitado` | `false` | turns the Matrix module on; section absent or `false` = module off (none of it loads) | MX-07 |
| `matrix.homeserver_url` | — | base address of the Synapse client API (`http://` or `https://`); checked only with the module on | MX-07 |
| `matrix.server_name` | — | domain of the Matrix ids, as in the Synapse `server_name` (e.g. `matrix-dev.local`, `localhost:18008`) | MX-07 |
| `matrix.prefixo` | `ph` | prefix for virtual users, aliases and `sender_localpart`: `[a-z0-9]{1,12}` | MX-07 |
| `matrix.concorrencia` | `8` | rooms mirrored in parallel; 1 to 64 | MX-07 |
| `matrix.intervalo_ms` | `500` | wait between cursor reads; 50 to 60000 ms | MX-07 |
| `matrix.poder_agente` | `50` | power level of the agent's ghost in the room; 0 to 100 | MX-07 |
| `matrix.midia_retencao_dias` | `4` | days media is kept in Synapse (phase 2); 1 to 365 | MX-07 |
| `fluxos.ativo` | `true` | turns the flow engine on: the G9, the prompt, the tools and the effects stop when `false`; Fichas keep being created and closed | FX-26, flows PRD §9 |
| `fluxos.max_fluxos_por_org` | `100` | published flows per org; 1 to 10000 | flows PRD §9 |
| `fluxos.max_fonte_bytes` | `32768` | size of a flow script, in bytes; 1024 to 1048576 | flows PRD §9 |
| `fluxos.max_nos` | `60` | steps, decisions and ends per flow; 1 to 60 (the compiler does not accept more) | flows PRD §9 |
| `fluxos.max_dados_bytes` | `65536` | size of an execution's `dados_json`, in bytes; 1024 to 10485760 | flows PRD §9 |
| `fluxos.max_ticks_regra` | `10000` | steps of one rule evaluation before it is cut off; 100 to 1000000 | flows PRD §9 |
| `fluxos.max_heap_regra` | `1048576` | memory of one rule evaluation, in bytes (1 MiB); 65536 to 67108864 | flows PRD §9 |
| `fluxos.max_saltos_decisao` | `10` | decisions one pass goes through before it fails; 1 to 100 | flows PRD §9 |
| `fluxos.cache_revisoes_mb` | `16` | RAM cap of compiled flow revisions, in MB; `0` keeps none (each use recompiles); 0 to 4096 | flows PRD §9, FX-19 |

An agent chooses a named provider with its `provider` field (`--provider` in the CLI, `provider` in the API);
empty, `openai_compativel` or `antigravity2api` use the `[llm]` default. The provider contract, both ways of
configuring an endpoint, the gateway-specific rules and how to test without a real key are in
[`PROVEDORES.md`](PROVEDORES.md).

The plugin program is a **local path on the server** (it never comes from a request). With it, the server discovers
the manifest of each `kind` — starting the plugin once and keeping the result in memory — and validates the
integration configuration. Without the declaration, an integration `config_json` is refused with
`422 manifest_indisponivel` (the message code stays as it is) and the manifest route answers `404`.

### Site conversation settings

`[site_chat]` configures the administrative routes for trusted website adapters. The
`max_text_bytes` limit defaults to 16384 and accepts 1–262144 UTF-8 bytes. The `allowed_tools`
list defaults to empty. It accepts `consultar_base`, `buscar_produto`, `cart_add_item`,
`cart_remove_item`, `cart_summary`, `cart_clear` and `register_lead`. Agents with any enabled tool missing from
this list are refused. The setting is server-side and requires no additional credential.
See [Site conversations](API.md#site-conversations) for the HTTP contract.

### Internal HTTP tools

Declare a tool under `[http_tools."<organization UUID>".<tool name>]` in the server's `poph.toml`.
The agent must enable the same name in its structured instructions. Names must match
`^[a-z][a-z0-9_]{2,40}$`. Built-in tool names and variants of `register_lead` are reserved.
The exact name `register_lead` remains available as an HTTP tool, including on the site when explicitly
allowed by `[site_chat].allowed_tools`. Other HTTP tools are available only outside site conversations.

~~~toml
[http_tools."0199a11d-2000-7000-8000-000000000001".register_lead]
url = "https://site-service.internal/internal/leads"
method = "POST"
authorization_env = "POPH_HTTP_TOOL_SITE_LEADS_AUTHORIZATION"
description = "Registra o interesse do visitante com o consentimento confirmado."
parameters = { type = "object", properties = { consent = { type = "boolean" } }, required = ["consent"] }
response_fields = { ok = "/ok", status = "/status" }
~~~

Set the named environment variable to the complete `Authorization` header value, such as
`Bearer <org service token>`. Use an organization-scoped credential, provisioned by the receiving
service. The destination and schema are trusted server configuration. Supported methods are
`POST`, `PUT`, `PATCH`, `GET` and `DELETE`. Each sends a JSON body. The
`timeout_ms` setting defaults to 5000 and accepts 1–60000. Redirects, proxies and automatic
HTTP retries are disabled.

The request carries `Idempotency-Key: <turn_id>:<call_id>`. The body has `context` with
`org_id`, `agent_id`, `conversation_id`, `turn_id` and `message_id`, plus the original
provider `call_id` and a separate `arguments` object from the model. The receiving service
resolves the organization from the credential and verifies the supplied context. Treat
`arguments` as untrusted input. The message ID identifies the message that triggered the turn.

The tool returns the selected response fields as JSON. Each configured field maps to a JSON Pointer
in the service response. Missing fields are omitted. The turn history records the call and a bounded
summary of the selected fields, with values redacted for keys such as `token` and `password`.
Select fields suitable for storage in the turn history. For `register_lead`, the result remains omitted.
HTTP arguments are omitted for all HTTP tools. A non-success status, timeout, malformed
response or response with no selected fields returns a stable tool error and lets the model
finish the turn. JSON responses may be up to 64 KiB.

Within the same turn, an identical call ID and arguments replay the recorded outcome.
Changing the arguments under that ID returns a conflict. The receiving service must enforce
durable uniqueness for the same turn and call ID, including when a request reaches it and the
Harness times out waiting for the response.

## Environment variables

Non-secret fields (they override the file):

| Variable | Field |
|---|---|
| `POPH_SERVIDOR_ENDERECO` | `servidor.endereco` |
| `POPH_SERVIDOR_PORTA` | `servidor.porta` |
| `POPH_DADOS_DIRETORIO` | `dados.diretorio` |
| `POPH_LOG_NIVEL` | `log.nivel` |
| `POPH_LOG_FORMATO` | `log.formato` |
| `POPH_RETENCAO_DIAS` | `retencao.dias` |
| `POPH_LLM_URL_BASE` | `llm.url_base` |
| `POPH_LLM_MODELO` | `llm.modelo` |
| `POPH_JEV_ATIVO` | `jev.ativo` (`true` or `false`) |
| `POPH_JEV_URL_BASE` | `jev.url_base` |

An empty variable overrides nothing.

| `POPH_FLUXOS_ATIVO` | `fluxos.ativo` (`true` or `false`) |
| `POPH_FLUXOS_MAX_FLUXOS_POR_ORG` | `fluxos.max_fluxos_por_org` |
| `POPH_FLUXOS_MAX_FONTE_BYTES` | `fluxos.max_fonte_bytes` |
| `POPH_FLUXOS_MAX_NOS` | `fluxos.max_nos` |
| `POPH_FLUXOS_MAX_DADOS_BYTES` | `fluxos.max_dados_bytes` |
| `POPH_FLUXOS_MAX_TICKS_REGRA` | `fluxos.max_ticks_regra` |
| `POPH_FLUXOS_MAX_HEAP_REGRA` | `fluxos.max_heap_regra` |
| `POPH_FLUXOS_MAX_SALTOS_DECISAO` | `fluxos.max_saltos_decisao` |
| `POPH_FLUXOS_CACHE_REVISOES_MB` | `fluxos.cache_revisoes_mb` |

`POPH_CLUSTER` is not a field of `poph.toml`: it points to the path of the `cluster.toml` for routing between
instances (default: `cluster.toml` in the same directory as `poph.toml`). Format and rules in `docs/CLUSTER.md`.

`RUST_LOG`, if set, takes priority over `log.nivel` in the log filter.

## Secrets (environment only)

They never go into a file, report, log or API response. `poph doctor` only shows whether they are set.

| Variable | Use | Required |
|---|---|---|
| `POPH_CHAVE_CIFRA` | encryption key for stored secrets (AES-256-GCM, from HR-07) | always |
| `ANTIGRAVITY_API_KEY` | LLM provider | always |
| `TYPESAFE_API_KEY` | Jev | only with `jev.ativo = true` |
| `EVOLUTION_API_KEY` | Evolution (WhatsApp) | only with the `[evolution]` section |
| `POPH_CLUSTER_SECRET` | shared secret between instances (forwarding HMAC, HR-50b) | only with a `cluster.toml` that declares instances |
| `POPH_MATRIX_AS_TOKEN` | Application Service token of the Matrix module, which Harness presents to Synapse | only with `[matrix] habilitado = true` |
| `POPH_MATRIX_HS_TOKEN` | Application Service token that Synapse presents to Harness | only with `[matrix] habilitado = true` |
| `POPH_MATRIX_ADMIN_TOKEN` | Synapse administrative API (deletes expired media) | **phase 2**; not read yet |

The names `ANTIGRAVITY_API_KEY`, `TYPESAFE_API_KEY` and `EVOLUTION_API_KEY` are the same as in POP Atende, so the
same environment can be reused. An empty value or one with only spaces counts as missing.

The Matrix module has its own rule: **off, nothing from it is loaded**. Without `[matrix]` in the file, or with
`habilitado = false`, the section's configuration is not checked (it may be empty), the two tokens are not read,
and no task or route of the module exists. With `habilitado = true`, the section is validated — the error names the
key and never repeats the value — and both tokens become required: if one is missing, startup refuses with the
variable name, without printing the value. The tokens live in the `Sensitive` type and do not appear in `Debug`,
`Display`, logs, errors or responses (`crates/poph-matrix/src/config.rs`).

The Matrix API client (`crates/poph-matrix/src/cliente.rs`, MX-09) uses a **fixed 10 s limit per call**
(`TEMPO_LIMITE_PADRAO`): the `[matrix]` section has no timeout key, and `concorrencia` and `intervalo_ms` belong to
the loop, not to the client. A `429` with `retry_after_ms` is expected and retried **once** (up to 60 s of wait);
beyond that the client returns the error with the requested wait, and the loop is the one that waits. No other
retry happens inside the client: the loop decides based on transient errors (network, timeout, 429, 5xx) and
definitive ones (4xx, response outside the contract).

### Application Service registration (`poph matrix registro`)

```bash
poph matrix registro --saida /etc/poph/registro.yaml --url https://harness.exemplo.com
poph matrix registro --verificar /etc/poph/registro.yaml
```

- Registration works without an org context. It reads the `[matrix]` section and the two tokens from the
  environment and writes the file with **0600** permissions on Unix, including when `--forcar` replaces an
  existing file. Keep the generated registration out of version control. The CLI never prints either token.
- `--url` is the address where **Synapse reaches Harness** (the section's `homeserver_url` is Synapse's address).
  Without `--url`, `POPH_MATRIX_URL_AS` is used.
- `--id` (default `poph-matrix`) becomes the Application Service `id`; in phase 3, each instance has its own.
- Registration validates the Matrix settings as enabled, including when the module is off. The registration
  defines the Application Service for the instance independently of the module's current state.
- The generated YAML sets `rate_limited: false` to keep Synapse from throttling bursts by the bot. The instance's
  exclusive user and alias namespaces coexist with the non-exclusive `#dep_.*` alias shared with the Python
  registration.
- `--verificar` reads the generated YAML and the repository's registration template. It accepts top-level
  scalars and the `users`, `aliases` and `rooms` namespace groups. It ignores the room list and reports
  unsupported syntax by line where it can. It compares `url`, `sender_localpart`, `rate_limited`,
  `namespaces.*` and both tokens. Token diagnostics identify the affected field without printing its value.
- Synapse reads registrations at startup. Restart the environment's Synapse after generating one.

### Operating the Matrix module per org (`poph matrix estado|ligar|desligar|espaco`, MX-23)

```bash
poph --offline --org loja matrix estado [--json]
poph --offline --org loja matrix ligar [--json]
poph --offline --org loja matrix desligar [--json]
poph --offline --org loja matrix espaco [--json]
poph --offline --org loja matrix espaco --departamento <id> [--espaco <!id|#alias>] [--json]
poph --offline --org loja matrix espaco desligar --departamento <id> [--json]
```

- Like the other administrative CLI commands (`org`, `department`, `token`), they require `--offline` and `--org`
  (slug or id) and work directly on the database. The CLI does **not** use a token and does **not** talk to the
  server. `poph serve` picks up the change in the next cycle, without restarting.
- `estado` shows whether the module is on, the backfill cutoff, how many messages are still to be mirrored, the
  isolated failures and the last reason. It does not show whether Synapse is responding: that probe runs on the
  server (in the API, the `connected` field).
- `ligar` and `desligar` read the org's version (the same one as the API's `ETag`) before writing. If it changes
  between the read and the write, nothing is written and the exit code is `3`. Turning off keeps the backfill
  cutoff and does not delete any room or space.
- `espaco --departamento` creates the department's space; with `--espaco` it links a space that already exists in
  Element (id `!…` or alias `#…`). It talks to the Synapse in `[matrix]`, so it requires `POPH_MATRIX_AS_TOKEN` and
  `POPH_MATRIX_HS_TOKEN` in the environment. Repeating it does not create a second space.
- `espaco desligar --departamento <id>` removes the department's space only from Harness. Nothing leaves Element.
  On the next conversation change for that department, the room is placed back into the same space.
- `espaco` without `--departamento` lists the org's spaces.
- Output: `0` ok; `1` error (org or department does not exist, configuration, Synapse or database); `3` the version
  changed between the read and the write. `--json` has a stable format, with `versao_relatorio` = 1.
- Unlike the API, the command writes audit records with the actor `cli`.

## `poph doctor`

```
poph doctor            # readable output
poph doctor --json     # stable output for machines (versao_relatorio = 1)
poph --config arq.toml doctor
```

Checks: the binary and the compiler used; the configuration file; valid configuration; the flows module (`fluxos.ativo` and the caps in use, as information); the data folder (exists and
accepts writes, or can be created); `harness.db` and `rag.db` (SQLite format and schema version, read from the
`schema_migrations` table, in read-only mode); required variables.

- It writes only a test file in the data folder and deletes it right after.
- A missing database is a **warning** (create it with `poph migrate`). A database that exists but is not SQLite is an **error**.
- With the flow engine off, the report shows a **warning**, not an error: cases keep being created, only the engine stays stopped. The caps always appear, because they apply even with the engine off (validation runs first).
- Exit code: `0` no error (warnings do not count); `1` with an error; `2` when it runs without a command.

## `poph migrate` (HR-07)

```
poph migrate            # text: version before → after and what was applied
poph migrate --json     # versao_relatorio = 1: harness, rag, ja_estava_em_dia
```

Creates and/or updates `harness.db` and `rag.db` from the configuration. It is **idempotent**: running it again
applies nothing and exits with `0`. Run it with the server stopped — the server opens its own write connection, and
the storage rule is one writer at a time. The server also migrates on its own when it opens the database. The schema,
the tables and the storage decisions are in `docs/DADOS.md`.

## `poph model install` (HR-15)

```
poph model install           # text
poph model install --json    # versao_relatorio = 1: files, bytes, sha256 and situacao
```

Downloads to `dados.modelos` the embedding model (int8, 118 MB) and the tokenizer (9 MB). Each file is downloaded to a
temporary name, its sha256 is checked, and only then does it get the final name: a wrong hash never stays in the
folder. A file that already checks out is not downloaded again.

- It is the **only** command that downloads a model. A turn never downloads silently: if the model is missing, the
  error tells you to run `poph model install`.
- Model loading: checks the sha256 values at startup; if a file is missing or different, Harness does not start and
  the error says what to do.
- Exit code: `0` installed; `1` with an error (in `--json`, the error is in the `erro` field).

## Logs

- Output goes to standard error (stderr), so the `doctor` JSON stays valid on stdout.
- `poph serve` installs the subscriber at startup: without it the WhatsApp route and the shutdown loop leave no
  trace. The level comes from `RUST_LOG` when it is set, otherwise from `log.nivel`.
- Turn context: `contexto_turno(org, agente, conversa, turno)` in the `poph` crate. Available from HR-13 onward.
- Rule: no customer content and no secrets. Sensitive values use `Sensitive`, which prints as `[oculto]`.

## CLI conventions

- The administrative commands that work directly on the database (`org`, `token`, `category`, `department`, `kb`
  and `file`) require `--offline`: the server-backed mode does not exist yet.
- A business error exits with `1` and a message in Portuguese on stderr; the value of a token only appears when it
  is created. Wrong command-line usage exits with `2` and is decided by the parser.
- `--json` is a stable format with `versao_relatorio: 1` and never carries a secret.

## Sections documented elsewhere

The file has sections whose rules live with their subject, not here:

| Section | Rules in |
|---|---|
| `[turno]` (`intervalo_despacho_ms`, `tentativas_falha_modelo`, `atraso_falha_modelo_ms`) | [`TURNO.md`](TURNO.md) |
| `[handoff]` (automatic message and the risk guidance) | [`JEV.md`](JEV.md) |
| `[admin].tokens_globais` (label plus Argon2id hash; the clear value only exists in `poph token global` output) | [`API.md`](API.md) |
| `[cluster]` (`cluster.toml`: instances, org map, fingerprint) | [`CLUSTER.md`](CLUSTER.md) |

## Metrics registry

`poph-types` keeps the in-memory registry only (`Registry`, `Counter`, `Gauge`, `Histogram`, `BaseMetrics`);
`/metrics` renders it in the Prometheus text format. Two rules apply to every name: it must start with
`poph_` and use lowercase letters, digits and `_` only, and each name is registered **once** — registering
the same name again is an error. The catalog and what each metric means are in [`API.md`](API.md).

## Tests

```
cd apps/pop-harness && bash scripts/ci.sh
```

The `poph` tests prove precedence, errors with their origin and key, valid JSON, and that the secret sentinels do not
appear in stdout, stderr or in a `trace` log in JSON.

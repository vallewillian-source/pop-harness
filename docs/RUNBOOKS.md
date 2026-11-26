# POP Harness: operational runbooks

Step-by-step guides for day-to-day work on an installed app: starting and stopping, upgrading the version,
backing up and restoring an org, adding a plugin, calibrating a Jev gate, and operating a cluster of instances.

Each runbook says when to use it, what has to be ready, the commands, what to check afterwards, what to do if
it fails, and what not to do. When a step was not run in the verification of this page, it is marked
**not verified**, with the reason.

**Verification of this page:** 2026-11-21, on commit `035adbb6`, with the binary built that day. Everything ran
in a throwaway environment at `/tmp/hr58b-run`, with no LLM provider, no real WhatsApp and no embedding model.
The command `poph token global` was **not** run.

Section 6 (cluster and forwarding) was verified later the same day, on the same machine, with two real instances
running the binary from commit `eff9dd1f` (forwarding of HR-50b-2), in `/tmp/hr58b2-run-2`. Same rules: no
`poph token global`, no real WhatsApp, no LLM.

## Before you start

| What | Where it is |
|---|---|
| Program | `target/debug/poph` (built by `cargo build -p poph`) |
| Configuration | the `poph.toml` file given in `--config` (or in the `POPH_CONFIG` variable) |
| Databases | `harness.db` (data) and `rag.db` (search), inside the folder set in `[dados] diretorio` |
| Attachments and model | the `blobs/` and `modelos/` folders inside the data folder |
| Logs | there is no dedicated log file: the server writes to the terminal (standard error). To run in the background, redirect to a file (see runbook 1) |
| Secrets | only in environment variables, never in `poph.toml`: `POPH_CHAVE_CIFRA` and `ANTIGRAVITY_API_KEY` (required); `TYPESAFE_API_KEY` (only with Jev on); `EVOLUTION_API_KEY` (only with WhatsApp) |

Example `poph.toml` used in the verification (the default port is 7480; non-default values are not required):

```toml
[servidor]
endereco = "127.0.0.1"
porta = 18961

[dados]
diretorio = "/tmp/hr58b-run/a/dados"

[log]
nivel = "info"
```

General rules:

- The commands `--offline` and `migrate` write directly to the database. Run them with the server **stopped**
  (`docs/BACKUP.md` and `docs/CONFIGURACAO.md` require this; it follows the one-writer-at-a-time rule in
  `docs/DADOS.md`). I did not test these commands with the server running.
- Never put a secret in a file, in a log or in a response. When pasting a command, replace the `<...>` with the
  value stored outside the repository.

---

## 1. Start and stop

**When to use:** first start, after restarting the machine, or when putting an instance online.

**Prerequisites:**

- compiled binary (`cargo build -p poph`);
- `POPH_CHAVE_CIFRA` and `ANTIGRAVITY_API_KEY` set in the session environment;
- `poph.toml` written (see the example above).

### Commands

Check the environment and prepare the databases (server stopped):

```bash
cd pop-harness
export POPH_CHAVE_CIFRA='<cipher key, kept outside the repository>'
export ANTIGRAVITY_API_KEY='<LLM provider key>'
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml doctor
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml migrate
```

Start in the terminal (it stays attached; Ctrl-C stops it):

```bash
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml serve
```

Start in the background, with the log in a file:

```bash
cd pop-harness
setsid nohup ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml serve \
  > /tmp/hr58b-run/a/serve.log 2>&1 < /dev/null &
echo $! > /tmp/hr58b-run/a/serve.pid
```

### What to check afterwards

```bash
curl -s http://127.0.0.1:18961/healthz
```

Response from the verification run:

```json
{"status":"ok","version":"0.1.0","uptime_seconds":2,"databases":{"harness":2,"rag":0}}
```

- `status: ok` with `HTTP 200` = the server is responding.
- `databases` shows the version of each database.
- The log shows `servidor no ar endereco=127.0.0.1:18961` ("server up").
- Without the embedding model, the warning `busca na base desligada: modelo de embedding indisponível` ("search
  on the knowledge base is off: embedding model unavailable") appears. This is expected until you run
  `poph model install`, which needs internet (**not verified** here).
- Console: `http://127.0.0.1:18961/` returns `HTTP 200`. I did not open the screen in a browser in this verification.

### Stop without corrupting data

- In the terminal where it runs: **Ctrl-C**.
- In the background: `kill -INT $(cat /tmp/hr58b-run/a/serve.pid)`.

Confirmed in the verification: the process exited in about 1 s, the log showed `encerrando o servidor…`
("shutting down the server"), the port was released, and `doctor` run afterwards confirmed both databases intact.

Do not use `kill -9`: I did not test it, and it skips any clean shutdown. Use SIGINT.

### Production

There is no production installation of POP Harness in this repository or on this machine: **not verified**.
The procedure is the same: the same binary, with a different `poph.toml` (port, data folder) and the keys in the
environment of the real installation's process. Do not confuse it with `/opt/pop-atende`, which belongs to POP Atende.

### If it fails

- `doctor` shows `variável ... ausente` ("variable ... missing"): the key is not in the session environment. Set it
  and run again. `doctor` does not show the value.
- `doctor` shows `banco ... não é SQLite` ("database ... is not SQLite"): **do not run `migrate`**. The file at the
  database path is not a POP Harness database; check the data folder before anything else.
- Port in use: I did not test the error message. Check with `ss -ltn | grep <porta>` and change `porta`
  in `poph.toml`.

### What not to do

- Do not put a secret in `poph.toml`.
- Do not delete the data folder to "reset" an installation in use: it holds the conversations, the org
  configuration and the attachments.

---

## 2. Upgrade the version

**When to use:** replacing the binary with a new version of the code.

**Prerequisites:** access to the data folder and to the current binary; the server must be stopped.

### Safe order

1. **Stop the server** (see runbook 1).
2. **Keep the current binary and a copy of the data folder**, with the server stopped:

   ```bash
   cd pop-harness
   cp target/debug/poph /tmp/poph-versao-anterior
   cp -a /tmp/hr58b-run/a/dados /tmp/hr58b-run/a/dados.antes-da-atualizacao
   ```

   Replace `/tmp/hr58b-run/a/dados` with the real data folder of the installation.

3. **Bring in the new version of the code** through the team's git workflow.
4. **Compile:**

   ```bash
   cd pop-harness
   cargo build -p poph
   ```

   In the verification, with the cache already in place, it took about 10 s and rebuilt `poph-server` and `poph`.
   From scratch it takes longer.

5. **Migrate the databases** (server stopped):

   ```bash
   ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml migrate
   ```

   Running it again does nothing. In the verification, the harness database went from version 0 to 2 the first
   time, and the second run answered `já estava na versão 2` ("was already at version 2"). Exit code `0` = databases up to date.

6. **Check:** `./target/debug/poph --config /tmp/hr58b-run/a/poph.toml doctor` with no errors.
7. **Start** again (runbook 1) and check `/healthz`.

### If the start fails

1. Stop the new version (Ctrl-C or SIGINT).
2. Put the binary back: `cp /tmp/poph-versao-anterior target/debug/poph`.
3. If the migration already ran, restore the data folder. With the server stopped, move the current one aside and
   restore the copy (do not delete anything):

   ```bash
   mv /tmp/hr58b-run/a/dados /tmp/hr58b-run/a/dados.depois-da-falha
   mv /tmp/hr58b-run/a/dados.antes-da-atualizacao /tmp/hr58b-run/a/dados
   ```

4. Start the previous version and check `/healthz`.

**Not verified:** whether the old binary opens a database that the new version has already migrated. I did not test
it. That is why the safe way back is the copy from step 2. Migrations only move forward (`docs/DADOS.md`: a published
migration is not edited), and there is no command to undo a migration.

### What not to do

- Do not migrate with the server running.
- Do not trust a copy of the data folder made while the server was running: it may be half-written.
- Do not skip the copy: the way back depends on it, and writes made after the copy are lost when you go back.

---

## 3. Back up and restore an org

**When to use:** before changing a large org, to move an org to another installation, or to rebuild an org on a
new installation.

### What the file contains

| Always included | Included only with `--com-conversas` | Never included |
|---|---|---|
| Org configuration, products, categories, departments | Conversations, messages and attachments (with the bytes) | Administration tokens (they are created again) |
| Agents and instructions | Turns (the "why" behind each reply) | Audit trail (`audit_log`) |
| Knowledge base | Both queues (inbound and outbound) | Integration secret, not even encrypted (only the mark that it existed goes) |
| File library (with the bytes) | Quote carts | Internal path of the files |
| Integrations (without the secret) | | |

Format: JSON with `formato: "poph-backup-org"` and `versao_formato: 1`. A version the installation does not know is
refused.

### Export with the CLI (server stopped)

```bash
cd pop-harness
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml --offline backup export clinica-prova --out /tmp/hr58b-run/backup-clinica.json
```

- `clinica-prova` is the org's slug or id.
- `--com-conversas` includes the history. Without it, the file has no conversations.
- It does not overwrite an existing file. To replace one, use `--sobrescrever`.
- `--json` gives stable output for scripts.

Output from the verification:

```
Org exportada para /tmp/hr58b-run/backup-clinica.json (2 KB).
  agents: 1
Blobs no arquivo: 0 (um por sha256; o caminho é recalculado na importação).
Conversas não entraram (use --com-conversas para levá-las).
```

(The program prints these messages in Portuguese. In order: "Org exported to ... (2 KB)"; "Blobs in the file: 0 (one per sha256; the path is recalculated on import)"; "Conversations were not included (use --com-conversas to include them)".)

Check that nothing unwanted came out:

```bash
grep -c blob_path /tmp/hr58b-run/backup-clinica.json
grep -c secrets_enc /tmp/hr58b-run/backup-clinica.json
```

Expected: `0` and `0`. In the verification both returned `0` for both files (with and without conversations).

### Restore with the CLI (clean installation)

```bash
cd pop-harness
./target/debug/poph --config /tmp/hr58b-run/b/poph.toml --offline backup import /tmp/hr58b-run/backup-clinica.json
```

- If the installation does not have the org, the import creates it with the same id and name.
- With `--org <slug-ou-id>` the import writes into an org that already exists.
- Running it again with the same file, into an org that already has those rows, is **refused** without
  `--sobrescrever`: the output says how many rows would be updated, in which tables, with up to 5 examples, and nothing
  is written. With `--sobrescrever`, the import goes ahead and the report shows `inseridas` (inserted) and
  `atualizadas` (updated).

Output from the verification on the clean installation: `Org ... criada.` ("org created"), `agents: 1 nova(s)`
("1 new"), and the warning that the index is pending (without the embedding model, the knowledge base and the
products have no index until you run the commands below).

### Through the API (same as the CLI)

- Export: `GET /admin/v1/orgs/{org}/backup` (without conversations) or `GET /admin/v1/orgs/{org}/backup?com_conversas=true`.
  Response `200`, `content-type: application/json`, `content-disposition: attachment` with a name like
  `clinica-prova-backup-20261121-1639.json`, and `x-content-type-options: nosniff`.
- Import: `POST /admin/v1/orgs/{org}/backup/import`, with the **text of the file** as the body, in
  `Content-Type: application/json`. It is not multipart. Limit of 64 MB; above that the response is `413`.
- Without `?sobrescrever=true`, if the import would update rows that already exist in the org, the response is `409`
  with `code: "importacao_sobrescreve"`, `details.tabelas` (table → rows), `details.linhas` and
  `details.exemplos` (up to 5). Nothing is written. With `?sobrescrever=true`, the import goes ahead.

Example:

```bash
TOKEN=$(cat <file where you stored the admin token>)
curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  --data-binary @/tmp/hr58b-run/backup-clinica.json \
  http://127.0.0.1:18961/admin/v1/orgs/<id-da-org>/backup/import
```

In the refusal test: importing the file into the same org without authorization returned `409` with
`importacao_sobrescreve` and nothing changed; with `?sobrescrever=true`, it returned `200` and the agent went back to
the name in the file. Importing the same file into another org on the same installation returned `409` with the list
of the conflict and the sentence saying nothing was written. The full error codes are in `docs/API.md`.

### What to check afterwards

- List the orgs and agents on the target installation and compare them with the source.
- If the report says the index is pending, install the model (`poph model install`, needs internet) and run, with the
  server stopped:
  `./target/debug/poph --config /tmp/hr58b-run/b/poph.toml --org clinica-prova --offline kb reindex` and
  `./target/debug/poph --config /tmp/hr58b-run/b/poph.toml --org clinica-prova --offline product reindex`
  (change the `poph.toml` path and the slug to match your installation).
  In the verification, without the model, `kb reindex` only warned that the model was not installed. Reindexing
  with the model present is **not verified** here.
- Integrations come back without the secret. Type the secret again in the Integrations tab of the Console. **Not
  verified** in this run; the rule is covered by the repository's tests.

### If it fails

- Cut-off file or one that is not JSON: the import refuses with "não é um backup do POP Harness" ("not a POP Harness
  backup"). Nothing is written (confirmed). Copy the file again.
- Unknown format version: the message says which version the installation understands (confirmed with version `99`).
  Use an installation with the version that exported the file.
- Conflict: "o id do arquivo já existe nesta instância em outra org" ("the file's id already exists on this instance
  in another org"). Nothing is written (confirmed).

### What not to do

- **Do not authorize the overwrite (`--sobrescrever` or `?sobrescrever=true`) to undo a change.** Without the
  authorization, the import is refused and shows what would be updated. With it, the rows go back to the file's content:
  in the verification, an agent that was on version 2 (with the Jev configuration changed) went back to version 1
  and lost the change made after the file was created. Read the refusal list before authorizing. To go back in time,
  restore onto a clean installation.
- **Do not import the same file into two orgs on the same installation.** It returns `409`, because the ids in the
  file already exist. Restoring onto a clean installation works. This is a known limit, already recorded in the
  backup deliverables.
- Do not export or import with the server running (see the general rules).
- Treat the file as customer data: with `--com-conversas` it carries messages and attachments.

---

## 4. Add a plugin

**When to use:** connecting an integration that runs in an external process (a separate program the server starts
and talks to).

**How it works:** the server starts the program and sends it the first line, the *handshake*, with the protocol, the
org, the type (`kind`), the configuration and the secrets. The program answers with the manifest (name, version,
what it accepts and the configuration format). If the program does not answer, the type shows up as
*unavailable*; the server does not go down. The protocol is described in `docs/PLUGIN_PROTOCOLO.md`.

**Prerequisites:**

- the plugin program compiled, with an absolute path. To test, the example plugin:
  `cargo build -q -p poph-plugin-eco --bins` (this is what `scripts/plugin-echo.sh` does). The binary goes to
  `target/debug/plugin-eco`.

### Step 1: declare it in `poph.toml`

```toml
[[plugins.integracoes]]
kind = "eco"
programa = "target/debug/plugin-eco"
argumentos = []
```

- `kind` is unique and cannot be empty.
- `programa` is a local path on the server. It never comes from a request.
- `argumentos` must not carry a secret: they are visible in the process list (`ps`). Secrets arrive only through the
  handshake.

### Step 2: restart the server

The manifest is read on the first query and kept in memory (see "Limits" below). Restart the server (runbook 1).

### Step 3: check

Replace `<id-da-org>` with the org's id (it appears in `poph --config <seu poph.toml> --offline org list`, with the
server stopped) and use the admin token in `Authorization`:

```bash
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:18961/admin/v1/orgs/<id-da-org>/manifests
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:18961/admin/v1/orgs/<id-da-org>/manifests/eco
```

Output from the verification, with the example plugin and a second type deliberately broken (`/bin/false`):

```json
{"items":[{"kind":"eco","nome":"Eco (exemplo)","situacao":"ok","versao":"0.1.0"},
          {"kind":"quebrado","mensagem":"o plugin deste tipo não respondeu ao handshake; verifique o programa declarado em `[plugins].integracoes`","situacao":"indisponivel"}]}
```

(The message in the output says, in Portuguese: "the plugin of this type did not answer the handshake; check the program declared in `[plugins].integracoes`".)

- `situacao: ok` = manifest read, with name and version.
- `situacao: indisponivel` = the program did not answer. For the detail (`.../manifests/quebrado`) the response is `503`
  with `manifesto_indisponivel`.
- No token: `401`.
- The Operation screen (instance health) shows the same list. **Not verified** in this run, because the Console was not
  opened in the browser.

### Limits

- **Changing a plugin only takes effect after restarting the server.** The manifest stays cached until the process ends.
  This behavior is written in the comment in `crates/poph-server/src/manifestos.rs`. I did not repeat the test in this run.
- **A failure stays cached for a while.** A fixed plugin remains unavailable for up to 30 s (figure from the plugin
  list delivery; not re-measured here).
- **Removing a plugin:** delete the block and restart. **Not verified.**

### If it fails

- The server log shows `não consegui ler o manifesto do plugin kind=... programa=... erro=handshake recusado`
  ("could not read the plugin manifest ... handshake refused"). The cause is usually one of: the program path does not
  exist, the file is not executable, or the program closes before answering.
- Different protocol: the server refuses an incompatible version (`docs/PLUGIN_PROTOCOLO.md`).

### What not to do

- Do not put a secret in `argumentos`.
- Do not point to a program that comes from outside (from a request, from a file sent by a customer).
  The path comes only from the configuration.
- Do not count on swapping a plugin without restarting.

---

## 5. Calibrate a Jev gate

**When to use:** deciding whether a gate is on by default or only on one agent, or measuring the effect of a change
before touching the default.

**What Jev is:** the service that decides, by choosing among known options, things like handing off to a human, waiting
longer or processing now. It is separate from the text model. The full details are in `docs/JEV.md`.

### The gates

| Gate | Question it answers | On by default |
|---|---|---|
| G1 | hand off to a human now? | no |
| G2 | wait longer or process now? | no |
| G3 | needs knowledge base, product or nothing? | **yes** |
| G4 | which department receives the handoff? | no |
| G5 | which product does the customer want? | **yes** |
| G6 | can the reply be sent? | no |
| G7 | is it prompt injection or abuse? | no |
| G8 | did the customer end the conversation? | **yes** |

The default is in code, in the `padrao()` function in `crates/poph-types/src/config.rs`: G3, G5 and G8 on, the others
off (confirmed by reading the code). Changing the default is a code change and goes through review; it is not just a
configuration edit.

The global switch is `[jev] ativo`, which comes off (`docs/CONFIGURACAO.md`: "turns Jev on"). The variable
`POPH_JEV_ATIVO` overrides the file. With the global switch off, Jev is not consulted, even with the gates on. This
effect was not tested in this run; the reading comes from the documentation.

### Turn on and off

**For the whole installation** (`poph.toml`):

```toml
[jev]
ativo = true
url_base = "https://api.typesafe.ai"

[jev.gates]
g1 = true
```

With Jev on, set `TYPESAFE_API_KEY` in the environment. **Not verified** with the real Jev: the verification did not
have the key.

**On one agent only** (it is the only per-agent setting):

```bash
cd pop-harness
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml --org clinica-prova agent show atendente-prova --json
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml --org clinica-prova agent update atendente-prova \
  --if-match 1 --offline --jev-json '{"ativo": true, "gates": {"g1": true, "g4": false}}'
```

- `--if-match` is the version read in `agent show` (the `version` field). If it does not match, the change is refused
  with `conflito de versão ... esperada 2, atual 1` ("version conflict ... expected 2, current 1") (confirmed).
- `{"ativo": false}` turns off all gates for that agent. Whatever the agent does not declare inherits from the global setting.
- Returning to `{}` inherits everything from the global setting. **Not tested** going back.

**There is no per-org setting.** What you switch is the whole installation or each agent. To change a whole org, adjust
each of its agents.

### Measure before changing the default

The labeled sets are in `tests/jev/g1.jsonl` through `tests/jev/g8.jsonl`. With the real Jev:

```bash
cd pop-harness
TYPESAFE_API_KEY='<chave do Jev>' ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml jev eval g1 tests/jev/g1.jsonl
TYPESAFE_API_KEY='<chave do Jev>' ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml jev eval g1 tests/jev/g1.jsonl --json > /tmp/g1-real.json
```

(`<chave do Jev>` = the Jev key, to be replaced with the value stored outside the repository.)

- Read the hits, abstentions and errors. The most important is the **confident wrong decision** (`docs/JEV.md`
  explains how to read it and gives the measurements of 2026-11-20 for each gate).
- When changing the default, measure with the real Jev first.

**Beware of the fake Jev.** `scripts/fake-jev.py` is only for testing the plumbing, not the quality.
In the verification, I ran `jev eval g1` against it: 135 cases, 113 hits and 22 errors, 0 call failures. These numbers
**do not** support any decision. With `g5`, the fake answered no questions (it only covers some gates), and the output
showed abstention in every case. That is a limit of the fake, not of the command.

**Not verified in this run:** measurement with the real Jev (no key) and the effect of the gate inside a turn (requires
the LLM provider).

### What not to do

- Do not turn on a gate by default without measuring with the real Jev.
- Do not use the fake Jev to make decisions.
- Do not expect per-org settings: it is per installation or per agent.

---

## 6. Cluster and forwarding between instances

**When to use:** when there is more than one POP Harness instance, each with some organizations. Also when an org was
served by the wrong instance, when the map of who lives where needs to change, or when a request comes back with `421`
or `503`.

**How it works:** all instances read the same map file (`cluster.toml`), and each organization has a single owner. If a
request arrives at an instance that is not the owner, that instance forwards it to the owner, signed, and the owner
answers as if it had received it directly. The file format, the rules and the list of error codes are in
`docs/CLUSTER.md` and `docs/API.md`. Only the operation is covered here.

**Prerequisites:**

- a `cluster.toml` on each instance, with the same content, except for each one's `instance_id`. It sits next to
  `poph.toml` (or at the path given in `POPH_CLUSTER`);
- the cluster secret (`POPH_CLUSTER_SECRET`) set in the environment of **all** instances, with the same value, before
  starting. It never goes in the file;
- the same global token on all instances (`[admin] tokens_globais`), so that reloading and the consolidated list work
  on any of them;
- the instances can reach each other through the URLs in the map.

### Step 1: check the map before starting

```bash
export POPH_CLUSTER_SECRET='<cluster secret, kept outside the repository>'
BIN=target/debug/poph
$BIN --config <poph.toml-desta-instancia> cluster check
$BIN --config <poph.toml-desta-instancia> cluster show
```

Replace `<poph.toml-desta-instancia>` with the absolute path of the `poph.toml` of the instance you are on.
The addresses `127.0.0.1:18901` and `:18902` in the examples are the ones used in this section's verification; use
your installation's.

Real output of `cluster check` with a valid map:

```text
Mapa do cluster válido: /tmp/hr58b2-run-2/a/cluster.toml
  2 instância(s)
  fingerprint: a222897d055e72f43f624231b84a35c9
  segredo compartilhado: definido (POPH_CLUSTER_SECRET)
```

(Program output, in Portuguese: "valid cluster map", "2 instance(s)", "shared secret: set (POPH_CLUSTER_SECRET)".)

What to check in `cluster show`: the "local instance" is this machine, and each org appears on the line of the instance
that should own it. Without `cluster.toml`, `show` says `modo: instância única` ("mode: single instance"), which is
expected on an installation without a cluster.

Each instance shows two fingerprints. The "fingerprint" includes the local instance, so it differs on each one; to
compare the map between instances, use the "map fingerprint (without the local instance)", or `cluster check --remoto`
from step 3. This is the same fingerprint that appears in `/healthz` as `cluster_map_fingerprint`.

If `check` rejects it, it lists each problem by key in the file. Real example: `orgs.<id>: a instância "c" não existe em
instances` ("instance 'c' does not exist in instances"). Fix it and run again; it exits with code 1 while there is a problem.

### Step 2: change the map without restarting

1. Edit the instance's `cluster.toml`.
2. Reload that instance (global token; an org token does not work):

```bash
curl -s -X POST -H "Authorization: Bearer <token-global>" http://127.0.0.1:18901/admin/v1/cluster/reload
```

3. Do the same on the other instance.

The response includes `mudou` (changed), `fingerprint_antes` (before), `fingerprint_depois` (after), `orgs_que_trocaram`
(orgs that switched) and the instances added or removed. While the two instances do not have the same map, a request
between them is refused with `421 mapa_divergente` (divergent map); no request is processed with the wrong map.

**Reloading does not move data.** If an org goes from instance `b` to `a`, the data stays on `b` until you export and
import it (runbook 3). Until then, the new owner does not have the data, and the org appears empty or missing to whoever
calls. This specific case was not tested.

**Bad cases, shown in this verification:**

- **File with a typo:** the reload responds `400` with `cluster_invalido` and the list of problems. The map in use stays
  the previous one. Check with `curl -s http://127.0.0.1:18901/healthz`: the `cluster_map_fingerprint` field does not
  change. Fix the file and reload.
- **File deleted** (or `POPH_CLUSTER` pointing to a wrong path): the reload responds `200` with `modo_depois:
  instancia_unica` (mode after: single instance), and the instance starts treating **all** organizations as its own.
  This is dangerous, because it may answer for an org that belongs to another instance. Restore the file and reload; the
  mode goes back to `cluster` and the map fingerprint is the same as before.

### Step 3: check the instances against each other

```bash
$BIN --config <poph.toml-desta-instancia> cluster check --remoto
```

The command queries the `/healthz` of each other instance (3 s timeout each, in parallel). With both online and the
same map, the output is this, and the exit code is 0:

```text
consulta remota (mapa local: 9a6b810b853d4649e51e27f2c5056959):
  b (http://127.0.0.1:18902): ok (mapa 9a6b810b853d4649e51e27f2c5056959)
  todas as instâncias respondem com o mesmo mapa
```

(Program output, in Portuguese: "remote query (local map: ...)"; "all instances answer with the same map".)

The three possible problems, with exit code 1:

- **`inacessível`** (unreachable): the other instance did not answer (down, wrong port or URL). Shown with `b` stopped:
  `inacessível — error sending request for url (…/healthz)`.
- **`mapa divergente — ela usa <impressão>`** (divergent map — it uses <fingerprint>): the other one's map is different.
  Shown by editing and reloading only `a`.
- **`identidade divergente`** (divergent identity): the other one declares itself with a different `instance_id`.
  Not verified in this run.

Without `--remoto`, the command only validates the local file. With `--json`, the result comes in a format for scripts.
The command does not check the URL of the instance itself.

### Step 4: see the organizations of all instances

```bash
curl -s -H "Authorization: Bearer <token-global>" "http://127.0.0.1:18901/admin/v1/orgs?cluster=true"
```

Each organization includes `instancia`, and the `instancias` field includes `situacao` per instance: `local`, `ok` or
`inacessivel`. When an instance is down, it shows as `inacessivel` with `orgs: 0` and a `mensagem`, and the list returns
`200` with the organizations from the others. Each neighboring instance has a 2 s timeout. Without the parameter, the
response shows only this instance's organizations. With an org token, the parameter is ignored.

Shown with `b` stopped: the list came back with `200` in about 0.4 s, with `b` marked as `inacessivel`.

### Step 5: diagnose a forwarding error

| Response | What it means | What to do |
|---|---|---|
| `421 mapa_divergente` | the map of the instance that forwarded is different from this instance's map. `details` includes `fingerprint_local` and `fingerprint_recebido` | run `cluster check --remoto` on the instance that answered; align the `cluster.toml` of both and reload both |
| `421 rota_em_laco` with `details.instancia_dona` | the organization does not live on this instance, and the request had already been forwarded | this instance's map is wrong: check `cluster show` here and fix the file |
| `421 rota_em_laco` with `details.hop` | a request that was already forwarded arrived asking for one more forward | sign of a mismatched map; do as above |
| `503 instancia_fora` | the owner did not answer (connection refused or timeout). `details` includes `instancia` and `url` | bring the owner up (runbook 1) and check its `/healthz`. There is no automatic retry: the caller has to repeat the request |
| `401 assinatura_invalida` | signature does not match, timestamp outside the 60 s window, instance not declared in the map, or repeated nonce | check that `POPH_CLUSTER_SECRET` is the same on both and restart if it changed; check that the clocks of both machines are right |

Shown in this verification: `421 mapa_divergente`, `421 rota_em_laco` (with `instancia_dona` and with `hop`), `401` for
an old timestamp, for a tampered signature and for an unknown instance, `503 instancia_fora`. A difference in secret
between instances was **not** tested.

**Procedure: an organization was served by the wrong instance**

1. On the instance that answered, run `cluster show`: does the organization appear on the line of the instance that should
   own it?
2. Run `cluster check --remoto`: does the other instance have the same map?
3. If not, fix the `cluster.toml` of the wrong instance, reload that instance, and then the other.
4. If the response was `503`, bring the owner instance up.
5. Check from the instance that answered: the organization should come back the same as from the owner.

### What does NOT exist yet

- Moving an organization between instances automatically. Today it is export and import by hand (runbook 3).
- Automatic retry or switching instance when the owner fails in the middle of a request.
- The Console screen using `?cluster=true`. The operation screen does not show the instances yet.
- Forwarding with the global token. The original `Authorization` travels along, and the owner is the one who validates it.

### What not to do

- Do not edit one instance's map and expect the others to follow. Reload both.
- Do not delete `cluster.toml` to "undo" a change. The instance becomes a single instance and starts answering for all
  organizations.
- Do not change `POPH_CLUSTER_SECRET` with the server running. The change only applies after restarting, and until then
  the instances do not sign or verify with the same secret.
- Do not move an organization between instances without moving its data first.
- Do not put the secret in `cluster.toml`, in `poph.toml`, in a log or in a message.

### Not verified in this run

- Instances on different machines and with HTTPS: the verification used two HTTP ports on the same machine.
- `identidade divergente` in `cluster check --remoto`.
- Reload with an org token (should return `403`): not tested in this run. The `401` without a token was tested.
- Different secret between instances, and a clock outside the 60 s window: not tested.
- Forwarded live chat (SSE): covered by the automated test of HR-50b-2, not repeated here.
- Real WhatsApp messages arriving at the wrong instance: not verified, because there is no Evolution in this run.
- Moving an organization with data between instances: not tested.

---

## 7. Instalar um fluxo

Quando usar: a organização já tem um fluxo escrito (`.fluxo`) e você quer deixá-lo valendo para um
agente começar atendimentos por ele. Publicar cria uma **revisão numerada imutável**; o rascunho pode
continuar sendo editado sem mexer em quem já está atendendo na revisão publicada.

O que precisa estar pronto: instalação migrada, org e agente criados, e o arquivo do fluxo em mãos. Nada
disso precisa do servidor no ar.

**Verificação desta página:** 23/11/2026, com o binário de `cargo build -p poph` no commit `6ececcf`,
numa pasta descartável em `/tmp/fx31/runbook2`, sem LLM e sem WhatsApp. Os dados são fictícios
(org `clinica-norte`, agente `atendente`, fluxo `agendar_visita`).

### Passo a passo

```bash
BIN=target/debug/poph
export POPH_DADOS_DIRETORIO=/tmp/fx31/runbook2/dados   # só para a prova; em produção é a pasta de dados real

# 1. Validar antes de publicar: sai com 0 quando compila e 1 quando tem erro.
$BIN flow validar agendar.fluxo

# 2. Publicar. Na primeira vez, cria o fluxo e publica a revisão 1.
$BIN --org clinica-norte flow publicar agendar.fluxo

# 3. Conferir que está lá e qual revisão vale.
$BIN --org clinica-norte flow listar
$BIN --org clinica-norte flow mostrar agendar_visita

# 4. Liberar o fluxo ao agente — o agente só usa fluxo liberado a ele.
$BIN --org clinica-norte flow liberar --fluxo agendar_visita atendente

# 5. Conferir as Fichas que já existem (nenhuma, numa instalação limpa).
$BIN --org clinica-norte ficha listar
```

Saída observada na prova (recortada):

```
$ poph flow validar agendar.fluxo
agendar.fluxo: script válido.

$ poph --org clinica-norte flow publicar agendar.fluxo
fluxo `agendar_visita` criado e publicado como revisão 1.

$ poph --org clinica-norte flow listar
01a209c9-d1af-7223-b927-a08c8ab58a04  active    agendar_visita   Agendar visita  (revisão 1, versão 2)

$ poph --org clinica-norte flow liberar --fluxo agendar_visita atendente
fluxo `agendar_visita` liberado ao agente `Atendente`.
```

### Publicar uma revisão nova

O fluxo já existe, então a publicação exige a versão atual (o `--if-match`), que vem do `flow mostrar`:

```bash
$BIN --org clinica-norte flow mostrar agendar_visita --json   # veja `version`
$BIN --org clinica-norte flow publicar agendar-v2.fluxo --if-match 2
$BIN --org clinica-norte flow diff agendar_visita --de 1 --para 2
```

- Sem `--if-match`, a CLI recusa e explica: *o fluxo já existe: informe --if-match com a versão atual*.
- Versão velha responde conflito (*a versão informada não confere*) e **nada é gravado**. É a mesma
  proteção contra duas pessoas publicando ao mesmo tempo.
- A revisão 1 continua idêntica depois da 2: o que muda é a revisão que o fluxo passa a usar.
- O `diff` mostra o que mudou e **quantos atendimentos estão presos em cada revisão**, para você não
  publicar sem saber quem fica onde.

### Formatar o script

```bash
$BIN flow formatar agendar.fluxo             # mostra a forma canônica na tela
$BIN flow formatar agendar.fluxo --escrever  # grava no próprio arquivo
```

Formatar é idempotente: formatar a saída de novo não muda nada. Script que não compila sai com 1 e o
arquivo não é tocado.

### O que conferir depois

- `flow listar` mostra o fluxo com a revisão certa.
- `flow mostrar <slug>` mostra o rascunho e a revisão que vale hoje.
- `agent show <agente>` lista os fluxos liberados a ele.
- `ficha listar` começa a trazer Fichas quando o agente iniciar um atendimento por esse fluxo.

### Se falhar

| Mensagem | Causa | O que fazer |
|---|---|---|
| `o script não passa na validação: N erro(s)` | o script não compila | `flow validar` mostra linha e coluna; nada foi gravado |
| `informe --if-match com a versão atual` | o fluxo já existe | `flow mostrar` e repetir com a versão |
| `a versão informada não confere` | outra publicação entrou antes | ler de novo e repetir; a sua tentativa não gravou nada |
| `não existe fluxo ... nesta org` | slug/id errado ou de outra org | `flow listar` |
| `agente ... não encontrado` | slug do agente errado | `agent list` |

### O que não fazer

- Não editar o banco à mão para "corrigir" uma revisão: revisão publicada é imutável por gatilho, e a
  tentativa de `UPDATE` falha. O caminho é publicar outra revisão.
- Não usar `--if-match` de uma leitura antiga depois de outra pessoa publicar.
- Não liberar o fluxo ao agente antes de conferir que a revisão publicada é a que você quer.

---

## 8. When a flow does not start

Check in this order. Each step is a command or a field in the turn record; none needs the server stopped.

1. **Is the engine on?** `poph doctor` shows whether the flow engine runs. With `[fluxos] ativo = false`, the G9 gate,
   the flow block in the prompt, the flow tools and the effects are all off. Conversations and Cases keep working.
2. **Is the flow published and granted to this agent?** `poph --org <org> flow mostrar <slug>` shows the revision in
   use. `poph --org <org> agent show <agente>` lists the flows granted to the agent. A flow that is not granted is
   never offered to the Jev, so it never starts.
3. **Is the Case open and without an active execution?** `poph --org <org> ficha listar --agente <agente>` shows the
   Cases and their executions. A closed Case does not start a flow; the customer's next message opens a new Case. An
   active execution blocks a new one: finish it or abandon it (`POST .../flow-runs/{run_id}/abandon`).
4. **What did the Jev decide?** The `turns.jev_json` record of the turn has the G9 result (`gate: "g9"`, `motivo`,
   `confianca`, `margem`). `nenhum`, low confidence or a narrow margin means nothing starts, by design. `fallback`
   means the Jev failed and the model received the `fluxo_iniciar` tool. Two flows with similar triggers can tie: make
   the `quando` and `nao_quando` texts of each flow more distinct.
5. **Is the agent within the limit?** The G9 does not ask when the agent has more than 20 flows granted; the API
   refuses the 21st.
6. **Metrics.** `poph_fluxos_g9_nenhum_total`, `poph_fluxos_g9_abaixo_do_limiar_total` and
   `poph_fluxos_g9_fallback_total` are incremented by the turn. `poph_fluxos_execucoes_iniciadas_total`,
   `poph_fluxos_execucoes_concluidas_total`, `poph_fluxos_execucoes_abandonadas_total`,
   `poph_fluxos_execucoes_falhas_total` and `poph_fluxos_erros_regra_total` are incremented by the store, in the
   instance that runs `poph serve` (`docs/FLUXOS.md` §5.4).

---

## 9. Build the ARM64 binary (HR-56)

Use this on the native ARM64 test machine (HR-56). There is no cross-compilation: the workstation has no cross C
compiler (D-02), so the build has to run on the same architecture as the host.

### What has to be ready

- Docker on the ARM64 machine, used without sudo.
- The Rust toolchain on the host, mounted into the container at `/w/rustup` and `/w/cargo`.
- The current directory mounted as `/w`, with the repository at `src/apps/pop-harness` inside it (the path the
  `docker run` below expects).

### Commands

```
docker build -t poph-build-arm:24.04 -f scripts/Dockerfile.build-arm scripts/
docker run --rm -u $(id -u):$(id -g) -v "$PWD:/w" -w /w/src/apps/pop-harness \
  -e RUSTUP_HOME=/w/rustup -e CARGO_HOME=/w/cargo -e PATH=/w/cargo/bin:/usr/local/bin:/usr/bin:/bin \
  poph-build-arm:24.04 bash scripts/build-dist.sh /w/dist/poph-arm
```

### Why the base image is Ubuntu 24.04

- **glibc:** the test host is Ubuntu 24.04 with glibc 2.39. The image must not have a newer glibc than the
  host, or the binary will not run there. Ubuntu 24.04 gives 2.39, the same as the host.
- **libstdc++:** the ONNX Runtime binary (crate `ort`, downloaded, linked statically) was built with the
  libstdc++ of GCC 13 or newer. Debian 12 (GCC 12) fails at link time with the undefined symbols
  `_M_replace_cold` and `__cxa_call_terminate`.

### What to check afterwards

- The build log shows `gcc` 13 or 14 and `ldd` with glibc 2.39. The `RUN` line of the image prints both versions.
- `scripts/build-dist.sh` finishes and writes the binary to `/w/dist/poph-arm`.

### If it fails

- Link error with `_M_replace_cold` or `__cxa_call_terminate`: the base image is older than GCC 13 (Debian 12).
  Go back to `ubuntu:24.04`.

### What not to do

- Do not cross-compile from x86_64: the workstation has no cross C compiler (D-02).
- Do not change the base image without repeating both checks above.

**Not verified in this run:** no ARM64 machine was available in the session that wrote this section, so the
image was not built and the commands above were not run. The rules come from the image's own comments, which
were moved here when they were removed from the Dockerfile.

## Verification log

| Runbook | Run in this verification | Not verified (reason) |
|---|---|---|
| 1. Start and stop | `doctor` (without and with the test keys), `migrate` (twice), `serve` in the background, `/healthz`, `HTTP 200` on the Console, stop with SIGINT, `doctor` afterwards | production (there is none); port in use (not tested); `kill -9` (not tested) |
| 2. Upgrade | `cargo build -p poph`, `migrate` (twice, with the return to "nothing to do") | a real new version (the code was the same); going back with an already migrated database (not tested) |
| 3. Backup | `backup export` with and without conversations, export without `--sobrescrever` (refused), `backup import` into a clean installation and again (idempotent), refusal of a cut-off file and of version 99, conflict with another org; HTTP: `GET backup` (200, headers), `POST import` (200 and 409) | integration secret (covered by the repository's tests); reindexing with a model (no model in the verification); 413; 401 and 403 on backup; an org with conversations (the verification org had none) |
| 4. Plugin | declare, start, list and detail (`ok`, `indisponivel`, 503), `401` without a token | Operation screen in the browser; swapping a plugin (only from the code text); removal |
| 5. Jev gates | `agent show`, `agent update` with `--if-match` (success and conflict), `jev eval g1` and `g5` with the fake Jev | real Jev; effect inside a turn; going back to `{}`; quality measurement |
| 6. Cluster and forwarding | `cluster show`, `cluster check` (valid and with an error), `cluster check --remoto` (healthy, instance stopped, divergent map), reload with a valid, invalid and deleted file, `?cluster=true` with and without the parameter, forwarding of an org from another instance (200), `421 mapa_divergente`, `421 rota_em_laco`, `401` for timestamp, signature and unknown instance, `503 instancia_fora` | reload with an org token (403); divergent identity; different secret; HTTPS and separate machines; real SSE and WhatsApp; moving an org with data |

| 9. ARM64 build | nothing (no ARM64 machine in the session) | everything: image build, `build-dist.sh`, the glibc and GCC checks |

Count: about 65 calls between CLI, HTTP and build across runbooks 1 to 5; about 60 commands and HTTP calls in runbook 6.
I did not run `poph token global`.

Some of the commands ran with the binary from 13:29 and some after a `cargo build -p poph` that rebuilt `poph-server`
and `poph`. The HTTP check of the backup and the agent calls were done with the rebuilt binary.

# External contract baseline

This directory holds the external contracts of the harness captured on `main` before the
English/no-comments refactor. Every task of the refactor is compared against it: the contracts
below must stay identical byte for byte, even when the internal code is renamed or cleaned.

The baseline is generated, never edited by hand. `LEIAME.md` is the only hand-written file and it
is preserved between captures.

## Recapture

```bash
bash scripts/contracts.sh snapshot
```

The command rebuilds the `poph` binary, captures everything into a temporary directory and only
then replaces `base/`. It needs no network and no client data.

## Compare

```bash
bash scripts/contracts.sh compare
```

The command captures the current tree the same way and compares it with `base/`. It prints
`0 diferenças` and exits `0` when nothing changed; otherwise it prints the differences and exits
`1`. A different description in the OpenAPI document, a renamed CLI flag or a changed database
column all fail the comparison.

Both commands run `cargo build --locked -p poph` and the ignored test
`crates/poph-server/tests/contracts_baseline.rs`, so they need the workspace toolchain. Running
the test directly also works:

```bash
POPH_CONTRACT_SNAPSHOT_DIR=/tmp/contratos \
  cargo test --locked -p poph-server --test contracts_baseline -- --ignored
```

## What is captured

| Path | Contract | How it is produced |
| --- | --- | --- |
| `openapi/openapi.json` | HTTP API document | `poph openapi`, re-serialized with sorted keys and two-space indent |
| `cli/*.txt` | Every `--help`, byte for byte, one file per command and subcommand; `poph__version.txt` is `poph --version` | the whole command tree is walked from the root help output |
| `db/harness-schema.txt` | Schema of a database migrated from zero | `sqlite_master` rows (`type`, `name`, `tbl_name`, `sql`) sorted by type and name |
| `db/harness-migrations.txt` | Applied migration versions and names | `schema_migrations`, without the timestamp column |
| `db/harness-schema-upgraded.txt` | Schema of an old database after upgrading | a version 1 database is built with the runner bootstrap plus the `0001` migration file embedded in the binary, then the remaining migrations are applied by the real runner in `Store::abrir_em`; the test asserts it equals the from-zero schema |
| `db/harness-migrations-upgraded.txt` | Applied migration versions of the upgraded database | same database, same query |
| `db/rag-schema.txt` | Schema of `rag.db` | same query on `rag.db` |
| `db/migrate-empty.json` | `poph migrate --json` on an empty directory (versions before and after, migrations applied) | fixed environment, relative data directory |
| `db/migrate-again.json` | the same command on an already migrated directory (`0` migrations applied) | same |
| `api/*.json` | JSON of every read route, with the status code | real server on an ephemeral port over a fixed test database, authenticated with a global token; one file per route |
| `metrics/types.txt` | Names and types of the `poph_*` series | `# TYPE` lines of `/metrics`, sorted and deduplicated; values are not part of the contract |
| `flows/*.star` | The three example flow scripts | copied from `tests/fluxos/exemplos/` |
| `flows/*.ir.json` | The IR compiled from each example | `poph_flows::compilar` with the default limits; the capture fails if a script produces any diagnostic |
| `config/default.toml` | Keys and default values of `poph.toml` | `HarnessConfig::default()` serialized to TOML |
| `config/doctor-empty.json` | `poph doctor --json` on a directory without databases | fixed environment |
| `config/doctor-migrated.json` | `poph doctor --json` after `poph migrate` | same |

The test database is seeded through the store repositories with fixed names: one org, one agent
with instructions, one product category, one product, one knowledge group with one item, one
library file, one flow with two revisions, one conversation with two messages and one turn, one
test-chat conversation, one admin token, one WhatsApp integration and the Matrix module turned on
with one space. The store clock is fixed, so every recorded timestamp is the same on every run.

## Normalization

Some values cannot be reproduced byte for byte, so the capture normalizes them. Everything else is
kept as the server answered:

- every UUID, standalone or inside a larger string, becomes `<uuid>`;
- every ISO-8601 timestamp becomes `<timestamp>`;
- the temporary data directory path becomes `<data_dir>`;
- `uptime_seconds` becomes `0`;
- the compiler version of `poph doctor` becomes `rustc <version>`.

JSON is written with sorted keys and two-space indent. Text files keep a single trailing newline.
In the database schema, the SQL of each object loses its `--` comments and runs of whitespace
become a single space, so the schema is compared by what it declares. Removing a comment from a
published migration therefore keeps the comparison green, while any change of table, column,
index, trigger, type or default fails it.

## What is out of scope

- Screens of the Console: they are captured by the visual baseline of the Designer task.
- `console/src/api/schema.d.ts`: it is generated from the OpenAPI document and checked by
  `console/scripts/verify-api.sh`, so it is not a source here.
- The `/openapi.json` route: it serves the same document as `poph openapi`.
- The Application Service routes of Matrix: they are the Synapse contract and answer `404` while
  the module is off.
- `GET .../test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content`:
  it needs a stored attachment, and the same file delivery is covered by the library file content
  route.
- Values of the metrics series, `uptime_seconds` and free disk space in `poph doctor`: they change
  between runs and are not part of the contract.
- Routes that need an external service (Evolution, Synapse, LLM providers) beyond the deterministic
  error they answer when unconfigured.

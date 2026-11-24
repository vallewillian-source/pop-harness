# POP Harness: history retention (HR-51)

Deletes history that is older than each org's retention period. Default: 30 days (PRD §7.4). `0` = never delete.

## Commands

```
poph retention run [--dry-run] [--json]     # all orgs
poph --org <slug|id> retention run [...]    # one org only
poph retention show [--json]                # retention days for each org
poph --org <slug|id> retention show         # one org only
```

- `run` deletes. `run --dry-run` only shows what would be removed; the database and the files folder stay as they were.
- The period comes from `retencao.dias` in `orgs.config_json`. Without that key, the instance setting `retencao.dias` applies (default 30).
- An invalid value (negative, text, decimal, above 36,500) is an error. The whole org is skipped, and the message says "Nothing was deleted in this org". No org is ever deleted "by default".
- Output: `0` when no org had an error; `1` when any org did; `2` is incorrect usage.
- `--json` has `versao_relatorio: 1`. In `run`, the field `banco.ids_conversas` lists the affected conversations (what the dry run shows so you can check before deleting).

## What gets removed (only what is expired, in the org being swept)

| Table | Criteria |
|---|---|
| `messages` (and their `attachments`) | `created_at` earlier than the cutoff, even if the conversation is open |
| `outbox` | `state = 'sent'` and sent before the cutoff |
| `inbox` | `processed_at` filled in and earlier than the cutoff |
| `turns` | `state` other than `running`, finished before the cutoff, with no unsent output and no message pointing to it |
| `conversations` | created before the cutoff and **empty**: no messages, turns, output or cart |
| blobs in `<blobs>/<org>/` | sha256 with no live attachment or library file; abandoned `.tmp` files |

A conversation is only removed after the cutoff has deleted its messages. A conversation created before the
cutoff that has a recent message stays in full.

## What is never removed

Knowledge base (groups and items), products and categories, the agent's file library, orgs, agents,
instructions, integrations, tokens, departments, cart and `audit_log`.

Flows and Cases (FX-34) are not removed either:

- **Cases** (`fichas`): the Case is the record of the attendance and outlives the conversation (PRD L12). If the
  sweep deletes the conversation, the Case stays and loses only its link: `conversation_id` becomes null
  (`ON DELETE SET NULL`). Its `status` does not change; closing the Case is the end of the attendance, not the
  retention.
- **Executions, steps, events and effects** (`fluxo_execucoes`, `fluxo_execucao_etapas`, `ficha_eventos`,
  `fluxo_efeitos`): what happened in the attendance. All stay, including those of a Case whose conversation has
  already been removed.
- **Flows, published revisions and agent permissions** (`fluxos`, `fluxo_revisoes`, `agente_fluxos`): a published
  revision is immutable and only leaves with the whole flow (deletion with `force`) or with the org.

There is no retention for Cases: while the product decision does not change, a Case does not expire.

Pending items are never deleted: unprocessed inbox entries, output in `pending`, `sending` or `failed`, and
a turn in `running`.

## How it works internally

- The clock is passed in as a parameter (`Opcoes::agora_ms`). Tests simulate the future without sleeping.
- The deletion rule lives in `poph-store` (repository `retencao()`). The `poph-retention` crate decides the
  cutoff, reads the config and gathers the files.
- Order per org: read the retention → delete in the database (one transaction per org) → list the blobs with no
  owner → delete the blobs → write the audit entry.
- If the database fails, no file is deleted.
- With `--dry-run`, the database runs the same calculations inside a savepoint and rolls back at the end. The
  counts match the real run.
- Blobs: only a file whose name has 64 hexadecimal characters (the sha256) or ends in `.tmp` is considered.
  Folders and symbolic links are never considered. A file modified in the last hour is kept (fixed grace
  period), so a blob that another request just wrote and has not yet registered is not deleted.
- Audit: one `audit_log` row per org swept, with `action = retention`. It holds the counts (`mensagens`,
  `anexos`, `turnos`, `inbox`, `outbox`, `conversas`, `arquivos_sem_dono`, `arquivos_apagados`), the cutoff and
  `simulacao` (`true` in a dry run). No row is written for a skipped org.

## Known limits

- **Sweep without a date index.** The query for expired messages reads the whole table of the org. It shows up
  with a lot of history (millions of messages). Medium severity; measure it in the load test.
- **Empty-conversation check without an index on `outbox(conversation_id)`.** Same cause; with many candidate
  conversations the sweep becomes slow. Medium severity; fix with an index in a future migration.
- **Audit written after the files.** If the audit row fails after the blobs have been removed, the report marks an
  error, but the files are already gone. The next sweep does not repeat the database deletion. Low severity.
- **Crash in the middle of blob removal.** What is left becomes orphaned and is removed in the next sweep. No live
  data is lost. Low severity.
- **Fixed 1-hour grace period**, not configurable. Low severity.
- **Does not run by itself.** The worker in `poph serve` and the scheduling are left for the integration; today it
  only runs through `poph retention run`. Run it with the server stopped or without load, as with `poph migrate`.
- **A message deleted in an open conversation does not come back.** This is the retention rule, not a defect.

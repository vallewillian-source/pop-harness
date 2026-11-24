# POP Harness: single-file org backup (HR-53)

Exports a whole org to **one file** and rebuilds that org from it on another instance.
It is the basis of the Console's "export/import org" (HR-48): the screen will call the same logic.

## Commands

```
poph --offline backup export <org> --out <arquivo> [--com-conversas] [--sobrescrever] [--json]
poph --offline backup import <arquivo> [--org <destino>] [--sobrescrever] [--json]
```

- `<org>` is an id (UUID v7) or a slug. Both commands run **directly on the database** and require `--offline`
  (like `org`, `file` and `kb`): run them with the server stopped.
- `--com-conversas` includes conversations, messages, attachments, turns, both queues (`inbox`/`outbox`) and
  the carts. Without the flag, the file carries only the configuration, the knowledge base, the catalog and the
  library.
- `export` **does not overwrite** an existing file without `--sobrescrever` (the flag's name is Portuguese and is
  kept; here it means "overwrite"). The file is written to a temporary file next to it and renamed at the end: an
  error in the middle does not leave a half-finished backup.
- `import --org <destino>` writes inside an **already existing** org (id or slug), keeping that org's name and
  configuration. Without `--org`, the source org from the file is used — created if it does not exist.
- `import` **refuses by default** when it would update a row that already exists in the destination: it says how
  many rows, in which tables, and up to five examples (`tabela: slug`), writes nothing and exits with code `1`.
  `--sobrescrever` authorizes it (the `--sobrescrever` of `export` is a different thing: there it is the output
  file). A clean destination goes in without the flag.
- Output: `0` when finished (including with the pending-index warning); `1` on error; `2` is incorrect command-line
  usage. `--json` has `versao_relatorio: 1`.

## File format

JSON, with whatever extension the operator wants (suggestion: `.json`). Structure:

| Field | What it is |
|---|---|
| `formato` | `"poph-backup-org"` — checked on import |
| `versao_formato` | `2` — version 1 remains importable; an unknown version is refused with a clear message |
| `exportado_em` | instant of the export (ms since the Unix epoch) |
| `org_origem` | `{id, slug, nome}` of the org the file came from |
| `com_conversas` | whether the history was included |
| `org` | the full row of the `orgs` table |
| `tabelas` | table → list of rows (objects column → value) |
| `blobs` | list of `{sha256, conteudo_base64}` — one per content, deduplicated by sha256 |

### What goes in

| Table | Always |
|---|---|
| `product_categories`, `departments`, `products` | yes |
| `agents`, `agent_instructions`, `agent_product_categories` | yes |
| `knowledge_groups`, `knowledge_items`, `agent_knowledge_groups` | yes |
| `file_library` (+ blob bytes) | yes |
| `integrations` (without the secret) | yes |
| `fluxos`, `fluxo_revisoes` (source and IR), `agente_fluxos` | yes |
| `conversations`, `messages`, `attachments` (+ blob bytes), `turns`, `inbox`, `outbox`, `carts`, `cart_items` | only with `--com-conversas` |
| `fichas`, `ficha_eventos`, `fluxo_execucoes`, `fluxo_execucao_etapas`, `fluxo_efeitos` | only with `--com-conversas` |

Published revisions keep their numbers and are immutable on import, including when overwriting is authorized.
Version 1 backups with conversations are imported with a Ficha created for each conversation.

**Customer name (`conversations.client_name`, MX-32).** The column travels with `conversations` and goes in under the
same `--com-conversas`, like `external_peer`: it is personal data of the customer, and the handling is the same.
A backup made before migration `0008` does not have the column; the import accepts that file and leaves the name
empty in the conversations.

**Send id (`outbox.external_message_id`, MX-40).** Travels with `outbox` under `--com-conversas`, like the rest of
the queue. It is not customer data: it is the id the channel gave the sent message. A backup made before migration
`0011` does not have the column; the import accepts the file and leaves the id empty, with no effect on the rest of
the queue (the item stays `sent`).

### What does **not** go in

- **`admin_tokens`**: credential. The token is created again with `poph token`.
- **`audit_log`**: infrastructure history, not part of the org's state.
- **`integrations.secrets_enc`**: the encrypted secret. Neither in plain text nor encrypted (see below).
- **`blob_path`**: internal path (`<org>/<sha256>`). The file keeps `sha256` and the bytes; the path is recalculated
  for the destination org on import.
- **Matrix module tables (`matrix_orgs`, `matrix_rooms`, `matrix_spaces`, `matrix_estado`, `matrix_falhas`,
  `matrix_avisos_falha`)**: they **do not** travel in the backup. The room belongs to the environment's Synapse, and
  a `room_id` copied to another installation would point to a room that does not exist there (or, worse, to one
  from another environment). On the destination instance the module starts turned off and, when the org turns it
  on, it creates the rooms again from the conversations that exist. The same applies to import: a file never
  carries these tables, and their schema is not touched.

## Secret

From the integration secret, the file carries only:

- `tem_segredo: true` — a secret was saved;
- `segredos_campos: ["token"]` — the **names** of the fields filled in, when the process's encryption key
  (`POPH_CHAVE_CIFRA`) can open the blob. Without the key (or with a different key from the one that saved it), the
  value is `null`: only the mark remains.

The value is never written, in any form. On import, the integration comes in with **empty** secrets and the report
says how many must be typed again (Integrations tab of the Console). Re-importing the same file **does not erase**
a secret that was typed in after the first import.

On export, prove that the value did not leave with a `grep` for the real secret in the file (the CI test does this
with an org that has an encrypted secret saved).

## Import

- **Atomic**: all rows go in one transaction of the writer. An error at any point undoes everything — no half-done
  state is left. The blobs are written first (they are content-addressed: writing the same hash again changes
  nothing); if the database refuses, they stay orphaned and retention removes them later.
- **Finds by the table key**: each row is looked up by `id` (or by the columns of the composite primary key). A row
  that does not exist is inserted (`inseridas`); a row that already exists is **updated** (`atualizadas`) with the
  value from the file — and that is why the update needs explicit authorization (below). Importing the same file
  twice, with `--sobrescrever`, leaves the same state, without duplicating anything.
- **Update requires authorization (HR-48e-3)**: without `--sobrescrever`, the import checks before writing (read-only)
  and, if any row in the file already exists in the destination, **refuses**: it shows the total, the tables and up
  to five examples, and writes nothing. Re-importing an old file into the same org would revert what changed after
  it (agent version, instructions, Jev) — this is the case the refusal protects against. In a destination org
  (`--org`), the org's own row is not counted: it is not touched.
- **Does not delete**: what exists in the destination and is not in the file stays as it is. To rebuild exactly,
  import into a clean instance.
- **Ids preserved**: agents, products, conversations, messages and turns keep their ids, versions and timestamps.
  This is what keeps conversation → agent, message → turn and attachment → message linked.
- **Natural key conflict**: if the destination already has an agent/product with the same **slug** (or
  category/department/group with the same name, or the org with the same slug), with a **different id**, the import
  is refused **before writing**, with the list of what conflicted. There is no silent "merge". Importing the same
  content (same ids) is an update, not a conflict.
- **Unknown column**: if the file carries a column this version of the schema does not have, the import is refused
  with the table and column name (a file from a newer version).
- **Old file**: a file generated before the `origin` column (migration `0003`) does not carry `origin` or
  `author_name`, and can still be imported. Messages come in as `agent`, without an author.

### Blobs

The content of each library file and each attachment goes into the file in base64, with its sha256. On import the
sha256 is checked before writing (a changed or truncated file is refused), the blob is written by `poph-files` into
the folder of the **destination org**, and the rows' `blob_path` is recalculated.

## Re-indexing

After importing, the org's knowledge base and catalog are re-indexed without a manual step (`sincronizar_org` of
each), so `poph kb search` and the `buscar_produto` tool already work.

- Without the embedding model installed (`poph model install`), the import **ends with a warning** and the index
  stays pending: run `poph kb reindex` and `poph product reindex` when the model is available. Nothing is saved as a
  silent empty index.
- The report carries `indice.base_conhecimento` and `indice.produtos` with agents/chunks; the warning, if any, goes
  in `aviso` (and in the default error output, in text mode).

## Validation and errors

| Situation | Response |
|---|---|
| truncated file or one that is not JSON | `o arquivo … não é um backup do POP Harness (JSON inválido: …)` |
| different `formato` | "este arquivo não é um backup do POP Harness" |
| unknown `versao_formato` | "versão de formato desconhecida: N; esta instalação entende a M" |
| blob whose content does not match the sha256 | "o conteúdo de um arquivo do backup não confere com o sha256 gravado" |
| column outside the schema | "o arquivo traz a coluna `x` em `tabela`…" |
| natural key repeated with another id | list of conflicts, and nothing is written (not even with `--sobrescrever`) |
| rows that already exist in the destination, without `--sobrescrever` | total, tables and up to 5 examples, and nothing is written |
| blob missing from disk on export | error with the file name and the sha256 (no path) |

(The messages above are emitted by the code in Portuguese and are kept as they are, because they are the text the
operator sees.)

## Limits

- **No compression**: the file is JSON with the blobs in base64 (~4/3 of the byte size). A 16 MB file becomes ~21 MB
  in the backup. Through the API, the body limit is 64 MB.
- **Whole file in memory**: export and import load the file and the blobs into memory. For orgs with many large
  files this weighs on memory; there is no sending in pieces.
- **No secret**: the integration comes back without a credential; whoever imported has to type the secret again.
  Rotating `POPH_CHAVE_CIFRA` before exporting only removes the field names (`null`), never the value.
- **History**: `--com-conversas` carries the whole live history of the org (retention already deletes what is expired
  first, if it is configured).
- **Concurrency**: this is an offline command. With the server writing at the same time, the snapshot may not reflect
  the last instant of each table (the read uses a single connection, but does not lock the server). Run it with the
  server stopped.
- **Import into an org that already has data**: re-importing the same file updates **only** if it comes with
  `--sobrescrever` (without it, it refuses with the list of what would be overwritten); a different file with the
  same slugs and another id is always refused. There is no "merge" mode.
- **HTTP**: the same two operations exist in the administrative API (`GET …/backup` and `POST …/backup/import`, with
  `?sobrescrever=true` to authorize the update) — see `docs/API.md`. The Console uses these routes, not the CLI.

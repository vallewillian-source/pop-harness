# Agent file library and `enviar_arquivo` (HR-24)

The agent only sends the customer a file that is in **its library** (decision D13). The model never chooses a disk path
or content: it chooses an **id** from a list it sees in its own request. The `poph-files` crate (HR-23) writes the file;
the turn engine decides what goes out; the `outbox` (HR-12) delivers it to the channel.

## Model

Table `file_library` (it already existed in the HR-07 schema, so there is no new migration):

| column | content |
|---|---|
| `id` | UUID v7 of the file |
| `org_id` | owning org |
| `agent_id` | owning agent (the file exists only for it) |
| `name` | name shown to the customer (up to 200 characters) |
| `description` | text the model reads to decide whether to send it (up to 500) |
| `mime` | **real** type, detected from the content |
| `size_bytes` | size in bytes |
| `sha256` | hash of the content (the blob is deduplicated by it) |
| `blob_path` | blob path **relative** to the blobs folder (`<org>/<sha256>`) |
| `created_at` | creation time (ms) |

The content is not kept in SQLite. The blob is stored at `<dados.blobs>/<org>/<sha256>`, written by `poph-files`, and
is shared by any record with the same hash within the org.

There is no version or `If-Match`: the table is not editable. To change a file's content, delete it and upload it again
(the blob is not duplicated if the content is the same).

## Upload through the CLI

```
poph file upload <caminho> --agent <id|slug> [--name "Nome mostrado"] [--description "…"] [--json] \
  --org <slug|id> --offline
poph file list   --agent <id|slug> [--limit 50] [--json] --org … --offline
poph file show   <id> --agent <id|slug> [--json] --org … --offline
poph file delete <id> --agent <id|slug> [--json] --org … --offline
```

CLI rules:

- `--offline`, `--org` and `--agent` are required (as with `org`, `token`, `kb`).
- Upload goes through `poph-files`: 16 MB limit, real type from the content, misleading extension refused, blob by hash.
  A refused file leaves nothing written.
- The file is read with a cap (`limite + 1` bytes): a file of several GB never goes into memory whole just to be refused.
- Business error: exit code 1 and a message in Portuguese on the standard error output (the code emits Portuguese).
  `--json` outputs with `versao_relatorio: 1`.
- `delete` removes the record; the blob stays on disk until retention (HR-51) removes what has no owner.

PDF and Office need the `poph-extrator` next to the executable (see `docs/ARQUIVOS.md`). Without it, uploading a document
fails with a clear message; images keep working.

## `enviar_arquivo` tool

- Name: `enviar_arquivo`. Arguments: **only** `id` (`additionalProperties: false`). Any other key (path, name, content)
  is refused with a readable error.
- The id is checked against the library **of the same agent and the same org**. An id from another agent, another org,
  or a non-existent id returns `erro: não existe arquivo com esse id na sua biblioteca…` to the model (the message is in
  Portuguese, as the code emits it). The turn continues and the model can carry on without the file.
- Per-agent enablement (D-04): without `enviar_arquivo` in the agent's tool list, the tool is not sent to the model.
- The tool **description** is completed, on every turn, with the agent's library list:

  ```
  - <id> — <nome> (<mime>, <tamanho>): <descrição>
  ```

  This is how the model decides what to send, without needing another call. The list shows up to 60 files
  (`LIMITE_LISTAGEM`); above that, a notice shows how many were left out. An empty library is shown as empty.
- The tool's result comes back as **data** (name and mime with no `<` or `>`), not as an instruction.

## What goes into the `outbox`

Item `kind = 'file'` with `payload_json`:

```json
{
  "blob_ref": "<org>/<sha256>",
  "arquivo_id": "<uuid v7>",
  "nome": "catalogo.pdf",
  "mime": "application/pdf",
  "tamanho_bytes": 4096,
  "sha256": "<64 hex>"
}
```

- The text reply of the turn uses `seq = 0`; the file uses a `seq` derived from the id (range 1 to 1,000,000,000),
  stable per file.
- **Idempotency:** repeating the same call in the same turn hits the same `(turn_id, seq)` and does not duplicate; the
  tool replies "already queued in this turn". Two different files in the same turn take separate items.
- The state follows `pending → sending → sent | failed` through HR-12 (`D11`: at least one delivery).

## What is missing for real sending

- **Channel transport** (HR-25/HR-26): today the item stays in the `outbox`; WhatsApp and the plugin come with the same
  contract.
- **Console (HR-46)**: show the sent file as an attachment in the conversation. Today the item lives in the `outbox`;
  the console transport confirms the item without printing anything (only text is printed).
- **Per-turn file limit**: there is no limit beyond the agent's `max_rodadas_ferramenta` — each round can queue one or
  more files.

## Tests

- `crates/poph-store/tests/biblioteca.rs` — repository: create/list/count/delete, isolation by org and by agent within the
  same org, agent from another org refused, field validation.
- `crates/poph-agent/tests/biblioteca_arquivos.rs` — tool and turn: id of another agent/org refused (including with a
  forged org in the context), non-existent id with a readable error and the turn carrying on, `SendFile` in the `outbox`
  with `blob_ref` and no duplicate on repetition, an agent without the tool does not receive it, an agent with it sees
  the file descriptions, empty library.
- `crates/poph/tests/cli_arquivo.rs` — end-to-end CLI: upload/list/show/delete, blob dedupe by hash, file above 16 MB
  refused, misleading extension refused, errors with exit code 1, `--offline` required, library per agent.

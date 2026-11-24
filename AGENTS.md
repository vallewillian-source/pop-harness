# AGENTS.md — POP Harness (Rust)

Code map for whoever (person or AI agent) is going to change this project. Also read
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the contribution flow. To install, configure and run, read the
[`README.md`](README.md).

> **Language.** Code, identifiers and comments are in English, and code carries no comments. Product text — error
> messages, log text, the Jev and agent prompts, the Console UI, test data — stays in Brazilian Portuguese, and so
> do the names the outside sees (tables, routes, CLI flags, configuration keys). Documentation is in English. The
> exceptions are listed in full under "Rules that apply here"; anything outside them is not allowed.

POP Harness is independent of POP Atende. Nothing in the workspace imports POP Atende code: mentions of it
are only in comments and test names.

This directory is **its own Git repository** (`github.com/vallewillian-source/pop-harness`), used as a
submodule by whoever consumes it. Commit and push of POP Harness happen **in here**; the consuming
repository only stores a pointer to the commit. Before committing, run `bash scripts/ci.sh` and check that
no customer data, machine path or internal document entered the diff.

## Before changing anything

1. Read the document for the subject in `docs/` (list in the "Business rules" section below). The docs
   describe the design and the limits. When the code changes a behaviour, update the document for the same
   subject.
2. Scope and base decisions: `docs/VISAO_GERAL.md` and `docs/DECISOES.md`. Task and acceptance status:
   `ROADMAP.md`. Channel contract: `docs/CONTRATO_CANAL.md`. Decisions about data, API contracts and
   architecture belong to the maintainer; whoever implements does not change them alone. References to
   "PRD §x", "D6" or "HR-xx" in docs and code comments come from the specification removed on 2026-11-21;
   the equivalence is in `docs/VISAO_GERAL.md` §10, and the commit that closed each task cites it.
3. There is no deploy from this project. There is no deploy script here: `scripts/build-dist.sh` only
   produces the local package. Sending to the ARM64 test machine or to production requires explicit
   authorization from the maintainer.

## Rules that apply here

- **All documentation is in English.** Every document in this repository is written in English: `docs/`, the
  root `*.md` files, `console/UX_SPEC.md` and the READMEs in the test fixtures. Do not add a document in
  Portuguese. If a document in Portuguese is found, translate it or delete it when it is obsolete. Published
  names (`fichas`, `poph ficha`) keep their form inside code formatting; in English prose, use the English term
  from `docs/GLOSSARY.md` (for example, *Case* for *Ficha*).
- **Code is in English, with no comments.** Identifiers, log keys, test names and code file names are in English.
  Comments are removed: code and configuration carry none, so the code has to read on its own; an explanation that
  would have been a comment belongs in the matching document in `docs/`. The only exceptions are the shebang in the
  first line of a script and a directive that breaks the tool if it goes (`# noqa`, `# type: ignore`, coverage
  ignore markers and the like), each one in the versioned allowlist `scripts/allow-comments.txt`, one line per
  exception with the reason. Code written from now on is born this way, including work already in progress.
  `bash scripts/check-comments.sh` is the gate; the files still to be cleaned are listed in
  `scripts/pendentes-comentario.txt` (generated, one line per file with its module and the count), and each module
  task removes its own lines on the commit that cleans them. The gate reads what the repository publishes: a file
  kept only on this machine by `.git/info/exclude` is out, and an allowlist exception that matches nothing fails.
  `bash scripts/check-names.sh` is the same kind of gate for identifiers: the Portuguese words of
  `scripts/glossary-map.txt` (with their inflections) are refused outside `scripts/allow-names.txt` — the closed
  list of section 7.2, generated from the published inventory — and outside `scripts/pendentes-nomes.txt`, which
  each module task shrinks with `bash scripts/check-names.sh --atualizar-pendentes` on the commit that renames it.
  Strings and comments are never read, so product text is out of it.
- **Product text is in Portuguese.** Error messages, log message text, the Jev and agent prompts, the Console UI
  and test data are written in Brazilian Portuguese, because the customer and the operator read them. Only the
  `tracing` key is an identifier and becomes English; the message text does not change.
- **Published names keep today's value.** Tables, columns, indexes, triggers, routes, `operationId`, JSON fields,
  `poph.toml` keys, CLI flags and command names, environment variables, metric names, plugin protocol terms and
  the words of the flow language do not change; an internal name that becomes English keeps the published value
  through an explicit attribute. The list of what is frozen is the inventory of the second wave (the names the
  outside sees), which is a separate decision.
- **Flows are pure in the engine.** `poph-fluxos` decides and `poph-store` writes. A flow transition is written once, in one transaction, through `Store::execucoes()` (`aplicar_transicao`). Effects are queued and delivered by the server loop, never by the engine.
- **Secrets never leak.** Keys come only from the environment (`POPH_CHAVE_CIFRA`, `ANTIGRAVITY_API_KEY`,
  `TYPESAFE_API_KEY`, `EVOLUTION_API_KEY`, `POPH_CLUSTER_SECRET`). They never go into a file, log, report
  or API response. The `Sensivel` type (in `crates/poph-types/src/segredo.rs`) prints `[oculto]` in
  `Debug` and `Display`. Use it for any sensitive value that passes through a log or error.
- **Migrations.** A new migration is a file `crates/poph-store/migrations/harness/NNNN_name.sql`,
  registered in `crates/poph-store/src/migracoes.rs` with the next number. Never edit a published
  migration — with one exception: a published migration may lose its comments and nothing else. The file name, the
  tables, columns and indexes and the SQL itself stay exactly as they are. The runner stores the version and the
  file name, with no checksum of the content (`crates/poph-store/src/migracoes.rs`), so a database already migrated
  sees no difference; the proof is the schema of a database migrated from scratch and of an old one, identical
  before and after. It is applied by `poph migrate` **and also by the server** when it opens the database
  (verified: a new database, with the server starting, already came out at version 2 without
  `poph migrate`).
- **An org token cannot do global actions.** Creating an org, creating or revoking a global token and
  reloading the cluster require a global token. With an org token the API answers `403 forbidden`. With
  no token, `401`.
- **API contract.** Do not edit `console/src/api/schema.d.ts` by hand: it is generated from `poph openapi`
  by `bash console/gen-api.sh`. CI (`verificar-api.sh`) fails if the file diverges from the server.
- **Git.** Local commits follow the repository's policy. Push and deploy only with the maintainer's
  authorization.

## Crates (`crates/`)

Names and dependencies come from `cargo metadata` for this workspace, not from memory.

| Crate | What it does | Imports (internal, normal dependencies) |
|---|---|---|
| `poph-types` | Domain types, IDs, errors, contracts, configuration (`config.rs`), `Sensivel` (`segredo.rs`) | none (it is the base) |
| `poph-core` | LLM provider client (`llm/` module, with the antigravity provider) | `poph-types` |
| `poph-store` | SQLite storage (`harness.db` and `rag.db`): one writer, WAL readers, migrations, per-org repositories, backup | `poph-types` |
| `poph-rag` | Local embedding (ONNX Runtime, int8), batch queue with a per-org limit, model installation | `poph-types` |
| `poph-files` | File intake: size limit, type from content, blob by hash, PDF/Office text extracted in an isolated process (`poph-extrator` binary) | `poph-types` |
| `poph-jev` | Jev client, gate framework, question batching, decision cache | `poph-types` |
| `poph-fluxos` | Flows and Cases module. Starlark script → structure (IR) compiler, validator, rules, execution engine, prompt block, diff between revisions and formatter. All pure, no database, no network: the tables live in `poph-store`, the turn wires the engine in `poph-agent`, and the API lives in `poph-server`. Language specification: `docs/FLUXOS_LINGUAGEM.md`. **Under construction** | `poph-types` and the Starlark library (`starlark`, `blake3` with `pure`). No internal crate other than `poph-types` |
| `poph-plugins` | Supervision of external-process plugins: JSON-lines protocol over stdio, handshake, timeout, restart with backoff | `poph-types` |
| `poph-evolution` | EvolutionAPI (WhatsApp) HTTP client: sending, received media, channel manifest | `poph-types` |
| `poph-matrix` | Matrix module: Application Service configuration and secrets; Matrix API client (rooms, events, state, profile, virtual-user registration, `m.space.child`, alias resolution, space hierarchy); virtual users and profile (with `eh_fantasma`/`eh_bot` for anti-echo); per-conversation room provisioning (deterministic alias, Element metadata, `creating` → `ready`), with persistence injected through the `PortaSalas` port; per-department spaces (department resolution, space creation via the `#dep_<department>` alias, linking to an existing space with a permission check, `m.space.child`/`m.space.parent`/restricted join, batch invites and space swap on transfer), with persistence injected through the `PortaEspacos` port; format of mirrored messages; classifier of events received from Synapse (agent reply, agent media or noise, with an enumerated reason). The module's tables live in `poph-store` (`matrix_*`) and `poph-server` implements the port. **`poph-agent` does not import this crate** | `poph-types` |
| `poph-agent` | Hot state: agent configuration cache, per-conversation actor, turn (`turno/`), context window, summary, limits, knowledge base, products, cart | `poph-core`, `poph-files`, `poph-jev`, `poph-rag`, `poph-store`, `poph-types` |
| `poph-retention` | Per-org retention sweep, orphan blobs, dry run (`--dry-run`), audit | `poph-store`, `poph-types` |
| `poph-server` | Admin API (axum), authentication, OpenAPI, embedded Console, cluster, turn input and output, WhatsApp, encryption, the Matrix Application Service receiver (`api/matrix_as.rs`), delivery of the agent's reply to the customer (`matrix_resposta.rs`, using the atomic operation in `poph-store`), the room notice when that reply fails for good (`matrix_falhas_aviso.rs`) and the module's composition root (`matrix_inicio.rs`: turns it on, degrades without taking the server down, honours `matrix_orgs` on every cycle and stops in order) | `poph-agent`, `poph-evolution`, `poph-files`, `poph-matrix`, `poph-plugins`, `poph-rag`, `poph-store`, `poph-types`. `poph-core` only in tests |
| `poph` | The single binary: CLI (clap) and `serve`. One module per command group (`org.rs`, `agente.rs`, `cluster.rs`, `servidor.rs`...) | almost all of the above |
| `poph-plugin-eco` | Example plugin (reversed text) and template for new plugins | `poph-types`. Only in tests: `poph-agent`, `poph-plugins`, `poph-store` |

### Who may import what

The rule is this table. Cargo prevents cycles, but **there is no automatic check** of the direction of the
layers: there is no dependency-cruiser equivalent in this tree. When adding a dependency between crates,
check it against the table. Two rules the table makes clear:

- `poph-types` depends on no internal crate. Everything starts from it.
- Capability crates (`poph-store`, `poph-rag`, `poph-files`, `poph-jev`, `poph-plugins`,
  `poph-evolution`, `poph-matrix`, `poph-core`, `poph-fluxos`) know neither the server nor the CLI.
  `poph-agent` (turn) and `poph-server` (HTTP) put the pieces together.
- `poph-agent` does **not** import `poph-matrix`: the Matrix module is wired by `poph-server` and reads what
  the turn already recorded. If the turn ever needs the module, that goes back to the maintainer first.

## Business rules and where they are

| Subject | Code | Doc |
|---|---|---|
| Turn: from message to reply | `poph-agent/src/turno/`, `poph-server/src/entrada_turno.rs`, `saida_whatsapp.rs` | `docs/TURNO.md`, `docs/ENTRADA_E_SAIDA.md` |
| Concurrency: one actor per conversation, queues, limits | `poph-agent/src/atores.rs`, `limites.rs`, `config_quente.rs` | `docs/MEMORIA_E_CONCORRENCIA.md` |
| Context and rolling summary | `poph-agent/src/contexto.rs`, `resumo.rs` | `docs/RESUMO.md` |
| Knowledge base and hybrid search | `poph-agent/src/base_conhecimento.rs`, `poph-rag` | `docs/BASE_DE_CONHECIMENTO.md` |
| Products and `buscar_produto` | `poph-agent/src/produtos.rs`, `carrinho.rs` | `docs/PRODUTOS.md` |
| Files and the agent library | `poph-files`, `poph-agent/src/biblioteca.rs` | `docs/ARQUIVOS.md`, `docs/BIBLIOTECA_ARQUIVOS.md` |
| Jev: gates and fast decisions | `poph-jev/` (`gates.rs`, `motor.rs`, `cache.rs`) | `docs/JEV.md` |
| Authentication: global and org token | `poph-server/src/auth.rs` | `docs/API.md` (Authentication section) |
| Encryption of stored secrets | `poph-server/src/cifra.rs` | `docs/CONFIGURACAO.md` |
| Cluster: map, fingerprint, reload | `poph-server/src/cluster.rs`, `poph/src/cluster.rs`, `poph-types/src/config.rs` | `docs/CLUSTER.md` |
| Forwarding between instances | `poph-server/src/encaminhamento.rs` | `docs/CLUSTER.md` (Forwarding section) |
| History retention | `poph-retention/` | `docs/RETENCAO.md` |
| Org backup | `poph-store/src/backup.rs`, `poph-server/src/api/backup.rs` | `docs/BACKUP.md` |
| Integration plugins | `poph-plugins/`, `poph-plugin-eco/` | `docs/PLUGIN_PROTOCOLO.md`, `docs/PLUGIN_EXEMPLO.md` |
| WhatsApp (EvolutionAPI) | `poph-evolution/`, `poph-server/src/evolution.rs` | `docs/EVOLUTION.md` |
| Channel contract and turn events | `poph-types/src/integracao.rs`, `poph-agent/src/turno/mod.rs` | `docs/CONTRATO_CANAL.md` |
| Matrix Application Service receiver (routes Synapse calls) | `poph-server/src/api/matrix_as.rs` | `docs/API.md` (Application Service section) |
| Agent reply in Element → customer on WhatsApp | `poph-server/src/matrix_resposta.rs` (on top of `poph-matrix/src/eventos.rs` and the atomic operation in `poph-store`) | `docs/ENTRADA_E_SAIDA.md` §5 |
| Room notice when the agent's reply fails for good on WhatsApp | `poph-server/src/matrix_falhas_aviso.rs` (`AvisoDeFalhas`, wired by `matrix_inicio`; table `matrix_avisos_falha`, migration `0009`) | `docs/ENTRADA_E_SAIDA.md` §8, `docs/DADOS.md` §11.6 |
| Per-department spaces (what the agent sees organised in Element) | `poph-matrix/src/espacos.rs` (`PortaEspacos` port, implemented by `poph-server` over `store.matrix()`) | `docs/DADOS.md` §11.4 |
| Matrix module wiring in the server (starts, degrades, per-org on/off, ordered stop) | `poph-server/src/matrix_inicio.rs` (composition root; `PortaEspacosStore` over `store.matrix()`) | `docs/ENTRADA_E_SAIDA.md` §8, `docs/API.md` |
| Overview, base decisions, status | (no code) | `docs/VISAO_GERAL.md`, `docs/DECISOES.md`, `ROADMAP.md` |
| Flows and Cases (language, compiler, engine, effects, API and CLI) | `crates/poph-fluxos/`; `poph-store` (`fichas`, `fluxos`, `fluxo_*`); `poph-agent` (`turno/fluxo*.rs`); `poph-server` (`api/fichas.rs`, `api/fluxos.rs`) | `docs/FLUXOS.md` (overview, operation, limits); `docs/FLUXOS_LINGUAGEM.md` (language, IR and diagnostics) |
| Databases, schema and migrations | `poph-store/` (`migracoes.rs`, `migrations/harness/`, `escritor.rs`) | `docs/DADOS.md` |
| Agents and instructions | `poph-agent/src/compositor.rs`, `poph-server/src/api/instrucoes.rs` | `docs/AGENTES.md` |

When changing the Jev: it chooses, ranks or abstains. The backend validates and executes. A Jev failure
must never become an effect. This principle is in `docs/JEV.md`.

## Console (`console/`)

React, TypeScript, Vite. It is embedded in the binary: `console/dist` enters through `poph-server`'s
`build.rs`. It depends on the `@sofacoding/sofa-ui` package, referenced by a local path outside this
repository (see `ROADMAP.md`).

- `src/api/`: the API access layer. `cliente.ts` handles transport, the token and the error format
  `{ code, message, details }`. Each area has its own file (`agentes.ts`, `operacao.ts`...). Types come
  from the generated `schema.d.ts`. When the contract leaves a response body as `unknown` (as in
  `operacao.ts`), the types are written by hand in that layer. Do not touch the generated schema.
- `src/telas/`: one screen per file ("telas" = screens). Each has its component test next to it
  (`Tela.test.tsx`).
- `src/componentes/`: dialogs, modals, side menu, top bar. `navegacao.ts` has the menu list.
- `src/estilos/`: CSS per screen.
- Required states: loading, empty and error. An action button is disabled while the request runs, to
  prevent double submission.
- Token: kept only in memory and in `sessionStorage`. `401` ends the session. `403` does not: it shows the
  call's error.
- Visual: `@sofacoding/sofa-ui`. Background `#151719`, which is what applies to the Console according to
  `console/UX_SPEC.md`. Avoid a card inside a card: it is the maintainer's preference for this Console.
- Adding a screen: an entry in `navegacao.ts`, a route in `App.tsx`, and a component test next to it.

## Tests and CI

A single command, from the project root:

```bash
bash scripts/ci.sh
```

Order: cache pruning (`limpar-cache.sh`), Console bundle build, API client check, Console tests
(`console/scripts/testar.sh`), formatting (`cargo fmt --check`), clippy with `-D warnings`, build, Rust tests
(`cargo test --workspace`), model RAM test, aarch64 check of the crates without C, `cargo deny` and
`cargo audit`.

Skips without the Console installed: the three Console steps (bundle, API client and tests) declare
themselves skipped when `console/node_modules` does not exist, and exit with 0. That is in the script.

Smaller commands:

- One crate: `cargo test --locked -p poph-retention`.
- One component test: `cd console && npx vitest run src/telas/Operacao.test.tsx`.
- All Console tests: `cd console && pnpm run testar`.
- Model RAM measurement, which is `#[ignore]`: `cargo test --locked -p poph-rag --test ram -- --ignored --nocapture`.

ARM: x86_64 workstations have no cross compiler for aarch64, so `rusqlite` (in C) does not build there. The
workstation's ARM check covers only `cargo check` on the crates without C. The ARM binary is built on a
native ARM64 machine. See `DEVELOPMENT.md`.

How to prove a change: run the test of the crate you touched, plus `ci.sh` before delivering. A test that
stays green without the changed code proves nothing: confirm that it fails when the behaviour breaks.

## Known limits

- The Operation screen does not show the cluster map in use, because there is no read route. It only shows
  the result of reloads done in the session.
- No automatic check of the dependency rules between crates (see above).
- The embedding model is downloaded with `poph model install`. Until it runs with network access, search in
  the knowledge base and in products is off.

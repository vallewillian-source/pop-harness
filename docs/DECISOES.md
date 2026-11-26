# POP Harness (Rust) — Decision log

> Log of the decisions made **after** the original specification (PRD and task list, removed on 21/11/2026).
> The base decisions D1 to D13 are in [`VISAO_GERAL.md`](VISAO_GERAL.md) §4 and remain closed; here are the
> operational decisions and the maintainer's answers that change the design. Each decision says where it bites,
> so whoever implements it does not have to look for it.

| ID | Date | Decision | Governs |
|---|---|---|---|
| D-01 | 19/11/2026 | Rust installed only for the user (`~/.rustup`, `~/.cargo`), no sudo: rustup 1.29.3, rustc/cargo 1.100.0 stable, targets `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, clippy, rustfmt, cargo-deny 0.22.0, cargo-audit 0.23.0. | HR-00, HR-04 |
| D-02 | 19/11/2026 | No cross linker on the workstation: the aarch64 target is only downloaded and validated by `cargo check`. The ARM binary is compiled **directly on the ARM64 test machine** in HR-56, with the maintainer's authorization at that time — using the ARM64 test machine still requires authorization. | HR-01 (ARM caveat), HR-56 |
| D-03 | 19/11/2026 | The Console serves the SofaUI fonts at `/res/fonts` as static files and sets font, color and background on the `body`. The library is **not** changed to a relative path. | HR-38, HR-37 |
| D-04 | 19/11/2026 · maintainer | The list of tools enabled per agent **actually filters** what the agent can use. It stops being a decorative field as in POP Atende (HR-03 §7). | HR-09, HR-13, HR-19, HR-24 |
| D-05 | 19/11/2026 · maintainer | Knowledge base with inactive status **leaves the index and the search**, and an agent with inactive status **stops answering** (it receives no new turn). | HR-09, HR-16, HR-17, HR-26 |
| D-06 | 19/11/2026 · maintainer | Intent-based routing rules are **out of V1**: no screen and no implementation. They may appear as a future item, with no hook in the code. | HR-04, HR-21, HR-37, HR-52 |
| D-07 | 19/11/2026 · maintainer | **Numeric V1 targets fixed as the HR-01 spike proposed (M1–M13), with no tightening**: 12 KB per idle agent, 96 KB per indexed agent, 150 MB with 10k idle, 450 MB with 2k indexes, LRU of 512 MB, internal latency ≤ 2 ms p50 / ≤ 5 ms p95, writes ≥ 50k msg/s and p95 ≤ 5 ms at 2,000 msg/s, ≥ 20k reads/s, ingestion of 2,000 msg/s per instance, 200 configurable simultaneous turns, alert if the first text exceeds 12 s p95, load with 10k agents + 500 active conversations on ARM64. | HR-11, HR-12, HR-16, HR-55, HR-56, HR-59 |
| D-08 | 19/11/2026 · maintainer | **Crate split approved**: `poph-core` is broken into three: `poph-types`, `poph-store` (SQLite + memory), `poph-rag` (embedding + index, the only `unsafe`/FFI), `poph-agent` (LLM, Jev, composer, turn), `poph-server` (axum, API, SSE, plugins, Evolution) and `poph` (binary + embedded console). | HR-04b, HR-06 to HR-13, HR-15 to HR-27, HR-56 |

## Effect of each decision on the design

### D-04 — Enabled tools filter
- `agent_instructions.structured_json.ferramentas` (name, enabled, when to use) becomes the **source of truth** for
  the toolset: the engine offers the LLM only the tools marked as enabled for that agent. A disabled tool does not
  appear in the list sent to the provider.
- Consequence for HR-09: the field must be typed and validated (known tool name, closed list from the engine), not
  free text.
- Consequence for the turn (HR-13): the tool loop rejects a call to a non-enabled tool even if the model invents the
  name (tool error, not execution).
- Without explicit configuration, the default per agent type defined in HR-09 applies — the decision does not
  authorize "everything enabled".

### D-05 — Inactive means inactive
- An inactive knowledge item/group is not in `rag.db` (or it leaves on reindexing) and does not come back in hybrid
  search or in the `consultar_base` tool.
- An inactive agent does not open a new turn: a message received from the integration is recorded and discarded
  without a reply (with a record in `audit_log`/`turns`), following the same pattern as a paused connection.
- A conversation in progress with an agent that became inactive mid-way does not continue the turn.

### D-06 — Intent-based routing out of V1
- No intent rules table is created in `harness.db`; routing is only a transfer to a department (G1/G4).
- No rules screen appears in the Console (HR-37/HR-38) and no route in the API (HR-21).
- The importer (HR-52) ignores these rules when bringing an agent from POP Atende, without failing because of it.

### D-07 — Numeric targets count as acceptance criteria
- The full list, with the measurement behind each number, is the internal measurement report, §9 (M1–M13), which is
  not published; the values in the table above are the short version and do not replace the original table.
- HR-55 (load) and HR-59 (acceptance) measure against these numbers; a number that was not measured is "pending",
  never "equivalent". ARM64 measurements are done in HR-56, on the ARM64 test machine.
- Perceived latency targets (M12) monitor the provider: they are not an excuse for Harness to spend more.

### D-08 — Crate split approved
- The current workspace (HR-04 part A) has an almost empty `poph-core`. The split into `poph-store`, `poph-rag` and
  `poph-agent` happens **starting at HR-04b/HR-06**, without replay: `poph-core` stops existing when the first of the
  three brings real code, and the name is never reused.
- Boundary rule: `unsafe`/FFI only in `poph-rag` (embedding) and in the SQLite part; `poph-agent` does not depend on
  axum, so the turn can be tested through the CLI (HR-13) before the API (HR-20).
- `exclude = ["spike"]` in the workspace still applies; the spike does not become a crate.

### D-09 — Local RAG: `ort` + int8, short chunks and isolated file extraction
- Embedding runtime: `ort` (ONNX Runtime) 2.0.0-rc.15 linked statically, same model family as POP Atende
  (multilingual MiniLM, window 128, original `sentence-transformers` tokenizer).
- Quantization: int8 in V1, with fp32 as a fallback. Switching requires reindexing (`poph rag reindex --model`).
- Chunker: ~350 characters (~100 tokens) with an 80-character overlap, respecting paragraphs and sentences — this is
  what lets int8 pass the parity threshold (R@3 0.80 against 0.45 with 1000-character chunks). PRD §9 already
  allowed smaller chunks if truncation at 128 tokens were confirmed — it was confirmed: only 47% of the text of a
  1000-character chunk is read by the embedding today.
- Consequence for HR-15/HR-16: record the chunker version and the model's sha256 in `rag.db`, and measure parity for
  real with the acceptance corpus (int8 came out at 0.90, at the limit).
- Files: PDF with `pdf-extract`, Office with `calamine` (xlsx/xls) and `zip`+`quick-xml` (docx/pptx). Extraction
  **runs in an isolated process**, with a memory limit and a timeout: a 149 KB PDF took 2.5 GB RSS in both readers
  tested. A PDF with only images returns empty and the turn answers "não consegui ler" (the phrase the code emits,
  kept in Portuguese); `.doc`/`.ppt` are refused.

## Pending decisions (they do not block the code)
- Use of the ARM64 test machine for the ARM test: ask for authorization in HR-56.
- ~~Dark background token of the Console (`#151719` in SofaUI × `#121a24` in the old instruction)~~ —
  resolved in HR-37: `#151719` is the canonical one and `#121a24` is revoked (see D-11).

### D-10 — TLS in the provider client and the check's ARM coverage in the workstation
- The provider client (HR-10) was accepted **without TLS**: the antigravity gateway runs on `http://127.0.0.1`, and
  `https` is refused with an explicit error. This holds only until then: **before HR-26** (Evolution) and
  **HR-28** (Jev, https), the client starts using `rustls` — both integrations talk https to the outside world.
  **Done in HR-10b** (see D-16): both clients already speak https. HR-26 (Evolution) uses the same path; whoever
  opens HR-26 no longer needs to enable TLS.
- **`cargo check --target aarch64` on the workstation:** only valid for crates with pure code. Anything that
  compiles C (embedded SQLite from `rusqlite`, `ring`/`aws-lc` for TLS, `onig` for the tokenizer) fails for lack of
  a cross C compiler — this is not a defect in our code. The real ARM check is HR-56, on the ARM64 test machine,
  with the same commands. **Maintainer decision (19/11/2026, 19:49):** do not install the ARM cross compiler on the
  workstation; the full ARM check stays for HR-56, on the ARM64 test machine. The local CI does not gate on it.
- **Limit of simultaneous LLM calls per org:** uses the keys the config already has (`limites`), with no new key;
  the link with the turn is in HR-13.

### D-11 — Console: `#151719` background, fonts in `/res/fonts` and `body` on the host
Confirmed in HR-37 (accepted on 19/11/2026). The canonical dark background of the Console is the `--pop-bg` token
(`#151719`) from SofaUI/DESIGN.md; the mention of `#121a24` in an old instruction is **revoked**. The matching light
theme is `#F5F6F7`. The host binary serves `IBM Plex Sans`/`IBM Plex Mono` statically at `/res/fonts/` and sets
font, color and background on the `body` selector; the library is not changed to a relative path (HR-05 → HR-38).
Intent-based routing stays out of the interface (D-06).

### D-12 — TLS came in HR-15 (model download), not only in HR-26/28
D-10 planned TLS only before HR-26/28. `poph model install` needs to download over https, so `poph-rag` already uses
`rustls`. `ort` was configured with `tls-rustls` instead of the default `tls-native` feature, which pulled in
OpenSSL (banned in `deny.toml`); OpenSSL does not go into the binary, checked with `ldd`. A **narrow** exception
remains in `deny.toml` for `webpki-roots` (CDLA-Permissive-2.0), which is only a build dependency of the ONNX
Runtime download and does not go into the binary. The LLM client stayed without TLS until **HR-10b**, which enabled
it in both clients (D-16).

### D-13 — Embedding batch: groups in the queue, runs one chunk at a time in the model
Dynamic int8 quantization computes the scale over the tensor of the whole batch: the same text inside a batch came
out with cosine 0.991 against itself alone (0.991 < the 0.9999 required in HR-15). With one chunk at a time, cosine
returns to 1.0000000 and there is no measured cost (12.9–13.8 ms per chunk in a batch of 32 against 14.6 ms in the
spike). The queue keeps grouping (32 chunks / 10 ms), and the model is called per chunk.
Consequence for HR-16/HR-17: queries and documents use the same path, so the vectors compare.

### D-14 — RAM ceiling of the embedding model in V1: 470 MB
Measured in release, isolated process, 3 rounds: 458 MB RSS after loading (+447 MB over an 11 MB base), against
438 MB in the HR-02 spike (+4.6%). The difference was not investigated and does not block; I adopt **470 MB** as the
reference ceiling in V1 and keep the measurement in the HR-15 report. Review if HR-55 (load) or HR-59 shows memory
pressure on the 12 GB ARM64 test machine.

### D-15 — RAG relevance cutoff starts at 0 (parity), reviewed in HR-59
HR-17 delivers `rag_min_score` configurable per agent, with default **0** — the same behavior POP Atende has today.
With 0, the top-k comes back even without relevance: in HR-17's demo, 3 chunks came back where only 1 was relevant. I
keep 0 for two reasons: (a) declared parity with POP Atende is a PRD criterion, and changing the default changes what
the end customer receives without measurement on the real base; (b) the HR-17 corpus is synthetic (24 items), and a
cutoff calibrated on it could block a good chunk on the Exemplo base. Calibrating the default is an HR-59 item, with
the real base; until then the cutoff is adjusted per agent.

### D-16 — TLS in the provider and Jev clients (HR-10b) and what the ARM check stopped covering
Done in HR-10b (20/11/2026). The two HTTP clients (`poph-core` for the provider, `poph-jev` for Jev) now speak
`https` with the `rustls` feature of `reqwest` — the same one used for the model download in `poph-rag`, with the
**system trust roots** (rustls-platform-verifier) and without OpenSSL, which stays banned in `deny.toml`. `http://`
is still accepted (local gateway).
- **A TLS failure is definitive**: an untrusted certificate, a server name that does not match the certificate, or a
  refused handshake become their own errors (`ErroLlm::Tls` / `JevError::Tls`), with no retry, carrying the cause's
  message (e.g. `invalid peer certificate: UnknownIssuer`). There is no public option to "ignore the certificate";
  the test uses an extra root, never disabled validation.
- **What the workstation's `cargo check --target aarch64` stopped covering:** `poph-core` and `poph-jev` were
  removed from the step, because TLS brings in `aws-lc-sys`, which compiles C — the cross build fails on the
  workstation for lack of `aarch64-linux-gnu-gcc` (`error occurred in cc-rs: failed to find tool
  "aarch64-linux-gnu-gcc"`), the same reason as D-02/D-10. Still in the check: `poph-types`, `poph-files`,
  `poph-plugins` and `poph-plugin-eco`. The real ARM proof for these two crates (and for the whole binary) is the
  build on the ARM64 test machine, in HR-56; the local CI does not gate on it.

### D-17 — Suspended tasks (21/11/2026, maintainer)
HR-29c (new round of G1 adjustment and measurement), HR-52 (importer of an agent from POP Atende), HR-55 (load test)
and HR-59 (end-to-end acceptance) were suspended to prioritize new development before the final tests. The
documentation was kept and no work in progress was interrupted. The state of each task is in the project's internal
tracking. While HR-55 is suspended, the D-07 targets have no measurement at scale.

### D-18 — Console organization table columns (21/11/2026, maintainer)
Node/Port, Agents and WhatsApp, planned in the approved design, stay **out** for now: they depend on the API
returning this data in the org listing. No task was opened.

### D-19 — Jev gates by default (20/11/2026, maintainer)
On by default: G3, G5 and G8. Off: G1, G2, G4, G6 and G7. A health risk does **not** turn into an unconditional
transfer: G1 depends on the policy configured by the company and stays off until the adjustment round and new
measurement (HR-29c, suspended). There is no per-organization gate adjustment: the global one (`poph.toml`) or the
per-agent one applies. Details in `JEV.md`.

### D-20 — Model failure, long text and backup (20 and 21/11/2026, maintainer)
- Model failure in the middle of a turn: the message is repeated later, with a limit on attempts
  (`tentativas_falha_modelo`, default 3); it does not warn the customer and does not leave the message without a
  reply. `TURNO.md`.
- A reply longer than 4000 characters on WhatsApp is cut with `…` in a single message, never split.
  `EVOLUTION.md`.
- Importing a backup that would overwrite data already present in the org is refused by default and requires explicit
  confirmation. `BACKUP.md`.
- WhatsApp: HR-26 closes without the test account; the test with a real number stays as a pending item, not as a blocker.

### D-21 — Matrix as a Harness module (21/11/2026, maintainer) — direction, no specification
Bring Matrix into the Harness as a removable module, in a separate crate, with few changes to the core, and create a
second integration role (observer of the conversation). Nothing specified or implemented yet; the survey of what is
missing is in `CONTRATO_CANAL.md` §7.

### D-22 — Flow language: Starlark (21/11/2026, maintainer, decided by measurement)
Flows are scripts in Starlark (crate `starlark`). The spike (FX-01) met every criterion within the proposed limits, so
the Rhai fallback was not used. Limits in force: 32 KB of source (`max_fonte_bytes`), 60 nodes (`max_nos`). An
allocation such as `"x" * 10**9` is stopped by the time limit, not by the memory limit, because it reaches about 2 GB
first. Details: `FLUXOS_LINGUAGEM.md`.

### D-23 — One Case per conversation, created with it (21/11/2026, maintainer)
Every conversation has exactly one Case, created in the same write as the conversation (unique index). It closes with
the conversation, in the same transaction. A transferred conversation keeps its Case open. A closed conversation is
not reopened (409 `conversa_encerrada`), so the customer's next message opens a new conversation and a new Case.
Concluding a flow does not close the Case. Details: `FLUXOS.md` §2.

### D-24 — One active execution per Case; flows can follow one another (21/11/2026, maintainer)
A partial unique index allows one active execution per Case. A second flow starts only after the first one ends.
Details: `FLUXOS.md` §3.

### D-25 — Permission per agent: explicit list, denied by default (21/11/2026, maintainer)
A new flow is used by no agent until someone grants it to that agent. There is no "grant to all agents" option yet.

### D-26 — Immutable revisions; an execution stays on the revision it started with (21/11/2026, maintainer)
Publishing creates a numbered revision that never changes: a database trigger refuses UPDATE and DELETE. Each
execution is pinned to the revision it started with, so a new publication does not change it. Details: `FLUXOS.md` §3.

### D-27 — The code starts flows; the Jev chooses (G9); the model is the fallback (21/11/2026, maintainer)
The turn validates the Jev's choice and opens the execution before the text model answers. If the Jev fails, the model
gets the `fluxo_iniciar` tool (reserve). `nenhum` or low confidence starts nothing. Details: `JEV.md` §5-I.

### D-28 — Effects leave the transition and are delivered by a queue, at least once (21/11/2026, maintainer)
A failed effect does not undo the transition: it becomes the `efeito_falhou` event in the Case, and the attendance goes
on. Delivery is at least once: a crash between sending and recording can repeat an effect, which is accepted and
documented. Details: `FLUXOS.md` §4.

### D-29 — Status of decisions L1 to L13 (PRD §3.2) (23/11/2026)
All thirteen are implemented: L1 (Starlark), L2 (pure `poph-fluxos` crate, depending only on `poph-types` and the
script library), L3 (built-in engine, Case always on, `[fluxos] ativo`), L4 (the one-Case invariant lives in the store),
L5 (one attendance = one conversation = one Case), L6 (one active execution per Case), L7 (concluding a flow does not
close the Case), L8 (permission by explicit list), L9 (immutable revisions), L10 (the code starts the flow, decided by the
Jev; reserve tool), L11 (effects through a queue, failure does not undo the transition), L12 (retention does not delete
the Case) and L13 (reopen of a closed conversation returns 409). None was removed.

### D-30 — Decisions still open for the maintainer (PRD §15) (23/11/2026)
The defaults below are in force. The maintainer confirmed items 1, 2, 4 and 6 on 23/11/2026 (D-31); items 3 and 5
were not decided by him and stay in force by default:

1. Customer's farewell (G8) does not close the attendance; the agent closes it with `atendimento_encerrar`.
   **Confirmed by the maintainer on 23/11/2026 (D-31).**
2. Cases are never deleted; there is no retention for Cases yet. **Confirmed by the maintainer on 23/11/2026 (D-31).**
3. No "grant to all agents" option; permission is per agent. *Not decided by the maintainer (D-31); in force by default.*
4. `notificar_whatsapp` without a WhatsApp integration stays `pendente`. The PRD proposed `falhou` after the attempt
   limit; the implementation does not do that. **Confirmed by the maintainer on 23/11/2026 (D-31).**
5. The cap of 20 flows per agent is implemented. It is a proposal from the PRD, not a decision.
   *Not decided by the maintainer (D-31); in force by default.*
6. Numeric targets (PRD §5 and §9) are proposals. The two measured D-07 targets are **official since 23/11/2026 by the
   maintainer's decision (D-31)**; the FX-01 target (≤ 300 KB per loaded flow) stays a proposal. The measurements are in
   `FLUXOS.md`, section Measurements.

### D-31 — Decisions confirmed by the maintainer (23/11/2026)
The maintainer answered the items of D-30 on 23/11/2026: four are confirmed as already implemented and two stay in force
without his decision. No code, default or configuration changes; the behaviour he confirmed is the one that was already
running.

1. **Customer's farewell (G8) does not close the attendance** (D-30 item 1). The agent closes it with
   `atendimento_encerrar`. Evidence: FX-25 and the flow proof (`FLUXOS.md` §7).
2. **Cases are never deleted; there is no retention of their own for Cases** (D-30 item 2). The retention job does not
   touch them. Evidence: FX-34, `RETENCAO.md`.
3. **`notificar_whatsapp` without a WhatsApp integration stays `pendente`** (D-30 item 4): the effect does not move to
   `falhou` after the attempt limit. Evidence: FX-21, `FLUXOS.md` §4.
4. **The measured numeric targets are official** (D-30 item 6): message write throughput **≥ 50,000 msg/s** and **p95 ≤
   5 ms** at 2,000 msg/s. Evidence: FX-36 — 57,075–66,832 msg/s with 32 and 256 writers, p95 96–115 µs; numbers in
   `FLUXOS.md`, section Measurements. The FX-01 target of **≤ 300 KB per loaded flow remains a proposal**, and the rest
   of D-07 (12 KB per idle agent, 150 MB with 10,000 agents, ARM64) remains **unmeasured**.
5. **G9 stays on by default, with confidence 0.70 and margin 0.10**, kept after the measurement against the real Jev
   (FX-37). Numbers, false positives/negatives, abstention and latency: `JEV.md` §5-I. The default in the code is
   unchanged.

Not decided by the maintainer (in force by default, as in D-30): there is no "grant to all agents" option — permission
is per agent — and the cap of 20 flows per agent is implemented.

### D-32 — Code in English, no comments; product text in Portuguese (23/11/2026, maintainer)
The maintainer decided on 23/11/2026 that the repository moves to code in English with no comments, without changing
any external contract. The rule lives in `AGENTS.md` ("Rules that apply here") and revokes the previous one, which
kept identifiers and comments in Portuguese. It bites at two moments, the comment cleanup and the identifier
translation, each with its own CI gate.

- **Comment.** None in code or configuration. The only exceptions are the shebang in the first line of a script and
  a directive that breaks the tool if it goes (`# noqa`, `# type: ignore`, coverage ignore markers and the like),
  each one in the versioned allowlist `scripts/allow-comments.txt`, one line per exception with the reason. A
  comment that carries a rule is not simply deleted: the rule moves to the matching document in `docs/`.
- **Product text stays in Portuguese**: error messages, log message text, the Jev and agent prompts, the Console UI
  and test data. The `tracing` key is an identifier and becomes English; the message text does not change.
- **Identifier.** Internal names become English. Published names keep today's value through an explicit attribute:
  tables, columns, indexes, triggers, routes, `operationId`, JSON fields, `poph.toml` keys, CLI flags and command
  names, environment variables, metric names, plugin protocol terms, the words of the flow language and the IR
  stored in the database.
- **Migration.** A published migration may lose its comments and nothing else: file name, tables, columns, indexes
  and the SQL stay. The runner stores version and file name, with no checksum of the content
  (`crates/poph-store/src/migrations.rs`), so a database already migrated sees no difference; the proof is the schema
  of a database migrated from scratch and of an old one, identical before and after.

Why: the code has to be readable by any developer, and the repository is public.

The two gates that enforce it are part of the same refactor. The comment check (`scripts/check-comments.sh`, with
the allowlist and a by-module pending list) enters after the comment remover and before the first module is cleaned,
so a comment cannot come back. The identifier check (`scripts/check-names.sh`, with the glossary as its dictionary
and its own allowlist) enters before the first module is renamed.

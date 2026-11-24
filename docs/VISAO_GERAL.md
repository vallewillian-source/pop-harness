# POP Harness (Rust): overview

> Starting point for the project. It says what the Harness is, what is in and out of scope, the base decisions and
> the path of a message from end to end. What is missing for `1.0` is in [`../ROADMAP.md`](../ROADMAP.md).
> The channel contract is in [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md). The identifiers `HR-xx`, `RL-xx`,
> `MX-xx`, `FX-xx` and `RP-xx` that appear in comments and documents are task numbers from the project history:
> the commit that closed the task cites them. The original specification (PRD and task list) is not part of this
> repository; what was valid in it is here, in `DECISOES.md` and in the docs by topic.

## 1. What it is

A multi-tenant AI customer-service engine, in Rust: one binary (`poph`) that runs the agents of several
organizations on the same instance. It has three entry points: the CLI, the administrative API (`/admin/v1`) and the
web Console (React, embedded in the binary), used by the internal team to administer and test.

It is independent of POP Atende: it does not use its Postgres, does not import its code and does not replace its
engine (switching the LangGraph engine will only be discussed after the Harness is stable).

Priorities, in this order: (1) deliver the message as fast as possible; (2) low RAM usage; (3) vertical scale
(thousands of agents) and horizontal scale (several instances); (4) minimalism (few dependencies, few processes, no
agent framework).

## 2. What is inside

- Orgs, product categories, agents, departments (with a default department), structured instructions, knowledge
  base, products with a quote cart, the agent's file library and integrations.
- Own search (RAG): local embedding, hybrid search (vector + text) with a relevance cut.
  [`BASE_DE_CONHECIMENTO.md`](BASE_DE_CONHECIMENTO.md).
- Jev (quick decisions) at 9 points, G1 to G9 (G9 starts flows), each behind a flag. [`JEV.md`](JEV.md).
- LLM behind an interface; today one provider, compatible with the OpenAI chat API, with the model configurable per
  agent.
- Channels as integrations with a single contract: WhatsApp through EvolutionAPI (native module) and a plugin running
  in an external process. [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md), [`PLUGIN_PROTOCOLO.md`](PLUGIN_PROTOCOLO.md).
- Files: images and PDF/Office (as text) received; files from the agent's library sent.
  [`ARQUIVOS.md`](ARQUIVOS.md).
- Local history with configurable retention, org backup/restore, static routing between instances.
  [`RETENCAO.md`](RETENCAO.md), [`BACKUP.md`](BACKUP.md), [`CLUSTER.md`](CLUSTER.md).
- Cases and flows: every conversation starts with a Case, which closes with the conversation. Flows are Starlark
  scripts, owned by the org, with immutable revisions and per-agent permission. The engine runs in the turn, before the
  text model. [`FLUXOS.md`](FLUXOS.md), [`FLUXOS_LINGUAGEM.md`](FLUXOS_LINGUAGEM.md).
- Matrix and human handoff: a removable module (`poph-matrix`) that mirrors the conversation in an Element room,
  receives the reply from the human operator and pauses the agent. [`MATRIX_AMBIENTE_LOCAL.md`](MATRIX_AMBIENTE_LOCAL.md),
  [`DADOS.md`](DADOS.md) §11.

## 3. What is outside

PDF documents and templates; calendar; e-mail; website widget; persistent customer profile (memory is the
conversation window with a rolling summary); audio and video transcription; OCR of scanned PDFs; WASM plugins;
intent-based routing rules (D-06); visual flow editor (the Console does not change for flows); a retention policy for Cases beyond the current one (Cases are never deleted for now). See also the known gaps
in [`../ROADMAP.md`](../ROADMAP.md).

## 4. Base decisions (D1 to D13)

These decisions came from the original specification and still hold. Later ones are in [`DECISOES.md`](DECISOES.md)
(D-01 onwards, with a hyphen so they are not confused with these).

| # | Decision |
|---|---|
| D1 | Rust; Cargo workspace at the root of the repository, `console/` next to it; binary `poph`. |
| D2 | Integration plugins: native and external process (JSON-lines over stdio). WASM is left for later. |
| D3 | Conversation and message history local, in SQLite, with retention configurable per org (default 30 days). |
| D4 | Static routing per org between instances. |
| D5 | Customer files: images and PDF/Office as text. No audio. |
| D6 | LLM: antigravity2api, model `gemini-3.8-flash-tiered` by default, configurable per agent. |
| D7 | Embedding: same model as POP Atende (`paraphrase-multilingual-MiniLM-L12-v2`), local. |
| D8 | Jev decides what is a choice among known options; the LLM only writes text. Jev chooses, ranks or abstains; the code validates and executes; a Jev failure falls back to the behavior without it. |
| D9 | Internal administrative Console, with token login (global or per org). |
| D10 | No Redis, no Postgres, no agent framework. |
| D11 | A message to the customer is delivered at least once: if sending fails midway, it is resent, accepting a duplicate. |
| D12 | A conversation transferred to a human stops being answered by the agent until it is reopened; customer messages keep being recorded. |
| D13 | The agent only sends files from the agent's library, never free paths. |

## 5. One message, end to end

Details in [`TURNO.md`](TURNO.md) and [`ENTRADA_E_SAIDA.md`](ENTRADA_E_SAIDA.md).

1. **Input.** The channel (Evolution webhook, plugin, Console chat or CLI) delivers the message. It is saved in the
   `inbox` with a unique key per channel and external id (duplicates are discarded), and only then does the webhook
   receive OK. If the org belongs to another instance, the request is forwarded to its owner.
2. **Queue per conversation.** One actor per conversation, one turn at a time. An idle conversation leaves memory.
3. **Grouping window.** A short wait (default 2 s, cap 8 s) to group messages sent in sequence; Jev's G2 can shorten or
   lengthen it.
4. **In parallel:** one call to Jev with the questions of all enabled gates, and the search in the knowledge base and
   in the products. If G1 says transfer, the turn ends without calling the LLM.
5. **Agent loop.** Prompt with a stable prefix (for the provider's cache) and a volatile block after it; LLM with
   streaming; tools enabled for the agent (`consultar_base`, `buscar_produto`, `enviar_arquivo`, cart). File text and
   retrieved chunks come in marked as data, never as instructions.
6. **Output.** The reply goes to the `outbox` (idempotent per turn and sequence); the channel sends it through the same
   integration it came from. States: `pending`, `sending`, `sent`, `failed`.
7. **Record.** The turn saves Jev's decisions, retrieved chunks, tools, times and tokens; the Console shows this in the
   "why" panel.

## 6. Data and isolation

Memory (hot state, lost on restart), `harness.db` (durable SQLite), `rag.db` (rebuildable SQLite) and blobs on disk per
org and hash. Every business table has `org_id` and every access filters by it; the vector index and caches are
partitioned by org. Details in [`DADOS.md`](DADOS.md).
Secrets are encrypted at rest, come only from the environment, and never appear in logs. Admin tokens are stored as
hashes, with global or org scope.

## 7. Console: design rules

They apply to any new screen ([`CONSOLE.md`](CONSOLE.md), [`../console/UX_SPEC.md`](../console/UX_SPEC.md)): no cards,
and never a card inside a card (sections separated by a title and a 1 px line); background `#151719`; amber accent
only on the primary action, the selected row and the active item; status as a 6 px dot plus text; slugs, ids, ports
and commands in monospace font; visible focus and keyboard use; empty, loading and error states on every screen.
Uppercase labels and counters in the menu follow the approved layout design.

## 8. Risks still open

| Risk | Status |
|---|---|
| Behavior parity with the LangGraph engine | Not measured with a real agent. POP Bench can serve as a base. |
| RAM and throughput with thousands of agents | Targets set in D-07; the load test (HR-55) is suspended, so nothing was confirmed at scale. |
| ARM64 build | Validated on the ARM64 test machine at a commit from 20/11; later builds were not rebuilt there. |
| One SQLite writer per instance | Measured in the spike; no full load test. |
| `ort` still a release candidate | Version pinned; the change is isolated behind the embedder interface. |
| Real WhatsApp | Proven only with a fake Evolution and an isolated environment; test with a real number pending (does not block). |

## 9. Where each subject is

| Subject | Document |
|---|---|
| What is missing for `1.0` | [`../ROADMAP.md`](../ROADMAP.md) |
| Channels and integration contract | [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md) |
| Later decisions | [`DECISOES.md`](DECISOES.md) |
| Configuration and variables | [`CONFIGURACAO.md`](CONFIGURACAO.md) |
| Data, schema, concurrency | [`DADOS.md`](DADOS.md), [`MEMORIA_E_CONCORRENCIA.md`](MEMORIA_E_CONCORRENCIA.md) |
| Administrative API | [`API.md`](API.md) |
| Operations | [`RUNBOOKS.md`](RUNBOOKS.md), [`../DEVELOPMENT.md`](../DEVELOPMENT.md) |
| Code map and working rules | [`../AGENTS.md`](../AGENTS.md) |

## 10. Old references ("PRD §x", "D6", "HR-xx")

Comments in the Rust code and some docs still cite sections of the removed PRD. Equivalence:

| Citation | Where it is now |
|---|---|
| §3 (scope) | sections 2 and 3 of this document |
| §4 (decisions D1–D13) | section 4 of this document |
| §5.2, §5.3, §5.4 (crates, dependencies) | [`../AGENTS.md`](../AGENTS.md), [`../DEVELOPMENT.md`](../DEVELOPMENT.md), `DECISOES.md` D-08 |
| §6, §6.1, §6.3 (turn, prompt, window) | [`TURNO.md`](TURNO.md) |
| §7, §7.1, §7.2, §7.3 (data) | [`DADOS.md`](DADOS.md) |
| §7.4 (retention) | [`RETENCAO.md`](RETENCAO.md) |
| §8.1 (API) | [`API.md`](API.md) |
| §8.3 (integration contract) | [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md) |
| §8.4 (LLM provider) | [`CONFIGURACAO.md`](CONFIGURACAO.md) (`llm.*`) |
| §8.5, §10 (Jev) | [`JEV.md`](JEV.md) |
| §9 (RAG) | [`BASE_DE_CONHECIMENTO.md`](BASE_DE_CONHECIMENTO.md), `DECISOES.md` D-09 |
| §11 (files) | [`ARQUIVOS.md`](ARQUIVOS.md) |
| §12 (cluster) | [`CLUSTER.md`](CLUSTER.md) |
| §13 (Console) | [`CONSOLE.md`](CONSOLE.md) and section 7 of this document |
| §14, §15 (security, observability) | section 6 of this document, [`CONFIGURACAO.md`](CONFIGURACAO.md), [`RUNBOOKS.md`](RUNBOOKS.md) |
| §16 (risks) | section 8 of this document |
| §17 (acceptance criteria) | [`../ROADMAP.md`](../ROADMAP.md) |
| HR-xx, RL-xx, MX-xx, FX-xx, RP-xx | commit that closed the task (`git log --grep`) |

The same comments cite `PARIDADE_AGENTE.md` (what POP Harness inherited from POP Atende), `ESTADO.md` (internal task
list) and `spike/` (measurement prototypes). None of the three is published; the content that matters to users of the
project is in the documents in this folder.

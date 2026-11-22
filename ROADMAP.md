# Roadmap

This file says what the project prioritises and what is out of scope. It changes with the maintainers'
decisions and is not a promise of dates. To propose something, open a proposal issue.

## In development (status as of 2026-11-22)

These items have a settled design and appear in the README as part of the product, but are not in the
binary yet:

- [ ] **Flows:** `poph flow` (validate, publish, diff, format), effects, revisions, the `fluxo_iniciar` tool
      and the G9 gate. The language is in [`docs/FLUXOS_LINGUAGEM.md`](docs/FLUXOS_LINGUAGEM.md).
- [ ] **Cases:** a Case is already created for each conversation and closed with it; still missing are
      `poph ficha`, listing, and the flow history as a sub-item.
- [ ] **Matrix, phase 2:** images and files in both directions, messages from the device, and expiry.
- [ ] **Matrix, phase 3:** several instances on the same Synapse.
- [ ] **Console:** Matrix, flows and Cases screens (today they appear as "coming soon" in the menu).
- [ ] **Console outside the monorepo:** publish or include the `@sofacoding/sofa-ui` package.

## Before `1.0`

What is missing for the project to be recommendable for production by anyone:

- [ ] **Full load test.** 10,000 idle agents and 500 active conversations, on x86_64 and ARM64, with a
      published "target × measured" table. Today the targets are fixed ([`docs/DECISOES.md`](docs/DECISOES.md), D-07)
      and there are only partial measurements.
- [ ] **Reproducible benchmark** of latency and memory against alternatives, with public method and scripts.
- [ ] **WhatsApp with a real number.** Today the channel has been proven only against a fake EvolutionAPI.
- [ ] **Generic model provider.** Replace the `antigravity2api` identifier with a documented
      OpenAI-compatible provider tested against more than one endpoint, and document how to write another
      provider.
- [ ] **Swappable Jev decision.** Today the Jev is an optional third-party service. Allow another decision
      service behind the same interface, with the same principle (the service chooses, the code validates).
- [ ] **External plugin as a channel in the server.** Wire the contract and the supervisor, which are
      ready, to message receiving and sending in `poph serve`.
- [ ] **Search parity measured with a real agent**, not just a synthetic corpus, and calibration of the
      default relevance cut-off.
- [ ] **Container image** and an ARM64 package rebuilt on every release.
- [ ] **Person login in the Console**, roles and a per-token rate limit.

## Later

Ideas that fit the project, in no order and with no date:

- Retention policy for Cases.
- Visual flow editor in the Console (the script format was designed so it can edit without losing anything).
- Audio and video transcription.
- WASM plugins.
- New channels as plugins: email and a website widget.

## Out of scope

Things we decided **not** to do, so nobody spends time on them:

- A generic agent framework. Harness is a customer-service engine, not an orchestration library.
- Redis, Postgres or any other mandatory service. The installation stays a binary and files.
- Intent-routing rules in the core (handoff goes to a department).
- Compatibility with the flow-editor JSON of other products.

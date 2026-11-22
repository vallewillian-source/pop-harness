# Changelog

All notable changes to this project are recorded in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/). In the `0.x` series any version may bring an incompatible
change; it is always marked **Breaking** in the notes.

## [Unreleased]

### Added

- Multi-tenant customer-service engine in Rust: orgs, agents with structured instructions, departments,
  categories, products with a quote cart, and a per-agent file library.
- Knowledge base with hybrid search (vector and text) and local embeddings in ONNX (int8).
- The Jev, with eight fast-decision gates (G1 to G8), measured against labelled sets, and G9 to start flows.
- WhatsApp through EvolutionAPI as a native channel, and a channel contract for external processes
  (JSON-lines over stdio).
- Flows written in Starlark, with immutable revisions, per-agent permission and one Case per conversation.
- Optional Matrix module: one room per conversation, spaces per department, agent replies in Element and
  pausing the AI agent.
- Admin API at `/admin/v1` with OpenAPI 3.1, embedded web Console and the `poph` CLI.
- Static routing between instances with signed forwarding, org backup and restore, history retention,
  Prometheus metrics and `poph doctor`.
- Community files: licence, contributing guide, code of conduct, security policy, issue and pull request
  templates.

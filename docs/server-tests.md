# Server tests: what each file proves

This document covers the integration tests in `crates/poph-server/tests/`. The explanatory comments that used to sit at the top of these files and inside them were moved here, so the test source keeps no comments. Each section lists what the file checks, which test checks it, and the notes that were in the comments.

Where a comment made a claim that the test does not fully check, the section says so under "Gaps". Nothing was added to or removed from the tests themselves.

## Renamed files

| Previous name | Current name |
| --- | --- |
| `teste_chat_encaminhado.rs` | `forwarded_chat_test.rs` |
| `produto_exclusao.rs` | `product_deletion.rs` |
| `orgs_agentes_ativos.rs` | `orgs_active_agents.rs` |
| `resiliencia_queda.rs` | `crash_resilience.rs` |
| `conteudo_arquivo.rs` | `file_content.rs` |
| `manifestos_plugins.rs` | `plugin_manifests.rs` |
| `teste_chat.rs` | `test_chat.rs` |

`console.rs`, `site_chat.rs`, `cluster_reload.rs` and `backup_http.rs` keep their names.

## `site_chat.rs`

The original file had no header comment and no inline comments. Its tests are:

- `two_visitors_exchange_messages_with_isolated_history_and_replays`
- `overlapping_turns_cancel_and_resume_without_owning_the_subscription`
- `restart_marks_interrupted_turns_failed_and_processes_queued_turns`
- `tool_policy_and_request_limits_are_enforced`
- `configured_http_tool_carries_site_context_and_replays_without_repeating_effects`
- `measure_site_fake_model` and `measure_site_real_model` are latency measurements marked `#[ignore]`. They do not run in the default test suite.

## `forwarded_chat_test.rs`

Covers server-sent events (SSE) across forwarding between two instances (HR-50b-2).

Setup: two real instances on ephemeral ports, each with a temporary database. Both sides use the same instance map, and only `instance_id` differs. The organization and the agent live in instance `b`. Instance `b` runs the turn on a fake provider that delivers a piece of text and then stays open for 30 seconds.

Test: `sse_from_remote_org_flows_through_forwarding_and_disconnect_cancels_on_owner`.

What it checks:

- The conversation and the message sent through `a` are served by `b`. The conversation creation through `a` returns 201, and the organization does not exist in `a`'s database.
- The SSE stream is not buffered by forwarding. The first event must arrive within 5 seconds and the first model text within 10 seconds, long before the 30-second turn ends. No end-of-turn event may arrive while the turn is still open.
- When the client disconnects in the middle of the stream, the owner (`b`) cancels the turn. The turn ends in state `failed`, its error mentions cancellation, no outbound message is stored in the conversation, and `b`'s queue is empty afterwards.

Gaps:

- The original header said the first event and the first text arrive "in milliseconds". The test enforces the limits of 5 and 10 seconds above, not milliseconds.
- The original header said nothing from the turn stays in `a`'s database. The test only checks that the organization is absent from `a`. It does not check the turn rows in `a`.

Notes from the removed comments:

- The two listeners are bound before the instance maps are built, because each map needs the other side's address.
- `start_instance` returns the instance and its store. The turn engine exists only in `b`, which owns the organization.
- The conversation is created through `a`, so the POST is forwarded. The later read of the turns from `b`'s store uses that conversation id.

## `console.rs`

Covers the Console served by the binary (HR-38). The server runs on an ephemeral port with a temporary database, and the test uses a real HTTP client.

| Test | Checks |
| --- | --- |
| `root_serves_console_or_notice_page` | `/` and the client routes serve the Console page (single-page app). |
| `client_route_falls_back_to_index` | Client routes return the page. |
| `missing_asset_is_not_page` | A missing asset returns 404 as JSON, never HTML with the wrong type. |
| `server_routes_are_not_shadowed` | `/admin/v1`, `/healthz`, `/metrics` and `/webhook` keep their own format. An unknown route under the API returns a JSON error. |
| `non_get_method_does_not_get_console` | A POST to `/orgs` returns a 404 JSON response, not the Console page. |
| `sofa_ui_fonts_come_from_binary` | The SofaUI fonts are served from `/res/fonts` when the Console bundle is in the binary. |
| `security_headers_accompany_page` | The page carries the security headers and `Cache-Control: no-cache`, so a new deploy is not stuck in the browser cache. |
| `head_returns_same_headers_without_body` | HEAD returns the same headers as GET, with no body. |

Gaps:

- The font test depends on `has_bundle()`. When the binary has no bundle, the test only checks the 404 JSON branch. The WOFF2 signature check runs only when the bundle is present.

Notes from the removed comments:

- Each scenario gets its own folder. A process-wide atomic counter, together with a timestamp, keeps two scenarios created at the same instant from sharing a folder. The coarse clock under load made this necessary, which caused "database is locked" failures. No test checks this directly; it is a design note of the helper.

## `product_deletion.rs`

Covers deleting a product with `force` (HR-42f).

A cart item points to its product. Deleting the product clears that reference (`ON DELETE SET NULL`). Without `force` the API must refuse with 409 and report the counts. With `force` the product is deleted and the cart item stays, with the price stored at the time it was added.

Setup: real server, temporary database, ephemeral port. No test touches the external network.

Tests:

- `product_in_cart_without_force_answers_409_and_keeps_it`
- `with_force_deletes_and_item_keeps_stored_price`
- `product_without_cart_deletes_without_force`
- `token_from_other_org_does_not_delete_and_missing_product_answers_404`

What the 409 test checks: two cart items in two carts with the same product report `itens_de_carrinho = 2` and `carrinhos = 2`. The product still exists, and both cart rows keep the product name.

Gaps:

- The admin API has no cart route, so the helper `add_to_cart` writes through the store, the same path the turn uses. Adding to a cart through the API is therefore not exercised here.

Notes from the removed comments:

- Products need a category. The helper creates one per product, with a name that is unique within the organization.

## `orgs_active_agents.rs`

Covers the `agents_active_count` field in the organization listing (CF-03a), which is the "active agents" column of the Console. The server runs on an ephemeral port with a temporary database, and the test uses a real HTTP client.

Tests:

- `listing_shows_active_agents_per_org`
- `cursor_pagination_stays_the_same`
- `page_makes_no_per_row_query`

What they check:

- The field is always present in the listing without a cluster, and equals the number of agents with status `active`. An inactive agent does not count.
- An organization with no active agent returns `0` and is not dropped from the list.
- Cursor pagination is unchanged, and every page carries the field.
- Reading one organization (`GET /admin/v1/orgs/{id}`) and listing with an organization token return the same number, with no extra field.
- A 12-organization page makes fewer than 12 database reads (`Store::reads_done`). The test checks that bound only.

Gaps:

- The original comment said a page costs "the same number of reads" whether it lists 1 or 12 organizations. The test does not assert that. It only asserts that the 12-organization page costs fewer than 12 reads, and the 1-organization count is only printed in the failure message.
- The fourth agent is made inactive through the repository, because the API route for changing status is `PATCH`, which requires `If-Match`. The inactive path through the API is therefore not checked in this file.

Notes from the removed comments:

- Changing the agent status through the API is a `PATCH` that requires `If-Match`. The test sets the status in the repository instead, because the removed comment said the API path was not the point of this check.

## `crash_resilience.rs` (was `resiliencia_queda.rs`)

Covers resilience when the process dies or the model provider fails in the middle of a turn (HR-57). Each test starts a real server on an ephemeral port, with the turn running on a fake LLM provider. No external network and no key is used. The database is temporary.

What it checks:

- A turn left `running` by a previous process becomes `failed` at startup, with a reason, and the conversation accepts the next message at once.
- The client's message is never lost.
- A provider failure (down, timeout, cut stream) ends the turn as `failed` with a reason, with no invented answer and no duplicated answer.
- A test turn reservation expires, so a conversation is not stuck in `409` forever.

Tests: `turno_interrompido_vira_falha_na_subida_e_a_conversa_aceita_a_proxima_mensagem` (now `turn_interrupted_becomes_failed_on_boot_and_the_conversation_accepts_the_next_message`), `modelo_indisponivel_deixa_turno_falho_sem_perder_a_mensagem`, `stream_cortado_no_meio_deixa_turno_falho_sem_resposta_parcial`, `saida_do_console_presa_em_sending_e_confirmada_na_subida`, `reserva_de_turno_de_teste_vence`.

Gaps:

- The header says the real `kill -9` run is a script at `/tmp/hr57-run/queda.py`. That script is not in the repository, and the directory does not exist on the current machine. The `kill -9` case is covered here only by simulating the leftover state in the database.

Notes from the removed comments:

- The "no batching window" line means the test does not wait 2 seconds per turn for the grouping window.
- Each boot-recovery test checks four things in order: the turn becomes failed with a reason; the client message is still stored; the conversation accepts a new message without a 409 left over from the crash; the new question is answered once, with no duplicated answer.

## `cluster_reload.rs`

Covers reloading the cluster map (HR-50a) end to end, with a real server and a real file.

What it checks:

- `POST /admin/v1/cluster/reload` with the global token re-reads `cluster.toml` from the running process. It returns a fingerprint before and after, a summary of both maps, and what changed.
- Editing the file changes the map. An organization that moves to another instance shows up in `orgs_que_trocaram`.
- An invalid file returns `400 cluster_invalido` and keeps the old map. A later reload with the good file confirms the old map was the one in use.
- No token returns `401`. An organization token returns `403`, because reload needs the global token.
- A missing file returns to single-instance mode.
- No response carries the shared secret, and the fingerprint does not carry it either.

Tests: `recarga_le_o_arquivo_do_processo_vivo_e_aponta_o_que_mudou` (now `reload_reads_the_live_process_file_and_reports_what_changed`) and `recarga_exige_token_global` (now `reload_requires_global_token`).

Gaps: none recorded in the header beyond the claims above.

Notes from the removed comments:

- Since HR-50b the map is applied for real: an organization missing from `[orgs]` goes to `default_instance`. This test creates an organization at run time to get an organization token, so the default is the local instance. Otherwise the request would be forwarded to `b`, which does not exist in this test.

## `file_content.rs` (was `conteudo_arquivo.rs`)

Covers file content served through the admin API (HR-46b). Two routes return bytes: the file in an agent's library, and an attachment of a message in the test chat.

What it checks:

- `200` with bytes identical to what was uploaded, SHA-256 checked on both sides, the stored MIME as `Content-Type`, and a `Content-Disposition` with a sanitized file name.
- `401` without a token. `403` with a token from another organization. `404` for an unknown id, for a file of another agent in the same organization, for an attachment outside its path (organization, agent, conversation, message), and for a blob missing from disk.
- No response carries the blob path (`data/blobs`, `<org>/<sha256>`). The metadata route still has no `blob_path`.

Tests: `biblioteca_conteudo_entrega_os_bytes_com_cabecalhos` (now `library_content_serves_bytes_with_headers`), `biblioteca_conteudo_isola_org_agente_e_id` (now `library_content_isolates_org_agent_and_id`), `anexo_conteudo_entrega_os_bytes_do_chat_de_teste` (now `attachment_content_serves_bytes_from_the_test_chat`).

Gaps: none recorded in the header beyond the claims above.

Notes from the removed comments:

- The PNG used in the tests is a minimal file (signature and IHDR), the same one the other tests use.
- The attachment helper uses the same path format the inbound receiver writes.

## `plugin_manifests.rs` (was `manifestos_plugins.rs`)

Covers the plugin manifest and the integration configuration and secret (HR-44c), end to end. The server runs on an ephemeral port with a temporary database and a real HTTP client. The fake plugin runs as an external process and speaks the protocol over stdio, the same path as `poph-plugin-eco`. There is no external network.

What it checks:

- The manifest route returns what the plugin declared in the handshake, and the plugin is not started again on each request (the fake plugin counts its starts). Test: `manifest_comes_from_handshake_and_is_cached`.
- A type with no declared plugin returns `404`. A declared plugin that does not answer the handshake returns `503`, and the body has no program path or system error. Test: `type_without_plugin_is_404_and_broken_plugin_is_503`.
- Configuration is validated field by field against the manifest schema: required field, type, enum, and a field outside the schema (`400`, with the field name). Test: `config_and_secret_are_validated_field_by_field`.
- The secret is stored encrypted: the data folder does not contain the value in clear text, the create response, the detail route and the list route never return it, and the process log never receives it. Test: `create_stores_config_and_encrypted_secret_without_exposing_it`.
- Without a manifest, a type with configuration is refused with `422`. A type without a manifest and without configuration is still created as before. Test: `config_without_manifest_stays_refused`.
- `PATCH` replaces the configuration, merges the secret, and requires `If-Match`: `428` without it and `412` with an old version. Removing the required secret is refused by the schema (`400`), and the stored state stays at the last valid write. Test: `patch_replaces_config_merges_secret_and_requires_if_match`.
- A token from another organization gets `403` and does not start the plugin. Test: `other_org_token_cannot_read_manifest`.
- The list route returns the declared types ordered by `kind`. The broken type is marked as unavailable, with no name or version, and the list still returns. Its message carries the operation phrase, and the body does not contain the program path, arguments or secret. A second listing reuses the cache and does not start the plugin again. Test: `list_returns_declared_types_and_flags_broken_one`.
- The list route requires a token and respects the token's organization (`403` for another organization). Test: `list_requires_token_and_respects_token_org`.
- The public manifest types serialize, so the screen and the CLI can use them from outside. Test: `public_manifest_types_serialize`.

Gaps:

- The `428` and `412` rules are checked only for `PATCH`. The creation route is not checked for `If-Match`.
- The order of the list is checked with two types only.
- The log check reads what this process wrote to its captured log. It does not check other log sinks.

Notes from the removed comments:

- The test token is global to the tests. The server stores only its hash.
- The cipher key is for the test only and never goes into a configuration file.
- The secret value is a fixed test value that must not appear in a response, in the database or in the log.
- The log capture helper exists to prove what does NOT leave the process.
- The fake plugin's manifest declares a configuration with a required field, an enum and a number, and a required secret. The fake plugin writes its own script, answers the handshake with the manifest, and counts its starts in a file.
- The data folder reader reads every file in the folder, not only `harness.db`, because SQLite writes to a WAL file first. Reading only the main file would give a false negative.
- The "broken" plugin is declared but does not speak the protocol. This is the fixture that proves the `503`.
- The Python script embedded in the fake plugin has one comment of its own, inside the fixture text. It was kept unchanged because it is part of the fixture content, not a comment of the test code.

## `backup_http.rs`

Covers the export and import routes of an organization (HR-48e-1), end to end: a real server on an ephemeral port, a database, a blob folder and a real HTTP client. The tests do not call the handlers directly.

What it checks:

- Export returns the same format as the CLI (`poph-backup-org` version 2, source organization, `tables`, `blobs`) as an attachment. Conversations are left out by default and included with `com_conversas=true`. An invalid value is refused with `400`. Tests: `export_returns_org_file_without_secret_or_path` (with the `export_source` helper).
- The export has no integration secret, neither in clear text nor encrypted, and no blob path. Test: `export_returns_org_file_without_secret_or_path`.
- Import rebuilds the target organization, keeps the SHA-256 of each blob (the download returns the same bytes), asks for the secrets again, and does not duplicate anything on a second import with `?sobrescrever=true`. Test: `import_rebuilds_org_keeps_blob_and_does_not_duplicate`.
- A natural-key conflict (same slug, different id) returns `409` and writes nothing, even with `?sobrescrever=true`. Test: `slug_conflict_is_409_and_writes_nothing`.
- Importing into the same instance for another organization returns `409`, because the ids are preserved. Test: `import_same_instance_other_org_is_409`.
- A second import that would update rows returns `409` with what would change, and writes nothing, unless `?sobrescrever=true` is given. With the flag, the agent goes back to the file's state. A non-boolean value returns `400`. Test: `import_that_would_update_rows_is_refused_without_overwrite`.
- A corrupt file and an unknown version both return `400` without writing. Test: `corrupt_file_and_unknown_version_are_400_without_writing`.
- Without a token, `401`. With a token of another organization, `403`. This holds on both routes. Test: `requires_token_and_respects_token_org`.
- A body over the limit returns `413` and is refused without loading the whole body. Test: `body_over_limit_is_413`.
- Without an embedding model, import ends with an explicit warning and a pending index, never a silent empty index. With the model installed, the imported knowledge base is searchable. Tests: the warning check is inside `import_rebuilds_org_keeps_blob_and_does_not_duplicate`; the search check is `with_model_installed_imported_base_is_searchable`.

Gaps:

- `with_model_installed_imported_base_is_searchable` returns without checking anything on a machine where the model is not installed. On such a machine, the search claim is not verified.
- The warning and pending-index check runs only on the branch without the model. The branch with the model is checked only by the test above.

Notes from the removed comments:

- The test token is global. The server stores only its hash.
- The scenario cipher key lets the integration secret be decrypted in the scenario. It is used for the field names only.
- The secret that must not appear in the file or in a response is a fixed sentinel value.
- The PNG used in the library is a minimal file (signature only), because content detection uses the signature.
- Each scenario server has its own database, blob folder and configuration. Dropping the scenario stops the server and removes the folder.
- `com_modelo` links the repository model folder into the data folder, only when that folder exists, as the CLI tests do.
- The organization helper builds the material a backup must carry: an agent with instructions, a category, a department, a product, a group and a base item, a library file (a real blob), and an integration with a secret.
- The group link is created on purpose. Without it, the agent cannot see the group, and the base has no chunks to reindex.
- Importing into another instance (another database) is the real use: restoring a file on a new install. Importing into the same instance for another organization is refused on purpose, because the ids are preserved.
- Importing back into the same organization would revert what changed after the file was made. By default the import refuses; only `?sobrescrever=true` authorizes it.
- One case changes the agent's name after the export. That is the adjustment that a reimport would revert.
- The byte-limit test sends one byte above the cap and expects a refusal, instead of loading the body.
- Without the model, the test on the model path returns early, as noted in the gaps.

## `test_chat.rs` (was `teste_chat.rs`)

Covers the test chat of the Console (HR-22): server-sent events (SSE), attachments, cancellation and concurrency. Each test starts a real server on an ephemeral port, with the turn running over a fake LLM provider and a fake extractor script. There is no external network and no key. The database is temporary.

What it checks:

- Events arrive in order (message confirmed, turn opens, deltas, end). An attachment is stored, and its extracted text reaches the prompt as untrusted content. The turn is over and the reply is confirmed in the outbound queue. Test: `test_conversation_gets_sse_in_order_and_attachment_text_in_prompt`.
- Two connections on the same conversation do not create two turns: the second gets `409`, and the second tab's message does not enter the turn. The first turn is then cancelled, leaves no reply, and the conversation accepts a new message, which completes. Test: `two_connections_same_conversation_do_not_make_two_turns`.
- Closing the stream cancels the turn: its state becomes `failed` with an error mentioning the cancellation, and no reply is left in the conversation or in the outbound queue. Test: `disconnect_cancels_turn_without_stuck_queue`.
- Without a turn engine, the route returns `503`. Test: `route_without_turn_responds_503`.
- A token of another organization returns `403` on the three routes. A conversation of one agent does not open through the other agent. Reopening: no `If-Match` gives `428`, an old version gives `412`, the right version gives `200`. Test: `org_isolation_and_reopen_with_if_match`.
- The list of test conversations returns the newest first, with a cursor. Test: `test_conversation_list_newest_first_with_cursor`.
- A refused file returns `400` with the error in the `file` field, and no message is written. The conversation stays free for the next message. Test: `refused_file_returns_in_file_field_and_writes_no_message`.
- When the agent sends a file, the history says which file it is without leaking the blob path, and the download returns the same bytes. Another organization cannot read this history. A conversation with no file has an empty list, not null. Test: `history_shows_agent_file_and_download_returns_bytes`.
- The tool call record separates executed calls from refused ones, with summarized arguments and results. `tools` stays a list of names. A turn with no tool call has an empty list, never null. Test: `history_carries_tool_call_record`.

Gaps:

- The header said a new conversation starts with empty memory. No assertion in the file checks that.
- The grouping window of the turn is disabled in this scenario, so the test does not cover grouping.

Notes from the removed comments:

- The fake extractor is a script that imitates `poph-extrator`: it returns the document text without reading it.
- The multipart body is built by hand, because the test client does not have the multipart feature.
- The helper that waits for the last turn to leave the running state is `wait_turn_finish`.
- The scenario can put a file in the agent's library with the `send_file` tool enabled. The scripts receive the id of that file, so the model can ask for it. The tool list comes from the agent's instructions, which is how the turn knows which tools to offer the model.
- The scenario disables the grouping window ("the test does not wait 2 s per turn").
- The read limit of the outbound queue in the checks is 200 (constant `BATCH_LIMIT`).
- The outbound state is the real state of the queue: the console transport confirms the output before the end event.
- The default script is a turn that asks the model to send a file and then answers in text.
- The SSE test reads until the first piece of the model's text, then closes the connection. This is how the cancel test starts the turn.

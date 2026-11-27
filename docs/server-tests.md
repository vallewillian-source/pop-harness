# Server tests: what each file proves

This document covers the integration tests in `crates/poph-server/tests/`. The explanatory comments that used to sit at the top of these files and inside them were moved here, so the test source keeps no comments. Each section lists what the file checks, which test checks it, and the notes that were in the comments.

Where a comment made a claim that the test does not fully check, the section says so under "Gaps". Nothing was added to or removed from the tests themselves.

## Renamed files

| Previous name | Current name |
| --- | --- |
| `teste_chat_encaminhado.rs` | `forwarded_chat_test.rs` |
| `produto_exclusao.rs` | `product_deletion.rs` |
| `orgs_agentes_ativos.rs` | `orgs_active_agents.rs` |

`console.rs` and `site_chat.rs` keep their names.

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

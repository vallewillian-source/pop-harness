# CLI tests: what each file proves

This document covers the end-to-end tests in `crates/poph/tests/`. The explanatory comments that used to sit at the top of these files and inside them were moved here, so the test source keeps no comments. Each section lists what the file checks, which test checks it, and the notes that were in the comments.

Nothing was added to or removed from the tests themselves.

## `cli.rs`

`poph` binary end to end: a real process, a controlled environment. The secret values used here are sentinels (`SENTINELA-...`); the tests prove that none of them shows up on stdout, stderr or JSON, even with `trace` logging.

Helpers: `poph` runs the binary with a clean environment, an isolated current directory and the given variables; `full_environment` is the complete and correct environment, with the data directory inside the test directory.

- `version_and_help_keep_working`: `--version` prints `poph 0.1.0`; `--help` lists `doctor`.
- `missing_command_shows_help_and_exits_two`: clap writes the help to stderr when the command is missing, and the exit code is 2.
- `doctor_with_correct_environment_exits_zero_and_shows_no_secret`: `doctor` exits 0, reports the environment as ready, names the defined variables and prints no sentinel.
- `doctor_json_is_valid_and_shows_no_secret`.
- `missing_required_variable_exits_nonzero_and_names_it`.
- `secrets_do_not_leak_in_log_even_at_trace_level_and_json`.
- `invalid_toml_names_file_and_key_and_exits_nonzero`.
- `file_and_environment_precedence_shows_in_doctor`: the environment wins over the file, so the data directory is the one from the environment.
- `migrate_creates_databases_and_is_idempotent`: the first run creates both databases from scratch and applies the initial schema; the second run has nothing to apply and keeps the same exit code. The schema version comes from the single source in `poph-store` (the Harness schema target version), and `doctor` shows the real schema version while still not leaking secrets.

## `cli_admin.rs`

End to end for `poph org`, `token`, `category` and `department` (HR-08): a real process, a real database in a temporary directory, `--offline` on every command. The test organization is created by the CLI itself, except when the test needs agents — the test `agent` is created through the store, as in `cli_agente.rs`.

Helpers: the fixed UUID v4 is rejected by the CLI with a reason because it is not a v7; the temporary directory holds the data directory, and the database is created on the first CLI run; the helper opens the store directly to seed what the CLI does not create yet (such as agents); organizations are created by the CLI and the v7 UUID is read from the JSON output; `field` reads a field from the JSON output that must exist.

- `end_to_end_creates_and_lists_the_four`: creates and lists organizations, tokens, categories and departments, then shows each one.
- `org_token_shows_once_and_stays_only_as_hash`: no command after creation shows the value, neither as text nor as JSON; `show` reports the value as unavailable; the database does not store the value in clear either (not even in the write-ahead file). What is stored is an argon2 hash of the **secret** (HR-20): the value carries the token id (`poph_<uuid>_<secret>`), and the API finds the record by the uuid before checking the secret.
- `revoked_token_is_no_longer_active_and_repeating_revocation_is_fine`.
- `org_with_agents_only_goes_with_force`: without `--force` the deletion is refused, warns how many agents it would take along, and the refusal must not have deleted anything.
- `empty_org_goes_without_force`.
- `org_with_category_also_asks_for_force`.
- `exit_codes_zero_one_and_two`: 0 means success; 1 means an error to fix (nonexistent organization, missing `--offline`); 2 means no action (wrong command-line usage, missing required argument such as `--if-match` on update).
- `error_messages_are_readable`: invalid slug, duplicated slug, non-v7 UUID, missing `--org`, invalid product category id, missing token, repeated record and an action with no field to change — each with its readable message. A repeated name in the same organization is refused without case distinction.
- `change_requires_the_right_version`: with the old version (1), after the change (now 2), the write is refused.
- `category_only_goes_with_force_and_department_goes_directly`: the refusal does not delete the category; a department leaves directly.
- `data_from_one_org_does_not_show_in_another`: besides agents, the deletion would take categories, departments and tokens along; one organization does not see or revoke the other's data.

## `cli_agente.rs`

End to end for `poph agent` and `poph instructions`: a real process, a real database in a temporary directory, a controlled environment. The organization is seeded by the store (the `poph org` command is HR-08 and does not exist in this crate yet). The secrets used here are sentinels, and the tests check that they do not appear in the output.

Helpers: the temporary directory holds a migrated database and an organization ready for the CLI; the `Store` is closed before the CLI opens the database, because the dedicated writer shuts down; helpers create a second organization in the same directory (returning the slug) and a product category in the given organization (returning the id); the instructions JSON uses V1 tool names.

- `cli_creates_agent_with_instructions_and_lists_showing_the_composed_text`: `show` prints the composed text and `list` shows the agent.
- `stale_if_match_is_refused_and_version_rises_on_every_write`: agent version 1 becomes 2 and version 1 stops being valid; a missing `--if-match` on the instructions update is a clear error that does not touch the database.
- `d05_inactive_agent_stops_opening_turn_via_cli`: an inactive agent stops opening turns and still appears in the list for the Console; an unknown status is refused.
- `one_org_neither_sees_nor_deletes_another_org_agent`: reading by id is also scoped by organization, and the refusal to delete leaves the agent in place.
- `tool_outside_v1_is_refused_with_the_engine_list`: instructions with a V1 tool name are refused with the engine tool list, and nothing is written.
- `hand_edited_markdown_is_preserved_without_recomposition`: a hand-edited markdown is kept as written; deleting the instructions does not delete the agent.
- `without_org_the_command_fails_saying_what_is_missing`.
- `help_lists_the_two_new_subcommands`.
- `product_category_link_stays_scoped_by_org`: a category from another organization is refused, because the link does not cross isolation.

## `cli_arquivo.rs`

End to end for `poph file` (HR-24): a real process and a real database. Image upload does not depend on `poph-extrator`; documents (PDF/Office) depend on it and are covered by the `poph-files` crate tests. The focus here is the CLI contract: a readable refusal with exit code 1 (large file, misleading extension), a blob deduplicated by hash, `list`/`show`/`delete`, a stable `--json`, and isolation per agent.

Helpers: the minimal PNG is detected by content through its signature; `ok` and `ok_json` run with `--org` and `--offline` and return stdout or the parsed JSON; `failed` returns the exit code and stderr; `blobs` counts the blobs in the organization folder.

- `uploads_lists_shows_and_deletes_file`: list shows the registered name and description, show returns the same file with a relative blob path, delete empties the list, and the blob stays on disk, because retention deletes orphans.
- `same_content_twice_yields_a_single_blob`: two different file records share one blob by hash.
- `file_above_16_mb_is_refused`: 17 MB of zeros is refused by size before any document reading, and nothing is written.
- `misleading_extension_is_refused`: image content under a PDF name is refused because the real type is not the declared one.
- `missing_file_and_wrong_agent_fail_with_code_one`: a missing file, a nonexistent agent and an invalid id (not a v7 UUID) fail with code 1.
- `without_offline_the_command_refuses`: without `--offline` the command fails and says so.
- `library_is_per_agent`: a second agent in the same organization neither sees nor deletes the file, and the owner keeps it.

## `cli_backup.rs`

End to end for `poph backup` (HR-53): a real process, a real database and real blobs. The scenario is an organization with an agent, instructions, a category, a department, a product, a knowledge-base item, a file in the library, an integration with an encrypted secret and a conversation with a message, an attachment, a turn and an output item. The test exports to a single file, imports into a clean instance and compares the state row by row. It also proves idempotency, the absence of the secret in the file, the isolation of the neighbor organization and the refusal of a corrupted file.

Automatic reindexing depends on the embedding model: without it installed, the test checks the warning instead of the search (as in the `poph kb` tests).

Constants: the integration secret must not leave in the file (neither in clear nor encrypted); the cipher key exists only so the secret is decryptable in field names; the LLM/Jev secret must not leak either; the minimal PNG is detected by content through its signature.

Helpers: the scenario without the embedding model runs the commands and leaves the index pending with a warning; the scenario with the model is used when it is installed (real search and reindexing); `seed_origin` fills the origin organization with one resource of each type, an integration with a secret and a conversation, while the integration with the encrypted secret and the history are seeded directly through the store; `compare_snapshots` and `compare_blobs` compare two snapshots row by row, ignoring the secret mark on integrations.

- `export_and_import_rebuild_org_in_a_single_file`: one file, with the declared format, no secret and no blob path. A clean instance with the embedding model imports, reindexes and compares the state. The secret did not come: the integration exists, without a secret, with the operator configuration. Reindexing: with the model, the knowledge base answers without a manual step; without it, the warning comes.
- `importing_into_existing_org_does_not_touch_neighbor_org`: the neighbor organization gains the imported resources with the ids from the file, without losing its own; what belonged to the neighbor stays identical (its own agent intact).
- `corrupted_or_wrong_version_file_is_refused_without_writing`: truncated content is not JSON, an unknown format version is refused, and nothing is written — the instance still has no organization.
- `export_refuses_overwrite_without_flag`: overwriting is refused by default (HR-48e-3). Importing again without `--overwrite` stops before writing and says how many rows and in which tables; with `--overwrite`, importing again duplicates nothing and updates the same rows.
- `export_refuses_blob_missing_on_disk`.
- `imported_blob_lands_in_destination_org_folder`: the blob path is recalculated for the destination organization (the folder changes with the organization), and the origin organization folder does not exist in this instance.

## `cli_cluster.rs`

`poph cluster show|check` end to end: a real binary, real files. It proves:

- without `cluster.toml`, both commands say the installation runs in single-instance mode;
- with the file, `show` brings the local instance, the organizations per instance and the fingerprint;
- `show --json` has a stable format (`versao_relatorio: 1`) and no secret;
- an invalid map (`check`) exits 1 and points at the key; a file with broken TOML does the same;
- a cluster without `POPH_CLUSTER_SECRET` is refused, without printing any secret value;
- `POPH_CLUSTER` points at a file outside the `poph.toml` directory.

- `no_cluster_toml_means_single_instance_mode`.
- `show_prints_map_with_local_instance_and_orgs_per_instance`.
- `show_json_has_stable_format_and_no_secret`.
- `invalid_map_fails_check_pointing_at_key`.
- `broken_toml_fails_with_source`.
- `cluster_without_secret_in_environment_is_refused`.
- `poph_cluster_points_at_file_outside_poph_toml_dir`: `poph.toml` sits in the directory while the map is in a separate file named by `POPH_CLUSTER`.
- `map_fingerprint_is_equal_across_instances_of_same_cluster` (HR-50b): `x-poph-mapa` compares the shared content, not the local name, so two instances with the same `cluster.toml` (only `instance_id` differs) have the same map fingerprint and different local fingerprints.
- `remote_check_approves_equal_map_across_instances` (HR-50b-2): the fake server answers a single JSON response per connection on a fake `/healthz` and returns the address; the listener is already open so the address enters the map **before** the test computes the fingerprint, because the URL is part of the canonical text. The fingerprint is computed before the fake server answers, and the fake server stays up until the end of the test process (it is not joined to the thread).
- `remote_check_fails_divergent_map_and_instance_down`: a divergent map on one side and the other side down exits 1 and points at both; the port nobody listens on is opened and closed immediately. In JSON: `ok: false`, one remote problem per instance and no secret.
- `remote_check_without_cluster_consults_nothing`: nothing to consult, exits 0 and says so.

## `cli_flow.rs`

End to end for `poph flow` and `poph ficha` (FX-31): a real process and a real database in a temporary directory. The scripts and the publication rules are tested in the API (`crates/poph-server/tests/fluxos_api.rs`); here the command line is checked: exit codes, the shape of `--json`, the error messages and the effect on the database, read by the repository itself. The data is fictional.

Fixtures: `SCRIPT_V1` is the same script the API uses, with the `confirmacao` step as the starting point of revision 2; `SCRIPT_V2` carries another instruction in `confirmacao`, so revision 2 changes one sentence; `BROKEN_SCRIPT` has a syntax error on line 3 (the `nome` value does not exist).

Helpers: the scenario is a temporary directory with the data directory, and the database is born on the first CLI run; `poph` runs with a clean environment, an isolated current directory and the test data directory; `write_file` writes a file into the test directory and returns the path; `create_org` and `create_agent` create through the CLI and return the id; `store` opens the test database directly through the repository (the CLI already migrated it); `keys` lists the top-level keys of a JSON object in order, to prove the shape does not change; `publish_ok` publishes a script in a new file and checks it worked (creation, revision 1); `fluxo_id` reads the id of the `agendar_visita` flow of the org from the repository.

- `validate_good_script_exits_zero_and_json_has_stable_shape`.
- `validate_bad_script_exits_one_and_points_at_line_and_column`.
- `publish_lists_and_shows_the_flow`.
- `publish_new_revision_requires_version_and_leaves_revision_1_untouched`: without `--if-match` the publication of an existing flow does not happen; an old version is a conflict and nothing is written; the right version produces revision 2. The draft becomes the published text, so `mostrar` cannot bring old text, and revision 1 keeps the text byte for byte as published.
- `publish_invalid_script_writes_nothing`.
- `format_is_idempotent_and_writes_to_the_same_file`: the script enters in compact form, which is not the canonical one, so without writing the file stays as it is and the test fails; writing puts exactly the canonical form in the file, and formatting the canonical form again changes nothing.
- `format_script_that_does_not_compile_exits_one`: a bad script is not written.
- `diff_shows_what_changed_and_the_cases_in_each_revision`.
- `release_and_withdraw_change_agent_permission`.
- `exit_codes_for_usage_and_error`: without `--org` on a command that reads org data it is a business error, code 1; a file that does not exist and a status outside the vocabulary also exit 1; a nonexistent subcommand is wrong usage, code 2 (clap decides).
- `list_empty_cases_and_show_missing_case`.
- `case_with_filter_and_detail_appear_in_the_cli`.

## `cli_jev.rs`

End to end for `poph jev eval` (HR-28): a real process against a fake local Jev on an ephemeral port, with no external network and no real key. What is checked here is the command line — exit codes, the printed report, a stable JSON and the refusal when the key is missing.

Helper: `jev_server` starts a fake Jev that always answers the given choice and confidence, with an optional `usage`.

- `measures_the_dataset_and_prints_accuracy_abstention_and_latency`.
- `json_report_is_stable`.
- `without_key_refuses_and_says_what_is_missing`.
- `unknown_gate_and_missing_file_fail_with_a_message`.
- `help_lists_the_subcommand`.
- `abstention_shows_choice_and_reason`: acceptance of the calibration — with an answer below the threshold (confidence 0.50 against the G1 threshold 0.70) the report shows the model choice, the confidence and the reason for the fallback, in the text and in the JSON.
- `without_usage_says_not_reported`: with no `usage` in the response body the report says "not reported" instead of inventing a number.

## `cli_kb.rs`

End to end for `poph kb` (HR-17): a real process, a real database and a real index. The index needs the embedding model, so the scenario points the model folder at the repository one (symlink in `dados/modelos`); without the installed model the tests are skipped, like the `poph-rag` ones.

- `cli_kb_creates_indexes_searches_edits_and_deletes`.
- `cli_kb_refuses_item_above_one_thousand_characters`.
- `repo_models_dir_exists`: guarantees the scenario symlink points at an existing folder (environment diagnosis).
- `cli_rag_reindex_is_idempotent`: `poph rag reindex` (the name the PRD uses) really reindexes and is idempotent — running again without a change generates no embedding; a lost index is rebuilt from scratch.

## `cli_matrix.rs`

End to end for `poph matrix registro`: a real process, a controlled environment. What must be proven (MX-21): the file comes out with mode 0600, it does not overwrite without `--forcar`, a missing token names the variable, no token appears on stdout or stderr, and `--verificar` accepts what this command generated and refuses (naming the field) what diverges from the configuration.

Helpers: `config` writes the minimal configuration with the `[matrix]` section on and valid; `environment` is the clean environment with only what the command needs.

- `generates_with_0600_without_token_in_output_and_verify_accepts_it`.
- `refuses_overwrite_without_force_and_overwrites_with_force`.
- `missing_token_names_the_variable_without_showing_the_other`.
- `without_url_the_error_explains_what_is_missing`.
- `url_from_environment_variable_works`.
- `verify_refuses_swapped_url_naming_the_field`.
- `verify_refuses_swapped_prefix_naming_the_namespaces`.
- `verify_refuses_token_different_from_environment_without_showing_value`.
- `without_matrix_section_the_error_says_what_is_missing`.
- `verify_missing_file_does_not_pass_in_silence`.
- `output_and_verify_together_are_refused`.

## `cli_matrix_module.rs`

End to end for `poph matrix estado|ligar|desligar|espaco` (MX-23): a real process, a real database and, in the space tests, a **fake Synapse** up on an ephemeral port. It proves:

- `estado`, `ligar` and `desligar` show and write the same as the API (MX-22): the backfill cutoff is written on the first turn-on, repeating does not move the cutoff and turning off preserves it;
- `espaco --departamento` creates the space once (repeating does not create a second), `espaco` lists, and `espaco desligar` removes the row without deleting anything in Synapse;
- a nonexistent org, use without `--offline`/`--org`, the module off, a missing token and a department from another org give a clear error, with no token value in the output;
- no output carries the token values (the canary values).

The version check (`412`, exit code 3) is proven by a unit test in `matrix_admin.rs`, because the CLI is what reads the version and writes, and no external process can change the row mid-way.

The fake Synapse block is a copy of the routes of `crates/poph-server/tests/matrix_api.rs`: each test file is a crate, and a `poph-server` test cannot be imported here. It runs on an ephemeral port in its own thread with its own runtime, because the test is synchronous (it runs `poph` as a process, and the process talks to the real Synapse over HTTP).

Helpers: `Scenario` is a test directory with its own `poph.toml` and database, and `homeserver` points at the fake Synapse (or at a dead address when the test does not talk to it); `create_org` and `create_department` create through the CLI and return the id; `without_tokens` proves no token value appears in the output; `spaces_in_synapse` counts the spaces (`m.space`) the fake Synapse holds; `query_argument` is the minimal percent-encoding of a user id in the `user_id` parameter.

- `state_turn_on_and_off_through_the_cli_preserve_the_cutoff`.
- `org_and_usage_without_the_flags_give_a_clear_error`.
- `space_creates_once_lists_and_turns_off_without_deleting_in_element`.
- `space_without_tokens_names_the_variable_and_writes_nothing`.
- `space_with_module_off_refuses_with_the_instruction`.
- `department_space_from_another_org_does_not_exist_for_this_org`.
- `space_refuses_combinations_and_invalid_space`.

## `cli_product.rs`

End to end for `poph product` (HR-18): a real process, a real database and a real index. The index needs the embedding model, so the scenario points the model folder at the repository one (symlink in `dados/modelos`); without the installed model the tests are skipped, like the `poph-rag` and `poph kb` ones.

`Scenario::new` returns `None` when the model is not installed (the test is skipped). Notes kept from the tests: the write reindexes the org agent; an old `If-Match` is refused; without `--force` the deletion does not happen; a negative price is refused with a clear error and nothing is written; a nonexistent category is refused.

- `cli_product_creates_shows_updates_and_deletes`.
- `cli_product_reindexes_and_respects_the_price_limit`.

## `cli_retention.rs`

End to end for `poph retention run|show` (HR-51): a real process and a real database in a temporary directory. The scan itself (what leaves, what stays, the simulation) is covered by `crates/poph-retention/tests/database.rs`; this file checks the command line: exit codes, JSON shape and error messages.

Helpers: `Scenario` is a temporary directory with the data directory, and the database is created on the first CLI run; `poph` runs the binary with a clean environment, an isolated current directory and the test data directory; `create_org` creates an organization with the CLI (version 1) and returns the id read from the JSON; `configure` replaces the organization configuration (the version is 1, the one from creation).

- `dry_run_json_has_stable_shape_and_deletes_nothing`: `retention run --dry-run --json` keeps a stable JSON shape and deletes nothing.
- `run_without_orgs_exits_zero_and_warns`: with no organizations the run exits 0 and says there is nothing to scan.
- `org_with_invalid_config_exits_one_and_says_nothing_was_deleted`.
- `missing_org_exits_one_with_clear_message`: a nonexistent slug exits 1 with a readable message.
- `show_reports_default_and_org_zero_retention`: `show` prints the instance default and the organization that never deletes.
- `show_json_with_invalid_org_exits_one`: the JSON keeps the report shape and the failing organization carries the error message.
- `run_keeps_case_execution_step_event_and_effect` (FX-34): `MODULE_TABLES` lists the tables of the flows and cases module, and the scan must not lose a row of any of them. `poph retention run` (and `--dry-run`) deletes the old empty conversation but does not touch anything in the recorded case; after the scan the case keeps `conversation_id` null. The conversation is seeded through the repository, in the same database the CLI scans, and then aged by `DAY_MS`; `--dry-run` deletes nothing. The only field that changes on purpose is the case link to the deleted conversation.

## `tools_without_store.rs`

HR-46f: without the embedding model installed, the turn still offers the tools that only use the database — `enviar_arquivo` included — and `consultar_base`/`buscar_produto` stay out.

The defect fixed: the tool registry was built **inside** the knowledge base block. On an installation without `poph model install` the provider request went out with `tools: []`, the model did not receive `enviar_arquivo` and the turn ended `done` without sending anything.

Here the real `poph serve` binary is started with a fake OpenAI-compatible provider and **without** the models folder. What is proven: (a) the provider request carries `tools` with `enviar_arquivo` and without `consultar_base`/`buscar_produto` (all three were enabled on the agent, so the absence comes from the registry, not the filter); (b) the model calls `enviar_arquivo` and the test chat history returns `files_out`; (c) the library download returns the same bytes as the seeded file (same sha256); (d) the history response carries no blob path (`blob_ref`/`blobs/`).

Helpers: `test_dir` is a throwaway directory per test; `Servidor` is the `poph serve` child, killed at the end of the test even when an assertion fails; `fake_provider` is a fake OpenAI-compatible provider whose first request asks for `enviar_arquivo` and whose following requests answer text, keeping each request body for the test; `read_request` reads a whole HTTP request (headers plus body by `content-length`); `evento` writes an SSE event in chunked encoding; `multipart` builds a body with only the `text` field. `Scenario` seeds the database (organization, agent with the three tools enabled, file in the library), starts the fake provider and `poph serve` without the models folder.

Notes: the grouping window is disabled (`grouping_window_ms: 1`) so the test does not wait 2 s per turn; the second test is the control for the per-agent filter (D-04): with `enviar_arquivo` disabled in the instructions the tool exists in the registry but does not go to the model.

- `without_embedding_model_turn_offers_file_tool_and_file_is_sent`.
- `without_embedding_model_disabled_tool_stays_out`.

## `serve.rs`

`poph serve` refuses to start with a database that is not ready (HR-20). The rule: a missing database is created already migrated (new installation); an existing database the code does not understand — a corrupt file or a schema behind the migrations — requires `poph migrate` first. This file proves the refusal: the process exits with code 1, explains what to do and does not open a port. The expected version comes from the single source in `poph-store` (the Harness schema target version), so the next migration does not leave the assertion behind.

- `serve_refuses_database_not_from_harness`: an existing but unreadable database is not a Harness SQLite, and the message says the version could not be read.
- `serve_refuses_database_behind_migrations`: simulates a database from a previous version — the schema exists but without the migration record — and checks that the message carries the expected version and names `poph migrate`.

## `serve_logs.rs`

`poph serve` writes a log of the operations and writes no sensitive data (HR-26f). It starts the real binary (`poph serve`) over a temporary database with a configured Evolution instance and delivers a text webhook on the public route, then reads the process stderr. Proof in two parts: (i) the capture works — the startup line (`servidor no ar`) and the registered message line appear; (ii) nothing sensitive appears in the log or in the responses: phone, JID, Evolution key, instance token, the right secret and the wrong secret that was sent.

Helpers: `Setup` is a temporary directory with configuration and database ready (organization, agent and WhatsApp integration); `free_port` picks a free port; `prepare` builds the setup; `Servidor` is the `poph serve` process with stderr read in the background; `start` starts it with a clean environment (no `RUST_LOG`, to prove the level comes from the configuration); `stop` kills the process and waits for stderr to finish being read; `http` is a minimal HTTP request (no library) to talk to the real process; `wait_until_up` polls `/healthz`; `text_event`/`state_event` build the Evolution webhook payloads.

- `server_logs_webhook_operation_without_sensitive_data`: the text message is registered as `nova` and no forbidden value (phone, JID, key, token, secrets, message text) appears in the log or in the responses.
- `server_logs_connection_state_without_sensitive_data`: the connection state update is registered with the new state (`connected`) and the instance (`loja`), and no secret appears.

## `token_api.rs`

End to end for the new organization token format (HR-20, item 2). The token is **created by the CLI** (`poph --offline token create`) and **used in the API**: the server starts in process, in the same data directory, on an ephemeral port. It proves the two sides agree on the format `poph_<token uuid>_<secret>`, that the clear secret only appears on creation and that a wrong secret does not authenticate.

Helpers: `Scenario` is a temporary directory with the data directory; `create_org`/`create_token` create them through the CLI and read the JSON (the clear token value only exists in the creation output); `start_server` starts the server in process in the test data directory and returns the address.

- `token_created_by_cli_authenticates_against_api`: the CLI token authenticates and sees only its own organization (200 for its own, 403 for another); a wrong secret and a nonexistent uuid are 401; neither `list` nor `show` nor `--json` returns the value after creation.

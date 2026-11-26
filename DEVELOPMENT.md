# Development

How to set up the environment, build, test and package POP Harness. For working rules and the code map,
read [`AGENTS.md`](AGENTS.md). To contribute, read [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Requirements

| Item | Version | Notes |
|---|---|---|
| Linux | x86_64 or ARM64 | other systems have not been tested |
| Rust | `1.100.0` | pinned in `rust-toolchain.toml`; `rustup` installs it on its own. Targets `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, plus `clippy` and `rustfmt` |
| C compiler | `gcc` or `clang` | the bundled SQLite (`rusqlite`, `bundled`) and TLS (`aws-lc-sys`) compile C code |
| `cargo-deny`, `cargo-audit` | recent | `cargo install cargo-deny cargo-audit --locked`; used by `scripts/ci.sh` |
| Node and pnpm | Node 22.18 or newer | only for the Console |

## Local files

A local run writes to `data/`: the two databases, the attachments and the embedding model. That folder, `.env*`,
keys and certificates (`*.pem`, `*.key`), `cluster.toml` and the local registrations (`registro*.yaml`) are in
`.gitignore` and are never committed. Configuration and secrets come from the environment, as
[`README.md`](README.md) lists; a secret has no file form.

## Build

```bash
cargo build --workspace                 # development: <target>/debug/poph
cargo build --release --workspace       # release: <target>/release/poph and <target>/release/poph-extrator
```

`<target>` is the Cargo target directory of the checkout, which a local `.cargo/config.toml` may move to a
shared folder; `cargo metadata --format-version 1 --no-deps | jq -r .target_directory` prints it. Scripts
that need the binary must ask Cargo for it instead of assuming `target/`.

`poph` looks for `poph-extrator` in its own executable's folder, so the two must sit side by side. Without
it, receiving PDF and Office files refuses with a clear message; images do not depend on it.

### Console

The Console (`console/`, React, TypeScript and Vite) is embedded in the binary: `poph-server`'s `build.rs`
reads `console/dist` if it exists. Without the bundle, the server still builds and serves a minimal page
explaining how to generate it, so `cargo build` never depends on `pnpm`.

> **Known limitation:** the Console imports the `@sofacoding/sofa-ui` package from a local path outside
> this repository, so a clean clone cannot build the bundle yet. See [`ROADMAP.md`](ROADMAP.md).

```bash
cd console
pnpm install          # needs network
pnpm build            # generates console/dist
cd ..
cargo build --workspace
```

The Console steps of `scripts/ci.sh` — the bundle, the API client check and the component tests — declare
themselves skipped when `console/node_modules` does not exist, so the local CI of a clean clone still runs
offline; `pnpm install` is the only step that needs network and it is manual on purpose. Dependabot does not
watch `console/` for the same reason: the package comes from a local path and there is no published lockfile.
Add the npm ecosystem to `.github/dependabot.yml` once `@sofacoding/sofa-ui` is on npm.

To develop with auto-reload, start the server on the default port (7480) and run `pnpm dev` in `console/`:
Vite opens at `http://localhost:5199` and forwards the API to 7480. If the server uses another port, adjust
`console/vite.config.ts`.

The TypeScript API client is generated from the server's OpenAPI document. After changing a route:

```bash
bash console/gen-api.sh        # regenerates console/src/api/schema.d.ts (do not edit by hand)
```

## Dependencies

`deny.toml` holds the policy; `scripts/ci.sh` runs `cargo deny check` and `cargo audit`.

- **Licences.** Only the permissive ones in the list are accepted (Apache-2.0, Apache-2.0 with the LLVM
  exception, BSD-2-Clause, BSD-3-Clause, ISC, MIT, Unicode-3.0 and Zlib). Anything else — GPL, AGPL, no
  licence — fails the check, and adding a licence takes a reason in the pull request. The list covers the
  dependencies of the plan, including the ones not in the tree yet, so `unused-allowed-license = "allow"`
  keeps the check from reporting the missing ones.
- **One exception.** `webpki-roots` ships the trust roots under CDLA-Permissive-2.0. It comes only with the
  ONNX Runtime download during the build of `poph-rag` (`ort-sys` → `ureq`) and does not reach the binary,
  which `cargo tree -p poph -e normal` confirms.
- **OpenSSL is banned.** `openssl` and `openssl-sys` fail the check: TLS is `rustls` (see below), which also
  keeps the ARM64 build free of an OpenSSL build.
- **The workspace's own crates** are not published and declare no SPDX licence, so `[licenses.private]`
  ignores them.
- **Duplicates and sources.** The same crate in more than one version is a warning, a wildcard requirement is
  an error, and crates.io is the only accepted source: no unknown registry, no git dependency.

## Test

```bash
bash scripts/ci.sh                        # everything, in CI order
cargo test --locked -p poph-retention     # one crate
cargo test --workspace                    # all Rust tests
(cd console && pnpm run testar)           # Console tests
(cd console && npx vitest run src/screens/Operation.test.tsx)   # one component test
```

`scripts/ci.sh` runs: cache pruning, the Console bundle, the API client check, Console tests, the comment
remover test, the comment gate, the glossary check, the identifier gate, `cargo fmt --check`, `cargo clippy -D warnings`, build, Rust tests, the model RAM
test, `cargo check` for ARM64, `cargo deny` and `cargo audit`. The Console steps declare themselves skipped when
`console/node_modules` does not exist. It ends with `CI LOCAL: OK` or the number of checks that failed.

Tests that talk to external services (model, Jev, EvolutionAPI, Synapse) use in-process fake servers: the
suite needs no network and no keys. Tests marked `#[ignore]` are measurements or depend on files from
outside the repository, and say in the attribute what they need.

### Comment gate

Code and configuration carry no comment (`AGENTS.md`); the only exceptions are the shebang and a directive that
breaks the tool, one line each in `scripts/allow-comments.txt`. `bash scripts/check-comments.sh` is the gate and
reads the scan of `scripts/strip-comments.py`:

```bash
bash scripts/check-comments.sh                              # whole repository against the pending list
bash scripts/check-comments.sh crates/poph-jev              # one module: prints nothing when it is clean
bash scripts/check-comments.sh --atualizar-pendentes        # regenerate scripts/pendentes-comentario.txt
```

The `--atualizar-pendentes` runs belong to the commit that closes a wave of cleaned modules, not to a
module pull request: an author leaves both pending lists alone.

`scripts/pendentes-comentario.txt` is the whitelist of modules not cleaned yet: one line per file, with the module
and the number of comments. It is generated, never edited by hand, and it shrinks to nothing as the modules are
cleaned. The gate fails in four cases: a comment outside the allowlist in a file that is not listed, a listed file
that no longer has comments, a count that no longer matches the tree, and a malformed list. It also fails when an
exception in `scripts/allow-comments.txt` stops matching any comment, so a stale exception is not forgotten. Only
files the repository publishes are scanned: what `.git/info/exclude` keeps on this machine is out, because it does
not exist in a clone. It runs on Python and `git` (no toolchain) and takes about 5 s on the whole tree.

### Identifier gate

Identifiers are in English (`AGENTS.md`). `bash scripts/check-names.sh` reads the Portuguese terms of
`scripts/glossary-map.txt`, adds the forms the map does not spell out (plural, feminine, verb conjugation) and
fails when one of them is left in an identifier:

```bash
bash scripts/check-names.sh                              # whole repository against the pending list
bash scripts/check-names.sh crates/poph-jev              # one module: prints nothing when it is clean
bash scripts/check-names.sh --atualizar-pendentes        # regenerate scripts/pendentes-nomes.txt
bash scripts/check-names.sh --atualizar-allowlist        # regenerate scripts/allow-names.txt
bash scripts/check-names.sh --measure                    # most frequent Portuguese identifiers
```

`scripts/allow-names.txt` is the closed list of section 7.2 of the refactor: the words of the flow language, every
published name (table, column, index, trigger, route, `operationId`, JSON field, `poph.toml` key, CLI command and
flag, environment variable, metric, plugin protocol term, backup key), the words spelled the same in both languages
and the example flow scripts, which are frozen byte by byte. The file is written by `--atualizar-allowlist` from the
published inventory, so a hand edit fails the gate; a whole file is excepted with the `path` kind, never a
directory. `scripts/pendentes-nomes.txt` is the same ceiling per file as the comment list: an extra identifier in a
listed file fails, a listed file that became clean fails, and only the commit that closes a wave regenerates the
list. Identifier extraction and term splitting come from `scripts/glossary-terms.py`, which already skips string
and comment by the syntax of `scripts/strip-comments.py`, so product text never trips the gate. Only files the
repository publishes are scanned. It runs on Python and `git` (no toolchain) and takes about 5 s on the whole tree.

### Renaming tool

`scripts/rename.sh` renames identifiers and modules with the language tool, never by text substitution:
`rust-analyzer` for Rust (types, functions, files and the `mod`/`use` paths that point at them) and the
TypeScript language service of `console/` for the Console, through `scripts/rename_ts.mjs`.

```bash
bash scripts/rename.sh --map <file> --dry-run   # plan, writes nothing
bash scripts/rename.sh --map <file>             # applies the map, in order
```

The map has one rename per line, `old<TAB>new` or `old -> new`, and `#` starts a comment. A left side with a
`/` is a file rename and must stay in the same directory with the same extension. The tool refuses, with exit
code 3, a published name (table, column, index, trigger, route, `operationId`, JSON field, `poph.toml` key, CLI
command or flag, environment variable, metric, plugin protocol term, backup key, flow language word), a new name
that still carries a Portuguese term, and an identifier no language tool can rename safely; the edit of a pair
that fails is undone, so a refused rename leaves the tree as it was. Exit code 4 means a leftover of the old
name, 5 means a language tool is missing. After the renames of a module, `bash scripts/check-names.sh <paths>` has
to print nothing; `--atualizar-pendentes` and `python3 scripts/glossary-terms.py --gerar` run in the commit that
closes the wave, not in the module pull request.

The language tools are not part of the CI: `scripts/tests/test-rename.py` (about a second, `python3` only)
stubs the tool and proves the map parsing, the refusals, the edit applier and the leftover decision.

### Measurement and acceptance scripts

| Script | What it is for |
|---|---|
| `scripts/medir-memoria.sh`, `scripts/medir-memoria-roteiro.sh` | RSS and throughput of the Matrix module with a fake Synapse |
| `scripts/medir-resiliencia.sh` | Harness and Synapse going down in the middle of mirroring |
| `scripts/matrix-dev.sh` | starts a development Synapse with two registrations ([`docs/MATRIX_AMBIENTE_LOCAL.md`](docs/MATRIX_AMBIENTE_LOCAL.md)) |
| `scripts/plugin-eco.sh` | exercises the example plugin |
| `scripts/jev-relatorio.py`, `scripts/jev-falso.py` | report and fake server for the Jev ([`docs/JEV.md`](docs/JEV.md)) |

## Distribution package

```bash
bash scripts/build-dist.sh [output_folder]
```

Builds in release mode with `Cargo.lock`, gathers `poph` and `poph-extrator`, generates `SHA256SUMS` and a
`.tar.gz` next to the folder. The script refuses to overwrite an existing output. The package is **native**
to the architecture where the script runs: there is no cross-compilation: the workstation has no cross C compiler, and SQLite (bundled) and ONNX Runtime must be built for the host architecture. ONNX Runtime is linked statically
into `poph`; the embedding model is not in the package (install it with `poph model install`). The package
still depends on the system shared libraries listed in `LDD-poph.txt`.

### ARM64

The ARM64 build is made on a native ARM64 machine. `scripts/Dockerfile.build-arm` defines an Ubuntu 24.04
image (glibc 2.39) with `gcc` 13, so the binary runs on Ubuntu 24.04 hosts. The official `rust` image is
Debian and is used only to copy the pinned toolchain.

On x86_64, CI runs `cargo check --target aarch64-unknown-linux-gnu` only for the crates without C code
(`poph-types`, `poph-files`, `poph-plugins`, `poph-plugin-eco` and `poph-fluxos`). The others need a C
compiler for ARM and are only verified in a real ARM64 build.

The exclusion is per C dependency: `poph-core` (model provider over `rustls`, which pulls `aws-lc-sys`) and
`poph-jev` (same TLS stack) need the cross C compiler, and `poph-server` too, because it depends on
`poph-store` and on the bundled SQLite of `rusqlite`. `poph-fluxos` stays in the check because `blake3` — a
dependency of `starlark` — is declared with the `pure` feature: without it the crate compiles C and would
leave the check as well. `poph-plugin-eco` needs only `poph-types` and `serde_json`, so it is pure code too.

## TLS

The HTTP clients (model provider, Jev, embedding model download) use `rustls` with the system trust roots.
OpenSSL is banned in `deny.toml`. A TLS failure is final and not retried, and there is no option to ignore
certificates. To test with your own CA, install it in the system.

## Build cache

Since Cargo 1.100, each different command shape (`clippy --all-targets`, `check`, another target) produces
a new set of artifacts in `target/`, and Cargo never deletes the old ones, so the directory grows.
`scripts/ci.sh` starts by pruning the cache, and the `dev` profile reduces `debuginfo` to
`debug = "line-tables-only"`: it keeps the file and the line in a backtrace and drops the tables of variables
and types that made every test binary grow by hundreds of megabytes.

```bash
bash scripts/limpar-cache.sh --seco     # shows what it would delete
bash scripts/limpar-cache.sh            # prunes units unused for 3 days; tightens retention above 25 GB
bash scripts/limpar-cache.sh --dias 7   # longer retention
bash scripts/limpar-cache.sh --tudo     # cargo clean
```

Pruning refuses to run while `cargo` or `rustc` is running, so it does not break a build in progress.
`--forcar` ignores that guard and is only for controlled experiments.

#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

failures=0

run() {
    local name="$1"
    shift
    echo
    echo "==> ${name}: $*"
    if "$@"; then
        echo "<== OK: ${name}"
    else
        local rc=$?
        echo "<== FALHOU: ${name} (código ${rc})"
        failures=$((failures + 1))
    fi
}

run "build cache (prune what has not been used for 3 days)" bash scripts/clean-cache.sh --dias 3
run "console (Console bundle embedded in the binary)" bash console/scripts/build.sh
run "console (OpenAPI client up to date)" bash console/scripts/verify-api.sh
run "console (component tests)" bash console/scripts/test.sh

run "comments (remover test)" python3 scripts/tests/test-strip-comments.py
run "comments (gate: allowlist and pending per module)" bash scripts/check-comments.sh
run "glossary (terms, frozen and table up to date)" python3 scripts/glossary-terms.py --validar
run "names (gate: allowlist of published names and pending per module)" bash scripts/check-names.sh
run "names (renamer test)" python3 scripts/tests/test-rename.py

run "formatting" cargo fmt --all -- --check
run "clippy" cargo clippy --workspace --all-targets --locked -- -D warnings
run "build" cargo build --workspace --locked
run "tests" cargo test --workspace --locked
run "ram (model measurement, poph-rag alone)" cargo test --locked -p poph-rag --test ram -- --ignored --nocapture
run "aarch64 (check, crates without C)" cargo check --locked --target aarch64-unknown-linux-gnu -p poph-types -p poph-files -p poph-plugins -p poph-flows
run "aarch64 (check, example plugin)" cargo check --locked --target aarch64-unknown-linux-gnu -p poph-plugin-eco
run "deny (licenses, bans, sources, advisories)" cargo deny check
run "audit (vulnerabilities)" cargo audit

echo
if [ "$failures" -eq 0 ]; then
    echo "CI LOCAL: OK (todas as checagens passaram)"
    exit 0
fi
echo "CI LOCAL: FALHOU (${failures} checagem(ns) com erro)"
exit 1

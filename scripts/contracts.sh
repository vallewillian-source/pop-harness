#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

BASE="$ROOT/tests/contratos/base"
MODE="${1:-}"

fail() {
    echo "contracts: $*" >&2
    exit 2
}

cargo_target_dir() {
    local meta dir
    meta="$(cargo metadata --format-version 1 --no-deps 2>/dev/null)" || return 1
    [ -n "$meta" ] || return 1
    if command -v jq > /dev/null 2>&1; then
        dir="$(printf '%s' "$meta" | jq -er '.target_directory' 2>/dev/null)" || return 1
    elif command -v python3 > /dev/null 2>&1; then
        dir="$(printf '%s' "$meta" | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null)" || return 1
    else
        return 1
    fi
    [ -n "$dir" ] && [ "$dir" != "null" ] || return 1
    printf '%s\n' "$dir"
}

capture() {
    local target="$1"
    mkdir -p "$target" || return 1

    echo "contracts: compilando o binário" >&2
    if ! cargo build --locked -q -p poph > "$target/.build.log" 2>&1; then
        tail -40 "$target/.build.log" >&2
        return 1
    fi
    rm -f "$target/.build.log"

    local build_dir binary
    build_dir="$(cargo_target_dir)" || build_dir="$ROOT/target"
    binary="$build_dir/debug/poph"
    if [ ! -x "$binary" ]; then
        fail "binário poph não encontrado em $binary; o diretório de build do Cargo é $build_dir"
    fi

    echo "contracts: cli (todos os --help)" >&2
    python3 scripts/contracts.py cli "$target/cli" "$binary" "$target" || return 1

    echo "contracts: openapi" >&2
    local raw="$target/.openapi.json"
    "$binary" openapi > "$raw" || return 1
    python3 scripts/contracts.py canonical-json "$raw" "$target/openapi/openapi.json" || return 1
    rm -f "$raw"

    echo "contracts: doctor e migração" >&2
    python3 scripts/contracts.py doctor "$target" "$binary" "$target/.dados" || return 1
    rm -rf "$target/.dados"

    echo "contracts: api, banco, métricas, fluxos e configuração" >&2
    POPH_CONTRACT_SNAPSHOT_DIR="$target" cargo test --locked -q -p poph-server \
        --test contracts_baseline -- --ignored > "$target/.capture.log" 2>&1
    local status=$?
    if [ "$status" -ne 0 ]; then
        tail -40 "$target/.capture.log" >&2
        return 1
    fi
    rm -f "$target/.capture.log"

    echo "contracts: scripts de exemplo dos fluxos" >&2
    mkdir -p "$target/flows" || return 1
    cp tests/fluxos/exemplos/*.star "$target/flows/" || return 1
    return 0
}

scratch() {
    mktemp -d "${TMPDIR:-/tmp}/poph-contratos.XXXXXX"
}

case "$MODE" in
snapshot)
    work="$(scratch)" || exit 1
    trap 'rm -rf "$work"' EXIT
    capture "$work" || { echo "contracts: captura falhou" >&2; exit 1; }
    if [ -f "$BASE/LEIAME.md" ]; then
        cp "$BASE/LEIAME.md" "$work/LEIAME.md" || exit 1
    fi
    rm -rf "$BASE"
    mkdir -p "$BASE" || exit 1
    cp -a "$work"/. "$BASE"/ || exit 1
    echo "contracts: linha de base gravada em tests/contratos/base"
    ;;
compare)
    [ -d "$BASE" ] || fail "linha de base ausente: rode 'bash scripts/contracts.sh snapshot'"
    work="$(scratch)" || exit 1
    trap 'rm -rf "$work"' EXIT
    capture "$work" || { echo "contracts: captura falhou" >&2; exit 1; }
    differences="$(diff -r -x "LEIAME.md" "$BASE" "$work")"
    status=$?
    if [ "$status" -eq 0 ]; then
        echo "0 diferenças"
        exit 0
    fi
    if [ "$status" -eq 1 ]; then
        echo "$differences"
        echo
        echo "contracts: contrato externo mudou"
        exit 1
    fi
    echo "$differences" >&2
    fail "diff falhou"
    ;;
*)
    echo "uso: bash scripts/contracts.sh snapshot|compare" >&2
    exit 2
    ;;
esac

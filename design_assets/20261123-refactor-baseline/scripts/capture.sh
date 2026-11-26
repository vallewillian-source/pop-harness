#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    echo "Usage: capture.sh [--record]"
    echo "Runs the full cycle of seeding, starting the server, capturing and comparing against the baseline."
    echo "  --record    Updates the baseline files in design_assets/20261123-refactor-baseline"
    exit 0
fi

REQUIRED_PORT="17485"
if [ -n "${POPH_PORT:-}" ] && [ "${POPH_PORT}" != "$REQUIRED_PORT" ]; then
    echo "Error: The Console visual baseline strictly requires port $REQUIRED_PORT (POPH_PORT=${POPH_PORT} refused), because the login screen renders the port in its interface." >&2
    exit 1
fi
PORT="$REQUIRED_PORT"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_DIR="$(cd "$HERE/.." && pwd)"
ROOT="$(cd "$BASE_DIR/../.." && pwd)"
cd "$ROOT"

if [ -n "${POPH_BIN:-}" ]; then
    BINARY="$POPH_BIN"
else
    BUILD_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -er '.target_directory')" || {
        echo "Error: cargo metadata failed; cannot tell where Cargo stored poph." >&2
        exit 1
    }
    BINARY="$BUILD_DIR/debug/poph"
    if [ ! -x "$BINARY" ]; then
        echo "Building poph in debug mode..."
        cargo build --locked -q -p poph
    fi
fi

if [ ! -x "$BINARY" ]; then
    echo "Error: poph binary not found at $BINARY" >&2
    exit 1
fi

RECORD=0
if [ "${1:-}" = "--record" ] || [ "${1:-}" = "--update" ] || [ "${1:-}" = "--recapture" ]; then
    RECORD=1
fi

TMP_DEMO="$(mktemp -d /tmp/poph-baseline-demo.XXXXXX)"
TMP_SECOND="$(mktemp -d /tmp/poph-baseline-second.XXXXXX)"

cleanup() {
    if [ -n "${SERVER_PID:-}" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$TMP_DEMO" "$TMP_SECOND"
}
trap cleanup EXIT

if fuser "$PORT/tcp" >/dev/null 2>&1; then
    fuser -k "$PORT/tcp" 2>/dev/null || true
    sleep 0.5
fi

echo "1. Seeding the demo database in $TMP_DEMO..."
python3 "$HERE/seed.py" "$TMP_DEMO" "$PORT"

echo "2. Starting poph serve on port $PORT..."
export POPH_CHAVE_CIFRA="demo-cifra-123456789012345678901234"
export ANTIGRAVITY_API_KEY="test"
export POPH_CONFIG="$TMP_DEMO/poph.toml"
export POPH_DADOS_DIRETORIO="$TMP_DEMO/data"
export POPH_PORT="$PORT"

"$BINARY" serve > "$TMP_DEMO/serve.log" 2>&1 &
SERVER_PID=$!

for i in $(seq 1 30); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "Error: poph serve exited unexpectedly. Log:" >&2
        cat "$TMP_DEMO/serve.log" >&2
        exit 1
    fi
    if curl -s "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

echo "3. Warming up the server..."
node "$HERE/capture.mjs" "$TMP_DEMO/warmup" >/dev/null 2>&1 || true

if [ "$RECORD" -eq 1 ]; then
    echo "4. Capturing the definitive baseline screens..."
    node "$HERE/capture.mjs" "$BASE_DIR"

    echo "5. Capturing a second round to prove stability..."
    node "$HERE/capture.mjs" "$TMP_SECOND"

    echo "6. Comparing stability..."
    python3 "$HERE/compare.py" "$BASE_DIR" "$TMP_SECOND"
    echo "Baseline updated and verified successfully in: $BASE_DIR"
else
    echo "4. Capturing screens for validation against the baseline..."
    node "$HERE/capture.mjs" "$TMP_SECOND"

    echo "5. Comparing the capture against the baseline ($BASE_DIR)..."
    python3 "$HERE/compare.py" "$BASE_DIR" "$TMP_SECOND"
    echo "Validation finished successfully against the baseline: $BASE_DIR"
fi

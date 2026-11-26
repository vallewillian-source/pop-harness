#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

FOLDER="$(mktemp -d "${TMPDIR:-/tmp}/poph-eco-demo.XXXXXX")"
clean() {
    rm -rf "$FOLDER"
}
trap clean EXIT

echo "==> building the example plugin and the demo"
cargo build -q -p poph-plugin-eco --bins --examples

echo "==> temporary database: ${FOLDER}"
"${ROOT}/target/debug/examples/plugin-eco-demo" "$FOLDER"

alive="$(pgrep -cf "${ROOT}/target/debug/plugin-eco" || true)"
alive="${alive:-0}"
echo
echo "==> plugin processes still running: ${alive}"
if [ "$alive" -ne 0 ]; then
    echo "FAILED: a plugin process was left running after the demo"
    pgrep -af "${ROOT}/target/debug/plugin-eco" || true
    exit 1
fi
echo "OK: the test conversation went through the echo plugin and nothing was left running"

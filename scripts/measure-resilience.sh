#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

ROUNDS="${1:-3}"
SAMPLES="${2:-40}"
OUT_DIR="target/mx29"
mkdir -p "$OUT_DIR"
CSV="$(pwd)/$OUT_DIR/measurement.csv"

MX29_ROUNDS="$ROUNDS" MX29_SAMPLES="$SAMPLES" MX29_CSV="$CSV" \
  cargo test --locked -p poph-server --test matrix_resilience \
  -- --ignored measure_customer_first --nocapture --test-threads=1

echo "CSV: $CSV"

#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    echo "Uso: capturar.sh"
    echo "Executa o ciclo completo de seed, subida do servidor, captura dupla e comparação da linha de base."
    exit 0
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_DIR="$(cd "$HERE/.." && pwd)"
ROOT="$(cd "$BASE_DIR/../.." && pwd)"
cd "$ROOT"

BINARY="$ROOT/target/debug/poph"
if [ ! -x "$BINARY" ]; then
    echo "poph debug binary não encontrado, tentando release..."
    BINARY="$ROOT/target/release/poph"
fi

if [ ! -x "$BINARY" ]; then
    echo "Compilando poph em modo debug..."
    cargo build --locked -p poph
    BINARY="$ROOT/target/debug/poph"
fi

PORT="${POPH_PORT:-17485}"
TMP_DEMO="$(mktemp -d /tmp/poph-baseline-demo.XXXXXX)"
TMP_SECOND="$(mktemp -d /tmp/poph-baseline-second.XXXXXX)"

cleanup() {
    if [ -n "${SERVER_PID:-}" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$TMP_DEMO" "$TMP_SECOND"
}
trap cleanup EXIT

echo "1. Semeando banco de demonstração em $TMP_DEMO..."
python3 "$HERE/seed.py" "$TMP_DEMO" "$PORT"

echo "2. Subindo poph serve na porta $PORT..."
export POPH_CHAVE_CIFRA="demo-cifra-123456789012345678901234"
export ANTIGRAVITY_API_KEY="test"
export POPH_CONFIG="$TMP_DEMO/poph.toml"
export POPH_DADOS_DIRETORIO="$TMP_DEMO/data"
export POPH_PORT="$PORT"

"$BINARY" serve > "$TMP_DEMO/serve.log" 2>&1 &
SERVER_PID=$!

echo "3. Aquecendo servidor..."
node "$HERE/capture.mjs" "$TMP_DEMO/warmup" >/dev/null 2>&1 || true

echo "4. Capturando telas da linha de base (Run 1)..."
node "$HERE/capture.mjs" "$BASE_DIR"

echo "5. Capturando segunda rodada para prova de estabilidade (Run 2)..."
node "$HERE/capture.mjs" "$TMP_SECOND"

echo "6. Comparando estabilidade..."
python3 "$HERE/compare.py" "$BASE_DIR" "$TMP_SECOND"

echo "Concluído com sucesso! Arquivos gravados em: $BASE_DIR"

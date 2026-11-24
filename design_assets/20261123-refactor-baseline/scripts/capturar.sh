#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    echo "Uso: capturar.sh [--gravar]"
    echo "Executa o ciclo completo de seed, subida do servidor, captura e comparação contra a linha de base."
    echo "  --gravar    Atualiza os arquivos de linha de base em design_assets/20261123-refactor-baseline"
    exit 0
fi

REQUIRED_PORT="17485"
if [ -n "${POPH_PORT:-}" ] && [ "${POPH_PORT}" != "$REQUIRED_PORT" ]; then
    echo "Erro: A linha de base visual do Console exige estritamente a porta $REQUIRED_PORT (POPH_PORT=${POPH_PORT} recusada), pois a tela de login exibe a porta na interface." >&2
    exit 1
fi
PORT="$REQUIRED_PORT"

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

GRAVAR=0
if [ "${1:-}" = "--gravar" ] || [ "${1:-}" = "--update" ] || [ "${1:-}" = "--recapturar" ]; then
    GRAVAR=1
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

for i in $(seq 1 30); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "Erro: poph serve encerrou inesperadamente. Log:" >&2
        cat "$TMP_DEMO/serve.log" >&2
        exit 1
    fi
    if curl -s "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

echo "3. Aquecendo servidor..."
node "$HERE/capture.mjs" "$TMP_DEMO/warmup" >/dev/null 2>&1 || true

if [ "$GRAVAR" -eq 1 ]; then
    echo "4. Capturando telas da linha de base definitiva..."
    node "$HERE/capture.mjs" "$BASE_DIR"

    echo "5. Capturando segunda rodada para prova de estabilidade..."
    node "$HERE/capture.mjs" "$TMP_SECOND"

    echo "6. Comparando estabilidade..."
    python3 "$HERE/compare.py" "$BASE_DIR" "$TMP_SECOND"
    echo "Linha de base atualizada e verificada com sucesso em: $BASE_DIR"
else
    echo "4. Capturando telas para validação contra a linha de base..."
    node "$HERE/capture.mjs" "$TMP_SECOND"

    echo "5. Comparando captura contra a linha de base ($BASE_DIR)..."
    python3 "$HERE/compare.py" "$BASE_DIR" "$TMP_SECOND"
    echo "Validação concluída com sucesso contra a linha de base: $BASE_DIR"
fi

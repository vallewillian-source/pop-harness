#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

MODE=""
ROOMS=""
MESSAGES=1
ROUND=1
PORT_SERVE="${MX30_PORTA_SERVE:-18432}"
PORT_FAKE="${MX30_PORTA_FALSO:-18431}"
LIMIT_S="${MX30_LIMITE_S:-3600}"
ROOT="$(pwd)/target/mx30"
BIN="$(pwd)/target/release/poph"
CSV="$ROOT/memoria.csv"

fail() { echo "error: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --modo) MODE="$2"; shift 2 ;;
        --salas) ROOMS="$2"; shift 2 ;;
        --mensagens) MESSAGES="$2"; shift 2 ;;
        --rodada) ROUND="$2"; shift 2 ;;
        *) fail "unknown argument: $1" ;;
    esac
done

case "$MODE" in
    desligado|ligado|real) ;;
    *) fail "--modo must be desligado, ligado or real" ;;
esac
[ -n "$ROOMS" ] || fail "--salas is required"
[ -x "$BIN" ] || fail "$BIN does not exist: build it first with 'cargo build --release --locked -p poph'"

mkdir -p "$ROOT/logs"
DIR="$ROOT/banco-$MODE-$ROOMS-$MESSAGES-$ROUND"
rm -rf "$DIR"
mkdir -p "$DIR"

port_busy() { ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$1$"; }
port_busy "$PORT_SERVE" && fail "port $PORT_SERVE is in use (set MX30_PORTA_SERVE)"
port_busy "$PORT_FAKE" && fail "port $PORT_FAKE is in use (set MX30_PORTA_FALSO)"

if [ "$MODE" = real ]; then
    CREDS="$HOME/.cache/poph-matrix/credenciais.env"
    [ -f "$CREDS" ] || fail "dev Synapse credentials missing (run matrix-dev.sh subir)"
    set -a; . "$CREDS"; set +a
    AS_TOKEN="${POPH_MATRIX_DEV_HARNESS_AS_TOKEN:-}"
    HS_TOKEN="${POPH_MATRIX_DEV_HARNESS_HS_TOKEN:-}"
    [ -n "$AS_TOKEN" ] && [ -n "$HS_TOKEN" ] || fail "Harness registration tokens missing from the credentials file"
    SERVER_NAME="${POPH_MATRIX_DEV_SERVER_NAME:-matrix-dev.local}"
    HOMESERVER_URL="${POPH_MATRIX_DEV_BASE_URL:-http://127.0.0.1:18008}"
else
    AS_TOKEN="$(python3 -I -c 'import secrets; print(secrets.token_hex(32))')"
    HS_TOKEN="$(python3 -I -c 'import secrets; print(secrets.token_hex(32))')"
    SERVER_NAME="matrix-dev.local"
    HOMESERVER_URL="http://127.0.0.1:$PORT_FAKE"
fi
export POPH_MATRIX_AS_TOKEN="$AS_TOKEN"
export POPH_MATRIX_HS_TOKEN="$HS_TOKEN"

ENABLED=0
[ "$MODE" = ligado ] && ENABLED=1
[ "$MODE" = real ] && ENABLED=1

SEED_START=$(date +%s%N)
MX30_PASTA="$DIR" MX30_SALAS="$ROOMS" MX30_MENSAGENS_POR_SALA="$MESSAGES" MX30_LIGADO="$ENABLED" \
    cargo test --locked -p poph-server --test matrix_medicao_memoria -- --ignored --nocapture \
    semeia_conversas_para_a_medicao_de_memoria >"$ROOT/logs/semeadura-$MODE-$ROOMS-$MESSAGES-$ROUND.log" 2>&1 \
    || fail "seeding failed; see $ROOT/logs/semeadura-$MODE-$ROOMS-$MESSAGES-$ROUND.log"
SEED_END=$(date +%s%N)
SEED_MS=$(( (SEED_END - SEED_START) / 1000000 ))

CONFIG="$DIR/poph.toml"
{
    echo "[servidor]"
    echo "endereco = \"127.0.0.1\""
    echo "porta = $PORT_SERVE"
    echo ""
    echo "[dados]"
    echo "diretorio = \"$DIR/dados\""
    echo ""
    echo "[log]"
    echo "nivel = \"info\""
    echo "formato = \"legivel\""
    if [ "$MODE" != desligado ]; then
        echo ""
        echo "[matrix]"
        echo "habilitado = true"
        echo "homeserver_url = \"$HOMESERVER_URL\""
        echo "server_name = \"$SERVER_NAME\""
        echo "prefixo = \"ph\""
    fi
} >"$CONFIG"

FAKE_PID=""
if [ "$MODE" != real ]; then
    MX30_AS_TOKEN="$AS_TOKEN" python3 -I "$(pwd)/scripts/medicao-memoria/synapse_falso.py" \
        --porta "$PORT_FAKE" >"$ROOT/logs/falso-$MODE-$ROOMS-$MESSAGES-$ROUND.log" 2>&1 &
    FAKE_PID=$!
    for _ in $(seq 1 50); do
        curl -fsS "http://127.0.0.1:$PORT_FAKE/_fake/estatisticas" >/dev/null 2>&1 && break
        sleep 0.1
    done
fi

stats() {
    if [ -n "$FAKE_PID" ]; then
        curl -fsS "http://127.0.0.1:$PORT_FAKE/_fake/estatisticas" 2>/dev/null || echo '{}'
    else
        echo '{}'
    fi
}

mirrored_messages() {
    curl -fsS "http://127.0.0.1:$PORT_SERVE/metrics" 2>/dev/null \
        | awk '/^poph_matrix_mensagens_espelhadas_total/ {print $2; exit}' || true
}

rss_kb() { awk '/^VmRSS:/ {print $2}' "/proc/$1/status" 2>/dev/null || echo 0; }
ticks() { awk '{print $14+$15}' "/proc/$1/stat" 2>/dev/null || echo 0; }
CLK_TCK=$(getconf CLK_TCK)
peak_kb() { awk '/^VmHWM:/ {print $2}' "/proc/$1/status" 2>/dev/null || echo 0; }

LOG_SERVE="$ROOT/logs/serve-$MODE-$ROOMS-$MESSAGES-$ROUND.log"
"$BIN" --config "$CONFIG" serve >"$LOG_SERVE" 2>&1 &
SERVE_PID=$!
stop() {
    kill -TERM "$SERVE_PID" 2>/dev/null || true
    wait "$SERVE_PID" 2>/dev/null || true
    [ -n "$FAKE_PID" ] && kill "$FAKE_PID" 2>/dev/null || true
}
trap stop EXIT

for _ in $(seq 1 300); do
    curl -fsS "http://127.0.0.1:$PORT_SERVE/healthz" >/dev/null 2>&1 && break
    kill -0 "$SERVE_PID" 2>/dev/null || fail "poph serve exited on startup; see $LOG_SERVE"
    sleep 0.1
done
curl -fsS "http://127.0.0.1:$PORT_SERVE/healthz" >/dev/null 2>&1 || fail "poph serve did not answer on /healthz"

START_T0=$(date +%s%N)
RSS_START=$(rss_kb "$SERVE_PID")
TICKS_SERVE0=$(ticks "$SERVE_PID")
TICKS_FAKE0=0
[ -n "$FAKE_PID" ] && TICKS_FAKE0=$(ticks "$FAKE_PID")

EXPECTED=$(( ROOMS * MESSAGES ))
DRAIN_S=0
if [ "$MODE" != desligado ]; then
    while :; do
        if [ -n "$FAKE_PID" ]; then
            SENDS=$(stats | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("envios_unicos",0))' 2>/dev/null || echo 0)
        else
            SENDS=$(mirrored_messages || true)
            SENDS=${SENDS:-0}
        fi
        if [ "$SENDS" -ge "$EXPECTED" ]; then break; fi
        NOW=$(date +%s%N)
        ELAPSED=$(( (NOW - START_T0) / 1000000000 ))
        if [ "$ELAPSED" -ge "$LIMIT_S" ]; then
            fail "limit of ${LIMIT_S}s reached with $SENDS of $EXPECTED messages in the fake"
        fi
        kill -0 "$SERVE_PID" 2>/dev/null || fail "poph serve exited during the throughput run; see $LOG_SERVE"
        sleep 0.2
    done
    END_T1=$(date +%s%N)
    DRAIN_NS=$(( END_T1 - START_T0 ))
    DRAIN_S=$(python3 -I -c "print(round($DRAIN_NS/1e9, 3))")
    TICKS_SERVE1=$(ticks "$SERVE_PID")
    TICKS_FAKE1=0
    [ -n "$FAKE_PID" ] && TICKS_FAKE1=$(ticks "$FAKE_PID")
    CPU_SERVE=$(python3 -I -c "print(round(($TICKS_SERVE1-$TICKS_SERVE0)/$CLK_TCK/$DRAIN_S*100, 1))")
    CPU_FAKE=$(python3 -I -c "print(round(($TICKS_FAKE1-$TICKS_FAKE0)/$CLK_TCK/$DRAIN_S*100, 1))")
fi

SAMPLES=""
for _ in $(seq 1 10); do
    SAMPLES="$SAMPLES $(rss_kb "$SERVE_PID")"
    sleep 1
done
RSS_STEADY=$(echo $SAMPLES | tr ' ' '\n' | python3 -I -c 'import sys,statistics; v=[int(x) for x in sys.stdin.read().split()]; print(int(statistics.median(v)))')
PEAK=$(peak_kb "$SERVE_PID")

if [ -n "$FAKE_PID" ]; then
    STATS=$(stats)
    UNIQUE_SENDS=$(echo "$STATS" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("envios_unicos",0))' 2>/dev/null || echo 0)
    ROOMS_CREATED=$(echo "$STATS" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("salas",0))' 2>/dev/null || echo 0)
    INVITES=$(echo "$STATS" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("convites",0))' 2>/dev/null || echo 0)
    REQUESTS=$(echo "$STATS" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("requisicoes",0))' 2>/dev/null || echo 0)
else
    UNIQUE_SENDS=$(mirrored_messages || true)
    UNIQUE_SENDS=${UNIQUE_SENDS:-0}
    ROOMS_CREATED=nd
    INVITES=nd
    REQUESTS=nd
fi

THROUGHPUT="0"
CPU_SERVE=${CPU_SERVE:-0}
CPU_FAKE=${CPU_FAKE:-0}
if [ "$MODE" != desligado ] && [ "$UNIQUE_SENDS" -gt 0 ]; then
    THROUGHPUT=$(python3 -I -c "print(round($UNIQUE_SENDS/$DRAIN_S, 1))")
fi

if [ ! -f "$CSV" ]; then
    echo "modo,salas,mensagens_por_sala,rodada,rss_inicio_kb,rss_regime_kb,rss_pico_kb,tempo_drenagem_s,msgs_por_s,cpu_serve_pct,cpu_falso_pct,salas_criadas,envios_unicos,convites,requisicoes,semeadura_ms" >"$CSV"
fi
echo "$MODE,$ROOMS,$MESSAGES,$ROUND,$RSS_START,$RSS_STEADY,$PEAK,$DRAIN_S,$THROUGHPUT,$CPU_SERVE,$CPU_FAKE,$ROOMS_CREATED,$UNIQUE_SENDS,$INVITES,$REQUESTS,$SEED_MS" >>"$CSV"

echo "mode=$MODE rooms=$ROOMS messages_per_room=$MESSAGES round=$ROUND"
echo "  RSS at startup: $RSS_START kB | steady: $RSS_STEADY kB | peak: $PEAK kB"
echo "  drain: ${DRAIN_S}s | throughput: $THROUGHPUT msg/s | CPU serve: ${CPU_SERVE}% | CPU fake: ${CPU_FAKE}% | rooms created: $ROOMS_CREATED | unique sends: $UNIQUE_SENDS"
echo "  seeding: ${SEED_MS} ms | CSV: $CSV"

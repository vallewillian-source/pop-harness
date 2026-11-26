#!/usr/bin/env bash
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMPLATES="$APP/tests/matrix"

DIR="${POPH_MATRIX_DEV_DIR:-$HOME/.cache/poph-matrix}"
DATA="$DIR/data"
LOGS="$DIR/logs"
CREDENTIALS="$DIR/credenciais.env"

PORT="${POPH_MATRIX_DEV_PORTA:-18008}"
PORT_PY="${POPH_MATRIX_DEV_PORTA_PY:-18009}"
PORT_H="${POPH_MATRIX_DEV_PORTA_H:-18010}"
IMAGE="${POPH_MATRIX_DEV_IMAGEM:-matrixdotorg/synapse:v1.120.0}"

SERVER_NAME="matrix-dev.local"
CONTAINER="poph-matrix-synapse"
NETWORK="poph-matrix-net"
LABEL="poph.matrix.dev=1"

IP_LAN="$(hostname -I | awk '{print $1}')"
URL_PY="${POPH_MATRIX_DEV_URL_PY:-http://$IP_LAN:$PORT_PY}"
URL_H="${POPH_MATRIX_DEV_URL_H:-http://$IP_LAN:$PORT_H/matrix}"

BASE="http://127.0.0.1:$PORT"

fail() { echo "error: $*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 not found in PATH"
}

mask() { sed -E 's/(_TOKEN[:=])[^ ]*/\1[hidden]/g'; }

port_free() {
    local p="$1"
    if ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$p$"; then
        return 1
    fi
    return 0
}

generate_token() {
    python3 -c 'import secrets; print(secrets.token_hex(32))'
}

render() {
    local template="$1" dest="$2" as_token="$3" hs_token="$4" url="$5"
    [ -f "$template" ] || fail "template not found: $template"
    sed \
        -e "s|__AS_TOKEN__|$as_token|g" \
        -e "s|__HS_TOKEN__|$hs_token|g" \
        -e "s|__AS_URL__|$url|g" \
        "$template" | docker run --rm -i -v "$DATA:/data" --entrypoint sh "$IMAGE" \
        -c 'cat > "$1" && chmod 644 "$1"' _ "$dest"
}

configure_homeserver() {
    docker run --rm -i -v "$DATA:/data" --entrypoint python3 "$IMAGE" - "$PORT" <<'PYCFG'
import re, sys
path, port = "/data/homeserver.yaml", sys.argv[1]
with open(path, encoding="utf-8") as f:
    text = f.read()
if "app_service_config_files:" not in text:
    text = re.sub(
        r"(?m)^server_name:.*$",
        lambda m: m.group(0) + "\n"
        "\n# MX-01: the TWO Application Service registrations live side by side in the same Synapse.\n"
        "app_service_config_files:\n"
        "  - /data/registro-python.yaml\n"
        "  - /data/registro-harness.yaml\n"
        "\ntrusted_key_servers: []\n"
        "suppress_key_server_warning: true",
        text, count=1,
    )
text = re.sub(r"(?m)^public_baseurl:.*$", f'public_baseurl: "http://127.0.0.1:{port}/"', text)
with open(path, "w", encoding="utf-8") as f:
    f.write(text)
PYCFG
}

up() {
    require docker
    require curl
    require python3

    if ! port_free "$PORT"; then
        if ! docker port "$CONTAINER" 2>/dev/null | grep -q ":$PORT$"; then
            fail "port $PORT is already used by another process; pick another one with POPH_MATRIX_DEV_PORTA"
        fi
    fi

    mkdir -p "$DATA" "$LOGS"
    chmod 700 "$DIR"

    local as_py hs_py as_h hs_h
    if [ "${POPH_MATRIX_DEV_REUSAR_TOKENS:-0}" = "1" ] && [ -f "$CREDENTIALS" ]; then
        as_py="$(credential_value POPH_MATRIX_DEV_PYTHON_AS_TOKEN)"
        hs_py="$(credential_value POPH_MATRIX_DEV_PYTHON_HS_TOKEN)"
        as_h="$(credential_value POPH_MATRIX_DEV_HARNESS_AS_TOKEN)"
        hs_h="$(credential_value POPH_MATRIX_DEV_HARNESS_HS_TOKEN)"
        [ -n "$as_h" ] && [ -n "$hs_h" ] || fail "credenciais.env has no Harness tokens; run without POPH_MATRIX_DEV_REUSAR_TOKENS"
        echo "--- reusing the tokens from $CREDENTIALS ---"
    else
        as_py="$(generate_token)"; hs_py="$(generate_token)"
        as_h="$(generate_token)";  hs_h="$(generate_token)"
    fi
    local previous_admin=""
    if [ -f "$CREDENTIALS" ]; then
        previous_admin="$(credential_value POPH_MATRIX_DEV_ADMIN_TOKEN)"
    fi
    umask 077
    {
        echo "# FICTITIOUS credentials of the MX-01 environment (outside git)."
        echo "POPH_MATRIX_DEV_SYNAPSE_DIR=$DATA"
        echo "POPH_MATRIX_DEV_BASE_URL=$BASE"
        echo "POPH_MATRIX_DEV_SERVER_NAME=$SERVER_NAME"
        echo "POPH_MATRIX_DEV_PYTHON_AS_TOKEN=$as_py"
        echo "POPH_MATRIX_DEV_PYTHON_HS_TOKEN=$hs_py"
        echo "POPH_MATRIX_DEV_HARNESS_AS_TOKEN=$as_h"
        echo "POPH_MATRIX_DEV_HARNESS_HS_TOKEN=$hs_h"
        [ -n "$previous_admin" ] && echo "POPH_MATRIX_DEV_ADMIN_TOKEN=$previous_admin"
    } > "$CREDENTIALS"
    chmod 600 "$CREDENTIALS"

    render "$TEMPLATES/registro-python-teste.yaml.template" /data/registro-python.yaml "$as_py" "$hs_py" "$URL_PY"
    if [ -n "${POPH_MATRIX_DEV_REGISTRO_H:-}" ]; then
        [ -f "$POPH_MATRIX_DEV_REGISTRO_H" ] \
            || fail "POPH_MATRIX_DEV_REGISTRO_H: file not found: $POPH_MATRIX_DEV_REGISTRO_H"
        copy_registry "$POPH_MATRIX_DEV_REGISTRO_H" /data/registro-harness.yaml
        echo "--- Harness registration: using the ready file $POPH_MATRIX_DEV_REGISTRO_H ---"
    else
        render "$TEMPLATES/registro-teste.yaml.template" /data/registro-harness.yaml "$as_h" "$hs_h" "$URL_H"
    fi

    if [ ! -f "$DATA/homeserver.yaml" ]; then
        echo "--- generating homeserver.yaml (server_name=$SERVER_NAME) ---"
        docker run --rm \
            -v "$DATA:/data" \
            -e "SYNAPSE_SERVER_NAME=$SERVER_NAME" \
            -e "SYNAPSE_REPORT_STATS=no" \
            --entrypoint /start.py \
            "$IMAGE" generate >/dev/null
        configure_homeserver
    fi
    configure_homeserver

    docker network inspect "$NETWORK" >/dev/null 2>&1 || docker network create --label "$LABEL" "$NETWORK" >/dev/null

    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
        docker rm -f "$CONTAINER" >/dev/null
    fi

    echo "--- starting the isolated Synapse ---"
    docker run -d \
        --name "$CONTAINER" \
        --label "$LABEL" \
        --network "$NETWORK" \
        -p "127.0.0.1:$PORT:8008" \
        -v "$DATA:/data" \
        -e "SYNAPSE_SERVER_NAME=$SERVER_NAME" \
        -e "SYNAPSE_REPORT_STATS=no" \
        "$IMAGE" >/dev/null

    echo "--- waiting for $BASE/_matrix/client/versions ---"
    local i
    for i in $(seq 1 90); do
        if ! docker container inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null | grep -q true; then
            docker logs "$CONTAINER" 2>&1 | tail -30 | mask >&2
            fail "the container $CONTAINER stopped during startup"
        fi
        if curl -fsS -m 2 -o /dev/null "$BASE/_matrix/client/versions" 2>/dev/null; then
            break
        fi
        sleep 2
    done
    curl -fsS -m 2 -o /dev/null "$BASE/_matrix/client/versions" \
        || { docker logs "$CONTAINER" 2>&1 | tail -40 | mask >&2; fail "Synapse did not respond at $BASE"; }

    echo
    echo "Synapse is up:"
    echo "  URL:         $BASE"
    echo "  server_name: $SERVER_NAME"
    echo "  container:   $CONTAINER   network: $NETWORK"
    echo "  data:        $DATA"
    echo "  credentials: $CREDENTIALS  (0600, fictitious)"
    echo
    echo "Application Services loaded (look for this line in the log):"
    docker logs "$CONTAINER" 2>&1 | grep -iE "application service|Loaded application" | tail -5 | mask || true
    echo
    echo "Registration URLs (as seen from the container):"
    echo "  python:  $URL_PY"
    echo "  harness: $URL_H"
}

stop() {
    require docker
    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
        docker stop "$CONTAINER" >/dev/null
        echo "container $CONTAINER stopped (data kept in $DATA)"
    else
        echo "container $CONTAINER does not exist"
    fi
}

clean() {
    require docker
    echo "--- before ---"
    docker ps -a --filter "name=$CONTAINER" --format 'container: {{.Names}} ({{.Status}})'
    docker volume ls --filter "label=$LABEL" --format 'volume: {{.Name}}'

    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
        docker rm -f "$CONTAINER" >/dev/null
        echo "container $CONTAINER removed"
    fi
    if docker network inspect "$NETWORK" >/dev/null 2>&1; then
        docker network rm "$NETWORK" >/dev/null
        echo "network $NETWORK removed"
    fi
    if [ -d "$DATA" ]; then
        docker run --rm -v "$DATA:/data" --entrypoint sh "$IMAGE" \
            -c 'rm -rf /data/* /data/.[!.]* 2>/dev/null; true' || true
    fi
    case "$DIR" in
        "$HOME"/.cache/poph-matrix|"$HOME"/.cache/poph-matrix/*)
            rm -rf "$DIR"
            echo "data and credentials removed: $DIR"
            ;;
        *)
            echo "data kept in $DIR (folder outside the default path; delete it by hand if you want)"
            ;;
    esac

    echo "--- after ---"
    docker ps -a --filter "name=$CONTAINER" --format 'container: {{.Names}} ({{.Status}})'
    docker volume ls --filter "label=$LABEL" --format 'volume: {{.Name}}'
    echo "nothing from another project was touched"
}

copy_registry() {
    local origin="$1" dest="$2"
    docker run --rm -i -v "$DATA:/data" --entrypoint sh "$IMAGE" \
        -c 'cat > "$1" && chmod 644 "$1"' _ "$dest" < "$origin"
}

credential_value() { sed -n "s/^$1=//p" "$CREDENTIALS" | head -1; }

url() {
    echo "URL: $BASE"
    echo "server_name: $SERVER_NAME"
    echo "credentials: $CREDENTIALS"
}

case "${1:-}" in
    subir)  up ;;
    parar)  stop ;;
    limpar) clean ;;
    url)    url ;;
    *)
        echo "usage: $0 {subir|parar|limpar|url}" >&2
        exit 2
        ;;
esac

#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Acceptance run of the Matrix module (MX-31), the phase 1 gate.

Starts a real `poph serve` against the development Synapse from MX-01
(scripts/matrix-dev.sh, port 18008), with the NEW Harness registration (the environment
file, with a `url` ending in `/matrix`, which is the real path) and the OLD Python
registration loaded at the same time, fake Evolution and LLM servers, a real human agent
on Synapse and the current Element served locally.

It does NOT do production, deploy, push or code fixes. It is an environment proof.
No token, password or phone number is printed; the values stay in 0600 files.

Usage:
  bash apps/pop-harness/scripts/acceptance-matrix.sh
  bash apps/pop-harness/scripts/acceptance-matrix.sh --no-element   # no screenshots
  bash apps/pop-harness/scripts/acceptance-matrix.sh --phase2         # includes acceptance A14 (phase 2)
  bash apps/pop-harness/scripts/acceptance-matrix.sh --keep        # keeps the working folder

Variables:
  POPH_ACEITE_DIR       working folder (default ~/.cache/poph-aceite-matrix)
  CHROME_BIN            browser for the screenshots (default google-chrome)
  ACCEPTANCE_SPACE_WAIT  seconds the A6 check waits for the room to join the new space and
                        leave the old one (default 60); the assertion fails when it runs out

Exits 0 when every checked criterion passes, 1 when one fails, and 2 when an environment
prerequisite is missing (Synapse, container, busy port).
USAGE
}

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "$APP/../.." && pwd)"
POPH="${POPH_BIN:-$APP/target/debug/poph}"
AUX="$APP/scripts/acceptance-matrix-aux.py"
ELEMENT_JS="$APP/scripts/acceptance-matrix-element.mjs"

DIR="${POPH_ACEITE_DIR:-$HOME/.cache/poph-aceite-matrix}"
KEEP_DATA=0
NO_ELEMENT=0
PHASE2=0
for arg in "$@"; do
    case "$arg" in
        --keep) KEEP_DATA=1 ;;
        --no-element) NO_ELEMENT=1 ;;
        --phase2) PHASE2=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "argumento desconhecido: $arg" >&2; exit 2 ;;
    esac
done

SYNAPSE="http://127.0.0.1:18008"
SERVER="matrix-dev.local"
IP_LAN="$(hostname -I | awk '{print $1}')"
CONTAINER="poph-matrix-synapse"
SERVE_PORT=18010
LLM_PORT=18401
EVO_PORT=18402
ELEMENT_PORT=18403

fail() { echo "erro: $*" >&2; exit 2; }
warn() { echo "aviso: $*" >&2; }
need() { command -v "$1" >/dev/null 2>&1 || fail "$1 não está no PATH"; }
hide() { sed -E 's/(token|senha|secret|apikey)([^ ]*)[:=" ]+[^ "]+/\1\2 [oculto]/gi'; }

echo "== 0. pré-requisitos =="
need curl
need python3
need docker
[ -x "$POPH" ] || fail "binário ausente: $POPH (rode cargo build -p poph)"
[ -f "$AUX" ] || fail "auxiliar ausente: $AUX"
curl -fsS -m 3 -o /dev/null "$SYNAPSE/_matrix/client/versions" \
    || fail "Synapse de dev fora do ar em $SYNAPSE (suba com: bash apps/pop-harness/scripts/matrix-dev.sh subir)"
docker container inspect "$CONTAINER" >/dev/null 2>&1 \
    || fail "container $CONTAINER não existe (é o Synapse da MX-01)"
if ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$SERVE_PORT$"; then
    fail "porta $SERVE_PORT ocupada; pare o processo antes (é a porta do AS do Harness)"
fi
if ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$ELEMENT_PORT$"; then
    fail "porta $ELEMENT_PORT ocupada; pare o processo antes"
fi

echo "== 1. pasta de trabalho =="
if [ "$KEEP_DATA" = 0 ]; then
    rm -rf "$DIR"
fi
mkdir -p "$DIR/falsos" "$DIR/data" "$DIR/capturas"
chmod 700 "$DIR"

PIDS=()
cleanup() {
    local pid
    for pid in "${PIDS[@]:-}"; do
        kill "$pid" 2>/dev/null || true
    done
    PIDS=()
}
trap cleanup EXIT INT TERM

[ -f "$HOME/.cache/poph-matrix/credenciais.env" ] \
    || fail "credenciais do MX-01 ausentes (~/.cache/poph-matrix/credenciais.env)"
# shellcheck disable=SC1090
. "$HOME/.cache/poph-matrix/credenciais.env"
AS_TOKEN="$POPH_MATRIX_DEV_HARNESS_AS_TOKEN"
HS_TOKEN="$POPH_MATRIX_DEV_HARNESS_HS_TOKEN"
PY_AS_TOKEN="$POPH_MATRIX_DEV_PYTHON_AS_TOKEN"
ADMIN_TOKEN="${POPH_MATRIX_DEV_ADMIN_TOKEN:-}"
if [ -z "$ADMIN_TOKEN" ] && [ -f "$HOME/.cache/poph-matrix/admin-token.env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cache/poph-matrix/admin-token.env"
    ADMIN_TOKEN="${POPH_MATRIX_DEV_ADMIN_TOKEN:-}"
fi
[ -n "$AS_TOKEN" ] && [ -n "$HS_TOKEN" ] || fail "credenciais do MX-01 sem os tokens do Harness"

WEBHOOK_SECRET="aceite-$(python3 -c 'import secrets;print(secrets.token_hex(12))')"
CIPHER_KEY="aceite-$(python3 -c 'import secrets;print(secrets.token_hex(16))')"
AGENT_USER="atendente_$(python3 -c 'import secrets;print(secrets.token_hex(3))')"
AGENT_PASSWORD="aceite-$(python3 -c 'import secrets;print(secrets.token_hex(8))')"
printf '%s' "$AGENT_USER" > "$DIR/atendente.usuario"
printf '%s' "$AGENT_PASSWORD" > "$DIR/atendente.senha"
chmod 600 "$DIR/atendente.senha" "$DIR/atendente.usuario"

cat > "$DIR/ambiente.sh" <<ENV
export POPH_CONFIG="$DIR/poph.toml"
export POPH_CHAVE_CIFRA="$CIPHER_KEY"
export ANTIGRAVITY_API_KEY="chave-falsa-do-llm-falso"
export EVOLUTION_API_KEY="chave-falsa-da-evolution-falsa"
export POPH_EVO_SEGREDO_ACCEPTANCE="$WEBHOOK_SECRET"
export POPH_MATRIX_AS_TOKEN="$AS_TOKEN"
export POPH_MATRIX_HS_TOKEN="$HS_TOKEN"
export POPH_MATRIX_ADMIN_TOKEN="$ADMIN_TOKEN"
export ACCEPTANCE_HUMAN_TOKEN="\${ACCEPTANCE_HUMAN_TOKEN:-}"
export ACCEPTANCE_AS_TOKEN="$AS_TOKEN"
export ACCEPTANCE_WEBHOOK_SECRET="$WEBHOOK_SECRET"
export PATH="$HOME/.cargo/bin:\$PATH"
ENV
chmod 600 "$DIR/ambiente.sh"
# shellcheck disable=SC1091
. "$DIR/ambiente.sh"

RETENTION_TOML=""
[ "$PHASE2" = 1 ] && RETENTION_TOML="midia_retencao_dias = 1"
cat > "$DIR/poph.toml" <<TOML
servidor.endereco = "0.0.0.0"
servidor.porta = $SERVE_PORT
dados.diretorio = "$DIR/data"
log.nivel = "info"
log.formato = "legivel"
llm.provedor = "antigravity2api"
llm.url_base = "http://127.0.0.1:$LLM_PORT/v1"
llm.modelo = "modelo-falso"
llm.timeout_segundos = 30
llm.retentativas = 0
[evolution]
url_base = "http://127.0.0.1:$EVO_PORT"
[matrix]
habilitado = true
homeserver_url = "$SYNAPSE"
server_name = "$SERVER"
prefixo = "ph"
concorrencia = 4
intervalo_ms = 300
${RETENTION_TOML}
TOML

echo "== 2. registro do Synapse (caminho real, com /matrix) =="
CURRENT_REGISTRY="$HOME/.cache/poph-matrix/data/registro-harness.yaml"
[ -f "$CURRENT_REGISTRY" ] \
    || fail "registro do Harness ausente em $CURRENT_REGISTRY (gere com: poph matrix registro --saida ...)"
REGISTRY_URL="$(sed -nE 's/^url:[[:space:]]*"([^"]+)".*/\1/p' "$CURRENT_REGISTRY" | head -1)"
case "$REGISTRY_URL" in
    "http://$IP_LAN:$SERVE_PORT/matrix") : ;;
    *) fail "o registro do Harness aponta para '$REGISTRY_URL'; este roteiro espera 'http://$IP_LAN:$SERVE_PORT/matrix' (o caminho real, com /matrix). Se o IP da estação mudou, regenere com: poph matrix registro --saida $CURRENT_REGISTRY --url http://$IP_LAN:$SERVE_PORT/matrix --forcar" ;;
esac
cp "$CURRENT_REGISTRY" "$DIR/registro-harness.yaml"
chmod 600 "$DIR/registro-harness.yaml"
POPH_MATRIX_DEV_REUSAR_TOKENS=1 \
POPH_MATRIX_DEV_REGISTRO_H="$DIR/registro-harness.yaml" \
    bash "$APP/scripts/matrix-dev.sh" up > "$DIR/synapse-subida.log" 2>&1
grep -q "Loaded application service" "$DIR/synapse-subida.log" \
    || fail "o Synapse não carregou os Application Services; veja $DIR/synapse-subida.log"
echo "registro usado: $REGISTRY_URL (arquivo do ambiente, sem alteração)"

echo "== 3. falsos (LLM e Evolution) =="
python3 "$AUX" fakes "$DIR/falsos" "$LLM_PORT" "$EVO_PORT" > "$DIR/falsos.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 50); do
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$LLM_PORT/" 2>/dev/null && break || true
    sleep 0.2
done

echo "== 4. banco, org, departamentos, agente e token =="
"$POPH" migrate > "$DIR/migrate.txt" 2>&1
"$POPH" --offline org create --name "Acceptance Matrix" --slug "acceptance-matrix" --json > "$DIR/org.json"
"$POPH" --offline --org "acceptance-matrix" department create --name "Comercial" --json > "$DIR/dep-sales.json"
"$POPH" --offline --org "acceptance-matrix" department create --name "Suporte" --json > "$DIR/dep-support.json"
ORG="$(python3 -c "import json;print(json.load(open('$DIR/org.json'))['org']['id'])")"
SALES_DEPT="$(python3 -c "import json;print(json.load(open('$DIR/dep-sales.json'))['department']['id'])")"
SUPPORT_DEPT="$(python3 -c "import json;print(json.load(open('$DIR/dep-support.json'))['department']['id'])")"
"$POPH" --offline --org "acceptance-matrix" agent create --name "Ana Comercial" --slug "ana" \
    --department "$SALES_DEPT" --grouping-window-ms 0 --json > "$DIR/agent.json"
AGENT_ID="$(python3 -c "import json;print(json.load(open('$DIR/agent.json'))['agent']['id'])")"
"$POPH" --offline --org "acceptance-matrix" token create --label "acceptance" --json > "$DIR/token.json"
TOKEN_ADMIN="$(python3 -c "import json;print(json.load(open('$DIR/token.json'))['valor_do_token'])")"

echo "== 5. subida do servidor (1/2: criar a integração) =="
"$POPH" serve > "$DIR/serve-1.log" 2>&1 &
SERVE_PID=$!
PIDS+=("$SERVE_PID")
for _ in $(seq 1 100); do
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" 2>/dev/null && break || true
    sleep 0.2
done
curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" || fail "o servidor não subiu (veja $DIR/serve-1.log)"
curl -sS -o "$DIR/integracao.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/agents/$AGENT_ID/integrations" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H 'Content-Type: application/json' \
    -d '{"kind":"whatsapp"}' > "$DIR/integracao.status"
[ "$(cat "$DIR/integracao.status")" = "201" ] || fail "não criou a integração WhatsApp"
INTEGRATION_ID="$(python3 -c "import json;print(json.load(open('$DIR/integracao.json'))['id'])")"
kill "$SERVE_PID" 2>/dev/null || true
wait "$SERVE_PID" 2>/dev/null || true
PIDS=("${PIDS[@]/$SERVE_PID}")

cat >> "$DIR/poph.toml" <<TOML

[[evolution.instancias]]
org_id = "$ORG"
instancia = "acceptance"
integracao_id = "$INTEGRATION_ID"
segredo_env = "POPH_EVO_SEGREDO_ACCEPTANCE"
TOML

echo "== 6. atendente humano e espaços =="
docker exec "$CONTAINER" register_new_matrix_user -c /data/homeserver.yaml \
    -u "$AGENT_USER" -p "$AGENT_PASSWORD" --no-admin --exists-ok http://localhost:8008 \
    > "$DIR/registro-atendente.log" 2>&1
curl -fsS -X POST "$SYNAPSE/_matrix/client/v3/login" -H 'Content-Type: application/json' \
    -d "{\"type\":\"m.login.password\",\"identifier\":{\"type\":\"m.id.user\",\"user\":\"$AGENT_USER\"},\"password\":\"$AGENT_PASSWORD\"}" \
    > "$DIR/login.json"
HUMAN_TOKEN="$(python3 -c "import json;print(json.load(open('$DIR/login.json'))['access_token'])")"
printf '%s' "$HUMAN_TOKEN" > "$DIR/atendente.token"
chmod 600 "$DIR/atendente.token"
sed -i "s|^export ACCEPTANCE_HUMAN_TOKEN=.*|export ACCEPTANCE_HUMAN_TOKEN=\"$HUMAN_TOKEN\"|" "$DIR/ambiente.sh"
export ACCEPTANCE_HUMAN_TOKEN="$HUMAN_TOKEN"

python3 "$AUX" spaces "$DIR" > "$DIR/espacos.log"
SPACE_SALES="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['comercial']['alias'])")"
SPACE_SUPPORT="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['suporte']['alias'])")"
ROOM_SALES="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['comercial']['sala'])")"
ROOM_SUPPORT="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['suporte']['sala'])")"

echo "== 7. módulo ligado e espaços vinculados =="
"$POPH" --offline --org "acceptance-matrix" matrix ligar --json > "$DIR/ligado.json"
"$POPH" --offline --org "acceptance-matrix" matrix espaco --departamento "$SALES_DEPT" --espaco "$SPACE_SALES" --json > "$DIR/espaco-comercial.json"
"$POPH" --offline --org "acceptance-matrix" matrix espaco --departamento "$SUPPORT_DEPT" --espaco "$SPACE_SUPPORT" --json > "$DIR/espaco-suporte.json"

echo "== 8. subida do servidor (2/2: módulo no ar) =="
"$POPH" serve > "$DIR/serve.log" 2>&1 &
SERVE_PID=$!
PIDS+=("$SERVE_PID")
for _ in $(seq 1 100); do
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" 2>/dev/null && break || true
    sleep 0.2
done
curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" || fail "o servidor não subiu (veja $DIR/serve.log)"

RESULT="$DIR/resultado.txt"
: > "$RESULT"
note() { printf '%s\t%s\n' "$1" "$2" >> "$RESULT"; echo "[$1] $2"; }

ELEMENT_OK=0
start_element() {
    if [ "$NO_ELEMENT" = 1 ]; then
        note A12 "NÃO CONFERIDO: rodada com --no-element"
        return 1
    fi
    if ! command -v node >/dev/null 2>&1 || ! command -v "${CHROME_BIN:-google-chrome}" >/dev/null 2>&1; then
        note A12 "NÃO CONFERIDO: node ou Chrome ausente"
        return 1
    fi
    if [ ! -f "$DIR/element/index.html" ]; then
        echo "  baixando o Element v1.12.35..."
        if ! curl -fsSL -o "$DIR/element.tar.gz" \
            https://github.com/element-hq/element-web/releases/download/v1.12.35/element-v1.12.35.tar.gz; then
            note A12 "NÃO CONFERIDO: sem internet para baixar o Element"
            return 1
        fi
        mkdir -p "$DIR/element"
        tar -xzf "$DIR/element.tar.gz" -C "$DIR/element" --strip-components=1
    fi
    [ -f "$DIR/element/index.html" ] || { note A12 "NÃO CONFERIDO: Element não extraído"; return 1; }
    python3 - > "$DIR/element/config.json" <<JSON
import json
c = json.load(open("$DIR/element/config.sample.json"))
c["default_server_config"] = {"m.homeserver": {"base_url": "$SYNAPSE", "server_name": "$SERVER"}, "m.identity_server": {}}
c["disable_custom_urls"] = True
c["disable_guests"] = True
c["disable_3pid_login"] = True
c["force_verification"] = False
c["room_directory"] = {"servers": ["$SERVER"]}
c["enable_presence_by_hs_url"] = {"$SYNAPSE": False}
c["features"] = {}
json.dump(c, open("$DIR/element/config.json", "w"), indent=2)
JSON
    python3 -m http.server "$ELEMENT_PORT" --bind 127.0.0.1 --directory "$DIR/element" \
        > "$DIR/element.log" 2>&1 &
    PIDS+=($!)
    sleep 1
    ELEMENT_OK=1
    return 0
}

capture_element() {
    [ "$ELEMENT_OK" = 1 ] || return 1
    local mode="$1" room_arg="$2" space_arg="$3" room2_arg="${4:-}"
    CHROME_BIN="${CHROME_BIN:-google-chrome}" \
    ACCEPTANCE_ELEMENT="http://127.0.0.1:$ELEMENT_PORT" \
    ACCEPTANCE_PASSWORD="$(cat "$DIR/atendente.senha")" \
    ACCEPTANCE_USER="$AGENT_USER" \
        node "$ELEMENT_JS" "$mode" "$DIR/capturas" "$room_arg" "$space_arg" "$DIR" "$room2_arg" \
        > "$DIR/element-captura-$mode.log" 2>&1
}

in_space() {
    local output
    output="$(python3 "$AUX" space-parent "$1" "$2")"
    printf '%s' "$output" | grep -q '"_status"' && return 1
    [ "$output" != "{}" ]
}

outside_space() {
    local output
    output="$(python3 "$AUX" space-parent "$1" "$2" 2>/dev/null || true)"
    [ -n "$output" ] || return 2
    [ "$output" = "{}" ] && return 0
    printf '%s' "$output" | grep -qE '"_status"[[:space:]]*:[[:space:]]*404' && return 0
    printf '%s' "$output" | grep -q '"_status"' && return 2
    return 1
}

echo "== 8b. Element servido localmente (sem alterar o app) =="
start_element || true

echo "== 9. A1: mensagem do cliente, sala nomeada e já no espaço =="
python3 "$AUX" webhook "$ORG" "acceptance" ACCEPTANCE-MSG-1 \
    "Bom dia! Queria saber o preço do plano anual." "Ana Souza" > "$DIR/webhook1.json" 2>&1
CONVERSATION=""
for _ in $(seq 1 60); do
    CONVERSATION="$(python3 - "$DIR/data/harness.db" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select conversation_id from matrix_rooms order by created_at limit 1").fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$CONVERSATION" ] && break
    sleep 1
done
[ -n "$CONVERSATION" ] || fail "nenhuma sala foi criada em 60 s (veja $DIR/serve.log)"
ROOM=""
for _ in $(seq 1 60); do
    ROOM="$(python3 - "$DIR/data/harness.db" "$CONVERSATION" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select room_id from matrix_rooms where conversation_id=? and state='ready'", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$ROOM" ] && break
    sleep 1
done
[ -n "$ROOM" ] || fail "a sala da conversa não ficou pronta em 60 s (veja $DIR/serve.log)"
echo "conversa=$CONVERSATION sala=$ROOM" | tee "$DIR/ids.txt"

IN_SPACE=0
for _ in $(seq 1 60); do in_space "$ROOM" "$ROOM_SALES" && { IN_SPACE=1; break; }; sleep 1; done

for _ in $(seq 1 20); do
    python3 "$AUX" join "$ROOM" > "$DIR/entrar-a1.json" 2>&1 || true
    grep -q '^200' "$DIR/entrar-a1.json" && break
    sleep 1
done

for _ in $(seq 1 40); do
    N="$(python3 - "$DIR/data/harness.db" "$CONVERSATION" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select count(*) from messages where conversation_id=? and origin='agent'", (sys.argv[2],)).fetchone()[0])
FIMSQL
)"
    [ "$N" -ge 1 ] && break
    sleep 1
done

python3 "$AUX" room-state "$ROOM" > "$DIR/room-state-a1.json"
NAME="$(python3 -c "
import json
d=json.load(open('$DIR/room-state-a1.json'))
print(d['m.room.name'].get('name',''))
")"
NAME_META="$(python3 -c "
import json
d=json.load(open('$DIR/room-state-a1.json'))
for k,v in d.items():
    if k.endswith('.atendimento.metadata'):
        print(v.get('client_name',''))
        break
")"
if [ "$NAME" = "Ana Souza" ] && [ "$NAME_META" = "Ana Souza" ] && [ "$IN_SPACE" = 1 ]; then
    note A1 "OK: sala 'Ana Souza' (metadata idem) já no espaço Comercial na criação, sem transferência"
else
    note A1 "FALHOU: nome da sala='$NAME' (metadata='$NAME_META'), no espaço Comercial=$IN_SPACE"
fi
if [ "$ELEMENT_OK" = 1 ]; then
    if capture_element a1 "$ROOM" "$ROOM_SALES"; then
        note A12 "OK (A1): capturas a1-sala.png e a1-espaco.png (sala nomeada e sob o espaço Comercial)"
    else
        note A12 "NÃO CONFERIDO: a captura do A1 falhou (veja $DIR/element-captura-a1.log)"
    fi
fi

echo "== 10. A6: transferência pelo departamento move a sala de espaço =="
SPACE_WAIT_S="${ACCEPTANCE_SPACE_WAIT:-60}"
SPACE_CREATED=0
for _ in $(seq 1 "$SPACE_WAIT_S"); do
    SPACE_CREATED="$(python3 - "$DIR/data/harness.db" "$CONVERSATION" "$ROOM_SALES" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select space_room_id from matrix_rooms where conversation_id=?", (sys.argv[2],)).fetchone()
print(1 if r and r[0] == sys.argv[3] else 0)
FIMSQL
)"
    [ "$SPACE_CREATED" = 1 ] && break
    sleep 1
done
[ "$SPACE_CREATED" = 1 ] \
    || echo "aviso: o espaço da criação não foi gravado em ${SPACE_WAIT_S}s; a transferência segue (o A6 vai reprovar se o produto não souber de onde sair)"
python3 "$AUX" transfer "$DIR/data/harness.db" "$CONVERSATION" "$SUPPORT_DEPT" > "$DIR/transferencia.txt"
MOVED=0
OUTSIDE=0
READ_FAILED=0
for _ in $(seq 1 "$SPACE_WAIT_S"); do
    if in_space "$ROOM" "$ROOM_SUPPORT"; then MOVED=1; else MOVED=0; fi
    RC_OUTSIDE=0
    outside_space "$ROOM" "$ROOM_SALES" || RC_OUTSIDE=$?
    case "$RC_OUTSIDE" in
        0) OUTSIDE=1; READ_FAILED=0 ;;
        2) READ_FAILED=1 ;;
        *) OUTSIDE=0; READ_FAILED=0 ;;
    esac
    [ "$MOVED" = 1 ] && [ "$OUTSIDE" = 1 ] && break
    sleep 1
done
if [ "$MOVED" = 1 ]; then
    note A6 "OK: sala no espaço Suporte"
else
    note A6 "FALHOU: a sala não entrou no espaço Suporte em ${SPACE_WAIT_S}s"
fi
if [ "$OUTSIDE" = 1 ]; then
    note A6 "OK: sala fora do espaço Comercial"
else
    OUTSIDE_WITH="$(python3 "$AUX" space-parent "$ROOM" "$ROOM_SALES")"
    if [ "$READ_FAILED" = 1 ]; then
        note A6 "FALHOU: não consegui ler o vínculo da sala com o espaço Comercial em ${SPACE_WAIT_S}s ($OUTSIDE_WITH)"
    else
        note A6 "FALHOU: a sala continua no espaço antigo depois de ${SPACE_WAIT_S}s ($OUTSIDE_WITH)"
    fi
fi



echo "== 11. A2: agent reply =="
python3 "$AUX" attendant-send "$ROOM" "Claro! O plano anual sai por R\$ 1.188, com 50% de desconto no primeiro ano." > "$DIR/envio-atendente.json"
for _ in $(seq 1 40); do
    grep -qF '*[atendente' "$DIR/falsos/evo.jsonl" 2>/dev/null && break
    sleep 0.5
done
if grep -qF '*[atendente' "$DIR/falsos/evo.jsonl" 2>/dev/null; then
    note A2 "OK: resposta do atendente chegou ao Evolution falso com o prefixo do nome"
else
    note A2 "FALHOU: a resposta do atendente não chegou ao Evolution falso"
fi
STATUS="$(python3 - "$DIR/data/harness.db" "$CONVERSATION" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select status from conversations where id=?", (sys.argv[2],)).fetchone()[0])
FIMSQL
)"
[ "$STATUS" = "transferred" ] && note A2 "OK: conversa transferred (o agente parou)" || note A2 "FALHOU: conversa ficou '$STATUS'"

BEFORE="$(wc -l < "$DIR/falsos/evo.jsonl" 2>/dev/null || echo 0)"
python3 "$AUX" webhook "$ORG" "acceptance" ACCEPTANCE-MSG-2 "Perfeito, vou fechar! Pode mandar o link?" "Ana Souza" >/dev/null 2>&1
sleep 5
AFTER="$(wc -l < "$DIR/falsos/evo.jsonl" 2>/dev/null || echo 0)"
[ "$BEFORE" = "$AFTER" ] && note A2 "OK: mensagem seguinte do cliente não gerou resposta" || note A2 "FALHOU: o agente respondeu depois da transferência"

echo "== 12. A7: reabertura, fechamento e conversa nova =="
VERSION="$(curl -sS "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/conversations/$CONVERSATION" \
    -H "Authorization: Bearer $TOKEN_ADMIN" | python3 -c "import json,sys;print(json.load(sys.stdin)['version'])")"
curl -sS -o "$DIR/reopen.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/conversations/$CONVERSATION/reopen" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H "If-Match: $VERSION" > "$DIR/reopen.status"
[ "$(cat "$DIR/reopen.status")" = "200" ] && note A7 "OK: reabertura pela API" || note A7 "FALHOU: reabertura devolveu $(cat "$DIR/reopen.status")"
sleep 4
python3 "$AUX" room-state "$ROOM" > "$DIR/estado-reaberta.json"
VERSION="$(python3 -c "import json;print(json.load(open('$DIR/reopen.json'))['version'])")"
curl -sS -o "$DIR/close.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/conversations/$CONVERSATION/close" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H "If-Match: $VERSION" > "$DIR/close.status"
[ "$(cat "$DIR/close.status")" = "200" ] && note A7 "OK: fechamento pela API" || note A7 "FALHOU: fechamento devolveu $(cat "$DIR/close.status")"
sleep 4
STATUS_META="$(python3 "$AUX" room-state "$ROOM" | python3 -c "import json,sys;print(json.load(sys.stdin)['com.sofacoding.atendimento.metadata'].get('status'))")"
[ "$STATUS_META" = "closed" ] && note A7 "OK: metadata closed" || note A7 "FALHOU: metadata '$STATUS_META'"
python3 "$AUX" webhook "$ORG" "acceptance" ACCEPTANCE-MSG-4 "Oi de novo, nova conversa." "Ana Souza" >/dev/null 2>&1
NEW_COUNT=0
for _ in $(seq 1 40); do
    NEW_COUNT="$(python3 - "$DIR/data/harness.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select count(*) from conversations").fetchone()[0])
PY
)"
    [ "$NEW_COUNT" -ge 2 ] && break
    sleep 1
done
[ "$NEW_COUNT" -ge 2 ] && note A7 "OK: mensagem depois do fechamento abriu conversa nova" || note A7 "FALHOU: não abriu conversa nova"

echo "== 12b. nome do cliente chegando AFTER da sala criada (MX-32/MX-33) =="
PHONE_LATE="5521977776666"
ROOM2=""
CONVERSATION2=""
ACCEPTANCE_PHONE="$PHONE_LATE" python3 "$AUX" webhook "$ORG" "acceptance" ACCEPTANCE-MSG-N1 \
    "Oi, tudo bem?" "" > "$DIR/webhook-nome1.json" 2>&1
for _ in $(seq 1 60); do
    CONVERSATION2="$(python3 - "$DIR/data/harness.db" "$PHONE_LATE" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select id from conversations where external_peer=? order by created_at desc limit 1", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$CONVERSATION2" ] && break
    sleep 1
done
for _ in $(seq 1 60); do
    ROOM2="$(python3 - "$DIR/data/harness.db" "$CONVERSATION2" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select room_id from matrix_rooms where conversation_id=? and state='ready'", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$ROOM2" ] && break
    sleep 1
done
if [ -z "$CONVERSATION2" ] || [ -z "$ROOM2" ]; then
    note MX33 "FALHOU: a conversa/sala sem nome não foi criada (conversa='$CONVERSATION2' sala='$ROOM2')"
else
    for _ in $(seq 1 20); do
        python3 "$AUX" join "$ROOM2" > "$DIR/entrar-nome.json" 2>&1 || true
        grep -q '^200' "$DIR/entrar-nome.json" && break
        sleep 1
    done
    python3 "$AUX" room-state "$ROOM2" > "$DIR/room-name-before.json"
    NAME_BEFORE="$(python3 -c "
import json
d=json.load(open('$DIR/room-name-before.json'))
print(d['m.room.name'].get('name',''))
")"
    GHOST_BEFORE="$(python3 "$AUX" profile "@ph_wa_${PHONE_LATE}:matrix-dev.local" | python3 -c "import json,sys;print(json.load(sys.stdin).get('displayname',''))")"
    NOTICES_BEFORE="$(python3 "$AUX" notices "$ROOM2" | python3 -c "import json,sys;print(json.load(sys.stdin).get('total','error'))")"

    ACCEPTANCE_PHONE="$PHONE_LATE" python3 "$AUX" webhook "$ORG" "acceptance" ACCEPTANCE-MSG-N2 \
        "Sou o Carlos Pereira." "Carlos Pereira" > "$DIR/webhook-nome2.json" 2>&1
    NAME_AFTER=""
    for _ in $(seq 1 60); do
        NAME_AFTER="$(python3 "$AUX" room-state "$ROOM2" | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(d['m.room.name'].get('name',''))
")"
        [ "$NAME_AFTER" = "Carlos Pereira" ] && break
        sleep 1
    done
    python3 "$AUX" room-state "$ROOM2" > "$DIR/room-name-after.json"
    NAME_META2="$(python3 -c "
import json
d=json.load(open('$DIR/room-name-after.json'))
for k,v in d.items():
    if k.endswith('.atendimento.metadata'):
        print(v.get('client_name',''))
        break
")"
    GHOST_AFTER="$(python3 "$AUX" profile "@ph_wa_${PHONE_LATE}:matrix-dev.local" | python3 -c "import json,sys;print(json.load(sys.stdin).get('displayname',''))")"
    NOTICES_AFTER="$(python3 "$AUX" notices "$ROOM2" | python3 -c "import json,sys;print(json.load(sys.stdin).get('total','error'))")"
    if [ "$NAME_BEFORE" != "Carlos Pereira" ] && [ "$NAME_AFTER" = "Carlos Pereira" ] \
        && [ "$NAME_META2" = "Carlos Pereira" ] \
        && [ "$GHOST_AFTER" = "Carlos Pereira (WhatsApp)" ] \
        && [ "$NOTICES_BEFORE" = "$NOTICES_AFTER" ]; then
        note MX33 "OK: sala nasceu '$NAME_BEFORE'/fantasma '$GHOST_BEFORE' e passou a 'Carlos Pereira' (metadata e fantasma idem) sem novo aviso (notices $NOTICES_BEFORE -> $NOTICES_AFTER)"
    else
        note MX33 "FALHOU: antes='$NAME_BEFORE'/fantasma '$GHOST_BEFORE', depois='$NAME_AFTER'/metadata '$NAME_META2'/fantasma '$GHOST_AFTER', notices $NOTICES_BEFORE -> $NOTICES_AFTER"
    fi
fi

echo "== 13. A11: dois registros, sem evento cruzado =="
BEFORE_TX="$(grep -c 'transação do Application Service processada' "$DIR/serve.log" 2>/dev/null || echo 0)"
ROOM_PY="$(SYNAPSE="$SYNAPSE" python3 - "$PY_AS_TOKEN" <<'PY'
import json, os, sys, urllib.request
base = os.environ["SYNAPSE"]; token = sys.argv[1]
def request(method, path, body=None):
    payload = None if body is None else json.dumps(body).encode()
    r = urllib.request.Request(base + path, data=payload, method=method,
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=20) as resp: return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e: return e.code, json.loads(e.read() or b"{}")
s, d = request("POST", "/_matrix/client/v3/createRoom?user_id=%40wa_as_bot%3Amatrix-dev.local",
    {"room_alias_name": "wa_teste_aceite_" + __import__("secrets").token_hex(3), "name": "Sala do Python (teste)", "preset": "private_chat"})
if s != 200: print(""); raise SystemExit(0)
room = d["room_id"]
import urllib.parse
request("PUT", f"/_matrix/client/v3/rooms/{urllib.parse.quote(room, safe='')}/send/m.room.message/py-acceptance?user_id=%40wa_as_bot%3Amatrix-dev.local",
    {"msgtype": "m.text", "body": "evento do registro antigo"})
print(room)
PY
)"
sleep 5
AFTER_TX="$(grep -c 'transação do Application Service processada' "$DIR/serve.log" 2>/dev/null || echo 0)"
if [ -n "$ROOM_PY" ] && [ "$BEFORE_TX" = "$AFTER_TX" ]; then
    note A11 "OK: registro antigo ativo (criou $ROOM_PY) e o Harness não recebeu a transação dele"
else
    note A11 "FALHOU: transações do Harness $BEFORE_TX -> $AFTER_TX (sala do Python '$ROOM_PY')"
fi
grep -q "Loaded application service" "$DIR/synapse-subida.log" \
    && note A11 "OK: o log do Synapse mostra os dois registros carregados"

echo "== 14. A12: Element (capturas finais) =="
if [ "$ELEMENT_OK" = 1 ]; then
    if capture_element final "$ROOM" "$ROOM_SUPPORT" "$ROOM2"; then
        note A12 "OK: capturas finais em $DIR/capturas (sala no Suporte, espaço, home e sala com o nome tardio)"
    else
        note A12 "NÃO CONFERIDO: a captura final falhou (veja $DIR/element-captura-final.log)"
    fi
fi

echo "== 15. A3, A4, A5, A8, A9, A10, A13 =="
note A3 "coberto por cargo test -p poph-server --test matrix_resilience (MX-29)"
note A4 "coberto pela MX-29 (docs/MATRIZ_RESILIENCIA.md)"
note A5 "coberto pela MX-29 (docs/MATRIZ_RESILIENCIA.md)"
note A8 "coberto por cargo test -p poph-server --test matrix_inicio (MX-20)"
note A9 "coberto por cargo test -p poph-server --test matrix_isolation (MX-28)"
note A10 "coberto pela MX-28 (valor-canário) e MX-26"
if [ -f "$APP/docs/MATRIZ_MEMORIA.md" ]; then
    note A13 "medido em docs/MATRIZ_MEMORIA.md (MX-30)"
else
    note A13 "NÃO MEDIDO: docs/MATRIZ_MEMORIA.md ainda não existe (MX-30 em execução)"
fi

if [ "$PHASE2" = 1 ]; then
    FIXTURES="$DIR/fixtures"
    python3 "$AUX" fixtures "$FIXTURES" > "$DIR/fixtures.json"
    AGENT_IMAGE="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['image/png'])")"
    AGENT_PDF="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['application/pdf'])")"
    AGENT_AUDIO="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['audio/ogg'])")"
    AGENT_VIDEO="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['video/mp4'])")"
    PHONE_F2="5521988887777"

    restart_serve() {
        kill "$SERVE_PID" 2>/dev/null || true
        wait "$SERVE_PID" 2>/dev/null || true
        "$POPH" serve > "$DIR/serve-phase2.log" 2>&1 &
        SERVE_PID=$!
        PIDS+=("$SERVE_PID")
        for _ in $(seq 1 150); do
            curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" 2>/dev/null && break
            sleep 0.2
        done
        curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" \
            || fail "o servidor não voltou depois do reinício (veja $DIR/serve-phase2.log)"
    }

    echo "== 16. A14/MX-43: imagem e PDF do cliente aparecem na sala =="
    CONVERSATION3="$(python3 - "$DIR/data/harness.db" "$PHONE_F2" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute(
    "select id from conversations where external_peer=? order by created_at desc limit 1",
    (sys.argv[2],),
).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    ROOM3=""
    for _ in $(seq 1 60); do
        ROOM3="$(python3 - "$DIR/data/harness.db" "$CONVERSATION3" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select room_id from matrix_rooms where conversation_id=? and state='ready'", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
        [ -n "$ROOM3" ] && break
        sleep 1
    done
    if [ -z "$CONVERSATION3" ] || [ -z "$ROOM3" ]; then
        fail "a conversa mais recente não tem sala pronta (conversa='$CONVERSATION3' sala='$ROOM3')"
    fi
    for _ in $(seq 1 20); do
        python3 "$AUX" join "$ROOM3" > "$DIR/entrar-f2.json" 2>&1 || true
        grep -q '^200' "$DIR/entrar-f2.json" && break
        sleep 1
    done

    python3 "$AUX" webhook-media "$ORG" "acceptance" ACCEPTANCE-F2-IMG image \
        "Segue a foto da fachada da loja." "Ana Souza" > "$DIR/f2-img.json" 2>&1 || true
    python3 "$AUX" webhook-media "$ORG" "acceptance" ACCEPTANCE-F2-PDF document \
        "Segue o laudo assinado." "Ana Souza" > "$DIR/f2-pdf.json" 2>&1 || true
    for _ in $(seq 1 60); do
        python3 "$AUX" events "$ROOM3" 80 > "$DIR/events-f2.json"
        if python3 - "$DIR/events-f2.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
types = {e["msgtype"] for e in d}
raise SystemExit(0 if {"m.image", "m.file"} <= types else 1)
PY
        then break; fi
        sleep 1
    done
    RESULT_MX43="$(python3 - "$DIR/events-f2.json" "$PHONE_F2" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
client = "@ph_wa_%s:matrix-dev.local" % sys.argv[2]


def good(e, kind):
    block = list((e.get("bloco_de_anexo") or {}).values())
    return (
        e["msgtype"] == kind
        and e["remetente"] == client
        and (e.get("mxc") or "").startswith("mxc://")
        and bool(block)
        and bool(block[0].get("id"))
        and block[0].get("source") == "whatsapp"
    )


image = [e for e in d if good(e, "m.image")]
file = [e for e in d if good(e, "m.file")]
captions = {e["corpo"] for e in d if e["msgtype"] == "m.text" and e["remetente"] == client}
ok = (
    bool(image)
    and bool(file)
    and "Segue a foto da fachada da loja." in captions
    and "Segue o laudo assinado." in captions
)
if ok:
    print(
        "OK: imagem (%s) e arquivo (%s) na sala pelo fantasma do cliente, com o bloco do anexo "
        "e as duas legendas" % (image[0]["corpo"], file[0]["corpo"])
    )
else:
    print("FALHOU: imagem=%d arquivo=%d legendas=%s" % (len(image), len(file), sorted(captions)))
PY
)"
    note MX43 "$RESULT_MX43"

    echo "== 17. A14/MX-45: audio, video, sticker, contact, location and poll =="
    for media_kind in audio video sticker contact location poll; do
        python3 "$AUX" webhook-type "$ORG" "acceptance" "ACCEPTANCE-F2-${media_kind^^}" "$media_kind" "Ana Souza" \
            > "$DIR/f2-$media_kind.json" 2>&1 || true
    done
    for _ in $(seq 1 60); do
        python3 "$AUX" events "$ROOM3" 100 > "$DIR/events-f2b.json"
        N_NOTICES="$(python3 - "$DIR/events-f2b.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(len([e for e in d if e["msgtype"] == "m.notice" and "ainda não é suportado" in (e["corpo"] or "")]))
PY
)"
        [ "$N_NOTICES" -ge 6 ] && break
        sleep 1
    done
    RESULT_MX45="$(python3 - "$DIR/events-f2b.json" "$DIR/data/harness.db" <<'PY'
import json, sqlite3, sys
d = json.load(open(sys.argv[1]))
notices = {e["corpo"] for e in d if e["msgtype"] == "m.notice"}
expected = {
    "audio": "O cliente enviou um áudio. Esse tipo de mensagem ainda não é suportado.",
    "video": "O cliente enviou um vídeo. Esse tipo de mensagem ainda não é suportado.",
    "figurinha": "O cliente enviou uma figurinha. Esse tipo de mensagem ainda não é suportado.",
    "contato": "O cliente enviou um contato. Esse tipo de mensagem ainda não é suportado.",
    "localizacao": "O cliente enviou uma localização. Esse tipo de mensagem ainda não é suportado.",
    "enquete": "O cliente enviou uma enquete. Esse tipo de mensagem ainda não é suportado.",
}
missing = [kind for kind, text in expected.items() if text not in notices]
con = sqlite3.connect(sys.argv[2])
history = con.execute(
    "select count(*) from messages where text like '%que o sistema não consegue abrir%'"
).fetchone()[0]
if not missing and history >= 6:
    print("OK: seis avisos na sala (m.notice) e %d textos de histórico no turno" % history)
else:
    print("FALHOU: faltando %s; históricos=%d" % (missing, history))
PY
)"
    note MX45 "$RESULT_MX45"

    echo "== 18. A14/MX-41/42: mensagem do aparelho, sem eco duplicado =="
    python3 "$AUX" webhook-device "$ORG" "acceptance" ACCEPTANCE-F2-DEVICE \
        "Mensagem digitada no celular da loja." > "$DIR/f2-device.json" 2>&1 || true
    for _ in $(seq 1 60); do
        python3 "$AUX" events "$ROOM3" 120 > "$DIR/events-f2c.json"
        N_DEVICE="$(python3 - "$DIR/events-f2c.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(len([e for e in d if (e["corpo"] or "") == "Mensagem digitada no celular da loja."]))
PY
)"
        [ "$N_DEVICE" -ge 1 ] && break
        sleep 1
    done
    python3 "$AUX" webhook-device "$ORG" "acceptance" ACCEPTANCE-F2-DEVICE \
        "Mensagem digitada no celular da loja." > "$DIR/f2-device-2.json" 2>&1 || true
    sleep 4
    python3 "$AUX" events "$ROOM3" 120 > "$DIR/events-f2c2.json"
    RESULT_MX41="$(python3 - "$DIR/events-f2c2.json" "$DIR/data/harness.db" "$DIR/f2-device-2.json" "$CONVERSATION3" <<'PY'
import json, sqlite3, sys
d = json.load(open(sys.argv[1]))
target = "Mensagem digitada no celular da loja."
events = [e for e in d if (e["corpo"] or "") == target]
from_device = [e for e in events if e["remetente"].startswith("@ph_ap_")]
redelivery = open(sys.argv[3]).read()
con = sqlite3.connect(sys.argv[2])
status = con.execute("select status from conversations where id=?", (sys.argv[4],)).fetchone()[0]
ok = len(from_device) == 1 and len(events) == 1 and "duplicada" in redelivery and status == "transferred"
if ok:
    print(
        "OK: uma mensagem pelo fantasma do aparelho (%s), reentrega 'duplicada' sem segundo "
        "evento e conversa %s" % (from_device[0]["remetente"], status)
    )
else:
    print(
        "FALHOU: eventos=%d do aparelho=%d reentrega=%s status=%s"
        % (len(events), len(from_device), redelivery.strip()[:80], status)
    )
PY
)"
    note MX41 "$RESULT_MX41"

    echo "== 19. A14/MX-44: imagem e PDF do atendente chegam ao Evolution =="
    BEFORE_MEDIA="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
    python3 "$AUX" attendant-send-media "$ROOM3" "$AGENT_IMAGE" image/png \
        "foto-da-loja.png" > "$DIR/f2-atendente-img.json" 2>&1 || true
    sleep 2
    python3 "$AUX" attendant-send-media "$ROOM3" "$AGENT_PDF" application/pdf \
        "laudo-assinado.pdf" > "$DIR/f2-atendente-pdf.json" 2>&1 || true
    for _ in $(seq 1 40); do
        AFTER_MEDIA="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
        [ "$AFTER_MEDIA" -ge $((BEFORE_MEDIA + 2)) ] && break
        sleep 1
    done
    RESULT_MX44="$(python3 - "$DIR/falsos/evo_midia.jsonl" "$BEFORE_MEDIA" "$DIR/serve-phase2.log" <<'PY'
import json, os, sys
path, before = sys.argv[1], int(sys.argv[2])
lines = []
if os.path.exists(path):
    with open(path, encoding="utf-8") as f:
        lines = [json.loads(l) for l in f if l.strip()]
new_entries = lines[before:]
image = [m for m in new_entries if m.get("mediatype") == "image" and (m.get("caption") or "").startswith("*[atendente")]
document = [m for m in new_entries if m.get("mediatype") == "document" and (m.get("caption") or "").startswith("*[atendente")]
if image and document:
    print(
        "OK: imagem e PDF chegaram ao Evolution falso como %s/%s com a legenda do atendente (%s)"
        % (image[0]["mediatype"], document[0]["mediatype"], image[0]["caption"])
    )
else:
    reason = "sem registro"
    for log in sys.argv[3:]:
        if os.path.exists(log):
            with open(log, errors="replace") as f:
                for line in f:
                    if "arquivo_indisponivel" in line:
                        reason = "arquivo_indisponivel"
    print("FALHOU: o arquivo do atendente não chegou ao Evolution (motivo no log: %s; entradas novas: %d)" % (reason, len(new_entries)))
PY
)"
    if printf '%s' "$RESULT_MX44" | grep -q 'FALHOU'; then
        python3 "$AUX" events "$ROOM3" 120 > "$DIR/events-f2d.json"
        MXC_AND_SENDER="$(python3 - "$DIR/events-f2d.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for e in d:
    if e["msgtype"] in ("m.image", "m.file") and not e["remetente"].startswith("@ph_") and e.get("mxc"):
        print("%s\t%s" % (e["mxc"], e["remetente"]))
        break
PY
)"
        MXC_AGENT="$(printf '%s' "$MXC_AND_SENDER" | cut -f1)"
        SENDER_AGENT="$(printf '%s' "$MXC_AND_SENDER" | cut -f2)"
        if [ -n "$MXC_AGENT" ]; then
            SERV_ID="${MXC_AGENT#mxc://}"
            HTTP_CODE="$(curl -sS -o "$DIR/diag-midia.json" -w '%{http_code}' \
                -H "Authorization: Bearer $AS_TOKEN" \
                "http://127.0.0.1:18008/_matrix/client/v1/media/download/${SERV_ID%%/*}/${SERV_ID#*/}?user_id=$(python3 -c "import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=''))" "$SENDER_AGENT")")"
            ERROR="$(python3 -c "import json;print(json.load(open('$DIR/diag-midia.json')).get('errcode',''))" 2>/dev/null || true)"
            RESULT_MX44="$RESULT_MX44 — diagnóstico: baixar o arquivo com o token do Application Service e o usuário do atendente devolve HTTP $HTTP_CODE $ERROR"
        fi
    fi
    note MX44 "$RESULT_MX44"

    echo "== 20. MX-49: áudio e vídeo do atendente chegam ao Evolution =="
    BEFORE_MEDIA49="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
    python3 "$AUX" events "$ROOM3" 200 > "$DIR/events-mx49-antes.json"
    TXN_AUDIO="acceptance-mx49-audio-$(python3 -c 'import time;print(int(time.time()*1000))')"
    TXN_VIDEO="acceptance-mx49-video-$(python3 -c 'import time;print(int(time.time()*1000))')"
    python3 "$AUX" attendant-send-media "$ROOM3" "$AGENT_AUDIO" audio/ogg \
        "audio-da-loja.ogg" "$TXN_AUDIO" > "$DIR/mx49-audio.json" 2>&1 || true
    sleep 2
    python3 "$AUX" attendant-send-media "$ROOM3" "$AGENT_VIDEO" video/mp4 \
        "video-da-loja.mp4" "$TXN_VIDEO" > "$DIR/mx49-video.json" 2>&1 || true
    for _ in $(seq 1 60); do
        AFTER_MEDIA49="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
        [ "$AFTER_MEDIA49" -ge $((BEFORE_MEDIA49 + 2)) ] && break
        sleep 1
    done
    python3 "$AUX" attendant-send-media "$ROOM3" "$AGENT_AUDIO" audio/ogg \
        "audio-da-loja.ogg" "$TXN_AUDIO" > "$DIR/mx49-audio-redelivery.json" 2>&1 || true
    sleep 5
    python3 "$AUX" events "$ROOM3" 200 > "$DIR/events-mx49-depois.json"
    RESULT_MX49="$(python3 - "$DIR/falsos/evo_midia.jsonl" "$BEFORE_MEDIA49" \
        "$AGENT_AUDIO" "$AGENT_VIDEO" "$DIR/events-mx49-antes.json" \
        "$DIR/events-mx49-depois.json" <<'PY'
import hashlib, json, os, sys

evo, before, audio_path, video_path, before_json, after_json = sys.argv[1:7]
lines = []
if os.path.exists(evo):
    with open(evo, encoding="utf-8") as f:
        lines = [json.loads(l) for l in f if l.strip()]
new_entries = lines[int(before):]
sha_audio = hashlib.sha256(open(audio_path, "rb").read()).hexdigest()
sha_video = hashlib.sha256(open(video_path, "rb").read()).hexdigest()
caption = lambda m: (m.get("caption") or "").startswith("*[atendente")


def found(mime, sha):
    return [
        m
        for m in new_entries
        if m.get("mimetype") == mime
        and m.get("mediatype") == "document"
        and caption(m)
        and m.get("sha256") == sha
        and (m.get("bytes") or 0) > 0
    ]


audio = found("audio/ogg", sha_audio) or found("audio/opus", sha_audio)
video = found("video/mp4", sha_video)


def notices(path):
    with open(path, encoding="utf-8") as f:
        events = json.load(f)
    return [
        e
        for e in events
        if e.get("msgtype") == "m.notice"
        and "ainda não é suportado" in (e.get("corpo") or "")
    ]


before_notices = len(notices(before_json))
after_notices = len(notices(after_json))
new_notices = after_notices - before_notices
redelivery = len(
    [m for m in new_entries if m.get("mimetype") in ("audio/ogg", "audio/opus")]
)
if audio and video and new_notices == 0 and redelivery == 1:
    print(
        "OK: áudio (%s) e vídeo (%s) chegaram ao Evolution falso como documento, com a legenda "
        "do atendente (%s), bytes íntegros e nenhum aviso de tipo não aceito; reentrega da mesma "
        "transação não gerou segundo envio"
        % (audio[0]["mimetype"], video[0]["mimetype"], audio[0]["caption"])
    )
else:
    print(
        "FALHOU: áudio=%d vídeo=%d envios de áudio=%d avisos novos de tipo não aceito=%d "
        "(entradas novas no Evolution: %d)"
        % (len(audio), len(video), redelivery, new_notices, len(new_entries))
    )
PY
)"
    note MX49 "$RESULT_MX49"

    echo "== 21. A14/MX-46: mídia redigida e apagada depois do prazo =="
    if [ -z "$ADMIN_TOKEN" ]; then
        note MX46 "NÃO CONFERIDO: credenciais do MX-01 sem POPH_MATRIX_DEV_ADMIN_TOKEN"
    else
        python3 "$AUX" prepare-expiry "$DIR/data/harness.db" 1 > "$DIR/f2-expiration.json"
        restart_serve
        for _ in $(seq 1 40); do
            PENDING="$(python3 - "$DIR/data/harness.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select count(*) from matrix_midia where redacted_at is null or deleted_at is null").fetchone()[0])
PY
)"
            [ "$PENDING" = "0" ] && break
            sleep 1
        done
        python3 "$AUX" events "$ROOM3" 120 > "$DIR/events-f2e.json"
        VANISHED=0
        TOTAL_MEDIA=0
        for MXC in $(python3 - "$DIR/data/harness.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
for (mxc,) in c.execute("select mxc from matrix_midia"):
    print(mxc)
PY
); do
            TOTAL_MEDIA=$((TOTAL_MEDIA + 1))
            MEDIA_STATE="$(python3 "$AUX" media-exists "$MXC")"
            printf 'apagada\t%s\t%s\n' "$MXC" "$MEDIA_STATE" >> "$DIR/f2-expiration-files.txt"
            printf '%s' "$MEDIA_STATE" | grep -q '"status": 404' && VANISHED=$((VANISHED + 1))
        done
        RESULT_MX46="$(python3 - "$DIR/events-f2e.json" "$DIR/data/harness.db" "$VANISHED" "$TOTAL_MEDIA" "$DIR/serve-phase2.log" <<'PY'
import json, sqlite3, sys
d = json.load(open(sys.argv[1]))
con = sqlite3.connect(sys.argv[2])
pending = con.execute(
    "select count(*) from matrix_midia where redacted_at is null or deleted_at is null"
).fetchone()[0]
blocks = []
for e in d:
    for value in (e.get("bloco_expirado") or {}).values():
        if isinstance(value, dict) and all(k in value for k in ("filename", "mimetype", "size", "sent_at", "kind")):
            blocks.append(value)
names = sorted({b["filename"] for b in blocks})
metrics = ""
try:
    with open(sys.argv[5], errors="replace") as f:
        metrics = f.read()
except OSError:
    pass
expiry_on = "expiração da mídia ligada" in metrics
gone, total = int(sys.argv[3]), int(sys.argv[4])
ok = (
    pending == 0
    and len(names) >= 2
    and all(str(b["sent_at"]).endswith("Z") for b in blocks)
    and gone == total
    and total >= 2
)
if ok:
    print(
        "OK: %d mídias redigidas com o bloco da expiração (%s, sent_at em ISO Z), %d/%d arquivos "
        "devolvem 404 e nenhuma pendente; expiração ligada no log"
        % (len(blocks), ", ".join(names), gone, total)
    )
else:
    print(
        "FALHOU: pendentes=%d redigidas=%d arquivos 404=%d/%d expiração ligada=%s"
        % (pending, len(blocks), gone, total, expiry_on)
    )
PY
)"
        note MX46 "$RESULT_MX46"
    fi

    if grep -qE '^(MX43|MX44|MX45|MX41|MX46)[[:space:]]+FALHOU' "$RESULT"; then
        note A14 "FALHOU: há critério da fase 2 com falha (ver MX43/MX44/MX45/MX41/MX46)"
    elif grep -qE '^(MX43|MX44|MX45|MX41|MX46)[[:space:]]+NÃO CONFERIDO' "$RESULT"; then
        note A14 "PARCIAL: há critério da fase 2 não conferido"
    else
        note A14 "OK: imagem e PDF do cliente na sala (MX-43), áudio etc. viram aviso (MX-45), mensagem do aparelho sem eco (MX-41/42), imagem e PDF do atendente ao cliente (MX-44) e expiração da mídia (MX-46)"
    fi
fi

echo
echo "== resumo =="
cat "$RESULT"
echo
echo "Artefatos: $DIR"
if grep -qE '^(A1|A2|A6|A7|A11|A12|MX33|A14|MX41|MX43|MX44|MX45|MX46|MX49)[[:space:]]+FALHOU' "$RESULT"; then
    echo "Aceite NÃO passou: há critério com falha (ver docs/MATRIZ_ACEITE.md)." >&2
    exit 1
fi
echo "Critérios conferidos passaram."

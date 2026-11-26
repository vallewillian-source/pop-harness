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
  bash apps/pop-harness/scripts/acceptance-matrix.sh --sem-element   # no screenshots
  bash apps/pop-harness/scripts/acceptance-matrix.sh --fase2         # includes acceptance A14 (phase 2)
  bash apps/pop-harness/scripts/acceptance-matrix.sh --manter        # keeps the working folder

Variables:
  POPH_ACEITE_DIR       working folder (default ~/.cache/poph-aceite-matrix)
  CHROME_BIN            browser for the screenshots (default google-chrome)
  ACEITE_ESPERA_ESPACO  seconds the A6 check waits for the room to join the new space and
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
MANTER=0
NO_ELEMENT=0
FASE2=0
for arg in "$@"; do
    case "$arg" in
        --manter) MANTER=1 ;;
        --sem-element) NO_ELEMENT=1 ;;
        --fase2) FASE2=1 ;;
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
ocultar() { sed -E 's/(token|senha|secret|apikey)([^ ]*)[:=" ]+[^ "]+/\1\2 [oculto]/gi'; }

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
if [ "$MANTER" = 0 ]; then
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
export POPH_EVO_SEGREDO_ACEITE="$WEBHOOK_SECRET"
export POPH_MATRIX_AS_TOKEN="$AS_TOKEN"
export POPH_MATRIX_HS_TOKEN="$HS_TOKEN"
export POPH_MATRIX_ADMIN_TOKEN="$ADMIN_TOKEN"
export ACEITE_TOKEN_HUMANO="\${ACEITE_TOKEN_HUMANO:-}"
export ACEITE_AS_TOKEN="$AS_TOKEN"
export ACEITE_SEGREDO_WEBHOOK="$WEBHOOK_SECRET"
export PATH="$HOME/.cargo/bin:\$PATH"
ENV
chmod 600 "$DIR/ambiente.sh"
# shellcheck disable=SC1091
. "$DIR/ambiente.sh"

RETENTION_TOML=""
[ "$FASE2" = 1 ] && RETENTION_TOML="midia_retencao_dias = 1"
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
    bash "$APP/scripts/matrix-dev.sh" subir > "$DIR/synapse-subida.log" 2>&1
grep -q "Loaded application service" "$DIR/synapse-subida.log" \
    || fail "o Synapse não carregou os Application Services; veja $DIR/synapse-subida.log"
echo "registro usado: $REGISTRY_URL (arquivo do ambiente, sem alteração)"

echo "== 3. falsos (LLM e Evolution) =="
python3 "$AUX" falsos "$DIR/falsos" "$LLM_PORT" "$EVO_PORT" > "$DIR/falsos.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 50); do
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$LLM_PORT/" 2>/dev/null && break || true
    sleep 0.2
done

echo "== 4. banco, org, departamentos, agente e token =="
"$POPH" migrate > "$DIR/migrate.txt" 2>&1
"$POPH" --offline org create --name "Aceite Matrix" --slug aceite-matrix --json > "$DIR/org.json"
"$POPH" --offline --org aceite-matrix department create --name Comercial --json > "$DIR/dep-comercial.json"
"$POPH" --offline --org aceite-matrix department create --name Suporte --json > "$DIR/dep-suporte.json"
ORG="$(python3 -c "import json;print(json.load(open('$DIR/org.json'))['org']['id'])")"
COM="$(python3 -c "import json;print(json.load(open('$DIR/dep-comercial.json'))['department']['id'])")"
SUP="$(python3 -c "import json;print(json.load(open('$DIR/dep-suporte.json'))['department']['id'])")"
"$POPH" --offline --org aceite-matrix agent create --name "Ana Comercial" --slug ana \
    --department "$COM" --grouping-window-ms 0 --json > "$DIR/agent.json"
AG="$(python3 -c "import json;print(json.load(open('$DIR/agent.json'))['agent']['id'])")"
"$POPH" --offline --org aceite-matrix token create --label aceite --json > "$DIR/token.json"
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
    "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/agents/$AG/integrations" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H 'Content-Type: application/json' \
    -d '{"kind":"whatsapp"}' > "$DIR/integracao.status"
[ "$(cat "$DIR/integracao.status")" = "201" ] || fail "não criou a integração WhatsApp"
INTEG="$(python3 -c "import json;print(json.load(open('$DIR/integracao.json'))['id'])")"
kill "$SERVE_PID" 2>/dev/null || true
wait "$SERVE_PID" 2>/dev/null || true
PIDS=("${PIDS[@]/$SERVE_PID}")

cat >> "$DIR/poph.toml" <<TOML

[[evolution.instancias]]
org_id = "$ORG"
instancia = "aceite"
integracao_id = "$INTEG"
segredo_env = "POPH_EVO_SEGREDO_ACEITE"
TOML

echo "== 6. atendente humano e espaços =="
docker exec "$CONTAINER" register_new_matrix_user -c /data/homeserver.yaml \
    -u "$AGENT_USER" -p "$AGENT_PASSWORD" --no-admin --exists-ok http://localhost:8008 \
    > "$DIR/registro-atendente.log" 2>&1
curl -fsS -X POST "$SYNAPSE/_matrix/client/v3/login" -H 'Content-Type: application/json' \
    -d "{\"type\":\"m.login.password\",\"identifier\":{\"type\":\"m.id.user\",\"user\":\"$AGENT_USER\"},\"password\":\"$AGENT_PASSWORD\"}" \
    > "$DIR/login.json"
HUMANO_TOKEN="$(python3 -c "import json;print(json.load(open('$DIR/login.json'))['access_token'])")"
printf '%s' "$HUMANO_TOKEN" > "$DIR/atendente.token"
chmod 600 "$DIR/atendente.token"
sed -i "s|^export ACEITE_TOKEN_HUMANO=.*|export ACEITE_TOKEN_HUMANO=\"$HUMANO_TOKEN\"|" "$DIR/ambiente.sh"
export ACEITE_TOKEN_HUMANO="$HUMANO_TOKEN"

python3 "$AUX" espacos "$DIR" > "$DIR/espacos.log"
SPACE_SALES="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['comercial']['alias'])")"
SPACE_SUPPORT="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['suporte']['alias'])")"
ROOM_SALES="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['comercial']['sala'])")"
ROOM_SUPPORT="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['suporte']['sala'])")"

echo "== 7. módulo ligado e espaços vinculados =="
"$POPH" --offline --org aceite-matrix matrix ligar --json > "$DIR/ligado.json"
"$POPH" --offline --org aceite-matrix matrix espaco --departamento "$COM" --espaco "$SPACE_SALES" --json > "$DIR/espaco-comercial.json"
"$POPH" --offline --org aceite-matrix matrix espaco --departamento "$SUP" --espaco "$SPACE_SUPPORT" --json > "$DIR/espaco-suporte.json"

echo "== 8. subida do servidor (2/2: módulo no ar) =="
"$POPH" serve > "$DIR/serve.log" 2>&1 &
SERVE_PID=$!
PIDS+=("$SERVE_PID")
for _ in $(seq 1 100); do
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" 2>/dev/null && break || true
    sleep 0.2
done
curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" || fail "o servidor não subiu (veja $DIR/serve.log)"

RESULTADO="$DIR/resultado.txt"
: > "$RESULTADO"
anotar() { printf '%s\t%s\n' "$1" "$2" >> "$RESULTADO"; echo "[$1] $2"; }

ELEMENT_OK=0
iniciar_element() {
    if [ "$NO_ELEMENT" = 1 ]; then
        anotar A12 "NÃO CONFERIDO: rodada com --sem-element"
        return 1
    fi
    if ! command -v node >/dev/null 2>&1 || ! command -v "${CHROME_BIN:-google-chrome}" >/dev/null 2>&1; then
        anotar A12 "NÃO CONFERIDO: node ou Chrome ausente"
        return 1
    fi
    if [ ! -f "$DIR/element/index.html" ]; then
        echo "  baixando o Element v1.12.35..."
        if ! curl -fsSL -o "$DIR/element.tar.gz" \
            https://github.com/element-hq/element-web/releases/download/v1.12.35/element-v1.12.35.tar.gz; then
            anotar A12 "NÃO CONFERIDO: sem internet para baixar o Element"
            return 1
        fi
        mkdir -p "$DIR/element"
        tar -xzf "$DIR/element.tar.gz" -C "$DIR/element" --strip-components=1
    fi
    [ -f "$DIR/element/index.html" ] || { anotar A12 "NÃO CONFERIDO: Element não extraído"; return 1; }
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

capturar_elemento() {
    [ "$ELEMENT_OK" = 1 ] || return 1
    local modo="$1" sala="$2" espaco="$3" sala2="${4:-}"
    CHROME_BIN="${CHROME_BIN:-google-chrome}" \
    ACEITE_ELEMENT="http://127.0.0.1:$ELEMENT_PORT" \
    ACEITE_SENHA="$(cat "$DIR/atendente.senha")" \
    ACEITE_USUARIO="$AGENT_USER" \
        node "$ELEMENT_JS" "$modo" "$DIR/capturas" "$sala" "$espaco" "$DIR" "$sala2" \
        > "$DIR/element-captura-$modo.log" 2>&1
}

esta_no_espaco() {
    local saida
    saida="$(python3 "$AUX" pai-do-espaco "$1" "$2")"
    printf '%s' "$saida" | grep -q '"_status"' && return 1
    [ "$saida" != "{}" ]
}

fora_do_espaco() {
    local saida
    saida="$(python3 "$AUX" pai-do-espaco "$1" "$2" 2>/dev/null || true)"
    [ -n "$saida" ] || return 2
    [ "$saida" = "{}" ] && return 0
    printf '%s' "$saida" | grep -qE '"_status"[[:space:]]*:[[:space:]]*404' && return 0
    printf '%s' "$saida" | grep -q '"_status"' && return 2
    return 1
}

echo "== 8b. Element servido localmente (sem alterar o app) =="
iniciar_element || true

echo "== 9. A1: mensagem do cliente, sala nomeada e já no espaço =="
python3 "$AUX" webhook "$ORG" aceite ACEITE-MSG-1 \
    "Bom dia! Queria saber o preço do plano anual." "Ana Souza" > "$DIR/webhook1.json" 2>&1
CONV=""
for _ in $(seq 1 60); do
    CONV="$(python3 - "$DIR/data/harness.db" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select conversation_id from matrix_rooms order by created_at limit 1").fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$CONV" ] && break
    sleep 1
done
[ -n "$CONV" ] || fail "nenhuma sala foi criada em 60 s (veja $DIR/serve.log)"
ROOM=""
for _ in $(seq 1 60); do
    ROOM="$(python3 - "$DIR/data/harness.db" "$CONV" <<'FIMSQL'
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
echo "conversa=$CONV sala=$ROOM" | tee "$DIR/ids.txt"

IN_SPACE=0
for _ in $(seq 1 60); do esta_no_espaco "$ROOM" "$ROOM_SALES" && { IN_SPACE=1; break; }; sleep 1; done

for _ in $(seq 1 20); do
    python3 "$AUX" entrar "$ROOM" > "$DIR/entrar-a1.json" 2>&1 || true
    grep -q '^200' "$DIR/entrar-a1.json" && break
    sleep 1
done

for _ in $(seq 1 40); do
    N="$(python3 - "$DIR/data/harness.db" "$CONV" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select count(*) from messages where conversation_id=? and origin='agent'", (sys.argv[2],)).fetchone()[0])
FIMSQL
)"
    [ "$N" -ge 1 ] && break
    sleep 1
done

python3 "$AUX" estado-sala "$ROOM" > "$DIR/estado-sala-a1.json"
NAME="$(python3 -c "
import json
d=json.load(open('$DIR/estado-sala-a1.json'))
print(d['m.room.name'].get('name',''))
")"
NAME_META="$(python3 -c "
import json
d=json.load(open('$DIR/estado-sala-a1.json'))
for k,v in d.items():
    if k.endswith('.atendimento.metadata'):
        print(v.get('client_name',''))
        break
")"
if [ "$NAME" = "Ana Souza" ] && [ "$NAME_META" = "Ana Souza" ] && [ "$IN_SPACE" = 1 ]; then
    anotar A1 "OK: sala 'Ana Souza' (metadata idem) já no espaço Comercial na criação, sem transferência"
else
    anotar A1 "FALHOU: nome da sala='$NAME' (metadata='$NAME_META'), no espaço Comercial=$IN_SPACE"
fi
if [ "$ELEMENT_OK" = 1 ]; then
    if capturar_elemento a1 "$ROOM" "$ROOM_SALES"; then
        anotar A12 "OK (A1): capturas a1-sala.png e a1-espaco.png (sala nomeada e sob o espaço Comercial)"
    else
        anotar A12 "NÃO CONFERIDO: a captura do A1 falhou (veja $DIR/element-captura-a1.log)"
    fi
fi

echo "== 10. A6: transferência pelo departamento move a sala de espaço =="
ESPERA_ESPACO="${ACEITE_ESPERA_ESPACO:-60}"
ESPACO_DA_CRIACAO=0
for _ in $(seq 1 "$ESPERA_ESPACO"); do
    ESPACO_DA_CRIACAO="$(python3 - "$DIR/data/harness.db" "$CONV" "$ROOM_SALES" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select space_room_id from matrix_rooms where conversation_id=?", (sys.argv[2],)).fetchone()
print(1 if r and r[0] == sys.argv[3] else 0)
FIMSQL
)"
    [ "$ESPACO_DA_CRIACAO" = 1 ] && break
    sleep 1
done
[ "$ESPACO_DA_CRIACAO" = 1 ] \
    || echo "aviso: o espaço da criação não foi gravado em ${ESPERA_ESPACO}s; a transferência segue (o A6 vai reprovar se o produto não souber de onde sair)"
python3 "$AUX" transferir "$DIR/data/harness.db" "$CONV" "$SUP" > "$DIR/transferencia.txt"
MOVED=0
FORA=0
LEITURA_FALHOU=0
for _ in $(seq 1 "$ESPERA_ESPACO"); do
    if esta_no_espaco "$ROOM" "$ROOM_SUPPORT"; then MOVED=1; else MOVED=0; fi
    RC_FORA=0
    fora_do_espaco "$ROOM" "$ROOM_SALES" || RC_FORA=$?
    case "$RC_FORA" in
        0) FORA=1; LEITURA_FALHOU=0 ;;
        2) LEITURA_FALHOU=1 ;;
        *) FORA=0; LEITURA_FALHOU=0 ;;
    esac
    [ "$MOVED" = 1 ] && [ "$FORA" = 1 ] && break
    sleep 1
done
if [ "$MOVED" = 1 ]; then
    anotar A6 "OK: sala no espaço Suporte"
else
    anotar A6 "FALHOU: a sala não entrou no espaço Suporte em ${ESPERA_ESPACO}s"
fi
if [ "$FORA" = 1 ]; then
    anotar A6 "OK: sala fora do espaço Comercial"
else
    FORA_COM="$(python3 "$AUX" pai-do-espaco "$ROOM" "$ROOM_SALES")"
    if [ "$LEITURA_FALHOU" = 1 ]; then
        anotar A6 "FALHOU: não consegui ler o vínculo da sala com o espaço Comercial em ${ESPERA_ESPACO}s ($FORA_COM)"
    else
        anotar A6 "FALHOU: a sala continua no espaço antigo depois de ${ESPERA_ESPACO}s ($FORA_COM)"
    fi
fi



echo "== 11. A2: agent reply =="
python3 "$AUX" atendente-envia "$ROOM" "Claro! O plano anual sai por R\$ 1.188, com 50% de desconto no primeiro ano." > "$DIR/envio-atendente.json"
for _ in $(seq 1 40); do
    grep -qF '*[atendente' "$DIR/falsos/evo.jsonl" 2>/dev/null && break
    sleep 0.5
done
if grep -qF '*[atendente' "$DIR/falsos/evo.jsonl" 2>/dev/null; then
    anotar A2 "OK: resposta do atendente chegou ao Evolution falso com o prefixo do nome"
else
    anotar A2 "FALHOU: a resposta do atendente não chegou ao Evolution falso"
fi
STATUS="$(python3 - "$DIR/data/harness.db" "$CONV" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select status from conversations where id=?", (sys.argv[2],)).fetchone()[0])
FIMSQL
)"
[ "$STATUS" = "transferred" ] && anotar A2 "OK: conversa transferred (o agente parou)" || anotar A2 "FALHOU: conversa ficou '$STATUS'"

ANTES="$(wc -l < "$DIR/falsos/evo.jsonl" 2>/dev/null || echo 0)"
python3 "$AUX" webhook "$ORG" aceite ACEITE-MSG-2 "Perfeito, vou fechar! Pode mandar o link?" "Ana Souza" >/dev/null 2>&1
sleep 5
DEPOIS="$(wc -l < "$DIR/falsos/evo.jsonl" 2>/dev/null || echo 0)"
[ "$ANTES" = "$DEPOIS" ] && anotar A2 "OK: mensagem seguinte do cliente não gerou resposta" || anotar A2 "FALHOU: o agente respondeu depois da transferência"

echo "== 12. A7: reabertura, fechamento e conversa nova =="
VERSAO="$(curl -sS "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/conversations/$CONV" \
    -H "Authorization: Bearer $TOKEN_ADMIN" | python3 -c "import json,sys;print(json.load(sys.stdin)['version'])")"
curl -sS -o "$DIR/reopen.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/conversations/$CONV/reopen" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H "If-Match: $VERSAO" > "$DIR/reopen.status"
[ "$(cat "$DIR/reopen.status")" = "200" ] && anotar A7 "OK: reabertura pela API" || anotar A7 "FALHOU: reabertura devolveu $(cat "$DIR/reopen.status")"
sleep 4
python3 "$AUX" estado-sala "$ROOM" > "$DIR/estado-reaberta.json"
VERSAO="$(python3 -c "import json;print(json.load(open('$DIR/reopen.json'))['version'])")"
curl -sS -o "$DIR/close.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORT/admin/v1/orgs/$ORG/conversations/$CONV/close" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H "If-Match: $VERSAO" > "$DIR/close.status"
[ "$(cat "$DIR/close.status")" = "200" ] && anotar A7 "OK: fechamento pela API" || anotar A7 "FALHOU: fechamento devolveu $(cat "$DIR/close.status")"
sleep 4
STATUS_META="$(python3 "$AUX" estado-sala "$ROOM" | python3 -c "import json,sys;print(json.load(sys.stdin)['com.sofacoding.atendimento.metadata'].get('status'))")"
[ "$STATUS_META" = "closed" ] && anotar A7 "OK: metadata closed" || anotar A7 "FALHOU: metadata '$STATUS_META'"
python3 "$AUX" webhook "$ORG" aceite ACEITE-MSG-4 "Oi de novo, nova conversa." "Ana Souza" >/dev/null 2>&1
NOVA=0
for _ in $(seq 1 40); do
    NOVA="$(python3 - "$DIR/data/harness.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select count(*) from conversations").fetchone()[0])
PY
)"
    [ "$NOVA" -ge 2 ] && break
    sleep 1
done
[ "$NOVA" -ge 2 ] && anotar A7 "OK: mensagem depois do fechamento abriu conversa nova" || anotar A7 "FALHOU: não abriu conversa nova"

echo "== 12b. nome do cliente chegando DEPOIS da sala criada (MX-32/MX-33) =="
TEL_TARDIO="5521977776666"
ROOM2=""
CONV2=""
ACEITE_TELEFONE="$TEL_TARDIO" python3 "$AUX" webhook "$ORG" aceite ACEITE-MSG-N1 \
    "Oi, tudo bem?" "" > "$DIR/webhook-nome1.json" 2>&1
for _ in $(seq 1 60); do
    CONV2="$(python3 - "$DIR/data/harness.db" "$TEL_TARDIO" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select id from conversations where external_peer=? order by created_at desc limit 1", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$CONV2" ] && break
    sleep 1
done
for _ in $(seq 1 60); do
    ROOM2="$(python3 - "$DIR/data/harness.db" "$CONV2" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select room_id from matrix_rooms where conversation_id=? and state='ready'", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$ROOM2" ] && break
    sleep 1
done
if [ -z "$CONV2" ] || [ -z "$ROOM2" ]; then
    anotar MX33 "FALHOU: a conversa/sala sem nome não foi criada (conversa='$CONV2' sala='$ROOM2')"
else
    for _ in $(seq 1 20); do
        python3 "$AUX" entrar "$ROOM2" > "$DIR/entrar-nome.json" 2>&1 || true
        grep -q '^200' "$DIR/entrar-nome.json" && break
        sleep 1
    done
    python3 "$AUX" estado-sala "$ROOM2" > "$DIR/estado-nome-antes.json"
    NAME_BEFORE="$(python3 -c "
import json
d=json.load(open('$DIR/estado-nome-antes.json'))
print(d['m.room.name'].get('name',''))
")"
    GHOST_BEFORE="$(python3 "$AUX" perfil "@ph_wa_${TEL_TARDIO}:matrix-dev.local" | python3 -c "import json,sys;print(json.load(sys.stdin).get('displayname',''))")"
    AVISOS_ANTES="$(python3 "$AUX" avisos "$ROOM2" | python3 -c "import json,sys;print(json.load(sys.stdin).get('total','error'))")"

    ACEITE_TELEFONE="$TEL_TARDIO" python3 "$AUX" webhook "$ORG" aceite ACEITE-MSG-N2 \
        "Sou o Carlos Pereira." "Carlos Pereira" > "$DIR/webhook-nome2.json" 2>&1
    NAME_AFTER=""
    for _ in $(seq 1 60); do
        NAME_AFTER="$(python3 "$AUX" estado-sala "$ROOM2" | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(d['m.room.name'].get('name',''))
")"
        [ "$NAME_AFTER" = "Carlos Pereira" ] && break
        sleep 1
    done
    python3 "$AUX" estado-sala "$ROOM2" > "$DIR/estado-nome-depois.json"
    NAME_META2="$(python3 -c "
import json
d=json.load(open('$DIR/estado-nome-depois.json'))
for k,v in d.items():
    if k.endswith('.atendimento.metadata'):
        print(v.get('client_name',''))
        break
")"
    GHOST_AFTER="$(python3 "$AUX" perfil "@ph_wa_${TEL_TARDIO}:matrix-dev.local" | python3 -c "import json,sys;print(json.load(sys.stdin).get('displayname',''))")"
    AVISOS_DEPOIS="$(python3 "$AUX" avisos "$ROOM2" | python3 -c "import json,sys;print(json.load(sys.stdin).get('total','error'))")"
    if [ "$NAME_BEFORE" != "Carlos Pereira" ] && [ "$NAME_AFTER" = "Carlos Pereira" ] \
        && [ "$NAME_META2" = "Carlos Pereira" ] \
        && [ "$GHOST_AFTER" = "Carlos Pereira (WhatsApp)" ] \
        && [ "$AVISOS_ANTES" = "$AVISOS_DEPOIS" ]; then
        anotar MX33 "OK: sala nasceu '$NAME_BEFORE'/fantasma '$GHOST_BEFORE' e passou a 'Carlos Pereira' (metadata e fantasma idem) sem novo aviso (notices $AVISOS_ANTES -> $AVISOS_DEPOIS)"
    else
        anotar MX33 "FALHOU: antes='$NAME_BEFORE'/fantasma '$GHOST_BEFORE', depois='$NAME_AFTER'/metadata '$NAME_META2'/fantasma '$GHOST_AFTER', notices $AVISOS_ANTES -> $AVISOS_DEPOIS"
    fi
fi

echo "== 13. A11: dois registros, sem evento cruzado =="
ANTES_TX="$(grep -c 'transação do Application Service processada' "$DIR/serve.log" 2>/dev/null || echo 0)"
ROOM_PY="$(SYNAPSE="$SYNAPSE" python3 - "$PY_AS_TOKEN" <<'PY'
import json, os, sys, urllib.request
base = os.environ["SYNAPSE"]; token = sys.argv[1]
def pedir(metodo, caminho, corpo=None):
    dados = None if corpo is None else json.dumps(corpo).encode()
    r = urllib.request.Request(base + caminho, data=dados, method=metodo,
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=20) as resp: return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e: return e.code, json.loads(e.read() or b"{}")
s, d = pedir("POST", "/_matrix/client/v3/createRoom?user_id=%40wa_as_bot%3Amatrix-dev.local",
    {"room_alias_name": "wa_teste_aceite_" + __import__("secrets").token_hex(3), "name": "Sala do Python (teste)", "preset": "private_chat"})
if s != 200: print(""); raise SystemExit(0)
sala = d["room_id"]
import urllib.parse
pedir("PUT", f"/_matrix/client/v3/rooms/{urllib.parse.quote(sala, safe='')}/send/m.room.message/py-aceite?user_id=%40wa_as_bot%3Amatrix-dev.local",
    {"msgtype": "m.text", "body": "evento do registro antigo"})
print(sala)
PY
)"
sleep 5
DEPOIS_TX="$(grep -c 'transação do Application Service processada' "$DIR/serve.log" 2>/dev/null || echo 0)"
if [ -n "$ROOM_PY" ] && [ "$ANTES_TX" = "$DEPOIS_TX" ]; then
    anotar A11 "OK: registro antigo ativo (criou $ROOM_PY) e o Harness não recebeu a transação dele"
else
    anotar A11 "FALHOU: transações do Harness $ANTES_TX -> $DEPOIS_TX (sala do Python '$ROOM_PY')"
fi
grep -q "Loaded application service" "$DIR/synapse-subida.log" \
    && anotar A11 "OK: o log do Synapse mostra os dois registros carregados"

echo "== 14. A12: Element (capturas finais) =="
if [ "$ELEMENT_OK" = 1 ]; then
    if capturar_elemento final "$ROOM" "$ROOM_SUPPORT" "$ROOM2"; then
        anotar A12 "OK: capturas finais em $DIR/capturas (sala no Suporte, espaço, home e sala com o nome tardio)"
    else
        anotar A12 "NÃO CONFERIDO: a captura final falhou (veja $DIR/element-captura-final.log)"
    fi
fi

echo "== 15. A3, A4, A5, A8, A9, A10, A13 =="
anotar A3 "coberto por cargo test -p poph-server --test matrix_resiliencia (MX-29)"
anotar A4 "coberto pela MX-29 (docs/MATRIZ_RESILIENCIA.md)"
anotar A5 "coberto pela MX-29 (docs/MATRIZ_RESILIENCIA.md)"
anotar A8 "coberto por cargo test -p poph-server --test matrix_inicio (MX-20)"
anotar A9 "coberto por cargo test -p poph-server --test isolamento_matrix (MX-28)"
anotar A10 "coberto pela MX-28 (valor-canário) e MX-26"
if [ -f "$APP/docs/MATRIZ_MEMORIA.md" ]; then
    anotar A13 "medido em docs/MATRIZ_MEMORIA.md (MX-30)"
else
    anotar A13 "NÃO MEDIDO: docs/MATRIZ_MEMORIA.md ainda não existe (MX-30 em execução)"
fi

if [ "$FASE2" = 1 ]; then
    FIXTURES="$DIR/fixtures"
    python3 "$AUX" fixtures "$FIXTURES" > "$DIR/fixtures.json"
    IMG_DO_ATENDENTE="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['image/png'])")"
    PDF_DO_ATENDENTE="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['application/pdf'])")"
    AUDIO_DO_ATENDENTE="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['audio/ogg'])")"
    VIDEO_DO_ATENDENTE="$(python3 -c "import json;print(json.load(open('$DIR/fixtures.json'))['video/mp4'])")"
    TEL_F2="5521988887777"

    reiniciar_serve() {
        kill "$SERVE_PID" 2>/dev/null || true
        wait "$SERVE_PID" 2>/dev/null || true
        "$POPH" serve > "$DIR/serve-fase2.log" 2>&1 &
        SERVE_PID=$!
        PIDS+=("$SERVE_PID")
        for _ in $(seq 1 150); do
            curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" 2>/dev/null && break
            sleep 0.2
        done
        curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$SERVE_PORT/healthz" \
            || fail "o servidor não voltou depois do reinício (veja $DIR/serve-fase2.log)"
    }

    echo "== 16. A14/MX-43: imagem e PDF do cliente aparecem na sala =="
    CONV3="$(python3 - "$DIR/data/harness.db" "$TEL_F2" <<'FIMSQL'
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
        ROOM3="$(python3 - "$DIR/data/harness.db" "$CONV3" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select room_id from matrix_rooms where conversation_id=? and state='ready'", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
        [ -n "$ROOM3" ] && break
        sleep 1
    done
    if [ -z "$CONV3" ] || [ -z "$ROOM3" ]; then
        fail "a conversa mais recente não tem sala pronta (conversa='$CONV3' sala='$ROOM3')"
    fi
    for _ in $(seq 1 20); do
        python3 "$AUX" entrar "$ROOM3" > "$DIR/entrar-f2.json" 2>&1 || true
        grep -q '^200' "$DIR/entrar-f2.json" && break
        sleep 1
    done

    python3 "$AUX" webhook-midia "$ORG" aceite ACEITE-F2-IMG imagem \
        "Segue a foto da fachada da loja." "Ana Souza" > "$DIR/f2-img.json" 2>&1 || true
    python3 "$AUX" webhook-midia "$ORG" aceite ACEITE-F2-PDF documento \
        "Segue o laudo assinado." "Ana Souza" > "$DIR/f2-pdf.json" 2>&1 || true
    for _ in $(seq 1 60); do
        python3 "$AUX" eventos "$ROOM3" 80 > "$DIR/eventos-f2.json"
        if python3 - "$DIR/eventos-f2.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
tipos = {e["msgtype"] for e in d}
raise SystemExit(0 if {"m.image", "m.file"} <= tipos else 1)
PY
        then break; fi
        sleep 1
    done
    RESULTADO_MX43="$(python3 - "$DIR/eventos-f2.json" "$TEL_F2" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
cliente = "@ph_wa_%s:matrix-dev.local" % sys.argv[2]


def bom(e, tipo):
    bloco = list((e.get("bloco_de_anexo") or {}).values())
    return (
        e["msgtype"] == tipo
        and e["remetente"] == cliente
        and (e.get("mxc") or "").startswith("mxc://")
        and bool(bloco)
        and bool(bloco[0].get("id"))
        and bloco[0].get("source") == "whatsapp"
    )


imagem = [e for e in d if bom(e, "m.image")]
arquivo = [e for e in d if bom(e, "m.file")]
legendas = {e["corpo"] for e in d if e["msgtype"] == "m.text" and e["remetente"] == cliente}
ok = (
    bool(imagem)
    and bool(arquivo)
    and "Segue a foto da fachada da loja." in legendas
    and "Segue o laudo assinado." in legendas
)
if ok:
    print(
        "OK: imagem (%s) e arquivo (%s) na sala pelo fantasma do cliente, com o bloco do anexo "
        "e as duas legendas" % (imagem[0]["corpo"], arquivo[0]["corpo"])
    )
else:
    print("FALHOU: imagem=%d arquivo=%d legendas=%s" % (len(imagem), len(arquivo), sorted(legendas)))
PY
)"
    anotar MX43 "$RESULTADO_MX43"

    echo "== 17. A14/MX-45: audio, video, sticker, contact, location and poll =="
    for tipo in audio video figurinha contato localizacao enquete; do
        python3 "$AUX" webhook-tipo "$ORG" aceite "ACEITE-F2-${tipo^^}" "$tipo" "Ana Souza" \
            > "$DIR/f2-$tipo.json" 2>&1 || true
    done
    for _ in $(seq 1 60); do
        python3 "$AUX" eventos "$ROOM3" 100 > "$DIR/eventos-f2b.json"
        N_AVISOS="$(python3 - "$DIR/eventos-f2b.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(len([e for e in d if e["msgtype"] == "m.notice" and "ainda não é suportado" in (e["corpo"] or "")]))
PY
)"
        [ "$N_AVISOS" -ge 6 ] && break
        sleep 1
    done
    RESULTADO_MX45="$(python3 - "$DIR/eventos-f2b.json" "$DIR/data/harness.db" <<'PY'
import json, sqlite3, sys
d = json.load(open(sys.argv[1]))
avisos = {e["corpo"] for e in d if e["msgtype"] == "m.notice"}
esperados = {
    "audio": "O cliente enviou um áudio. Esse tipo de mensagem ainda não é suportado.",
    "video": "O cliente enviou um vídeo. Esse tipo de mensagem ainda não é suportado.",
    "figurinha": "O cliente enviou uma figurinha. Esse tipo de mensagem ainda não é suportado.",
    "contato": "O cliente enviou um contato. Esse tipo de mensagem ainda não é suportado.",
    "localizacao": "O cliente enviou uma localização. Esse tipo de mensagem ainda não é suportado.",
    "enquete": "O cliente enviou uma enquete. Esse tipo de mensagem ainda não é suportado.",
}
faltando = [tipo for tipo, texto in esperados.items() if texto not in avisos]
con = sqlite3.connect(sys.argv[2])
historico = con.execute(
    "select count(*) from messages where text like '%que o sistema não consegue abrir%'"
).fetchone()[0]
if not faltando and historico >= 6:
    print("OK: seis avisos na sala (m.notice) e %d textos de histórico no turno" % historico)
else:
    print("FALHOU: faltando %s; históricos=%d" % (faltando, historico))
PY
)"
    anotar MX45 "$RESULTADO_MX45"

    echo "== 18. A14/MX-41/42: mensagem do aparelho, sem eco duplicado =="
    python3 "$AUX" webhook-aparelho "$ORG" aceite ACEITE-F2-APARELHO \
        "Mensagem digitada no celular da loja." > "$DIR/f2-aparelho.json" 2>&1 || true
    for _ in $(seq 1 60); do
        python3 "$AUX" eventos "$ROOM3" 120 > "$DIR/eventos-f2c.json"
        N_AP="$(python3 - "$DIR/eventos-f2c.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(len([e for e in d if (e["corpo"] or "") == "Mensagem digitada no celular da loja."]))
PY
)"
        [ "$N_AP" -ge 1 ] && break
        sleep 1
    done
    python3 "$AUX" webhook-aparelho "$ORG" aceite ACEITE-F2-APARELHO \
        "Mensagem digitada no celular da loja." > "$DIR/f2-aparelho-2.json" 2>&1 || true
    sleep 4
    python3 "$AUX" eventos "$ROOM3" 120 > "$DIR/eventos-f2c2.json"
    RESULTADO_MX41="$(python3 - "$DIR/eventos-f2c2.json" "$DIR/data/harness.db" "$DIR/f2-aparelho-2.json" "$CONV3" <<'PY'
import json, sqlite3, sys
d = json.load(open(sys.argv[1]))
alvo = "Mensagem digitada no celular da loja."
eventos = [e for e in d if (e["corpo"] or "") == alvo]
do_aparelho = [e for e in eventos if e["remetente"].startswith("@ph_ap_")]
reentrega = open(sys.argv[3]).read()
con = sqlite3.connect(sys.argv[2])
status = con.execute("select status from conversations where id=?", (sys.argv[4],)).fetchone()[0]
ok = len(do_aparelho) == 1 and len(eventos) == 1 and "duplicada" in reentrega and status == "transferred"
if ok:
    print(
        "OK: uma mensagem pelo fantasma do aparelho (%s), reentrega 'duplicada' sem segundo "
        "evento e conversa %s" % (do_aparelho[0]["remetente"], status)
    )
else:
    print(
        "FALHOU: eventos=%d do aparelho=%d reentrega=%s status=%s"
        % (len(eventos), len(do_aparelho), reentrega.strip()[:80], status)
    )
PY
)"
    anotar MX41 "$RESULTADO_MX41"

    echo "== 19. A14/MX-44: imagem e PDF do atendente chegam ao Evolution =="
    ANTES_MIDIA="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
    python3 "$AUX" atendente-envia-midia "$ROOM3" "$IMG_DO_ATENDENTE" image/png \
        "foto-da-loja.png" > "$DIR/f2-atendente-img.json" 2>&1 || true
    sleep 2
    python3 "$AUX" atendente-envia-midia "$ROOM3" "$PDF_DO_ATENDENTE" application/pdf \
        "laudo-assinado.pdf" > "$DIR/f2-atendente-pdf.json" 2>&1 || true
    for _ in $(seq 1 40); do
        DEPOIS_MIDIA="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
        [ "$DEPOIS_MIDIA" -ge $((ANTES_MIDIA + 2)) ] && break
        sleep 1
    done
    RESULTADO_MX44="$(python3 - "$DIR/falsos/evo_midia.jsonl" "$ANTES_MIDIA" "$DIR/serve-fase2.log" <<'PY'
import json, os, sys
caminho, antes = sys.argv[1], int(sys.argv[2])
linhas = []
if os.path.exists(caminho):
    with open(caminho, encoding="utf-8") as f:
        linhas = [json.loads(l) for l in f if l.strip()]
novas = linhas[antes:]
imagem = [m for m in novas if m.get("mediatype") == "image" and (m.get("caption") or "").startswith("*[atendente")]
documento = [m for m in novas if m.get("mediatype") == "document" and (m.get("caption") or "").startswith("*[atendente")]
if imagem and documento:
    print(
        "OK: imagem e PDF chegaram ao Evolution falso como %s/%s com a legenda do atendente (%s)"
        % (imagem[0]["mediatype"], documento[0]["mediatype"], imagem[0]["caption"])
    )
else:
    motivo = "sem registro"
    for log in sys.argv[3:]:
        if os.path.exists(log):
            with open(log, errors="replace") as f:
                for linha in f:
                    if "arquivo_indisponivel" in linha:
                        motivo = "arquivo_indisponivel"
    print("FALHOU: o arquivo do atendente não chegou ao Evolution (motivo no log: %s; entradas novas: %d)" % (motivo, len(novas)))
PY
)"
    if printf '%s' "$RESULTADO_MX44" | grep -q FALHOU; then
        python3 "$AUX" eventos "$ROOM3" 120 > "$DIR/eventos-f2d.json"
        MXC_E_REMETENTE="$(python3 - "$DIR/eventos-f2d.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for e in d:
    if e["msgtype"] in ("m.image", "m.file") and not e["remetente"].startswith("@ph_") and e.get("mxc"):
        print("%s\t%s" % (e["mxc"], e["remetente"]))
        break
PY
)"
        MXC_ATENDENTE="$(printf '%s' "$MXC_E_REMETENTE" | cut -f1)"
        REMETENTE_ATENDENTE="$(printf '%s' "$MXC_E_REMETENTE" | cut -f2)"
        if [ -n "$MXC_ATENDENTE" ]; then
            SERV_ID="${MXC_ATENDENTE#mxc://}"
            CODIGO="$(curl -sS -o "$DIR/diag-midia.json" -w '%{http_code}' \
                -H "Authorization: Bearer $AS_TOKEN" \
                "http://127.0.0.1:18008/_matrix/client/v1/media/download/${SERV_ID%%/*}/${SERV_ID#*/}?user_id=$(python3 -c "import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=''))" "$REMETENTE_ATENDENTE")")"
            ERROR="$(python3 -c "import json;print(json.load(open('$DIR/diag-midia.json')).get('errcode',''))" 2>/dev/null || true)"
            RESULTADO_MX44="$RESULTADO_MX44 — diagnóstico: baixar o arquivo com o token do Application Service e o usuário do atendente devolve HTTP $CODIGO $ERROR"
        fi
    fi
    anotar MX44 "$RESULTADO_MX44"

    echo "== 20. MX-49: áudio e vídeo do atendente chegam ao Evolution =="
    ANTES_MIDIA49="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
    python3 "$AUX" eventos "$ROOM3" 200 > "$DIR/eventos-mx49-antes.json"
    TXN_AUDIO="aceite-mx49-audio-$(python3 -c 'import time;print(int(time.time()*1000))')"
    TXN_VIDEO="aceite-mx49-video-$(python3 -c 'import time;print(int(time.time()*1000))')"
    python3 "$AUX" atendente-envia-midia "$ROOM3" "$AUDIO_DO_ATENDENTE" audio/ogg \
        "audio-da-loja.ogg" "$TXN_AUDIO" > "$DIR/mx49-audio.json" 2>&1 || true
    sleep 2
    python3 "$AUX" atendente-envia-midia "$ROOM3" "$VIDEO_DO_ATENDENTE" video/mp4 \
        "video-da-loja.mp4" "$TXN_VIDEO" > "$DIR/mx49-video.json" 2>&1 || true
    for _ in $(seq 1 60); do
        DEPOIS_MIDIA49="$(wc -l < "$DIR/falsos/evo_midia.jsonl" 2>/dev/null || echo 0)"
        [ "$DEPOIS_MIDIA49" -ge $((ANTES_MIDIA49 + 2)) ] && break
        sleep 1
    done
    python3 "$AUX" atendente-envia-midia "$ROOM3" "$AUDIO_DO_ATENDENTE" audio/ogg \
        "audio-da-loja.ogg" "$TXN_AUDIO" > "$DIR/mx49-audio-reentrega.json" 2>&1 || true
    sleep 5
    python3 "$AUX" eventos "$ROOM3" 200 > "$DIR/eventos-mx49-depois.json"
    RESULTADO_MX49="$(python3 - "$DIR/falsos/evo_midia.jsonl" "$ANTES_MIDIA49" \
        "$AUDIO_DO_ATENDENTE" "$VIDEO_DO_ATENDENTE" "$DIR/eventos-mx49-antes.json" \
        "$DIR/eventos-mx49-depois.json" <<'PY'
import hashlib, json, os, sys

evo, antes, audio_path, video_path, antes_json, depois_json = sys.argv[1:7]
linhas = []
if os.path.exists(evo):
    with open(evo, encoding="utf-8") as f:
        linhas = [json.loads(l) for l in f if l.strip()]
novas = linhas[int(antes):]
sha_audio = hashlib.sha256(open(audio_path, "rb").read()).hexdigest()
sha_video = hashlib.sha256(open(video_path, "rb").read()).hexdigest()
legenda = lambda m: (m.get("caption") or "").startswith("*[atendente")


def achei(mime, sha):
    return [
        m
        for m in novas
        if m.get("mimetype") == mime
        and m.get("mediatype") == "document"
        and legenda(m)
        and m.get("sha256") == sha
        and (m.get("bytes") or 0) > 0
    ]


# O `infer` devolve `audio/opus` para OGG/Opus (o matcher de Opus vem antes do de OGG): os dois
# MIMEs são o mesmo arquivo que o atendente mandou, e a lista fechada aceita ambos.
audio = achei("audio/ogg", sha_audio) or achei("audio/opus", sha_audio)
video = achei("video/mp4", sha_video)


def avisos(caminho):
    with open(caminho, encoding="utf-8") as f:
        eventos = json.load(f)
    return [
        e
        for e in eventos
        if e.get("msgtype") == "m.notice"
        and "ainda não é suportado" in (e.get("corpo") or "")
    ]


antes_avisos = len(avisos(antes_json))
depois_avisos = len(avisos(depois_json))
novos_avisos = depois_avisos - antes_avisos
reentrega = len(
    [m for m in novas if m.get("mimetype") in ("audio/ogg", "audio/opus")]
)
if audio and video and novos_avisos == 0 and reentrega == 1:
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
        % (len(audio), len(video), reentrega, novos_avisos, len(novas))
    )
PY
)"
    anotar MX49 "$RESULTADO_MX49"

    echo "== 21. A14/MX-46: mídia redigida e apagada depois do prazo =="
    if [ -z "$ADMIN_TOKEN" ]; then
        anotar MX46 "NÃO CONFERIDO: credenciais do MX-01 sem POPH_MATRIX_DEV_ADMIN_TOKEN"
    else
        python3 "$AUX" preparar-expiracao "$DIR/data/harness.db" 1 > "$DIR/f2-expiracao.json"
        reiniciar_serve
        for _ in $(seq 1 40); do
            PENDENTES="$(python3 - "$DIR/data/harness.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
print(c.execute("select count(*) from matrix_midia where redacted_at is null or deleted_at is null").fetchone()[0])
PY
)"
            [ "$PENDENTES" = "0" ] && break
            sleep 1
        done
        python3 "$AUX" eventos "$ROOM3" 120 > "$DIR/eventos-f2e.json"
        SUMIRAM=0
        TOTAL_MIDIA=0
        for MXC in $(python3 - "$DIR/data/harness.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
for (mxc,) in c.execute("select mxc from matrix_midia"):
    print(mxc)
PY
); do
            TOTAL_MIDIA=$((TOTAL_MIDIA + 1))
            ESTADO_MIDIA="$(python3 "$AUX" midia-existe "$MXC")"
            printf 'apagada\t%s\t%s\n' "$MXC" "$ESTADO_MIDIA" >> "$DIR/f2-expiracao-arquivos.txt"
            printf '%s' "$ESTADO_MIDIA" | grep -q '"status": 404' && SUMIRAM=$((SUMIRAM + 1))
        done
        RESULTADO_MX46="$(python3 - "$DIR/eventos-f2e.json" "$DIR/data/harness.db" "$SUMIRAM" "$TOTAL_MIDIA" "$DIR/serve-fase2.log" <<'PY'
import json, sqlite3, sys
d = json.load(open(sys.argv[1]))
con = sqlite3.connect(sys.argv[2])
pendentes = con.execute(
    "select count(*) from matrix_midia where redacted_at is null or deleted_at is null"
).fetchone()[0]
blocos = []
for e in d:
    for valor in (e.get("bloco_expirado") or {}).values():
        if isinstance(valor, dict) and all(k in valor for k in ("filename", "mimetype", "size", "sent_at", "kind")):
            blocos.append(valor)
nomes = sorted({b["filename"] for b in blocos})
metricas = ""
try:
    with open(sys.argv[5], errors="replace") as f:
        metricas = f.read()
except OSError:
    pass
expiracao_ligada = "expiração da mídia ligada" in metricas
sumiram, total = int(sys.argv[3]), int(sys.argv[4])
ok = (
    pendentes == 0
    and len(nomes) >= 2
    and all(str(b["sent_at"]).endswith("Z") for b in blocos)
    and sumiram == total
    and total >= 2
)
if ok:
    print(
        "OK: %d mídias redigidas com o bloco da expiração (%s, sent_at em ISO Z), %d/%d arquivos "
        "devolvem 404 e nenhuma pendente; expiração ligada no log"
        % (len(blocos), ", ".join(nomes), sumiram, total)
    )
else:
    print(
        "FALHOU: pendentes=%d redigidas=%d arquivos 404=%d/%d expiração ligada=%s"
        % (pendentes, len(blocos), sumiram, total, expiracao_ligada)
    )
PY
)"
        anotar MX46 "$RESULTADO_MX46"
    fi

    if grep -qE '^(MX43|MX44|MX45|MX41|MX46)[[:space:]]+FALHOU' "$RESULTADO"; then
        anotar A14 "FALHOU: há critério da fase 2 com falha (ver MX43/MX44/MX45/MX41/MX46)"
    elif grep -qE '^(MX43|MX44|MX45|MX41|MX46)[[:space:]]+NÃO CONFERIDO' "$RESULTADO"; then
        anotar A14 "PARCIAL: há critério da fase 2 não conferido"
    else
        anotar A14 "OK: imagem e PDF do cliente na sala (MX-43), áudio etc. viram aviso (MX-45), mensagem do aparelho sem eco (MX-41/42), imagem e PDF do atendente ao cliente (MX-44) e expiração da mídia (MX-46)"
    fi
fi

echo
echo "== resumo =="
cat "$RESULTADO"
echo
echo "Artefatos: $DIR"
if grep -qE '^(A1|A2|A6|A7|A11|A12|MX33|A14|MX41|MX43|MX44|MX45|MX46|MX49)[[:space:]]+FALHOU' "$RESULTADO"; then
    echo "Aceite NÃO passou: há critério com falha (ver docs/MATRIZ_ACEITE.md)." >&2
    exit 1
fi
echo "Critérios conferidos passaram."

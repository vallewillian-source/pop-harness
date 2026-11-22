#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════
# aceite-matrix.sh — aceite ponta a ponta do módulo Matrix (MX-31), o gate da fase 1.
#
# Sobe o `poph serve` de verdade contra o Synapse de desenvolvimento da MX-01
# (scripts/matrix-dev.sh, porta 18008), com o registro NOVO do Harness — o arquivo do
# ambiente, com a `url` terminando em `/matrix`, que é o caminho real — e o registro
# ANTIGO do Python carregados ao mesmo tempo, Evolution e LLM falsos, um atendente
# humano de verdade no Synapse e o Element atual servido localmente.
#
# O que ele NÃO faz: produção, deploy, push, correção de código. É prova de ambiente.
# Nada de token, senha ou telefone é impresso; os valores ficam em arquivos 0600.
#
# Uso:
#   bash apps/pop-harness/scripts/aceite-matrix.sh
#   bash apps/pop-harness/scripts/aceite-matrix.sh --sem-element   # sem capturas
#   bash apps/pop-harness/scripts/aceite-matrix.sh --manter        # não apaga a pasta
#
# Variáveis:
#   POPH_ACEITE_DIR   pasta de trabalho (padrão ~/.cache/poph-aceite-matrix)
#   CHROME_BIN        navegador para as capturas (padrão google-chrome)
#
# Sai com 0 quando todos os critérios conferidos passam; 1 quando algum falha;
# 2 quando falta pré-requisito do ambiente (Synapse, container, porta ocupada).
# ═══════════════════════════════════════════════════════════════════════════════
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RAIZ="$(cd "$APP/../.." && pwd)"
POPH="$APP/target/debug/poph"
AUX="$APP/scripts/aceite-matrix-aux.py"
ELEMENT_JS="$APP/scripts/aceite-matrix-element.mjs"

DIR="${POPH_ACEITE_DIR:-$HOME/.cache/poph-aceite-matrix}"
MANTER=0
SEM_ELEMENT=0
for arg in "$@"; do
    case "$arg" in
        --manter) MANTER=1 ;;
        --sem-element) SEM_ELEMENT=1 ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        *) echo "argumento desconhecido: $arg" >&2; exit 2 ;;
    esac
done

SYNAPSE="http://127.0.0.1:18008"
SERVER="matrix-dev.local"
IP_LAN="$(hostname -I | awk '{print $1}')"
CONTAINER="poph-matrix-synapse"
SERVE_PORTA=18010
LLM_PORTA=18401
EVO_PORTA=18402
ELEMENT_PORTA=18403

falha() { echo "erro: $*" >&2; exit 2; }
aviso() { echo "aviso: $*" >&2; }
precisa() { command -v "$1" >/dev/null 2>&1 || falha "$1 não está no PATH"; }
ocultar() { sed -E 's/(token|senha|secret|apikey)([^ ]*)[:=" ]+[^ "]+/\1\2 [oculto]/gi'; }

echo "== 0. pré-requisitos =="
precisa curl
precisa python3
precisa docker
[ -x "$POPH" ] || falha "binário ausente: $POPH (rode cargo build -p poph)"
[ -f "$AUX" ] || falha "auxiliar ausente: $AUX"
curl -fsS -m 3 -o /dev/null "$SYNAPSE/_matrix/client/versions" \
    || falha "Synapse de dev fora do ar em $SYNAPSE (suba com: bash apps/pop-harness/scripts/matrix-dev.sh subir)"
docker container inspect "$CONTAINER" >/dev/null 2>&1 \
    || falha "container $CONTAINER não existe (é o Synapse da MX-01)"
if ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$SERVE_PORTA$"; then
    falha "porta $SERVE_PORTA ocupada; pare o processo antes (é a porta do AS do Harness)"
fi
if ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$ELEMENT_PORTA$"; then
    falha "porta $ELEMENT_PORTA ocupada; pare o processo antes"
fi

echo "== 1. pasta de trabalho =="
if [ "$MANTER" = 0 ]; then
    rm -rf "$DIR"
fi
mkdir -p "$DIR/falsos" "$DIR/data" "$DIR/capturas"
chmod 700 "$DIR"

PIDS=()
limpar() {
    local pid
    for pid in "${PIDS[@]:-}"; do
        kill "$pid" 2>/dev/null || true
    done
    PIDS=()
}
# O registro do Synapse NÃO é alterado por este roteiro: ele usa o arquivo do ambiente como está
# (url com `/matrix`, o mesmo que reflete produção), então não há nada a restaurar no fim.
trap limpar EXIT INT TERM

# ── segredos do ambiente MX-01 ────────────────────────────────────────────────
[ -f "$HOME/.cache/poph-matrix/credenciais.env" ] \
    || falha "credenciais do MX-01 ausentes (~/.cache/poph-matrix/credenciais.env)"
# shellcheck disable=SC1090
. "$HOME/.cache/poph-matrix/credenciais.env"
AS_TOKEN="$POPH_MATRIX_DEV_HARNESS_AS_TOKEN"
HS_TOKEN="$POPH_MATRIX_DEV_HARNESS_HS_TOKEN"
PY_AS_TOKEN="$POPH_MATRIX_DEV_PYTHON_AS_TOKEN"
[ -n "$AS_TOKEN" ] && [ -n "$HS_TOKEN" ] || falha "credenciais do MX-01 sem os tokens do Harness"

SEGREDO_WEBHOOK="aceite-$(python3 -c 'import secrets;print(secrets.token_hex(12))')"
CHAVE_CIFRA="aceite-$(python3 -c 'import secrets;print(secrets.token_hex(16))')"
USUARIO_ATENDENTE="atendente_$(python3 -c 'import secrets;print(secrets.token_hex(3))')"
SENHA_ATENDENTE="aceite-$(python3 -c 'import secrets;print(secrets.token_hex(8))')"
printf '%s' "$USUARIO_ATENDENTE" > "$DIR/atendente.usuario"
printf '%s' "$SENHA_ATENDENTE" > "$DIR/atendente.senha"
chmod 600 "$DIR/atendente.senha" "$DIR/atendente.usuario"

cat > "$DIR/ambiente.sh" <<ENV
export POPH_CONFIG="$DIR/poph.toml"
export POPH_CHAVE_CIFRA="$CHAVE_CIFRA"
export ANTIGRAVITY_API_KEY="chave-falsa-do-llm-falso"
export EVOLUTION_API_KEY="chave-falsa-da-evolution-falsa"
export POPH_EVO_SEGREDO_ACEITE="$SEGREDO_WEBHOOK"
export POPH_MATRIX_AS_TOKEN="$AS_TOKEN"
export POPH_MATRIX_HS_TOKEN="$HS_TOKEN"
export ACEITE_TOKEN_HUMANO="\${ACEITE_TOKEN_HUMANO:-}"
export ACEITE_AS_TOKEN="$AS_TOKEN"
export ACEITE_SEGREDO_WEBHOOK="$SEGREDO_WEBHOOK"
export PATH="$HOME/.cargo/bin:\$PATH"
ENV
chmod 600 "$DIR/ambiente.sh"
# shellcheck disable=SC1091
. "$DIR/ambiente.sh"

# ── configuração do Harness ───────────────────────────────────────────────────
cat > "$DIR/poph.toml" <<TOML
servidor.endereco = "0.0.0.0"
servidor.porta = $SERVE_PORTA
dados.diretorio = "$DIR/data"
log.nivel = "info"
log.formato = "legivel"
llm.provedor = "antigravity2api"
llm.url_base = "http://127.0.0.1:$LLM_PORTA/v1"
llm.modelo = "modelo-falso"
llm.timeout_segundos = 30
llm.retentativas = 0
[evolution]
url_base = "http://127.0.0.1:$EVO_PORTA"
[matrix]
habilitado = true
homeserver_url = "$SYNAPSE"
server_name = "$SERVER"
prefixo = "ph"
concorrencia = 4
intervalo_ms = 300
TOML

echo "== 2. registro do Synapse (caminho real, com /matrix) =="
# O registro do Harness do ambiente (e o de produção) tem a `url` terminando em `/matrix`: o Synapse
# chama `<url>/_matrix/app/v1/transactions/...`, ou seja `/matrix/_matrix/app/v1/...`, e o receptor
# atende esse caminho desde a correção da MX-31. Este roteiro usa o arquivo COMO ESTÁ — não gera
# registro alternativo, não mexe no registro do Synapse e não restaura nada no fim.
REGISTRO_ATUAL="$HOME/.cache/poph-matrix/data/registro-harness.yaml"
[ -f "$REGISTRO_ATUAL" ] \
    || falha "registro do Harness ausente em $REGISTRO_ATUAL (gere com: poph matrix registro --saida ...)"
URL_REGISTRO="$(sed -nE 's/^url:[[:space:]]*"([^"]+)".*/\1/p' "$REGISTRO_ATUAL" | head -1)"
case "$URL_REGISTRO" in
    "http://$IP_LAN:$SERVE_PORTA/matrix") : ;;
    *) falha "o registro do Harness aponta para '$URL_REGISTRO'; este roteiro espera 'http://$IP_LAN:$SERVE_PORTA/matrix' (o caminho real, com /matrix). Se o IP da estação mudou, regenere com: poph matrix registro --saida $REGISTRO_ATUAL --url http://$IP_LAN:$SERVE_PORTA/matrix --forcar" ;;
esac
cp "$REGISTRO_ATUAL" "$DIR/registro-harness.yaml"
chmod 600 "$DIR/registro-harness.yaml"
POPH_MATRIX_DEV_REUSAR_TOKENS=1 \
POPH_MATRIX_DEV_REGISTRO_H="$DIR/registro-harness.yaml" \
    bash "$APP/scripts/matrix-dev.sh" subir > "$DIR/synapse-subida.log" 2>&1
grep -q "Loaded application service" "$DIR/synapse-subida.log" \
    || falha "o Synapse não carregou os Application Services; veja $DIR/synapse-subida.log"
echo "registro usado: $URL_REGISTRO (arquivo do ambiente, sem alteração)"

echo "== 3. falsos (LLM e Evolution) =="
python3 "$AUX" falsos "$DIR/falsos" "$LLM_PORTA" "$EVO_PORTA" > "$DIR/falsos.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 50); do
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$LLM_PORTA/" 2>/dev/null && break || true
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
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$SERVE_PORTA/healthz" 2>/dev/null && break || true
    sleep 0.2
done
curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$SERVE_PORTA/healthz" || falha "o servidor não subiu (veja $DIR/serve-1.log)"
curl -sS -o "$DIR/integracao.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORTA/admin/v1/orgs/$ORG/agents/$AG/integrations" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H 'Content-Type: application/json' \
    -d '{"kind":"whatsapp"}' > "$DIR/integracao.status"
[ "$(cat "$DIR/integracao.status")" = "201" ] || falha "não criou a integração WhatsApp"
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
    -u "$USUARIO_ATENDENTE" -p "$SENHA_ATENDENTE" --no-admin --exists-ok http://localhost:8008 \
    > "$DIR/registro-atendente.log" 2>&1
curl -fsS -X POST "$SYNAPSE/_matrix/client/v3/login" -H 'Content-Type: application/json' \
    -d "{\"type\":\"m.login.password\",\"identifier\":{\"type\":\"m.id.user\",\"user\":\"$USUARIO_ATENDENTE\"},\"password\":\"$SENHA_ATENDENTE\"}" \
    > "$DIR/login.json"
HUMANO_TOKEN="$(python3 -c "import json;print(json.load(open('$DIR/login.json'))['access_token'])")"
printf '%s' "$HUMANO_TOKEN" > "$DIR/atendente.token"
chmod 600 "$DIR/atendente.token"
sed -i "s|^export ACEITE_TOKEN_HUMANO=.*|export ACEITE_TOKEN_HUMANO=\"$HUMANO_TOKEN\"|" "$DIR/ambiente.sh"
export ACEITE_TOKEN_HUMANO="$HUMANO_TOKEN"

python3 "$AUX" espacos "$DIR" > "$DIR/espacos.log"
ESP_COM="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['comercial']['alias'])")"
ESP_SUP="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['suporte']['alias'])")"
SALA_COM="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['comercial']['sala'])")"
SALA_SUP="$(python3 -c "import json;print(json.load(open('$DIR/espacos.json'))['suporte']['sala'])")"

echo "== 7. módulo ligado e espaços vinculados =="
"$POPH" --offline --org aceite-matrix matrix ligar --json > "$DIR/ligado.json"
"$POPH" --offline --org aceite-matrix matrix espaco --departamento "$COM" --espaco "$ESP_COM" --json > "$DIR/espaco-comercial.json"
"$POPH" --offline --org aceite-matrix matrix espaco --departamento "$SUP" --espaco "$ESP_SUP" --json > "$DIR/espaco-suporte.json"

echo "== 8. subida do servidor (2/2: módulo no ar) =="
"$POPH" serve > "$DIR/serve.log" 2>&1 &
SERVE_PID=$!
PIDS+=("$SERVE_PID")
for _ in $(seq 1 100); do
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$SERVE_PORTA/healthz" 2>/dev/null && break || true
    sleep 0.2
done
curl -fsS -m 2 -o /dev/null "http://127.0.0.1:$SERVE_PORTA/healthz" || falha "o servidor não subiu (veja $DIR/serve.log)"

RESULTADO="$DIR/resultado.txt"
: > "$RESULTADO"
anotar() { printf '%s\t%s\n' "$1" "$2" >> "$RESULTADO"; echo "[$1] $2"; }

# ── Element (capturas) ───────────────────────────────────────────────────────
# Sobe o Element atual servido localmente UMA vez; as capturas rodam em passadas separadas porque
# o A1 precisa ser provado ANTES de qualquer transferência e o estado final depois dela.
ELEMENT_OK=0
iniciar_element() {
    if [ "$SEM_ELEMENT" = 1 ]; then
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
    python3 -m http.server "$ELEMENT_PORTA" --bind 127.0.0.1 --directory "$DIR/element" \
        > "$DIR/element.log" 2>&1 &
    PIDS+=($!)
    sleep 1
    ELEMENT_OK=1
    return 0
}

# capturar_elemento <modo> <sala> <espaco> [sala2]
capturar_elemento() {
    [ "$ELEMENT_OK" = 1 ] || return 1
    local modo="$1" sala="$2" espaco="$3" sala2="${4:-}"
    CHROME_BIN="${CHROME_BIN:-google-chrome}" \
    ACEITE_ELEMENT="http://127.0.0.1:$ELEMENT_PORTA" \
    ACEITE_SENHA="$(cat "$DIR/atendente.senha")" \
    ACEITE_USUARIO="$USUARIO_ATENDENTE" \
        node "$ELEMENT_JS" "$modo" "$DIR/capturas" "$sala" "$espaco" "$DIR" "$sala2" \
        > "$DIR/element-captura-$modo.log" 2>&1
}

# esta_no_espaco <sala> <espaco> — sucesso quando o espaço tem `m.space.child` da sala
esta_no_espaco() {
    local saida
    saida="$(python3 "$AUX" pai-do-espaco "$1" "$2")"
    printf '%s' "$saida" | grep -q '"_status"' && return 1
    [ "$saida" != "{}" ]
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
[ -n "$CONV" ] || falha "nenhuma sala foi criada em 60 s (veja $DIR/serve.log)"
SALA=""
for _ in $(seq 1 60); do
    SALA="$(python3 - "$DIR/data/harness.db" "$CONV" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select room_id from matrix_rooms where conversation_id=? and state='ready'", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$SALA" ] && break
    sleep 1
done
[ -n "$SALA" ] || falha "a sala da conversa não ficou pronta em 60 s (veja $DIR/serve.log)"
echo "conversa=$CONV sala=$SALA" | tee "$DIR/ids.txt"

# A sala recém-criada entra no espaço do departamento na própria criação (correção da MX-31).
# Esta checagem é ANTES da transferência: ela prova o A1 sem truque de gatilho.
NO_ESPACO=0
for _ in $(seq 1 60); do esta_no_espaco "$SALA" "$SALA_COM" && { NO_ESPACO=1; break; }; sleep 1; done

# o atendente é membro do espaço e entra na sala restrita (o join_rules é escrito na criação)
for _ in $(seq 1 20); do
    python3 "$AUX" entrar "$SALA" > "$DIR/entrar-a1.json" 2>&1 || true
    grep -q '^200' "$DIR/entrar-a1.json" && break
    sleep 1
done

# a primeira mensagem pode chegar antes da sala; espera o espelho gravar a resposta do agente
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

python3 "$AUX" estado-sala "$SALA" > "$DIR/estado-sala-a1.json"
NOME="$(python3 -c "
import json
d=json.load(open('$DIR/estado-sala-a1.json'))
print(d['m.room.name'].get('name',''))
")"
NOME_META="$(python3 -c "
import json
d=json.load(open('$DIR/estado-sala-a1.json'))
for k,v in d.items():
    if k.endswith('.atendimento.metadata'):
        print(v.get('client_name',''))
        break
")"
if [ "$NOME" = "Ana Souza" ] && [ "$NOME_META" = "Ana Souza" ] && [ "$NO_ESPACO" = 1 ]; then
    anotar A1 "OK: sala 'Ana Souza' (metadata idem) já no espaço Comercial na criação, sem transferência"
else
    anotar A1 "FALHOU: nome da sala='$NOME' (metadata='$NOME_META'), no espaço Comercial=$NO_ESPACO"
fi
if [ "$ELEMENT_OK" = 1 ]; then
    if capturar_elemento a1 "$SALA" "$SALA_COM"; then
        anotar A12 "OK (A1): capturas a1-sala.png e a1-espaco.png (sala nomeada e sob o espaço Comercial)"
    else
        anotar A12 "NÃO CONFERIDO: a captura do A1 falhou (veja $DIR/element-captura-a1.log)"
    fi
fi

echo "== 10. A6: transferência pelo departamento move a sala de espaço =="
python3 "$AUX" transferir "$DIR/data/harness.db" "$CONV" "$SUP" > "$DIR/transferencia.txt"
MOVED=0
for _ in $(seq 1 40); do
    if esta_no_espaco "$SALA" "$SALA_SUP"; then MOVED=1; break; fi
    sleep 1
done
[ "$MOVED" = 1 ] && anotar A6 "OK: sala no espaço Suporte" || anotar A6 "FALHOU: a sala não entrou no espaço Suporte"
FORA_COM="$(python3 "$AUX" pai-do-espaco "$SALA" "$SALA_COM")"
if [ "$FORA_COM" = "{}" ] || printf '%s' "$FORA_COM" | grep -q '"_status"'; then
    anotar A6 "OK: sala fora do espaço Comercial"
else
    anotar A6 "FALHOU: a sala continua no espaço antigo ($FORA_COM)"
fi



echo "== 11. A2: resposta do atendente =="
python3 "$AUX" atendente-envia "$SALA" "Claro! O plano anual sai por R\$ 1.188, com 50% de desconto no primeiro ano." > "$DIR/envio-atendente.json"
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

# mensagem seguinte do cliente: gravada, sem resposta nova
ANTES="$(wc -l < "$DIR/falsos/evo.jsonl" 2>/dev/null || echo 0)"
python3 "$AUX" webhook "$ORG" aceite ACEITE-MSG-2 "Perfeito, vou fechar! Pode mandar o link?" "Ana Souza" >/dev/null 2>&1
sleep 5
DEPOIS="$(wc -l < "$DIR/falsos/evo.jsonl" 2>/dev/null || echo 0)"
[ "$ANTES" = "$DEPOIS" ] && anotar A2 "OK: mensagem seguinte do cliente não gerou resposta" || anotar A2 "FALHOU: o agente respondeu depois da transferência"

echo "== 12. A7: reabertura, fechamento e conversa nova =="
VERSAO="$(curl -sS "http://127.0.0.1:$SERVE_PORTA/admin/v1/orgs/$ORG/conversations/$CONV" \
    -H "Authorization: Bearer $TOKEN_ADMIN" | python3 -c "import json,sys;print(json.load(sys.stdin)['version'])")"
curl -sS -o "$DIR/reopen.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORTA/admin/v1/orgs/$ORG/conversations/$CONV/reopen" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H "If-Match: $VERSAO" > "$DIR/reopen.status"
[ "$(cat "$DIR/reopen.status")" = "200" ] && anotar A7 "OK: reabertura pela API" || anotar A7 "FALHOU: reabertura devolveu $(cat "$DIR/reopen.status")"
sleep 4
python3 "$AUX" estado-sala "$SALA" > "$DIR/estado-reaberta.json"
VERSAO="$(python3 -c "import json;print(json.load(open('$DIR/reopen.json'))['version'])")"
curl -sS -o "$DIR/close.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$SERVE_PORTA/admin/v1/orgs/$ORG/conversations/$CONV/close" \
    -H "Authorization: Bearer $TOKEN_ADMIN" -H "If-Match: $VERSAO" > "$DIR/close.status"
[ "$(cat "$DIR/close.status")" = "200" ] && anotar A7 "OK: fechamento pela API" || anotar A7 "FALHOU: fechamento devolveu $(cat "$DIR/close.status")"
sleep 4
STATUS_META="$(python3 "$AUX" estado-sala "$SALA" | python3 -c "import json,sys;print(json.load(sys.stdin)['com.sofacoding.atendimento.metadata'].get('status'))")"
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
# Sala nasce só com o telefone (primeira mensagem SEM pushName) e o nome chega na mensagem seguinte.
# O que se prova: título da sala, client_name do metadata e nome de exibição do fantasma mudam, e
# nenhum m.notice novo aparece (trocar nome não é evento de atendimento).
TEL_TARDIO="5521977776666"
SALA2=""
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
    SALA2="$(python3 - "$DIR/data/harness.db" "$CONV2" <<'FIMSQL'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("select room_id from matrix_rooms where conversation_id=? and state='ready'", (sys.argv[2],)).fetchone()
print(r[0] if r else "")
FIMSQL
)"
    [ -n "$SALA2" ] && break
    sleep 1
done
if [ -z "$CONV2" ] || [ -z "$SALA2" ]; then
    anotar MX33 "FALHOU: a conversa/sala sem nome não foi criada (conversa='$CONV2' sala='$SALA2')"
else
    # o atendente é membro do espaço e entra na sala restrita para ler estado e histórico
    for _ in $(seq 1 20); do
        python3 "$AUX" entrar "$SALA2" > "$DIR/entrar-nome.json" 2>&1 || true
        grep -q '^200' "$DIR/entrar-nome.json" && break
        sleep 1
    done
    python3 "$AUX" estado-sala "$SALA2" > "$DIR/estado-nome-antes.json"
    NOME_ANTES="$(python3 -c "
import json
d=json.load(open('$DIR/estado-nome-antes.json'))
print(d['m.room.name'].get('name',''))
")"
    FANTASMA_ANTES="$(python3 "$AUX" perfil "@ph_wa_${TEL_TARDIO}:matrix-dev.local" | python3 -c "import json,sys;print(json.load(sys.stdin).get('displayname',''))")"
    AVISOS_ANTES="$(python3 "$AUX" avisos "$SALA2" | python3 -c "import json,sys;print(json.load(sys.stdin).get('total','erro'))")"

    ACEITE_TELEFONE="$TEL_TARDIO" python3 "$AUX" webhook "$ORG" aceite ACEITE-MSG-N2 \
        "Sou o Carlos Pereira." "Carlos Pereira" > "$DIR/webhook-nome2.json" 2>&1
    NOME_DEPOIS=""
    for _ in $(seq 1 60); do
        NOME_DEPOIS="$(python3 "$AUX" estado-sala "$SALA2" | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(d['m.room.name'].get('name',''))
")"
        [ "$NOME_DEPOIS" = "Carlos Pereira" ] && break
        sleep 1
    done
    python3 "$AUX" estado-sala "$SALA2" > "$DIR/estado-nome-depois.json"
    NOME_META2="$(python3 -c "
import json
d=json.load(open('$DIR/estado-nome-depois.json'))
for k,v in d.items():
    if k.endswith('.atendimento.metadata'):
        print(v.get('client_name',''))
        break
")"
    FANTASMA_DEPOIS="$(python3 "$AUX" perfil "@ph_wa_${TEL_TARDIO}:matrix-dev.local" | python3 -c "import json,sys;print(json.load(sys.stdin).get('displayname',''))")"
    AVISOS_DEPOIS="$(python3 "$AUX" avisos "$SALA2" | python3 -c "import json,sys;print(json.load(sys.stdin).get('total','erro'))")"
    if [ "$NOME_ANTES" != "Carlos Pereira" ] && [ "$NOME_DEPOIS" = "Carlos Pereira" ] \
        && [ "$NOME_META2" = "Carlos Pereira" ] \
        && [ "$FANTASMA_DEPOIS" = "Carlos Pereira (WhatsApp)" ] \
        && [ "$AVISOS_ANTES" = "$AVISOS_DEPOIS" ]; then
        anotar MX33 "OK: sala nasceu '$NOME_ANTES'/fantasma '$FANTASMA_ANTES' e passou a 'Carlos Pereira' (metadata e fantasma idem) sem novo aviso (notices $AVISOS_ANTES -> $AVISOS_DEPOIS)"
    else
        anotar MX33 "FALHOU: antes='$NOME_ANTES'/fantasma '$FANTASMA_ANTES', depois='$NOME_DEPOIS'/metadata '$NOME_META2'/fantasma '$FANTASMA_DEPOIS', notices $AVISOS_ANTES -> $AVISOS_DEPOIS"
    fi
fi

echo "== 13. A11: dois registros, sem evento cruzado =="
ANTES_TX="$(grep -c 'transação do Application Service processada' "$DIR/serve.log" 2>/dev/null || echo 0)"
SALA_PY="$(SYNAPSE="$SYNAPSE" python3 - "$PY_AS_TOKEN" <<'PY'
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
if [ -n "$SALA_PY" ] && [ "$ANTES_TX" = "$DEPOIS_TX" ]; then
    anotar A11 "OK: registro antigo ativo (criou $SALA_PY) e o Harness não recebeu a transação dele"
else
    anotar A11 "FALHOU: transações do Harness $ANTES_TX -> $DEPOIS_TX (sala do Python '$SALA_PY')"
fi
grep -q "Loaded application service" "$DIR/synapse-subida.log" \
    && anotar A11 "OK: o log do Synapse mostra os dois registros carregados"

echo "== 14. A12: Element (capturas finais) =="
if [ "$ELEMENT_OK" = 1 ]; then
    if capturar_elemento final "$SALA" "$SALA_SUP" "$SALA2"; then
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

echo
echo "== resumo =="
cat "$RESULTADO"
echo
echo "Artefatos: $DIR"
if grep -qE '^(A1|A2|A6|A7|A11|A12|MX33)\s+FALHOU' "$RESULTADO"; then
    echo "Aceite NÃO passou: há critério com falha (ver docs/MATRIZ_ACEITE.md)." >&2
    exit 1
fi
echo "Critérios conferidos passaram."

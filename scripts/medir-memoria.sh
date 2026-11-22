#!/usr/bin/env bash
# Mede a memória (RSS) do `poph serve` com o módulo Matrix desligado e ligado, e a vazão de
# mensagens espelhadas por segundo (MX-30, critério A13). Uma rodada = um processo novo, um banco
# novo com N conversas semeadas, e o `poph serve` de verdade (`target/release/poph`) sobre ele.
#
# Uso (a partir de apps/pop-harness):
#   bash scripts/medir-memoria.sh --modo desligado --salas 1000 --rodada 1
#   bash scripts/medir-memoria.sh --modo ligado   --salas 1000 --mensagens 1 --rodada 1
#   bash scripts/medir-memoria.sh --modo ligado   --salas 1000 --mensagens 10 --rodada 1   # vazão
#   bash scripts/medir-memoria.sh --modo real     --salas 20   --rodada 1   # Synapse de dev (MX-01)
#
# Saída: uma linha por rodada em target/mx30/memoria.csv (fora do git) e o resumo na tela.
# Não imprime token: os tokens são gerados ou lidos do ambiente e passados só por variável.
#
# Padrões de produção mantidos: concorrência 8, intervalo 500 ms, janela de agrupamento padrão,
# nível de log `info`. Synapse falso em processo separado (scripts/medicao-memoria/synapse_falso.py).
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

MODO=""
SALAS=""
MENSAGENS=1
RODADA=1
PORTA_SERVE="${MX30_PORTA_SERVE:-18432}"
PORTA_FALSO="${MX30_PORTA_FALSO:-18431}"
LIMITE_S="${MX30_LIMITE_S:-3600}"
RAIZ="$(pwd)/target/mx30"
BIN="$(pwd)/target/release/poph"
CSV="$RAIZ/memoria.csv"

falha() { echo "erro: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --modo) MODO="$2"; shift 2 ;;
        --salas) SALAS="$2"; shift 2 ;;
        --mensagens) MENSAGENS="$2"; shift 2 ;;
        --rodada) RODADA="$2"; shift 2 ;;
        *) falha "argumento desconhecido: $1" ;;
    esac
done

case "$MODO" in
    desligado|ligado|real) ;;
    *) falha "--modo precisa ser desligado, ligado ou real" ;;
esac
[ -n "$SALAS" ] || falha "--salas é obrigatório"
[ -x "$BIN" ] || falha "$BIN não existe: rode antes 'cargo build --release --locked -p poph'"

mkdir -p "$RAIZ/logs"
PASTA="$RAIZ/banco-$MODO-$SALAS-$MENSAGENS-$RODADA"
rm -rf "$PASTA"
mkdir -p "$PASTA"

porta_ocupada() { ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$1$"; }
porta_ocupada "$PORTA_SERVE" && falha "porta $PORTA_SERVE ocupada (use MX30_PORTA_SERVE)"
porta_ocupada "$PORTA_FALSO" && falha "porta $PORTA_FALSO ocupada (use MX30_PORTA_FALSO)"

# Tokens: gerados por rodada (ou lidos do arquivo de credenciais do dev, no modo real).
if [ "$MODO" = real ]; then
    CRED="$HOME/.cache/poph-matrix/credenciais.env"
    [ -f "$CRED" ] || falha "credenciais do Synapse de dev ausentes (rode matrix-dev.sh subir)"
    # Carrega o arquivo sem imprimir nada; o valor nunca sai daqui.
    set -a; . "$CRED"; set +a
    # Registro do Harness (não o do Python): é o que o Synapse de dev reconhece para o AS do Harness.
    AS_TOKEN="${POPH_MATRIX_DEV_HARNESS_AS_TOKEN:-}"
    HS_TOKEN="${POPH_MATRIX_DEV_HARNESS_HS_TOKEN:-}"
    [ -n "$AS_TOKEN" ] && [ -n "$HS_TOKEN" ] || falha "tokens do registro do Harness ausentes no arquivo de credenciais"
    SERVIDOR_MATRIX="${POPH_MATRIX_DEV_SERVER_NAME:-matrix-dev.local}"
    URL_HOMESERVER="${POPH_MATRIX_DEV_BASE_URL:-http://127.0.0.1:18008}"
else
    AS_TOKEN="$(python3 -I -c 'import secrets; print(secrets.token_hex(32))')"
    HS_TOKEN="$(python3 -I -c 'import secrets; print(secrets.token_hex(32))')"
    SERVIDOR_MATRIX="matrix-dev.local"
    URL_HOMESERVER="http://127.0.0.1:$PORTA_FALSO"
fi
export POPH_MATRIX_AS_TOKEN="$AS_TOKEN"
export POPH_MATRIX_HS_TOKEN="$HS_TOKEN"

LIGADO=0
[ "$MODO" = ligado ] && LIGADO=1
[ "$MODO" = real ] && LIGADO=1

# 1) Semeadura: N conversas (e M mensagens do cliente em cada), com a org ligada ou não.
INICIO_SEMEADURA=$(date +%s%N)
MX30_PASTA="$PASTA" MX30_SALAS="$SALAS" MX30_MENSAGENS_POR_SALA="$MENSAGENS" MX30_LIGADO="$LIGADO" \
    cargo test --locked -p poph-server --test matrix_medicao_memoria -- --ignored --nocapture \
    semeia_conversas_para_a_medicao_de_memoria >"$RAIZ/logs/semeadura-$MODO-$SALAS-$MENSAGENS-$RODADA.log" 2>&1 \
    || falha "semeadura falhou; veja $RAIZ/logs/semeadura-$MODO-$SALAS-$MENSAGENS-$RODADA.log"
FIM_SEMEADURA=$(date +%s%N)
SEMEADURA_MS=$(( (FIM_SEMEADURA - INICIO_SEMEADURA) / 1000000 ))

# 2) Configuração do poph serve (só o necessário; o resto é padrão de produção).
CFG="$PASTA/poph.toml"
{
    echo "[servidor]"
    echo "endereco = \"127.0.0.1\""
    echo "porta = $PORTA_SERVE"
    echo ""
    echo "[dados]"
    echo "diretorio = \"$PASTA/dados\""
    echo ""
    echo "[log]"
    echo "nivel = \"info\""
    echo "formato = \"legivel\""
    if [ "$MODO" != desligado ]; then
        echo ""
        echo "[matrix]"
        echo "habilitado = true"
        echo "homeserver_url = \"$URL_HOMESERVER\""
        echo "server_name = \"$SERVIDOR_MATRIX\""
        echo "prefixo = \"ph\""
    fi
} >"$CFG"

# 3) Synapse falso (só nos modos ligado e desligado; o desligado nunca fala com ele).
FALSO_PID=""
if [ "$MODO" != real ]; then
    MX30_AS_TOKEN="$AS_TOKEN" python3 -I "$(pwd)/scripts/medicao-memoria/synapse_falso.py" \
        --porta "$PORTA_FALSO" >"$RAIZ/logs/falso-$MODO-$SALAS-$MENSAGENS-$RODADA.log" 2>&1 &
    FALSO_PID=$!
    for _ in $(seq 1 50); do
        curl -fsS "http://127.0.0.1:$PORTA_FALSO/_fake/estatisticas" >/dev/null 2>&1 && break
        sleep 0.1
    done
fi

estatisticas() {
    if [ -n "$FALSO_PID" ]; then
        curl -fsS "http://127.0.0.1:$PORTA_FALSO/_fake/estatisticas" 2>/dev/null || echo '{}'
    else
        echo '{}'
    fi
}

# Mensagens que o módulo entregou ao Synapse, lidas do /metrics do próprio serve (MX-26). Usado
# no modo real, onde não há falso para contar; nos outros modos o falso conta de forma independente.
mensagens_espelhadas() {
    curl -fsS "http://127.0.0.1:$PORTA_SERVE/metrics" 2>/dev/null \
        | awk '/^poph_matrix_mensagens_espelhadas_total/ {print $2; exit}' || true
}

rss_kb() { awk '/^VmRSS:/ {print $2}' "/proc/$1/status" 2>/dev/null || echo 0; }
# Tempo de CPU (usuário + sistema) de um processo, em ticks: a diferença entre duas leituras
# dividida pelo tempo de parede dá a fração de um núcleo que o processo usou.
ticks() { awk '{print $14+$15}' "/proc/$1/stat" 2>/dev/null || echo 0; }
CLK_TCK=$(getconf CLK_TCK)
pico_kb() { awk '/^VmHWM:/ {print $2}' "/proc/$1/status" 2>/dev/null || echo 0; }

# 4) Sobe o poph serve de verdade.
LOG_SERVE="$RAIZ/logs/serve-$MODO-$SALAS-$MENSAGENS-$RODADA.log"
"$BIN" --config "$CFG" serve >"$LOG_SERVE" 2>&1 &
SERVE_PID=$!
PARAR() {
    kill -TERM "$SERVE_PID" 2>/dev/null || true
    wait "$SERVE_PID" 2>/dev/null || true
    [ -n "$FALSO_PID" ] && kill "$FALSO_PID" 2>/dev/null || true
}
trap PARAR EXIT

for _ in $(seq 1 300); do
    curl -fsS "http://127.0.0.1:$PORTA_SERVE/healthz" >/dev/null 2>&1 && break
    kill -0 "$SERVE_PID" 2>/dev/null || falha "o poph serve saiu na subida; veja $LOG_SERVE"
    sleep 0.1
done
curl -fsS "http://127.0.0.1:$PORTA_SERVE/healthz" >/dev/null 2>&1 || falha "o poph serve não respondeu em /healthz"

INICIO_T0=$(date +%s%N)
RSS_INICIO=$(rss_kb "$SERVE_PID")
TICKS_SERVE0=$(ticks "$SERVE_PID")
TICKS_FALSO0=0
[ -n "$FALSO_PID" ] && TICKS_FALSO0=$(ticks "$FALSO_PID")

# 5) Vazão: espera a fila esvaziar (todas as mensagens chegarem ao falso) ou o limite.
ESPERADO=$(( SALAS * MENSAGENS ))
DRENAGEM_S=0
if [ "$MODO" != desligado ]; then
    while :; do
        if [ -n "$FALSO_PID" ]; then
            ENVIOS=$(estatisticas | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("envios_unicos",0))' 2>/dev/null || echo 0)
        else
            ENVIOS=$(mensagens_espelhadas || true)
            ENVIOS=${ENVIOS:-0}
        fi
        if [ "$ENVIOS" -ge "$ESPERADO" ]; then break; fi
        AGORA=$(date +%s%N)
        DECORRIDO=$(( (AGORA - INICIO_T0) / 1000000000 ))
        if [ "$DECORRIDO" -ge "$LIMITE_S" ]; then
            falha "limite de ${LIMITE_S}s atingido com $ENVIOS de $ESPERADO mensagens no falso"
        fi
        kill -0 "$SERVE_PID" 2>/dev/null || falha "o poph serve saiu durante a vazão; veja $LOG_SERVE"
        sleep 0.2
    done
    FIM_T1=$(date +%s%N)
    DRENAGEM_NS=$(( FIM_T1 - INICIO_T0 ))
    DRENAGEM_S=$(python3 -I -c "print(round($DRENAGEM_NS/1e9, 3))")
    TICKS_SERVE1=$(ticks "$SERVE_PID")
    TICKS_FALSO1=0
    [ -n "$FALSO_PID" ] && TICKS_FALSO1=$(ticks "$FALSO_PID")
    CPU_SERVE=$(python3 -I -c "print(round(($TICKS_SERVE1-$TICKS_SERVE0)/$CLK_TCK/$DRENAGEM_S*100, 1))")
    CPU_FALSO=$(python3 -I -c "print(round(($TICKS_FALSO1-$TICKS_FALSO0)/$CLK_TCK/$DRENAGEM_S*100, 1))")
fi

# 6) Regime: RSS amostrado por 10 s depois de a fila esvaziar (ou no mesmo tempo, no desligado).
AMOSTRAS=""
for _ in $(seq 1 10); do
    AMOSTRAS="$AMOSTRAS $(rss_kb "$SERVE_PID")"
    sleep 1
done
RSS_REGIME=$(echo $AMOSTRAS | tr ' ' '\n' | python3 -I -c 'import sys,statistics; v=[int(x) for x in sys.stdin.read().split()]; print(int(statistics.median(v)))')
PICO=$(pico_kb "$SERVE_PID")

if [ -n "$FALSO_PID" ]; then
    ESTAT=$(estatisticas)
    ENVIOS_UNICOS=$(echo "$ESTAT" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("envios_unicos",0))' 2>/dev/null || echo 0)
    SALAS_CRIADAS=$(echo "$ESTAT" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("salas",0))' 2>/dev/null || echo 0)
    CONVITES=$(echo "$ESTAT" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("convites",0))' 2>/dev/null || echo 0)
    REQS=$(echo "$ESTAT" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("requisicoes",0))' 2>/dev/null || echo 0)
else
    # Modo real: o Synapse é o de dev; não há contagem de salas, convites nem requisições daqui.
    ENVIOS_UNICOS=$(mensagens_espelhadas || true)
    ENVIOS_UNICOS=${ENVIOS_UNICOS:-0}
    SALAS_CRIADAS=nd
    CONVITES=nd
    REQS=nd
fi

VAZAO="0"
CPU_SERVE=${CPU_SERVE:-0}
CPU_FALSO=${CPU_FALSO:-0}
if [ "$MODO" != desligado ] && [ "$ENVIOS_UNICOS" -gt 0 ]; then
    VAZAO=$(python3 -I -c "print(round($ENVIOS_UNICOS/$DRENAGEM_S, 1))")
fi

if [ ! -f "$CSV" ]; then
    echo "modo,salas,mensagens_por_sala,rodada,rss_inicio_kb,rss_regime_kb,rss_pico_kb,tempo_drenagem_s,msgs_por_s,cpu_serve_pct,cpu_falso_pct,salas_criadas,envios_unicos,convites,requisicoes,semeadura_ms" >"$CSV"
fi
echo "$MODO,$SALAS,$MENSAGENS,$RODADA,$RSS_INICIO,$RSS_REGIME,$PICO,$DRENAGEM_S,$VAZAO,$CPU_SERVE,$CPU_FALSO,$SALAS_CRIADAS,$ENVIOS_UNICOS,$CONVITES,$REQS,$SEMEADURA_MS" >>"$CSV"

echo "modo=$MODO salas=$SALAS mensagens_por_sala=$MENSAGENS rodada=$RODADA"
echo "  RSS na subida: $RSS_INICIO kB | regime: $RSS_REGIME kB | pico: $PICO kB"
echo "  drenagem: ${DRENAGEM_S}s | vazão: $VAZAO msg/s | CPU serve: ${CPU_SERVE}% | CPU falso: ${CPU_FALSO}% | salas criadas: $SALAS_CRIADAS | envios únicos: $ENVIOS_UNICOS"
echo "  semeadura: ${SEMEADURA_MS} ms | CSV: $CSV"

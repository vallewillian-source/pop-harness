#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════
# matrix-dev.sh — Synapse de DESENVOLVIMENTO isolado, com DOIS Application
# Services carregados ao mesmo tempo (MX-01 do módulo Matrix do POP Harness):
#   - poph-matrix-dev        → o registro NOVO do Harness (prefixos ph_)
#   - popatende-python-teste → o registro do POP Atende (Python) com tokens fictícios
#
# Nada aqui toca o dev do POP Atende, a máquina ARM64 de testes ou produção: projeto próprio, rede
# própria, porta alta em 127.0.0.1 e dados fora do git (~/.cache/poph-matrix).
#
# Uso:
#   bash apps/pop-harness/scripts/matrix-dev.sh subir   # sobe (ou reinicia) e espera responder
#   bash apps/pop-harness/scripts/matrix-dev.sh parar   # para o container, preserva os dados
#   bash apps/pop-harness/scripts/matrix-dev.sh limpar  # apaga SÓ o que este script criou
#   bash apps/pop-harness/scripts/matrix-dev.sh url     # imprime a URL e o server_name
#
# Variáveis de ambiente (opcionais):
#   POPH_MATRIX_DEV_DIR      diretório de dados (padrão ~/.cache/poph-matrix)
#   POPH_MATRIX_DEV_PORTA    porta do Synapse em 127.0.0.1 (padrão 18008)
#   POPH_MATRIX_DEV_PORTA_PY porta do AS do Python no host (padrão 18009)
#   POPH_MATRIX_DEV_PORTA_H  porta do AS do Harness no host (padrão 18010)
#   POPH_MATRIX_DEV_URL_PY   url do registro do Python (padrão http://<IP-LAN>:18009)
#   POPH_MATRIX_DEV_URL_H    url do registro do Harness (padrão http://<IP-LAN>:18010/matrix)
#   POPH_MATRIX_DEV_IMAGEM   imagem do Synapse (padrão matrixdotorg/synapse:v1.120.0)
#   POPH_MATRIX_DEV_REGISTRO_H  registro do Harness PRONTO (ex.: gerado por `poph matrix
#                            registro`): em vez do modelo, o Synapse carrega este arquivo tal
#                            como está (MX-21). O arquivo NÃO é copiado para o git.
#   POPH_MATRIX_DEV_REUSAR_TOKENS=1  reaproveita os tokens de credenciais.env em vez de gerar
#                            novos — use junto do registro pronto, para os tokens do arquivo
#                            continuarem batendo com o ambiente.
# ═══════════════════════════════════════════════════════════════════════════════
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMPLATES="$APP/tests/matrix"

DIR="${POPH_MATRIX_DEV_DIR:-$HOME/.cache/poph-matrix}"
DADOS="$DIR/data"
LOGS="$DIR/logs"
CREDENCIAIS="$DIR/credenciais.env"

PORTA="${POPH_MATRIX_DEV_PORTA:-18008}"
PORTA_PY="${POPH_MATRIX_DEV_PORTA_PY:-18009}"
PORTA_H="${POPH_MATRIX_DEV_PORTA_H:-18010}"
IMAGEM="${POPH_MATRIX_DEV_IMAGEM:-matrixdotorg/synapse:v1.120.0}"

SERVER_NAME="matrix-dev.local"
CONTAINER="poph-matrix-synapse"
REDE="poph-matrix-net"
MARCA="poph.matrix.dev=1"

# IP pelo qual o container alcança o host neste docker rootless
# (slirp4netns --disable-host-loopback: 127.0.0.1 e o gateway da bridge não servem).
IP_LAN="$(hostname -I | awk '{print $1}')"
URL_PY="${POPH_MATRIX_DEV_URL_PY:-http://$IP_LAN:$PORTA_PY}"
URL_H="${POPH_MATRIX_DEV_URL_H:-http://$IP_LAN:$PORTA_H/matrix}"

BASE="http://127.0.0.1:$PORTA"

falha() { echo "erro: $*" >&2; exit 1; }

precisa() {
    command -v "$1" >/dev/null 2>&1 || falha "$1 não encontrado no PATH"
}

# Máscara para qualquer linha que porventura carregue um token (o script não
# imprime token nenhum de propósito; isto é a segunda barreira).
mascarar() { sed -E 's/(_TOKEN[:=])[^ ]*/\1[oculto]/g'; }

porta_livre() {
    local p="$1"
    if ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$p$"; then
        return 1
    fi
    return 0
}

gerar_token() {
    # 64 hex, sem depender de openssl.
    python3 -c 'import secrets; print(secrets.token_hex(32))'
}

# Escreve um registro renderizado DENTRO do container. O Synapse deste ambiente
# roda como uid 991 (o /start.py usa gosu), que não é o uid do host: um arquivo
# 0600 escrito pelo host ficaria ilegível para ele. Escrevendo pelo container,
# com 0644 e o diretório de dados em 0700 no host, só o próprio Synapse lê.
renderizar() {
    local template="$1" destino="$2" as_token="$3" hs_token="$4" url="$5"
    [ -f "$template" ] || falha "template não encontrado: $template"
    sed \
        -e "s|__AS_TOKEN__|$as_token|g" \
        -e "s|__HS_TOKEN__|$hs_token|g" \
        -e "s|__AS_URL__|$url|g" \
        "$template" | docker run --rm -i -v "$DADOS:/data" --entrypoint sh "$IMAGEM" \
        -c 'cat > "$1" && chmod 644 "$1"' _ "$destino"
}

# Edita o homeserver.yaml DENTRO do container: os arquivos gerados pertencem ao
# uid do root do container rootless, que não é o uid do host.
configurar_homeserver() {
    docker run --rm -i -v "$DADOS:/data" --entrypoint python3 "$IMAGEM" - "$PORTA" <<'PYCFG'
import re, sys
caminho, porta = "/data/homeserver.yaml", sys.argv[1]
with open(caminho, encoding="utf-8") as f:
    texto = f.read()
if "app_service_config_files:" not in texto:
    texto = re.sub(
        r"(?m)^server_name:.*$",
        lambda m: m.group(0) + "\n"
        "\n# MX-01: os DOIS registros de Application Service convivem no mesmo Synapse.\n"
        "app_service_config_files:\n"
        "  - /data/registro-python.yaml\n"
        "  - /data/registro-harness.yaml\n"
        "\ntrusted_key_servers: []\n"
        "suppress_key_server_warning: true",
        texto, count=1,
    )
texto = re.sub(r"(?m)^public_baseurl:.*$", f'public_baseurl: "http://127.0.0.1:{porta}/"', texto)
with open(caminho, "w", encoding="utf-8") as f:
    f.write(texto)
PYCFG
}

subir() {
    precisa docker
    precisa curl
    precisa python3

    if ! porta_livre "$PORTA"; then
        # A porta pode estar ocupada pelo próprio container deste script (subir de
        # novo reinicia o ambiente). Só é erro se quem ocupa for outra coisa.
        if ! docker port "$CONTAINER" 2>/dev/null | grep -q ":$PORTA$"; then
            falha "a porta $PORTA já está ocupada por outro processo; escolha outra com POPH_MATRIX_DEV_PORTA"
        fi
    fi

    mkdir -p "$DADOS" "$LOGS"
    chmod 700 "$DIR"

    # Tokens fictícios NOVOS a cada execução; ficam só no diretório de dados (0600).
    # Com POPH_MATRIX_DEV_REUSAR_TOKENS=1 eles vêm do credenciais.env anterior — necessário quando
    # o registro do Harness é um arquivo pronto (POPH_MATRIX_DEV_REGISTRO_H), gerado com esses tokens.
    local as_py hs_py as_h hs_h
    if [ "${POPH_MATRIX_DEV_REUSAR_TOKENS:-0}" = "1" ] && [ -f "$CREDENCIAIS" ]; then
        as_py="$(valor_credencial POPH_MATRIX_DEV_PYTHON_AS_TOKEN)"
        hs_py="$(valor_credencial POPH_MATRIX_DEV_PYTHON_HS_TOKEN)"
        as_h="$(valor_credencial POPH_MATRIX_DEV_HARNESS_AS_TOKEN)"
        hs_h="$(valor_credencial POPH_MATRIX_DEV_HARNESS_HS_TOKEN)"
        [ -n "$as_h" ] && [ -n "$hs_h" ] || falha "credenciais.env sem os tokens do Harness; rode sem POPH_MATRIX_DEV_REUSAR_TOKENS"
        echo "--- reaproveitando os tokens de $CREDENCIAIS ---"
    else
        as_py="$(gerar_token)"; hs_py="$(gerar_token)"
        as_h="$(gerar_token)";  hs_h="$(gerar_token)"
    fi
    # O token de administrador (MX-46) não é gerado aqui: quando existe no arquivo anterior, é
    # preservado, porque ele foi criado à mão e regravar as credenciais não pode revogá-lo.
    local admin_anterior=""
    if [ -f "$CREDENCIAIS" ]; then
        admin_anterior="$(valor_credencial POPH_MATRIX_DEV_ADMIN_TOKEN)"
    fi
    umask 077
    {
        echo "# Credenciais FICTÍCIAS do ambiente MX-01 (fora do git)."
        echo "POPH_MATRIX_DEV_SYNAPSE_DIR=$DADOS"
        echo "POPH_MATRIX_DEV_BASE_URL=$BASE"
        echo "POPH_MATRIX_DEV_SERVER_NAME=$SERVER_NAME"
        echo "POPH_MATRIX_DEV_PYTHON_AS_TOKEN=$as_py"
        echo "POPH_MATRIX_DEV_PYTHON_HS_TOKEN=$hs_py"
        echo "POPH_MATRIX_DEV_HARNESS_AS_TOKEN=$as_h"
        echo "POPH_MATRIX_DEV_HARNESS_HS_TOKEN=$hs_h"
        [ -n "$admin_anterior" ] && echo "POPH_MATRIX_DEV_ADMIN_TOKEN=$admin_anterior"
    } > "$CREDENCIAIS"
    chmod 600 "$CREDENCIAIS"

    renderizar "$TEMPLATES/registro-python-teste.yaml.template" /data/registro-python.yaml "$as_py" "$hs_py" "$URL_PY"
    if [ -n "${POPH_MATRIX_DEV_REGISTRO_H:-}" ]; then
        [ -f "$POPH_MATRIX_DEV_REGISTRO_H" ] \
            || falha "POPH_MATRIX_DEV_REGISTRO_H: arquivo não encontrado: $POPH_MATRIX_DEV_REGISTRO_H"
        copiar_registro "$POPH_MATRIX_DEV_REGISTRO_H" /data/registro-harness.yaml
        echo "--- registro do Harness: usando o arquivo pronto $POPH_MATRIX_DEV_REGISTRO_H ---"
    else
        renderizar "$TEMPLATES/registro-teste.yaml.template" /data/registro-harness.yaml "$as_h" "$hs_h" "$URL_H"
    fi

    # homeserver.yaml e chave de assinatura: gerados uma vez, preservados entre execuções.
    if [ ! -f "$DADOS/homeserver.yaml" ]; then
        echo "--- gerando homeserver.yaml (server_name=$SERVER_NAME) ---"
        # Sem --user: em docker rootless o root do container É o usuário do host,
        # então o bind mount em $DADOS fica gravável; com --user, o uid do
        # container mapeia para um subuid e a escrita em /data falha.
        docker run --rm \
            -v "$DADOS:/data" \
            -e "SYNAPSE_SERVER_NAME=$SERVER_NAME" \
            -e "SYNAPSE_REPORT_STATS=no" \
            --entrypoint /start.py \
            "$IMAGEM" generate >/dev/null
        configurar_homeserver
    fi
    # O homeserver.yaml é do container (dono = uid do root do container): a conferência
    # das chaves do MX-01 e da porta é feita dentro dele, não com sed no host.
    configurar_homeserver

    docker network inspect "$REDE" >/dev/null 2>&1 || docker network create --label "$MARCA" "$REDE" >/dev/null

    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
        docker rm -f "$CONTAINER" >/dev/null
    fi

    echo "--- subindo o Synapse isolado ---"
    docker run -d \
        --name "$CONTAINER" \
        --label "$MARCA" \
        --network "$REDE" \
        -p "127.0.0.1:$PORTA:8008" \
        -v "$DADOS:/data" \
        -e "SYNAPSE_SERVER_NAME=$SERVER_NAME" \
        -e "SYNAPSE_REPORT_STATS=no" \
        "$IMAGEM" >/dev/null

    echo "--- aguardando $BASE/_matrix/client/versions ---"
    local i
    for i in $(seq 1 90); do
        if ! docker container inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null | grep -q true; then
            docker logs "$CONTAINER" 2>&1 | tail -30 | mascarar >&2
            falha "o container $CONTAINER parou durante a subida"
        fi
        if curl -fsS -m 2 -o /dev/null "$BASE/_matrix/client/versions" 2>/dev/null; then
            break
        fi
        sleep 2
    done
    curl -fsS -m 2 -o /dev/null "$BASE/_matrix/client/versions" \
        || { docker logs "$CONTAINER" 2>&1 | tail -40 | mascarar >&2; falha "Synapse não respondeu em $BASE"; }

    echo
    echo "Synapse no ar:"
    echo "  URL:         $BASE"
    echo "  server_name: $SERVER_NAME"
    echo "  container:   $CONTAINER   rede: $REDE"
    echo "  dados:       $DADOS"
    echo "  credenciais: $CREDENCIAIS  (0600, fictícias)"
    echo
    echo "Application Services carregados (procure esta linha no log):"
    docker logs "$CONTAINER" 2>&1 | grep -iE "application service|Loaded application" | tail -5 | mascarar || true
    echo
    echo "URLs dos registros (vistas do container):"
    echo "  python:  $URL_PY"
    echo "  harness: $URL_H"
}

parar() {
    precisa docker
    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
        docker stop "$CONTAINER" >/dev/null
        echo "container $CONTAINER parado (dados preservados em $DADOS)"
    else
        echo "container $CONTAINER não existe"
    fi
}

limpar() {
    precisa docker
    echo "--- antes ---"
    docker ps -a --filter "name=$CONTAINER" --format 'container: {{.Names}} ({{.Status}})'
    docker volume ls --filter "label=$MARCA" --format 'volume: {{.Name}}'

    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
        docker rm -f "$CONTAINER" >/dev/null
        echo "container $CONTAINER removido"
    fi
    if docker network inspect "$REDE" >/dev/null 2>&1; then
        docker network rm "$REDE" >/dev/null
        echo "rede $REDE removida"
    fi
    # Este script não usa volume nomeado (o /data é bind mount em $DADOS);
    # só remove o diretório se ele estiver dentro do diretório de dados do MX-01.
    # O /data pertence ao uid do Synapse dentro do container; o host não apaga
    # o conteúdo. Esvazia pelo container e só então remove o diretório pelo host.
    if [ -d "$DADOS" ]; then
        docker run --rm -v "$DADOS:/data" --entrypoint sh "$IMAGEM" \
            -c 'rm -rf /data/* /data/.[!.]* 2>/dev/null; true' || true
    fi
    case "$DIR" in
        "$HOME"/.cache/poph-matrix|"$HOME"/.cache/poph-matrix/*)
            rm -rf "$DIR"
            echo "dados e credenciais removidos: $DIR"
            ;;
        *)
            echo "dados preservados em $DIR (diretório fora do padrão; apague à mão se quiser)"
            ;;
    esac

    echo "--- depois ---"
    docker ps -a --filter "name=$CONTAINER" --format 'container: {{.Names}} ({{.Status}})'
    docker volume ls --filter "label=$MARCA" --format 'volume: {{.Name}}'
    echo "nada de outro projeto foi tocado"
}

# Copia um registro JÁ PRONTO para dentro do container (mesma razão do renderizar: dono/uid).
copiar_registro() {
    local origem="$1" destino="$2"
    docker run --rm -i -v "$DADOS:/data" --entrypoint sh "$IMAGEM" \
        -c 'cat > "$1" && chmod 644 "$1"' _ "$destino" < "$origem"
}

# Lê um valor de credenciais.env sem imprimir o resto (usado por POPH_MATRIX_DEV_REUSAR_TOKENS).
valor_credencial() { sed -n "s/^$1=//p" "$CREDENCIAIS" | head -1; }

url() {
    echo "URL: $BASE"
    echo "server_name: $SERVER_NAME"
    echo "credenciais: $CREDENCIAIS"
}

case "${1:-}" in
    subir)  subir ;;
    parar)  parar ;;
    limpar) limpar ;;
    url)    url ;;
    *)
        echo "uso: $0 {subir|parar|limpar|url}" >&2
        exit 2
        ;;
esac

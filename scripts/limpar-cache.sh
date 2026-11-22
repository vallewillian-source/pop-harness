#!/usr/bin/env bash
# HR-60 — Poda do cache de compilação do Cargo.
#
# Por que existe: desde o Cargo 1.100 o layout do build-dir mudou (cargo#15010) e TODO artefato
# intermediário passa a ficar em `target/<perfil>/build/<crate>/<hash-da-unidade>/out/`. O hash
# muda quando a configuração efetiva da unidade muda (perfil dev/test/check, features unificadas
# pela forma do comando, alvo, toolchain e hashes das dependências). O Cargo nunca apaga os
# diretórios de hashes antigos, e cada binário de teste levava debuginfo completo (~300 MB antes
# da HR-60b; hoje o perfil dev usa `line-tables-only` e os binários ficam ~56% menores — a poda
# continua necessária, porque as unidades antigas nunca são apagadas pelo Cargo).
# Resultado: o `target/` cresce a cada forma de comando diferente até encher o disco.
#
# O que faz: apaga as unidades e os diretórios de incremental que não são tocados há N dias
# (padrão 3), preservando o que um build recente pode estar usando, e reduz a retenção sozinho
# quando o `target/` passa do limite em GB. Nunca toca em código, em `Cargo.lock` nem em arquivo
# fora de `target/`.
#
# Uso:
#   scripts/limpar-cache.sh                 # poda o que passou de 3 dias; reduz se passar de 25 GB
#   scripts/limpar-cache.sh --seco          # só mostra o que apagaria
#   scripts/limpar-cache.sh --dias 7        # retenção maior
#   scripts/limpar-cache.sh --limite-gb 40  # teto de tamanho do target
#   scripts/limpar-cache.sh --tudo          # `cargo clean` (apaga o target inteiro; respeita build ativo)
#   scripts/limpar-cache.sh --forcar        # poda mesmo com build em andamento (só para teste)
#
# Saída: resumo com bytes liberados. Código 0 também quando pula por build em andamento —
# o CI não deve falhar por causa da poda.

set -uo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RAIZ" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

DIAS=3
LIMITE_GB=25
SECO=0
TUDO=0
FORCAR=0
MINUTOS_ATIVOS=120

uso() {
    sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dias) DIAS="${2:-}"; shift 2 ;;
        --dias=*) DIAS="${1#*=}"; shift ;;
        --limite-gb) LIMITE_GB="${2:-}"; shift 2 ;;
        --limite-gb=*) LIMITE_GB="${1#*=}"; shift ;;
        --minutos-ativos) MINUTOS_ATIVOS="${2:-}"; shift 2 ;;
        --seco|--dry-run) SECO=1; shift ;;
        --tudo) TUDO=1; shift ;;
        --forcar) FORCAR=1; shift ;;
        -h|--ajuda|--help) uso; exit 0 ;;
        *) echo "argumento desconhecido: $1" >&2; exit 2 ;;
    esac
done

case "$DIAS" in ''|*[!0-9.]*) echo "--dias precisa de número (ex.: 3 ou 0.5)" >&2; exit 2 ;; esac
case "$LIMITE_GB" in ''|*[!0-9]*) echo "--limite-gb precisa de inteiro" >&2; exit 2 ;; esac

num() { awk -v b="${1:-0}" 'BEGIN{ printf "%.2f GB", b/1073741824 }'; }

ALVO="$RAIZ/target"
if [ ! -d "$ALVO" ]; then
    echo "cache: sem target/ em $RAIZ — nada a podar"
    exit 0
fi

if [ "$TUDO" -eq 1 ]; then
    # Mesma guarda da poda por idade: `--tudo` também não pode apagar o target de um build em curso.
    if [ "$FORCAR" -eq 0 ] && { pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; }; then
        echo "cache: há um cargo/rustc em execução nesta máquina — --tudo adiado (use --forcar para apagar de qualquer forma)"
        exit 0
    fi
    antes=$(du -sb "$ALVO" 2>/dev/null | cut -f1)
    if [ "$SECO" -eq 1 ]; then
        echo "cache: --seco com --tudo: apagaria o target/ inteiro ($(num "$antes") )"
        exit 0
    fi
    echo "cache: apagando o target/ inteiro (cargo clean)"
    cargo clean 2>/dev/null || rm -rf "$ALVO"
    echo "cache: liberado $(num "$antes") — o próximo build compila do zero"
    exit 0
fi

# Um build de outro processo pode estar usando diretórios antigos; podar agora poderia quebrá-lo.
if [ "$FORCAR" -eq 0 ] && { pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; }; then
    echo "cache: há um cargo/rustc em execução nesta máquina — poda adiada (não falha o CI)"
    exit 0
fi

poda() {
    local dias="$1" rotulo="$2"
    local minutos
    minutos=$(awk -v d="$dias" 'BEGIN{ printf "%d", d*1440 }')
    [ "$minutos" -lt 1 ] && minutos=1

    local antes_dir=${#REMOVER[@]}
    # janela de proteção: nunca maior que metade da retenção (senão a retenção curta não poda nada)
    local ativo=$MINUTOS_ATIVOS
    [ "$ativo" -ge "$minutos" ] && ativo=$(( minutos / 2 ))
    [ "$ativo" -lt 5 ] && ativo=0
    local alvo

    # 1) Unidades do build-dir novo: target/<perfil>/build/<crate>/<hash16>/
    while IFS= read -r -d '' alvo; do
        local base; base=$(basename "$alvo")
        [[ "$base" =~ ^[0-9a-f]{16}$ ]] || continue
        # ainda em uso recente? então fica
        if [ -n "$(find "$alvo" -newermt "-${ativo} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        # algum arquivo tocado dentro da retenção? então fica
        if [ -n "$(find "$alvo" -newermt "-${minutos} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        REMOVER+=("$alvo")
    done < <(find "$ALVO" -mindepth 4 -maxdepth 4 -type d -path '*/build/*/*' -print0 2>/dev/null)

    # 2) Layout antigo: target/<perfil>/build/<crate>-<hash>/  (tem out/, output, stderr)
    while IFS= read -r -d '' alvo; do
        # diretório de crate do layout novo (contém filhos <hash16>): não é unidade antiga
        if [ -n "$(find "$alvo" -mindepth 1 -maxdepth 1 -type d -regextype posix-extended -regex '.*/[0-9a-f]{16}' -print -quit 2>/dev/null)" ]; then
            continue
        fi
        if [ -n "$(find "$alvo" -newermt "-${ativo} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        if [ -n "$(find "$alvo" -newermt "-${minutos} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        REMOVER+=("$alvo")
    done < <(find "$ALVO" -mindepth 3 -maxdepth 3 -type d -path '*/build/*-*' -print0 2>/dev/null)

    # 3) Incremental: target/<perfil>/incremental/<crate>-<sufixo>/
    while IFS= read -r -d '' alvo; do
        if [ -n "$(find "$alvo" -newermt "-${ativo} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        if [ -n "$(find "$alvo" -newermt "-${minutos} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        REMOVER+=("$alvo")
    done < <(find "$ALVO" -mindepth 3 -maxdepth 3 -type d -path '*/incremental/*' -print0 2>/dev/null)

    local novos=$(( ${#REMOVER[@]} - antes_dir ))
    [ "$rotulo" != "" ] && echo "cache: retenção ${rotulo} (${minutos} min) — ${novos} diretório(s) candidato(s)"
}

tamanho() { du -sb "$ALVO" 2>/dev/null | cut -f1; }

inicio=$(tamanho)
REMOVER=()

echo "cache: target/ com $(num "$inicio") — procurando diretórios sem uso há mais de ${DIAS} dia(s)"
poda "$DIAS" "principal"

# Sem o que podar na retenção normal e o target já grande? Aperta a retenção.
if [ ${#REMOVER[@]} -eq 0 ] && [ $(( inicio / 1073741824 )) -ge "$LIMITE_GB" ]; then
    echo "cache: nada passou de ${DIAS} dia(s) e o target/ está acima de ${LIMITE_GB} GB — apertando"
    poda 1 "1 dia"
    if [ ${#REMOVER[@]} -eq 0 ] && [ $(( inicio / 1073741824 )) -ge "$LIMITE_GB" ]; then
        poda 0.05 "1 hora"
    fi
fi

if [ ${#REMOVER[@]} -eq 0 ]; then
    echo "cache: nada a podar; target/ segue com $(num "$(tamanho)")"
    exit 0
fi

total=0
for alvo in "${REMOVER[@]}"; do
    tamanho_alvo=$(du -sb "$alvo" 2>/dev/null | cut -f1)
    total=$(( total + ${tamanho_alvo:-0} ))
    if [ "$SECO" -eq 1 ]; then
        echo "  [seco] $(num "${tamanho_alvo:-0}")  ${alvo#"$RAIZ"/}"
    else
        rm -rf -- "$alvo"
    fi
done

if [ "$SECO" -eq 1 ]; then
    echo "cache [seco]: ${#REMOVER[@]} diretório(s), $(num "$total") seriam liberados"
    exit 0
fi

# Remove diretórios de crate que ficaram vazios.
find "$ALVO" -mindepth 3 -maxdepth 3 -type d -path '*/build/*' -empty -delete 2>/dev/null

fim=$(tamanho)
echo "cache: ${#REMOVER[@]} diretório(s) removido(s), $(num "$total") liberados; target/ de $(num "$inicio") para $(num "$fim")"
exit 0

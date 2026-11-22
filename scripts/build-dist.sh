#!/usr/bin/env bash
# Build de distribuição do POP Harness (HR-56). Gera em release o `poph` e o `poph-extrator`
# lado a lado numa pasta de saída, mais um .tar.gz e a soma sha256. O `poph` procura o
# `poph-extrator` na pasta do próprio executável (docs/ARQUIVOS.md), então a pasta funciona sozinha,
# sem `target/` no PATH.
#
# O pacote é sempre NATIVO para a arquitetura da máquina onde o script roda:
#   x86_64  -> estação de desenvolvimento
#   aarch64 -> máquina ARM64 de testes (produção é ARM; o build ARM é feito lá, D-02/D-10)
# Não há compilação cruzada: a estação não tem compilador C cruzado (D-02), e o SQLite embutido
# (rusqlite `bundled`) e o ONNX Runtime precisam ser compilados/baixados para a arquitetura do host.
#
# O ONNX Runtime já entra linkado estaticamente no `poph` (crate poph-rag, `ort` com download-binaries
# resolve os binários da arquitetura do host). O modelo de embedding (118 MB) NÃO vai no pacote:
# é baixado depois com `poph model install`, com sha256 conferido (ver DEVELOPMENT.md).
#
# Uso: apps/pop-harness/scripts/build-dist.sh [pasta_de_saida]
#      padrão: apps/pop-harness/target/dist/poph-<versão>-<alvo>
set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RAIZ"
export PATH="$HOME/.cargo/bin:$PATH"
inicio=$SECONDS

ALVO="$(rustc -vV | sed -n 's/^host: //p')"
case "$ALVO" in
    x86_64-*|aarch64-*) ;;
    *)
        echo "build-dist: arquitetura não suportada pelo pacote: ${ALVO}." >&2
        echo "build-dist: os alvos são x86_64 (estação) e aarch64 (máquina ARM64 de testes)." >&2
        exit 2
        ;;
esac

VERSAO="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if git -C "$RAIZ" rev-parse --git-dir >/dev/null 2>&1; then
    COMMIT="$(git -C "$RAIZ" rev-parse --short HEAD)"
    if [ -n "$(git -C "$RAIZ" status --porcelain -- .)" ]; then
        COMMIT="${COMMIT} (com alterações não commitadas nesta pasta)"
    fi
else
    COMMIT="desconhecido (fora de git)"
fi

SAIDA="${1:-$RAIZ/target/dist/poph-${VERSAO}-${ALVO}}"
if [ -e "$SAIDA" ] || [ -e "${SAIDA}.tar.gz" ]; then
    echo "build-dist: saída já existe; escolha outra pasta: $SAIDA" >&2
    exit 1
fi

echo "==> build release nativo (${ALVO}): cargo build --release --locked -p poph -p poph-files --bins"
cargo build --release --locked -p poph -p poph-files --bins

ORIGEM="${CARGO_TARGET_DIR:-$RAIZ/target}/release"
for bin in poph poph-extrator; do
    if [ ! -x "$ORIGEM/$bin" ]; then
        echo "build-dist: não achei o executável ${ORIGEM}/${bin} depois do build." >&2
        exit 1
    fi
done

echo "==> copiando para ${SAIDA}"
mkdir -p "$SAIDA"
install -m 0755 "$ORIGEM/poph" "$ORIGEM/poph-extrator" "$SAIDA/"

# Confere que o pacote não depende de biblioteca do ONNX Runtime fora do executável.
LDD_POPH="$(ldd "$SAIDA/poph" 2>&1 || true)"
if [[ "${LDD_POPH,,}" == *onnx* ]]; then
    echo "build-dist: o poph depende de libonnxruntime dinâmica; o pacote ficaria incompleto." >&2
    echo "$LDD_POPH" >&2
    exit 1
fi

# Fumaça: os dois executáveis da pasta de saída respondem.
"$SAIDA/poph" --version >/dev/null
RESPOSTA_EXTRATOR="$("$SAIDA/poph-extrator" 2>/dev/null || true)"
if [[ "$RESPOSTA_EXTRATOR" != *'"ok":false'* ]]; then
    echo "build-dist: o poph-extrator não respondeu como esperado: ${RESPOSTA_EXTRATOR}" >&2
    exit 1
fi

(cd "$SAIDA" && sha256sum poph poph-extrator > SHA256SUMS)

cat > "$SAIDA/LEIA-ME.txt" <<EOF
POP Harness ${VERSAO} (${ALVO})
Gerado por scripts/build-dist.sh em $(date -Iseconds).
Commit de origem: ${COMMIT}

Conteúdo:
  poph              binário único: CLI e servidor (ONNX Runtime embutido)
  poph-extrator     leitor de PDF e Office, chamado pelo poph (manter na mesma pasta)
  SHA256SUMS        soma dos dois executáveis (sha256sum -c SHA256SUMS)

Modelo de embedding (não vem no pacote, 118 MB):
  ./poph model install     baixa o modelo e confere o sha256. O destino é a pasta de modelos da
                           configuração (padrão data/modelos, dentro da pasta de dados).

Sem Rust nem target/: copie a pasta inteira para o destino e rode ./poph --version.
EOF

# Dependências que o executável carrega (para o registro do pacote).
printf '%s\n' "$LDD_POPH" > "$SAIDA/LDD-poph.txt"

TAMANHO_POPH="$(stat -c %s "$SAIDA/poph")"
TAMANHO_EXTRATOR="$(stat -c %s "$SAIDA/poph-extrator")"
TAR="${SAIDA}.tar.gz"
tar -C "$(dirname "$SAIDA")" -czf "$TAR" "$(basename "$SAIDA")"
TAMANHO_TAR="$(stat -c %s "$TAR")"

echo
echo "==> pronto em $((SECONDS - inicio)) s"
echo "    pasta:          $SAIDA"
echo "    pacote:         $TAR ($TAMANHO_TAR bytes)"
echo "    poph:           $TAMANHO_POPH bytes"
echo "    poph-extrator:  $TAMANHO_EXTRATOR bytes"
echo "    versão:         $("$SAIDA/poph" --version)"
echo "    commit:         $COMMIT"

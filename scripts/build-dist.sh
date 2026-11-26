#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
START=$SECONDS

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
case "$TARGET" in
    x86_64-*|aarch64-*) ;;
    *)
        echo "build-dist: architecture not supported by the package: ${TARGET}." >&2
        echo "build-dist: the targets are x86_64 (workstation) and aarch64 (ARM64 test machine)." >&2
        exit 2
        ;;
esac

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    COMMIT="$(git -C "$ROOT" rev-parse --short HEAD)"
    if [ -n "$(git -C "$ROOT" status --porcelain -- .)" ]; then
        COMMIT="${COMMIT} (com alterações não commitadas nesta pasta)"
    fi
else
    COMMIT="desconhecido (fora de git)"
fi

OUTPUT="${1:-$ROOT/target/dist/poph-${VERSION}-${TARGET}}"
if [ -e "$OUTPUT" ] || [ -e "${OUTPUT}.tar.gz" ]; then
    echo "build-dist: output already exists; choose another folder: $OUTPUT" >&2
    exit 1
fi

echo "==> native release build (${TARGET}): cargo build --release --locked -p poph -p poph-files --bins"
cargo build --release --locked -p poph -p poph-files --bins

SOURCE_DIR="${CARGO_TARGET_DIR:-$ROOT/target}/release"
for bin in poph poph-extrator; do
    if [ ! -x "$SOURCE_DIR/$bin" ]; then
        echo "build-dist: executable not found at ${SOURCE_DIR}/${bin} after the build." >&2
        exit 1
    fi
done

echo "==> copying to ${OUTPUT}"
mkdir -p "$OUTPUT"
install -m 0755 "$SOURCE_DIR/poph" "$SOURCE_DIR/poph-extrator" "$OUTPUT/"

LDD_POPH="$(ldd "$OUTPUT/poph" 2>&1 || true)"
if [[ "${LDD_POPH,,}" == *onnx* ]]; then
    echo "build-dist: poph depends on a dynamic libonnxruntime; the package would be incomplete." >&2
    echo "$LDD_POPH" >&2
    exit 1
fi

"$OUTPUT/poph" --version >/dev/null
EXTRACTOR_REPLY="$("$OUTPUT/poph-extrator" 2>/dev/null || true)"
if [[ "$EXTRACTOR_REPLY" != *'"ok":false'* ]]; then
    echo "build-dist: poph-extrator did not answer as expected: ${EXTRACTOR_REPLY}" >&2
    exit 1
fi

(cd "$OUTPUT" && sha256sum poph poph-extrator > SHA256SUMS)

cat > "$OUTPUT/LEIA-ME.txt" <<EOF
POP Harness ${VERSION} (${TARGET})
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

printf '%s\n' "$LDD_POPH" > "$OUTPUT/LDD-poph.txt"

SIZE_POPH="$(stat -c %s "$OUTPUT/poph")"
SIZE_EXTRACTOR="$(stat -c %s "$OUTPUT/poph-extrator")"
TARBALL="${OUTPUT}.tar.gz"
tar -C "$(dirname "$OUTPUT")" -czf "$TARBALL" "$(basename "$OUTPUT")"
SIZE_TAR="$(stat -c %s "$TARBALL")"

echo
echo "==> done in $((SECONDS - START)) s"
echo "    folder:         $OUTPUT"
echo "    package:        $TARBALL ($SIZE_TAR bytes)"
echo "    poph:           $SIZE_POPH bytes"
echo "    poph-extrator:  $SIZE_EXTRACTOR bytes"
echo "    version:        $("$OUTPUT/poph" --version)"
echo "    commit:         $COMMIT"

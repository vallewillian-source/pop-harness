#!/usr/bin/env bash
# Gera o cliente TypeScript do Console a partir do OpenAPI do servidor (HR-20).
#
# Uso:
#   bash console/gen-api.sh              # grava console/src/api/schema.d.ts
#   bash console/gen-api.sh --out ARQ    # grava em outro lugar
#   bash console/gen-api.sh --url URL    # pega o OpenAPI de um servidor no ar
#
# O documento OpenAPI não é escrito à mão: sai de `poph openapi` (o mesmo que o servidor publica
# em `/admin/v1/openapi.json`). O cliente é gerado pelo `openapi-typescript`, que o script busca
# pelo `npx` — a primeira execução precisa de rede para baixar a ferramenta; depois ela fica no
# cache do npm.
#
# Requisitos: cargo (só quando usa `poph openapi`), node >= 22 e npx.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$root/console/src/api/schema.d.ts"
url=""
openapi=""

while [ $# -gt 0 ]; do
    case "$1" in
        --out) output="$2"; shift 2 ;;
        --url) url="$2"; shift 2 ;;
        --openapi) openapi="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) echo "argumento desconhecido: $1" >&2; exit 2 ;;
    esac
done

export PATH="$HOME/.cargo/bin:$PATH"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

if [ -z "$openapi" ]; then
    openapi="$temporary/openapi.json"
    if [ -n "$url" ]; then
        echo "== OpenAPI de $url"
        if command -v curl >/dev/null; then
            curl --fail --silent --show-error "${url%/}/admin/v1/openapi.json" -o "$openapi"
        else
            node -e '
                const fs = require("node:fs");
                fetch(process.argv[1]).then((r) => {
                    if (!r.ok) throw new Error(`HTTP ${r.status}`);
                    return r.text();
                }).then((t) => fs.writeFileSync(process.argv[2], t));
            ' "${url%/}/admin/v1/openapi.json" "$openapi"
        fi
    else
        echo "== OpenAPI de \`poph openapi\`"
        cargo run --quiet --manifest-path "$root/Cargo.toml" -p poph -- openapi --out "$openapi"
    fi
fi

echo "== Cliente TypeScript ($output)"
mkdir -p "$(dirname "$output")"
if [ -x "$root/console/node_modules/.bin/openapi-typescript" ]; then
    "$root/console/node_modules/.bin/openapi-typescript" "$openapi" -o "$output"
else
    npx --yes openapi-typescript@7 "$openapi" -o "$output"
fi

if [ ! -s "$output" ]; then
    echo "o gerador não produziu nada em $output" >&2
    exit 1
fi
if ! grep -q "^export interface paths" "$output"; then
    echo "$output não parece um cliente gerado do OpenAPI (sem \`export interface paths\`)" >&2
    exit 1
fi

lines="$(wc -l < "$output")"
routes="$(grep -c '^    "/' "$output" || true)"
echo "OK: $output ($lines lines, $routes routes)"

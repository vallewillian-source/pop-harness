#!/usr/bin/env bash
set -euo pipefail

console="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="$(cd "$console/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

if [ ! -x "$console/node_modules/.bin/openapi-typescript" ]; then
    echo "PULADO: console/node_modules ausente."
    echo "        Rode \`cd apps/pop-harness/console && pnpm install\` (exige rede uma vez) e repita."
    exit 0
fi

temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

bash "$console/gen-api.sh" --out "$temporary/schema.d.ts" >"$temporary/log" 2>&1 || {
    cat "$temporary/log"
    echo "o gerador do cliente falhou" >&2
    exit 1
}

if ! diff -u "$console/src/api/schema.d.ts" "$temporary/schema.d.ts" >"$temporary/diff"; then
    echo "console/src/api/schema.d.ts está diferente do OpenAPI do servidor." >&2
    echo "Rode \`cd apps/pop-harness && bash console/gen-api.sh\` e versione o resultado." >&2
    head -60 "$temporary/diff" >&2
    exit 1
fi

echo "OK: cliente do OpenAPI em dia ($(wc -l <"$console/src/api/schema.d.ts") linhas)"

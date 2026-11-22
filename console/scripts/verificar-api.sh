#!/usr/bin/env bash
# Confere que `console/src/api/schema.d.ts` é exatamente o que o gerador produz hoje.
#
# Regra do contrato (HR-38): o cliente do OpenAPI é gerado, nunca escrito à mão. Se o documento do
# servidor mudou e o arquivo não foi regenerado, esta checagem falha — o CI pega a divergência.
#
# Rede: com `console/node_modules` instalado, a checagem roda **offline** (usa o
# `openapi-typescript` local). Sem `node_modules`, ela se declara pulada e sai com 0, para o CI
# continuar funcionando num clone limpo.
set -euo pipefail

console="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
raiz="$(cd "$console/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

if [ ! -x "$console/node_modules/.bin/openapi-typescript" ]; then
    echo "PULADO: console/node_modules ausente."
    echo "        Rode \`cd apps/pop-harness/console && pnpm install\` (exige rede uma vez) e repita."
    exit 0
fi

temporaria="$(mktemp -d)"
trap 'rm -rf "$temporaria"' EXIT

bash "$console/gen-api.sh" --out "$temporaria/schema.d.ts" >"$temporaria/log" 2>&1 || {
    cat "$temporaria/log"
    echo "o gerador do cliente falhou" >&2
    exit 1
}

if ! diff -u "$console/src/api/schema.d.ts" "$temporaria/schema.d.ts" >"$temporaria/diff"; then
    echo "console/src/api/schema.d.ts está diferente do OpenAPI do servidor." >&2
    echo "Rode \`cd apps/pop-harness && bash console/gen-api.sh\` e versione o resultado." >&2
    head -60 "$temporaria/diff" >&2
    exit 1
fi

echo "OK: cliente do OpenAPI em dia ($(wc -l <"$console/src/api/schema.d.ts") linhas)"

#!/usr/bin/env bash
# Testes de componente do Console (HR-39a): Vitest + Testing Library em jsdom.
#
# Sem `node_modules` a etapa se declara pulada e sai com 0, igual ao build do bundle e à checagem
# do cliente OpenAPI — o CI offline de um clone limpo continua funcionando.
set -euo pipefail

console="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$console"

if [ ! -d node_modules ]; then
    echo "PULADO: console/node_modules ausente — sem testes de componente."
    echo "        Rode \`cd apps/pop-harness/console && pnpm install\` (exige rede uma vez) e repita."
    exit 0
fi

if ! command -v pnpm >/dev/null; then
    echo "pnpm não encontrado no PATH; instale o pnpm (corepack enable)." >&2
    exit 1
fi

pnpm run testar
echo "OK: testes de componente do Console"

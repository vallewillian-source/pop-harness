#!/usr/bin/env bash
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

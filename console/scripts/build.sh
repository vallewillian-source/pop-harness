#!/usr/bin/env bash
# Gera o bundle do Console (`console/dist`) que o `build.rs` do `poph-server` embute no binário.
#
# Rede: só a primeira instalação (`pnpm install`) precisa dela. Sem `node_modules` esta etapa se
# declara pulada — o `cargo build` segue funcionando e serve a página de aviso em `/`.
set -euo pipefail

console="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$console"

if [ ! -d node_modules ]; then
    echo "PULADO: console/node_modules ausente — o binário vai sair sem o Console."
    echo "        Rode \`cd apps/pop-harness/console && pnpm install\` e depois este script."
    exit 0
fi

if ! command -v pnpm >/dev/null; then
    echo "pnpm não encontrado no PATH; instale o pnpm (corepack enable) ou use npm run build." >&2
    exit 1
fi

pnpm run build
echo "OK: bundle em console/dist ($(find dist -type f | wc -l) arquivos)"

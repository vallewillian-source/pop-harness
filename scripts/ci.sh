#!/usr/bin/env bash
set -uo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RAIZ" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

falhas=0

executa() {
    local nome="$1"
    shift
    echo
    echo "==> ${nome}: $*"
    if "$@"; then
        echo "<== OK: ${nome}"
    else
        local rc=$?
        echo "<== FALHOU: ${nome} (código ${rc})"
        falhas=$((falhas + 1))
    fi
}

executa "cache de compilação (poda do que não é usado há 3 dias)" bash scripts/limpar-cache.sh --dias 3
executa "console (bundle do Console embutido no binário)" bash console/scripts/build.sh
executa "console (cliente do OpenAPI em dia)" bash console/scripts/verify-api.sh
executa "console (testes de componente)" bash console/scripts/test.sh

executa "comentários (teste do removedor)" python3 scripts/tests/test-strip-comments.py
executa "comentários (trava: allowlist e pendentes por módulo)" bash scripts/check-comments.sh
executa "glossário (termos, congelados e tabela em dia)" python3 scripts/glossary-terms.py --validar
executa "nomes (trava: allowlist de publicados e pendentes por módulo)" bash scripts/check-names.sh
executa "nomes (teste do renomeador)" python3 scripts/tests/test-rename.py

executa "formatação" cargo fmt --all -- --check
executa "clippy" cargo clippy --workspace --all-targets --locked -- -D warnings
executa "build" cargo build --workspace --locked
executa "testes" cargo test --workspace --locked
executa "ram (medição do modelo, poph-rag sozinho)" cargo test --locked -p poph-rag --test ram -- --ignored --nocapture
executa "aarch64 (check, crates sem C)" cargo check --locked --target aarch64-unknown-linux-gnu -p poph-types -p poph-files -p poph-plugins -p poph-fluxos
executa "aarch64 (check, plugin de exemplo)" cargo check --locked --target aarch64-unknown-linux-gnu -p poph-plugin-eco
executa "deny (licenças, bans, fontes, advisories)" cargo deny check
executa "audit (vulnerabilidades)" cargo audit

echo
if [ "$falhas" -eq 0 ]; then
    echo "CI LOCAL: OK (todas as checagens passaram)"
    exit 0
fi
echo "CI LOCAL: FALHOU (${falhas} checagem(ns) com erro)"
exit 1

#!/usr/bin/env bash
# Registro e conversa de teste do plugin de exemplo (HR-27).
#
# Compila o plugin `plugin-eco` e roda a demonstração de ponta a ponta num banco temporário:
#
#   outbox (texto) → supervisor → plugin eco → evento de entrada → inbox → mensagem invertida
#
# Uso: apps/pop-harness/scripts/plugin-eco.sh
# Não precisa de passo manual: o banco, a org, o agente e a conversa são criados pela demonstração,
# e a pasta temporária é apagada no fim (inclusive quando algo falha).
set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RAIZ" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

PASTA="$(mktemp -d "${TMPDIR:-/tmp}/poph-eco-demo.XXXXXX")"
limpar() {
    rm -rf "$PASTA"
}
trap limpar EXIT

echo "==> compilando o plugin de exemplo e a demonstração"
cargo build -q -p poph-plugin-eco --bins --examples

echo "==> banco temporário: ${PASTA}"
"${RAIZ}/target/debug/examples/plugin-eco-demo" "$PASTA"

# `pgrep` sai com 1 quando não acha ninguém: com `|| true` o script não morre no caso bom.
vivos="$(pgrep -cf "${RAIZ}/target/debug/plugin-eco" || true)"
vivos="${vivos:-0}"
echo
echo "==> processos do plugin ainda vivos: ${vivos}"
if [ "$vivos" -ne 0 ]; then
    echo "FALHOU: sobrou processo do plugin depois da demonstração"
    pgrep -af "${RAIZ}/target/debug/plugin-eco" || true
    exit 1
fi
echo "OK: conversa de teste passou pelo plugin eco e nada ficou rodando"

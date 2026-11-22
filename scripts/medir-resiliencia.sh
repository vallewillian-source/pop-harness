#!/usr/bin/env bash
# Mede o caminho do cliente (webhook até a resposta chegar ao WhatsApp) com o módulo Matrix
# desligado e ligado, com o Synapse fora, lento (5 s) e no ar (MX-29).
#
# Uso (a partir de apps/pop-harness):
#   bash scripts/medir-resiliencia.sh [rodadas] [amostras]
#
# Padrão: 3 rodadas de 40 mensagens por cenário. O CSV sai em target/mx29/medicao.csv (fora do
# git). O resumo impresso traz a mediana e o p95 de cada cenário. Ver docs/MATRIZ_RESILIENCIA.md.
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

RODADAS="${1:-3}"
AMOSTRAS="${2:-40}"
SAIDA_DIR="target/mx29"
mkdir -p "$SAIDA_DIR"
CSV="$(pwd)/$SAIDA_DIR/medicao.csv"

MX29_RODADAS="$RODADAS" MX29_AMOSTRAS="$AMOSTRAS" MX29_CSV="$CSV" \
  cargo test --locked -p poph-server --test matrix_resiliencia \
  -- --ignored medicao_cliente_primeiro --nocapture --test-threads=1

echo "CSV: $CSV"

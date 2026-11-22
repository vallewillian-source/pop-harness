#!/usr/bin/env bash
# Roteiro completo da medição de RAM e vazão do módulo Matrix (MX-30). Chama scripts/medir-memoria.sh
# na matriz de cenários do docs/MATRIZ_MEMORIA.md: módulo desligado e ligado em 100, 1.000 e 10.000
# salas, vazão com 10 mensagens por sala, e uma rodada pequena contra o Synapse de dev (MX-01).
#
# Uso (a partir de apps/pop-harness), depois de `cargo build --release --locked -p poph`:
#   bash scripts/medir-memoria-roteiro.sh              # matriz completa (demora; ver o relatório)
#   bash scripts/medir-memoria-roteiro.sh rapido       # 100 salas, uma rodada: checagem rápida
#
# O CSV sai em target/mx30/memoria.csv (fora do git). Se uma rodada falhar, o roteiro para (set -e).
set -euo pipefail

cd "$(dirname "$0")/.."
MEDIR="bash scripts/medir-memoria.sh"

if [ "${1:-}" = rapido ]; then
    $MEDIR --modo desligado --salas 100 --rodada 1
    $MEDIR --modo ligado --salas 100 --mensagens 1 --rodada 1
    exit 0
fi

for rodada in 1 2 3; do
    for salas in 100 1000; do
        $MEDIR --modo desligado --salas "$salas" --rodada "$rodada"
        $MEDIR --modo ligado --salas "$salas" --mensagens 1 --rodada "$rodada"
    done
done

for rodada in 1 2 3; do
    $MEDIR --modo desligado --salas 10000 --rodada "$rodada"
done
for rodada in 1 2; do
    $MEDIR --modo ligado --salas 10000 --mensagens 1 --rodada "$rodada"
done

# Vazão: 1.000 salas com 10 mensagens cada (10.000 mensagens), duas rodadas.
for rodada in 1 2; do
    $MEDIR --modo ligado --salas 1000 --mensagens 10 --rodada "$rodada"
done

# Synapse real de desenvolvimento (MX-01, porta 18008), escala pequena.
MX30_LIMITE_S=300 $MEDIR --modo real --salas 20 --mensagens 1 --rodada 1

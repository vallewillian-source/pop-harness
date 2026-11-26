#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
MEASURE="bash scripts/medir-memoria.sh"

if [ "${1:-}" = rapido ]; then
    $MEASURE --modo desligado --salas 100 --rodada 1
    $MEASURE --modo ligado --salas 100 --mensagens 1 --rodada 1
    exit 0
fi

for round in 1 2 3; do
    for rooms in 100 1000; do
        $MEASURE --modo desligado --salas "$rooms" --rodada "$round"
        $MEASURE --modo ligado --salas "$rooms" --mensagens 1 --rodada "$round"
    done
done

for round in 1 2 3; do
    $MEASURE --modo desligado --salas 10000 --rodada "$round"
done
for round in 1 2; do
    $MEASURE --modo ligado --salas 10000 --mensagens 1 --rodada "$round"
done

for round in 1 2; do
    $MEASURE --modo ligado --salas 1000 --mensagens 10 --rodada "$round"
done

MX30_LIMITE_S=300 $MEASURE --modo real --salas 20 --mensagens 1 --rodada 1

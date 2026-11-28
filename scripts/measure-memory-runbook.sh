#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
MEASURE="bash scripts/measure-memory.sh"

if [ "${1:-}" = quick ]; then
    $MEASURE --mode off --rooms 100 --round 1
    $MEASURE --mode on --rooms 100 --messages 1 --round 1
    exit 0
fi

for round in 1 2 3; do
    for rooms in 100 1000; do
        $MEASURE --mode off --rooms "$rooms" --round "$round"
        $MEASURE --mode on --rooms "$rooms" --messages 1 --round "$round"
    done
done

for round in 1 2 3; do
    $MEASURE --mode off --rooms 10000 --round "$round"
done
for round in 1 2; do
    $MEASURE --mode on --rooms 10000 --messages 1 --round "$round"
done

for round in 1 2; do
    $MEASURE --mode on --rooms 1000 --messages 10 --round "$round"
done

MX30_LIMIT_S=300 $MEASURE --mode real --rooms 20 --messages 1 --round 1

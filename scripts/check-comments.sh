#!/usr/bin/env bash
set -uo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

exec python3 "$RAIZ/scripts/check_comments.py" --raiz "$RAIZ" "$@"

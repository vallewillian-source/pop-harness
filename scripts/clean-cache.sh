#!/usr/bin/env bash
# HR-60 — Pruning the Cargo build cache.
#
# Why it exists: since Cargo 1.100 the build-dir layout changed (cargo#15010) and EVERY intermediate
# artifact now lives in `target/<profile>/build/<crate>/<unit-hash>/out/`. The hash changes when the
# effective configuration of the unit changes (dev/test/check profile, features unified by the command
# shape, target, toolchain and dependency hashes). Cargo never deletes old hash directories, and each
# test binary carried full debuginfo (~300 MB before HR-60b; today the dev profile uses
# `line-tables-only` and the binaries are ~56% smaller — pruning is still needed, because Cargo never
# deletes the old units).
# Result: `target/` grows with every different command shape until the disk is full.
#
# What it does: deletes the units and incremental directories that have not been touched for N days
# (default 3), keeping what a recent build may still be using, and reduces retention on its own when
# `target/` exceeds the GB limit. Never touches code, `Cargo.lock` or any file outside `target/`.
#
# Usage:
#   scripts/clean-cache.sh                 # prunes what is older than 3 days; reduces if over 25 GB
#   scripts/clean-cache.sh --dry-run          # only shows what would be deleted
#   scripts/clean-cache.sh --days 7        # longer retention
#   scripts/clean-cache.sh --limit-gb 40  # size ceiling for target
#   scripts/clean-cache.sh --all          # `cargo clean` (deletes the whole target; respects an active build)
#   scripts/clean-cache.sh --force        # prunes even with a build running (test only)
#
# Output: summary with bytes freed. Exit code 0 also when it skips because a build is running —
# CI must not fail because of pruning.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

DAYS=3
LIMIT_GB=25
DRY=0
ALL=0
FORCE=0
ACTIVE_MINUTES=120

usage() {
    sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --days) DAYS="${2:-}"; shift 2 ;;
        --days=*) DAYS="${1#*=}"; shift ;;
        --limit-gb) LIMIT_GB="${2:-}"; shift 2 ;;
        --limit-gb=*) LIMIT_GB="${1#*=}"; shift ;;
        --active-minutes) ACTIVE_MINUTES="${2:-}"; shift 2 ;;
        --dry-run) DRY=1; shift ;;
        --all) ALL=1; shift ;;
        --force) FORCE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

case "$DAYS" in ''|*[!0-9.]*) echo "--days needs a number (e.g. 3 or 0.5)" >&2; exit 2 ;; esac
case "$LIMIT_GB" in ''|*[!0-9]*) echo "--limit-gb needs an integer" >&2; exit 2 ;; esac

num() { awk -v b="${1:-0}" 'BEGIN{ printf "%.2f GB", b/1073741824 }'; }

TARGET="$ROOT/target"
if [ ! -d "$TARGET" ]; then
    echo "cache: no target/ in $ROOT — nothing to prune"
    exit 0
fi

if [ "$ALL" -eq 1 ]; then
    if [ "$FORCE" -eq 0 ] && { pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; }; then
        echo "cache: a cargo/rustc is running on this machine — --all postponed (use --force to delete anyway)"
        exit 0
    fi
    before=$(du -sb "$TARGET" 2>/dev/null | cut -f1)
    if [ "$DRY" -eq 1 ]; then
        echo "cache: --dry-run with --all: would delete the whole target/ ($(num "$before") )"
        exit 0
    fi
    echo "cache: deleting the whole target/ (cargo clean)"
    cargo clean 2>/dev/null || rm -rf "$TARGET"
    echo "cache: freed $(num "$before") — the next build compiles from scratch"
    exit 0
fi

if [ "$FORCE" -eq 0 ] && { pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; }; then
    echo "cache: a cargo/rustc is running on this machine — pruning postponed (does not fail CI)"
    exit 0
fi

prune() {
    local days="$1" label="$2"
    local minutes
    minutes=$(awk -v d="$days" 'BEGIN{ printf "%d", d*1440 }')
    [ "$minutes" -lt 1 ] && minutes=1

    local before_dir=${#REMOVE[@]}
    local active=$ACTIVE_MINUTES
    [ "$active" -ge "$minutes" ] && active=$(( minutes / 2 ))
    [ "$active" -lt 5 ] && active=0
    local candidate

    while IFS= read -r -d '' candidate; do
        local base; base=$(basename "$candidate")
        [[ "$base" =~ ^[0-9a-f]{16}$ ]] || continue
        if [ -n "$(find "$candidate" -newermt "-${active} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        if [ -n "$(find "$candidate" -newermt "-${minutes} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        REMOVE+=("$candidate")
    done < <(find "$TARGET" -mindepth 4 -maxdepth 4 -type d -path '*/build/*/*' -print0 2>/dev/null)

    while IFS= read -r -d '' candidate; do
        if [ -n "$(find "$candidate" -mindepth 1 -maxdepth 1 -type d -regextype posix-extended -regex '.*/[0-9a-f]{16}' -print -quit 2>/dev/null)" ]; then
            continue
        fi
        if [ -n "$(find "$candidate" -newermt "-${active} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        if [ -n "$(find "$candidate" -newermt "-${minutes} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        REMOVE+=("$candidate")
    done < <(find "$TARGET" -mindepth 3 -maxdepth 3 -type d -path '*/build/*-*' -print0 2>/dev/null)

    while IFS= read -r -d '' candidate; do
        if [ -n "$(find "$candidate" -newermt "-${active} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        if [ -n "$(find "$candidate" -newermt "-${minutes} minutes" -print -quit 2>/dev/null)" ]; then
            continue
        fi
        REMOVE+=("$candidate")
    done < <(find "$TARGET" -mindepth 3 -maxdepth 3 -type d -path '*/incremental/*' -print0 2>/dev/null)

    local added=$(( ${#REMOVE[@]} - before_dir ))
    [ "$label" != "" ] && echo "cache: retention ${label} (${minutes} min) — ${added} candidate directory(ies)"
}

size() { du -sb "$TARGET" 2>/dev/null | cut -f1; }

start=$(size)
REMOVE=()

echo "cache: target/ at $(num "$start") — looking for directories unused for more than ${DAYS} day(s)"
prune "$DAYS" "main"

if [ ${#REMOVE[@]} -eq 0 ] && [ $(( start / 1073741824 )) -ge "$LIMIT_GB" ]; then
    echo "cache: nothing older than ${DAYS} day(s) and target/ is above ${LIMIT_GB} GB — tightening"
    prune 1 "1 day"
    if [ ${#REMOVE[@]} -eq 0 ] && [ $(( start / 1073741824 )) -ge "$LIMIT_GB" ]; then
        prune 0.05 "1 hour"
    fi
fi

if [ ${#REMOVE[@]} -eq 0 ]; then
    echo "cache: nothing to prune; target/ stays at $(num "$(size)")"
    exit 0
fi

total=0
for candidate in "${REMOVE[@]}"; do
    candidate_size=$(du -sb "$candidate" 2>/dev/null | cut -f1)
    total=$(( total + ${candidate_size:-0} ))
    if [ "$DRY" -eq 1 ]; then
        echo "  [dry] $(num "${candidate_size:-0}")  ${candidate#"$ROOT"/}"
    else
        rm -rf -- "$candidate"
    fi
done

if [ "$DRY" -eq 1 ]; then
    echo "cache [dry]: ${#REMOVE[@]} directory(ies), $(num "$total") would be freed"
    exit 0
fi

find "$TARGET" -mindepth 3 -maxdepth 3 -type d -path '*/build/*' -empty -delete 2>/dev/null

end=$(size)
echo "cache: ${#REMOVE[@]} directory(ies) removed, $(num "$total") freed; target/ from $(num "$start") to $(num "$end")"
exit 0

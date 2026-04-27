#!/usr/bin/env bash
# Extract sister Hopper repos with full history preserved.
#
# Operates on a fresh clone of the canonical repo placed in `extract-work/`,
# never on the working tree itself. For each target it:
#   1. Re-clones the canonical repo into a scratch dir
#   2. Runs `git filter-repo --path <subdir>` to keep only that crate
#   3. Rewrites paths so the crate lives at the new repo root
#
# Push the resulting repos to GitHub manually after review.
#
# Requires: git-filter-repo (https://github.com/newren/git-filter-repo)
#
# Usage:
#   ./scripts/extract-sister-repos.sh --list
#   ./scripts/extract-sister-repos.sh
#   ./scripts/extract-sister-repos.sh --only hopper-runtime

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$ROOT/extract-work"
SOURCE_REMOTE="${SOURCE_REMOTE:-$ROOT}"

# Sister-repo extraction plan: <new-repo-name>=<source-subdir>[:rename-to]
TARGETS=(
    "hopper-runtime=crates/hopper-runtime"
    "hopper-core=crates/hopper-core"
    "hopper-macros=crates/hopper-macros:macros"
    "hopper-derive=crates/hopper-macros-proc:derive"
    "hopper-spl=crates/hopper-token,crates/hopper-token-2022,crates/hopper-associated-token,crates/hopper-metaplex"
    "hopper-cli=tools/hopper-cli"
    "hopper-bench=bench"
)

list_plan() {
    echo "Extraction plan (target -> source paths):"
    for t in "${TARGETS[@]}"; do
        local name="${t%%=*}"
        local rest="${t#*=}"
        echo "  $name <- $rest"
    done
}

require_filter_repo() {
    if ! command -v git-filter-repo >/dev/null 2>&1; then
        echo "git-filter-repo is required. Install with: pip install git-filter-repo" >&2
        exit 1
    fi
}

extract_one() {
    local spec="$1"
    local name="${spec%%=*}"
    local paths_csv="${spec#*=}"
    local target="$WORK/$name"

    rm -rf "$target"
    git clone --quiet "$SOURCE_REMOTE" "$target"

    local args=()
    IFS=',' read -ra paths <<<"$paths_csv"
    for p in "${paths[@]}"; do
        local src="${p%%:*}"
        args+=("--path" "$src")
        if [[ "$p" == *:* ]]; then
            local dst="${p#*:}"
            args+=("--path-rename" "$src:$dst")
        fi
    done

    echo "[$name] filtering with: ${args[*]}"
    (cd "$target" && git filter-repo --force "${args[@]}")
    echo "[$name] extracted to $target"
}

main() {
    if [[ "${1:-}" == "--list" ]]; then
        list_plan
        exit 0
    fi

    require_filter_repo
    mkdir -p "$WORK"

    if [[ "${1:-}" == "--only" ]]; then
        local want="${2:-}"
        for t in "${TARGETS[@]}"; do
            [[ "${t%%=*}" == "$want" ]] && extract_one "$t" && exit 0
        done
        echo "Unknown target: $want" >&2
        list_plan
        exit 1
    fi

    for t in "${TARGETS[@]}"; do
        extract_one "$t"
    done

    echo
    echo "All sister repos extracted under $WORK"
    echo "Review each, then push to its new GitHub remote."
}

main "$@"

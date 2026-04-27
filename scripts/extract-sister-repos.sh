#!/usr/bin/env bash
# Extract sister Hopper repos with full history preserved, then optionally
# create and push them to the BluefootLabs GitHub org.
#
# Operates on a fresh clone of the canonical repo placed in `extract-work/`,
# never on the working tree itself. For each target it:
#   1. Re-clones the canonical repo into a scratch dir
#   2. Runs `git filter-repo --path <subdir>` to keep only that crate
#   3. Rewrites paths so the crate lives at the new repo root
#   4. With --push, runs `gh repo create BluefootLabs/<name>` and pushes main
#
# Requires:
#   - git-filter-repo (pip install git-filter-repo)
#   - gh CLI authenticated with org access (only needed for --push)
#
# Usage:
#   ./scripts/extract-sister-repos.sh --list
#   ./scripts/extract-sister-repos.sh                    # extract only
#   ./scripts/extract-sister-repos.sh --only hopper-runtime
#   ./scripts/extract-sister-repos.sh --push             # extract + push all
#   ./scripts/extract-sister-repos.sh --only hopper-cli --push

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$ROOT/extract-work"
SOURCE_REMOTE="${SOURCE_REMOTE:-$ROOT}"
GH_OWNER="${GH_OWNER:-BluefootLabs}"
GH_VISIBILITY="${GH_VISIBILITY:-public}"

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

require_gh() {
    if ! command -v gh >/dev/null 2>&1; then
        echo "gh CLI is required for --push. Install: https://cli.github.com/" >&2
        exit 1
    fi
    if ! gh auth status >/dev/null 2>&1; then
        echo "gh CLI is not authenticated. Run: gh auth login" >&2
        exit 1
    fi
}

push_one() {
    local name="$1"
    local target="$WORK/$name"
    local slug="$GH_OWNER/$name"

    if gh repo view "$slug" >/dev/null 2>&1; then
        echo "[$name] $slug already exists; pushing main"
    else
        echo "[$name] creating $slug ($GH_VISIBILITY)"
        gh repo create "$slug" "--$GH_VISIBILITY" --confirm >/dev/null
    fi

    git -C "$target" remote remove origin 2>/dev/null || true
    git -C "$target" remote add origin "https://github.com/$slug.git"
    git -C "$target" branch -M main
    git -C "$target" push -u origin main --force
    git -C "$target" push origin --tags || true
    echo "[$name] pushed to https://github.com/$slug"
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
    local do_push=0
    local only=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --list) list_plan; exit 0 ;;
            --push) do_push=1; shift ;;
            --only) only="${2:-}"; shift 2 ;;
            *) echo "Unknown arg: $1" >&2; exit 1 ;;
        esac
    done

    require_filter_repo
    [[ "$do_push" -eq 1 ]] && require_gh
    mkdir -p "$WORK"

    if [[ -n "$only" ]]; then
        for t in "${TARGETS[@]}"; do
            if [[ "${t%%=*}" == "$only" ]]; then
                extract_one "$t"
                [[ "$do_push" -eq 1 ]] && push_one "$only"
                exit 0
            fi
        done
        echo "Unknown target: $only" >&2
        list_plan
        exit 1
    fi

    for t in "${TARGETS[@]}"; do
        extract_one "$t"
        [[ "$do_push" -eq 1 ]] && push_one "${t%%=*}"
    done

    echo
    echo "All sister repos extracted under $WORK"
    if [[ "$do_push" -eq 0 ]]; then
        echo "Re-run with --push to create them under $GH_OWNER and push."
    fi
}

main "$@"

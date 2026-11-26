#!/usr/bin/env bash
# Keep source files small enough that two pull requests rarely edit the same one
# (DEVELOPMENT.md, "Where code goes"). Fails when a tracked .rs file:
#
#   - has more than 3000 lines and isn't in scripts/file-size-limits.txt
#   - is in that list and is longer than its limit there, or no longer over 3000
#   - is in scripts/no-inline-tests.txt and has a #[test] in it
#
#   scripts/check-file-sizes.sh              # this repository (mise run lint runs it)
#   scripts/check-file-sizes.sh --root DIR   # another git work tree (the tests)
set -euo pipefail

CAP=3000
LIMITS=scripts/file-size-limits.txt
NO_TESTS=scripts/no-inline-tests.txt

root="$(cd "$(dirname "$0")/.." && pwd)"
if [ "${1:-}" = "--root" ]; then
    root="${2:?--root needs a folder}"
fi
cd "$root"

failed=0
fail() {
    echo "$1" >&2
    failed=1
}

tracked() { git ls-files --error-unmatch -- "$1" > /dev/null 2>&1; }

# A list file's entries, without comments and blank lines.
entries() { [ -f "$1" ] && tr -d '\r' < "$1" | grep -vE '^[[:space:]]*(#|$)' || true; }

# The files with a limit of their own: "<path> <max lines>" per line.
limit_paths=()
limit_maxes=()
while IFS= read -r line; do
    if [[ ! "$line" =~ ^.+\ [0-9]+$ ]]; then
        fail "$LIMITS: bad entry '$line' (expected \"<path> <max lines>\")"
        continue
    fi
    limit_paths+=("${line% *}")
    limit_maxes+=("${line##* }")
done < <(entries "$LIMITS")

# limit_of PATH: its limit, or nothing if it isn't listed.
limit_of() {
    local i
    for i in "${!limit_paths[@]}"; do
        if [ "${limit_paths[$i]}" = "$1" ]; then
            echo "${limit_maxes[$i]}"
            return
        fi
    done
}

while IFS= read -r -d '' file; do
    [ -f "$file" ] || continue
    # Lines, counting a last one that has no newline.
    n="$(grep -c '' "$file" || true)"
    max="$(limit_of "$file")"
    if [ -z "$max" ]; then
        if [ "$n" -gt "$CAP" ]; then
            fail "$file: $n lines, over the $CAP-line cap"
        fi
    elif [ "$n" -gt "$max" ]; then
        fail "$file: $n lines, over its limit of $max"
    elif [ "$n" -le "$CAP" ]; then
        fail "$file: $n lines, under the cap: remove it from $LIMITS"
    fi
done < <(git ls-files -z -- '*.rs')

for i in "${!limit_paths[@]}"; do
    tracked "${limit_paths[$i]}" || fail "${limit_paths[$i]}: listed in $LIMITS but not in the repository"
done

# A test attribute on a line of its own: #[test], or a path ending in it (#[tokio::test]).
while IFS= read -r file; do
    if ! tracked "$file"; then
        fail "$file: listed in $NO_TESTS but not in the repository"
        continue
    fi
    while IFS=: read -r number _; do
        fail "$file:$number: inline test; tests go in their own file"
    done < <(grep -anE '^[[:space:]]*#\[([A-Za-z_][A-Za-z0-9_]*::)*test\]' "$file" || true)
done < <(entries "$NO_TESTS")

if [ "$failed" -ne 0 ]; then
    echo "See DEVELOPMENT.md, \"Where code goes\", for where new code and tests belong." >&2
fi
exit "$failed"

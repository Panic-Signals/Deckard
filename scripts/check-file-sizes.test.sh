#!/usr/bin/env bash
# Tests for check-file-sizes.sh: each case builds a small git repository in a temporary
# folder and runs the check on it.
#
#   scripts/check-file-sizes.test.sh        # prints "ok <case>" or "FAIL <case>" per case
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
check="$here/check-file-sizes.sh"
failed=0
tmp=""
err=""
status=0

cleanup() { [ -n "$tmp" ] && rm -rf "$tmp"; }
trap cleanup EXIT

# A new empty repository with both list files present and empty.
repo() {
    cleanup
    tmp="$(mktemp -d)"
    git -C "$tmp" init -q
    mkdir -p "$tmp/scripts"
    : > "$tmp/scripts/file-size-limits.txt"
    : > "$tmp/scripts/no-inline-tests.txt"
}

# lines FILE N: a file of N lines, ending in a newline.
lines() {
    mkdir -p "$(dirname "$tmp/$1")"
    seq 1 "$2" > "$tmp/$1"
}

# lines_open FILE N: a file of N lines whose last line has no newline.
lines_open() {
    mkdir -p "$(dirname "$tmp/$1")"
    seq 1 "$2" | perl -pe 'chomp if eof' > "$tmp/$1"
}

run() {
    git -C "$tmp" add -A . > /dev/null 2>&1
    err="$("$check" --root "$tmp" 2>&1 > /dev/null)"
    status=$?
}

# expect CASE STATUS [TEXT]: the last run's exit status, and text its stderr must contain
# (with status 0, stderr must be empty).
expect() {
    local name="$1" want="$2" text="${3:-}"
    if [ "$status" -ne "$want" ]; then
        echo "FAIL $name: exit $status, expected $want: $err"
        failed=1
    elif [ -n "$text" ] && [[ "$err" != *"$text"* ]]; then
        echo "FAIL $name: stderr lacks '$text': $err"
        failed=1
    elif [ "$want" -eq 0 ] && [ -n "$err" ]; then
        echo "FAIL $name: stderr not empty: $err"
        failed=1
    else
        echo "ok $name"
    fi
}

repo
lines a.rs 10
run
expect clean 0

repo
lines a.rs 3001
run
expect over-default-cap 1 "a.rs: 3001 lines, over the 3000-line cap"

repo
lines a.rs 3000
run
expect at-default-cap 0

repo
lines big.rs 3400
echo "big.rs 3500" > "$tmp/scripts/file-size-limits.txt"
run
expect listed-within-limit 0

repo
lines big.rs 3501
echo "big.rs 3500" > "$tmp/scripts/file-size-limits.txt"
run
expect listed-over-limit 1 "big.rs: 3501 lines, over its limit of 3500"

repo
lines big.rs 2000
echo "big.rs 3500" > "$tmp/scripts/file-size-limits.txt"
run
expect listed-under-cap 1 "big.rs: 2000 lines, under the cap: remove it from scripts/file-size-limits.txt"

repo
lines a.rs 10
echo "gone.rs 3500" > "$tmp/scripts/file-size-limits.txt"
run
expect stale-entry 1 "gone.rs: listed in scripts/file-size-limits.txt but not in the repository"

repo
lines a.rs 10
echo "gone.rs" > "$tmp/scripts/no-inline-tests.txt"
run
expect stale-no-tests-entry 1 "gone.rs: listed in scripts/no-inline-tests.txt but not in the repository"

repo
lines_open a.rs 3000
run
expect no-trailing-newline-at-cap 0

repo
lines_open a.rs 3001
run
expect no-trailing-newline-over 1 "a.rs: 3001 lines, over the 3000-line cap"

repo
lines a.rs 10
git -C "$tmp" add -A . > /dev/null
echo "target/" > "$tmp/.gitignore"
git -C "$tmp" add .gitignore > /dev/null
lines target/x.rs 5000
seq 1 5000 > "$tmp/scratch.rs"
err="$("$check" --root "$tmp" 2>&1 > /dev/null)"
status=$?
expect untracked-ignored 0

repo
{ seq 1 6; echo "    #[test]"; echo "    fn t() {}"; } > "$tmp/lib.rs"
echo "lib.rs" > "$tmp/scripts/no-inline-tests.txt"
run
expect inline-test 1 "lib.rs:7: inline test; tests go in their own file"

repo
{ echo "// see #[test] below"; echo 'let s = "#[test]";'; } > "$tmp/lib.rs"
echo "lib.rs" > "$tmp/scripts/no-inline-tests.txt"
run
expect test-in-comment 0

repo
{ echo "fn a() {}"; echo "#[tokio::test]"; echo "async fn t() {}"; } > "$tmp/lib.rs"
echo "lib.rs" > "$tmp/scripts/no-inline-tests.txt"
run
expect path-test 1 "lib.rs:2: inline test"

repo
{ echo "#[test]"; echo "fn t() {}"; } > "$tmp/other.rs"
lines lib.rs 10
echo "lib.rs" > "$tmp/scripts/no-inline-tests.txt"
run
expect tests-allowed-elsewhere 0

repo
lines big.rs 3400
lines lib.rs 10
printf '# The limits.\n\nbig.rs 3500\n' > "$tmp/scripts/file-size-limits.txt"
printf '# No tests here.\n\nlib.rs\n' > "$tmp/scripts/no-inline-tests.txt"
run
expect comments-and-blanks 0

repo
lines "my dir/a.rs" 3001
run
expect path-with-space 1 "my dir/a.rs: 3001 lines, over the 3000-line cap"

repo
lines a.rs 3001
lines b.rs 3002
run
expect reports-every-violation 1 "b.rs: 3002 lines"

repo
lines a.rs 3001
run
expect points-at-the-docs 1 "DEVELOPMENT.md"

repo
lines big.rs 5000
echo "big.rs" > "$tmp/scripts/file-size-limits.txt"
run
expect limit-without-a-number 1 "scripts/file-size-limits.txt: bad entry 'big.rs'"

repo
lines big.rs 3501
printf '# The limits.\r\nbig.rs 3500\r\n' > "$tmp/scripts/file-size-limits.txt"
run
expect crlf-limits-file 1 "big.rs: 3501 lines, over its limit of 3500"

repo
{ seq 1 6; echo "    #[test]"; } > "$tmp/lib.rs"
printf 'lib.rs\r\n' > "$tmp/scripts/no-inline-tests.txt"
run
expect crlf-no-tests-file 1 "lib.rs:7: inline test"

repo
seq 1 3001 | perl -pe 's/\n/\r\n/' > "$tmp/a.rs"
run
expect crlf-source-over 1 "a.rs: 3001 lines, over the 3000-line cap"

repo
{ printf 'const Z: &str = "\0";\n'; echo "#[test]"; echo "fn t() {}"; } > "$tmp/lib.rs"
echo "lib.rs" > "$tmp/scripts/no-inline-tests.txt"
run
expect nul-byte-in-source 1 "lib.rs:2: inline test"

exit "$failed"

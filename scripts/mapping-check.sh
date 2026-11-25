#!/bin/sh
# Check a plug-in mapping against its plug-in's parameter dump, without the plug-in, and
# render each of its pages (knobs at their defaults, at 0 and at 1; the default and LCD
# themes) to PNGs to look at.
#
#   scripts/mapping-check.sh <mapping.lua> <dump.json> [out folder]
#
# Make the dump with `cargo run -p deckard-plugins --bin plugin-probe -- dump <plugin>
# --out <dump.json>`. The problems it lists: parameters the plug-in doesn't have,
# choices its text never shows, choice maps naming a choice that isn't one, and picture
# inputs that need real units but have neither a taper nor Hz, ms or dB in their text.
set -e
if [ $# -lt 2 ]; then
  sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
fi
abs() { (cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd)" "$(basename "$1")"); }
DECKARD_MAPPING=$(abs "$1")
DECKARD_DUMP=$(abs "$2")
DECKARD_RENDER_DIR=${3:-${TMPDIR:-/tmp}/deckard-mapping-check}
mkdir -p "$DECKARD_RENDER_DIR"
DECKARD_RENDER_DIR=$(cd "$DECKARD_RENDER_DIR" && pwd)
export DECKARD_MAPPING DECKARD_DUMP DECKARD_RENDER_DIR
cd "$(dirname "$0")/.."
cargo test -p deckard-surface --lib check_a_mapping -- --ignored --nocapture

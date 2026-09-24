#!/usr/bin/env bash
# The one shot the harness cannot take: the Omarchy bar with the widget's
# popup open. Writes a demo feed seeded at the *real* now in the system zone
# (the popup buckets against the machine clock), to target/showcase/bar.
# Putting it where the bar reads it is done by hand, with OmaCal quit — the
# running app rewrites that file.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="$PWD/target/showcase/bar"
NOW=$(date +%s%3N)
OMACAL_SHOWCASE_OUT="$OUT" OMACAL_SHOWCASE_NOW="$NOW" \
  cargo test -p omacal --lib showcase_payloads -- --ignored --quiet >/dev/null
echo "$OUT/upcoming.json"

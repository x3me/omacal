#!/usr/bin/env bash
# Writes the payloads `shots.spec.ts` renders: the demo seed at a fixed
# moment, answered by the real read commands, plus real Omarchy palettes.
# The zone is fixed here because the seed and the display zone both follow
# the system's. Output: target/showcase/*.json (override with $1).
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${1:-$PWD/target/showcase}"
NOW="${SHOWCASE_NOW:-1791970800000}" # Wed 2026-10-14 11:40, Europe/Berlin
OMACAL_SHOWCASE_OUT="$OUT" OMACAL_SHOWCASE_NOW="$NOW" TZ=Europe/Berlin \
OMACAL_SHOWCASE_THEME_ROOT="${OMARCHY_THEMES:-$HOME/.local/share/omarchy/themes}" \
OMACAL_SHOWCASE_THEMES="${SHOWCASE_THEMES:-tokyo-night,catppuccin,gruvbox,rose-pine,everforest,catppuccin-latte}" \
  cargo test -p omacal --lib showcase_payloads -- --ignored --quiet
echo "payloads in $OUT"

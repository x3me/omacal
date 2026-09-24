#!/usr/bin/env bash
# Turns target/showcase/shots/*.png into the website's images: WebP, plus the
# theme strip (one week cut into four slices, each in a different Omarchy
# theme). Output: target/showcase/site (override with $1).
set -euo pipefail
cd "$(dirname "$0")/../.."
SHOTS=target/showcase/shots
OUT="${1:-target/showcase/site}"
mkdir -p "$OUT"
for name in week tasks month bigyear popover weather quickadd; do
  magick "$SHOTS/$name.png" -quality 88 "$OUT/omacal-$name.webp"
done
parts=()
i=0
for theme in tokyo-night gruvbox catppuccin-latte everforest; do
  magick "$SHOTS/theme-$theme.png" -crop 480x760+$((i * 480))+0 +repage "$SHOTS/slice-$i.png"
  parts+=("$SHOTS/slice-$i.png")
  i=$((i + 1))
done
magick "${parts[@]}" +append -quality 88 "$OUT/omacal-themes.webp"
ls -la "$OUT"

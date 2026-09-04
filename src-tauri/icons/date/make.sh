#!/usr/bin/env bash
# Renders the tray's date icons: one 128px PNG per day of the month, drawn in
# the mark's own periwinkle so the tray still reads as OmaCal's.
#
# The tray host draws icons and nothing else — no text beside them, on Linux —
# so a date in the tray has to *be* the icon. Rendering all thirty-one once and
# committing them keeps the app free of a font rasterizer and of any runtime
# text shaping, and keeps the glyphs identical on every machine (asked for
# 2026-09-04).
#
# Re-run after changing the design; commit what it writes.
#   ./make.sh
set -euo pipefail
cd "$(dirname "$0")"

command -v rsvg-convert >/dev/null || { echo "rsvg-convert is required (librsvg)" >&2; exit 1; }

for day in $(seq 1 31); do
  # ~75px of cap height on a 128px canvas: as large as two digits fit, which is
  # what makes the number legible when the bar scales it to ~22px.
  cat > /tmp/omacal-date-$$.svg <<SVG
<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <text x="64" y="62" fill="#8DA9F0"
        font-family="DejaVu Sans, Liberation Sans, Arial, sans-serif"
        font-size="104" font-weight="700" letter-spacing="-5"
        text-anchor="middle" dominant-baseline="central">$day</text>
</svg>
SVG
  out=$(printf 'date-%02d.png' "$day")
  rsvg-convert -w 128 -h 128 /tmp/omacal-date-$$.svg -o "$out"
  rm -f /tmp/omacal-date-$$.svg
done
echo "wrote date-01.png … date-31.png"

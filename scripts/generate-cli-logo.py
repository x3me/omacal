#!/usr/bin/env python3
"""The CLI's logo, generated — never hand-edited.

Draws the OmaCal mark and the block-letter wordmark straight onto a pixel
grid sized for a terminal, and writes it as truecolor half-block ANSI art to
src-tauri/src/cli_logo.ans, which cli.rs embeds at compile time.

    python3 scripts/generate-cli-logo.py

Drawn, not resampled: the first version scaled the 512px icon down to a
36-cell grid, and every rounded edge became a smear of in-between colours
while the navy tile behind the bars was invisible on a dark terminal except
for its muddy rim (Plamen, 2026-09-05: "honestly a bit ugly"). Here every
pixel is either the mark's periwinkle, the dot's orange, or nothing — and
"nothing" is a space, so the terminal's own background shows through
whatever colour it is. No background escape codes exist in the output.

Each terminal cell carries two pixel rows: `█` when both are painted, `▀`
or `▄` when one is, in the pixel's own colour as the foreground.
"""

PERIWINKLE = (141, 169, 240)
ORANGE = (249, 115, 22)
PAD = "  "  # left margin, so the logo does not hug the terminal edge
OUT = "src-tauri/src/cli_logo.ans"

# ---- the mark, 30 px wide × 20 px tall (10 text rows) -------------------
# Proportions from the icon (bars of 48 on a 512 tile, the middle one
# starting a quarter of the way in, the bottom one two thirds long, the dot
# ahead of the middle bar), snapped to whole pixels so every edge is crisp.
W, H = 30, 20
mark = [[None] * W for _ in range(H)]


def pill(x0, x1, y0, colour):
    """A 4px-tall bar from x0 to x1 inclusive, rows y0..y0+3, with rounded
    ends: the top and bottom rows step in one pixel."""
    for y in range(y0, y0 + 4):
        inset = 1 if y in (y0, y0 + 3) else 0
        for x in range(x0 + inset, x1 - inset + 1):
            mark[y][x] = colour


def dot(x0, y0, colour):
    """A 4×4 disc: the square minus its corners."""
    for dy in range(4):
        for dx in range(4):
            if (dx in (0, 3)) and (dy in (0, 3)):
                continue
            mark[y0 + dy][x0 + dx] = colour


pill(6, 29, 1, PERIWINKLE)   # top bar, full width
dot(0, 8, ORANGE)            # the dot, ahead of the middle bar
pill(12, 29, 8, PERIWINKLE)  # middle bar, starting a quarter in
pill(6, 21, 15, PERIWINKLE)  # bottom bar, two thirds long

# ---- the wordmark, block letters, 5 text rows ---------------------------
WORD = [
    " ███   █   █   ███    ████   ███   █    ",
    "█   █  ██ ██  █   █  █      █   █  █    ",
    "█   █  █ █ █  █████  █      █████  █    ",
    "█   █  █   █  █   █  █      █   █  █    ",
    " ███   █   █  █   █   ████  █   █  █████",
]
GAP = "   "
WORD_ROW = 2  # first text row the letters sit on; centred against the bars


def sgr(c):
    return f"\x1b[38;2;{c[0]};{c[1]};{c[2]}m"


RESET = "\x1b[0m"


def encode(cells):
    """Runs of same-coloured cells share one escape; spaces carry none."""
    out, cur = [], None
    for ch, colour in cells:
        if ch == " ":
            if cur is not None:
                out.append(RESET)
                cur = None
            out.append(" ")
            continue
        if colour != cur:
            if cur is not None:
                out.append(RESET)
            out.append(sgr(colour))
            cur = colour
        out.append(ch)
    if cur is not None:
        out.append(RESET)
    return "".join(out).rstrip()


rows = []
for row in range(H // 2):
    cells = []
    for x in range(W):
        top, bot = mark[2 * row][x], mark[2 * row + 1][x]
        if top and bot:
            cells.append(("█", top))
        elif top:
            cells.append(("▀", top))
        elif bot:
            cells.append(("▄", bot))
        else:
            cells.append((" ", None))
    letters = WORD[row - WORD_ROW] if 0 <= row - WORD_ROW < len(WORD) else ""
    if letters:
        cells += [(" ", None)] * len(GAP)
        cells += [((ch if ch == "█" else " "), PERIWINKLE) for ch in letters]
    rows.append(PAD + encode(cells))

with open(OUT, "w", encoding="utf-8") as f:
    f.write("\n".join(rows) + "\n")
print(f"wrote {OUT}: {len(rows)} rows")

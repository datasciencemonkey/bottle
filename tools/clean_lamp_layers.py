#!/usr/bin/env python3
"""Remove the baked gold spark from the Genie layers, so the MOG view can draw and orbit its own (#lamp-alive).

`tools/render_mog_fallback.py` renders the `Sparkle` node of scene.rml into every
`mog-rive/fallback/layers/genie-<mood>.png`. The lamp view (src/views/mog/lamp-mascot.ts) now
draws that spark as a separate element that orbits the head, so the baked one must go.

This script clears the gold pixels of the baked spark (and its soft glow) in a disc round the
known spark centre of each pose: the gold and white core, and every soft (not opaque) glow pixel.
Opaque pink, red and dark art pixels stay. It is
idempotent: a second run changes nothing. It writes `layers/SPARK-CLEAN.sha256`, the SHA-256 of
this script and of each cleaned layer, so `tests/mog-spark-clean.test.ts` can tell a layer that
was edited by hand, or re-rendered without this step. `render_mog_fallback.py` calls it last.

Run:  uv run --no-project --with pillow --with numpy python tools/clean_lamp_layers.py
"""
import hashlib
import os

import numpy as np
from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LAYERS = os.path.join(ROOT, "mog-rive", "fallback", "layers")
MANIFEST = os.path.join(LAYERS, "SPARK-CLEAN.sha256")

# The spark centre (art px, 400 px square) of each pose. The spark is always on the centre line.
# Measured as the alpha-weighted centroid of the gold pixels above the head.
SPARK_CENTRE = {
    "idle": (200, 146),
    "listening": (200, 89),
    "processing": (200, 115),
    "answering": (200, 27),
    "success": (200, 29),
    "error": (200, 106),
    "sleeping": (200, 238),
}
# Standing poses: the spark sits on the antenna. Rows from here down are the antenna, never cleaned.
KEEP_BELOW = {"answering": 40, "success": 40}
RADIUS = {"sleeping": 34}  # the Zz sits close to the dim spark
DEFAULT_RADIUS = 46
# The faint tail of the glow (alpha up to TAIL_ALPHA) reaches further out and would show as a dim ring.
TAIL_RADIUS = 58
TAIL_ALPHA = 24


def clean(path, cx, cy, keep_below, radius):
    img = Image.open(path).convert("RGBA")
    a = np.array(img).astype(int)
    yy, xx = np.mgrid[0 : a.shape[0], 0 : a.shape[1]]
    disc = (xx - cx) ** 2 + (yy - cy) ** 2 <= radius**2
    r, g, b, al = a[..., 0], a[..., 1], a[..., 2], a[..., 3]
    gold = (g - b >= 35) & (r > 150)
    white_core = (r > 240) & (g > 230) & (b > 180)
    # The glow is soft (alpha below 250); the solid art, pink head and red body, is opaque.
    soft = al < 250
    tail = ((xx - cx) ** 2 + (yy - cy) ** 2 <= TAIL_RADIUS**2) & (al <= TAIL_ALPHA)
    kill = (yy < keep_below) & ((disc & (gold | white_core | soft)) | tail)
    a[kill, 3] = 0
    a[kill, 0:3] = 0
    Image.fromarray(a.astype("uint8"), "RGBA").save(path, optimize=True)


def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def main():
    for mood, (cx, cy) in SPARK_CENTRE.items():
        path = os.path.join(LAYERS, f"genie-{mood}.png")
        clean(path, cx, cy, KEEP_BELOW.get(mood, 10_000), RADIUS.get(mood, DEFAULT_RADIUS))
    lines = [f"{sha(os.path.abspath(__file__))}  tools/clean_lamp_layers.py"]
    for mood in sorted(SPARK_CENTRE):
        lines.append(f"{sha(os.path.join(LAYERS, f'genie-{mood}.png'))}  genie-{mood}.png")
    with open(MANIFEST, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print("cleaned", len(SPARK_CENTRE), "layers; wrote", MANIFEST)


if __name__ == "__main__":
    main()

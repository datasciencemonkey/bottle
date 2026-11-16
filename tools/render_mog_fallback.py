#!/usr/bin/env python3
"""Render the transparent PNG fallback of the MOG view (UX-09b, UX-03, ADR 0027).

Writes, under mog-rive/fallback/:
  mood-<mood>.png   one frame per mood (7), with a transparent background
  attention.png     only the "?" cue (UX-03), transparent everywhere else, drawn over any mood

The Rive CLI paints its screenshots over a dark clear colour, so this script renders each frame
twice: once over a black and once over a white backdrop rectangle (added to a temporary copy of
the project, never to mog-rive/). Difference matting gives the exact alpha:
  alpha = 1 - (white - black) / 255,  colour = black / alpha.
The cue is the difference between an attention frame and the same frame with the `Attention`
node moved off the artboard, so the cue file holds only the "?" bubble and none of the body.

Run:  uv run --no-project --with pillow python tools/render_mog_fallback.py
It also writes fallback/SOURCE.sha256, the SHA-256 of scene.rml, so `tools/ci_checks.py fallback`
can tell when scene.rml changed after the PNGs were rendered (UX-09b). CI has no Rive CLI.
Needs the Rive CLI (`rive` on PATH, or ~/.rive/bin/rive). No `rive login` is needed.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
PROJECT = os.path.join(ROOT, "mog-rive")
OUT = os.path.join(PROJECT, "fallback")
MOODS = ["idle", "listening", "processing", "answering", "success", "error", "sleeping"]
SIZE = 400
# A pixel belongs to the "?" cue when the frame with the cue differs from the frame without it
# by more than this (sum of RGBA channel differences, 0..1020).
CUE_DIFF = 24

BACKDROP = """        <Shape x="200" y="200" name="FallbackBackdrop">
            <Rectangle width="4000" height="4000" originX="0.5" originY="0.5" name="Path"/>
            <Fill name="Fill">
                <SolidColor colorValue="{argb}" name="Color"/>
            </Fill>
        </Shape>
"""


def rive_bin():
    found = shutil.which("rive")
    if found:
        return found
    home = os.path.expanduser("~/.rive/bin/rive")
    if os.path.exists(home):
        return home
    sys.exit("rive CLI not found: install it or put it on PATH")


def frames():
    """(name, commands) per frame. Each frame runs alone, so the float and blink layers are at
    the same time in the black and the white run, and in the attention and the plain frame."""
    out = []
    for m in MOODS:
        level = 0.6 if m == "listening" else 0
        out.append((f"mood-{m}", [f"data level={level}", f"data mood={m}", "advance 1.7s"]))
    out.append(("cue", ["data mood=idle", "data attention=true", "advance 1.7s"]))
    return out


ATTENTION = '<Node x="118" y="-58" opacity="0" name="Attention" id="0:71">'


def project_with_backdrop(tmp, argb, cue=True):
    """A temporary copy of mog-rive/ with a full-size rectangle behind the art (last sibling).
    With `cue=False` the `Attention` node sits off the artboard (its animations key only `y`
    and `opacity`, so `x` stays put)."""
    dst = os.path.join(tmp, f"mog-rive-{argb}-{int(cue)}")
    shutil.copytree(PROJECT, dst, ignore=shutil.ignore_patterns("build", "shots", "fallback"))
    scene = os.path.join(dst, "scene.rml")
    with open(scene, encoding="utf-8") as f:
        text = f.read()
    marker = "    </Artboard>"
    if text.count(marker) != 1:
        sys.exit("scene.rml: expected one </Artboard>")
    text = text.replace(marker, BACKDROP.format(argb=argb) + marker)
    if not cue:
        if text.count(ATTENTION) != 1:
            sys.exit("scene.rml: expected one Attention node at x=118")
        text = text.replace(ATTENTION, ATTENTION.replace('x="118"', 'x="5000"'))
    with open(scene, "w", encoding="utf-8") as f:
        f.write(text)
    return dst


def render(project, tmp, tag):
    paths = {}
    for name, cmds in frames():
        path = os.path.join(tmp, f"{name}-{tag}.png")
        lines = [f"viewport {SIZE}x{SIZE}", *cmds, f"screenshot {path}", "quit"]
        proc = subprocess.run([rive_bin(), project, "--commands"], input="\n".join(lines) + "\n",
                              capture_output=True, text=True, cwd=project)
        if proc.returncode != 0 or any(ln.startswith("err") for ln in proc.stdout.splitlines()):
            sys.stderr.write(proc.stdout + proc.stderr)
            sys.exit(f"rive --commands failed for {name}")
        paths[name] = path
    return paths


def matte(black_path, white_path):
    from PIL import Image

    b = Image.open(black_path).convert("RGB")
    w = Image.open(white_path).convert("RGB")
    out = Image.new("RGBA", b.size)
    bp, wp, op = b.load(), w.load(), out.load()
    for y in range(b.size[1]):
        for x in range(b.size[0]):
            cb, cw = bp[x, y], wp[x, y]
            a = 255 - round(sum(cw[i] - cb[i] for i in range(3)) / 3)
            a = max(0, min(255, a))
            if a == 0:
                op[x, y] = (0, 0, 0, 0)
            else:
                op[x, y] = tuple(min(255, round(cb[i] * 255 / a)) for i in range(3)) + (a,)
    return out


def cue_only(on, off):
    """The pixels of `on` that differ from `off`; transparent elsewhere."""
    from PIL import Image

    out = Image.new("RGBA", on.size)
    onp, offp, op = on.load(), off.load(), out.load()
    for y in range(on.size[1]):
        for x in range(on.size[0]):
            p, q = onp[x, y], offp[x, y]
            if sum(abs(p[i] - q[i]) for i in range(4)) > CUE_DIFF:
                op[x, y] = p
    return out


def write_source_hash(project=PROJECT):
    """Record the SHA-256 of scene.rml (line endings as LF) next to the PNGs."""
    with open(os.path.join(project, "scene.rml"), "rb") as f:
        digest = hashlib.sha256(f.read().replace(b"\r\n", b"\n")).hexdigest()
    with open(os.path.join(project, "fallback", "SOURCE.sha256"), "w", encoding="utf-8") as f:
        f.write(digest + "\n")


def main():
    os.makedirs(OUT, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mog-fallback-") as tmp:
        black = render(project_with_backdrop(tmp, "FF000000"), tmp, "black")
        white = render(project_with_backdrop(tmp, "FFFFFFFF"), tmp, "white")
        for m in MOODS:
            matte(black[f"mood-{m}"], white[f"mood-{m}"]).save(
                os.path.join(OUT, f"mood-{m}.png"), optimize=True)
        black_nocue = render(project_with_backdrop(tmp, "FF000000", cue=False), tmp, "black-nocue")
        white_nocue = render(project_with_backdrop(tmp, "FFFFFFFF", cue=False), tmp, "white-nocue")
        cue = cue_only(matte(black["cue"], white["cue"]), matte(black_nocue["cue"], white_nocue["cue"]))
        cue.save(os.path.join(OUT, "attention.png"), optimize=True)
    write_source_hash()
    print("wrote", OUT)


if __name__ == "__main__":
    main()

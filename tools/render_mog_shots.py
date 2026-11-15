#!/usr/bin/env python3
"""Render the MOG mascot shots with the Rive CLI.

Writes, under mog-rive/shots/:
  mood-<mood>.png         one frame per mood (7), the static PNG fallback (UX-09b)
  overlay-attention.png   idle + attention ("?" bubble, UX-03)
  overlay-recording.png   idle + recording (ring and REC dot, UX-04a)
  sheet.png               1600 x 800 contact sheet: 7 moods + both overlays

Run:  uv run --with pillow python tools/render_mog_shots.py
Needs the Rive CLI (`rive` on PATH, or ~/.rive/bin/rive).
"""
import os
import shutil
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
PROJECT = os.path.join(ROOT, "mog-rive")
SHOTS = os.path.join(PROJECT, "shots")
MOODS = ["idle", "listening", "processing", "answering", "success", "error", "sleeping"]
OVERLAYS = ["attention", "recording"]


def rive_bin():
    found = shutil.which("rive")
    if found:
        return found
    home = os.path.expanduser("~/.rive/bin/rive")
    if os.path.exists(home):
        return home
    sys.exit("rive CLI not found: install it or put it on PATH")


def commands():
    """One headless run: set the view model, let the state machine settle, capture."""
    lines = ["viewport 400x400", "advance 0.5s"]
    for m in MOODS:
        # level drives the listening ring; 0.6 shows it at a typical speaking level.
        lines += [f"data level={0.6 if m == 'listening' else 0}", f"data mood={m}",
                  "advance 1.2s", f"screenshot {os.path.join(SHOTS, f'mood-{m}.png')}"]
    lines += ["data level=0", "data mood=idle", "advance 1s"]
    for o in OVERLAYS:
        lines += [f"data {o}=true", "advance 0.6s",
                  f"screenshot {os.path.join(SHOTS, f'overlay-{o}.png')}", f"data {o}=false", "advance 0.4s"]
    lines.append("quit")
    return "\n".join(lines) + "\n"


def run():
    os.makedirs(SHOTS, exist_ok=True)
    proc = subprocess.run([rive_bin(), PROJECT, "--commands"], input=commands(),
                          capture_output=True, text=True, cwd=PROJECT)
    errors = [ln for ln in proc.stdout.splitlines() if ln.startswith("err")]
    if proc.returncode != 0 or errors:
        sys.stderr.write(proc.stdout + proc.stderr)
        sys.exit("rive --commands failed")


def sheet():
    from PIL import Image

    bg = (30, 30, 30, 255)
    out = Image.new("RGBA", (1600, 800), bg)
    for i, m in enumerate(MOODS):
        img = Image.open(os.path.join(SHOTS, f"mood-{m}.png")).convert("RGBA").resize((400, 400))
        out.alpha_composite(img, ((i % 4) * 400, (i // 4) * 400))
    # Last cell: the two overlays side by side at half size.
    cell = Image.new("RGBA", (400, 400), bg)
    for j, o in enumerate(OVERLAYS):
        img = Image.open(os.path.join(SHOTS, f"overlay-{o}.png")).convert("RGBA").resize((200, 200))
        cell.alpha_composite(img, (j * 200, 100))
    out.alpha_composite(cell, (1200, 400))
    out.convert("RGB").save(os.path.join(SHOTS, "sheet.png"))


if __name__ == "__main__":
    run()
    sheet()
    print("wrote", SHOTS)

#!/usr/bin/env python3
"""Shots of the Genie workspace-error card and the no-MOG hub notice (fake host, no live calls).

It starts the Vite dev server of this worktree on its own port, opens `card.html` and `hub.html`
in headless Chromium (Playwright) with the Tauri bridge stub of `lamp_alive_shots.py`, and pushes
mocked `card:update` and `hub:mogs` events. Nothing here touches the real desktop or the app.

Writes under docs/shots/genie-errors/ (device scale 2):
  card-workspace-error.png   the failure card with Try again and the "Change workspace…" link
  hub-no-mog-notice.png      the hub on the MOGs area with "Create a MOG to start"

Run:  SHOT_PORT=5391 uv run --no-project --with playwright --with pillow python tools/genie_errors_shots.py
"""
import os
import socket
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import lamp_alive_shots as L  # noqa: E402
from playwright.sync_api import sync_playwright  # noqa: E402

ROOT = L.ROOT
OUT = os.path.join(ROOT, "docs", "shots", "genie-errors")
L.PORT = int(os.environ.get("SHOT_PORT", "5391"))
G = "genie:00000000-0000-4000-8000-00000000c573"
FAILED = {
    "mogId": L.MOG_ID,
    "blurbs": [
        {
            "id": G,
            "text": "logfood (logfood.cloud.databricks.com) didn't answer. Are you on the VPN?",
            "pinned": False,
            "completed": True,
        },
        {"id": "ask:u1", "text": '"What tables can I see?"', "pinned": False, "completed": True},
    ],
    "chips": [
        {"id": f"retry:{G}", "label": "Try again"},
        {"id": f"workspace:{G}", "label": "Change workspace…"},
    ],
}


def card_shot(browser):
    init = (
        f"window.__LABEL__ = 'card-{L.MOG_ID}'; window.__TITLE__ = 'Bottle card (right)';"
        f"window.__POS__ = {{}}; window.__SIZE__ = {{'card-{L.MOG_ID}': {{width: 340, height: 64}}}};"
    )
    ctx = browser.new_context(viewport={"width": 340, "height": 64}, device_scale_factor=2)
    ctx.add_init_script(init + L.STUB)
    p = ctx.new_page()
    p.on("pageerror", lambda e: print("pageerror:", e, file=sys.stderr))
    p.goto(f"http://localhost:{L.PORT}/card.html")
    p.wait_for_selector("#root", state="attached")
    L.emit(p, "mog:target", {"mogId": L.MOG_ID, "name": "Log-mog", "target": True, "toast": False, "colorVariant": "ember"})
    L.emit(p, "card:update", FAILED)
    p.wait_for_timeout(1200)
    size = p.evaluate("window.__SIZE__['card-%s']" % L.MOG_ID)
    p.set_viewport_size({"width": int(size["width"]), "height": int(size["height"])})
    p.wait_for_timeout(300)
    p.screenshot(path=os.path.join(OUT, "card-workspace-error.png"), omit_background=True)
    print("card window", size)
    ctx.close()


def hub_shot(browser):
    stub = L.STUB.replace("window.__LABEL__", "'hub'")
    ctx = browser.new_context(viewport={"width": 960, "height": 640}, device_scale_factor=2)
    ctx.add_init_script(stub)
    p = ctx.new_page()
    p.on("pageerror", lambda e: print("pageerror:", e, file=sys.stderr))
    p.goto(f"http://localhost:{L.PORT}/hub.html")
    p.wait_for_selector("#root *", state="attached", timeout=15000)
    p.wait_for_timeout(800)
    L.emit(p, "hub:notice", {"version": 1, "accepted": True})
    p.wait_for_timeout(500)
    # The hub opens on Conversations; the press pushes a list with the notice.
    L.emit(p, "hub:mogs", {"mogs": [], "unreadable": [], "runningCount": 0, "maxRunning": 3,
                           "notice": {"kind": "info", "text": "Create a MOG to start"}})
    p.wait_for_selector("[data-hub-view='mogs']", state="visible", timeout=10000)
    p.wait_for_timeout(500)
    p.screenshot(path=os.path.join(OUT, "hub-no-mog-notice.png"))
    ctx.close()


def main():
    os.makedirs(OUT, exist_ok=True)
    with socket.socket() as sk:
        if sk.connect_ex(("127.0.0.1", L.PORT)) == 0:
            sys.exit(f"port {L.PORT} is in use: set SHOT_PORT to a free port")
    vite = subprocess.Popen(
        ["node", os.path.join(ROOT, "node_modules", "vite", "bin", "vite.js"), "--port", str(L.PORT), "--strictPort", "--host", "127.0.0.1"],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        L.wait_http(f"http://localhost:{L.PORT}/card.html")
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            card_shot(browser)
            hub_shot(browser)
            browser.close()
    finally:
        vite.terminate()


if __name__ == "__main__":
    main()

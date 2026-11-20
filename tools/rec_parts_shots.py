#!/usr/bin/env python3
"""Shots of the hub Recordings list for upload in parts (ADR follow-up to 0103).

It starts the Vite dev server of this worktree on its own port, opens `hub.html` in headless
Chromium (Playwright, its own browser) with a stub of the Tauri bridge, opens the Recordings
area and pushes mocked `hub:recordings` events. Fake rows only. Nothing here touches the real
desktop, the app, a Databricks workspace or any real recording.

Writes under docs/shots/rec-parts/ at 960 x 640:
  list-statuses.png   every status in one list
  device-only.png     a recording left on the device by an old build, with Upload
  waiting-network.png a call that waits for the network

Run:  uv run --no-project --with playwright python tools/rec_parts_shots.py
"""
import json
import os
import subprocess
import sys
import time
import urllib.request

from playwright.sync_api import sync_playwright

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
OUT = os.path.join(ROOT, "docs", "shots", "rec-parts")
PORT = int(os.environ.get("SHOT_PORT", "5293"))

STUB = """
(() => {
  const label = 'hub';
  let cb = 1, ev = 1;
  const listeners = {};
  window.__calls = [];
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label }, currentWebview: { label, windowLabel: label }, windows: [{label}], webviews: [{label, windowLabel: label}] },
    transformCallback(fn) { const id = cb++; window['_' + id] = fn; return id; },
    unregisterCallback(id) { delete window['_' + id]; },
    convertFileSrc: (p) => p,
    async invoke(cmd, args) {
      window.__calls.push(cmd);
      if (cmd === 'plugin:event|listen') {
        const id = ev++;
        (listeners[args.event] = listeners[args.event] || []).push({ id, handler: args.handler });
        return id;
      }
      if (cmd === 'plugin:event|unlisten') return null;
      return {};
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__emit = (event, payload) => {
    for (const l of listeners[event] || []) window['_' + l.handler]?.({ event, id: l.id, payload });
  };
})();
"""

NOW = int(time.time() * 1000)
MIN = 60_000


def err(text, label=None, target=None):
    e = {"text": text}
    if label:
        e["action"] = {"label": label, "target": target}
    return e


def row(i, ago_min, dur_s, status, **kw):
    return {"id": f"s{i}", "name": f"2026-11-20_{i:04d}", "startedAt": NOW - ago_min * MIN,
            "durationS": dur_s, "status": status, "canOpen": False, **kw}


ALL = [
    row(1, 1, 840, "recording", partsUploaded=2, partsTotal=3, canOpen=True),
    row(2, 25, 2700, "uploading", partsUploaded=7, partsTotal=9, canOpen=True),
    row(3, 70, 1500, "recording", waitingNetwork=True, partsUploaded=1, partsTotal=4, canOpen=True),
    row(4, 150, 1800, "transcribing", canOpen=True),
    row(5, 190, 3300, "writing_notes", canOpen=True),
    row(6, 1500, 2400, "done", canOpen=True),
    row(7, 1700, 3900, "failed", canOpen=True,
        error=err("The transcribe model isn't available. Pick another model in Settings.", "Open Settings", "settings")),
    row(8, 2900, 1200, "failed", onDeviceOnly=True,
        error=err("Bottle couldn't upload this recording. It is still on this device.", "Retry", "retry")),
]


def wait_http(url, tries=120):
    for _ in range(tries):
        try:
            urllib.request.urlopen(url, timeout=1)
            return
        except Exception:
            time.sleep(0.25)
    raise RuntimeError("vite did not start")


def emit(p, event, payload):
    p.evaluate("([e, x]) => window.__emit(e, x)", [event, payload])


def open_recordings(browser, rows, select=None):
    ctx = browser.new_context(viewport={"width": 960, "height": 640}, device_scale_factor=2)
    ctx.add_init_script(STUB)
    p = ctx.new_page()
    p.on("pageerror", lambda e: print("pageerror:", e, file=sys.stderr))
    p.goto(f"http://localhost:{PORT}/hub.html")
    p.wait_for_selector("#root *", state="attached", timeout=15000)
    p.wait_for_timeout(800)
    emit(p, "hub:notice", {"version": 1, "accepted": True})
    p.wait_for_timeout(500)
    if os.environ.get("SHOT_DEBUG"):
        p.screenshot(path="/tmp/rec-parts-debug.png")
        print(p.inner_text("body")[:600])
    p.get_by_role("button", name="Toggle the sidebar").first.click()
    p.wait_for_timeout(400)
    p.get_by_text("Recordings", exact=True).first.click()
    p.wait_for_selector("[data-hub-view='recordings']", state="visible", timeout=10000)
    emit(p, "hub:recordings", {"rows": rows})
    p.wait_for_selector("[data-recording-row]", timeout=10000)
    if select:
        p.click(f"[data-recording-row='{select}']")
    p.wait_for_timeout(500)
    return ctx, p


def main():
    os.makedirs(OUT, exist_ok=True)
    vite = subprocess.Popen(["pnpm", "exec", "vite", "--port", str(PORT), "--strictPort"], cwd=ROOT,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        wait_http(f"http://localhost:{PORT}/hub.html")
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            ctx, p = open_recordings(browser, ALL)
            p.screenshot(path=os.path.join(OUT, "list-statuses.png"))
            ctx.close()
            ctx, p = open_recordings(browser, ALL, select="s8")
            p.screenshot(path=os.path.join(OUT, "device-only.png"))
            ctx.close()
            ctx, p = open_recordings(browser, ALL, select="s3")
            p.screenshot(path=os.path.join(OUT, "waiting-network.png"))
            ctx.close()
            browser.close()
    finally:
        vite.terminate()
        try:
            vite.wait(timeout=10)
        except Exception:
            vite.kill()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Shots of the Recordings storage banner, Upload all and the file links (ADR 0103).

It starts the Vite dev server of this worktree on its own port, opens `hub.html` in headless
Chromium (Playwright, its own browser) with a stub of the Tauri bridge, opens the Recordings
area and pushes mocked `hub:recordings` events. Fake rows only. Nothing here touches the real
desktop, the app, a Databricks workspace or any real recording.

Writes under docs/shots/rec-list-links/ at 960 x 640:
  banner.png          the storage banner, the reason on each waiting row, Upload all off
  upload-all.png      Upload all running: per-row progress
  done-links.png      a done recording with its three links and the highlighted file names

Run:  uv run --no-project --with playwright python tools/rec_list_links_shots.py
"""
import json
import os
import subprocess
import sys
import time
import urllib.request

from playwright.sync_api import sync_playwright

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
OUT = os.path.join(ROOT, "docs", "shots", "rec-list-links")
PORT = int(os.environ.get("SHOT_PORT", "5311"))

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
REASON = "No My Files on this workspace. Choose a volume in Settings."


def row(i, ago_min, dur_s, status, **kw):
    return {"id": f"s{i}", "name": f"2026-11-20_{i:04d}", "startedAt": NOW - ago_min * MIN,
            "durationS": dur_s, "status": status, "canOpen": False, **kw}


def waiting(i, ago_min, **kw):
    return row(i, ago_min, 40 + i * 7, "failed", onDeviceOnly=True, needsUpload=True, **kw)


BLOCKED = [
    waiting(1, 20, error={"text": REASON, "action": {"label": "Choose where to save", "target": "settings"}}),
    waiting(2, 90, error={"text": REASON, "action": {"label": "Choose where to save", "target": "settings"}}),
    waiting(3, 400, error={"text": REASON, "action": {"label": "Choose where to save", "target": "settings"}}),
    waiting(4, 1500, error={"text": REASON, "action": {"label": "Choose where to save", "target": "settings"}}),
]
BANNER = {"rows": BLOCKED, "storageProblem": {"reason": REASON, "canUpload": False},
          "uploadAll": {"running": False, "total": 4, "done": 0, "failed": 0}}
RUNNING = {"rows": [
    waiting(1, 20, batch="queued"), waiting(2, 90, batch="queued"),
    waiting(3, 400, batch="running"), waiting(4, 1500, batch="done"),
], "uploadAll": {"running": True, "total": 4, "done": 1, "failed": 0}}
DONE = {"rows": [
    row(1, 30, 1500, "done", canOpen=True),
    row(2, 200, 2700, "done", canOpen=True),
    row(3, 1500, 900, "done", canOpen=True),
]}
DETAIL = {"id": "s1", "status": "done",
          "notesMd": "## Summary\nThe team agreed to ship on Friday.\n\n## Decisions\n- Ship Friday",
          "turns": [{"speaker": "Speaker 1", "startS": 0, "text": "Hello. Shall we ship on Friday?"},
                    {"speaker": "Speaker 2", "startS": 83, "text": "Yes, Friday works."}]}


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


def open_recordings(browser, payload, select=None, detail=None):
    ctx = browser.new_context(viewport={"width": 960, "height": 640}, device_scale_factor=2)
    ctx.add_init_script(STUB)
    p = ctx.new_page()
    p.on("pageerror", lambda e: print("pageerror:", e, file=sys.stderr))
    p.goto(f"http://localhost:{PORT}/hub.html")
    p.wait_for_selector("#root *", state="attached", timeout=15000)
    p.wait_for_timeout(800)
    emit(p, "hub:notice", {"version": 1, "accepted": True})
    p.wait_for_timeout(500)
    p.get_by_role("button", name="Toggle the sidebar").first.click()
    p.wait_for_timeout(400)
    p.get_by_text("Recordings", exact=True).first.click()
    p.wait_for_selector("[data-hub-view='recordings']", state="visible", timeout=10000)
    emit(p, "hub:recordings", payload)
    p.wait_for_selector("[data-recording-row]", timeout=10000)
    if select:
        p.click(f"[data-recording-row='{select}']")
        if detail:
            emit(p, "hub:recording.detail", detail)
    # The headless stub has no speech model: hide that onboarding step so the shot is the list.
    p.evaluate(
        "[...document.querySelectorAll('h1,h2,h3')].filter(h => h.textContent.trim() === 'Local speech')"
        ".forEach(h => (h.closest('section') || h.parentElement).remove())"
    )
    p.evaluate("document.querySelector(\"[data-hub-view='recordings']\").scrollIntoView({block: 'start'})")
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
            for name, payload, sel, det in [("banner", BANNER, None, None),
                                            ("upload-all", RUNNING, None, None),
                                            ("done-links", DONE, "s1", DETAIL)]:
                ctx, p = open_recordings(browser, payload, sel, det)
                p.screenshot(path=os.path.join(OUT, f"{name}.png"))
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

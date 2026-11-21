#!/usr/bin/env python3
"""Shot of the hub Settings "Dictation words" group (stt-vocab).

It starts the Vite dev server of this worktree on its own port, opens `hub.html` in headless
Chromium (Playwright, its own browser) with a stub of the Tauri bridge, saves a sample word list
in the page's localStorage, opens Settings and scrolls to the group. Nothing here touches the
real desktop, the app, a Databricks workspace or any model.

Writes docs/shots/stt-vocab/settings-dictation-words.png at 960 x 640.

Run:  uv run --no-project --with playwright python tools/stt_vocab_shots.py
"""
import json
import os
import subprocess
import sys
import time
import urllib.request

from playwright.sync_api import sync_playwright

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
OUT = os.path.join(ROOT, "docs", "shots", "stt-vocab")
PORT = int(os.environ.get("SHOT_PORT", "5297"))
WORDS = "unity cat log \u2192 Unity Catalog\nsathish \u2192 Sathish\nkay eight s \u2192 K8s"

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


def wait_http(url, tries=120):
    for _ in range(tries):
        try:
            urllib.request.urlopen(url, timeout=1)
            return
        except Exception:
            time.sleep(0.25)
    raise RuntimeError("vite did not start")


def main():
    os.makedirs(OUT, exist_ok=True)
    vite = subprocess.Popen(["pnpm", "exec", "vite", "--port", str(PORT), "--strictPort"], cwd=ROOT,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        wait_http(f"http://localhost:{PORT}/hub.html")
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            ctx = browser.new_context(viewport={"width": 960, "height": 640}, device_scale_factor=2)
            ctx.add_init_script(STUB)
            ctx.add_init_script("window.localStorage.setItem('bottle.dictation.words', " + json.dumps(WORDS) + ")")
            p = ctx.new_page()
            p.on("pageerror", lambda e: print("pageerror:", e, file=sys.stderr))
            p.goto(f"http://localhost:{PORT}/hub.html")
            p.wait_for_selector("#root *", state="attached", timeout=15000)
            p.wait_for_timeout(800)
            p.evaluate("([e, x]) => window.__emit(e, x)", ["hub:notice", {"version": 1, "accepted": True}])
            p.wait_for_timeout(500)
            p.get_by_role("button", name="Toggle the sidebar").first.click()
            p.wait_for_timeout(400)
            p.get_by_text("Settings", exact=True).first.click()
            p.wait_for_selector("[data-view='dictation-words']", state="visible", timeout=10000)
            p.locator("[data-view='dictation-words']").scroll_into_view_if_needed()
            p.wait_for_timeout(500)
            p.screenshot(path=os.path.join(OUT, "settings-dictation-words.png"))
            browser.close()
    finally:
        vite.terminate()
        try:
            vite.wait(timeout=10)
        except Exception:
            vite.kill()


if __name__ == "__main__":
    main()

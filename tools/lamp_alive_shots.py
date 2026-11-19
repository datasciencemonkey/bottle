#!/usr/bin/env python3
"""Shots and measures of the real MOG page for lamp-alive (spark orbit, eyes, name row, cards).

It starts the Vite dev server on its own port, opens `mog.html` (and `card.html`) in headless
Chromium (Playwright) with a stub of the Tauri bridge, and pushes mocked `mog:look`,
`mog:view_state` and `mog:target` events. Nothing here touches the real desktop or the app.

Writes under docs/shots/lamp-alive/ (the real 160 x 192 MOG window, device scale 2):
  eyes-<dir>.png     the eyes looking left, right, up and down
  eyes-sheet.png     the four, side by side
  orbit.gif          one loop of the orbiting spark (10 s sped up 4x)
  orbit-sheet.png    8 frames of the loop
  name-row.png       the MOG with the name pill under the lamp
  layout.json        the measured boxes of the art, the marker and the pill
  cpu.json           the measured idle cost (see measure_cost)

Run:  uv run --no-project --with playwright --with pillow python tools/lamp_alive_shots.py
Needs `pnpm install` once and a Chromium of Playwright (`playwright install chromium`).
"""
import json
import os
import subprocess
import sys
import time
import urllib.request

from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
OUT = os.path.join(ROOT, "docs", "shots", "lamp-alive")
PORT = int(os.environ.get("SHOT_PORT", "5287"))
MOG_ID = "00000000-0000-4000-8000-000000000001"
W, H = 160, 192

# A stub of the Tauri bridge: enough for `listen`, `emit` and `invoke` of the MOG and card views.
STUB = """
(() => {
  const label = window.__LABEL__;
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
      if (cmd === 'plugin:window|get_all_windows') return window.__WINDOWS__ || [];
      if (cmd === 'plugin:window|outer_position') return window.__POS__?.[args.label] || { x: 0, y: 0 };
      if (cmd === 'plugin:window|outer_size') return window.__SIZE__?.[args.label] || { width: 340, height: 64 };
      if (cmd === 'plugin:window|scale_factor') return 1;
      if (cmd === 'plugin:window|current_monitor') return { name: 'm', position: {x:0,y:0}, size: {width:1440,height:900}, scaleFactor: 1, workArea: { position: {x:0,y:0}, size: {width:1440,height:900} } };
      if (cmd === 'plugin:window|set_size') {
        const d = Object.values(JSON.parse(JSON.stringify(args.value)))[0];
        (window.__SIZE__ = window.__SIZE__ || {})[args.label || label] = { width: d.width, height: d.height };
        window.__set = (window.__set||[]).concat([['size', d]]);
        return null;
      }
      if (cmd === 'plugin:window|set_position') {
        const d = Object.values(JSON.parse(JSON.stringify(args.value)))[0];
        (window.__POS__ = window.__POS__ || {})[args.label || label] = { x: d.x, y: d.y };
        window.__set = (window.__set||[]).concat([['pos', d]]);
        return null;
      }
      return {};
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__emit = (event, payload) => {
    for (const l of listeners[event] || []) window['_' + l.handler]?.({ event, id: l.id, payload });
  };
})();
"""


def wait_http(url, tries=80):
    for _ in range(tries):
        try:
            urllib.request.urlopen(url, timeout=1)
            return
        except Exception:
            time.sleep(0.25)
    raise RuntimeError("vite did not start")


def page(browser, kind, label, scale=2, w=W, h=H):
    ctx = browser.new_context(viewport={"width": w, "height": h}, device_scale_factor=scale)
    ctx.add_init_script(f"window.__LABEL__ = {json.dumps(label)};" + STUB)
    p = ctx.new_page()
    p.on("pageerror", lambda e: print("pageerror:", e, file=sys.stderr))
    p.on("console", lambda m: m.type == "error" and print("console:", m.text, file=sys.stderr))
    p.goto(f"http://localhost:{PORT}/{kind}.html")
    p.wait_for_selector("[data-mascot='lamp']" if kind == "mog" else "#root", state="attached", timeout=15000)
    return ctx, p


def emit(p, event, payload):
    p.evaluate("([e, x]) => window.__emit(e, x)", [event, payload])


def mog_state(p, mood="idle", name="Log-mog"):
    ov = {"attention": False, "recording": False, "offline": False, "localOnly": False}
    emit(p, "mog:view_state", {"mogId": MOG_ID, "mood": mood, "overlays": ov})
    emit(p, "mog:target", {"mogId": MOG_ID, "name": name, "target": False, "toast": False, "colorVariant": "ember"})


def shot(p, path):
    p.screenshot(path=os.path.join(OUT, path), omit_background=True)


def shots_eyes(browser):
    ctx, p = page(browser, "mog", f"mog-{MOG_ID}")
    mog_state(p)
    p.mouse.move(80, 90)  # a pointer on the MOG so the Genie is out of the lamp
    p.wait_for_timeout(700)
    for name, (dx, dy) in {"left": (-1, 0), "right": (1, 0), "up": (0, -1), "down": (0, 1)}.items():
        emit(p, "mog:look", {"dx": dx, "dy": dy})
        p.wait_for_timeout(500)
        shot(p, f"eyes-{name}.png")
    emit(p, "mog:look", {"dx": 0, "dy": 0})
    p.wait_for_timeout(500)
    shot(p, "eyes-centre.png")
    ctx.close()
    names = ["left", "right", "up", "down"]
    sheet = Image.new("RGBA", (W * 2 * 4, H * 2), (30, 30, 34, 255))
    for i, n in enumerate(names):
        im = Image.open(os.path.join(OUT, f"eyes-{n}.png")).convert("RGBA")
        sheet.alpha_composite(im, (i * W * 2, 0))
    sheet.save(os.path.join(OUT, "eyes-sheet.png"))


def shots_orbit(browser):
    ctx, p = page(browser, "mog", f"mog-{MOG_ID}")
    mog_state(p)
    p.mouse.move(80, 90)
    p.wait_for_timeout(600)
    frames = []
    n = 40  # one loop of 10 s, a frame every 250 ms of animation time
    p.evaluate("document.getAnimations().forEach(a => a.pause())")
    for i in range(n):
        p.evaluate(
            "(t) => document.getAnimations().forEach(a => { a.currentTime = t })", i * 250.0
        )
        p.wait_for_timeout(30)
        path = os.path.join(OUT, f"_f{i:02d}.png")
        p.screenshot(path=path, omit_background=True)
        frames.append(Image.open(path).convert("RGBA"))
    ctx.close()
    bg = (34, 36, 42, 255)
    flat = []
    for f in frames:
        im = Image.new("RGBA", f.size, bg)
        im.alpha_composite(f)
        flat.append(im.convert("P", palette=Image.ADAPTIVE))
    flat[0].save(
        os.path.join(OUT, "orbit.gif"), save_all=True, append_images=flat[1:], duration=62, loop=0, optimize=True
    )
    sheet = Image.new("RGBA", (W * 2 * 4, H * 2 * 2), bg)
    for k, i in enumerate(range(0, n, 5)):
        sheet.alpha_composite(frames[i], ((k % 4) * W * 2, (k // 4) * H * 2))
    sheet.save(os.path.join(OUT, "orbit-sheet.png"))
    for i in range(n):
        os.remove(os.path.join(OUT, f"_f{i:02d}.png"))


def shots_name_row(browser):
    ctx, p = page(browser, "mog", f"mog-{MOG_ID}")
    mog_state(p, name="Log-mog")
    emit(p, "mog:target", {"mogId": MOG_ID, "name": "Log-mog", "target": True, "toast": False, "colorVariant": "ember"})
    p.mouse.move(80, 90)
    p.wait_for_timeout(700)
    shot(p, "name-row.png")
    box = lambda sel: p.evaluate(
        "(s) => { const r = document.querySelector(s).getBoundingClientRect(); return {x:r.x,y:r.y,w:r.width,h:r.height,bottom:r.bottom} }", sel
    )
    lay = {"window": {"w": W, "h": H}, "art": box(".lamp"), "mascot": box(".mascot"), "marker": box(".target-marker"), "pill": box(".identity-pill")}
    # Long names keep the pill centred, inside the window, with an ellipsis.
    emit(p, "mog:target", {"mogId": MOG_ID, "name": "A very long MOG name that cannot fit", "target": True, "toast": False, "colorVariant": "ember"})
    p.wait_for_timeout(100)
    lay["pill_long"] = box(".identity-pill")
    shot(p, "name-row-long.png")
    ctx.close()
    a, m, pill = lay["art"], lay["marker"], lay["pill"]
    lay["ok"] = {
        "pill_below_art": pill["y"] >= a["bottom"],
        "marker_below_art": m["y"] >= a["bottom"],
        "marker_above_pill": m["bottom"] <= pill["y"],
        "pill_inside": pill["bottom"] <= H and pill["x"] >= 0 and pill["x"] + pill["w"] <= W,
        "long_pill_inside": lay["pill_long"]["bottom"] <= H and lay["pill_long"]["x"] >= 0 and lay["pill_long"]["x"] + lay["pill_long"]["w"] <= W,
        "pill_centred": abs(pill["x"] + pill["w"] / 2 - W / 2) < 1,
    }
    with open(os.path.join(OUT, "layout.json"), "w") as f:
        json.dump(lay, f, indent=2)
    print("layout", json.dumps(lay["ok"]))
    assert all(lay["ok"].values()), lay


def measure_cost(browser, seconds=20):
    """The cost of the MOG page at idle, with the orbit on, hidden (paused) and with reduced motion.

    CPU is the sum of the browser, GPU and renderer processes of this headless Chromium
    (`SystemInfo.getProcessInfo`), as a share of one core. The main-thread counters come from
    `Performance.getMetrics`: if the orbit really runs on the compositor they do not grow.
    Headless Chromium paints with software GL, so the CPU is an upper bound of a Mac with a GPU.
    """
    root = browser.new_browser_cdp_session()

    def cpu():
        info = root.send("SystemInfo.getProcessInfo")["processInfo"]
        return sum(p["cpuTime"] for p in info)

    modes = {
        "orbit_on": {"reduced": False, "hidden": False},
        "orbit_paused_hidden": {"reduced": False, "hidden": True},
        "reduced_motion_static": {"reduced": True, "hidden": False},
    }
    out = {"seconds": seconds, "modes": {}}
    for name, m in modes.items():
        ctx = browser.new_context(
            viewport={"width": W, "height": H}, device_scale_factor=2,
            reduced_motion="reduce" if m["reduced"] else "no-preference",
        )
        ctx.add_init_script(f"window.__LABEL__ = 'mog-{MOG_ID}';" + STUB)
        p = ctx.new_page()
        p.goto(f"http://localhost:{PORT}/mog.html")
        p.wait_for_selector("[data-mascot='lamp']", state="attached", timeout=15000)
        mog_state(p)
        if m["hidden"]:
            emit(p, "mog:menu_hide", {"hidden": True})
        p.wait_for_timeout(3000)
        cdp = ctx.new_cdp_session(p)
        cdp.send("Performance.enable")
        met = lambda: {x["name"]: x["value"] for x in cdp.send("Performance.getMetrics")["metrics"]}
        m0, c0, t0 = met(), cpu(), time.time()
        p.wait_for_timeout(seconds * 1000)
        m1, c1, t1 = met(), cpu(), time.time()
        d = {k: round(m1[k] - m0[k], 4) for k in ("LayoutCount", "RecalcStyleCount", "ScriptDuration", "LayoutDuration", "RecalcStyleDuration", "TaskDuration")}
        running = p.evaluate("document.getAnimations().filter(a => a.playState === 'running').length")
        out["modes"][name] = {
            "cpu_percent_of_one_core": round(100 * (c1 - c0) / (t1 - t0), 2),
            "main_thread_delta": d,
            "running_animations": running,
        }
        ctx.close()
        print(name, json.dumps(out["modes"][name]))
    base = out["modes"]["reduced_motion_static"]["cpu_percent_of_one_core"]
    on = out["modes"]["orbit_on"]["cpu_percent_of_one_core"]
    out["orbit_overhead_cpu_percent_of_one_core"] = round(on - base, 2)
    with open(os.path.join(OUT, "cpu.json"), "w") as f:
        json.dump(out, f, indent=2)


CARD_ID = MOG_ID
SCREEN = (1440, 900)


def card_scene(browser, name, mog_x, payload, mood="processing"):
    """One MOG and its card, each in its own headless page, composed at their real window places."""
    mog_pos = {"x": mog_x, "y": 300}
    card_pos = {"x": mog_x + 160 - 13 + 10 - 12, "y": 342}  # where Rust places the first, 340 wide card
    init = (
        f"window.__LABEL__ = 'card-{CARD_ID}'; window.__WINDOWS__ = ['card-{CARD_ID}', 'mog-{MOG_ID}'];"
        f"window.__POS__ = {{'card-{CARD_ID}': {json.dumps(card_pos)}, 'mog-{MOG_ID}': {json.dumps(mog_pos)}}};"
        f"window.__SIZE__ = {{'card-{CARD_ID}': {{width: 340, height: 64}}, 'mog-{MOG_ID}': {{width: 160, height: {H}}}}};"
    )
    ctx = browser.new_context(viewport={"width": 340, "height": 64}, device_scale_factor=2)
    ctx.add_init_script(init + STUB.replace("const label = window.__LABEL__;", "const label = window.__LABEL__;"))
    p = ctx.new_page()
    p.on("pageerror", lambda e: print("pageerror:", e, file=sys.stderr))
    p.goto(f"http://localhost:{PORT}/card.html")
    p.wait_for_selector("#root", state="attached")
    emit(p, "mog:target", {"mogId": MOG_ID, "name": "Log-mog", "target": True, "toast": False, "colorVariant": "ember"})
    emit(p, "card:update", payload)
    p.wait_for_timeout(1200 if payload is not WORKING else 4600)  # the working pill shows its timer after 3 s
    size = p.evaluate("window.__SIZE__['card-%s']" % CARD_ID)
    pos = p.evaluate("window.__POS__['card-%s']" % CARD_ID)
    side = p.evaluate("document.documentElement.dataset.side || 'right'")
    p.set_viewport_size({"width": int(size["width"]), "height": int(size["height"])})
    p.wait_for_timeout(300)
    card_png = os.path.join(OUT, f"_card-{name}.png")
    p.screenshot(path=card_png, omit_background=True)
    ctx.close()
    # The MOG next to it.
    mctx, mp = page(browser, "mog", f"mog-{MOG_ID}")
    mog_state(mp, mood=mood)
    mp.mouse.move(80, 90)
    mp.wait_for_timeout(900)
    mog_png = os.path.join(OUT, f"_mog-{name}.png")
    mp.screenshot(path=mog_png, omit_background=True)
    mctx.close()
    # Compose on a desktop-like backdrop, around the MOG, with the screen edge marked.
    x0 = int(min(pos["x"], mog_x) - 30)
    x1 = int(max(pos["x"] + size["width"], mog_x + 160) + 30)
    y0, y1 = 280, int(max(pos["y"] + size["height"], 300 + H) + 30)
    sc = 2
    scene = Image.new("RGBA", ((x1 - x0) * sc, (y1 - y0) * sc), (58, 66, 84, 255))
    for im, (ix, iy) in [(mog_png, (mog_x, 300)), (card_png, (pos["x"], pos["y"]))]:
        scene.alpha_composite(Image.open(im).convert("RGBA"), (int(ix - x0) * sc, int(iy - y0) * sc))
        os.remove(im)
    scene.save(os.path.join(OUT, f"card-{name}.png"))
    info = {
        "mog": {"x": mog_x, "y": 300, "w": 160, "h": H},
        "card": {"x": pos["x"], "y": pos["y"], "w": size["width"], "h": size["height"]},
        "side": side,
    }
    gap = (
        pos["x"] + 12 - (mog_x + 160 - 13) if side == "right" else (mog_x + 18) - (pos["x"] + size["width"] - 12)
    )
    info["visible_gap_px"] = gap
    info["on_screen"] = pos["x"] >= 0 and pos["x"] + size["width"] <= SCREEN[0]
    print("card", name, json.dumps(info))
    return info


ANSWER_MD = "## Tables\n\nYou can see about **8,800 tables** in **150 catalogs**."
WORKING = {
    "mogId": MOG_ID,
    "blurbs": [
        {"id": "genie:00000000-0000-4000-8000-00000000c573", "text": "Asking Genie", "pinned": False, "completed": False},
        {"id": "ask:u1", "text": '"What tables can I see?"', "pinned": False, "completed": True},
    ],
    "chips": [],
}
ANSWERED = {
    "mogId": MOG_ID,
    "blurbs": [
        {"id": "genie:00000000-0000-4000-8000-00000000c573", "text": "You can see about **8,800 tables**.", "pinned": False, "completed": True},
        {"id": "ask:u1", "text": '"What tables can I see?"', "pinned": False, "completed": True},
    ],
    "chips": [],
    "sections": [
        {"id": "answer", "md": ANSWER_MD},
        {"id": "keyNumbers", "items": ["8,800 tables", "150 catalogs"]},
        {"id": "footer"},
        {"id": "dataFlow", "text": "Your question went to Genie One in your workspace."},
    ],
}


def shots_cards(browser):
    res = {}
    for side, x in (("right", 600), ("left", 1250)):
        res[f"working-{side}"] = card_scene(browser, f"working-{side}", x, WORKING)
        res[f"answer-{side}"] = card_scene(browser, f"answer-{side}", x, ANSWERED, mood="success")
    with open(os.path.join(OUT, "card-layout.json"), "w") as f:
        json.dump(res, f, indent=2)
    assert all(abs(r["visible_gap_px"] - 10) < 0.01 and r["on_screen"] for r in res.values()), res


def main():
    os.makedirs(OUT, exist_ok=True)
    import socket

    with socket.socket() as sk:
        if sk.connect_ex(("127.0.0.1", PORT)) == 0:
            sys.exit(f"port {PORT} is in use: set SHOT_PORT to a free port")
    vite = subprocess.Popen(
        ["node", os.path.join(ROOT, "node_modules", "vite", "bin", "vite.js"), "--port", str(PORT), "--strictPort", "--host", "127.0.0.1"],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        wait_http(f"http://localhost:{PORT}/mog.html")
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            what = sys.argv[1:] or ["eyes", "orbit", "name", "cards", "cost"]
            if "eyes" in what:
                shots_eyes(browser)
            if "orbit" in what:
                shots_orbit(browser)
            if "cards" in what:
                shots_cards(browser)
            if "cost" in what:
                measure_cost(browser)
            if "name" in what:
                shots_name_row(browser)
            browser.close()
    finally:
        vite.terminate()


if __name__ == "__main__":
    main()

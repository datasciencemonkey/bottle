"""[SP-01] Tests for the SP-01 measurement tool.

Run: uv run --no-project --with pytest --with pillow pytest spikes/sp-01/measure -q
"""

import json
from pathlib import Path

import pytest
from PIL import Image

import sp01_measure as m

SPIKE = Path(__file__).resolve().parent.parent


# Sharma, Wu, Dalal (2005) CIEDE2000 reference pairs (L1,a1,b1, L2,a2,b2, dE00).
SHARMA = [
    ((50.0, 2.6772, -79.7751), (50.0, 0.0, -82.7485), 2.0425),
    ((50.0, 3.1571, -77.2803), (50.0, 0.0, -82.7485), 2.8615),
    ((50.0, 2.5, 0.0), (50.0, 0.0, -2.5), 4.3065),
    ((50.0, 2.5, 0.0), (73.0, 25.0, -18.0), 27.1492),
    ((60.2574, -34.0099, 36.2677), (60.4626, -34.1751, 39.4387), 1.2644),
    ((2.0776, 0.0795, -1.1350), (0.9033, -0.0636, -0.5514), 0.9082),
]


@pytest.mark.parametrize("lab1,lab2,expected", SHARMA)
def test_sp_01_delta_e_2000_matches_sharma_reference(lab1, lab2, expected):
    """[SP-01] CIEDE2000 matches the published reference values."""
    assert m.delta_e_2000(lab1, lab2) == pytest.approx(expected, abs=1e-4)


def test_sp_01_srgb_white_and_black_to_lab():
    """[SP-01] sRGB to CIELAB (D65) end points."""
    white = m.srgb_to_lab((255, 255, 255))
    black = m.srgb_to_lab((0, 0, 0))
    assert white[0] == pytest.approx(100.0, abs=0.01)
    assert abs(white[1]) < 0.01 and abs(white[2]) < 0.01
    assert black == pytest.approx((0.0, 0.0, 0.0), abs=1e-6)


def _checker(w, h, a=(40, 40, 40), b=(200, 200, 200), cell=8):
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = a if ((x // cell + y // cell) % 2 == 0) else b
    return img


def test_sp_01_transparency_identical_images_pass():
    """[SP-01] Identical desktop and window capture give Delta E 0 (pass)."""
    ref = _checker(32, 32)
    stats = m.transparency_stats(ref, ref.copy())
    assert stats["max"] == 0.0
    assert stats["pass"] is True
    assert stats["n"] == 32 * 32


def test_sp_01_transparency_white_background_fails():
    """[SP-01] An opaque white window background fails the Delta E < 2 rule."""
    ref = _checker(16, 16)
    test = Image.new("RGB", (16, 16), (255, 255, 255))
    stats = m.transparency_stats(ref, test)
    assert stats["pass"] is False
    assert stats["n_over"] == 16 * 16


def test_sp_01_transparency_excludes_sprite_mask():
    """[SP-01] Pixels inside the sprite mask do not count against transparency."""
    ref = _checker(16, 16)
    test = ref.copy()
    for x in range(4, 8):
        for y in range(4, 8):
            test.putpixel((x, y), (255, 0, 0))
    mask = m.diff_mask(ref, test, threshold=2.0)
    assert sum(1 for v in mask.getdata() if v) == 16
    stats = m.transparency_stats(ref, test, exclude=mask)
    assert stats["pass"] is True
    assert stats["n"] == 16 * 16 - 16


def test_sp_01_halo_detects_white_fringe():
    """[SP-01] A white 1 px fringe around a dark sprite counts as halo."""
    ref = Image.new("RGB", (20, 20), (0, 0, 120))
    clean = ref.copy()
    for x in range(6, 14):
        for y in range(6, 14):
            clean.putpixel((x, y), (30, 30, 30))
    fringe = clean.copy()
    for x in range(5, 15):
        for y in (5, 14):
            fringe.putpixel((x, y), (255, 255, 255))
            fringe.putpixel((y, x), (255, 255, 255))
    assert m.halo_stats(ref, clean)["halo_px"] == 0
    assert m.halo_stats(ref, fringe)["halo_px"] > 0


def test_sp_01_white_flash_duration():
    """[SP-01] The longest run of near-white frames gives the flash time."""
    dark = Image.new("RGB", (8, 8), (10, 10, 10))
    white = Image.new("RGB", (8, 8), (255, 255, 255))
    frames = [(0, dark), (30, white), (60, white), (90, white), (120, dark)]
    assert m.white_flash_ms(frames) == pytest.approx(90.0)
    assert m.white_flash_ms([(0, dark), (30, dark)]) == 0.0


def test_sp_01_parse_hover_log_counts_by_type():
    """[SP-01] The hover log parser counts events by type and ignores junk."""
    lines = [
        json.dumps({"kind": "pointer", "type": "pointermove", "t": 1.0}),
        json.dumps({"kind": "pointer", "type": "pointermove", "t": 2.0}),
        json.dumps({"kind": "pointer", "type": "pointerenter", "t": 1.0}),
        json.dumps({"kind": "pointer", "type": "mouseover", "t": 1.0}),
        json.dumps({"kind": "info", "msg": "ready"}),
        "not json",
        "",
    ]
    counts = m.parse_hover_log(lines)
    assert counts == {"pointermove": 2, "pointerenter": 1, "mouseover": 1}


def test_sp_01_hover_summary_ratio():
    """[SP-01] Hover summary reports received/sent and pass when ratio >= 0.9."""
    s = m.hover_summary(sent=100, counts={"pointermove": 95, "pointerenter": 1})
    assert s["received_moves"] == 95
    assert s["ratio"] == pytest.approx(0.95)
    assert s["pass"] is True
    assert m.hover_summary(sent=100, counts={})["pass"] is False


def test_sp_01_nfr_14b_config_has_no_macos_private_api():
    """[SP-01][NFR-14b] The spike config does not turn on macOSPrivateApi."""
    conf = json.loads((SPIKE / "src-tauri" / "tauri.conf.json").read_text())
    assert conf.get("app", {}).get("macOSPrivateApi", False) is False


def test_sp_01_arc_3_mog_window_flags():
    """[SP-01][ARC-3] The mog window is transparent, frameless, on top, not focused."""
    conf = json.loads((SPIKE / "src-tauri" / "tauri.conf.json").read_text())
    win = conf["app"]["windows"][0]
    assert win["label"].startswith("mog")
    assert win["transparent"] is True
    assert win["decorations"] is False
    assert win["alwaysOnTop"] is True
    assert win["focus"] is False
    assert win.get("shadow") is False


def test_sp_01_arc_4_csp():
    """[SP-01][ARC-4] The CSP has no external connect-src and allows wasm."""
    conf = json.loads((SPIKE / "src-tauri" / "tauri.conf.json").read_text())
    csp = conf["app"]["security"]["csp"]
    assert "connect-src ipc: http://ipc.localhost" in csp
    assert "'wasm-unsafe-eval'" in csp
    assert "https:" not in csp


def test_sp_01_nfr_16b_no_rive_webgl_v1():
    """[SP-01][NFR-16b] The spike does not depend on @rive-app/webgl (v1)."""
    pkg = json.loads((SPIKE / "package.json").read_text())
    deps = {**pkg.get("dependencies", {}), **pkg.get("devDependencies", {})}
    assert "@rive-app/webgl" not in deps
    assert {"@rive-app/webgl2", "@rive-app/canvas"} <= set(deps)


def test_sp_01_parse_top_sums_processes_and_drops_first_sample():
    """[SP-01] CPU/RSS sampler sums app + WebKit rows per sample, skips sample 1."""
    text = "\n".join([
        "Processes: 1", "PID %CPU MEM", "10 99.0 10M", "11 99.0 10M",
        "Processes: 1", "PID %CPU MEM", "10 1.5 20M", "11 2.5 512K",
        "Processes: 1", "PID %CPU MEM", "10 3.0 1G", "11 1.0 30M",
    ])
    r = m.parse_top(text)
    assert r["samples"] == 2
    assert r["cpu_mean"] == pytest.approx((4.0 + 4.0) / 2)
    assert r["mem_mb_max"] == pytest.approx(1024 + 30)


def test_sp_01_ux_08a_hover_breakdown_splits_native_trusted_synthetic():
    """[SP-01][UX-08a] Native AppKit calls, trusted and forwarded DOM events count apart."""
    lines = [
        json.dumps({"kind": "native", "type": "pointermove"}),
        json.dumps({"kind": "native", "type": "pointerenter"}),
        json.dumps({"kind": "pointer", "type": "pointermove", "trusted": True}),
        json.dumps({"kind": "pointer", "type": "pointermove", "trusted": False}),
        json.dumps({"kind": "pointer", "type": "pointermove"}),
        json.dumps({"kind": "info", "msg": "ready"}),
    ]
    b = m.hover_breakdown(lines)
    assert b["native"] == {"pointermove": 1, "pointerenter": 1}
    assert b["page_trusted"] == {"pointermove": 2}
    assert b["page_synthetic"] == {"pointermove": 1}

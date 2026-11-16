"""[SP-01] Measurement helpers for the Rive transparent-window spike.

Pure functions (CIEDE2000, transparency, halo, white flash, hover log) plus a
small CLI used by run_measurements.sh and sign_and_rerun.sh.

Run tests: uv run --no-project --with pytest --with pillow pytest spikes/sp-01/measure -q
CLI:       uv run --no-project --with pillow python -I measure/sp01_measure.py <cmd> ...
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageFilter

DELTA_E_PASS = 2.0
FLASH_LIMIT_MS = 100.0
HOVER_PASS_RATIO = 0.9

# ---------------------------------------------------------------- colour


def _srgb_to_linear(c: float) -> float:
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


_WHITE_D65 = (0.95047, 1.0, 1.08883)


def _f(t: float) -> float:
    d = 6.0 / 29.0
    return t ** (1.0 / 3.0) if t > d**3 else t / (3 * d * d) + 4.0 / 29.0


def srgb_to_lab(rgb):
    """sRGB 8-bit triple to CIELAB (D65)."""
    r, g, b = (_srgb_to_linear(v) for v in rgb[:3])
    x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b
    y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b
    z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b
    fx, fy, fz = (_f(v / w) for v, w in zip((x, y, z), _WHITE_D65))
    return (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))


def delta_e_2000(lab1, lab2) -> float:
    """CIEDE2000 colour difference (Sharma, Wu, Dalal 2005)."""
    L1, a1, b1 = lab1
    L2, a2, b2 = lab2
    c1 = math.hypot(a1, b1)
    c2 = math.hypot(a2, b2)
    cbar7 = ((c1 + c2) / 2.0) ** 7
    g = 0.5 * (1 - math.sqrt(cbar7 / (cbar7 + 25.0**7)))
    a1p, a2p = (1 + g) * a1, (1 + g) * a2
    c1p, c2p = math.hypot(a1p, b1), math.hypot(a2p, b2)
    h1p = math.degrees(math.atan2(b1, a1p)) % 360 if c1p else 0.0
    h2p = math.degrees(math.atan2(b2, a2p)) % 360 if c2p else 0.0
    dLp = L2 - L1
    dCp = c2p - c1p
    if c1p * c2p == 0:
        dhp = 0.0
    else:
        dhp = h2p - h1p
        if dhp > 180:
            dhp -= 360
        elif dhp < -180:
            dhp += 360
    dHp = 2 * math.sqrt(c1p * c2p) * math.sin(math.radians(dhp / 2))
    Lbp = (L1 + L2) / 2
    Cbp = (c1p + c2p) / 2
    if c1p * c2p == 0:
        hbp = h1p + h2p
    elif abs(h1p - h2p) <= 180:
        hbp = (h1p + h2p) / 2
    elif h1p + h2p < 360:
        hbp = (h1p + h2p + 360) / 2
    else:
        hbp = (h1p + h2p - 360) / 2
    t = (
        1
        - 0.17 * math.cos(math.radians(hbp - 30))
        + 0.24 * math.cos(math.radians(2 * hbp))
        + 0.32 * math.cos(math.radians(3 * hbp + 6))
        - 0.20 * math.cos(math.radians(4 * hbp - 63))
    )
    d_theta = 30 * math.exp(-(((hbp - 275) / 25) ** 2))
    cbp7 = Cbp**7
    rc = 2 * math.sqrt(cbp7 / (cbp7 + 25.0**7))
    sl = 1 + (0.015 * (Lbp - 50) ** 2) / math.sqrt(20 + (Lbp - 50) ** 2)
    sc = 1 + 0.045 * Cbp
    sh = 1 + 0.015 * Cbp * t
    rt = -math.sin(math.radians(2 * d_theta)) * rc
    return math.sqrt(
        (dLp / sl) ** 2
        + (dCp / sc) ** 2
        + (dHp / sh) ** 2
        + rt * (dCp / sc) * (dHp / sh)
    )


class _LabCache(dict):
    def __missing__(self, key):
        v = srgb_to_lab(key)
        self[key] = v
        return v


_LAB = _LabCache()


def _de(p, q) -> float:
    if p[:3] == q[:3]:
        return 0.0
    return delta_e_2000(_LAB[p[:3]], _LAB[q[:3]])


# ---------------------------------------------------------- transparency


def diff_mask(ref: Image.Image, test: Image.Image, threshold: float = DELTA_E_PASS):
    """Mask ('1' mode) of pixels whose Delta E vs ref is >= threshold (the sprite)."""
    ref, test = ref.convert("RGB"), test.convert("RGB")
    mask = Image.new("1", ref.size, 0)
    rp, tp, mp = ref.load(), test.load(), mask.load()
    w, h = ref.size
    for y in range(h):
        for x in range(w):
            if _de(rp[x, y], tp[x, y]) >= threshold:
                mp[x, y] = 1
    return mask


def transparency_stats(ref, test, exclude=None, threshold: float = DELTA_E_PASS):
    """Delta E stats of test vs ref over pixels not in exclude.

    Pass when every counted pixel has Delta E < threshold.
    """
    ref, test = ref.convert("RGB"), test.convert("RGB")
    if ref.size != test.size:
        raise ValueError(f"size mismatch {ref.size} vs {test.size}")
    rp, tp = ref.load(), test.load()
    ep = exclude.load() if exclude is not None else None
    w, h = ref.size
    vals = []
    for y in range(h):
        for x in range(w):
            if ep is not None and ep[x, y]:
                continue
            vals.append(_de(rp[x, y], tp[x, y]))
    n = len(vals)
    n_over = sum(1 for v in vals if v >= threshold)
    vals.sort()
    return {
        "n": n,
        "max": max(vals) if vals else 0.0,
        "mean": (sum(vals) / n) if n else 0.0,
        "p99": vals[min(n - 1, int(n * 0.99))] if n else 0.0,
        "n_over": n_over,
        "pass": n > 0 and n_over == 0,
    }


def halo_stats(ref, test, threshold: float = DELTA_E_PASS, light_l: float = 85.0):
    """Count light fringe pixels on the 1-2 px ring just outside the sprite.

    The sprite is the set of pixels that differ from ref. A halo pixel is a ring
    pixel that differs from ref AND is light (L* >= light_l) while ref is not.
    Pixels in the ring are, by construction, the outermost differing pixels.
    """
    ref, test = ref.convert("RGB"), test.convert("RGB")
    mask = diff_mask(ref, test, threshold).convert("L")
    # Interior = mask eroded; ring = mask minus interior (outer edge of sprite).
    interior = mask.filter(ImageFilter.MinFilter(3))
    ring = ImageChops.subtract(mask, interior)
    rp, tp, gp = ref.load(), test.load(), ring.load()
    w, h = ref.size
    ring_px = halo = 0
    for y in range(h):
        for x in range(w):
            if not gp[x, y]:
                continue
            ring_px += 1
            lt = _LAB[tp[x, y][:3]][0]
            lr = _LAB[rp[x, y][:3]][0]
            if lt >= light_l and lr < light_l:
                halo += 1
    return {"ring_px": ring_px, "halo_px": halo, "pass": halo == 0}


def _is_near_white(img: Image.Image, min_l: float = 90.0, frac: float = 0.5) -> bool:
    img = img.convert("RGB")
    small = img.resize((min(32, img.width), min(32, img.height)))
    px = list(small.getdata())
    white = sum(1 for p in px if _LAB[p[:3]][0] >= min_l)
    return white >= frac * len(px)


def white_flash_ms(frames) -> float:
    """Longest run of near-white frames, in ms.

    frames: list of (t_ms, Image). A run lasts from its first white frame to the
    first non-white frame after it (or the last frame).
    """
    best = 0.0
    start = None
    for t, img in frames:
        if _is_near_white(img):
            if start is None:
                start = t
        elif start is not None:
            best = max(best, t - start)
            start = None
    if start is not None and frames:
        best = max(best, frames[-1][0] - start)
    return float(best)


# ----------------------------------------------------------------- hover


def parse_hover_log(lines):
    """Count pointer events by type from JSONL lines; ignore other lines."""
    counts: dict[str, int] = {}
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        if not isinstance(rec, dict) or rec.get("kind") != "pointer":
            continue
        typ = rec.get("type")
        if typ:
            counts[typ] = counts.get(typ, 0) + 1
    return counts


def hover_breakdown(lines) -> dict:
    """Split events into native AppKit calls, trusted DOM events and synthetic
    (host-forwarded, isTrusted=false) DOM events, counted by type."""
    out = {"native": {}, "page_trusted": {}, "page_synthetic": {}}
    for line in lines:
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        if not isinstance(rec, dict) or not rec.get("type"):
            continue
        if rec.get("kind") == "native":
            bucket = out["native"]
        elif rec.get("kind") == "pointer":
            bucket = out["page_synthetic"] if rec.get("trusted") is False else out["page_trusted"]
        else:
            continue
        bucket[rec["type"]] = bucket.get(rec["type"], 0) + 1
    return out


def hover_summary(sent: int, counts: dict) -> dict:
    moves = int(counts.get("pointermove", 0))
    ratio = moves / sent if sent else 0.0
    return {
        "sent": sent,
        "received_moves": moves,
        "pointerenter": int(counts.get("pointerenter", 0)),
        "mouseover": int(counts.get("mouseover", 0)),
        "ratio": ratio,
        "pass": sent > 0 and ratio >= HOVER_PASS_RATIO,
    }


# ------------------------------------------------------------------- CLI


def _cmd_transparency(a):
    ref = Image.open(a.ref)
    test = Image.open(a.test)
    excl = diff_mask(ref, test, a.sprite_threshold) if a.sprite_threshold else None
    if excl is not None:
        excl = excl.convert("L").filter(ImageFilter.MaxFilter(5)).point(lambda v: 255 if v else 0).convert("1")
    out = {
        "transparency": transparency_stats(ref, test, exclude=excl),
        "halo": halo_stats(ref, test),
    }
    print(json.dumps(out, indent=2))


def _cmd_composite(a):
    """Composite an RGBA snapshot over a background; write the result."""
    fg = Image.open(a.rgba).convert("RGBA")
    bg = Image.open(a.bg).convert("RGBA").resize(fg.size)
    Image.alpha_composite(bg, fg).convert("RGB").save(a.out)
    alpha = fg.getchannel("A")
    hist = alpha.histogram()
    print(json.dumps({
        "size": fg.size,
        "alpha0_px": hist[0],
        "alpha255_px": hist[255],
        "alpha_partial_px": sum(hist[1:255]),
    }))


def _cmd_flash(a):
    frames = []
    for p in sorted(Path(a.dir).glob("*.png")):
        frames.append((float(p.stem.split("_")[-1]), Image.open(p)))
    print(json.dumps({"frames": len(frames), "white_flash_ms": white_flash_ms(frames),
                      "pass": white_flash_ms(frames) <= FLASH_LIMIT_MS}))


def _cmd_hover(a):
    lines = Path(a.log).read_text().splitlines() if Path(a.log).exists() else []
    out = hover_summary(a.sent, parse_hover_log(lines))
    out["breakdown"] = hover_breakdown(lines)
    print(json.dumps(out))


def _cmd_checker(a):
    w, h, cell = a.w, a.h, a.cell
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = (40, 40, 40) if ((x // cell + y // cell) % 2 == 0) else (200, 200, 200)
    img.save(a.out)


def _mem_mb(tok: str) -> float:
    tok = tok.rstrip("+-")
    mult = {"B": 1 / 1048576, "K": 1 / 1024, "M": 1.0, "G": 1024.0}
    if tok and tok[-1] in mult:
        return float(tok[:-1]) * mult[tok[-1]]
    return float(tok) / 1048576


def parse_top(text: str) -> dict:
    """Sum %CPU and MEM per `top -l N -stats pid,cpu,mem` sample; drop sample 1."""
    samples, cur = [], None
    for line in text.splitlines():
        parts = line.split()
        if parts[:3] == ["PID", "%CPU", "MEM"]:
            cur = [0.0, 0.0]
            samples.append(cur)
            continue
        if cur is not None and len(parts) >= 3 and parts[0].isdigit():
            cur[0] += float(parts[1])
            cur[1] += _mem_mb(parts[2])
    samples = samples[1:]
    if not samples:
        return {"samples": 0}
    cpu = [s[0] for s in samples]
    mem = [s[1] for s in samples]
    return {"samples": len(samples), "cpu_mean": sum(cpu) / len(cpu), "cpu_max": max(cpu),
            "mem_mb_mean": sum(mem) / len(mem), "mem_mb_max": max(mem)}


def _cmd_top(a):
    print(json.dumps(parse_top(Path(a.file).read_text())))


def main(argv=None):
    p = argparse.ArgumentParser(prog="sp01_measure")
    sub = p.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("transparency")
    t.add_argument("ref")
    t.add_argument("test")
    t.add_argument("--sprite-threshold", type=float, default=DELTA_E_PASS)
    t.set_defaults(fn=_cmd_transparency)
    c = sub.add_parser("composite")
    c.add_argument("rgba")
    c.add_argument("bg")
    c.add_argument("out")
    c.set_defaults(fn=_cmd_composite)
    f = sub.add_parser("flash")
    f.add_argument("dir")
    f.set_defaults(fn=_cmd_flash)
    h = sub.add_parser("hover")
    h.add_argument("log")
    h.add_argument("--sent", type=int, required=True)
    h.set_defaults(fn=_cmd_hover)
    k = sub.add_parser("checker")
    k.add_argument("out")
    k.add_argument("--w", type=int, default=400)
    k.add_argument("--h", type=int, default=400)
    k.add_argument("--cell", type=int, default=16)
    k.set_defaults(fn=_cmd_checker)
    tp = sub.add_parser("top")
    tp.add_argument("file")
    tp.set_defaults(fn=_cmd_top)
    a = p.parse_args(argv)
    a.fn(a)


if __name__ == "__main__":
    sys.exit(main())

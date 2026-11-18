"""Fetch and check the vendored sherpa-onnx native archives (CAP-50a, CAP-50b, #45, ADR 0049).

The `local-stt` build of mog-core links the sherpa-onnx static libraries. The build must not
download them, so this script fetches them once from the pinned sherpa-onnx GitHub release and
checks each one against `crates/mog-core/sherpa-onnx-archives.sha256`. Then build with
`SHERPA_ONNX_ARCHIVE_DIR=<dest>`; that build works offline.

Run:  uv run --no-project python scripts/vendor_sherpa_onnx.py [--dest <dir>] [--all | --name <archive>]
      (default dest: vendor/sherpa-onnx, where .cargo/config.toml points SHERPA_ONNX_ARCHIVE_DIR)
Exit: 0 = every archive is present and matches its pin, 1 = a hash mismatch, 2 = a download error.
"""
from __future__ import annotations

import argparse
import hashlib
import platform
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PINS = ROOT / "crates" / "mog-core" / "sherpa-onnx-archives.sha256"
RELEASE = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/"
DEFAULT_DEST = ROOT / "vendor" / "sherpa-onnx"


def read_pins(path: Path = PINS) -> dict[str, str]:
    """Archive name -> SHA-256 from a `sha256sum` style file."""
    pins: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        sha, name = line.split("  ", 1)
        pins[name.strip()] = sha.strip()
    return pins


def host_archive() -> str:
    """The archive name that build.rs expects for this host."""
    system, machine = platform.system(), platform.machine().lower()
    table = {
        ("Darwin", "arm64"): "sherpa-onnx-v2.0.1-osx-arm64-static-lib.tar.bz2",
        ("Darwin", "x86_64"): "sherpa-onnx-v2.0.1-osx-x64-static-lib.tar.bz2",
        ("Windows", "amd64"): "sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2",
        ("Linux", "x86_64"): "sherpa-onnx-v2.0.1-linux-x64-static-lib.tar.bz2",
    }
    try:
        return table[(system, machine)]
    except KeyError:
        sys.exit(f"no pinned sherpa-onnx archive for {system}/{machine}")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def matches(path: Path, sha: str) -> bool:
    return path.is_file() and sha256_file(path) == sha


def download(name: str, part: Path) -> None:
    """Fetch `name` into `part`, resuming from its size with a Range request."""
    have = part.stat().st_size if part.exists() else 0
    req = urllib.request.Request(RELEASE + name)
    if have:
        req.add_header("Range", f"bytes={have}-")
    with urllib.request.urlopen(req, timeout=60) as resp:
        mode = "ab" if have and resp.status == 206 else "wb"
        with part.open(mode) as out:
            for chunk in iter(lambda: resp.read(1 << 20), b""):
                out.write(chunk)


def fetch(name: str, sha: str, dest: Path) -> int:
    """Download `name` into `dest` unless it already matches. Returns 0, 1 or 2."""
    target = dest / name
    if matches(target, sha):
        print(f"sha256 OK {sha} {name} (already vendored)")
        return 0
    part = dest / f"{name}.partial"
    if matches(part, sha):
        # A complete partial file: a Range request would get HTTP 416.
        part.replace(target)
        print(f"sha256 OK {sha} {name}")
        return 0
    try:
        try:
            download(name, part)
        except urllib.error.HTTPError as e:
            if e.code != 416 or not part.exists():
                raise
            # 416: the server has no bytes past the partial file, but its hash is wrong (checked
            # above). Resuming can never fix it, so delete it and download from the start.
            part.unlink()
            download(name, part)
    except OSError as e:
        print(f"download failed for {name}: {e}", file=sys.stderr)
        return 2
    got = sha256_file(part)
    if got != sha:
        part.unlink()
        print(f"SHA-256 mismatch for {name}: got {got}, want {sha}", file=sys.stderr)
        return 1
    part.replace(target)
    print(f"sha256 OK {sha} {name}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--dest", type=Path, default=DEFAULT_DEST)
    group = ap.add_mutually_exclusive_group()
    group.add_argument("--all", action="store_true", help="every pinned archive")
    group.add_argument("--name", help="one archive name from the pin file")
    args = ap.parse_args(argv)
    pins = read_pins()
    names = list(pins) if args.all else [args.name or host_archive()]
    unknown = [n for n in names if n not in pins]
    if unknown:
        print(f"not pinned: {', '.join(unknown)}", file=sys.stderr)
        return 2
    args.dest.mkdir(parents=True, exist_ok=True)
    rc = max(fetch(n, pins[n], args.dest) for n in names)
    if rc == 0:
        print(f"set SHERPA_ONNX_ARCHIVE_DIR={args.dest.resolve()}")
    return rc


if __name__ == "__main__":
    sys.exit(main())

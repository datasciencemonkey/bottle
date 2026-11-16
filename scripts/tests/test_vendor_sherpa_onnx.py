"""Tests for scripts/vendor_sherpa_onnx.py (CAP-50a, CAP-50b, #45). No network.

Run:  uv run --no-project --with pytest pytest scripts/tests -q
"""
import hashlib
import importlib.util
import re

from conftest import ROOT

spec = importlib.util.spec_from_file_location("vendor", ROOT / "scripts" / "vendor_sherpa_onnx.py")
vendor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vendor)


def test_cap_50a_pin_file_matches_the_rust_pins():
    """[CAP-50a] The pin file and SHERPA_ONNX_ARCHIVE_PINS in stt_model.rs list the same archives."""
    pins = vendor.read_pins()
    rust = (ROOT / "crates" / "mog-core" / "src" / "stt_model.rs").read_text(encoding="utf-8")
    assert len(pins) == 4
    for name, sha in pins.items():
        assert re.fullmatch(r"[0-9a-f]{64}", sha), name
        assert f'"{name}",\n        "{sha}"' in rust, name
        assert name.startswith("sherpa-onnx-v2.0.1-") and name.endswith("-lib.tar.bz2")


def test_cap_50a_host_archive_is_pinned():
    """[CAP-50a] The archive this host builds with has a pin."""
    assert vendor.host_archive() in vendor.read_pins()


def test_cap_50b_a_vendored_archive_needs_no_download(tmp_path, monkeypatch):
    """[CAP-50b] A present archive with the right hash is used as it is: no request goes out."""
    body = b"vendored archive"
    (tmp_path / "a.tar.bz2").write_bytes(body)

    def no_network(*_a, **_k):
        raise AssertionError("a download started")

    monkeypatch.setattr(vendor.urllib.request, "urlopen", no_network)
    assert vendor.fetch("a.tar.bz2", hashlib.sha256(body).hexdigest(), tmp_path) == 0


def test_cap_50a_a_changed_hash_stops_the_vendor_step(tmp_path, monkeypatch):
    """[CAP-50a] A download with other bytes is deleted and the script exits 1."""

    class Resp:
        status = 200

        def __init__(self):
            self.data = [b"other bytes", b""]

        def read(self, _n):
            return self.data.pop(0)

        def __enter__(self):
            return self

        def __exit__(self, *_a):
            return False

    monkeypatch.setattr(vendor.urllib.request, "urlopen", lambda *_a, **_k: Resp())
    want = hashlib.sha256(b"expected").hexdigest()
    assert vendor.fetch("a.tar.bz2", want, tmp_path) == 1
    assert not (tmp_path / "a.tar.bz2").exists()
    assert not (tmp_path / "a.tar.bz2.partial").exists()

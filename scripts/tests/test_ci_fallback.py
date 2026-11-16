"""Tests for the PNG fallback staleness check in tools/ci_checks.py (UX-09b, #350).

The check needs no Rive CLI: render_mog_fallback.py writes the SHA-256 of scene.rml to
mog-rive/fallback/SOURCE.sha256, and the check compares it with the current scene.rml.
"""
import hashlib
import sys

from conftest import ROOT

sys.path.insert(0, str(ROOT / "tools"))
import ci_checks  # noqa: E402
import render_mog_fallback  # noqa: E402

SCENE = b'<Artboard name="Mog"/>\n'


def tree(tmp_path, recorded=None):
    (tmp_path / "mog-rive" / "fallback").mkdir(parents=True)
    (tmp_path / "mog-rive" / "scene.rml").write_bytes(SCENE)
    if recorded is not None:
        (tmp_path / "mog-rive" / "fallback" / "SOURCE.sha256").write_text(recorded)
    return tmp_path


def test_ux_09b_fresh_hash_passes(tmp_path):
    """[UX-09b] The recorded hash equals the hash of scene.rml."""
    root = tree(tmp_path, hashlib.sha256(SCENE).hexdigest() + "\n")
    assert ci_checks.fallback_problems(root) == []


def test_ux_09b_changed_scene_fails(tmp_path):
    """[UX-09b] scene.rml changed after the PNGs were rendered."""
    root = tree(tmp_path, hashlib.sha256(SCENE).hexdigest() + "\n")
    (root / "mog-rive" / "scene.rml").write_bytes(SCENE + b"<!-- new -->\n")
    problems = ci_checks.fallback_problems(root)
    assert problems and "render_mog_fallback.py" in problems[0]


def test_ux_09b_missing_hash_fails(tmp_path):
    """[UX-09b] No SOURCE.sha256 means the PNGs have no recorded source."""
    assert ci_checks.fallback_problems(tree(tmp_path))


def test_ux_09b_line_endings_do_not_change_the_hash(tmp_path):
    """[UX-09b] A CRLF checkout of scene.rml gives the same hash."""
    root = tree(tmp_path, hashlib.sha256(SCENE).hexdigest())
    (root / "mog-rive" / "scene.rml").write_bytes(SCENE.replace(b"\n", b"\r\n"))
    assert ci_checks.fallback_problems(root) == []


def test_ux_09b_the_renderer_records_the_hash_the_check_reads(tmp_path):
    """[UX-09b] What render_mog_fallback.py writes passes the check."""
    root = tree(tmp_path)
    render_mog_fallback.write_source_hash(root / "mog-rive")
    assert ci_checks.fallback_problems(root) == []


def test_ux_09b_the_repository_fallback_is_fresh():
    """[UX-09b] The committed PNGs were rendered from the committed scene.rml."""
    ok, detail = ci_checks.check_fallback()
    assert ok, detail

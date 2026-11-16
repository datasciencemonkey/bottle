"""Tests for scripts/check_brand_gate.py (#84): the mascot brand review release gate."""
import shutil

from conftest import ROOT, run_script

import check_brand_gate as cbg

RECORD = """# Brand approval record

| Field | Value |
|---|---|
| Status | {status} |
| Tracking issue | #129 |
| Approver | {approver} |
| Date approved | {date} |
| Approval reference | {ref} |
"""

SCENE_OK = '<rml><g name="Lamp" fill="#FF5F46"/></rml>'
PKG_OK = '{"dependencies": {"@rive-app/canvas": "2.50.0"}}'


def record(status="approved", approver="A. Brand", date="2026-12-01", ref="go/brand ticket 42"):
    return RECORD.format(status=status, approver=approver, date=date, ref=ref)


def tree(tmp_path, rec=None, ticked=False, scene=SCENE_OK, pkg=PKG_OK, lock="lockfileVersion: '9'\n"):
    """Build a small repository tree and return its root."""
    (tmp_path / "mog-rive").mkdir()
    (tmp_path / "mog-rive" / "scene.rml").write_text(scene)
    (tmp_path / "package.json").write_text(pkg)
    (tmp_path / "pnpm-lock.yaml").write_text(lock)
    (tmp_path / "docs" / "release").mkdir(parents=True)
    if rec is not None:
        (tmp_path / "docs" / "release" / "BRAND-APPROVAL.md").write_text(rec)
    mark = "x" if ticked else " "
    (tmp_path / "docs" / "release" / "RELEASE-CHECKLIST.md").write_text(
        f"| Done | AT | Title |\n|---|---|---|\n| [{mark}] | AT-37 | Licences, brand and supply chain |\n")
    return tmp_path


def test_sec_22b_approved_record_passes_the_gate():
    """[SEC-22b] A record with approver, date and reference passes the gate."""
    assert cbg.gate_status(record()) == (True, "")


def test_sec_22b_pending_record_is_not_passed_with_reason():
    """[SEC-22b] A pending record is not passed and the reason names the status."""
    ok, reason = cbg.gate_status(record(status="pending", approver="", date="", ref=""))
    assert not ok and "pending" in reason


def test_sec_22b_missing_record_is_not_passed():
    """[SEC-22b] No record means not passed."""
    ok, reason = cbg.gate_status(None)
    assert not ok and "no brand approval record" in reason


def test_sec_22a_approved_without_approver_or_date_is_not_passed():
    """[SEC-22a] An approved status with no approver, a bad date or no reference does not pass."""
    for kw, word in (({"approver": "<name>"}, "Approver"), ({"date": "soon"}, "Date approved"),
                     ({"ref": "-"}, "Approval reference")):
        ok, reason = cbg.gate_status(record(**kw))
        assert not ok and word in reason


def test_sec_22b_record_must_reference_issue_129():
    """[SEC-22b] The record names the tracking issue #129."""
    ok, reason = cbg.gate_status(record().replace("#129", "#12"))
    assert not ok and "#129" in reason


def test_at_37_scene_with_lava_tail_wisp_or_brick_fails():
    """[AT-37, UX-10a, UX-10b] Automated art check finds FF3621 and Tail, Wisp or Brick nodes."""
    bad = '<g name="Tail"/><g name="Wisp"/><g name="Brick"/><path fill="#ff3621"/>'
    problems = cbg.art_problems(bad)
    assert len(problems) == 4
    assert cbg.art_problems(SCENE_OK) == []


def test_at_37_rive_webgl_package_fails_but_canvas_and_webgl2_pass():
    """[AT-37, NFR-16b] The package files must not name @rive-app/webgl."""
    assert cbg.package_problems('"@rive-app/webgl": "2.0.0"', "")
    assert cbg.package_problems("", "  '@rive-app/webgl@2.0.0':\n")
    assert cbg.package_problems('"@rive-app/webgl2": "2.0.0", "@rive-app/canvas": "2"', "'@rive-app/webgl2@2.0.0':") == []


def test_at_37_ticked_checklist_row_without_approval_fails(tmp_path):
    """[AT-37, SEC-22b] AT-37 cannot be ticked while the brand gate is not passed."""
    root = tree(tmp_path, rec=record(status="pending", approver="", date="", ref=""), ticked=True)
    assert any("AT-37" in p for p in cbg.check(root)[0])
    root2 = tree(tmp_path / "ok", rec=record(), ticked=True) if (tmp_path / "ok").mkdir() is None else None
    assert cbg.check(root2)[0] == []


def test_col_104_internal_build_not_blocked_by_pending_gate(tmp_path):
    """[COL-104] Default mode exits 0 when approval is pending; --external exits 1."""
    root = tree(tmp_path, rec=record(status="pending", approver="", date="", ref=""))
    internal = run_script("check_brand_gate.py", "--root", root)
    assert internal.returncode == 0 and "NOT PASSED" in internal.stdout
    external = run_script("check_brand_gate.py", "--root", root, "--external")
    assert external.returncode == 1 and "pending" in external.stdout


def test_sec_22b_external_passes_with_approved_record(tmp_path):
    """[SEC-22b, COL-104] --external exits 0 with an approved record and clean art."""
    root = tree(tmp_path, rec=record())
    proc = run_script("check_brand_gate.py", "--root", root, "--external")
    assert proc.returncode == 0 and "PASSED" in proc.stdout


def test_sec_22a_art_problem_fails_even_in_internal_mode(tmp_path):
    """[SEC-22a, AT-37] A Lava fill fails the check in every mode."""
    root = tree(tmp_path, rec=None, scene='<path fill="#FF3621"/>')
    assert run_script("check_brand_gate.py", "--root", root).returncode == 1


def test_sec_22b_repo_record_exists_and_is_not_passed():
    """[SEC-22b] The committed record is pending: the gate is not passed until the brand team approves (#129)."""
    text = (ROOT / "docs" / "release" / "BRAND-APPROVAL.md").read_text()
    ok, reason = cbg.gate_status(text)
    assert not ok and "pending" in reason
    assert run_script("check_brand_gate.py").returncode == 0

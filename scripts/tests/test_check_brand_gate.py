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
    tmp_path.mkdir(parents=True, exist_ok=True)
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
    pending = record(status="pending", approver="", date="", ref="")
    assert any("AT-37" in p for p in cbg.check(tree(tmp_path / "a", rec=pending, ticked=True))[0])
    assert cbg.check(tree(tmp_path / "b", rec=record(), ticked=True))[0] == []


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


def test_sec_22b_tracking_issue_must_match_exactly():
    """[SEC-22b] #1290, #1295 and "see #129x" do not pass as #129."""
    for bad in ("#1290", "#1295", "see #129x", "#12"):
        ok, reason = cbg.gate_status(record().replace("| #129 |", f"| {bad} |"))
        assert not ok and "#129" in reason, bad


def test_sec_22b_future_date_is_not_passed():
    """[SEC-22b] A Date approved after today does not pass."""
    ok, reason = cbg.gate_status(record(date="2099-01-01"))
    assert not ok and "future" in reason
    assert cbg.gate_status(record(date="2026-12-01"), today=__import__("datetime").date(2026, 12, 1))[0]


def test_sec_22b_junk_approver_is_not_passed():
    """[SEC-22b] Placeholder approvers do not pass."""
    for bad in ("pending", "me", "tbd", "", "approved", "n/a"):
        ok, reason = cbg.gate_status(record(approver=bad))
        assert not ok and "Approver" in reason, bad


def test_sec_22b_reference_must_be_url_or_ticket_id():
    """[SEC-22b] The Approval reference is a URL or a ticket ID."""
    for bad in ("x", "ok", "see email"):
        assert not cbg.gate_status(record(ref=bad))[0], bad
    for good in ("https://example.com/a/1", "BRAND-123", "AIBI-1851"):
        assert cbg.gate_status(record(ref=good))[0], good


def test_sec_22b_duplicate_field_is_a_problem(tmp_path):
    """[SEC-22b] A duplicate Status row is an error, whichever row comes last."""
    rec = record(status="pending", approver="", date="", ref="") + "| Status | approved |\n"
    ok, reason = cbg.gate_status(rec)
    assert not ok and "duplicate" in reason
    assert any("duplicate" in p for p in cbg.check(tree(tmp_path, rec=rec))[0])


def test_sec_22b_approved_but_incomplete_uses_parsed_fields(tmp_path):
    """[SEC-22b] Status approved with a bad field is a problem, also when the cell has extra spacing or case."""
    rec = record(status=" Approved ", approver="me")
    assert any("incomplete" in p for p in cbg.check(tree(tmp_path, rec=rec))[0])


def test_at_37_ticked_row_must_link_the_brand_record(tmp_path):
    """[AT-37, SEC-22b] A ticked AT-37 row names docs/release/BRAND-APPROVAL.md in its Evidence cell."""
    root = tree(tmp_path, rec=record(), ticked=True)
    assert any("Evidence" in p for p in cbg.check(root)[0])
    cl = root / "docs" / "release" / "RELEASE-CHECKLIST.md"
    cl.write_text("| [x] | AT-37 | T | release | P | C | [a](docs/evidence/AT-37.md) [b](docs/release/BRAND-APPROVAL.md) |\n")
    assert cbg.check(root)[0] == []


def test_col_104_missing_files_fail_only_under_external(tmp_path):
    """[COL-104, AT-37] A missing scene.rml, package.json or pnpm-lock.yaml fails under --external."""
    root = tree(tmp_path, rec=record())
    (root / "mog-rive" / "scene.rml").unlink()
    (root / "pnpm-lock.yaml").unlink()
    assert run_script("check_brand_gate.py", "--root", root).returncode == 0
    proc = run_script("check_brand_gate.py", "--root", root, "--external")
    assert proc.returncode == 1 and "missing mog-rive/scene.rml" in proc.stdout and "missing pnpm-lock.yaml" in proc.stdout


def test_at_37_ci_brand_check_uses_the_one_art_implementation():
    """[AT-37, UX-10a, UX-10b] tools/ci_checks.py check_brand calls check_brand_gate.art_problems."""
    import sys
    sys.path.insert(0, str(ROOT / "tools"))
    import ci_checks
    assert ci_checks.art_problems is cbg.art_problems
    assert "brandapproval" in ci_checks.CHECKS and "brandgate" not in ci_checks.CHECKS

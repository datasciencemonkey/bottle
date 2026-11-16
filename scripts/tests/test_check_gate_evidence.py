"""Tests for scripts/check_gate_evidence.py (#229, AT-22): no gate is a pass without evidence."""
import re
import shutil

from conftest import REAL_REQUIREMENTS, ROOT, run_script

import check_gate_evidence as cge

CHECKLIST = ROOT / "docs" / "release" / "RELEASE-CHECKLIST.md"

PASS_BODY = """# Evidence: {gate}

| Field | Value |
|---|---|
| Gate | {gate} |
| Gate type | {kind} |
| Result | {result} |
| Date run | {date} |
| Run by | impl-229 |
| Build | 9f5d6fe |
| Hardware | Apple M1 8 GB, macOS 14.4 |
| Profile | bottle-test |

## Method

Ran the procedure of section 15.

## Raw evidence

{raw}

## Result against the criterion

Meets the pass criterion.
"""


def evidence(tmp_path, name, gate, result="pass", kind="live", date="2026-11-20",
             raw="Log: 12 polls, all serial. p95 410 ms."):
    d = tmp_path / "evidence"
    d.mkdir(exist_ok=True)
    (d / name).write_text(PASS_BODY.format(gate=gate, kind=kind, result=result, date=date, raw=raw))
    return d


def checklist(tmp_path, ticks):
    """Copy the real checklist and tick rows: ticks maps AT ID -> evidence cell text."""
    text = CHECKLIST.read_text()
    out = []
    for line in text.splitlines():
        m = re.match(r"\| \[ \] \| (AT-\d+) \|", line)
        if m and m.group(1) in ticks:
            line = line.replace("| [ ] |", "| [x] |", 1)
            assert line.endswith("| |"), line
            line = line[:-3] + f"| {ticks[m.group(1)]} |"
        out.append(line)
    p = tmp_path / "CHECKLIST.md"
    p.write_text("\n".join(out) + "\n")
    return p


def run(checklist_path, evidence_dir):
    return cge.check(REAL_REQUIREMENTS.read_text(), checklist_path.read_text(), evidence_dir)


def test_at_22_repo_state_passes():
    """[AT-22] The committed checklist and docs/evidence have no pass without evidence."""
    proc = run_script("check_gate_evidence.py")
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "PASS" in proc.stdout


def test_at_22_no_ticked_row_and_no_evidence_passes(tmp_path):
    """[AT-22] An unticked checklist with no evidence directory is not an error."""
    assert run(checklist(tmp_path, {}), tmp_path / "missing") == []


def test_at_22_ticked_row_without_evidence_fails(tmp_path):
    """[AT-22] A ticked gate with an empty Evidence cell is reported."""
    errors = run(checklist(tmp_path, {"AT-07": ""}), tmp_path / "evidence")
    assert any("AT-07" in e and "no evidence" in e for e in errors), errors


def test_at_22_ticked_row_with_missing_file_fails(tmp_path):
    """[AT-22] A ticked gate that names a file that does not exist is reported."""
    errors = run(checklist(tmp_path, {"AT-07": "docs/evidence/AT-07.md"}), tmp_path / "evidence")
    assert any("AT-07" in e and "does not exist" in e for e in errors), errors


def test_at_22_ticked_row_with_pass_evidence_passes(tmp_path):
    """[AT-22] A ticked gate with a pass evidence file for the same AT is accepted."""
    d = evidence(tmp_path, "AT-07.md", "AT-07")
    assert run(checklist(tmp_path, {"AT-07": "[log](docs/evidence/AT-07.md)"}), d) == []


def test_at_22_ticked_row_with_fail_evidence_fails(tmp_path):
    """[AT-22] A ticked gate whose evidence says fail is reported."""
    d = evidence(tmp_path, "AT-07.md", "AT-07", result="fail")
    errors = run(checklist(tmp_path, {"AT-07": "docs/evidence/AT-07.md"}), d)
    assert any("AT-07" in e and "not pass" in e for e in errors), errors


def test_at_22_ticked_row_with_evidence_for_another_gate_fails(tmp_path):
    """[AT-22] Evidence for AT-02 does not count for the AT-07 row."""
    d = evidence(tmp_path, "AT-02.md", "AT-02")
    errors = run(checklist(tmp_path, {"AT-07": "docs/evidence/AT-02.md"}), d)
    assert any("AT-07" in e and "AT-02" in e for e in errors), errors


def test_at_22_unrun_spike_reported_as_pass_fails(tmp_path):
    """[AT-22] A spike marked pass with no run date or no raw evidence is reported."""
    d = evidence(tmp_path, "SP-03.md", "SP-03", kind="spike", date="")
    errors = run(checklist(tmp_path, {}), d)
    assert any("SP-03" in e and "Date run" in e for e in errors), errors
    d2 = tmp_path / "b"
    shutil.copytree(d, d2)
    (d2 / "SP-03.md").write_text(PASS_BODY.format(gate="SP-03", kind="spike", result="pass",
                                                  date="2026-11-20", raw="<paste raw output here>"))
    errors = run(checklist(tmp_path, {}), d2)
    assert any("SP-03" in e and "Raw evidence" in e for e in errors), errors


def test_at_22_not_run_spike_is_allowed(tmp_path):
    """[AT-22] A spike recorded as not run, with no data, is honest and passes the check."""
    d = evidence(tmp_path, "SP-04.md", "SP-04", kind="spike", result="not run", date="", raw="")
    assert run(checklist(tmp_path, {}), d) == []


def test_at_22_unknown_gate_and_bad_fields_fail(tmp_path):
    """[AT-22] Unknown gate IDs, gate types and result values are reported."""
    d = evidence(tmp_path, "x.md", "SP-99", kind="vibes", result="maybe")
    errors = run(checklist(tmp_path, {}), d)
    assert any("SP-99" in e for e in errors), errors
    assert any("vibes" in e for e in errors), errors
    assert any("maybe" in e for e in errors), errors


def test_at_22_missing_table_is_reported(tmp_path):
    """[AT-22] An evidence file without the field table is reported, not skipped."""
    d = tmp_path / "evidence"
    d.mkdir()
    (d / "notes.md").write_text("# Notes\n\nIt worked on my machine.\n")
    errors = run(checklist(tmp_path, {}), d)
    assert any("notes.md" in e and "Gate" in e for e in errors), errors


def test_at_22_ticked_at_22_needs_every_gate_spike_and_review(tmp_path):
    """[AT-22] Ticking AT-22 needs all other gates ticked, every spike run and every review passed."""
    d = evidence(tmp_path, "AT-22.md", "AT-22", kind="release")
    errors = run(checklist(tmp_path, {"AT-22": "docs/evidence/AT-22.md"}), d)
    assert any("AT-01" in e and "not ticked" in e for e in errors), errors
    assert any("SP-01" in e and "no run" in e for e in errors), errors
    for review in ("signing review", "admin preflight", "MDM keys", "legal review"):
        assert any(review in e for e in errors), (review, errors)


def test_at_22_cli_exit_codes(tmp_path):
    """[AT-22] The script exits 1 and prints FAIL when a gate has no evidence."""
    cl = checklist(tmp_path, {"AT-07": ""})
    proc = run_script("check_gate_evidence.py", "--checklist", cl, "--evidence-dir", tmp_path / "none")
    assert proc.returncode == 1
    assert "FAIL" in proc.stdout
    assert "AT-07" in proc.stdout


def test_at_22_template_documents_every_gate_type():
    """[AT-22] The evidence template has one section per gate type that the check accepts."""
    text = (ROOT / "docs" / "tests" / "EVIDENCE-TEMPLATE.md").read_text()
    for kind in cge.GATE_TYPES:
        assert re.search(rf"^## .*\b{re.escape(kind)}\b", text, re.IGNORECASE | re.MULTILINE), kind

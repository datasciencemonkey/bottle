#!/usr/bin/env python3
"""Check that no release gate is reported as a pass without evidence (#229, AT-22, NFR-23b).

  uv run --no-project scripts/check_gate_evidence.py \
      [--requirements research/final-requirements.md] \
      [--checklist docs/release/RELEASE-CHECKLIST.md] [--evidence-dir docs/evidence]

Rules (docs/tests/EVIDENCE-TEMPLATE.md has the file format):
1. Each docs/evidence/*.md file (not README.md) has a field table with a known Gate (an AT-nn or
   SP-nn ID of the requirements), a known Gate type and a Result of pass, fail, partial or not run.
2. A Result of pass needs a Date run (YYYY-MM-DD), a Run by value and a filled "Raw evidence"
   section. So an unrun spike cannot be a pass.
3. A ticked ([x]) row of the release checklist names one or more docs/evidence/*.md files in its
   Evidence cell. Each file exists, is for the same AT and has Result pass.
4. A ticked AT-22 row also needs: every other checklist row ticked, an evidence file for every
   spike with a Result other than "not run", and a pass file for each review gate type.
Exit 1 and print FAIL with one line per problem; else print PASS and exit 0.
"""
import argparse
import os
import re
import sys
from datetime import date
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import req_parse  # noqa: E402

GATE_TYPES = ("spike", "device", "live", "release", "MVP exit",
              "signing review", "admin preflight", "MDM keys", "legal review")
REVIEW_TYPES = ("signing review", "admin preflight", "MDM keys", "legal review")
RESULTS = ("pass", "fail", "partial", "not run")
REQUIRED = ("Gate", "Gate type", "Result")
EVIDENCE_LINK = re.compile(r"docs/evidence/[\w.-]+\.md")
ROW = re.compile(r"^\|\s*\[( |x|X)\]\s*\|\s*(AT-\d+)\s*\|")
PLACEHOLDER = re.compile(r"^\s*(<[^>]*>|TODO\b.*|-|n/?a)?\s*$", re.IGNORECASE)


def parse_evidence(text):
    """Return (fields, raw evidence text) of an evidence file."""
    fields, raw, section = {}, [], None
    for line in text.splitlines():
        if line.startswith("## "):
            section = line[3:].strip().lower()
            continue
        if section is None and line.startswith("|"):
            row = req_parse.cells(line)
            if len(row) >= 2 and row[0] not in ("Field", "") and not set(row[0]) <= set("-:"):
                fields[row[0]] = row[1]
        elif section == "raw evidence":
            raw.append(line)
    return fields, "\n".join(raw).strip()


def is_placeholder(text):
    return all(PLACEHOLDER.match(line) for line in text.splitlines()) if text else True


def valid_date(text):
    try:
        date.fromisoformat(text)
        return True
    except ValueError:
        return False


def load_evidence(evidence_dir, doc, errors):
    """Validate every evidence file. Return {file name: fields}."""
    files = {}
    if not evidence_dir.is_dir():
        return files
    spikes = {i for i in doc.known_ids if i.startswith("SP-")}
    for path in sorted(evidence_dir.glob("*.md")):
        if path.name == "README.md":
            continue
        fields, raw = parse_evidence(path.read_text(encoding="utf-8"))
        name = path.name
        missing = [f for f in REQUIRED if not fields.get(f)]
        if missing:
            errors.append(f"{name}: missing field(s) {', '.join(missing)} (see docs/tests/EVIDENCE-TEMPLATE.md)")
            continue
        gate, kind, result = fields["Gate"], fields["Gate type"], fields["Result"].lower()
        if gate not in doc.acceptance_tests and gate not in spikes:
            errors.append(f"{name}: Gate {gate} is not an AT or SP ID of the requirements")
        if kind not in GATE_TYPES:
            errors.append(f"{name}: Gate type {kind} is not one of {', '.join(GATE_TYPES)}")
        if result not in RESULTS:
            errors.append(f"{name}: Result {fields['Result']} is not one of {', '.join(RESULTS)}")
        if result == "pass":
            if not valid_date(fields.get("Date run", "")):
                errors.append(f"{name}: {gate} is reported as pass but has no valid Date run (YYYY-MM-DD)")
            if is_placeholder(fields.get("Run by", "")):
                errors.append(f"{name}: {gate} is reported as pass but has no Run by")
            if is_placeholder(raw):
                errors.append(f"{name}: {gate} is reported as pass but its Raw evidence section is empty")
        files[name] = {**fields, "Result": result}
    return files


def check_checklist(checklist_text, files, errors):
    """Validate ticked rows. Return (ticked AT IDs, all AT IDs)."""
    ticked, rows = set(), []
    for line in checklist_text.splitlines():
        m = ROW.match(line)
        if not m:
            continue
        at = m.group(2)
        rows.append(at)
        if m.group(1) == " ":
            continue
        ticked.add(at)
        cell = req_parse.cells(line)[-1]
        links = EVIDENCE_LINK.findall(cell)
        if not links:
            errors.append(f"{at}: the row is ticked but has no evidence (link a docs/evidence/*.md file)")
        for link in links:
            name = link.rsplit("/", 1)[1]
            fields = files.get(name)
            if fields is None:
                errors.append(f"{at}: evidence {link} does not exist or is not valid")
            elif fields["Gate"] != at:
                errors.append(f"{at}: evidence {link} is for {fields['Gate']}, not {at}")
            elif fields["Result"] != "pass":
                errors.append(f"{at}: evidence {link} has Result {fields['Result']}, not pass")
    return ticked, rows


def check_release(doc, files, ticked, rows, errors):
    """Rule 4: AT-22 ticked means every gate has evidence."""
    for at in rows:
        if at != "AT-22" and at not in ticked:
            errors.append(f"AT-22 is ticked but {at} is not ticked")
    run_spikes = {f["Gate"] for f in files.values() if f["Result"] in ("pass", "fail", "partial")}
    for sp in sorted((i for i in doc.known_ids if re.fullmatch(r"SP-\d+", i)), key=req_parse.id_sort_key):
        if sp not in run_spikes:
            errors.append(f"AT-22 is ticked but spike {sp} has no run evidence (pass, fail or partial)")
    passed = {f["Gate type"] for f in files.values() if f["Result"] == "pass"}
    for review in REVIEW_TYPES:
        if review not in passed:
            errors.append(f"AT-22 is ticked but no {review} evidence has Result pass")


def check(requirements_text, checklist_text, evidence_dir):
    """Return a list of problems; empty means no gate is a pass without evidence."""
    doc = req_parse.parse(requirements_text)
    errors = []
    files = load_evidence(Path(evidence_dir), doc, errors)
    ticked, rows = check_checklist(checklist_text, files, errors)
    if "AT-22" in ticked:
        check_release(doc, files, ticked, rows, errors)
    return errors


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--requirements", default="research/final-requirements.md")
    ap.add_argument("--checklist", default="docs/release/RELEASE-CHECKLIST.md")
    ap.add_argument("--evidence-dir", default="docs/evidence")
    args = ap.parse_args(argv)
    with open(args.requirements, encoding="utf-8") as f:
        requirements = f.read()
    with open(args.checklist, encoding="utf-8") as f:
        checklist = f.read()
    errors = check(requirements, checklist, args.evidence_dir)
    if errors:
        print("FAIL gate evidence (AT-22):")
        for e in errors:
            print(f"  {e}")
        return 1
    print("PASS gate evidence (AT-22): no gate is reported as a pass without evidence")
    return 0


if __name__ == "__main__":
    sys.exit(main())

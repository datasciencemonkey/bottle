#!/usr/bin/env python3
"""Check the mascot brand review release gate (#84, SEC-22a, SEC-22b, UX-10a..d, AT-37, COL-104).

  uv run --no-project scripts/check_brand_gate.py [--root .] [--external]

The gate passes only when docs/release/BRAND-APPROVAL.md has Status approved, an Approver, a
Date approved (YYYY-MM-DD), an Approval reference and the tracking issue #129.
Automated art checks (every mode): no Lava FF3621 and no Tail, Wisp or Brick node in
mog-rive/scene.rml (UX-10a, UX-10b); no `@rive-app/webgl` in package.json or pnpm-lock.yaml (NFR-16b).
A ticked AT-37 row in the release checklist needs a passed gate and a link to the record in its
Evidence cell (check_gate_evidence.py also needs a docs/evidence file there). With --external a missing art file fails.

Exit 1 on an art problem or an inconsistent record. A gate that is not passed exits 0, so internal
builds stay unblocked (COL-104). With --external (the first public download) it exits 1 too.
"""
import argparse
import re
import sys
from datetime import date
from pathlib import Path

RECORD = "docs/release/BRAND-APPROVAL.md"
PLACEHOLDER = re.compile(r"^\s*(<[^>]*>|TODO\b.*|tbd|-|n/?a)?\s*$", re.IGNORECASE)
WEBGL = re.compile(r"@rive-app/webgl(?![\w-])")
AT37_TICKED = re.compile(r"^\|\s*\[x\]\s*\|\s*AT-37\s*\|", re.IGNORECASE)
BAD_APPROVERS = {"pending", "approved", "tbd", "me", "none", "unknown", "n/a"}
REFERENCE = re.compile(r"https?://\S+|[A-Z][A-Z0-9]*-\d+")
ART_FILES = ("mog-rive/scene.rml", "package.json", "pnpm-lock.yaml")


def art_problems(scene):
    """Return problems in the mascot scene source (UX-10a, UX-10b, AT-37)."""
    problems = []
    if re.search(r"FF3621", scene, re.IGNORECASE):
        problems.append("scene.rml uses Lava FF3621 (UX-10b)")
    for name in ("Tail", "Wisp", "Brick"):
        if re.search(rf'name="{name}"', scene):
            problems.append(f'scene.rml has a "{name}" node (UX-10a, AT-37)')
    return problems


def package_problems(package_json, lockfile):
    """Return problems for the banned Rive runtime (NFR-16b, AT-37)."""
    return [f"{name} names @rive-app/webgl (NFR-16b)"
            for name, text in (("package.json", package_json), ("pnpm-lock.yaml", lockfile))
            if WEBGL.search(text)]


def fields(text):
    """Return ({field: value}, [duplicate field names]) of the record table."""
    out, dups = {}, []
    for line in text.splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if line.startswith("|") and len(cells) >= 2 and cells[0] not in ("Field", "") and not set(cells[0]) <= set("-:"):
            if cells[0] in out:
                dups.append(cells[0])
            out[cells[0]] = cells[1]
    return out, dups


def gate_status(record_text, today=None):
    """Return (passed, reason). The reason is empty when the gate is passed."""
    if record_text is None:
        return False, f"no brand approval record ({RECORD}); approval is tracked in #129"
    f, dups = fields(record_text)
    if dups:
        return False, f"duplicate field(s) in the record: {', '.join(sorted(set(dups)))}"
    status = f.get("Status", "").lower()
    if status != "approved":
        return False, f"brand approval is {status or 'missing'} (tracked in #129)"
    if f.get("Tracking issue", "") != "#129":
        return False, "the Tracking issue must be exactly #129"
    try:
        approved = date.fromisoformat(f.get("Date approved", ""))
    except ValueError:
        return False, "Date approved is missing or not YYYY-MM-DD"
    if approved > (today or date.today()):
        return False, "Date approved is in the future"
    if PLACEHOLDER.match(f.get("Approver", "")) or f["Approver"].lower() in BAD_APPROVERS:
        return False, "Approver is empty or a placeholder"
    if not REFERENCE.fullmatch(f.get("Approval reference", "")):
        return False, "Approval reference must be a URL or a ticket ID"
    return True, ""


def read(root, rel):
    path = Path(root) / rel
    return path.read_text(encoding="utf-8") if path.exists() else None


def check(root, external=False):
    """Return (problems, passed, reason). Problems fail in every mode."""
    problems = []
    texts = {}
    for rel in ART_FILES:
        texts[rel] = read(root, rel)
        if texts[rel] is None and external:
            problems.append(f"missing {rel}: the art checks cannot run")
    problems += art_problems(texts["mog-rive/scene.rml"] or "")
    problems += package_problems(texts["package.json"] or "", texts["pnpm-lock.yaml"] or "")
    record = read(root, RECORD)
    passed, reason = gate_status(record)
    if record is not None:
        f, dups = fields(record)
        problems += [f"{RECORD} has a duplicate {name} row" for name in sorted(set(dups))]
        if f.get("Status", "").lower() == "approved" and not passed:
            problems.append(f"{RECORD} says approved but is incomplete: {reason}")
    checklist = read(root, "docs/release/RELEASE-CHECKLIST.md") or ""
    for line in checklist.splitlines():
        if AT37_TICKED.match(line):
            if not passed:
                problems.append(f"AT-37 is ticked but the brand gate is not passed: {reason}")
            if RECORD not in line.rsplit("|", 2)[-2]:
                problems.append(f"AT-37 is ticked but its Evidence cell does not link {RECORD}")
    return problems, passed, reason


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=".")
    ap.add_argument("--external", action="store_true", help="release outside the team: a gate not passed fails")
    args = ap.parse_args(argv)
    problems, passed, reason = check(args.root, args.external)
    for p in problems:
        print(f"FAIL brand gate: {p}")
    if passed:
        print("PASSED brand gate (SEC-22b): brand approval record is complete")
    else:
        print(f"NOT PASSED brand gate (SEC-22b): {reason}")
    if problems or (args.external and not passed):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

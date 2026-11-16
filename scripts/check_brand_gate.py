#!/usr/bin/env python3
"""Check the mascot brand review release gate (#84, SEC-22a, SEC-22b, UX-10a..d, AT-37, COL-104).

  uv run --no-project scripts/check_brand_gate.py [--root .] [--external]

The gate passes only when docs/release/BRAND-APPROVAL.md has Status approved, an Approver, a
Date approved (YYYY-MM-DD), an Approval reference and the tracking issue #129.
Automated art checks (every mode): no Lava FF3621 and no Tail, Wisp or Brick node in
mog-rive/scene.rml (UX-10a, UX-10b); no `@rive-app/webgl` in package.json or pnpm-lock.yaml (NFR-16b).
Also a ticked AT-37 row in the release checklist needs a passed gate.

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
AT37_TICKED = re.compile(r"^\|\s*\[x\]\s*\|\s*AT-37\s*\|", re.IGNORECASE | re.MULTILINE)


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
    out = {}
    for line in text.splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if line.startswith("|") and len(cells) >= 2 and cells[0] not in ("Field", "") and not set(cells[0]) <= set("-:"):
            out[cells[0]] = cells[1]
    return out


def gate_status(record_text):
    """Return (passed, reason). The reason is empty when the gate is passed."""
    if record_text is None:
        return False, f"no brand approval record ({RECORD}); approval is tracked in #129"
    f = fields(record_text)
    status = f.get("Status", "").lower()
    if status != "approved":
        return False, f"brand approval is {status or 'missing'} (tracked in #129)"
    if "#129" not in f.get("Tracking issue", ""):
        return False, "the record must name the tracking issue #129"
    try:
        date.fromisoformat(f.get("Date approved", ""))
    except ValueError:
        return False, "Date approved is missing or not YYYY-MM-DD"
    for name in ("Approver", "Approval reference"):
        if PLACEHOLDER.match(f.get(name, "")):
            return False, f"{name} is empty"
    return True, ""


def read(root, rel):
    path = Path(root) / rel
    return path.read_text(encoding="utf-8") if path.exists() else None


def check(root):
    """Return (problems, passed, reason). Problems fail in every mode."""
    problems = art_problems(read(root, "mog-rive/scene.rml") or "")
    problems += package_problems(read(root, "package.json") or "", read(root, "pnpm-lock.yaml") or "")
    record = read(root, RECORD)
    passed, reason = gate_status(record)
    if record and re.search(r"^\|\s*Status\s*\|\s*approved\s*\|", record, re.IGNORECASE | re.MULTILINE) and not passed:
        problems.append(f"{RECORD} says approved but is incomplete: {reason}")
    if not passed and AT37_TICKED.search(read(root, "docs/release/RELEASE-CHECKLIST.md") or ""):
        problems.append(f"AT-37 is ticked but the brand gate is not passed: {reason}")
    return problems, passed, reason


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=".")
    ap.add_argument("--external", action="store_true", help="release outside the team: a gate not passed fails")
    args = ap.parse_args(argv)
    problems, passed, reason = check(args.root)
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

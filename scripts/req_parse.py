"""Shared parser for research/final-requirements.md and JUnit XML (#182, NFR-23a, NFR-23b).

Stdlib only. Used by req_coverage.py, gen_test_ledger.py and gen_release_checklist.py.

Rules (see docs/adr/0001-requirement-coverage-rules.md):
- A requirement row is a table row under a "### 5.x" heading whose first cell is an ID and that has
  8 cells: ID, Pri (M/S/C), Phase, Requirement, Acceptance criterion, Verify, Source, UC.
- An acceptance test is a table row under "## 15." whose first cell is AT-nn.
- The AT link of a requirement is the 14.3 table ("| AT-nn | ID, ID, ... |"). The AT IDs in the
  Verify column are kept separately as `verify_ats`.
- The known-ID set holds every hyphenated ID that starts a table row anywhere in the file
  (requirements, ATs, ARC-n, ARC-Fn, SP-nn, COL-nn).
- Cells split on "|" but not on the escaped "\\|".
"""
import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field

ID_RE = re.compile(r"^[A-Z]+-[A-Z]?\d+[a-z]?$")
PHASES = ("MVP-D", "MVP-M", "v1", "v2")
PRIORITIES = ("M", "S", "C")
CELL_SPLIT = re.compile(r"(?<!\\)\|")


@dataclass
class Requirement:
    id: str
    priority: str
    phase: str
    text: str
    verify: str
    uc: str
    line: int
    verify_ats: set = field(default_factory=set)


@dataclass
class AcceptanceTest:
    id: str
    name: str
    procedure: str
    criterion: str
    gate: str
    line: int


@dataclass
class Document:
    requirements: dict = field(default_factory=dict)        # ID -> Requirement (file order)
    acceptance_tests: dict = field(default_factory=dict)    # AT-nn -> AcceptanceTest
    at_links_14_3: dict = field(default_factory=dict)       # requirement ID -> set of AT IDs
    known_ids: set = field(default_factory=set)
    platforms: str = ""
    errors: list = field(default_factory=list)

    def at_links(self, req_id):
        """Every AT that verifies `req_id`: the 14.3 table plus the Verify column."""
        req = self.requirements.get(req_id)
        return set(self.at_links_14_3.get(req_id, set())) | (req.verify_ats if req else set())


def cells(line):
    """Split a markdown table row into stripped cells; `\\|` stays inside a cell."""
    body = line.strip()
    if body.startswith("|"):
        body = body[1:]
    if body.endswith("|") and not body.endswith("\\|"):
        body = body[:-1]
    return [c.strip() for c in CELL_SPLIT.split(body)]


def strip_bold(text):
    return text.replace("**", "").strip()


def parse(text):
    """Parse the requirements markdown into a Document. Problems go to Document.errors."""
    doc = Document()
    h2 = h3 = ""
    for n, line in enumerate(text.splitlines(), 1):
        if line.startswith("## "):
            h2, h3 = line[3:].strip(), ""
            continue
        if line.startswith("### "):
            h3 = line[4:].strip()
            continue
        if h2.startswith("15.") and not doc.platforms:
            m = re.search(r"reference hardware: (.+?\.)(?:\s|$)", line)
            if m:
                doc.platforms = m.group(1).rstrip(".")
        if not line.startswith("|"):
            continue
        row = cells(line)
        first = row[0]
        if not ID_RE.match(first):
            continue
        doc.known_ids.add(first)
        if re.match(r"5\.\d+ ", h3):
            _requirement_row(doc, row, n)
        elif h3.startswith("14.3") and first.startswith("AT-"):
            _link_row(doc, row, n)
        elif h2.startswith("15.") and first.startswith("AT-"):
            _at_row(doc, row, n)
    _check_links(doc)
    return doc


def _requirement_row(doc, row, n):
    rid = row[0]
    if len(row) != 8:
        doc.errors.append(f"line {n}: {rid}: expected 8 cells, found {len(row)}")
        return
    _, pri, phase, req, _crit, verify, _src, uc = row
    if pri not in PRIORITIES:
        doc.errors.append(f"line {n}: {rid}: priority {pri!r} is not M, S or C")
        return
    if phase not in PHASES:
        doc.errors.append(f"line {n}: {rid}: phase {phase!r} is not one of {', '.join(PHASES)}")
        return
    if rid in doc.requirements:
        doc.errors.append(f"line {n}: {rid}: duplicate ID (first at line {doc.requirements[rid].line})")
        return
    doc.requirements[rid] = Requirement(rid, pri, phase, req, verify, uc, n,
                                        set(re.findall(r"AT-\d+", verify)))


def _link_row(doc, row, n):
    at = row[0]
    if len(row) != 2:
        doc.errors.append(f"line {n}: 14.3 {at}: expected 2 cells, found {len(row)}")
        return
    for rid in filter(None, (r.strip() for r in row[1].split(","))):
        doc.at_links_14_3.setdefault(rid, set()).add(at)


def _at_row(doc, row, n):
    at = row[0]
    if len(row) != 5:
        doc.errors.append(f"line {n}: {at}: expected 5 cells, found {len(row)}")
        return
    if at in doc.acceptance_tests:
        doc.errors.append(f"line {n}: {at}: duplicate acceptance test")
        return
    _, name, proc, crit, gate = row
    doc.acceptance_tests[at] = AcceptanceTest(at, strip_bold(name), proc, crit, strip_bold(gate), n)


def _check_links(doc):
    for rid in sorted(doc.at_links_14_3):
        if rid not in doc.requirements:
            doc.errors.append(f"14.3: {rid} is not a requirement row in section 5")
    used = {a for ats in doc.at_links_14_3.values() for a in ats}
    used |= {a for r in doc.requirements.values() for a in r.verify_ats}
    if doc.acceptance_tests:
        for at in sorted(used - set(doc.acceptance_tests)):
            doc.errors.append(f"{at} is linked but is not defined in section 15")


# ---------------------------------------------------------------- tag extraction

BRACKET_RE = re.compile(r"\[([^\[\]]+)\]")
TAG_SHAPE = re.compile(r"^[A-Za-z]+-[A-Za-z]?\d+[a-zA-Z]?$")
SNAKE_RE = re.compile(r"(?<![a-z0-9])([a-z]+)_([a-z]?\d+)([a-z]?)(?:_([a-z]))?(?![a-z0-9])")


def extract_ids(text, known_ids):
    """Return (known IDs, unknown IDs) named in a test name or classname.

    Bracket tags: `[CAP-14b]`, `[MB-02, AT-02]`. Snake names: `cap_14b_stops` -> CAP-14b, matched
    case-insensitively against `known_ids`; the longest known match wins (`mem_01_b` -> MEM-01b
    when MEM-01b exists). A snake word counts only when its prefix is a known ID prefix.
    """
    lower = {k.lower(): k for k in known_ids}
    prefixes = {k.split("-")[0].lower() for k in known_ids}
    found, unknown = set(), set()
    for group in BRACKET_RE.findall(text):
        for tok in re.split(r"[,\s]+", group.strip()):
            if TAG_SHAPE.match(tok):
                hit = lower.get(tok.lower())
                if hit:
                    found.add(hit)
                else:
                    unknown.add(_canonical(tok))
    low = text.lower()
    pos = 0
    while True:
        m = SNAKE_RE.search(low, pos)
        if not m:
            break
        prefix, num, letter, next_letter = m.groups()
        if prefix not in prefixes:
            pos = m.start(2)
            continue
        cands = []
        if letter:
            cands.append(f"{prefix}-{num}{letter}")
        elif next_letter:
            cands.append(f"{prefix}-{num}{next_letter}")
        cands.append(f"{prefix}-{num}")
        hit = next((lower[c] for c in cands if c in lower), None)
        if hit:
            found.add(hit)
        else:
            unknown.add(_canonical(cands[0]))
        used_next_letter = bool(hit) and not letter and next_letter and hit.lower() == cands[0]
        pos = m.end() if used_next_letter else m.end(3)
    return found, unknown


def _canonical(tag):
    """`cap-14B` -> `CAP-14b`."""
    prefix, rest = tag.split("-", 1)
    m = re.match(r"([A-Za-z]?)(\d+)([A-Za-z]?)$", rest)
    if not m:
        return tag.upper()
    return f"{prefix.upper()}-{m.group(1).upper()}{m.group(2)}{m.group(3).lower()}"


# ---------------------------------------------------------------- JUnit

@dataclass
class TestCase:
    file: str
    name: str
    result: str  # passed | failed | skipped
    source: str  # the JUnit file

    @property
    def label(self):
        return f"{self.file}::{self.name}" if self.file else self.name


def read_junit(paths):
    """Read JUnit files. Return (testcases, missing paths, error messages)."""
    import os
    cases, missing, errors = [], [], []
    for path in paths:
        path = str(path)
        if not os.path.exists(path):
            missing.append(path)
            continue
        try:
            root = ET.parse(path).getroot()
        except (ET.ParseError, OSError) as e:
            errors.append(f"{path}: cannot read JUnit XML: {e}")
            continue
        for tc in root.iter("testcase"):
            if tc.find("failure") is not None or tc.find("error") is not None:
                result = "failed"
            elif tc.find("skipped") is not None:
                result = "skipped"
            else:
                result = "passed"
            cases.append(TestCase(tc.get("file") or tc.get("classname") or "",
                                  tc.get("name") or "", result, path))
    return cases, missing, errors


def tag_cases(cases, known_ids):
    """Return [(case, known IDs)] and {unknown ID: [test labels]}."""
    tagged, unknown = [], {}
    for case in cases:
        ids, bad = set(), set()
        for text in (case.name, case.file):
            k, u = extract_ids(text, known_ids)
            ids |= k
            bad |= u
        tagged.append((case, ids))
        for b in bad:
            unknown.setdefault(b, []).append(case.label)
    return tagged, unknown


def id_sort_key(rid):
    """Sort IDs in a natural order: AT-2 before AT-10, CAP-14a before CAP-14b."""
    prefix, _, rest = rid.partition("-")
    m = re.match(r"([A-Z]?)(\d+)([a-z]?)", rest)
    return (prefix, m.group(1), int(m.group(2)), m.group(3)) if m else (prefix, rest, 0, "")

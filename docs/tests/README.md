# Requirement coverage, test ledger and release checklist

Three stdlib Python scripts keep the requirement-to-test trace complete (NFR-23a) and list the
release gates (NFR-23b, AT-22). They read `research/final-requirements.md` and JUnit XML files.
Run them with `uv` from the repository root.

## How tests name requirements

Put the requirement or acceptance-test ID in the test name:

- TypeScript and other free-text names: a bracket tag, for example `it('[CAP-14b] stops hands-free on Esc', ...)`.
  One bracket can hold more than one ID: `[MB-02, AT-02]`.
- Rust and Python: a snake name, for example `fn cap_14b_stops_on_esc()` or `def test_nfr_23a_flags_must_row()`.
  The match is case-insensitive. The longest known ID wins (`mem_01_b_x` is MEM-01b if MEM-01b exists).

An ID that is not in the requirements makes `req_coverage.py` exit 1.

## scripts/req_coverage.py

Writes a coverage report: totals, rows with a passing test per milestone, unknown IDs, MUST rows
without an AT link in section 14.3, missing JUnit files and parse errors.

```
uv run --no-project scripts/req_coverage.py --requirements research/final-requirements.md \
    --junit <file.xml>... --out reports/requirements-coverage.md \
    [--json reports/requirements-coverage.json] [--strict-must]
```

Exit codes:

- 1: a test names an unknown ID, the requirements or a JUnit file cannot be parsed, or
  (with `--strict-must` only) a MUST row has no AT link in section 14.3.
- 0: all other cases. A missing JUnit file is reported and is not fatal.

## scripts/gen_test_ledger.py

Writes `docs/tests/TEST-LEDGER.md`: one row per (test, ID) with the columns
`Requirement ID | Issue | PR | Test (file::name) | AT | Last result | Commit`, and a summary per milestone.

```
gh issue list --state all --limit 1000 --json number,state,title,body > issues.json
uv run --no-project scripts/gen_test_ledger.py --requirements research/final-requirements.md \
    --junit <file.xml>... --out docs/tests/TEST-LEDGER.md [--commit <sha>] [--issues-json issues.json]
```

- Issue: every issue whose "Requirements to implement" section lists the ID. Blank without `--issues-json`.
- PR: always blank. The inputs do not give it, and a blank is better than a guess.
- AT: the ATs that verify the ID (section 14.3 and the Verify column). For an AT ID, the AT itself.

The committed ledger is the output with no JUnit files. A test checks that it is up to date.

## scripts/gen_release_checklist.py

Writes `docs/release/RELEASE-CHECKLIST.md`: every acceptance test in section 15 whose gate is
device, live, release or MVP exit, with its ID, title, gate, platforms and pass criterion.

```
uv run --no-project scripts/gen_release_checklist.py \
    --requirements research/final-requirements.md --out docs/release/RELEASE-CHECKLIST.md
```

When section 15 changes, run the script and commit the result. A test checks that the file is up to date.

## Tests

```
uv run --no-project --with pytest pytest scripts/tests -q
```

Fixtures are in `scripts/tests/fixtures/`. Some tests also run against the real requirements file.
The parse rules are in `docs/adr/0001-requirement-coverage-rules.md`.

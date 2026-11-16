# ADR 0001: Rules for requirement coverage and release gates

- Status: Accepted
- Date: 2026-11-15
- Issue: #182 (NFR-23a, NFR-23b, AT-22)

## Context

Issue #182 asks for a script that checks that every MUST row links to an acceptance test (AT), and for a
release checklist. The issue does not say how to read the requirements file, which IDs a test can name,
or what an "AT link" is. The file has two AT links per row: the Verify column of the section 5 tables and
the table in section 14.3.

## Decision

1. A requirement row is a table row under a `### 5.x` heading with 8 cells and Pri M, S or C.
   Cells split on `|` but not on `\|`.
2. An acceptance test is an `AT-nn` row in section 15.
3. The known-ID set is every hyphenated ID that starts a table row (requirements, AT, ARC, ARC-F, SP, COL).
   A test that names any other ID is an error (exit 1).
4. The NFR-23a check uses section 14.3, because NFR-23a says "Section 14.3 shows no MUST row without a
   test". The Verify column is also read and is reported when it has no AT. The check warns by default and
   fails only with `--strict-must`, so CI can turn it on when section 14.3 is complete.
5. A parse error in the requirements or in a JUnit file is fatal (exit 1). A missing JUnit file is not fatal.
6. Every AT with the gate device, live, release or MVP exit goes in the release checklist (NFR-23b, D79).
   The platforms come from the reference-hardware sentence in section 15.
7. The ledger leaves the PR column blank, because the inputs do not give it.

## Consequences

- On requirements v1.3, 12 MUST rows have an AT in the Verify column but no row in section 14.3:
  W-07d..W-07h, W-17c..W-17f and PG-18..PG-20. The counts table in 14.3 (753 rows, 637 MUST) is also out
  of date: the section 5 tables have 769 rows and 651 MUST rows. A requirements update must fix 14.3
  before CI uses `--strict-must`.
- When the requirements change, run `gen_release_checklist.py` and `gen_test_ledger.py` again. Tests fail
  when the committed files are out of date.

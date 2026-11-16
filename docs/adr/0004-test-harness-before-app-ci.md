# ADR 0004: The test harness lands before the app-build CI

- Status: Accepted
- Date: 2026-11-15
- Issue: #229 (depends on #227)
- Owner: impl-229 (foundation-lead team)

## Context

Issue #229 (test harness, AT-22) lists #227 (CI for the macOS and Windows builds) as a dependency.
#227 has another owner and is not on `main`. Today CI runs only `.github/workflows/repo-checks.yml`
(`tools/ci_checks.py`). It does not run `cargo test`, `pnpm test` or `pytest scripts/tests`.

The harness parts that the other issues need (the Genie One MCP double, the simulated clock and
the gate evidence check) do not need the app build. Other agents need them now to write
bridge and SessionGuard tests.

## Decision

We treat #227 as a soft dependency. The harness lands now:

- The TS, Rust and Python harness tests run locally (`pnpm test`, `cargo test --workspace`,
  `uv run --no-project --with pytest pytest scripts/tests -q`). The PR shows the output.
- The gate evidence check also runs in the existing CI through `tools/ci_checks.py` (check `gates`).
  This changes no workflow file.
- To run the TS, Rust and Python test suites in CI on macOS and Windows is the job of #227. #229
  does not edit `.github/workflows/`.
- Live and device tests skip with a clear message when `BOTTLE_TEST_PROFILE`, `BOTTLE_TEST_DEVICE`
  or `BOTTLE_TEST_HID_PORT` is not set (`docs/tests/README.md`). CI does not set them. A person
  runs the gates and records evidence (#270).

## Consequences

- Until #227 merges, a red harness test shows only in local runs and in the PR output, not in CI.
- When #227 adds the test jobs, it must run `pnpm test:ci`, `cargo test --workspace` and the
  pytest suite. It needs no change to the harness.
- A skipped live or device test is never a pass: `scripts/check_gate_evidence.py` fails a ticked
  gate without an evidence file.

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

Rules that stop false positives:

- A bracket token counts only when its prefix is upper case and is a real ID prefix (MB, CAP, AT, ...).
  So pytest parameter IDs such as `test_x[case-1]` or `[mb-01]` are not tags. `[CAP-999]` is an unknown ID.
- A snake word counts only when its prefix is a real ID prefix. If it matches no known ID, it is
  reported as unknown only when its number has 2 or more digits. So `test_w_2` is ignored and
  `mb_77_x` is the unknown ID MB-77.

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

## Test harness (#229, AT-22)

The shared parts that unit and acceptance tests use. Harness self-tests carry the ID `[AT-22]`.
ADR 0005 records that the harness lands before the app-build CI (#227).

### Genie One MCP double

`src/test-support/genie-one-mcp.ts`: `createGenieOneMcpDouble({ host, workspaceId, responses })`.
It answers JSON-RPC 2.0 `initialize`, `tools/list` and `tools/call` (`handle`), and returns the IPC
`mcp_call` shape `{ structuredContent, text, isError, httpStatus, rpcError? }` (`call`).
Each accepted `genie_ask` takes the next `ScriptedResponse`; each poll takes the next step.

| Scenario | What the double does |
|---|---|
| `scenarios.normalAnswer()` | One progress step, then `completed` with an answer, a query item and the deep link `https://<host>/one/chat/threads/<cid>?o=<wsid>&utm_source=genie_chat_mcp`. |
| busy conversation | A second `genie_ask` on a conversation with an unfinished response returns the live busy text (`BUSY_TEXT`) as `isError`, or as a JSON-RPC error with `busyEnvelope: 'jsonrpc'`. |
| `scenarios.unknownStatus(n)` | `n` polls with status `unknown`, then `completed`. |
| `scenarios.nullDeepLink()` | `completed` with `deep_link: null`. |
| `scenarios.nullProgressSteps()` | `in_progress` with `progress_steps: null`. |

`maxConcurrentPolls()` gives the peak of overlapping polls (serial polls give 1). `asks()`,
`polls()` and `calls()` give the call log. The tool list comes from the connector spec;
TODO(#29): switch it to `genie_tools.json` from SP-09 and set the real busy envelope.
The REST double for `genie_agent` (v1) is not built yet.

### Simulated monotonic clock

- Rust: `mog_core::clock::{MonotonicClock, SystemClock, SimClock}`. `SimClock::advance(Duration)`
  moves the time; clones share one time and are thread-safe. `StubSessionGuard::new(Arc<dyn
  MonotonicClock>)` takes the clock (the stop policy is #67).
- TS: `src/brain/clock.ts`, `createSystemClock()` and `createSimClock(startMs)` with `advance(ms)`.

### Live-workspace and device fixtures

Live and device tests read these environment variables. When one is not set, the test skips and
prints `skipped: <VAR> is not set, so <kind> tests do not run`. A malformed value fails the test.

| Variable | Used by | Value |
|---|---|---|
| `BOTTLE_TEST_PROFILE` | live tests | A Databricks CLI profile name with U2M sign-in (the `bottle-test` fixture; no Lakebase). You choose the profile. Never a token. |
| `BOTTLE_TEST_WAREHOUSE_ID` | live tests (optional) | A SQL warehouse ID. |
| `BOTTLE_TEST_DEVICE` | device tests | `mac-m1-8gb-14.4`, `mac-m1-current` or `win11-i5-16gb`. |
| `BOTTLE_TEST_HID_PORT` | HID rig tests (SP-03) | The Pi Pico serial port, for example `/dev/cu.usbmodem1` or `COM5`. Needs `BOTTLE_TEST_DEVICE`. |

- TS: `src/test-support/fixtures.ts`, `liveFixture()`, `deviceFixture()`, `hidRigFixture()`.
  In a test: `if (!live.enabled) return ctx.skip(live.reason);`.
- Rust: `mog_core::testkit::{live_from, device_from, hid_rig_from}` and the macro
  `mog_core::live_or_skip!()` in a test that returns `()`.

The devices, the profile and the rig need a person: TODO(#270) has the steps.

### Gate evidence (AT-22)

Each gate run is a file in `docs/evidence/` in the format of `docs/tests/EVIDENCE-TEMPLATE.md`.

```
uv run --no-project scripts/check_gate_evidence.py \
    [--checklist docs/release/RELEASE-CHECKLIST.md] [--evidence-dir docs/evidence]
```

It exits 1 when a checklist row is ticked without a pass evidence file for that AT, when an
evidence file says `pass` without a run date, a runner or raw evidence, or when AT-22 is ticked
while a gate is open, a spike has no run, or a review gate (signing, admin preflight, MDM keys,
legal) has no pass. `tools/ci_checks.py` runs it as the check `gates`.

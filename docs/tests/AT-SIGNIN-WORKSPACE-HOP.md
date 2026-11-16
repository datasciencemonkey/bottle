# AT-02, AT-12: sign-in recovery and workspace hop (#233)

These acceptance tests run the scripts of section 15 against the shipped code, with the #229
harness (`SimClock`, `mog_core::testkit`, `src/test-support/fixtures.ts`, the Genie One MCP double
and the router model double). Both ATs have the gate `live`. Most of the features under them are
not built, so most tests are pending. A pending or skipped test is never a pass. The gate is the
evidence file (`docs/tests/EVIDENCE-TEMPLATE.md`, `scripts/check_gate_evidence.py`).

## Run

```
cargo test -p mog-core --test at_02_signin_recovery
pnpm vitest run src/acceptance/at-02-signin-recovery.test.ts src/acceptance/at-12-workspace-hop.test.ts
```

## Files

| AT | File | Runs now on shipped code |
|---|---|---|
| AT-02 | `crates/mog-core/tests/at_02_signin_recovery.rs` | The R6 token buffer: a wipe sets every byte to zero, keeps the length and never shows in `Debug` (W-18c, buffer part only). |
| AT-02 | `src/acceptance/at-02-signin-recovery.test.ts` | Nothing yet: every test is pending. |
| AT-12 | `src/acceptance/at-12-workspace-hop.test.ts` | The router step returns `switch_workspace_session` and `switch_workspace_once`, resolves an alias hint to its profile and drops an unknown hint (RTR-11a). Parts of unequal length or more than 3 parts are ignored (RTR-11b). During a hop the router model call goes to the home profile (W-21a, router part only). The identity badge reads "<target> · Genie One" and the hop badge names the target and the scope (MOG-07, W-11a badge part, UX-39a). A hop sets the hop overlay and never changes the mood (UX-39b). |

W-21a, W-11a and W-18c each have a running test for one part and a pending test for the rest. The
requirement is met only when the pending test passes too. `scripts/req_coverage.py` counts any
passing test that names an ID, so its report lists these three as having a passing test.

Not in this PR: the AT-02 rows outside the 14.3 list (W-07d..W-07i, #162 and #163; W-17c and
W-17f, #16 and #161) and the AT-12 v1 Lakebase pointer for B (MEM-25c, AT-16). Their feature
issues add their own tests.

## Pending tests

Rust pending tests are `#[ignore = "pending TODO(#n) ..."]` and end in `todo!`. The live pending
test calls the R6 stub before it reads the fixture, so a forced run (`--run-ignored all`) fails and
never reads as a skip. TS pending tests are `it.skip` with `TODO(#n)` in the title and end in
`throw new Error('TODO(#n) ...')`. They compile. When the feature issue lands, its implementer
removes the mark and finishes the test.

| AT | Requirement IDs | Feature issue |
|---|---|---|
| AT-02 | W-02a, W-02b, W-02c, W-02d | #16 (PR #255 is open) |
| AT-02 | W-05a, W-05b, W-05c, W-05d | #18 |
| AT-02 | W-18a, W-18c (sign-out wiring) | #163, #18 |
| AT-02 | W-18b | #163 |
| AT-02 | W-03a, W-03b, W-03c, W-10a, W-10b | #17 |
| AT-02 | W-07a, W-07b, W-07c, W-15 | #162 |
| AT-02 | W-08 | #17, #163 |
| AT-02 | W-14, W-16a, W-16b, W-16c | #42 |
| AT-02 | W-05c, W-16b (live run) | #18, #42, a person (needs:u2m) |
| AT-12 | W-11a (resolution part), W-11b, W-11c | #164 |
| AT-12 | G-26a, G-26b, G-26c, G-26d, G-27a, G-27b | #145 |
| AT-12 | W-12a, W-12b, W-20, W-21a (STT, cleanup, writer, embedding, agent), W-21b | #165 |
| AT-12 | (v1) W-13a, W-13b, W-13c | #166 |
| AT-12 | (v1) G-50 | #146 |
| AT-12 | (v1) SEC-18, SEC-29a, SEC-29b | #218 |

The 29 min and 31 min follow-up of G-26d runs on the simulated clock (`createSimClock`) when #145
lands; it needs no live wait.

## Person steps (needs:human, needs:u2m)

A person runs these steps on a Bottle build that has the feature issues above. Each part takes less
than 30 minutes. You choose the profiles; this file never names one. Use `--profile <your-profile>`
in every command. Never run `databricks auth token`, and never put a token, a transcript or a
profile secret in a log or an evidence file. Write each result to `docs/evidence/<AT>-<part>.md` in
the format of `docs/tests/EVIDENCE-TEMPLATE.md`, with `Scope | part: <IDs> only`.

Before #16, #17, #18, #42, #162 and #163 (AT-02) or #145, #164 and #165 (AT-12) merge, the live
tests skip or stay pending, and no evidence file can say `pass`.

### AT-02 part A: expired U2M sign-in (about 15 min)

1. Connect `<your-profile>` (a `databricks-cli` U2M profile) to one MOG. Ask a question that takes
   more than 10 s, so a poll is open.
2. Note the MOG config file and its checksum: `shasum -a 256 <bottle-config-dir>/mogs/<mog-id>.json`.
3. In a terminal, end the sign-in: `databricks auth logout --profile <your-profile>`.
4. Wait for the MOG chip "Sign in to <host>". Expected: the open poll pauses; a second question waits.
5. Select the chip and finish sign-in in the browser.
6. Expected: the paused poll resumes with the same conversation ID and response ID (the card
   "Open in Genie" link has the same thread ID); the held question goes out; the config checksum
   is the same as in step 2.
7. After #16, #18, #42 and #163 remove the pending marks, run
   `BOTTLE_TEST_PROFILE=<your-profile> cargo test -p mog-core --test at_02_signin_recovery -- --nocapture`.
   Expected: no `skipped:` line, no `ignored` test and no failure.

### AT-02 part B: rejected profiles, old CLI, plaintext storage (about 15 min)

1. Have one PAT profile and one service-principal profile in `~/.databrickscfg`. Note the checksum:
   `shasum -a 256 ~/.databrickscfg`.
2. Open the profile picker. Expected: both show "Not supported (PAT or service principal)" and
   cannot be selected. No Genie call goes to their hosts.
3. Put a CLI older than 1.17 first on `PATH` and start Bottle. Expected: the step "Install the
   Databricks CLI" shows with the source, the version and the folder. The old CLI is unchanged.
4. Switch the CLI to plaintext token storage (`databricks auth describe --profile <your-profile>`
   shows the storage mode; the CLI docs name the setting), then start Bottle. Expected: the hub
   warning stops the use of the profile. Switch the storage back after this step.
5. Expected at the end: the `~/.databrickscfg` checksum is the same as in step 1.

### AT-12 part A: hop per question and per session (about 20 min)

Needs two `databricks-cli` U2M profiles on two workspaces: `<your-home-profile>` and
`<your-target-profile>`. Give the target a MOG alias in `mog.binding.profileAliases`.

1. Ask a question in the home workspace. Keep the card open.
2. Say "In <alias>, <question>". Expected: the banner "Asking in <target>. Context from <home> is
   not shared." shows first; the hop badge shows "(this question)"; the next question goes home.
3. Say "Use <alias> from now on". Expected: the hop badge shows "(this session)"; new questions
   start new conversations in the target.
4. Ask a follow-up on the card of step 1. Expected: it goes to the home workspace (pinned).
5. Open the data panel. Expected: "Questions about workspace B are processed in your home workspace."
6. Make a name ambiguous: two profiles whose names both match one word (for example `<word>-a` and
   `<word>-b`) and that have no MOG alias. Say "In <word>, <question>". Expected: the chip
   "Which workspace?" shows both; the MOG never picks one.
7. Start a meeting session, ask once, stop it, start a second session and ask again. Expected: the
   two card links have different thread IDs (G-26b).

The 29 min and 31 min parked follow-up (G-26d) is checked on the simulated clock, not in this run.

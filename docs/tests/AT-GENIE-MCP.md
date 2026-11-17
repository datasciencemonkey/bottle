# AT-07, AT-08, AT-09, AT-10, AT-33, AT-34: Genie One MCP behaviour (#234)

These acceptance tests run the scripts of section 15 against the shipped code, with the #229
harness: the Genie One MCP double (`src/test-support/genie-one-mcp.ts`), the sim clock and the
live fixture. Most of the Genie bridge is not built, so most tests are pending. The live parts
need a person with a U2M profile. A pending or skipped test is never a pass. The gate is the
evidence file (`docs/tests/EVIDENCE-TEMPLATE.md`, `scripts/check_gate_evidence.py`).

SP-09 (#29, `spikes/sp-09`, `docs/spikes/SP-09.md`) is a standalone spike crate. It proves the
thin client design offline (G-02, G-03, G-04b..d, G-05a/b, G-06a/b, G-07a, G-15a/b, G-22). The
product cannot import it, so these tests do not count its results. R8 (#30) takes its design.

## Run

```
cargo test -p mog-core --test at_07_genie_connect --test at_34_poll_budget
pnpm vitest run src/acceptance/at-07-genie-connect.test.ts src/acceptance/at-08-context-approval.test.ts \
  src/acceptance/at-09-outcomes.test.ts src/acceptance/at-10-cancel.test.ts \
  src/acceptance/at-33-genie-behaviour.test.ts src/acceptance/at-34-poll-budget.test.ts \
  src/test-support/genie-one-mcp.test.ts
```

## Files

| AT | File | Runs now on shipped code |
|---|---|---|
| AT-07 | `crates/mog-core/tests/at_07_genie_connect.rs` | The `mcp_call` contract part: only the brain may call it (SEC-02a); the server is only `genie_one`, and no argument can carry a URL, so the Beta or per-space MCP cannot be reached through IPC. These IPC checks do not count as G-03 or G-06a. R8 (#30) runs through `McpCallComponent` on the fake Genie One server of `crates/mog-core/tests/support/fake_mcp.rs`: G-04a..d, G-05a, G-06a (the allowlist), G-06b and G-07a. The Tauri host routing of `mcp_call` is TODO(#467). |
| AT-07 | `crates/mog-core/tests/mcp_transport.rs` | R8 (#30) in full against the fake server and a fake R6: G-04a..d, G-05a, G-05b, G-06a (allowlist, `tools/list` filter, the `genie-tools.json` contract), G-06b, G-07a, the 401 retry (W-05d), 4.6 rules 2, 3, 5, 6, 7, the size and time caps, and no token or content in logs, `Debug` or errors. |
| AT-07 | `src/acceptance/at-07-genie-connect.test.ts` | G-07b: concurrent poll callers on one conversation go through the shipped `ConversationManager.enqueuePoll` (brain actor). The double sees 1 open poll at most, and every poll keeps the same IDs. A control case without the queue (`[AT-22]`) sees an overlap, so the check can fail. A failed poll does not block the next one. The double lists the four tools of the contract file (`[AT-22]`). |
| AT-08 | `src/acceptance/at-08-context-approval.test.ts` | The display part of G-35a and G-35b (#89): the expanded card shows the exact "Sent to Genie" text and offers "Resend without context" only when a preamble went. |
| AT-09 | `src/acceptance/at-09-outcomes.test.ts` | Six double outcomes (completed, incomplete with and without an answer, failed, truncated, needs-input) on the shipped card parts: 3 blurbs newest on top and History (UX-11a, UX-12), a split takes one slot and a pin stays (UX-11b, UX-11c), every expanded section in order (UX-18), a 60-row truncated result shows 50 rows (G-24b display part), a chip that sends gives an action only on a click (UX-20c), success and error moods (UX-01a), one merged notification for a burst from 2 MOGs (UX-26b, UX-26c), and no notification for a chip, a visible card or a blocking Focus mode (UX-26a). |
| AT-10 | `src/acceptance/at-10-cancel.test.ts` | UX-19b (#78): Esc during a turn makes exactly one cancel call with the exchange ref, and the card stays to show "Cancellation requested", also when the cancel transport fails. The `genie_cancel_response` call is test glue in the callback; G-25a itself is #151. |
| AT-33 | `src/acceptance/at-33-genie-behaviour.test.ts` | G-19b (SEC-25a, #210): the double's deep link on the profile host passes `trustedUrl`; another host, a look-alike host, http and a link with credentials do not. |
| AT-34 | `crates/mog-core/tests/at_34_poll_budget.rs`, `src/acceptance/at-34-poll-budget.test.ts` | Nothing yet: all pending. |
| all | `src/test-support/genie-tools.json` | The Genie One tool contracts: a verbatim copy of the user's antigravity-bricks-plugin `plugins/databricks-genie/scripts/genie_tools.json` @ `a31fd2c` (MIT, `THIRD_PARTY_NOTICES.md`). The double serves them on `tools/list` and records each call that breaks an input schema (`contractViolations()`). The AT-07, AT-09 and AT-10 tests check that their calls have none. From the contract, `genie_cancel_response` takes only `conversation_id` and its status is always `cancellation_requested`, so the double never says that a turn stopped. |
| all | `src/test-support/genie-one-mcp.ts` | New double scenarios `incompleteWithAnswer`, `incompleteNoAnswer`, `failed`, `truncated(rows)` and `needsInput`, with `[AT-22]` self-tests. A query result can carry `truncated: true`. |

The contract parts prove only that the contract cannot ask for the wrong thing. The behaviour
needs the feature issue. The AT-09 test reads the double's terminal poll with test glue
(`finalPoll`). That glue is not the bridge; the event normaliser is #33.

## Pending tests

Rust pending tests are `#[ignore = "pending TODO(#n) ..."]` and end in `todo!` or in an
assertion that fails on the stub. TS pending tests are `it.skip` with `TODO(#n)` in the title and
end in an assertion or `expect.unreachable`. They compile. When the feature issue lands, its
implementer removes the mark and finishes the test.

| AT | Requirement IDs | Feature issue |
|---|---|---|
| AT-07 | G-02, G-03 (the endpoint config) | #28 |
| AT-07 | Live run of G-04a..G-06b on a real workspace (SP-09 live part) | #29 |
| AT-07 | G-07a, G-07c | #31 |
| AT-07 | W-06a, W-06b | #18 |
| AT-08 | G-28, G-29a, G-29b, G-29c, G-29d, G-32a, G-32b | #92 |
| AT-08 | G-30a, G-30b, G-30c | #90 |
| AT-08 | G-31, G-37a, G-37b, G-37c | #91 |
| AT-08 | G-33a, G-33b, G-34a, G-34b (and the G-35a, G-35b behaviour) | #152 |
| AT-08 | SEC-23 | #140 |
| AT-09 | G-21a, G-21b, G-22, G-23a, G-23b | #33 |
| AT-09 | UX-13, UX-14a, UX-14b, UX-15, UX-17 | #86 |
| AT-09 | UX-16a, UX-16b | #88 |
| AT-09 | G-12a, G-12b, G-12c | #149 |
| AT-09 | G-24a, G-24c | #150 |
| AT-09 | G-38a, G-38b | #40 |
| AT-10 | G-25a (the bridge call), G-25b, G-25c, G-25d | #151 |
| AT-10 | the acknowledged, no-effect and unknown cancel outcomes | #39 |
| AT-33 | G-14, G-15a, G-15b | #32 |
| AT-33 | G-17a, G-17b, G-19a, G-20a, G-20b | #87 |
| AT-33 | G-18a, G-18b | #31 |
| AT-34 | G-08 | #31 |
| AT-34 | G-09a, G-09b, G-09c, G-09d, G-10a, G-10b | #138 |
| AT-34 | G-11a, G-11b, G-11c | #148 |
| AT-34 | ADM-07 | #183 |

UX-01b (views never set the mood) is the inspection test `src/views/mood-write.test.ts`.

## Live and device steps (needs:human, needs:u2m)

A person runs these steps after the feature issues of each part have landed. Each part takes less
than 30 minutes. Bottle never picks a profile: use `--profile <your-profile>` for a U2M profile
on a test workspace that has Genie One on. Never put a token, a question, an answer, a transcript
or a profile secret in a log or an evidence file. Write each result to
`docs/evidence/<AT>-<part>.md` in the format of `docs/tests/EVIDENCE-TEMPLATE.md`, with
`Scope | part: <IDs> only`. Record counts, statuses and timings only.

Common setup (5 min):

1. `databricks auth login --host <your-test-workspace-url> --profile <your-profile>`.
2. `databricks auth token --profile <your-profile> >/dev/null && echo ok` prints `ok`.
3. `export BOTTLE_TEST_PROFILE=<your-profile>` and, if the test MOG has one,
   `export BOTTLE_TEST_WAREHOUSE_ID=<warehouse-id>`.
4. Remove the `#[ignore]` or `it.skip` mark of the live test of the part in a local branch only.

### AT-07: connect and poll (after #18, #28, #30, #31)

1. `cargo test -p mog-core --test at_07_genie_connect at_07_live -- --nocapture`.
2. Expected: one SCIM call gives the workspace ID; `tools/list` lists the four Genie tools; the
   URL of every call has `?o=<workspace_id>`; polls on one conversation never overlap, and their
   gaps follow Table G-POLL within ± 10 %; the conversation and response IDs stay the same.
3. Set the MOG backend URL to `https://<host>/api/2.0/mcp/genie`. Expected: the config does not
   load, and the error names the end of life 2026-11-30.

### AT-08: approve exact context (after #90, #91, #92, #140, #152)

1. Turn on "Share context with Genie" for a test MOG. Add 6 notes, one of them 200 chars long.
2. Ask one question. Expected: the preview shows the header, 5 items or less, each 160 chars or
   less; the target shows the profile host.
3. In four separate runs, change the question, the target profile, the conversation and the
   attempt after the approval. Expected: each change cancels the approval, and nothing is sent.
4. Ask a question that a workspace policy denies (the admin sets the policy). Expected: one
   second send without the preamble, then "A workspace policy blocked this question.".
5. Expected: the audit entry hash equals the hash of the "Sent to Genie" text of the card.

### AT-09: every response outcome (device; after #33, #86, #88, #149, #150, #40)

1. On a reference device (`BOTTLE_TEST_DEVICE`), ask questions that give a completed answer with
   numbers and units, a partial answer, a failure, a table with more than 50 rows, and a short
   clarify question.
2. Expected: each blurb keeps the status; key numbers are verbatim; the expanded card shows every
   section; the progress line never shows MCP, backend, polling, poll or thread.

### AT-10: cancel with uncertain result (after #39, #151)

1. Ask a long question. Press Esc while the card shows progress.
2. Expected: the card says "Cancellation requested"; no poll follows; no text says that the
   remote side stopped. Turn the network off before the cancel in a second run. Expected: the
   state is uncertain, with no remote-success claim.

### AT-33: live Genie One behaviours (after #31, #32, #87)

1. Ask a long question, then send a second question to the same conversation at once.
   Expected: `CONVERSATION_BUSY`, and no second question goes without a click.
2. Ask a short question. Expected: while steps are null, the card shows "Working… m:ss".
3. Open the deep link. Expected: it is `https://<host>/one/chat/threads/<cid>?o=<workspace_id>`.

### AT-34: poll budget and 429 (needs:admin; after #31, #138, #148, #183)

1. A workspace admin sets the `genie_one_mcp` MCP QPM for the test user to 40.
2. Start 4 long asks on one profile at once. Count the calls for 5 min (the transport trace has
   counts and times only). Expected: 40 calls/min or less; polls leave before queued asks.
3. The admin sets the QPM to 10 to force a 429. Expected: the rate halves for 5 min and then
   comes back by 5 calls/min each minute; with `Retry-After` the next call waits for it.
4. Record whether the live 429 carries `Retry-After` (SP-18, #136). For a 429 without it,
   expected: a new ask retries per Table G-BACKOFF (2 s start, factor 2, 60 s maximum, 5 tries or
   less, full jitter), and a poll only makes its interval longer. If the live 429 always has the
   header, the scripted 429 of `at_34_g_10b_*` (#138) is the evidence for this case.

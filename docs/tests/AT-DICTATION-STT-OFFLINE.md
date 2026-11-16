# AT-04, AT-05, AT-40, AT-43: dictation, STT and offline (#231)

These acceptance tests run the scripts of section 15 against the shipped code, with the #229
harness. Most features of these ATs are not built, so most tests are pending. The parts that need
a device or a person skip with the reason, or are steps in this file. A pending or skipped test is
never a pass. The gate is the evidence file (`docs/tests/EVIDENCE-TEMPLATE.md`,
`scripts/check_gate_evidence.py`).

## Run

```
cargo test -p mog-core --test at_04_offline --test at_40_stt_bench --test at_43_dictation
pnpm vitest run src/acceptance/at-04-offline.test.ts src/acceptance/at-05-meaning.test.ts \
  src/acceptance/at-40-stt-provider.test.ts src/acceptance/at-43-dictation.test.ts \
  src/test-support/cloud-calls.test.ts src/test-support/meaning.test.ts
```

## Files

| AT | File | Runs now on shipped code |
|---|---|---|
| AT-04 | `crates/mog-core/tests/at_04_offline.rs` | A transcript without `provider`, `model` or `runtime` is refused (CAP-72, contract part). |
| AT-04 | `src/acceptance/at-04-offline.test.ts` | Nothing yet: all pending. |
| AT-05 | `src/acceptance/at-05-meaning.test.ts` | Nothing yet: all pending. |
| AT-05 | `src/test-support/meaning.ts` | The corpus (30 synthetic items: numbers, units, negation, names, table names, self-corrections) and the meaning oracle. Self-tests `[AT-22]` in `meaning.test.ts`. |
| AT-04, AT-43 | `src/test-support/cloud-calls.ts` | The class (local or cloud) of every 4.5 command. A new command fails the typecheck until it has a class. Self-tests `[AT-22]` in `cloud-calls.test.ts`. |
| AT-40 | `crates/mog-core/tests/at_40_stt_bench.rs` | The scorer `mog_core::stt_bench` (ADR 0034): golden-set rules, WER, numbers, entities, hallucinations, p95 on 10 s, Opus delta, the gate. Self-tests `at_22_*`. The device run skips. |
| AT-40 | `src/acceptance/at-40-stt-provider.test.ts` | Nothing yet: the SttProvider rules are pending. |
| AT-43 | `crates/mog-core/tests/at_43_dictation.rs` | Only the brain may call `insert_text`; its arguments have no key, submit or Enter field; a multi-line text is one argument (MOG-06b, contract part). |
| AT-43 | `src/acceptance/at-43-dictation.test.ts` | Nothing yet: all pending. |
| AT-43 | `crates/mog-core/tests/latency_bench.rs` (#171) | NFR-01d: `nfr_01d_dictation_insert_p95_within_1_5_s_with_local_stt` and the device run `at_21_at_43_device_samples_meet_the_latency_budgets`. This PR does not copy them. |

The contract parts prove only that the contract cannot ask for the wrong thing. The behaviour
needs the feature issue.

## Pending tests

Rust pending tests are `#[ignore = "pending TODO(#n) ..."]` and end in `todo!`. TS pending tests are
`it.skip` with `TODO(#n)` in the title and end in an assertion or `expect.unreachable`. They compile.
When the feature issue lands, its implementer removes the mark and finishes the test.

| AT | Requirement IDs | Feature issue |
|---|---|---|
| AT-04 | CAP-60a, CAP-60b | #52 |
| AT-04 | CAP-72 (the engine fills model and runtime) | #46 |
| AT-04 | RTR-15a, RTR-15c | #143 |
| AT-04 | G-46a, G-46b | #41 |
| AT-04 | MEM-08f, MEM-19d | #102 |
| AT-04 | MEM-22a, MEM-22b | #199 |
| AT-04 | NFR-17a, NFR-17b (the run across components) | #175 |
| AT-05 | CAP-66a, CAP-66b, CAP-68 | #55 |
| AT-05 | CAP-70a, CAP-70b | #61 |
| AT-05 | MOG-05a, MOG-05b (and the late cleanup result) | #60 |
| AT-40 | CAP-20 | #23 |
| AT-40 | CAP-33a, CAP-33b | #126 |
| AT-40 | CAP-47a, CAP-47b, CAP-64a, CAP-64b | #52 |
| AT-40 | CAP-54a, CAP-54b, CAP-56a, CAP-56b | #49 |
| AT-40 | CAP-65a, CAP-65b (the runner and the release record) | #53 |
| AT-43 | MOG-06a, MOG-06c (the paste and the clipboard restore) | #58 |
| AT-43 | RTR-01b, RTR-01c, RTR-17a (Shift) | #59 |
| AT-43 | MOG-06d, UX-42 | #60 |

## Device and person steps

A person runs these steps. Each part takes less than 30 minutes once its feature issues have
landed. Never put a token, a transcript, audio or a profile secret in a log or an evidence file.
Use `--profile <your-profile>` where a profile is needed; Bottle never picks one. Write each
result to `docs/evidence/<AT>-<part>.md` in the format of `docs/tests/EVIDENCE-TEMPLATE.md`, with
`Scope | part: <IDs> only`. Use the device IDs of `docs/tests/README.md`.

### AT-40 part A: record the golden set (people, about 30 min per speaker; #53)

1. Read the scripts of #53 (240 or more utterances with numbers, names and table names). Record
   10 or more speakers in 4 conditions: `quiet` (a quiet room, laptop mic) and three more (for
   example `noise`, `far_field`, `accent`). Record 20 or more clips of 10 s each (9 to 11 s).
2. Record 20 or more non-speech clips: room noise, music, keyboard, cough, silence.
3. Keep the audio and the manifest outside the repository, on an encrypted disk. They are voices
   of real people. Write the manifest in the format of ADR 0034, decision 2. Do not commit it.

### AT-40 part B: run the benchmark (macOS M1 8 GB, about 20 min; blocked on #46, #49, #53)

1. Run the #53 runner for `local_parakeet` direct, `dbx_gemini` direct (with
   `--profile <your-profile>`, after the probe passes) and `local_parakeet` through Opus. It
   writes one result file for each.
2. Run:
   ```
   BOTTLE_TEST_DEVICE=mac-m1-8gb-14.4 BOTTLE_STT_GOLDEN=~/golden/manifest.json \
     BOTTLE_STT_RESULTS=~/golden/local.json,~/golden/gemini.json,~/golden/local-opus.json \
     BOTTLE_STT_DATE=$(date +%F) BOTTLE_STT_RUN_BY=<your role> \
     cargo test -p mog-core --test at_40_stt_bench at_40_device -- --nocapture
   ```
   Expected: `ok`, no `skipped` line, and a summary with `AT-40 STT benchmark: pass`. The summary
   has rates and counts only. Paste it into `docs/evidence/AT-40.md`.
3. Repeat step 1 and 2 on `win11-i5-16gb`.

### AT-04: dictate offline (macOS, about 15 min; blocked on #41, #52, #102, #143, #175)

1. Start Bottle with the network on. Turn Wi-Fi off and pull any cable.
2. Hold the talk key over a text editor and say "the offline test works". Expected: the text is
   inserted and the card shows "not cleaned".
3. Say "stop recording" during a hands-free capture. Expected: the capture stops.
4. Ask "what were the sales in march". Expected: an "Ask Genie" chip; click it; the card says the
   ask waits in the outbox.
5. Say "remember the Q3 target is 4 million". Expected: the item is saved; the embedding waits.
6. Turn the network on. Expected: the workspace line shows, then the ask replays once; the
   embedding runs. Quit Bottle and check the outbox is empty in Settings.
7. During steps 2 to 5, a packet capture (for example Little Snitch) shows no request from Bottle.

### AT-05: transcript meaning (macOS, about 15 min; blocked on #55, #60, #61)

1. Dictate the 30 sentences of `src/test-support/meaning.ts` (`MEANING_CORPUS`, field `raw`) into
   a text editor. Expected: each kept token is exact in the inserted text, or the raw text shows.
2. For one long sentence, edit the card text before cleanup returns (slow network). Expected: the
   late cleanup result does not change the edit. "Use raw text" puts the raw text back.

### AT-43: dictation into the active app (macOS and Windows, about 25 min; blocked on #58, #59, #60)

1. Copy the word `earlier` to the clipboard.
2. Dictate "line one new line line two" into TextEdit (Notepad), a browser text field, Terminal
   (Windows Terminal) and a password field. Expected: in the first three the text appears with no
   Enter (Terminal runs nothing); the pill shows "Inserted"; after 1 s the clipboard holds
   `earlier`. The password field shows the reason and Copy.
3. Remove the Accessibility permission for Bottle and dictate into TextEdit. Expected: the card
   shows the text, the reason and Copy.
4. Hold Shift with the talk key and dictate a plain sentence. Expected: the preview chip (the ask
   route), not an insertion.
5. With the app log on, check that no `agent_send` and no `mcp_call` happened in steps 2 and 3.
6. Do Part B of `docs/tests/LATENCY-BENCH.md` with `--gate AT-43` for NFR-01d.

# ADR 0032: The STT benchmark scorer rules (AT-40)

- Status: Accepted
- Date: 2026-11-16
- Issue: #231 (soft dependencies #44 SP-04, #47 SP-05; the golden set and release gate are #53)
- Owner: impl-231 (packaging-lead team)

## Context

AT-40 says: run the golden set (240 or more utterances, 10 or more speakers, 4 conditions,
20 non-speech clips) on both providers and the Opus path. Pass: WER 10 % or less (7 % or less for
quiet English), numbers 97 % or more, entities 85 % or more, 0 of 20 hallucinations, STT p95 1.0 s
or less on 10 s, Opus delta 0.5 pp or less. The requirements do not say how text is normalised
for WER, what a correct number or entity is, which clips give the p95, or what the 4 conditions
are. The scorer must decide these. #53 owns the golden set collection and the release record, and
it depends on #46 and #49. The scorer can be built and tested now on synthetic data.

## Decision

1. The scorer is the module `mog_core::stt_bench`, next to `latency_bench`. It uses
   `latency_bench::{percentile, SampleSource, MIN_TIME_SAMPLES}` and the testkit device fixture.
   It needs no new dependency.
2. Inputs are two JSON files: a manifest (one row per utterance: id, speaker, condition, lang,
   kind, reference, numbers, entities, audio_ms) and one result file per provider and route
   (id, text, latency_ms). Unknown fields are an error. The audio and the manifest of real
   speakers stay outside the repository.
3. Text is compared as tokens: lower case, punctuation removed, decimals, percents and dotted table
   names kept, thousands commas dropped. The TS AT-05 oracle (`src/test-support/meaning.ts`) uses
   the same rules.
4. WER is corpus level (word edits over reference words) over the speech utterances. Quiet
   English is the condition `quiet` with `lang` `en` or `en-*`.
5. A number or an entity is listed in the manifest as written in the reference. It counts as kept
   when its tokens appear in order and next to each other in the hypothesis. The manifest must
   not list a token that its reference does not have.
6. A non-speech clip with any token in its hypothesis is a hallucination.
7. "p95 on 10 s" is the nearest-rank p95 over the speech clips of 9 to 11 s. The golden set needs
   20 or more of them (the latency harness minimum).
8. The golden set needs 240 or more speech utterances plus 20 or more non-speech clips, 10 or
   more speakers, 4 or more conditions, and one condition named `quiet`. The other condition names
   are free; #53 chooses them.
9. AT-40 needs three runs: `local_parakeet` direct, `dbx_gemini` direct (CAP-47a) and
   `local_parakeet` through the Opus round trip (CAP-33a). The Opus run gates only the delta.
10. A missing result counts as deleted words, missed numbers and entities, and a time over any
    limit. A result for an unknown id fails the gate.
11. The gate is `pass` only for a run on a reference device with a run date and a runner. Doubles
    give `not evidence`. The summary has counts and rates only, never a reference or a hypothesis.

## Consequences

- `cargo test -p mog-core --test at_40_stt_bench` runs the scorer self-tests with no device.
- #53 writes the runner that produces the result files and the release record, and reuses this
  scorer. A change to these rules is a change to this ADR.
- The device run skips with the reason until a person sets `BOTTLE_TEST_DEVICE`,
  `BOTTLE_STT_GOLDEN` and `BOTTLE_STT_RESULTS`.

# ADR 0123: Databricks dictation engine, and system.ai models for call recording

- Status: Accepted (owner decision, 2026-11-24)
- Amends: ADR 0122 (dictation text path), ADR 0103 (model defaults)
- Related: ADR 0101, ADR 0088 (T4 chain), AGENTS.md section 1 (the "cloud ASR for dictation" line)

## Context

Local Parakeet is the only dictation engine. It does not cover every language, and some users want the
transcript of a Databricks model. The owner approved an opt-in Settings choice on 2026-11-24. The owner also
approved `system.ai` model ids for call recording. Bottle uses no `databricks-*` serving endpoint.

## Live probe

Profile DEFAULT, 2026-11-24. The clips are synthetic (macOS `say`, 16 kHz mono). The probe holds no real speech.
The notes show status codes, timings and shapes only.

| Call | Result |
|---|---|
| `POST /ai-gateway/mlflow/v1/chat/completions`, model `system.ai.gemini-3-8-flash`, `audio_url` data URL, WAV or FLAC | 200 |
| `POST /ai-gateway/mlflow/v1/responses`, same model, `input_file` part with a data URL | 200 |
| Responses with an `input_audio` part (data or `audio_url`) | 400. The model refuses the content type. |
| Chat completions, model `system.ai.claude-sonnet-5-5`, audio part | 400. The model takes text, image and document only. |
| Chat completions, model `system.ai.claude-sonnet-5-5`, text only (the notes call) | 200 in 1.2 s |

Latency of one transcribe call (3 runs each, FLAC):

| Clip | Verbatim | With a "remove fillers" prompt |
|---|---|---|
| About 3.7 s | 1.7 s to 2.3 s | 1.5 s to 2.4 s |
| About 10.3 s | 1.8 s to 2.5 s | 1.8 s to 2.5 s |

One call can transcribe and remove fillers. It did so in all runs. But the output is not word-exact: the model
also changes punctuation and joins phrases. The word check of ADR 0122 then has no stable base. So the engine
asks for the words as spoken. The existing pre-pass, cleanup model and word check stay in the brain.

## Decision

1. **Setting.** Settings → Workspace has "Dictation engine": "On this Mac" (default) or "Databricks". The
   contract has one new field, `cloudDictation` (bool, default false), in `workspace_settings_write` and in
   `AgentSettings`. There is no model field. The model is `system.ai.gemini-3-8-flash`, the recording transcribe
   default.
2. **Route.** Bottle uses chat completions with an `audio_url` data URL. This is the recording request, in one
   shared function (`recording_cloud::model::audio_chat_body`). The audio is FLAC, 16 kHz mono, from RAM. The call
   uses the home profile only, through `GatewayChatClient` (R7, token broker, tags, shared rate bucket).
3. **Deadline and cancel.** The deadline is 3.2 s, not the probed worst case (2.5 s) plus margin, and not the
   ADR 0122 ask-loop budget. The brain's arrival gate (`arrival-gate.ts`) holds Ask Genie arrivals for a pasting
   hold for at most 5 s after key-up (`ARRIVAL_CAP_MS`). The cloud deadline (3.2 s) plus the dictation cleanup
   budget (1.5 s, ADR 0122) is 4.7 s, under that 5 s cap with 0.3 s of slack for the rest of the paste path, so a
   slow cloud call cannot let an arrival move the card before the paste lands. No retry. Wipe and the zero-buffer
   event cancel the call; a cancel gives no transcript, enforced even if the HTTP call itself returns a reply in
   the same instant (see "Races" below).
4. **Local audio stays the fallback.** R4 still decodes the take with Parakeet during speech. It also keeps the
   take in RAM, up to a 120 s cap (the PTT hold cap, CAP-04a) and only when the choice is on, re-checked at both
   key-down and key-up (see "Races" below). On any cloud error, including a panic inside the cloud call, R4
   gives the local text. The MOG shows "Databricks dictation was not available. I used this Mac." A take with no
   sound never goes to the cloud.
5. **Same pipeline.** The cloud text is the transcript text. The filler pre-pass, the "Clean up dictation"
   switch, the word check and the paste work as before. A PTT take and a hands-free take go to the cloud, both
   capped at 120 s (decision 4): hands-free itself allows up to 5 min, but R4 pre-sizes its cloud buffer to the
   smaller, shared cap, so it never reserves the 19 MB a 5-min buffer would take for the common, short hold. A
   session take does not go to the cloud. A PTT overlay take during a call recording (the talk key's own
   behaviour, ADR 0118) may go to the cloud like any other PTT take; this is accepted, not a new decision.
6. **The T4 chain.** The brain still asks R4 for `local_parakeet`. T4 accepts a `dbx_gemini` reply to that
   request. The reply names its real provider, so the STT badge shows it. The MOG's fallback notice
   (`stt-wiring.ts`) reads the *current* Settings choice when the transcript resolves, not necessarily the one
   in effect when the hold began (Rust decides at commit time, an instant earlier); a user flip in that narrow
   window can show or skip the notice for a take that did not itself race the flip. Accepted: the transcript
   text is never wrong, only the notice, and a per-take flag from Rust is not worth a contract change for it.
7. **Call recording.** The defaults are now `system.ai.gemini-3-8-flash` (transcribe) and
   `system.ai.claude-sonnet-5-5` (notes). The endpoint rule accepts a dot. A saved value that is exactly an old
   default id reads as the new default. Any other saved value stays. A retry of an old job also uses the new ids.
8. **Settings read at key-down.** `cloud_dictation` is cached in one `AtomicBool`, kept by every file load and by
   `workspace_settings_write`. A key-down reads that atomic, not the file-backed settings cache: no lock, no
   clone, no disk.

## Races

A cloud call runs on its own thread, started at key-up (commit) and tracked in a small in-memory list of
(lease, cancel-flag) pairs so Wipe or the zero-buffer event can cancel it. Two races matter:

- **A cancel that lands right as the call finishes.** The HTTP wait loop checks the cancel flag every 20 ms; a
  cancel set in the gap between its last check and the reply arriving could, in the first cut of this feature,
  still be delivered as a successful cloud transcript. The fix: the call's own thread removes its list entry and
  re-reads the cancel flag under the one lock that cancelling also takes, and only then decides whether to
  deliver. Either the cancel is observed there (no delivery, matching a true cancel), or the thread's removal of
  its own entry happens first (the cancel request, if any, arrives too late to find an entry to flag, same as a
  cancel that was always going to be too late). A regression test sends a cloud reply that waits for the cancel
  flag and then still returns success, and asserts no transcript is delivered.
- **The choice switched off mid-hold.** The take decides whether to buffer audio for the cloud at key-down. R4
  re-checks the choice again at key-up, right before it would start the upload, so a user who switches to "On
  this Mac" partway through a hold gets no upload for that hold. `GatewayCloudStt` also refuses to call out
  (`off`) when asked anyway, as a second, independent gate.

A panicking cloud call is caught (`catch_unwind`) and treated as any other failure: the local text is delivered,
and the call's list entry is still removed. Uncaught, a panic would both skip delivery (the user sees no
transcript for that hold) and leak the list entry.

## Contract changes

- `AgentSettings.cloudDictation` and `workspace_settings_write.cloudDictation`: new bool.
- `EndpointName` accepts `.`: pattern `^[A-Za-z0-9_.-]{1,128}$`.
- `RECORDING_DEFAULT_TRANSCRIBE_ENDPOINT` and `RECORDING_DEFAULT_ENDPOINT`: new default values.

## Privacy

With "Databricks", the voice of each talk-key hold goes to the home workspace's AI Gateway. The disclosure line is:
"Databricks: your voice is sent to your home workspace's AI Gateway to transcribe it." The data disclosures view
and `docs/user/where-your-data-goes.md` say the same. Logs, errors and `Debug` output hold no audio, text, prompt
or token.

## Consequences

- With "Databricks", R4 holds up to 2 min of audio in RAM for a PTT or a hands-free take alike (decision 5). R4
  zeroes it after the call.
- A cloud call adds up to about 2.5 s after key-up (the probed worst case), bounded by the 3.2 s deadline. The
  user can switch back to "On this Mac".

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
3. **Deadline and cancel.** The call has a 6 s deadline and no retry. Wipe and the zero-buffer event cancel it. A
   cancel gives no transcript.
4. **Local audio stays the fallback.** R4 still decodes the take with Parakeet during speech. It also keeps the
   whole take in RAM, only when the choice is on. On any cloud error, R4 gives the local text. The MOG shows
   "Databricks dictation was not available. I used this Mac." A take with no sound never goes to the cloud.
5. **Same pipeline.** The cloud text is the transcript text. The filler pre-pass, the "Clean up dictation"
   switch, the word check and the paste work as before. A PTT take and a hands-free take go to the cloud. A
   session take does not.
6. **The T4 chain.** The brain still asks R4 for `local_parakeet`. T4 accepts a `dbx_gemini` reply to that
   request. The reply names its real provider, so the STT badge shows it.
7. **Call recording.** The defaults are now `system.ai.gemini-3-8-flash` (transcribe) and
   `system.ai.claude-sonnet-5-5` (notes). The endpoint rule accepts a dot. A saved value that is exactly an old
   default id reads as the new default. Any other saved value stays. A retry of an old job also uses the new ids.

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

- With "Databricks", R4 holds up to 2 min (PTT) or 5 min (hands-free) of audio in RAM. R4 zeroes it after the call.
- A cloud call adds about 2 s after key-up. The user can switch back to "On this Mac".

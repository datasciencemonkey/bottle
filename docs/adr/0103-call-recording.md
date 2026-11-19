# ADR 0103: Call recording to a Unity Catalog volume, with the transcript and notes from model serving

- Status: accepted (user decisions, 2026-11-19)
- Refs: #65, #107, #108, #116 (no new issue)
- Date: 2026-11-19
- Owner: rec-lead (contract PR by impl-rec-contract)
- Deviates from: CAP-07b ("never mix"), #116 (local STT for sessions) and AGENTS.md section 1
  ("Meeting recording" was out of scope)
- Refers to: ADR 0098 (the ⌥G key), ADR 0101 d7 (ask for a permission at once), ADR 0063
  (app settings), ADR 0091 (hub models outside the contract)

## Context

On 2026-11-19 the user moved call recording into the MVP. Bottle must record a call like a
call recorder: the mic and the other side of the call. Bottle must keep the audio, the
transcript and the notes in Unity Catalog, not on the device. The transcript and the notes
must come from Databricks model serving. No compute must start in the workspace.

The requirements describe a different meeting mode. They keep the You track and the Others track
apart (CAP-07a, CAP-07b) and run local STT on segments (#116). The user decisions below
replace these rules for call recording.

## Decisions

1. **One mixed stream.** Bottle records the mic and the system audio and mixes them to one
   16 kHz mono stream (sum with a soft clip). This deviates from CAP-07b. The transcribe model labels
   the speakers, so the mixed stream still gives speaker turns. macOS 14.4+ uses a global
   Core Audio process tap that excludes the Bottle process, with a private aggregate device.
   Older macOS records the mic only and shows a plain note. Windows uses WASAPI loopback, or
   the mic only. Bottle uses no private TCC API (CAP-10b). 5 s of zero buffers on the system
   audio sets `systemAudio: "silent_or_denied"` and shows a fix-it notice (CAP-10a).
2. **A knob, off by default.** Settings has one recording knob. Rust checks it: with the knob
   off, `recording_start` answers `recording_off`, and no capture path starts. A view check
   alone is not sufficient. When the user turns the knob on, Bottle asks for Microphone and
   System Audio Recording at once (the ADR 0101 d7 rule).
3. **Consent notice.** Before the first recording, Bottle shows this notice once, with one
   acknowledgement: "Recording a call may need everyone's consent. Let people know."
   Without the acknowledgement, `recording_start` answers `consent_required`.
4. **Length preset.** Settings has `lengthMin` (default 60). The hub offers 15, 30, 45, 60 and
   90 minutes, or a custom value. Rust accepts only 5 to 120. The recording stops by itself at
   that length. The user can stop it before. `RecordingState.endsAtMs` gives the stop time, so
   the REC state shows the time left.
5. **The record key.** The default record key is Right Option + R (⌥R) on macOS and Right
   Alt + R on Windows. It uses the ⌥G mechanism of ADR 0098. The user holds the talk key and
   taps R: a recording with the preset length starts. The same keys again stop it. The key works
   only while the knob is on. Bottle takes the R key only during the Right Option hold and only
   with the knob on. In other cases, ⌥R types as usual. An R press ends the PTT hold at once: the
   host sends `hotkey.ptt` `cancel`, so no dictation and no paste follow. Then the host sends
   `hotkey.record_toggle`, and the brain calls `recording_start` or `recording_stop`.
6. **The mic during a recording.** The recording holds the mic. One recording runs at a time;
   a second start answers `recording_busy`. While a recording runs, the talk key does not take
   the mic: `capture_start` answers `recording_in_progress`, and the MOG shows "Recording in
   progress". No dictation runs during a recording.
7. **Storage in a UC volume.** The default root is the user's My Files volume
   (`/Volumes/databricks/home/<workspace>/<user_name>/my_files/`). The `<workspace>` segment is
   not documented. Bottle tries the workspace ID first, then the first label of the host, with
   a read-only `GET /api/2.0/fs/directories<path>`. It tries no other value. If My Files does
   not resolve, Settings shows: "My Files isn't available on this workspace. Pick a volume, or a
   catalog and schema." The user gives an existing `/Volumes/c/s/v` path, or a `catalog.schema`.
   For a `catalog.schema`, Bottle creates the managed volume `bottle_audio` after the user
   confirms. Bottle keeps no recording on the device: the local temp audio exists only while it
   records and uploads. Bottle deletes it after the upload succeeds. After a failed upload the
   temp audio stays, so Retry works. At launch a left temp recording shows as "Upload failed"
   with Retry.
8. **Opus audio, one request.** Bottle encodes Opus in Ogg, 16 kbps mono. One hour is about
   9.6 MB as base64, under the 24 MB request budget. Thus one recording goes in one transcribe
   request, with no split. FLAC does not fit the budget. A smart split (cuts at silence, 3 to 5 s
   of overlap, the speaker roster carried to the next part, no duplicate lines) is used only if
   a later limit forces it. Before a request, Bottle checks the magic bytes of the file, and it
   sends only audio that it encoded: a payload that is not audio can give a 200 with invented
   text.
9. **Volume layout.** `<root>/Bottle/Recordings/<name>/`, where `<name>` is the local start time
   `YYYY-MM-DD_HHMM` (plus `_2`, `_3` on a clash). The folder holds the audio `<name>.opus` (Opus in Ogg: `OggS` magic, MIME `audio/ogg`) and the
   `<name>.instructions.md`, `<name>.job.json` (the status manifest),
   `<name>.transcript.json`, `<name>.transcript.md` and `<name>.notes.md`. Times in the
   transcript are from the start of the recording, across parts. With more than one part, a
   speaker label has the part as a prefix (for example `2.1`), because the labels of two parts
   can differ. The recordings list reads each `job.json` under `<root>/Bottle/Recordings/`,
   newest first.
10. **Model calls only, no compute.** Bottle uses no SQL warehouse, no job, no
    `ai_transcribe` and no statements API. Two chat calls go through the existing AI Gateway
    route `mlflow.chat_completions` (`POST /ai-gateway/mlflow/v1/chat/completions`,
    `model=<endpoint>`, NFR-21b), with the home profile token. The transcribe call sends the
    audio inline as an `audio_url` data URL and a prompt for the strict line format
    `[mm:ss] Speaker N: text`. Bottle parses the lines tolerantly. The answer content can be a
    list of text parts: Bottle joins them. The notes call is one chat call with the
    instructions and the transcript.
11. **Two model settings.** `transcribeEndpoint` (default `databricks-gemini-3-8-flash`) and
    `notesEndpoint` (default `databricks-claude-sonnet-5-5`) are Settings values under
    "Advanced". Rust checks each name (`[A-Za-z0-9_-]`). A
    missing endpoint, or no permission on it, gives `transcribe_endpoint_missing` or
    `notes_endpoint_missing`: "The <transcribe|notes> model isn't available. Pick another model in
    Settings." An audio file that fails the magic-byte check, or that the model cannot read,
    gives `audio_unreadable`. Audio over the request budget gives `too_long`. A 429 retries with backoff through the shared rate bucket, then gives
    `rate_limited`.
12. **Instructions per job.** Settings holds a preset prompt (the default is below). The user
    can edit the instructions for one recording at its start (optional, `recording_start
    {instructions?}`). Bottle saves them with the job (`<name>.instructions.md` and in the
    transcript JSON), and the notes call uses them. Rust checks each volume path with a strict
    character set.
13. **Link back.** When a job is done, Bottle shows a link to the folder in Catalog Explorer,
    a short OS notice and the MOG answer-ready moment. Rust builds the link from the profile
    host (`https://<host>/explore/data/volumes/<catalog>/<schema>/<volume>?o=<workspace_id>&volumePath=<folder>/`)
    and opens it with the OS opener (`recording_open {id, target: "folder"}`). A view never gets
    or passes a URL.
14. **Status flow.** `recording`, `uploading`, `transcribing` (the transcribe calls),
    `writing_notes` (the notes call and the file writes), then `done` or `failed` with an error
    kind and the failed step. Bottle writes `<name>.transcript.json`, `<name>.transcript.md`
    and `<name>.notes.md` to the volume, then shows the link. Retry starts again at the failed
    step.
15. **Errors in plain words, one action each.** The hub maps each `RecordingErrorKind` to one
    message. For example, `volume_permission`: "Bottle can't write to this volume. Pick another
    volume in Settings." `system_audio_denied`:
    "Bottle can't hear the other side of the call. Allow System Audio Recording."
16. **Ownership and security.** Rust owns the capture, the files, the network, the tokens and
    the pipeline. The brain relays the state to the MOG (REC mood, answer-ready moment, OS
    notice). Views render and send input only (`view_emit`). Tokens come only from TokenBroker
    with the home profile, live in memory and are never logged. Every call goes through
    HttpTransport to the profile host over HTTPS. The settings are a small JSON file in the
    app config dir (no secret), because AppSettings is still in memory (#77).

## The contract (this PR)

- Commands, brain and hub (the hub view reaches them through the brain, ARC-2):
  `recording_settings_read`, `recording_settings_write {patch}`, `recording_storage_probe`,
  `recording_volume_create {catalog, schema}`,
  `recording_start {instructions?}`, `recording_stop`, `recording_state`, `recording_list`,
  `recording_detail {id}`, `recording_retry {id}`, `recording_open {id, target}`,
  `recording_open_system_audio_settings` (Rust opens the OS privacy pane for the fix-it). A MOG view
  has no recording permission: a MOG "Record" click goes through `view_emit` to the brain.
- Events, core to brain: `hotkey.record_toggle` (`RecordToggle`), `recording.state`
  (`RecordingState`, not sticky) and `recording.job` (`RecordingSummary`).
- New refusal codes: `recording_off`, `recording_busy`, `recording_in_progress`,
  `unknown_recording`, `no_storage`, `volume_permission`, `transcribe_endpoint_missing`,
  `notes_endpoint_missing`, `too_long`, `audio_unreadable`, `upload_failed`, `system_audio_denied`. Rate limit,
  network, auth and internal faults use `rate_limited`, `offline`, `auth_expired` and
  `host_failed`.
- `mog_core::recording` holds the seams `RecordingSettingsStore`, `RecordingCapture` and
  `RecordingCloud`, and `RecordingComponent`, which checks the knob. In this PR every other
  command answers `not_implemented`.

## The PR split

- PR A (`issue/rec-0-contract`): this ADR and the contract.
- PR B (`issue/rec-1-capture`): system audio, the mixer, the Opus encoder, the session, start and
  stop, the length stop, sleep, quit and device loss, the record key, the permission ask, the
  `Info.plist` key.
- PR C (`issue/rec-2-cloud`): the settings file, My Files, the volume, the upload,
  the transcribe and notes calls, the result files, `job.json`, list, detail, retry and the
  link.
- PR D (`issue/rec-3-hub`): the Settings section, the Record control, the MOG REC state, the
  Recordings area and its detail, the notices.

## Default preset prompt

```
You get the transcript of a call. Speakers are labeled Speaker 1, Speaker 2, and so on.
Write short notes in Markdown with these sections:
## Summary
Three to five sentences.
## Decisions
A bullet list. Write "None" if there are no decisions.
## Action items
One sub-heading per speaker. Under each, a bullet list of that speaker's action items, with an owner and a due date if the call names one.
Use only facts from the transcript.
```

## Findings (probe of 2026-11-19)

The record of the probe is [docs/evidence/rec-transcribe-probe-2026-11-19.md](../evidence/rec-transcribe-probe-2026-11-19.md).

- The `file` and `audio_url` content types take inline data only. A Volumes path, a Files URL
  or a file ID is refused, and the model provider cannot authenticate to a Files URL.
- The request cap is about 30 MB. Bottle uses a 24 MB budget.
- Opus in Ogg at 16 kbps mono is about 9.6 MB of base64 per hour. FLAC does not fit.
- The audio costs about 1500 tokens per minute. A 60 min recording takes about 2 to 3 min.
- A payload that is not audio can give a 200 with invented text (decision 8).

## Consequences

- The requirements rows CAP-07b and the #116 local STT path do not apply to call recording.
  The audio goes to a model endpoint of the user's workspace through the AI Gateway.
  The requirements owner must update them.
- The recording audio and text leave the device for the user's workspace. The knob is off by
  default, and the consent notice shows before the first recording.
- The My Files path segment is not documented. If Databricks changes it, Bottle falls back to
  the Settings volume.

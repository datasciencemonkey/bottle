# ADR 0103: Call recording to a Unity Catalog volume, with a server-side transcript and notes

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
must come from Databricks AI functions.

The requirements describe a different meeting mode. They keep the You track and the Others track
apart (CAP-07a, CAP-07b) and run local STT on segments (#116). The user decisions below
replace these rules for call recording.

## Decisions

1. **One mixed stream.** Bottle records the mic and the system audio and mixes them to one
   16 kHz mono stream (sum with a soft clip). This deviates from CAP-07b. `ai_transcribe` does
   speaker diarization, so the mixed stream still gives speaker turns. macOS 14.4+ uses a global
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
   90 minutes, or a custom value. Rust accepts only 5 to 180. The recording stops by itself at
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
8. **FLAC parts.** Bottle encodes FLAC (pure-Rust encoder, cross-platform, about 25 MB per hour).
   `ai_transcribe` accepts FLAC. A part is at most 55 min, under the 1 h and 512 MB limits. A
   90 min recording gives two parts.
9. **Volume layout.** `<root>/Bottle/Recordings/<name>/`, where `<name>` is the local start time
   `YYYY-MM-DD_HHMM` (plus `_2`, `_3` on a clash). The folder holds `<name>.part01.flac` and the
   next parts, `<name>.instructions.md`, `<name>.job.json` (the status manifest),
   `<name>.transcript.json`, `<name>.transcript.md` and `<name>.notes.md`. Times in the
   transcript are from the start of the recording, across parts. With more than one part, a
   speaker label has the part as a prefix (for example `2.1`), because the labels of two parts
   can differ. The recordings list reads each `job.json` under `<root>/Bottle/Recordings/`,
   newest first.
10. **A server-side job transcribes.** After the upload, Bottle submits a one-time serverless
    job run (`POST /api/2.2/jobs/runs/submit`, a `spark_python_task` on serverless compute
    through `environments`, no cluster). The run reads the parts, runs `ai_transcribe` per
    part, runs `ai_query` with the instructions, and writes the transcript and notes files into
    the same folder. It also writes the final status (`done` or `failed` with the error kind)
    into `job.json`. Thus the job ends and the state is correct when the laptop sleeps or
    Bottle is closed. The task script is a fixed, versioned file in the repo, with its own
    tests. Bottle uploads it once per version to `<root>/Bottle/.bottle/` (for example
    `transcribe_v1.py`). Bottle records the `run_id` in `job.json` and reads
    `GET /api/2.2/jobs/runs/get` while it is awake.
11. **The SQL warehouse fallback.** If `runs/submit` fails because serverless jobs are not
    usable in the workspace, Bottle uses a SQL warehouse: the statements API runs
    `ai_transcribe` and `ai_query`, and Bottle writes the files. The user picks the warehouse
    once in Settings from `GET /api/2.0/sql/warehouses`. Bottle starts a stopped warehouse and
    waits. The warehouse setting is optional: a missing warehouse is an error
    (`no_warehouse`) only when the job path is not usable. `job.json` and `RecordingSummary`
    record the engine (`job` or `warehouse`).
12. **Instructions per job.** Settings holds a preset prompt (the default is below). The user
    can edit the instructions for one recording at its start (optional, `recording_start
    {instructions?}`). Bottle saves them with the job (`<name>.instructions.md` and in the
    transcript JSON). The notes come from `ai_query('<endpoint>', concat(instructions,
    transcript))`. The endpoint is a Settings value; the default is
    `databricks-claude-sonnet-5-5`. Rust checks the endpoint name (`[A-Za-z0-9_-]`) and each
    volume path (a strict character set), because SQL text uses them as literals.
13. **Link back.** When a job is done, Bottle shows a link to the folder in Catalog Explorer,
    a short OS notice and the MOG answer-ready moment. Rust builds the link from the profile
    host (`https://<host>/explore/data/volumes/<catalog>/<schema>/<volume>?o=<workspace_id>&volumePath=<folder>/`)
    and opens it with the OS opener (`recording_open {id, target: "folder"}`). A view never gets
    or passes a URL.
14. **Status flow.** `recording`, `uploading`, `transcribing` (the job run is queued or runs),
    then `done` or `failed` with an error kind and the failed step. `writing_notes` stays in the
    enum; the job path reports it only if the script updates `job.json` between its steps.
    Retry starts again at the failed step.
15. **Errors in plain words, one action each.** The hub maps each `RecordingErrorKind` to one
    message. For example, `ai_transcribe_disabled`: "AI Transcribe is off for this workspace.
    Ask your workspace admin to turn on AI Transcribe in Previews." `system_audio_denied`:
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
  `recording_volume_create {catalog, schema}`, `recording_warehouses`,
  `recording_start {instructions?}`, `recording_stop`, `recording_state`, `recording_list`,
  `recording_detail {id}`, `recording_retry {id}`, `recording_open {id, target}`. A MOG view
  has no recording permission: a MOG "Record" click goes through `view_emit` to the brain.
- Events, core to brain: `hotkey.record_toggle` (`RecordToggle`), `recording.state`
  (`RecordingState`, not sticky) and `recording.job` (`RecordingSummary`).
- New refusal codes: `recording_off`, `recording_busy`, `recording_in_progress`,
  `unknown_recording`, `no_storage`, `no_warehouse`, `warehouse_failed`, `volume_permission`,
  `ai_transcribe_disabled`, `endpoint_missing`, `too_long`, `upload_failed`,
  `system_audio_denied`. Network, auth and internal faults use `offline`, `auth_expired` and
  `host_failed`.
- `mog_core::recording` holds the seams `RecordingSettingsStore`, `RecordingCapture` and
  `RecordingCloud`, and `RecordingComponent`, which checks the knob. In this PR every other
  command answers `not_implemented`.

## The PR split

- PR A (`issue/rec-0-contract`): this ADR and the contract.
- PR B (`issue/rec-1-capture`): system audio, the mixer, FLAC, parts, the session, start and
  stop, the length stop, sleep, quit and device loss, the record key, the permission ask, the
  `Info.plist` key.
- PR C (`issue/rec-2-cloud`): the settings file, My Files, the volume, warehouses, the upload,
  the job run and its task script, the warehouse fallback, `job.json`, list, detail, retry and
  the link.
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

## Consequences

- The requirements rows CAP-07b and the #116 local STT path do not apply to call recording.
  The requirements owner must update them.
- The recording audio and text leave the device for the user's workspace. The knob is off by
  default, and the consent notice shows before the first recording.
- The My Files path segment is not documented. If Databricks changes it, Bottle falls back to
  the Settings volume.

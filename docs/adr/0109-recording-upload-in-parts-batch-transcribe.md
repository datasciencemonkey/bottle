# ADR 0109: Call recording uploads in parts during the call and transcribes once at the end

- Status: accepted (owner decision, 2026-11-20)
- Refs: #65, #107, #108
- Date: 2026-11-20
- Amends: ADR 0103 decisions 7, 8 and 9 (the contract change is three optional fields of `RecordingSummary`, approved by the owner on 2026-11-20)

## Context

ADR 0103 records one file and uploads it after the call. A crash or a sleep during a long call loses the call. The
audio stays only on the device until the end.

## Decisions

1. **Parts during capture.** The capture cuts a new Ogg Opus part every 5 minutes of audio (`PART_MAX_MINUTES`,
   counted in 16 kHz samples). A part is `<name>.partNN.opus`, about 600 KB. The cut is exact and needs no clock.
2. **Upload during the call.** One worker per recording sends each closed part to the volume folder while the
   call goes on. The order for each part is: upload (`overwrite=false`, a 409 counts as done), write `job.json`
   (the part is marked uploaded), delete the local part. This is the order of ADR 0103. If the upload or the
   `job.json` write fails, the part stays on the device.
3. **Consent.** Upload during the call needs no new consent. The user starts the recording, and
   `recording_start` already needs the knob and the consent notice. The worker starts only for such a recording.
   The knob, the length preset and the auto-stop apply as before.
4. **Recording never stops for a failed upload.** On a failure the worker waits 5 s, then 10 s, then 20 s, up to
   60 s, and tries again. The capture does not know about it. After the stop the worker keeps trying for a network
   fault. A permanent fault (no permission on the volume) fails the job with its kind, and the audio stays for Retry.
5. **One transcription at the end.** Bottle transcribes the whole call in one pass after the stop, so the speaker
   labels stay the same from the start to the end. No streaming and no chunking of the transcription.
6. **Join of the parts.** The transcribe request needs one audio file. Bottle joins the Ogg Opus parts in memory
   with no re-encode: it keeps the first `OpusHead` and `OpusTags`, drops the later ones, writes one serial number
   and page numbers from 0, gives packet `k` the granule `k * 960`, sets the exact end granule on the last packet,
   and sets the end-of-stream flag once. The result is one standard stream, not a chained Ogg. The parts are read
   back from the volume, because the local copies are deleted. The request size check of ADR 0103 stays
   (24 MB budget, 120 minutes is about 19 MB as base64). A part that is cut short by a crash keeps its whole packets.
   Parts that are not Ogg Opus (FLAC, WAV) or do not join go part by part, as before.
7. **Volume layout.** The audio files are `<name>.part01.opus`, `<name>.part02.opus`, and so on. This replaces
   `<name>.opus` of ADR 0103 decision 9. The transcript has one part, so the speaker label has no part prefix.
8. **Crash recovery.** At the start, Bottle writes `job.local.json` (user-only) in the temp dir: the recording ID
   and name, the start time, the profile name and the instructions. It holds no token and no audio. At launch, a
   dir with this file and parts resumes in the background, only with the knob on, the consent acknowledged and a
   storage set. The worker keeps the parts that `job.json` marks as uploaded and sends the rest. A recording with
   no `job.local.json` (an old build) stays on the device as "Not uploaded" until the user clicks Upload.
9. **Old location.** The temp audio was under `app_cache_dir()/recordings` before #592 and is now under
   `app_data_dir()/recordings`. The scan reads both. The same scan feeds the list and Retry.
10. **The Recordings list.** `recording_list` merges three sources: the `job.json` files in the volume, the live
    workers (the counts, and the network mark) and the temp folders on the device. Newest first. Each row has a
    status: recording, uploading N/M, waiting for network, transcribing, writing notes, done, failed with a reason,
    or not uploaded. "Waiting for network" is a `recording` or `uploading` status with `errorKind: network`. A
    `job.json` in `recording` with no worker or capture is a crash and reads as failed at the upload. A `job.json`
    in another running status that has not changed for 30 minutes reads as failed at its step. Actions: Retry
    (failed), Upload (not uploaded), Open the notes and the transcript (the detail), and Open in volume. The brain
    pushes the list on each `recording.job` event. Nothing polls.
11. **Contract.** `RecordingSummary` gets three optional fields: `partsUploaded`, `partsTotal` and `onDeviceOnly`.
    No enum changes. No new command or event.

## Consequences

- A crash loses at most the open part (5 minutes), and the stop uploads only the last part.
- The volume holds up to 24 audio files per call. The list and the detail read the manifest, not the folder.
- Rotation and upload run on the capture and worker threads. A slow network cannot slow the capture.
- The join relies on the parts coming from one encoder. A different encoder setting needs a new join check.
- Not tested live: that the model keeps one timeline across the joined stream. The join test decodes every packet
  and checks the page structure and the CRCs, but no model call ran (no live Databricks calls).

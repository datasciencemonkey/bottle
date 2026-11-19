# Call recording: transcribe probe evidence (2026-11-19)

Drafted to STE100 rules. This record holds no token values, no secrets and no workspace catalog or table names.

## Setup

- Date: 2026-11-19.
- Profile: DEFAULT. Auth is OAuth U2M through the Databricks CLI (`databricks auth token`).
- Transcribe endpoint: `databricks-gemini-3-8-flash`.
- Notes endpoint: `databricks-claude-sonnet-5-5`.
- Route: `POST /ai-gateway/mlflow/v1/chat/completions`, with the endpoint in the body field `model`. This route obeys NFR-21a and NFR-21b. The `/serving-endpoints/<name>/invocations` route gave the same transcript, but Bottle does not use it (NFR-21b).
- Audio: synthetic voices only (macOS `say`, voices Samantha and Daniel). `afconvert` and `ffmpeg` made the WAV, FLAC and Opus files.
- Writes: none. The probes wrote nothing to the workspace and nothing to a volume.

## Tested, with numbers

### Content types

The request content is `[{type:"text", text:<prompt>}, {type:"audio_url", audio_url:{url:"data:<mime>;base64,<data>"}}]`.

| Shape | Result |
|---|---|
| `audio_url` with a base64 data URL | HTTP 200, correct transcript. |
| `file` with `file_data` (a base64 data URL) and `filename` | HTTP 200, correct transcript. |
| `input_audio` | HTTP 400: "Content type in user messages can only be 'text', 'file', 'image_url', 'video_url', or 'audio_url'." |
| `file` with `file_id` only | HTTP 400: "Missing file_data field from the file object." |
| `file` with `url` set to a `/Volumes/...` path | HTTP 400: "Invalid URL scheme: ... Only HTTP and HTTPS URLs are supported." |
| `audio_url` with a `/Volumes/...` path or a `dbfs:` path | HTTP 400: "Invalid base64 data URL format. Expected format: data:{mimeType};base64,..." |
| `audio_url` with a Files API HTTPS URL | HTTP 400: "Cannot fetch content from the provided URL ... URL_ERROR-ERROR_AUTHENTICATION". The model service fetches the URL itself and cannot sign in. |

Result: the audio must be inline. A reference to a file in a volume does not work.

### Codecs

These data URL types gave a correct transcript: `audio/wav`, `audio/flac`, `audio/ogg` (Opus), `audio/webm` (Opus) and `audio/opus`.

### Request size cap

- A request of 30.09 MB reached the model layer (HTTP 400 for a test field, so the size passed).
- A request of 32 MB or more lost the TLS connection before a reply.
- Bottle sets a budget of 24 MB per request.

### Audio size

- Opus mono at 16 kbps is about 2 KB per second.
- 60 min of Opus is about 7.2 MB, or about 9.6 MB in base64.
- 90 min is about 14.4 MB in base64. 120 min is about 19.2 MB in base64.
- Result: one request can hold one recording of up to 120 min. Bottle does not split the audio.

### Tokens and latency

- Audio tokens: about 25 per second, about 1500 per minute (8191 audio tokens for 5.4 min).
- A 5.4 min clip with 2 speakers took 13.7 s. The reply had 2393 output tokens and 72 lines. The timestamps and the speaker turns were correct.
- A 22 s clip with 2 speakers took 2 to 3 s.
- The notes call (Claude, short text) took about 1 s. Its reply content is a plain string.
- The Gemini reply content is a list of parts `{type:"text", text, thoughtSignature}`. Bottle joins the `text` parts.

### Hallucination risk

A request with `data:text/plain;base64,...` (no audio) got HTTP 200 and a transcript that the model made up. Bottle sends only bytes that it encoded. Bottle checks the magic bytes (`OggS`, `fLaC`, `RIFF....WAVE`) and sets a fixed MIME type.

### Error bodies

| Case | Reply |
|---|---|
| Missing endpoint on the gateway route | HTTP 404 `{"error_code":"NOT_FOUND","message":"'<name>' does not exist."}` |
| Missing endpoint on `/serving-endpoints` | HTTP 404 `ENDPOINT_NOT_FOUND` |
| Corrupt audio bytes | HTTP 400 `BAD_REQUEST`, "The provided file is malformed or corrupted and could not be decoded." |

### "You" label hint

The prompt hint was: "The person closest to the microphone is the user who recorded the call; label them You if you can tell." We tried it two times on the 22 s clip. The transcript did not change: both voices stayed "Speaker 1" and "Speaker 2". The synthetic voices have the same level, so this test cannot show a benefit. The hint did not cause damage, so Bottle keeps it.

## Not tested

- Mic and system audio capture.
- The ⌥R key.
- The REC state of the MOG.
- Volume upload and file writes.
- The Recordings page and the Catalog Explorer link.
- Real human voices, noise, and 3 or more speakers.
- Live recordings longer than 5.4 min.
- Windows loopback capture.
- The full flow in the app, from start to end.

## How to run the probes again

1. Make a synthetic clip. Use synthetic voices only.

   ```sh
   say -v Samantha -o /tmp/probe/a.wav --data-format=LEI16@16000 "Good morning. Let's review the plan."
   ffmpeg -loglevel error -y -i /tmp/probe/a.wav -c:a libopus -b:a 16k -ac 1 /tmp/probe/a.ogg
   ```

2. Write the request body to a file. The token must not show in the output.

   ```sh
   B64=$(base64 -i /tmp/probe/a.ogg | tr -d '\n')
   cat > /tmp/probe/body.json <<EOF
   {"model":"databricks-gemini-3-8-flash","max_tokens":4000,"messages":[{"role":"user","content":[
     {"type":"text","text":"Transcribe this audio verbatim with speaker labels and mm:ss timestamps. Write one line per turn, exactly in this format: [mm:ss] Speaker N: text. Write nothing else."},
     {"type":"audio_url","audio_url":{"url":"data:audio/ogg;base64,$B64"}}]}]}
   EOF
   ```

3. Send the request. The token goes from the CLI to curl through a header file on stdin, so it does not show.

   ```sh
   databricks auth token --profile <your-profile> \
     | uv run --no-project python -c 'import json,sys; print("Authorization: Bearer " + json.load(sys.stdin)["access_token"])' \
     | curl -sS -H @- -H 'Content-Type: application/json' \
         --data @/tmp/probe/body.json \
         https://<workspace-host>/ai-gateway/mlflow/v1/chat/completions
   ```

4. For the notes call, send `{"model":"databricks-claude-sonnet-5-5","max_tokens":200,"messages":[{"role":"user","content":"<instructions>\n\nTranscript:\n<lines>"}]}` the same way.

5. To check the size cap without model cost, add a large unknown top-level field to a small request. HTTP 400 "Unknown name" means the size passed. A dropped connection means the size failed.

6. Delete `/tmp/probe` when you finish.

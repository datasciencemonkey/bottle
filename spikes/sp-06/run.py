#!/usr/bin/env python3
"""SP-06 (#48): Unity Gateway audio MIME types and payload size.

Sends audio data URIs to POST /ai-gateway/mlflow/v1/chat/completions with the default ASR model
(CAP-62a: system.ai.gemini-3-1-flash-lite) and records the status and error shape per case.

Cases:
- 30 s clips as audio/ogg (Vorbis), audio/opus (Ogg Opus), audio/flac and audio/wav.
- WAV request bodies of 3.5 MB (must be accepted) and 4.1 MB (must be rejected cleanly).
  A 4.1 MiB case shows if the limit is decimal MB or binary MiB.

Clip generator: a deterministic speech-band synth (stdlib `wave`) for WAV. `soundfile` encodes
OGG, Opus and FLAC; if it fails, ffmpeg does. The report records which tool made each clip.

Run:
  uv run --no-project --with httpx --with soundfile --with numpy \
      spikes/sp-06/run.py --profile <your-profile> --out sp06.json
"""
import argparse
import base64
import io
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import wave

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "common"))
import spikelib as sl  # noqa: E402

SPIKE = "SP-06"
DEFAULT_MODEL = "system.ai.gemini-3-1-flash-lite"  # CAP-62a
PATH = "/ai-gateway/mlflow/v1/chat/completions"
MIME = {"ogg": "audio/ogg", "opus": "audio/opus", "flac": "audio/flac", "wav": "audio/wav"}
MB = 1_000_000
MIB = 1_048_576
SIZE_CASES = {"3.5MB": 3_500_000, "4.1MB": 4_100_000, "4.1MiB": round(4.1 * MIB)}
PROMPT = "Transcribe this audio. Reply with the transcript only."


# ------------------------------------------------------------------ audio synthesis


def synth_samples(seconds, rate, seed=6):
    """Deterministic speech-band signal: harmonic 'syllables' at 4 Hz with pauses. int16 list."""
    n = int(seconds * rate)
    f0s = [110, 135, 160, 125, 145]
    per = max(1, rate // 4)
    chunks = {}
    for f0 in f0s:
        chunk = []
        for i in range(per):
            t = i / rate
            env = math.sin(math.pi * i / per)
            v = sum(math.sin(2 * math.pi * f0 * h * t) / h for h in (1, 2, 3, 5))
            chunk.append(int(9000 * env * v / 1.9))
        chunks[f0] = chunk
    silence = [0] * per
    out = []
    syl = 0
    while len(out) < n:
        out.extend(silence if syl % 7 == 6 else chunks[f0s[(syl + seed) % len(f0s)]])
        syl += 1
    return out[:n]


def wav_bytes(samples, rate):
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack("<%dh" % len(samples), *samples))
    return buf.getvalue()


def encode(fmt, samples, rate):
    """Return (bytes, tool). fmt in ogg, opus, flac, wav."""
    if fmt == "wav":
        return wav_bytes(samples, rate), "stdlib wave"
    try:
        import numpy as np
        import soundfile as sf

        data = np.asarray(samples, dtype=np.int16)
        buf = io.BytesIO()
        if fmt == "flac":
            sf.write(buf, data, rate, format="FLAC", subtype="PCM_16")
        elif fmt == "ogg":
            sf.write(buf, data, rate, format="OGG", subtype="VORBIS")
        else:
            sf.write(buf, data, 48000 if rate not in (8000, 12000, 16000, 24000, 48000) else rate,
                     format="OGG", subtype="OPUS")
        return buf.getvalue(), f"soundfile {sf.__version__} (libsndfile {sf.__libsndfile_version__})"
    except Exception as e:  # noqa: BLE001 - fall back to ffmpeg
        first = f"{type(e).__name__}"
    ff = shutil.which("ffmpeg")
    if not ff:
        raise sl.SpikeError(f"cannot encode {fmt}: soundfile failed ({first}) and ffmpeg is not on PATH")
    codec = {"flac": ["-c:a", "flac", "-f", "flac"], "ogg": ["-c:a", "libvorbis", "-f", "ogg"],
             "opus": ["-c:a", "libopus", "-f", "ogg"]}[fmt]
    proc = subprocess.run([ff, "-hide_banner", "-loglevel", "error", "-f", "wav", "-i", "pipe:0",
                           *codec, "pipe:1"], input=wav_bytes(samples, rate), capture_output=True)
    if proc.returncode != 0:
        raise sl.SpikeError(f"ffmpeg failed for {fmt}")
    return proc.stdout, "ffmpeg"


# ------------------------------------------------------------------ request bodies


def request_body(model, mime, audio):
    uri = f"data:{mime};base64," + base64.b64encode(audio).decode("ascii")
    msg = {"model": model, "max_tokens": 256, "temperature": 0,
           "messages": [{"role": "user", "content": [
               {"type": "text", "text": PROMPT},
               {"type": "audio_url", "audio_url": {"url": uri}}]}]}
    return json.dumps(msg, separators=(",", ":")).encode("utf-8")


def wav_seconds_for_body(target, model, rate=44100):
    """Seconds of 16-bit mono WAV at `rate` whose request body is as close to `target` as possible."""
    overhead = len(request_body(model, MIME["wav"], wav_bytes([], rate)))
    b64_budget = target - overhead
    raw = (b64_budget // 4) * 3
    frames = max(0, (raw - 44) // 2)
    return frames / rate


def sized_wav_body(target, model, rate=44100):
    """Request body of exactly `target` bytes or a few bytes under it, with real WAV audio."""
    secs = wav_seconds_for_body(target, model, rate)
    audio = wav_bytes(synth_samples(secs, rate), rate)
    body = request_body(model, MIME["wav"], audio)
    return body, secs, audio


# ------------------------------------------------------------------ evaluation


def classify(resp):
    """accepted / rejected_clean / rejected_unclean / transport_error."""
    if resp.status == 0:
        return "transport_error"
    if 200 <= resp.status < 300:
        return "accepted"
    shape = sl.error_shape(resp)
    if 400 <= resp.status < 500 and shape.get("json_keys"):
        return "rejected_clean"
    return "rejected_unclean"


def result_row(case, fmt, mime, body_len, audio_len, seconds, tool, resp):
    row = {"case": case, "format": fmt, "mime": mime, "body_bytes": body_len,
           "audio_bytes": audio_len, "seconds": round(seconds, 2), "encoder": tool,
           "status": resp.status, "elapsed_ms": resp.elapsed_ms, "outcome": classify(resp)}
    if row["outcome"] == "accepted":
        try:
            choice = (resp.json() or {}).get("choices", [{}])[0]
            # Content-free: only the length of the model's text.
            row["output_chars"] = len(sl.message_text(choice.get("message")))
            row["finish_reason"] = choice.get("finish_reason")
        except (ValueError, IndexError, AttributeError):
            row["output_chars"] = None
    else:
        row["error_shape"] = sl.error_shape(resp)
    return row


def evaluate(rows):
    by = {r["case"]: r for r in rows}
    fmt_rows = [r for r in rows if r["case"].startswith("30s-")]
    formats_known = len(fmt_rows) == 4 and all(r["outcome"] != "transport_error" for r in fmt_rows)
    ok_35 = by.get("size-3.5MB", {}).get("outcome") == "accepted"
    rej_41 = by.get("size-4.1MB", {}).get("outcome") == "rejected_clean"
    accepted = sorted(r["format"] for r in fmt_rows if r["outcome"] == "accepted")
    passed = formats_known and ok_35 and rej_41
    detail = (f"formats_known={formats_known} accepted_formats={accepted} 3.5MB_accepted={ok_35} "
              f"4.1MB_rejected_cleanly={rej_41} 4.1MiB={by.get('size-4.1MiB', {}).get('outcome')}")
    fallback = None if passed else "Use the accepted format; lower segments to 20 s."
    return {"passed": passed, "formats_known": formats_known, "accepted_formats": accepted,
            "accepted_3_5mb": ok_35, "rejected_4_1mb_cleanly": rej_41, "detail": detail,
            "fallback": fallback}


def run(args, api, encoder=None):
    encoder = encoder or encode
    rows = []
    rate = 16000
    base = synth_samples(30, rate)
    for fmt in ("ogg", "opus", "flac", "wav"):
        audio, tool = encoder(fmt, base, rate)
        body = request_body(args.model, MIME[fmt], audio)
        sl.say(f"{SPIKE}: 30s {fmt} body={len(body)} bytes ...")
        resp = api.call("POST", PATH, raw=body)
        rows.append(result_row(f"30s-{fmt}", fmt, MIME[fmt], len(body), len(audio), 30, tool, resp))
    for name, target in SIZE_CASES.items():
        body, secs, audio = sized_wav_body(target, args.model)
        sl.say(f"{SPIKE}: size {name} body={len(body)} bytes ({secs:.1f} s WAV 44.1 kHz) ...")
        resp = api.call("POST", PATH, raw=body)
        rows.append(result_row(f"size-{name}", "wav", MIME["wav"], len(body), len(audio), secs,
                               "stdlib wave", resp))
    return rows


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--profile", required=True, help="Databricks CLI profile (you choose it)")
    p.add_argument("--out", required=True, help="path of the JSON report")
    p.add_argument("--model", default=DEFAULT_MODEL)
    args = p.parse_args(argv)

    def go():
        host = sl.get_host(args.profile)
        token = sl.get_token(args.profile)
        api = sl.Api(host, token, sl.HttpxTransport(timeout=120))
        rows = run(args, api)
        ev = evaluate(rows)
        report = {"spike": SPIKE, "issue": 48, "model": args.model, "path": PATH,
                  "mb_definition": {"MB": MB, "MiB": MIB}, "results": rows, "evaluation": ev}
        sl.write_report(args.out, report, secrets=[token.reveal()])
        for r in rows:
            sl.say(f"  {r['case']:<12} status={r['status']} outcome={r['outcome']} body={r['body_bytes']}")
        sl.say(sl.verdict(SPIKE, ev["passed"], ev["detail"]))
        token.wipe()
        return 0 if ev["passed"] else 1

    return sl.main_guard(go)


if __name__ == "__main__":
    sys.exit(main())

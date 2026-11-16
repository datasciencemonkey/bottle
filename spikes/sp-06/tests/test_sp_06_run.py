"""[SP-06] Offline tests for spikes/sp-06/run.py (recorded fake responses, no network)."""
import base64
import io
import json
import os
import sys
import wave
from types import SimpleNamespace

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "common"))
from loader import FAKE_HOST, FakeTransport, fake_token, load_run, resp  # noqa: E402

import spikelib as sl  # noqa: E402

r6 = load_run("sp-06", "sp06_run")


def test_sp_06_wav_is_30s_mono_16bit():
    """[SP-06] The WAV synth makes a 30 s, 16 kHz, mono, 16-bit clip."""
    audio = r6.wav_bytes(r6.synth_samples(30, 16000), 16000)
    with wave.open(io.BytesIO(audio)) as w:
        assert (w.getnchannels(), w.getsampwidth(), w.getframerate()) == (1, 2, 16000)
        assert w.getnframes() / w.getframerate() == 30


def test_sp_06_synth_is_deterministic():
    """[SP-06] The same seed gives the same samples (repeatable runs)."""
    assert r6.synth_samples(0.1, 16000) == r6.synth_samples(0.1, 16000)


def test_sp_06_body_uses_audio_url_data_uri():
    """[SP-06] [CAP-53b] The request has a base64 audio_url data URI with the MIME type."""
    body = json.loads(r6.request_body("system.ai.gemini-3-1-flash-lite", "audio/flac", b"abc"))
    part = body["messages"][0]["content"][1]
    assert part["type"] == "audio_url"
    assert part["audio_url"]["url"] == "data:audio/flac;base64," + base64.b64encode(b"abc").decode()
    assert body["model"] == "system.ai.gemini-3-1-flash-lite"


def test_sp_06_sized_bodies_hit_the_targets():
    """[SP-06] The 3.5 MB, 4.1 MB and 4.1 MiB bodies are within 64 bytes under their targets."""
    for target in r6.SIZE_CASES.values():
        body, secs, _ = r6.sized_wav_body(target, r6.DEFAULT_MODEL)
        assert target - 64 <= len(body) <= target
    body, secs, _ = r6.sized_wav_body(r6.SIZE_CASES["3.5MB"], r6.DEFAULT_MODEL)
    assert secs <= 30, "the 3.5 MB case must stay inside the 30 s segment cap (CAP-53a)"


def test_sp_06_classify_outcomes():
    """[SP-06] 2xx is accepted; 4xx with a JSON error is a clean reject; 5xx or HTML is not clean."""
    assert r6.classify(resp(200, {"choices": []})) == "accepted"
    assert r6.classify(resp(413, {"error_code": "REQUEST_TOO_LARGE", "message": "too big"})) == "rejected_clean"
    assert r6.classify(resp(413, raw=b"<html>too big</html>", headers={"content-type": "text/html"})) == "rejected_unclean"
    assert r6.classify(resp(502, {"message": "bad gateway"})) == "rejected_unclean"
    assert r6.classify(resp(0, {"transport_error": "RemoteProtocolError"})) == "transport_error"


def _rows(**outcomes):
    rows = []
    for fmt in ("ogg", "opus", "flac", "wav"):
        rows.append({"case": f"30s-{fmt}", "format": fmt, "outcome": outcomes.get(fmt, "accepted")})
    for name in ("3.5MB", "4.1MB", "4.1MiB"):
        rows.append({"case": f"size-{name}", "format": "wav", "outcome": outcomes.get(name, "accepted")})
    return rows


def test_sp_06_pass_rule():
    """[SP-06] Pass: formats known, 3.5 MB accepted, 4.1 MB rejected cleanly."""
    ev = r6.evaluate(_rows(**{"4.1MB": "rejected_clean", "4.1MiB": "rejected_clean", "opus": "rejected_clean"}))
    assert ev["passed"] and ev["accepted_formats"] == ["flac", "ogg", "wav"]
    assert ev["fallback"] is None


def test_sp_06_fail_when_4_1mb_accepted_or_unclean():
    """[SP-06] A 4.1 MB body that is accepted, or rejected with a 5xx, fails and names the fallback."""
    for outcome in ("accepted", "rejected_unclean"):
        ev = r6.evaluate(_rows(**{"4.1MB": outcome}))
        assert not ev["passed"] and "20 s" in ev["fallback"]


def test_sp_06_fail_when_a_format_has_no_answer():
    """[SP-06] A transport error on one format means formats are not known."""
    ev = r6.evaluate(_rows(**{"ogg": "transport_error", "4.1MB": "rejected_clean"}))
    assert not ev["passed"] and not ev["formats_known"]


def test_sp_06_run_records_rows_without_content(tmp_path):
    """[SP-06] run() sends 7 cases to the gateway path; the row keeps only the output length."""
    tok = fake_token()

    def answer(method, url, headers, body):
        size = len(body)
        if size > r6.SIZE_CASES["3.5MB"]:
            return resp(400, {"error_code": "INVALID_PARAMETER_VALUE", "message": "Request size exceeds 4MB"})
        return resp(200, {"choices": [{"message": {"content": "secret words said"}, "finish_reason": "stop"}]})

    t = FakeTransport(answer)
    api = sl.Api(FAKE_HOST, sl.Token(tok), t)
    def fake_encoder(fmt, samples, rate):
        return r6.wav_bytes(samples, rate), "test"

    rows = r6.run(SimpleNamespace(model=r6.DEFAULT_MODEL), api, encoder=fake_encoder)
    assert len(rows) == 7
    assert all(rq["url"] == FAKE_HOST + r6.PATH for rq in t.requests)
    assert rows[0]["output_chars"] == len("secret words said")
    out = tmp_path / "r.json"
    text = sl.write_report(str(out), {"results": rows, "evaluation": r6.evaluate(rows)}, secrets=[tok])
    assert "secret words" not in text and tok not in text
    assert r6.evaluate(rows)["passed"]
